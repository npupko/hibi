import { describe, expect, test } from "bun:test";
import { Engine } from "../src/index.ts";
import { makeRepo } from "./helpers.ts";

/**
 * Reanchor attestation (ADR-002 D15 fitness function): `reanchor` without a
 * `--ref` must never leave `authoredTrust: verified` intact — the anti-gaming
 * rule that closes Fiberplane's relink-to-clear-CI hole.
 */
describe("reanchor attestation (D15)", () => {
  async function recordVerified() {
    const repo = await makeRepo();
    await repo.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await repo.write("doc.md", "# Doc\n\nRetries are capped at 5 attempts.\n");
    const engine = await Engine.open(repo.root);
    const rec = await engine.record({
      docPath: "doc.md",
      docQuote: "Retries are capped at 5 attempts.",
      code: [{ file: "src/retry.ts", quote: "MAX_ATTEMPTS = 5" }],
      authoredTrust: "verified",
      ref: "PR-1",
      enforcement: "enforced",
    });
    return { repo, engine, rec };
  }

  test("reanchor WITHOUT --ref downgrades verified → inferred and records it", async () => {
    const { repo, engine, rec } = await recordVerified();
    const propBefore = await engine.store.getProposition(
      rec.assertion.propositionId,
    );
    expect(propBefore?.authoredTrust).toBe("verified");

    const result = await engine.reanchor(rec.assertion.id, {});
    // The downgrade is surfaced on the result…
    expect(result.reanchorDowngrade).toEqual({
      from: "verified",
      to: "inferred",
      reason: "reanchored without --ref — no re-attestation of truth",
    });
    // …recorded on the assertion's attrs…
    const a = await engine.store.getAssertion(rec.assertion.id);
    expect(a?.attrs.reanchorDowngrade).toBeDefined();
    // …and applied to the (shared) proposition's authored trust.
    const propAfter = await engine.store.getProposition(
      rec.assertion.propositionId,
    );
    expect(propAfter?.authoredTrust).toBe("inferred");
    await repo.cleanup();
  });

  test("reanchor WITH --ref retains verified trust (re-attestation)", async () => {
    const { repo, engine, rec } = await recordVerified();
    const result = await engine.reanchor(rec.assertion.id, { ref: "PR-2" });
    expect(result.reanchorDowngrade).toBeUndefined();
    const prop = await engine.store.getProposition(rec.assertion.propositionId);
    expect(prop?.authoredTrust).toBe("verified");
    const a = await engine.store.getAssertion(rec.assertion.id);
    expect(a?.ref).toBe("PR-2");
    await repo.cleanup();
  });
});
