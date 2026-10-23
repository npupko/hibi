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

    const engine = await Engine.init({ anchorRoot, storeDir });

    expect(engine.store.dir).toBe(storeDir);
    expect(engine.store.anchorRoot).toBe(anchorRoot);
    expect(await exists(join(storeDir, "config.json"))).toBe(true);
    expect(await exists(join(anchorRoot, ".claims"))).toBe(false);
    expect(await ClaimStore.isInitialized({ anchorRoot, storeDir })).toBe(true);

    await engine.record({
      kind: "code",
      by: "human:tester@example.com",
      docPath: "README.md",
      docQuote: "Retries are capped at 5 attempts",
      evidence: [
        { file: "src/retry.ts", region: { quote: "MAX_ATTEMPTS = 5" } },
      ],
      ref: "testref",
    });
    const report = await engine.check();
    expect(report.exitCode).toBe(0);
    expect(report.verdicts).toHaveLength(1);
    expect(report.verdicts[0]?.evidence).toBe("unchanged");
    expect(report.verdicts[0]?.doc).toBe("unchanged");
  });

  test("check returns verdicts as data and never writes a document", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 5;\n");
    await writeFile(join(anchorRoot, "doc.md"), "# D\n\nN is 5 in code\n");

    const engine = await Engine.init(anchorRoot);
    expect(engine.store.dir).toBe(join(anchorRoot, ".claims"));

    // The default enforcement is `enforced`, so a code change gates (exit 2).
    const rec = await engine.record({
      kind: "code",
      by: "human:tester@example.com",
      docPath: "doc.md",
      docQuote: "N is 5 in code",
      evidence: [{ file: "code.ts", region: { quote: "N = 5" } }],
      ref: "r",
    });
    expect(rec.claim.enforcement).toBe("enforced");
    expect(rec.warnings).toEqual([]);

    await writeFile(join(anchorRoot, "code.ts"), "export const N = 50;\n");

    const readOnly = await engine.check();
    expect(readOnly.exitCode).toBe(2);
    expect(readOnly.verdicts[0]?.evidence).toBe("changed");
    expect(readOnly.verdicts[0]?.gates).toBe(true);
    expect(await readFile(join(anchorRoot, "doc.md"), "utf8")).toBe(
      "# D\n\nN is 5 in code\n",
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
      "worklist",
    ]);
    expect(Object.keys(report.summary).sort()).toEqual([
      "clean",
      "conflictPass",
      "conflicts",
      "doc",
      "evidence",
      "expired",
      "gating",
      "judge",
      "retired",
      "total",
      "warning",
      "worklist",
    ]);
    expect(report.summary.conflictPass).toEqual({
      pairs: 0,
      requests: 0,
      pending: 0,
      scope: "store",
    });
  });

  test("list, check --doc, supersede, archive, retire via the facade", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 5;\n");
    await writeFile(join(anchorRoot, "v1.md"), "# v1\n\nN is 5 in code\n");
    await writeFile(join(anchorRoot, "v2.md"), "# v2\n\nN is 5 in code\n");
    const e = await Engine.init(anchorRoot);

    const rec = await e.record({
      kind: "code",
      by: "human:tester@example.com",
      docPath: "v1.md",
      docQuote: "N is 5 in code",
      evidence: [{ file: "code.ts", region: { quote: "N = 5" } }],
    });

    const rows = await e.list({ path: "code.ts" });
    expect(rows.count).toBe(1);
    expect(rows.claims[0]?.documentId).toBe(rec.document.id);
    expect(rows.claims[0]?.side).toBe("evidence");
    expect(rows.claims[0]?.state).toBe("clean");

    const scoped = await e.check({ doc: "v1.md" });
    expect(scoped.verdicts).toHaveLength(1);
    expect(scoped.exitCode).toBe(0);

    const sup = await e.supersede({ from: "v1.md", to: "v2.md" });
    expect(sup.oldDoc.lifecycle).toBe("superseded");
    expect(sup.oldDoc.supersededBy).toBe(sup.newDoc.id);
    // The sentence appears verbatim in v2.md, so the claim relocates.
    expect(sup.relocated.map((r) => r.claimId)).toEqual([rec.claim.id]);
    expect(sup.misses).toEqual([]);
    expect(sup.strandedClaims).toEqual([]);
    expect((await e.store.getClaim(rec.claim.id))?.doc.file).toBe("v2.md");

    const arch = await e.archive("v1.md", "v2.md");
    expect(arch.document.lifecycle).toBe("archived");
    expect(await exists(join(anchorRoot, "archive", "v1.md"))).toBe(true);
    expect(await readFile(join(anchorRoot, "v1.md"), "utf8")).toContain(
      "# Archived",
    );

    const ret = await e.retire(rec.claim.id);
    expect(ret.claim.enforcement).toBe("retired");
    expect((await e.retire(rec.claim.id)).alreadyRetired).toBe(true);
  });

  test("the worklist leaves out claims on a superseded document", async () => {
    const anchorRoot = await tmp();
    await writeFile(
      join(anchorRoot, "v1.md"),
      "# v1\n\nPro costs $29/month.\n",
    );
    await writeFile(join(anchorRoot, "v2.md"), "# v2\n\nPricing moved.\n");
    const e = await Engine.init(anchorRoot);
    const rec = await e.record({
      kind: "attested",
      by: "human:tester@example.com",
      docPath: "v1.md",
      docQuote: "Pro costs $29/month.",
      ttl: "P90D",
      sources: [{ id: "stripe", resource: "https://stripe.example/p" }],
    });
    expect((await e.check()).worklist.map((w) => w.claimId)).toEqual([
      rec.claim.id,
    ]);
    // supersede moves every live claim off v1.md; a claim recorded on v1.md
    // after that is left on the superseded document.
    await e.supersede({ from: "v1.md", to: "v2.md" });
    await e.retire(rec.claim.id);
    const left = await e.record({
      kind: "attested",
      by: "human:tester@example.com",
      docPath: "v1.md",
      docQuote: "Pro costs $29/month.",
      ttl: "P90D",
      sources: [{ id: "stripe", resource: "https://stripe.example/p" }],
    });
    expect((await e.store.getDocument(left.document.id))?.lifecycle).toBe(
      "superseded",
    );
    const after = await e.check();
    expect(after.worklist).toEqual([]);
    expect((await e.list({ state: "stranded" })).count).toBe(1);
    expect((await e.list({ state: "pending" })).count).toBe(0);
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
        kind: "code",
        by: "human:tester@example.com",
        docPath: "doc.md",
        docQuote: "x is here",
        evidence: [{ file: "adir", region: { quote: "q" } }],
      }),
    ).rejects.toThrow(/EISDIR|directory/);
    // An empty code path resolves to the anchor-root dir, also EISDIR.
    await expect(
      engine.record({
        kind: "code",
        by: "human:tester@example.com",
        docPath: "doc.md",
        docQuote: "x is here",
        evidence: [{ file: "", region: { quote: "q" } }],
      }),
    ).rejects.toThrow(/EISDIR|directory/);
  });

  test("a missing precise code file is an error", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "d.md"), "# D\n\nx is here\n");
    const engine = await Engine.init(anchorRoot);
    await expect(
      engine.record({
        kind: "code",
        by: "human:tester@example.com",
        docPath: "d.md",
        docQuote: "x is here",
        evidence: [{ file: "nope.ts", region: { quote: "q" } }],
      }),
    ).rejects.toThrow("Evidence file not found on disk: nope.ts");
  });

  test("noAst runs text drift only and still detects a removed span", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 5;\n");
    await writeFile(join(anchorRoot, "doc.md"), "# D\n\nN is 5 in code\n");
    const engine = await Engine.init(anchorRoot, { noAst: true });
    const rec = await engine.record({
      kind: "code",
      by: "human:tester@example.com",
      docPath: "doc.md",
      docQuote: "N is 5 in code",
      evidence: [{ file: "code.ts", region: { quote: "N = 5" } }],
      ref: "r",
    });
    expect(rec.claim.evidence[0]?.selectors.map((s) => s.kind)).toEqual([
      "text-quote",
      "text-position",
    ]);
    expect((await engine.check()).exitCode).toBe(0);
    await writeFile(join(anchorRoot, "code.ts"), "// removed\n");
    const report = await engine.check();
    expect(report.exitCode).toBe(2);
    expect(report.verdicts[0]?.evidence).toBe("orphaned");
  });

  test("check --doc is scoped to its own document and does not bleed across docs", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "a.ts"), "export const A = 1;\n");
    await writeFile(join(anchorRoot, "b.ts"), "export const B = 2;\n");
    await writeFile(join(anchorRoot, "a.md"), "# A\n\nA is 1 here\n");
    await writeFile(join(anchorRoot, "b.md"), "# B\n\nB is 2 here\n");
    const e = await Engine.init(anchorRoot);
    await e.record({
      kind: "code",
      by: "human:tester@example.com",
      docPath: "a.md",
      docQuote: "A is 1 here",
      evidence: [{ file: "a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    await e.record({
      kind: "code",
      by: "human:tester@example.com",
      docPath: "b.md",
      docQuote: "B is 2 here",
      evidence: [{ file: "b.ts", region: { quote: "B = 2" } }],
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
      kind: "code",
      by: "human:tester@example.com",
      docPath: "README.md",
      docQuote: "The A constant is one",
      evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });

    await writeFile(
      join(anchorRoot, "src/a.ts"),
      "// header\n// header2\nexport const A = 1;\n",
    );
    expect((await engine.check()).verdicts[0]?.evidence).toBe("moved");

    const res = await engine.reanchor(rec.claim.id);
    expect(res.evidence).toBe("unchanged");
    expect(res.doc).toBe("unchanged");
    expect(res.before.evidence[0]?.quote).toBe("A = 1");
    expect(res.after.evidence[0]?.quote).toBe("A = 1");
    expect(res.warnings).toEqual([]);
    expect((await engine.check()).verdicts[0]?.evidence).toBe("unchanged");
  });

  test("reanchor leaves an orphaned side as stored without an explicit span", async () => {
    const anchorRoot = await tmp();
    await mkdir(join(anchorRoot, "src"), { recursive: true });
    await writeFile(join(anchorRoot, "src/a.ts"), "export const A = 1;\n");
    await writeFile(
      join(anchorRoot, "README.md"),
      "# D\n\nThe A constant is one.\n",
    );
    const engine = await Engine.init(anchorRoot);
    const rec = await engine.record({
      kind: "code",
      by: "human:tester@example.com",
      docPath: "README.md",
      docQuote: "The A constant is one",
      evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    // Delete the code file: that side is orphaned.
    await rm(join(anchorRoot, "src/a.ts"));
    await writeFile(join(anchorRoot, "src/b.ts"), "export const B = 2;\n");
    const left = await engine.reanchor(rec.claim.id);
    expect(left.relocated).toBe(0);
    expect(left.kept.map((k) => k.reason)).toEqual([
      "evidence: orphaned; pass --code-quote to re-anchor",
    ]);
    // An explicit replacement span on that side is accepted.
    const res = await engine.reanchor(rec.claim.id, {
      evidence: [{ file: "src/b.ts", region: { quote: "B = 2" } }],
    });
    expect(res.evidence).toBe("unchanged");
    expect(res.after.evidence[0]).toEqual({ file: "src/b.ts", quote: "B = 2" });
    expect((await engine.check()).verdicts[0]?.evidence).toBe("unchanged");
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
      kind: "code",
      by: "human:tester@example.com",
      docPath: "README.md",
      docQuote: "The A constant is one",
      evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    // Move the code span into a different file of the same language.
    await writeFile(join(anchorRoot, "src/a.ts"), "// moved away\n");
    await writeFile(join(anchorRoot, "src/b.ts"), "export const A = 1;\n");
    const res = await engine.reanchorSuggest(rec.claim.id);
    expect(res.action).toBe("reanchor-suggest");
    const docHits = res.candidates.filter((c) => c.side === "doc");
    const codeHits = res.candidates.filter((c) => c.side === "evidence");
    expect(docHits[0]?.file).toBe("README.md");
    expect(codeHits.map((c) => c.file)).toContain("src/b.ts");
    expect(codeHits[0]?.similarity).toBe(1);
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
      kind: "code",
      by: "human:tester@example.com",
      docPath: "wip.md",
      docQuote: "The A constant is one",
      evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    const id = rec.claim.id;
    const beforeDocId = rec.claim.documentId;

    const res = await engine.reanchor(id, {
      doc: "docs/a.md",
      docQuote: "The A constant is one",
    });
    expect(res.doc).toBe("unchanged");
    expect(res.evidence).toBe("unchanged");

    const moved = await engine.store.getClaim(id);
    expect(moved?.id).toBe(id);
    expect(moved?.documentId).not.toBe(beforeDocId);
    expect(moved?.doc.file).toBe("docs/a.md");
    expect(moved?.evidence[0]?.file).toBe("src/a.ts");

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
      kind: "code",
      by: "human:tester@example.com",
      docPath: "docs/a.md",
      docQuote: "The A constant is one",
      evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    const dest = await engine.store.getDocument(destId);
    if (!dest) throw new Error("destination document missing");
    await engine.store.putDocument({ ...dest, lifecycle: "superseded" });

    const rec = await engine.record({
      kind: "code",
      by: "human:tester@example.com",
      docPath: "wip.md",
      docQuote: "The A constant is one",
      evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    await engine.reanchor(rec.claim.id, {
      doc: "docs/a.md",
      docQuote: "The A constant is one",
    });

    expect((await engine.store.getDocument(destId))?.lifecycle).toBe("active");
  });
});

describe("record: the same sentence in another document", () => {
  test("existingClaims lists the prior claims with the same normalized sentence", async () => {
    const root = await tmp();
    await mkdir(join(root, "src"), { recursive: true });
    await writeFile(join(root, "src/a.ts"), "export const A = 1;\n");
    await writeFile(join(root, "a.md"), "# A\n\nThe A constant is one.\n");
    await writeFile(join(root, "b.md"), "# B\n\nThe A constant is one.\n");
    const engine = await Engine.init(root);

    const first = await engine.record({
      kind: "code",
      by: "human:tester@example.com",
      docPath: "a.md",
      docQuote: "The A constant is one.",
      evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    expect(first.existingClaims).toEqual([]);

    const second = await engine.record({
      kind: "code",
      by: "human:tester@example.com",
      docPath: "b.md",
      docQuote: "The A constant is one.",
      evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      ref: "r",
    });
    expect(second.updated).toBe(false);
    expect(second.existingClaims).toEqual([first.claim.id]);
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
      kind: "code",
      by: "human:tester@example.com",
      docPath: "a.md",
      docQuote: "The A constant is one.",
      evidence: [{ file: "src.ts", region: { quote: "A = 1" } }],
    });
    expect(rec.warnings).toHaveLength(1);
    expect(rec.warnings[0]).toContain("occurs 2 times");
  });
});

describe("recordBatch: all or nothing", () => {
  /** A repo with four documented constants and one recorded claim on the first. */
  async function batchRepo() {
    const root = await tmp();
    await mkdir(join(root, "src"), { recursive: true });
    await writeFile(
      join(root, "src/conf.ts"),
      "export const A = 1;\nexport const B = 2;\nexport const C = 3;\nexport const D = 4;\n",
    );
    await writeFile(
      join(root, "conf.md"),
      "# Conf\n\nThe A constant is one.\nThe B constant is two.\nThe C constant is three.\nThe D constant is four.\n",
    );
    const engine = await Engine.init(root);
    const call = (word: string, name: string, value: number) => ({
      kind: "code" as const,
      by: "agent:batch",
      docPath: "conf.md",
      docQuote: `The ${name} constant is ${word}.`,
      evidence: [
        { file: "src/conf.ts", region: { quote: `${name} = ${value}` } },
      ],
    });
    const calls = {
      a: call("one", "A", 1),
      b: call("two", "B", 2),
      c: call("three", "C", 3),
      d: call("four", "D", 4),
    };
    const a = await engine.record(calls.a);
    const claimFile = (id: string) =>
      join(engine.store.dir, "claims", `${id}.json`);
    return { root, engine, calls, a, claimFile };
  }

  test("a failure while applying rolls back this batch's writes and leaves another writer's records alone", async () => {
    const { root, engine, calls, a, claimFile } = await batchRepo();
    const aBefore = await readFile(claimFile(a.claim.id), "utf8");
    const other = await Engine.open(root);
    const theirs = new Map<string, string>();

    // Item 2 fails after validation; just before it, another process records
    // D and re-records A over the batch's update of A.
    const original = engine.record.bind(engine);
    let applied = 0;
    engine.record = async (call, opts) => {
      if (opts?.dryRun) return original(call, opts);
      if (applied++ === 2) {
        for (const c of [calls.d, calls.a]) {
          const id = (await other.record(c)).claim.id;
          theirs.set(id, await readFile(claimFile(id), "utf8"));
        }
        throw new Error("disk full");
      }
      return original(call, opts);
    };
    const res = await engine.recordBatch([calls.a, calls.b, calls.c]);
    expect(res).toEqual({
      results: [],
      failures: [{ index: 2, message: "disk full" }],
    });

    // B, created by the batch, is gone. D and the other writer's A are
    // byte-for-byte what that writer wrote.
    const claims = await engine.store.allClaims();
    expect(claims.map((c) => c.sentence).sort()).toEqual([
      "The A constant is one.",
      "The D constant is four.",
    ]);
    expect(claims.map((c) => c.id).sort()).toEqual([...theirs.keys()].sort());
    for (const [id, text] of theirs) {
      expect(await readFile(claimFile(id), "utf8")).toBe(text);
    }
    expect(theirs.get(a.claim.id)).not.toBe(aBefore);
  });

  test("a batch overwrite is restored to its previous text", async () => {
    const { engine, calls, a, claimFile } = await batchRepo();
    const aBefore = await readFile(claimFile(a.claim.id), "utf8");
    const original = engine.record.bind(engine);
    let applied = 0;
    engine.record = async (call, opts) => {
      if (!opts?.dryRun && applied++ === 1) throw new Error("disk full");
      return original(call, opts);
    };
    const res = await engine.recordBatch([calls.a, calls.b]);
    expect(res.failures).toEqual([{ index: 1, message: "disk full" }]);
    expect(await readFile(claimFile(a.claim.id), "utf8")).toBe(aBefore);
    expect((await engine.store.allClaims()).map((c) => c.id)).toEqual([
      a.claim.id,
    ]);
  });

  test("span failures are caught before anything is written", async () => {
    const { engine, calls } = await batchRepo();
    let writes = 0;
    const putClaim = engine.store.putClaim.bind(engine.store);
    engine.store.putClaim = async (c) => {
      writes++;
      return putClaim(c);
    };
    const res = await engine.recordBatch([
      calls.b,
      { ...calls.c, docQuote: "Not in the document at all." },
      {
        ...calls.d,
        evidence: [{ file: "src/missing.ts", region: { quote: "D = 4" } }],
      },
      {
        ...calls.d,
        evidence: [{ file: "src/conf.ts", region: { quote: "D = 44" } }],
      },
    ]);
    expect(res.failures.map((f) => f.index)).toEqual([1, 2, 3]);
    expect(res.failures[1]?.message).toContain("src/missing.ts");
    expect(writes).toBe(0);
  });
});

describe("user-supplied paths are normalized to the store's form", () => {
  async function seeded() {
    const root = await tmp();
    await mkdir(join(root, "docs"), { recursive: true });
    await mkdir(join(root, "src"), { recursive: true });
    await writeFile(join(root, "src/a.ts"), "export const A = 1;\n");
    await writeFile(join(root, "docs/a.md"), "# A\n\nA is 1 here, always.\n");
    await writeFile(join(root, "docs/b.md"), "# B\n\nA is 1 here, always.\n");
    const e = await Engine.init(root, { noAst: true });
    return { root, e };
  }

  test("./docs/a.md, docs/a.md, and the absolute path are one document", async () => {
    const { root, e } = await seeded();
    const rec = await e.record({
      kind: "code",
      by: "human:t@example.com",
      docPath: "./docs/a.md",
      docQuote: "A is 1 here, always.",
      evidence: [{ file: "./src/../src/a.ts", region: { quote: "A = 1" } }],
    });
    expect(rec.document.path).toBe("docs/a.md");
    expect(rec.claim.doc.file).toBe("docs/a.md");
    expect(rec.claim.evidence[0]?.file).toBe("src/a.ts");
    expect(rec.document.id).toBe(documentIdForPath("docs/a.md"));
    // Re-recording by the absolute path updates the same claim.
    const again = await e.record({
      kind: "code",
      by: "human:t@example.com",
      docPath: join(root, "docs/a.md"),
      docQuote: "A is 1 here, always.",
      evidence: [{ file: join(root, "src/a.ts"), region: { quote: "A = 1" } }],
    });
    expect(again.updated).toBe(true);
    expect(again.claim.id).toBe(rec.claim.id);

    for (const doc of ["docs/a.md", "./docs/a.md", join(root, "docs/a.md")]) {
      const report = await e.check({ doc });
      expect(report.verdicts.map((v) => v.claimId)).toEqual([rec.claim.id]);
    }
    for (const path of ["src/a.ts", "./src/a.ts", join(root, "src/a.ts")]) {
      expect((await e.list({ path })).claims.map((c) => c.claimId)).toEqual([
        rec.claim.id,
      ]);
    }
    const audit = await e.audit("./docs/a.md");
    expect(audit.doc).toBe("docs/a.md");
    expect(audit.rows.some((r) => r.claimId === rec.claim.id)).toBe(true);

    const moved = await e.reanchor(rec.claim.id, {
      doc: "./docs/b.md",
      docQuote: "A is 1 here, always.",
    });
    expect(moved.claim.doc.file).toBe("docs/b.md");
    const sup = await e.supersede({
      from: "./docs/b.md",
      to: join(root, "docs/a.md"),
      dryRun: true,
    });
    expect([sup.oldDoc.path, sup.newDoc.path]).toEqual([
      "docs/b.md",
      "docs/a.md",
    ]);
    const arch = await e.archive(join(root, "docs/b.md"), "./docs/a.md", {
      dryRun: true,
    });
    expect(arch.document.path).toBe("docs/b.md");
    expect(arch.successor).toBe("docs/a.md");
  });

  test("normalizePath keeps an alias prefix and normalizes the rest", async () => {
    const { normalizePath } = await import("../src/fs.ts");
    expect(normalizePath("/r", "api:./src/x.ts")).toBe("api:src/x.ts");
    expect(normalizePath("/r", "api:src/../lib/x.ts")).toBe("api:lib/x.ts");
    expect(normalizePath("/r", "./docs/a.md")).toBe("docs/a.md");
    expect(normalizePath("/r", "/r/docs/a.md")).toBe("docs/a.md");
    expect(normalizePath("/r", "/elsewhere/a.md")).toBe("../elsewhere/a.md");
  });

  test("check --doc on a path with no registered claims throws", async () => {
    const { e } = await seeded();
    await expect(e.check({ doc: "./docs/a.md" })).rejects.toThrow(
      "no claims are registered for docs/a.md; run hibi audit --doc docs/a.md",
    );
  });

  test("check --doc on a document whose claims are all retired throws the same error", async () => {
    const { e } = await seeded();
    const rec = await e.record({
      kind: "code",
      by: "human:t@example.com",
      docPath: "docs/a.md",
      docQuote: "A is 1 here, always.",
      evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
    });
    expect((await e.check({ doc: "docs/a.md" })).exitCode).toBe(0);
    await e.retire(rec.claim.id);
    await expect(e.check({ doc: "docs/a.md" })).rejects.toThrow(
      "no claims are registered for docs/a.md; run hibi audit --doc docs/a.md",
    );
  });
});

describe("evidence file containment", () => {
  test("record refuses a file outside the anchor root or outside its alias root", async () => {
    const outer = await tmp();
    const root = join(outer, "repo");
    await mkdir(join(root, "docs"), { recursive: true });
    await mkdir(join(outer, "api", "src"), { recursive: true });
    await writeFile(join(outer, "secret.ts"), "export const S = 1;\n");
    await writeFile(join(outer, "api", "src", "x.ts"), "export const X = 1;\n");
    await writeFile(join(root, "docs/a.md"), "# A\n\nS is 1 in that file.\n");
    const e = await Engine.init(root, { noAst: true });
    const cfgPath = join(e.store.dir, "config.json");
    const cfg = JSON.parse(await readFile(cfgPath, "utf8"));
    cfg.repos = { api: "../api" };
    await writeFile(cfgPath, JSON.stringify(cfg));
    const e2 = await Engine.open(root, { noAst: true });
    const call = (file: string) =>
      e2.record({
        kind: "code",
        by: "human:t@example.com",
        docPath: "docs/a.md",
        docQuote: "S is 1 in that file.",
        evidence: [{ file, region: { quote: "= 1" } }],
      });
    await expect(call("../secret.ts")).rejects.toThrow(
      "../secret.ts: path escapes its root; an evidence file must be inside the anchor root.",
    );
    await expect(call("api:../secret.ts")).rejects.toThrow(
      'api:../secret.ts: path escapes its root; an evidence file must be inside the directory of repo alias "api".',
    );
    // Inside the alias root is fine.
    const ok = await call("api:src/x.ts");
    expect(ok.claim.evidence[0]?.file).toBe("api:src/x.ts");
  });

  test("check reports an escaping stored evidence path as unavailable", async () => {
    const outer = await tmp();
    const root = join(outer, "repo");
    await mkdir(join(root, "docs"), { recursive: true });
    await mkdir(join(root, "src"), { recursive: true });
    await writeFile(join(outer, "secret.ts"), "export const S = 1;\n");
    await writeFile(join(root, "src/s.ts"), "export const S = 1;\n");
    await writeFile(join(root, "docs/a.md"), "# A\n\nS is 1 in that file.\n");
    const e = await Engine.init(root, { noAst: true });
    const rec = await e.record({
      kind: "code",
      by: "human:t@example.com",
      docPath: "docs/a.md",
      docQuote: "S is 1 in that file.",
      evidence: [{ file: "src/s.ts", region: { quote: "S = 1" } }],
    });
    // A hand-edited store pointing outside the root.
    await e.store.putClaim({
      ...rec.claim,
      evidence: rec.claim.evidence.map((b) => ({ ...b, file: "../secret.ts" })),
    });
    const report = await e.check();
    expect(report.verdicts[0]?.evidence).toBe("unavailable");
    expect(report.verdicts[0]?.reasons).toContain(
      "evidence: path escapes its root",
    );
    expect(report.verdicts[0]?.gates).toBe(false);
  });
});

describe("reanchor across repos and kinds", () => {
  test("a sibling repo that is not checked out keeps its bundle, with a warning", async () => {
    const outer = await tmp();
    const root = join(outer, "repo");
    await mkdir(join(root, "docs"), { recursive: true });
    await mkdir(join(outer, "api", "src"), { recursive: true });
    await writeFile(join(outer, "api", "src", "x.ts"), "export const X = 1;\n");
    await writeFile(join(root, "docs/a.md"), "# A\n\nX is 1 over there.\n");
    let e = await Engine.init(root, { noAst: true });
    const cfgPath = join(e.store.dir, "config.json");
    const cfg = JSON.parse(await readFile(cfgPath, "utf8"));
    cfg.repos = { api: "../api" };
    await writeFile(cfgPath, JSON.stringify(cfg));
    e = await Engine.open(root, { noAst: true });
    const rec = await e.record({
      kind: "code",
      by: "human:t@example.com",
      docPath: "docs/a.md",
      docQuote: "X is 1 over there.",
      evidence: [{ file: "api:src/x.ts", region: { quote: "X = 1" } }],
    });
    await rm(join(outer, "api"), { recursive: true, force: true });
    await writeFile(
      join(root, "docs/a.md"),
      "# A\n\nIntro line.\n\nX is 1 over there.\n",
    );
    const res = await e.reanchor(rec.claim.id);
    expect(res.warnings).toEqual([
      'evidence side api:src/x.ts kept unchanged: sibling repo "api" is not checked out at ../api',
    ]);
    expect(res.evidence).toBe("unavailable");
    expect(res.doc).toBe("unchanged");
    expect(res.claim.evidence).toEqual(rec.claim.evidence);
  });

  test("--ref-* replaces a reference span; --code-* on a reference claim and --ref-* on a code claim are errors", async () => {
    const root = await tmp();
    await mkdir(join(root, "docs"), { recursive: true });
    await mkdir(join(root, "src"), { recursive: true });
    await writeFile(join(root, "src/a.ts"), "export const A = 1;\n");
    await writeFile(
      join(root, "docs/a.md"),
      "# A\n\nRefunds take two weeks.\n\nA is 1 here, always.\n",
    );
    await writeFile(
      join(root, "docs/billing.md"),
      "# Billing\n\nRefunds are issued within 14 days.\n",
    );
    const e = await Engine.init(root, { noAst: true });
    const ref = await e.record({
      kind: "reference",
      by: "human:t@example.com",
      docPath: "docs/a.md",
      docQuote: "Refunds take two weeks.",
      evidence: [
        {
          file: "docs/billing.md",
          region: { quote: "Refunds are issued within 14 days." },
        },
      ],
    });
    const code = await e.record({
      kind: "code",
      by: "human:t@example.com",
      docPath: "docs/a.md",
      docQuote: "A is 1 here, always.",
      evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
    });
    await writeFile(
      join(root, "docs/billing.md"),
      "# Billing\n\nWe refund within 14 days.\n",
    );
    const out = await e.reanchor(ref.claim.id, {
      reference: {
        file: "./docs/billing.md",
        region: { quote: "We refund within 14 days." },
      },
    });
    expect(out.evidence).toBe("unchanged");
    expect(out.after.evidence).toEqual([
      { file: "docs/billing.md", quote: "We refund within 14 days." },
    ]);
    await expect(
      e.reanchor(ref.claim.id, {
        evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
      }),
    ).rejects.toThrow(
      `claim ${ref.claim.id} is a reference claim; --code-file, --code-quote, and --code-range are for code claims; pass --ref-file with --ref-quote or --ref-range.`,
    );
    await expect(
      e.reanchor(code.claim.id, {
        reference: { file: "docs/billing.md", region: { quote: "We refund" } },
      }),
    ).rejects.toThrow(
      `claim ${code.claim.id} is a code claim; --ref-file, --ref-quote, and --ref-range are for reference claims.`,
    );
  });
});

describe("conflict pass scope", () => {
  test("audit and check --doc judge only pairs with a claim in the document; whole-store check judges all", async () => {
    const root = await tmp();
    await mkdir(join(root, "docs"), { recursive: true });
    const write = (p: string, t: string) => writeFile(join(root, p), t);
    await write("docs/a.md", "# A\n\nZebra pricing is $10.\n");
    await write("docs/x.md", "# X\n\nQuokka pricing is $20.\n");
    await write("docs/y.md", "# Y\n\nQuokka pricing is $30.\n");
    const NARRATIVE = {
      factual_claim: 0.02,
      instruction: 0.02,
      opinion: 0.02,
      narrative: 0.94,
    };
    const { FakeJudge, conflictAnswer } = await import("../src/judge/fake.ts");
    const judge = new FakeJudge({
      triage: () => ({
        kind: "narrative",
        kindProbabilities: NARRATIVE,
        kindConfidence: 0.92,
      }),
      conflict: () =>
        conflictAnswer(
          { consistent: 0.1, contradictory: 0.1, unrelated: 0.8 },
          0.1,
          0.1,
        ),
    });
    const e = await Engine.init(root, { judge, noAst: true });
    for (const [doc, quote] of [
      ["docs/a.md", "Zebra pricing is $10."],
      ["docs/x.md", "Quokka pricing is $20."],
      ["docs/y.md", "Quokka pricing is $30."],
    ] as const) {
      await e.record({
        kind: "attested",
        by: "human:t@example.com",
        docPath: doc,
        docQuote: quote,
        ttl: "P90D",
      });
    }
    const conflicts = () =>
      judge.calls.filter((c) => c.judgment === "conflict").length;

    await e.audit("docs/a.md");
    const afterAudit = conflicts();
    const scoped = await e.check({ doc: "docs/a.md" });
    expect(scoped.summary.conflictPass.scope).toBe("doc");
    // a.md pairs with x.md and y.md through the money group and `pricing`.
    expect(scoped.summary.conflictPass.pairs).toBe(2);
    const whole = await e.check();
    expect(whole.summary.conflictPass.scope).toBe("store");
    expect(whole.summary.conflictPass.pairs).toBe(3);
    // The x/y pair was first judged by the whole-store check.
    expect(afterAudit).toBe(2);
    expect(conflicts()).toBe(3);
  });
});
