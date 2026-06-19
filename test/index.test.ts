/**
 * The public library facade (src/index.ts): the in-process surface a consumer
 * (e.g. atlas) imports instead of shelling out. Exercises the decoupled store
 * location, the Engine verbs, and the read-only "verdicts as data" path.
 *
 * Bound to the two-axis model (ADR-001): `record` is span-first (the documented
 * sentence is located by `docQuote` on the doc side; code targets pin the code
 * it describes), and a verdict reports a per-side `AnchorState` (`doc`/`code`)
 * plus the `gates`/`expired` flags — never a single rollup `state`.
 */
import { afterEach, describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { exists } from "../src/fs.ts";
import { type CheckReport, ClaimStore, Engine } from "../src/index.ts";

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

describe("library facade (§7.5)", () => {
  test("store dir decouples from the anchor root", async () => {
    const anchorRoot = await tmp();
    const storeDir = join(await tmp(), "investigation", ".claims");
    await mkdir(join(anchorRoot, "src"), { recursive: true });
    await writeFile(
      join(anchorRoot, "src/retry.ts"),
      "export const MAX_ATTEMPTS = 5;\n",
    );
    // The doc carries the documented sentence — the doc side anchors to it.
    await writeFile(
      join(anchorRoot, "README.md"),
      "# Doc\n\nRetries are capped at 5 attempts.\n",
    );

    const engine = await Engine.init(
      { anchorRoot, storeDir },
      { nonce: "deadbeef" },
    );

    // The store lives at the custom dir — NOT under <anchorRoot>/.claims.
    expect(engine.store.dir).toBe(storeDir);
    expect(engine.store.anchorRoot).toBe(anchorRoot);
    expect(await exists(join(storeDir, "config.json"))).toBe(true);
    expect(await exists(join(anchorRoot, ".claims"))).toBe(false);
    expect(await ClaimStore.isInitialized({ anchorRoot, storeDir })).toBe(true);

    // A claim anchored against the anchor root still resolves through the far store.
    await engine.record({
      docPath: "README.md",
      docQuote: "Retries are capped at 5 attempts",
      code: [{ file: "src/retry.ts", quote: "MAX_ATTEMPTS = 5" }],
      authoredTrust: "verified",
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
    await writeFile(join(anchorRoot, "doc.md"), "# D\n\nN is 5\n");

    const engine = await Engine.init(anchorRoot); // string form → store at <root>/.claims
    expect(engine.store.dir).toBe(join(anchorRoot, ".claims"));

    // `verified` → enforced, so a code change gates (exit 2).
    await engine.record({
      docPath: "doc.md",
      docQuote: "N is 5",
      code: [{ file: "code.ts", quote: "N = 5" }],
      authoredTrust: "verified",
      ref: "r",
    });

    // Change the value out from under the claim.
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 50;\n");

    // Read-only: suspect is reported, but the document is left untouched.
    const readOnly = await engine.check();
    expect(readOnly.exitCode).toBe(2);
    expect(readOnly.verdicts[0]?.code).toBe("changed");
    expect(readOnly.verdicts[0]?.gates).toBe(true);
    expect(await readFile(join(anchorRoot, "doc.md"), "utf8")).not.toContain(
      "HIBI:BEGIN",
    );

    // Write: now the banner is stamped into the document.
    const written = await engine.check({ write: true });
    expect(written.exitCode).toBe(2);
    expect(await readFile(join(anchorRoot, "doc.md"), "utf8")).toContain(
      "HIBI:BEGIN",
    );
  });

  test("check report keeps the CLI's contract shape (§9)", async () => {
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
  });

  test("query, status, supersede, retract, archive via the facade", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 5;\n");
    await writeFile(join(anchorRoot, "v1.md"), "# v1\n\nN is 5\n");
    await writeFile(join(anchorRoot, "v2.md"), "# v2\n");
    const e = await Engine.init(anchorRoot);

    await e.record({
      docPath: "v1.md",
      docQuote: "N is 5",
      code: [{ file: "code.ts", quote: "N = 5" }],
    });

    const hits = await e.query("code.ts");
    expect(hits).toHaveLength(1);
    expect(hits[0]?.documentPath).toBe("v1.md");
    expect(hits[0]?.coarse).toBe(false);

    const status = await e.status("v1.md");
    expect(status.found).toBe(true);
    expect(status.current).toBe(true);

    const sup = await e.supersede({
      newDocPath: "v2.md",
      oldDocPath: "v1.md",
      type: "supersedes",
    });
    expect(sup.oldDoc.lifecycle).toBe("superseded");

    const arch = await e.archive("v1.md", "v2.md");
    expect(arch.document.lifecycle).toBe("archived");
    // Archival reads/writes against the anchor root (the decoupled path).
    expect(await exists(join(anchorRoot, "archive", "v1.md"))).toBe(true);

    const ret = await e.retract("v2.md");
    expect(ret.lifecycle).toBe("retracted");
  });

  test("opening a store that was never initialized rejects", async () => {
    await expect(Engine.open(join(await tmp(), "nope"))).rejects.toThrow();
  });

  test("a precise code target on a directory surfaces the real I/O error, unmasked", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "doc.md"), "# D\n\nx is here\n");
    const engine = await Engine.init(anchorRoot);
    // A directory is NOT "not found" — reading it must surface EISDIR, never be
    // silently swallowed (only ENOENT degrades to a missing-file null).
    await mkdir(join(anchorRoot, "adir"), { recursive: true });
    await expect(
      engine.record({
        docPath: "doc.md",
        docQuote: "x is here",
        code: [{ file: "adir", quote: "q" }],
      }),
    ).rejects.toThrow(/EISDIR|directory/);
    // An empty code path likewise resolves to the anchor-root dir → EISDIR.
    await expect(
      engine.record({
        docPath: "doc.md",
        docQuote: "x is here",
        code: [{ file: "", quote: "q" }],
      }),
    ).rejects.toThrow(/EISDIR|directory/);
  });

  test("a missing precise code file degrades to a coarse edge, not an error", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "d.md"), "# D\n\nx is here\n");
    const engine = await Engine.init(anchorRoot);
    // A precise code target whose file is missing (ENOENT) cannot be located, so
    // the record degrades to a coarse navigational edge and stays `suggested`
    // (never `enforced`) — it does not throw (§9/§11.3).
    const res = await engine.record({
      docPath: "d.md",
      docQuote: "x is here",
      code: [{ file: "nope.ts", quote: "q" }],
    });
    expect(res.assertion.enforcement).toBe("suggested");
    expect(res.assertion.anchor.code[0]?.selectors.map((s) => s.kind)).toEqual([
      "path",
    ]);
  });

  test("noAst runs Tier-1 only and still detects text drift", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 5;\n");
    await writeFile(join(anchorRoot, "doc.md"), "# D\n\nN is 5\n");
    const engine = await Engine.init(anchorRoot, { noAst: true });
    await engine.record({
      docPath: "doc.md",
      docQuote: "N is 5",
      code: [{ file: "code.ts", quote: "N = 5" }],
      authoredTrust: "verified",
      ref: "r",
    });
    // Unchanged while the quoted text is present (Tier-1 text-quote selector)…
    expect((await engine.check()).exitCode).toBe(0);
    // …suspect once the anchored line is gone — no tree-sitter analyzer involved.
    await writeFile(join(anchorRoot, "code.ts"), "// removed\n");
    const report = await engine.check();
    expect(report.exitCode).toBe(2);
    expect(report.verdicts[0]?.code).not.toBe("unchanged");
  });

  test("status is scoped to its own document and does not bleed across docs", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "a.ts"), "export const A = 1;\n");
    await writeFile(join(anchorRoot, "b.ts"), "export const B = 2;\n");
    await writeFile(join(anchorRoot, "a.md"), "# A\n\nA is 1\n");
    await writeFile(join(anchorRoot, "b.md"), "# B\n\nB is 2\n");
    const e = await Engine.init(anchorRoot);
    // `verified` → enforced, so a removed anchor gates (status.current === false).
    await e.record({
      docPath: "a.md",
      docQuote: "A is 1",
      code: [{ file: "a.ts", quote: "A = 1" }],
      authoredTrust: "verified",
      ref: "r",
    });
    await e.record({
      docPath: "b.md",
      docQuote: "B is 2",
      code: [{ file: "b.ts", quote: "B = 2" }],
      authoredTrust: "verified",
      ref: "r",
    });

    // Break only a.ts (remove the anchored line entirely).
    await writeFile(join(anchorRoot, "a.ts"), "// removed\n");

    const a = await e.status("a.md");
    const b = await e.status("b.md");
    // a.md drifted; its status returns only its own (scoped) verdict.
    expect(a.current).toBe(false);
    expect(a.verdicts).toHaveLength(1);
    expect(
      a.verdicts.every((v) => v.documentId === a.verdicts[0]?.documentId),
    ).toBe(true);
    // b.md is untouched — scoping a.md's check did not affect it.
    expect(b.current).toBe(true);
    expect(b.verdicts).toHaveLength(1);
  });
});

