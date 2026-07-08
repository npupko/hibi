import { afterEach, describe, expect, test } from "bun:test";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { resolveRegion } from "../src/engine/record.ts";
import { ClaimStore } from "../src/store/store.ts";
import { makeRepo, record, type TempRepo } from "./helpers.ts";

describe("resolveRegion span parsing (§9)", () => {
  const content = "line one\nline two\nline three\n";
  test("a 1-based line range maps to char offsets (not raw char positions)", () => {
    // Lines 1-2 → "line one\nline two" (chars 0..17), NOT chars 1..2.
    expect(resolveRegion(content, { startLine: 1, endLine: 2 })).toEqual({
      start: 0,
      end: 17,
    });
  });
  test("an explicit char range stays char offsets", () => {
    expect(resolveRegion(content, { start: 0, end: 8 })).toEqual({
      start: 0,
      end: 8,
    });
  });
  test("a malformed (NaN) range is rejected, never silently anchored", () => {
    expect(() =>
      resolveRegion(content, { start: Number.NaN, end: 5 }),
    ).toThrow();
    expect(() =>
      resolveRegion(content, { startLine: Number.NaN, endLine: 2 }),
    ).toThrow();
  });
});

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

describe("claim store (§6, §8)", () => {
  test("init writes a config with a per-repo nonce and a cache .gitignore", async () => {
    const r = await repo();
    const config = await r.store.config();
    expect(config.nonce).toBe("deadbeef");
    expect(config.version).toBe("v2");
    const gi = await readFile(join(r.store.dir, ".gitignore"), "utf8");
    expect(gi).toContain("cache/");
  });

  test("nonces are random hex by default", () => {
    const n = ClaimStore.newNonce();
    expect(n).toMatch(/^[0-9a-f]{8}$/);
    expect(ClaimStore.newNonce()).not.toBe(n); // overwhelmingly likely
  });

  test("one file per claim — never a monolithic lockfile (§6)", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("src/b.ts", "export const B = 2;\n");
    await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
    });
    await record(r, {
      doc: "d.md",
      text: "B is 2 here",
      file: "src/b.ts",
      quote: "B = 2",
    });
    const claimFiles = await readdir(join(r.store.dir, "claims"));
    expect(claimFiles.filter((f) => f.endsWith(".json")).length).toBe(2);
  });

  test("propositions dedup by content fingerprint (§5)", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\nexport const B = 2;\n");
    const r1 = await record(r, {
      doc: "d1.md",
      text: "Same claim text",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const r2 = await record(r, {
      doc: "d2.md",
      text: "Same claim text",
      file: "src/a.ts",
      quote: "B = 2",
    });
    expect(r2.dedupedProposition).toBe(true);
    expect(r2.proposition.id).toBe(r1.proposition.id);
    const props = await r.store.allPropositions();
    expect(props.length).toBe(1); // one proposition, two assertions
    expect((await r.store.allAssertions()).length).toBe(2);
  });

  test("records carry a bidirectional anchor + enforcement that validate on load", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    const { assertion } = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const reopened = await ClaimStore.open(r.root);
    const loaded = await reopened.getAssertion(assertion.id);
    // Bidirectional anchor (§4): a doc-side bundle and ≥1 code-side bundle.
    expect(loaded?.anchor.doc.selectors.length).toBeGreaterThan(0);
    expect(loaded?.anchor.code.length).toBeGreaterThan(0);
    // The precise code side carries the structural (ast-node) selector.
    expect(
      loaded?.anchor.code.some((b) =>
        b.selectors.some((s) => s.kind === "ast-node"),
      ),
    ).toBe(true);
    // Enforcement persists; an inferred claim defaults to advisory `suggested`.
    expect(loaded?.enforcement).toBe("suggested");
  });

  test("open throws when no store exists", async () => {
    await expect(ClaimStore.open("/no/such/store/xyz")).rejects.toThrow();
  });

  test("a verified record with a precise bidirectional anchor is recorded enforced (§9/§10)", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    // Given a precise, resolvable anchor on both sides, `verified` succeeds and
    // is recorded `enforced` — the only enforcement that can gate (§9).
    const { assertion } = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
      trust: "verified",
    });
    expect(assertion.enforcement).toBe("enforced");
  });

  // `verified` trust requires every code target to resolve to a PRECISE span
  // (§9 — "an enforced claim requires both sides to resolve"; coarse path/glob
  // are navigation-only and "never reported as drift", §11.3), so a
  // gating-eligible claim needs a precise code anchor.
  test("verified trust refuses a coarse-only code side (§9/§10)", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await expect(
      record(r, {
        doc: "d.md",
        text: "coarse verified",
        file: "src/a.ts",
        trust: "verified",
        coarse: true,
      }),
    ).rejects.toThrow();
  });

  // The refusal keys off the *resulting* enforcement, not just `verified` trust:
  // an explicit `enforced` override over a coarse-only code side is refused too.
  test("an explicit `enforced` override still requires a precise code anchor (§9)", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await expect(
      record(r, {
        doc: "d.md",
        text: "coarse enforced",
        file: "src/a.ts",
        trust: "inferred",
        coarse: true,
        enforcement: "enforced",
      }),
    ).rejects.toThrow();
  });
});
