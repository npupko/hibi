/**
 * The public library facade (src/index.ts): the in-process surface a consumer
 * (e.g. atlas) imports instead of shelling out. Exercises the decoupled store
 * location, the Engine verbs, and the read-only "verdicts as data" path.
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
    await writeFile(join(anchorRoot, "README.md"), "# Doc\n\nProse.\n");

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
      text: "Retries are capped at 5 attempts",
      codeFile: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      authoredTrust: "verified",
      ref: "testref",
    });
    const report = await engine.check();
    expect(report.exitCode).toBe(0);
    expect(report.verdicts).toHaveLength(1);
    expect(report.verdicts[0]?.state).toBe("fresh");
  });

  test("check returns verdicts as data by default; stamps a banner only on write", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 5;\n");
    await writeFile(join(anchorRoot, "doc.md"), "# D\n\ntext\n");

    const engine = await Engine.init(anchorRoot); // string form → store at <root>/.claims
    expect(engine.store.dir).toBe(join(anchorRoot, ".claims"));

    await engine.record({
      docPath: "doc.md",
      text: "N is 5",
      codeFile: "code.ts",
      quote: "N = 5",
      authoredTrust: "verified",
      ref: "r",
    });

    // Change the value out from under the claim.
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 50;\n");

    // Read-only: suspect is reported, but the document is left untouched.
    const readOnly = await engine.check();
    expect(readOnly.exitCode).toBe(2);
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
    await writeFile(join(anchorRoot, "v1.md"), "# v1\n");
    await writeFile(join(anchorRoot, "v2.md"), "# v2\n");
    const e = await Engine.init(anchorRoot);

    await e.record({
      docPath: "v1.md",
      text: "N is 5",
      codeFile: "code.ts",
      quote: "N = 5",
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

  test("a precise record with an empty codeFile fails with a clear error", async () => {
    const anchorRoot = await tmp();
    const engine = await Engine.init(anchorRoot);
    // Without the guard this resolves to the anchor-root dir and throws EISDIR.
    await expect(
      engine.record({ docPath: "doc.md", text: "x", codeFile: "" }),
    ).rejects.toThrow(/codeFile/);
  });

  test("record reports a missing file clearly but does not mask other I/O errors", async () => {
    const anchorRoot = await tmp();
    const engine = await Engine.init(anchorRoot);
    // Missing file → the friendly, locatable message.
    await expect(
      engine.record({
        docPath: "d.md",
        text: "x",
        codeFile: "nope.ts",
        quote: "q",
      }),
    ).rejects.toThrow(/Code file not found/);
    // A directory is NOT "not found" — the real cause (EISDIR) must surface.
    await mkdir(join(anchorRoot, "adir"), { recursive: true });
    await expect(
      engine.record({
        docPath: "d.md",
        text: "x",
        codeFile: "adir",
        quote: "q",
      }),
    ).rejects.toThrow(/EISDIR|directory/);
  });

  test("noAst runs Tier-1 only and still detects text drift", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "code.ts"), "export const N = 5;\n");
    await writeFile(join(anchorRoot, "doc.md"), "# D\n\ntext\n");
    const engine = await Engine.init(anchorRoot, { noAst: true });
    await engine.record({
      docPath: "doc.md",
      text: "N is 5",
      codeFile: "code.ts",
      quote: "N = 5",
    });
    // Fresh while the quoted text is present (Tier-1 text-quote selector)…
    expect((await engine.check()).exitCode).toBe(0);
    // …suspect once the anchored line is gone — no tree-sitter analyzer involved.
    await writeFile(join(anchorRoot, "code.ts"), "// removed\n");
    const report = await engine.check();
    expect(report.exitCode).toBe(2);
    expect(report.verdicts[0]?.state).not.toBe("fresh");
  });

  test("status is scoped to its own document and does not bleed across docs", async () => {
    const anchorRoot = await tmp();
    await writeFile(join(anchorRoot, "a.ts"), "export const A = 1;\n");
    await writeFile(join(anchorRoot, "b.ts"), "export const B = 2;\n");
    const e = await Engine.init(anchorRoot);
    await e.record({
      docPath: "a.md",
      text: "A is 1",
      codeFile: "a.ts",
      quote: "A = 1",
    });
    await e.record({
      docPath: "b.md",
      text: "B is 2",
      codeFile: "b.ts",
      quote: "B = 2",
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
