import { afterEach, describe, expect, test } from "bun:test";
import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { resolveRegion } from "../src/engine/record.ts";
import { ClaimStore } from "../src/store/store.ts";
import { makeRepo, record, type TempRepo } from "./helpers.ts";

describe("resolveRegion span parsing", () => {
  const content = "line one\nline two\nline three\n";
  test("a 1-based line range maps to char offsets", () => {
    // Lines 1-2 cover "line one\nline two" (chars 0..17).
    expect(resolveRegion(content, { startLine: 1, endLine: 2 })).toEqual({
      start: 0,
      end: 17,
    });
  });
  test("a quote maps to its first occurrence", () => {
    expect(resolveRegion(content, { quote: "line two" })).toEqual({
      start: 9,
      end: 17,
    });
    expect(() => resolveRegion(content, { quote: "absent" })).toThrow(
      "Quote not found",
    );
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
  test("a line past the end of the file is rejected", () => {
    expect(() => resolveRegion(content, { startLine: 9, endLine: 9 })).toThrow(
      "past the end of the file",
    );
  });
  test("an empty spec is rejected", () => {
    expect(() => resolveRegion(content, {})).toThrow();
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

describe("claim store", () => {
  test("init writes a v3 config with a per-repo nonce", async () => {
    const r = await repo();
    const config = await r.store.config();
    expect(config.nonce).toBe("deadbeef");
    expect(config.version).toBe("v3");
  });

  test("nonces are random hex by default", () => {
    const n = ClaimStore.newNonce();
    expect(n).toMatch(/^[0-9a-f]{8}$/);
    expect(ClaimStore.newNonce()).not.toBe(n);
  });

  test("one file per claim, never a monolithic lockfile", async () => {
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

  test("propositions dedup by content fingerprint", async () => {
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
    expect(props.length).toBe(1);
    expect((await r.store.allAssertions()).length).toBe(2);
  });

  test("re-recording the same sentence on the same document updates the claim in place", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    const first = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const second = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
      verified: true,
    });
    expect(second.assertion.id).toBe(first.assertion.id);
    expect(second.assertion.verified).toBe(true);
    expect((await r.store.allAssertions()).length).toBe(1);
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
    expect(reopened.upgradedFrom).toBeUndefined();
    const loaded = await reopened.getAssertion(assertion.id);
    expect(loaded?.anchor.doc.selectors.length).toBeGreaterThan(0);
    expect(loaded?.anchor.code.length).toBeGreaterThan(0);
    expect(
      loaded?.anchor.code.some((b) =>
        b.selectors.some((s) => s.kind === "ast-node"),
      ),
    ).toBe(true);
    // A precise code span records `enforced` by default.
    expect(loaded?.enforcement).toBe("enforced");
    expect(loaded?.verified).toBe(false);
  });

  test("a line-range code target anchors the whole line", async () => {
    const r = await repo();
    await r.write("src/a.ts", "// header\nexport const A = 1;\n");
    const { assertion } = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      line: 2,
    });
    const tq = assertion.anchor.code[0]?.selectors.find(
      (s) => s.kind === "text-quote",
    );
    expect(tq?.kind === "text-quote" ? tq.exact : "").toBe(
      "export const A = 1;",
    );
  });

  test("open throws when no store exists", async () => {
    await expect(ClaimStore.open("/no/such/store/xyz")).rejects.toThrow();
  });

  test("open refuses an unknown store version", async () => {
    const r = await repo();
    await writeFile(
      join(r.store.dir, "config.json"),
      `${JSON.stringify({ version: "v9", nonce: "deadbeef" })}\n`,
    );
    await expect(ClaimStore.open(r.root)).rejects.toThrow(/model v9/);
  });

  test("a verified record with a precise bidirectional anchor is recorded enforced and verified", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    const { assertion } = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
      verified: true,
    });
    expect(assertion.enforcement).toBe("enforced");
    expect(assertion.verified).toBe(true);
  });

  test("a coarse-only code side records suggested, even when verified", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    const { assertion } = await record(r, {
      doc: "d.md",
      text: "coarse verified",
      file: "src/a.ts",
      verified: true,
      coarse: true,
    });
    expect(assertion.enforcement).toBe("suggested");
    expect(assertion.verified).toBe(true);
  });

  test("an explicit `enforced` over a coarse-only code side is refused", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await expect(
      record(r, {
        doc: "d.md",
        text: "coarse enforced",
        file: "src/a.ts",
        coarse: true,
        enforcement: "enforced",
      }),
    ).rejects.toThrow("an enforced claim needs a precise code span");
  });

  test("a doc quote shorter than 8 characters is refused", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await expect(
      record(r, {
        doc: "d.md",
        text: "A is 1",
        file: "src/a.ts",
        quote: "A = 1",
      }),
    ).rejects.toThrow("too short");
  });
});