describe("regression guards (review fixes)", () => {
  test("suggest is idempotent — re-running on unchanged content adds no duplicate claims (§6)", async () => {
    const anchorRoot = await tmp();
    await writeFile(
      join(anchorRoot, "SPEC.md"),
      "# Spec\n\nThe client MUST retry on timeout.\nRequests are capped at 5 attempts.\n",
    );
    const engine = await Engine.init(anchorRoot);

    const first = await engine.suggest("SPEC.md");
    expect(first.created.length).toBeGreaterThan(0);
    const afterFirst = (await engine.store.allAssertions()).length;

    // Same content → every proposition dedups and no new assertion is written.
    const second = await engine.suggest("SPEC.md");
    expect(second.created.length).toBe(0);
    expect((await engine.store.allAssertions()).length).toBe(afterFirst);
  });

  test("reanchor re-points a moved claim back to unchanged (§9)", async () => {
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
      code: [{ file: "src/a.ts", quote: "A = 1" }],
      authoredTrust: "verified",
      ref: "r",
    });

    // Relocate the anchored code — the claim goes `moved`.
    await writeFile(
      join(anchorRoot, "src/a.ts"),
      "// header\n// header2\nexport const A = 1;\n",
    );
    expect((await engine.check()).verdicts[0]?.code).toBe("moved");

    // Reanchor (no explicit spans) re-localizes both sides → settles unchanged.
    const res = await engine.reanchor(rec.assertion.id);
    expect(res.code).toBe("unchanged");
    expect(res.doc).toBe("unchanged");
    expect((await engine.check()).verdicts[0]?.code).toBe("unchanged");
  });

  test("reanchor reads the doc with hibi's banner stripped, never re-anchoring onto it (§8/§18-B)", async () => {
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
      code: [{ file: "src/a.ts", quote: "A = 1" }],
      authoredTrust: "verified",
      ref: "r",
    });

    // Drift the code and stamp a banner — its body restates the doc sentence.
    await writeFile(join(anchorRoot, "src/a.ts"), "export const A = 2;\n");
    await engine.check({ write: true });
    expect(await readFile(join(anchorRoot, "README.md"), "utf8")).toContain(
      "HIBI:BEGIN",
    );

    // Reanchor with no explicit doc span: it must re-localize onto the real prose,
    // not the banner's verbatim copy. If it latched onto the banner, the next
    // check (which strips the banner) would grade the doc side `orphaned`.
    const res = await engine.reanchor(rec.assertion.id);
    expect(res.doc).toBe("unchanged");
    expect((await engine.check()).verdicts[0]?.doc).toBe("unchanged");
  });
});
