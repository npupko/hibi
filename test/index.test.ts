/**
 * The public library facade (src/index.ts): the in-process surface a consumer
 * imports instead of shelling out. Exercises the decoupled store location,
 * the Engine verbs, and the read-only "verdicts as data" path.
 *
 * `record` is span-first: the documented sentence is located by `docQuote` on
 * the doc side and code targets pin the code it describes. A verdict reports a
 * per-side `AnchorState` (`doc`/`code`) plus the `gates`/`expired` flags.
 */
import { afterEach, describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { exists } from "../src/fs.ts";
import {
  type CheckReport,
  ClaimStore,
  documentIdForPath,
  Engine,
} from "../src/index.ts";

let dirs: string[] = [];
async function tmp(): Promise<string> {
  const d = await mkdtemp(join(tmpdir(), "ce-lib-"));
  dirs.push(d);
  return d;
}
afterEach(async () => {
  await Promise.all(dirs.map((d) => rm(d, { recursive: true, force: true })));
  dirs = [];
});

describe("library facade", () => {
  test("store dir decouples from the anchor root", async () => {
    const anchorRoot = await tmp();
    const storeDir = join(await tmp(), "investigation", ".claims");
    await mkdir(join(anchorRoot, "src"), { recursive: true });
    await writeFile(
      join(anchorRoot, "src/retry.ts"),
      "export const MAX_ATTEMPTS = 5;\n",
    );
    await writeFile(
      join(anchorRoot, "README.md"),
      "# Doc\n\nRetries are capped at 5 attempts.\n",
    );

    const engine = await Engine.init(
      { anchorRoot, storeDir },
      { nonce: "deadbeef" },
    );

    expect(engine.store.dir).toBe(storeDir);
    expect(engine.store.anchorRoot).toBe(anchorRoot);
    expect(await exists(join(storeDir, "config.json"))).toBe(true);
    expect(await exists(join(anchorRoot, ".claims"))).toBe(false);
    expect(await ClaimStore.isInitialized({ anchorRoot, storeDir })).toBe(true);

    await engine.record({
      docPath: "README.md",
      docQuote: "Retries are capped at 5 attempts",
      code: [{ file: "src/retry.ts", region: { quote: "MAX_ATTEMPTS = 5" } }],
      verified: true,
      ref: "testref",
    });
    const report = await engine.check();
    expect(report.exitCode).toBe(0);
    expect(report.verdicts).toHaveLength(1);
    expect(report.verdicts[0]?.code).toBe("unchanged");
    expect(report.verdicts[0]?.doc).toBe("unchanged");
  });

  test("check returns verdicts as data by default; stamps a banner only on write", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 5;\n");
    await writeFile(join(anchorRoot, "doc.md"), "# D\n\nN is 5 in code\n");

    const engine = await Engine.init(anchorRoot);
    expect(engine.store.dir).toBe(join(anchorRoot, ".claims"));

    // The default enforcement is `enforced`, so a code change gates (exit 2).
    const rec = await engine.record({
      docPath: "doc.md",
      docQuote: "N is 5 in code",
      code: [{ file: "code.ts", region: { quote: "N = 5" } }],
      ref: "r",
    });
    expect(rec.assertion.enforcement).toBe("enforced");
    expect(rec.assertion.verified).toBe(false);
    expect(rec.warnings).toEqual([]);

    await writeFile(join(anchorRoot, "code.ts"), "export const N = 50;\n");

    const readOnly = await engine.check();
    expect(readOnly.exitCode).toBe(2);
    expect(readOnly.verdicts[0]?.code).toBe("changed");
    expect(readOnly.verdicts[0]?.gates).toBe(true);
    expect(await readFile(join(anchorRoot, "doc.md"), "utf8")).not.toContain(
      "HIBI:BEGIN",
    );

    const written = await engine.check({ write: true });
    expect(written.exitCode).toBe(2);
    expect(await readFile(join(anchorRoot, "doc.md"), "utf8")).toContain(
      "HIBI:BEGIN",
    );
  });

  test("check report keeps the CLI's contract shape", async () => {
    const anchorRoot = await tmp();
    const engine = await Engine.init(anchorRoot);
    const report: CheckReport = await engine.check();
    expect(Object.keys(report).sort()).toEqual([
      "documents",
      "exitCode",
      "ref",
      "summary",
      "verdicts",
    ]);
    expect(Object.keys(report.summary).sort()).toEqual([
      "behavior",
      "clean",
      "code",
      "doc",
      "expired",
      "gating",
      "retired",
      "total",
      "warning",
    ]);
  });

  test("list, check --doc, supersede, archive, retire via the facade", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 5;\n");
    await writeFile(join(anchorRoot, "v1.md"), "# v1\n\nN is 5 in code\n");
    await writeFile(join(anchorRoot, "v2.md"), "# v2\n\nN is 5 in code\n");
    const e = await Engine.init(anchorRoot);

    const rec = await e.record({
      docPath: "v1.md",
      docQuote: "N is 5 in code",
      code: [{ file: "code.ts", region: { quote: "N = 5" } }],
    });

    const rows = await e.list({ path: "code.ts" });
    expect(rows.count).toBe(1);
    expect(rows.claims[0]?.documentPath).toBe("v1.md");
    expect(rows.claims[0]?.side).toBe("code");
    expect(rows.claims[0]?.severity).toBe("clean");

    const scoped = await e.check({ doc: "v1.md" });
    expect(scoped.verdicts).toHaveLength(1);
    expect(scoped.exitCode).toBe(0);

    const sup = await e.supersede({ from: "v1.md", to: "v2.md" });
    expect(sup.oldDoc.lifecycle).toBe("superseded");
    expect(sup.newDoc.edges).toContainEqual({
      type: "supersedes",
      target: sup.oldDoc.id,
    });
    // The sentence appears verbatim in v2.md, so the claim relocates.
    expect(sup.relocated.map((r) => r.claimId)).toEqual([rec.assertion.id]);
    expect(sup.misses).toEqual([]);
    expect(sup.strandedClaims).toEqual([]);
    expect(
      (await e.store.getAssertion(rec.assertion.id))?.anchor.doc.file,
    ).toBe("v2.md");

    const arch = await e.archive("v1.md", "v2.md");
    expect(arch.document.lifecycle).toBe("archived");
    expect(await exists(join(anchorRoot, "archive", "v1.md"))).toBe(true);
    expect(await readFile(join(anchorRoot, "v1.md"), "utf8")).toContain(
      "# Archived",
    );

    const ret = await e.retire(rec.assertion.id);
    expect(ret.assertion.enforcement).toBe("retired");
    expect((await e.retire(rec.assertion.id)).alreadyRetired).toBe(true);
  });

  test("opening a store that was never initialized rejects", async () => {
    await expect(Engine.open(join(await tmp(), "nope"))).rejects.toThrow();
  });

  test("a precise code target on a directory surfaces the real I/O error, unmasked", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "doc.md"), "# D\n\nx is here\n");
    const engine = await Engine.init(anchorRoot);
    // A directory is not "not found": reading it surfaces EISDIR.
    await mkdir(join(anchorRoot, "adir"), { recursive: true });
    await expect(
      engine.record({
        docPath: "doc.md",
        docQuote: "x is here",
        code: [{ file: "adir", region: { quote: "q" } }],
      }),
    ).rejects.toThrow(/EISDIR|directory/);
    // An empty code path resolves to the anchor-root dir, also EISDIR.
    await expect(
      engine.record({
        docPath: "doc.md",
        docQuote: "x is here",
        code: [{ file: "", region: { quote: "q" } }],
      }),
    ).rejects.toThrow(/EISDIR|directory/);
  });

  test("a missing precise code file is an error", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "d.md"), "# D\n\nx is here\n");
    const engine = await Engine.init(anchorRoot);
    await expect(
      engine.record({
        docPath: "d.md",
        docQuote: "x is here",
        code: [{ file: "nope.ts", region: { quote: "q" } }],
      }),
    ).rejects.toThrow("Code file not found on disk: nope.ts");
  });

  test("an enforced claim without a precise code span is refused; suggested allows a coarse one", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "d.md"), "# D\n\nx is here\n");
    const engine = await Engine.init(anchorRoot);
    await expect(
      engine.record({
        docPath: "d.md",
        docQuote: "x is here",
        code: [{ file: "src/**", coarse: true }],
      }),
    ).rejects.toThrow("an enforced claim needs a precise code span");
    const rec = await engine.record({
      docPath: "d.md",
      docQuote: "x is here",
      code: [{ file: "src/**", coarse: true }],
      enforcement: "suggested",
    });
    expect(rec.assertion.enforcement).toBe("suggested");
    expect(rec.assertion.anchor.code[0]?.selectors).toEqual([
      { kind: "coarse", pattern: "src/**" },
    ]);
  });

  test("noAst runs text drift only and still detects a removed span", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 5;\n");
    await writeFile(join(anchorRoot, "doc.md"), "# D\n\nN is 5 in code\n");
    const engine = await Engine.init(anchorRoot, { noAst: true });
    const rec = await engine.record({
      docPath: "doc.md",
      docQuote: "N is 5 in code",
      code: [{ file: "code.ts", region: { quote: "N = 5" } }],
      verified: true,
      ref: "r",
    });
    expect(rec.assertion.anchor.code[0]?.selectors.map((s) => s.kind)).toEqual([
      "text-quote",
      "text-position",
    ]);
    expect((await engine.check()).exitCode).toBe(0);
    await writeFile(join(anchorRoot, "code.ts"), "// removed\n");
    const report = await engine.check();
    expect(report.exitCode).toBe(2);
    expect(report.verdicts[0]?.code).toBe("orphaned");
  });

  test("check --doc is scoped to its own document and does not bleed across docs", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "a.ts"), "export const A = 1;\n");
    await writeFile(join(anchorRoot, "b.ts"), "export const B = 2;\n");
    await writeFile(join(anchorRoot, "a.md"), "# A\n\nA is 1 here\n");
    await writeFile(join(anchorRoot, "b.md"), "# B\n\nB is 2 here\n");
    const e = await Engine.init(anchorRoot);
    await e.record({
      docPath: "a.md",
      docQuote: "A is 1 here",
      code: [{ file: "a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    await e.record({
      docPath: "b.md",
      docQuote: "B is 2 here",
      code: [{ file: "b.ts", region: { quote: "B = 2" } }],
      ref: "r",
    });

    await writeFile(join(anchorRoot, "a.ts"), "// removed\n");

    const a = await e.check({ doc: "a.md" });
    const b = await e.check({ doc: "b.md" });
    expect(a.exitCode).toBe(2);
    expect(a.verdicts).toHaveLength(1);
    expect(a.verdicts[0]?.documentId).toBe(documentIdForPath("a.md"));
    expect(a.documents.map((d) => d.path)).toEqual(["a.md"]);
    expect(b.exitCode).toBe(0);
    expect(b.verdicts).toHaveLength(1);
    expect(b.documents.map((d) => d.path)).toEqual(["b.md"]);
  });
});

