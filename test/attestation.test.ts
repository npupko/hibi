import { describe, expect, test } from "bun:test";
import { Engine } from "../src/index.ts";
import { makeRepo } from "./helpers.ts";

/**
 * Reanchor attestation (ADR-002 D15 + ADR-003 D25 fitness function): `reanchor`
 * without a `--ref` downgrades `verified → inferred` — the anti-gaming rule that
 * closes Fiberplane's relink-to-clear-CI hole — UNLESS the re-anchor is a
 * provably-evidence-neutral pure move (D25): the doc quote is byte-identical and
 * uniquely re-resolved at similarity 1.0, and the code side is still `unchanged`.
 */
describe("reanchor attestation (D15 + D25)", () => {
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

  test("D25 — a pure move (identical sentence at a new offset, code unchanged) retains verified and writes no downgrade", async () => {
    const { repo, engine, rec } = await recordVerified();
    // Shift the sentence to a new offset without changing it or the code.
    await repo.write(
      "doc.md",
      "# Doc\n\nSome intro about retries and backoff.\n\nRetries are capped at 5 attempts.\n",
    );

    const result = await engine.reanchor(rec.assertion.id, {});
    // Evidence-neutral: no downgrade recorded, trust stays `verified`.
    expect(result.reanchorDowngrade).toBeUndefined();
    const a = await engine.store.getAssertion(rec.assertion.id);
    expect(a?.attrs.reanchorDowngrade).toBeUndefined();
    const prop = await engine.store.getProposition(rec.assertion.propositionId);
    expect(prop?.authoredTrust).toBe("verified");
    await repo.cleanup();
  });

  test("reanchor WITHOUT --ref on a reworded doc downgrades verified → inferred and records it", async () => {
    const { repo, engine, rec } = await recordVerified();
    const propBefore = await engine.store.getProposition(
      rec.assertion.propositionId,
    );
    expect(propBefore?.authoredTrust).toBe("verified");

    // Reword the sentence — the re-resolved doc quote is no longer byte-identical
    // to the stored exact, so the D25 pure-move exception does NOT apply.
    await repo.write(
      "doc.md",
      "# Doc\n\nRetries are capped at 5 attempts total.\n",
    );

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
