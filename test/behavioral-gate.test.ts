import { describe, expect, test } from "bun:test";
import { Engine } from "../src/index.ts";
import { makeRepo } from "./helpers.ts";

/**
 * Anti-facade invariant (ADR-002 fitness function, D14): the behavioral tier
 * must detect strictly more than Axis 1. A change to an *imported* file, while
 * the anchored span is untouched, must yield `code:unchanged · behavior:at-risk`.
 * If this can't happen, Tier 3 is a facade and must not ship.
 */
describe("behavioral change-gate v2 (anti-facade)", () => {
  async function seed() {
    const repo = await makeRepo();
    await repo.write("src/dep.ts", "export const RATE = 5;\n");
    await repo.write(
      "src/main.ts",
      'import { RATE } from "./dep.ts";\nexport function apply() {\n  return RATE;\n}\n',
    );
    await repo.write(
      "doc.md",
      "# Doc\n\nThe apply helper retries on failure.\n",
    );
    const engine = await Engine.open(repo.root);
    const rec = await engine.record({
      docPath: "doc.md",
      docQuote: "The apply helper retries on failure.",
      code: [
        {
          file: "src/main.ts",
          quote: "export function apply() {\n  return RATE;\n}",
        },
      ],
      behavioral: true,
      authoredTrust: "verified",
      ref: "PR-1",
      enforcement: "enforced",
    });
    return { repo, engine, claimId: rec.assertion.id };
  }

  test("baseline captures the anchored file AND its import", async () => {
    const { repo, engine, claimId } = await seed();
    const a = await engine.store.getAssertion(claimId);
    expect(a?.evidenceBaseline).toBeDefined();
    expect(Object.keys(a?.evidenceBaseline ?? {})).toContain("src/main.ts");
    expect(Object.keys(a?.evidenceBaseline ?? {})).toContain("src/dep.ts");
    await repo.cleanup();
  });

  test("imported file changed, anchored span untouched ⇒ code:unchanged · behavior:at-risk", async () => {
    const { repo, engine, claimId } = await seed();
    // Change ONLY the imported dependency; src/main.ts is byte-identical.
    await repo.write("src/dep.ts", "export const RATE = 50;\n");

    const report = await engine.check();
    const v = report.verdicts.find((x) => x.assertionId === claimId);
    expect(v).toBeDefined();
    expect(v?.code).toBe("unchanged");
    expect(v?.behavior).toBe("at-risk");
    // The changed evidence names the imported path (D14 noise control).
    expect(
      v?.evidence.changedEvidence.some((c) => c.path === "src/dep.ts"),
    ).toBe(true);
    // at-risk warns, never gates.
    expect(v?.gates).toBe(false);
    await repo.cleanup();
  });

  test("a clean tree keeps the behavioral claim resting (unverified)", async () => {
    const { repo, engine, claimId } = await seed();
    const report = await engine.check();
    const v = report.verdicts.find((x) => x.assertionId === claimId);
    expect(v?.code).toBe("unchanged");
    expect(v?.behavior).toBe("unverified");
    await repo.cleanup();
  });

  test("a neutral edit in an UNRELATED file never fires at-risk", async () => {
    const { repo, engine, claimId } = await seed();
    // A file outside the evidence set — not imported, not an include glob.
    await repo.write("src/unrelated.ts", "export const NOISE = 1;\n");
    const report = await engine.check();
    const v = report.verdicts.find((x) => x.assertionId === claimId);
    expect(v?.behavior).toBe("unverified");
    await repo.cleanup();
  });

  test("a changed verifier source ⇒ behavior:at-risk, changedEvidence kind verifier-source", async () => {
    const repo = await makeRepo();
    // A verifier `ref` that is itself a file enters the evidence set, so its
    // baseline hash is captured at record time (§17.6, D14).
    await repo.write("verify-rate.sh", "#!/bin/sh\ntest 5 -eq 5\n");
    await repo.write("src/rate.ts", "export const RATE = 5;\n");
    await repo.write("doc.md", "# Doc\n\nThe rate holds at five.\n");
    const engine = await Engine.open(repo.root);
    const rec = await engine.record({
      docPath: "doc.md",
      docQuote: "The rate holds at five.",
      code: [{ file: "src/rate.ts", quote: "export const RATE = 5;" }],
      behavioral: true,
      verifiers: [{ kind: "command", ref: "verify-rate.sh" }],
      authoredTrust: "verified",
      ref: "PR-1",
      enforcement: "enforced",
    });
    // The verifier source is part of the evidence set's baseline.
    const a = await engine.store.getAssertion(rec.assertion.id);
    expect(Object.keys(a?.evidenceBaseline ?? {})).toContain("verify-rate.sh");

    // Change ONLY the verifier source; the anchored file is byte-identical. Plain
    // check (no --run-verifiers) exercises the evidence-set gate, not execution.
    await repo.write("verify-rate.sh", "#!/bin/sh\ntest 50 -eq 50\n");
    const report = await engine.check();
    const v = report.verdicts.find((x) => x.assertionId === rec.assertion.id);
    expect(v?.code).toBe("unchanged");
    expect(v?.behavior).toBe("at-risk");
    // The change is attributed to its provenance — a verifier source, not a
    // plain import (the honest fix: the gate names *what kind* of evidence moved).
    const vs = v?.evidence.changedEvidence.find(
      (c) => c.path === "verify-rate.sh",
    );
    expect(vs?.kind).toBe("verifier-source");
    expect(vs?.detail).toBe("verifier source changed");
    // at-risk warns, never gates.
    expect(v?.gates).toBe(false);
    await repo.cleanup();
  });
});
