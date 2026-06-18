/**
 * The public library facade (src/index.ts): the in-process surface a consumer
 * (e.g. atlas) imports instead of shelling out. Exercises the decoupled store
 * location, the Engine verbs, and the read-only "verdicts as data" path.
 */
import { afterEach, describe, expect, test } from "bun:test";
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
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
async function fileExists(p: string): Promise<boolean> {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}

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
    expect(await fileExists(join(storeDir, "config.json"))).toBe(true);
    expect(await fileExists(join(anchorRoot, ".claims"))).toBe(false);
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
    expect(await fileExists(join(anchorRoot, "archive", "v1.md"))).toBe(true);

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
});