describe("v2 to v3 store upgrade", () => {
  async function writeV2Store(root: string) {
    const dir = join(root, ".claims");
    for (const sub of ["documents", "propositions", "claims"]) {
      await mkdir(join(dir, sub), { recursive: true });
    }
    const write = (rel: string, value: unknown) =>
      writeFile(join(dir, rel), `${JSON.stringify(value, null, 2)}\n`);
    await write("config.json", { version: "v2", nonce: "deadbeef" });
    await write("documents/doc_1.json", {
      id: "doc_1",
      path: "README.md",
      lifecycle: "active",
      pristine: false,
      frontmatterStatus: "code:changed",
      edges: [
        { type: "supersedes", target: "doc_0", derived: false },
        { type: "superseded-by", source: "doc_9", derived: true },
      ],
    });
    await write("propositions/prop_1.json", {
      id: "prop_1",
      textCache: "A is one.",
      authoredTrust: "verified",
      fingerprint: "f1",
    });
    await write("claims/asrt_1.json", {
      id: "asrt_1",
      propositionId: "prop_1",
      documentId: "doc_1",
      owner: "tester",
      ref: "r",
      anchor: {
        doc: {
          file: "README.md",
          selectors: [
            { kind: "text-quote", exact: "A is one.", prefix: "", suffix: "" },
            { kind: "text-position", start: 0, end: 9 },
            { kind: "inline-id", id: "hibi:claim:asrt_1" },
          ],
        },
        code: [
          { file: "src/a.ts", selectors: [{ kind: "path", path: "src/a.ts" }] },
        ],
      },
      enforcement: "enforced",
      behavioral: true,
      evidenceBaseline: {},
      verifiers: [{ kind: "command", ref: "bun test", proves: "x" }],
      attrs: { reanchorDowngrade: true, keep: 1 },
    });
    return dir;
  }

  test("open rewrites a v2 store in place and a second open is a no-op", async () => {
    const r = await repo();
    const dir = await writeV2Store(r.root);

    const store = await ClaimStore.open(r.root);
    expect(store.upgradedFrom).toBe("v2");
    expect((await store.config()).version).toBe("v3");
    expect(
      JSON.parse(await readFile(join(dir, "config.json"), "utf8")),
    ).toEqual({
      version: "v3",
      nonce: "deadbeef",
    });

    const a = await store.getAssertion("asrt_1");
    if (!a) throw new Error("upgraded assertion missing");
    expect(a.verified).toBe(true);
    expect(a.enforcement).toBe("enforced");
    expect(a.anchor.code[0]?.selectors).toEqual([
      { kind: "coarse", pattern: "src/a.ts" },
    ]);
    expect(a.anchor.doc.selectors.map((s) => s.kind)).toEqual([
      "text-quote",
      "text-position",
    ]);
    expect(a.verifiers).toEqual([{ kind: "command", ref: "bun test" }]);
    expect(a.attrs).toEqual({ keep: 1 });

    const raw = JSON.parse(
      await readFile(join(dir, "claims/asrt_1.json"), "utf8"),
    );
    expect("behavioral" in raw).toBe(false);
    expect("evidenceBaseline" in raw).toBe(false);

    const p = await store.getProposition("prop_1");
    expect(p).toEqual({
      id: "prop_1",
      textCache: "A is one.",
      fingerprint: "f1",
    });

    const d = await store.getDocument("doc_1");
    expect(d).toEqual({
      id: "doc_1",
      path: "README.md",
      lifecycle: "active",
      edges: [{ type: "supersedes", target: "doc_0" }],
    });

    const before = await readFile(join(dir, "claims/asrt_1.json"), "utf8");
    const again = await ClaimStore.open(r.root);
    expect(again.upgradedFrom).toBeUndefined();
    expect(await readFile(join(dir, "claims/asrt_1.json"), "utf8")).toBe(
      before,
    );
  });

  test("an unverified v2 proposition upgrades to verified false", async () => {
    const r = await repo();
    const dir = await writeV2Store(r.root);
    await writeFile(
      join(dir, "propositions/prop_1.json"),
      `${JSON.stringify({ id: "prop_1", textCache: "A is one.", authoredTrust: "inferred", fingerprint: "f1" })}\n`,
    );
    const store = await ClaimStore.open(r.root);
    expect((await store.getAssertion("asrt_1"))?.verified).toBe(false);
  });
});