describe("reanchor", () => {
  test("reanchor re-points a moved claim back to unchanged", async () => {
    const anchorRoot = await tmp();
    await mkdir(join(anchorRoot, "src"), { recursive: true });
    await writeFile(join(anchorRoot, "src/a.ts"), "export const A = 1;\n");
    await writeFile(
      join(anchorRoot, "README.md"),
      "# D\n\nThe A constant is one.\n",
    );
    const engine = await Engine.init(anchorRoot);
    const rec = await engine.record({
      docPath: "README.md",
      docQuote: "The A constant is one",
      code: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      verified: true,
      ref: "r",
    });

    await writeFile(
      join(anchorRoot, "src/a.ts"),
      "// header\n// header2\nexport const A = 1;\n",
    );
    expect((await engine.check()).verdicts[0]?.code).toBe("moved");

    const res = await engine.reanchor(rec.assertion.id);
    expect(res.code).toBe("unchanged");
    expect(res.doc).toBe("unchanged");
    expect(res.before.code[0]?.quote).toBe("A = 1");
    expect(res.after.code[0]?.quote).toBe("A = 1");
    expect(res.warnings).toEqual([]);
    expect((await engine.check()).verdicts[0]?.code).toBe("unchanged");
  });

  test("reanchor refuses an orphaned side without an explicit span", async () => {
    const anchorRoot = await tmp();
    await mkdir(join(anchorRoot, "src"), { recursive: true });
    await writeFile(join(anchorRoot, "src/a.ts"), "export const A = 1;\n");
    await writeFile(
      join(anchorRoot, "README.md"),
      "# D\n\nThe A constant is one.\n",
    );
    const engine = await Engine.init(anchorRoot);
    const rec = await engine.record({
      docPath: "README.md",
      docQuote: "The A constant is one",
      code: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    // Delete the code file: that side is orphaned.
    await rm(join(anchorRoot, "src/a.ts"));
    await writeFile(join(anchorRoot, "src/b.ts"), "export const B = 2;\n");
    await expect(engine.reanchor(rec.assertion.id)).rejects.toThrow(/orphaned/);
    // An explicit replacement span on that side is accepted.
    const res = await engine.reanchor(rec.assertion.id, {
      code: [{ file: "src/b.ts", region: { quote: "B = 2" } }],
    });
    expect(res.code).toBe("unchanged");
    expect(res.after.code[0]).toEqual({ file: "src/b.ts", quote: "B = 2" });
    expect((await engine.check()).verdicts[0]?.code).toBe("unchanged");
  });

  test("reanchorSuggest lists candidates for both sides", async () => {
    const anchorRoot = await tmp();
    await mkdir(join(anchorRoot, "src"), { recursive: true });
    await writeFile(join(anchorRoot, "src/a.ts"), "export const A = 1;\n");
    await writeFile(
      join(anchorRoot, "README.md"),
      "# D\n\nThe A constant is one.\n",
    );
    const engine = await Engine.init(anchorRoot);
    const rec = await engine.record({
      docPath: "README.md",
      docQuote: "The A constant is one",
      code: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    // Move the code span into a different file of the same language.
    await writeFile(join(anchorRoot, "src/a.ts"), "// moved away\n");
    await writeFile(join(anchorRoot, "src/b.ts"), "export const A = 1;\n");
    const res = await engine.reanchorSuggest(rec.assertion.id);
    expect(res.action).toBe("reanchor-suggest");
    const docHits = res.candidates.filter((c) => c.side === "doc");
    const codeHits = res.candidates.filter((c) => c.side === "code");
    expect(docHits[0]?.file).toBe("README.md");
    expect(codeHits.map((c) => c.file)).toContain("src/b.ts");
    expect(codeHits[0]?.similarity).toBe(1);
  });

  test("reanchor reads the doc with hibi's banner stripped, never re-anchoring onto it", async () => {
    const anchorRoot = await tmp();
    await mkdir(join(anchorRoot, "src"), { recursive: true });
    await writeFile(join(anchorRoot, "src/a.ts"), "export const A = 1;\n");
    await writeFile(
      join(anchorRoot, "README.md"),
      "# D\n\nThe A constant is one.\n",
    );
    const engine = await Engine.init(anchorRoot, { nonce: "deadbeef" });
    const rec = await engine.record({
      docPath: "README.md",
      docQuote: "The A constant is one",
      code: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      verified: true,
      ref: "r",
    });

    await writeFile(join(anchorRoot, "src/a.ts"), "export const A = 2;\n");
    await engine.check({ write: true });
    expect(await readFile(join(anchorRoot, "README.md"), "utf8")).toContain(
      "HIBI:BEGIN",
    );

    const res = await engine.reanchor(rec.assertion.id);
    expect(res.doc).toBe("unchanged");
    expect((await engine.check()).verdicts[0]?.doc).toBe("unchanged");
  });

  test("reanchor --doc re-homes a claim onto a different file, preserving its identity", async () => {
    const anchorRoot = await tmp();
    await mkdir(join(anchorRoot, "src"), { recursive: true });
    await mkdir(join(anchorRoot, "docs"), { recursive: true });
    await writeFile(join(anchorRoot, "src/a.ts"), "export const A = 1;\n");
    await writeFile(
      join(anchorRoot, "wip.md"),
      "# WIP\n\nThe A constant is one.\n",
    );
    await writeFile(
      join(anchorRoot, "docs/a.md"),
      "# A\n\nThe A constant is one.\n",
    );
    const engine = await Engine.init(anchorRoot);
    const rec = await engine.record({
      docPath: "wip.md",
      docQuote: "The A constant is one",
      code: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      verified: true,
      ref: "r",
    });
    const id = rec.assertion.id;
    const beforeDocId = rec.assertion.documentId;

    const res = await engine.reanchor(id, {
      doc: "docs/a.md",
      docQuote: "The A constant is one",
    });
    expect(res.doc).toBe("unchanged");
    expect(res.code).toBe("unchanged");

    const moved = await engine.store.getAssertion(id);
    expect(moved?.id).toBe(id);
    expect(moved?.documentId).not.toBe(beforeDocId);
    expect(moved?.anchor.doc.file).toBe("docs/a.md");
    expect(moved?.anchor.code[0]?.file).toBe("src/a.ts");

    const paths = (await engine.store.allDocuments())
      .map((dc) => dc.path)
      .sort();
    expect(paths).toContain("docs/a.md");
    expect(paths).toContain("wip.md");

    await rm(join(anchorRoot, "wip.md"));
    const verdicts = (await engine.check()).verdicts;
    expect(verdicts).toHaveLength(1);
    expect(verdicts[0]?.doc).toBe("unchanged");
  });

  test("reanchor --doc onto a superseded destination reactivates it", async () => {
    const anchorRoot = await tmp();
    await mkdir(join(anchorRoot, "src"), { recursive: true });
    await mkdir(join(anchorRoot, "docs"), { recursive: true });
    await writeFile(join(anchorRoot, "src/a.ts"), "export const A = 1;\n");
    await writeFile(
      join(anchorRoot, "wip.md"),
      "# WIP\n\nThe A constant is one.\n",
    );
    await writeFile(
      join(anchorRoot, "docs/a.md"),
      "# A\n\nThe A constant is one.\n",
    );
    const engine = await Engine.init(anchorRoot);
    const destId = documentIdForPath("docs/a.md");

    await engine.record({
      docPath: "docs/a.md",
      docQuote: "The A constant is one",
      code: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      verified: true,
      ref: "r",
    });
    const dest = await engine.store.getDocument(destId);
    if (!dest) throw new Error("destination document missing");
    await engine.store.putDocument({ ...dest, lifecycle: "superseded" });

    const rec = await engine.record({
      docPath: "wip.md",
      docQuote: "The A constant is one",
      code: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      verified: true,
      ref: "r",
    });
    await engine.reanchor(rec.assertion.id, {
      doc: "docs/a.md",
      docQuote: "The A constant is one",
    });

    expect((await engine.store.getDocument(destId))?.lifecycle).toBe("active");
  });
});

describe("record duplicate-proposition detection", () => {
  test("existingClaims lists the prior claims sharing a deduped proposition", async () => {
    const root = await tmp();
    await mkdir(join(root, "src"), { recursive: true });
    await writeFile(join(root, "src/a.ts"), "export const A = 1;\n");
    await writeFile(join(root, "a.md"), "# A\n\nThe A constant is one.\n");
    await writeFile(join(root, "b.md"), "# B\n\nThe A constant is one.\n");
    const engine = await Engine.init(root);

    const first = await engine.record({
      docPath: "a.md",
      docQuote: "The A constant is one.",
      code: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      verified: true,
      ref: "r",
    });
    expect(first.existingClaims).toEqual([]);

    const second = await engine.record({
      docPath: "b.md",
      docQuote: "The A constant is one.",
      code: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      verified: true,
      ref: "r",
    });
    expect(second.dedupedProposition).toBe(true);
    expect(second.existingClaims).toEqual([first.assertion.id]);

    const dupes = await engine.list({ state: "duplicate" });
    expect(dupes.count).toBe(2);
  });

  test("a repeated doc quote with disambiguating context records with a warning", async () => {
    const root = await tmp();
    await writeFile(join(root, "src.ts"), "export const A = 1;\n");
    await writeFile(
      join(root, "a.md"),
      "# A\n\nThe A constant is one.\n\nAgain: The A constant is one.\n",
    );
    const engine = await Engine.init(root);
    const rec = await engine.record({
      docPath: "a.md",
      docQuote: "The A constant is one.",
      code: [{ file: "src.ts", region: { quote: "A = 1" } }],
    });
    expect(rec.warnings).toHaveLength(1);
    expect(rec.warnings[0]).toContain("occurs 2 times");
  });
});
