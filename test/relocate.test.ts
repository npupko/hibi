/**
 * `relocate` (§9, Tier-1 silent-orphan hardening) — the batch consolidation
 * primitive re-homes every live claim whose documented sentence appears verbatim
 * in the destination, and reports the rest as misses rather than dropping them.
 */

import { afterEach, describe, expect, test } from "bun:test";
import { documentIdForPath } from "../src/engine/record.ts";
import { planRelocation } from "../src/engine/relocate.ts";
import { Engine } from "../src/index.ts";
import { makeRepo, record, type TempRepo } from "./helpers.ts";

let repos: TempRepo[] = [];
async function repo() {
  const r = await makeRepo();
  repos.push(r);
  return r;
}
afterEach(async () => {
  await Promise.all(repos.map((r) => r.cleanup()));
  repos = [];
});

describe("planRelocation (pure planner)", () => {
  test("verbatim sentence → match; absent sentence → miss (never dropped)", () => {
    const plan = planRelocation(
      [
        { claimId: "asrt_a", text: "Capped at 5 attempts." },
        { claimId: "asrt_b", text: "A sentence that is not present." },
        { claimId: "asrt_empty", text: "" },
      ],
      "# New doc\n\nCapped at 5 attempts.\n",
      "b.md",
    );
    expect(plan.matches.map((m) => m.claimId)).toEqual(["asrt_a"]);
    expect(plan.matches[0]?.quote).toBe("Capped at 5 attempts.");
    expect(plan.misses.map((m) => m.claimId).sort()).toEqual([
      "asrt_b",
      "asrt_empty",
    ]);
  });
});

describe("Engine.relocate (§9)", () => {
  test("re-homes a claim whose sentence is copied into the destination", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write("a.md", "# A\n\nCapped at 5 attempts.\n");
    const rec = await record(r, {
      doc: "a.md",
      text: "Capped at 5 attempts.",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });

    // Consolidate the sentence into b.md (a.md still on disk).
    await r.write("b.md", "# B\n\nCapped at 5 attempts.\n");

    const engine = await Engine.open(r.root);
    const result = await engine.relocate("a.md", "b.md");
    expect(result.relocated.map((x) => x.claimId)).toEqual([rec.assertion.id]);
    expect(result.misses).toEqual([]);

    // The claim moved documents, keeping its id, and settles clean.
    const moved = await r.store.getAssertion(rec.assertion.id);
    expect(moved?.documentId).toBe(documentIdForPath("b.md"));
    const report = await engine.check();
    const v = report.verdicts.find((x) => x.assertionId === rec.assertion.id);
    expect(v?.doc).toBe("unchanged");
  });

  test("a claim whose sentence is absent from the destination is a reported miss", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write("a.md", "# A\n\nOnly in A.\n");
    const rec = await record(r, {
      doc: "a.md",
      text: "Only in A.",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });

    await r.write("b.md", "# B\n\nSomething else entirely.\n");
    const engine = await Engine.open(r.root);
    const result = await engine.relocate("a.md", "b.md");
    expect(result.relocated).toEqual([]);
    expect(result.misses.map((m) => m.claimId)).toEqual([rec.assertion.id]);

    // The claim is NOT silently moved — it stays on a.md for a manual fix.
    const still = await r.store.getAssertion(rec.assertion.id);
    expect(still?.documentId).toBe(documentIdForPath("a.md"));
  });

  test("--dry-run previews without writing", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write("a.md", "# A\n\nCapped at 5 attempts.\n");
    const rec = await record(r, {
      doc: "a.md",
      text: "Capped at 5 attempts.",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    await r.write("b.md", "# B\n\nCapped at 5 attempts.\n");

    const engine = await Engine.open(r.root);
    const result = await engine.relocate("a.md", "b.md", { dryRun: true });
    expect(result.dryRun).toBe(true);
    expect(result.relocated.map((x) => x.claimId)).toEqual([rec.assertion.id]);
    // Nothing was persisted: the claim is still on a.md.
    const still = await r.store.getAssertion(rec.assertion.id);
    expect(still?.documentId).toBe(documentIdForPath("a.md"));
  });

  test("--from === --to is rejected, and a missing destination throws", async () => {
    const r = await repo();
    await r.write("a.md", "# A\n");
    const engine = await Engine.open(r.root);
    await expect(engine.relocate("a.md", "a.md")).rejects.toThrow();
    await expect(engine.relocate("a.md", "nope.md")).rejects.toThrow(
      /not found on disk/,
    );
  });
});
