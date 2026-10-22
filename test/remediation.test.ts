import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { archiveDocument } from "../src/engine/archive.ts";
import { runCheck } from "../src/engine/check.ts";
import { matchedSide } from "../src/engine/list.ts";
import { documentIdForPath, newDocument } from "../src/engine/record.ts";
import { exists } from "../src/fs.ts";
import { makeRepo, record, type TempRepo } from "./helpers.ts";

let analyzer: Awaited<ReturnType<typeof getAnalyzer>>;
beforeAll(async () => {
  analyzer = await getAnalyzer();
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

/** Register `path` as a document, as `record` does for its first claim. */
async function register(r: TempRepo, path: string): Promise<void> {
  await r.store.putDocument(
    newDocument(documentIdForPath(path), path, new Date()),
  );
}

describe("archival", () => {
  test("moves the doc out of the read path, writes a tombstone, sets lifecycle archived", async () => {
    const r = await repo();
    await r.write("old.md", "# Old policy\n\nOriginal content.\n");
    await register(r, "old.md");
    const result = await archiveDocument(r.store, "old.md", "new.md");
    expect(result.document.lifecycle).toBe("archived");
    expect(result.archivedTo).toBe(join("archive", "old.md"));
    expect(result.strandedClaims).toEqual([]);
    expect(await r.read(join("archive", "old.md"))).toContain(
      "Original content.",
    );
    const tomb = await r.read("old.md");
    expect(tomb).toContain("# Archived");
    expect(tomb).toContain("new.md");
    expect(tomb.startsWith("---")).toBe(false);
    expect(await exists(join(r.root, "archive", "old.md"))).toBe(true);
  });

  test("dry run neither moves the file nor writes the store", async () => {
    const r = await repo();
    await r.write("old.md", "# Old policy\n");
    await register(r, "old.md");
    const result = await archiveDocument(r.store, "old.md", undefined, {
      dryRun: true,
    });
    expect(result.dryRun).toBe(true);
    expect(await r.read("old.md")).toBe("# Old policy\n");
    expect(await exists(join(r.root, "archive", "old.md"))).toBe(false);
    expect((await r.store.getDocument(result.document.id))?.lifecycle).toBe(
      "active",
    );
  });

  test("a path that is not a registered document is an error", async () => {
    const r = await repo();
    await r.write("old.md", "# Old policy\n");
    await expect(archiveDocument(r.store, "old.md")).rejects.toThrow(
      "old.md is not a registered document; nothing to archive (a document is registered when a claim is recorded on it).",
    );
    expect(await exists(join(r.root, "archive", "old.md"))).toBe(false);
  });

  test("a registered document whose file is gone is still archived", async () => {
    const r = await repo();
    await register(r, "gone.md");
    const result = await archiveDocument(r.store, "gone.md");
    expect(result.archivedTo).toBeNull();
    expect((await r.store.getDocument(result.document.id))?.lifecycle).toBe(
      "archived",
    );
  });
});

describe("list --path", () => {
  test("matchedSide names the side whose file equals the path", async () => {
    const r = await repo();
    await r.write("src/auth/login.ts", "export const TOKEN_TTL = 3600;\n");
    await record(r, {
      doc: "arch.md",
      text: "Tokens live for an hour, 3600 seconds",
      file: "src/auth/login.ts",
      quote: "TOKEN_TTL = 3600",
    });
    const a = (await r.store.allClaims())[0];
    if (a === undefined) throw new Error("expected a recorded claim");
    expect(matchedSide(a, "src/auth/login.ts")).toBe("evidence");
    expect(matchedSide(a, "arch.md")).toBe("doc");
    expect(matchedSide(a, "src/auth")).toBeUndefined();
  });
});

describe("moved-only verdict warns and exits 0", () => {
  test("a position shift with intact content grades evidence moved (doc unchanged)", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const MAX = 5;\n");
    await record(r, {
      doc: "doc.md",
      text: "Max is 5",
      file: "src/a.ts",
      quote: "MAX = 5",
    });
    const a = (await r.store.allClaims())[0];
    if (a === undefined) throw new Error("expected a recorded claim");
    expect(a.enforcement).toBe("enforced");
    await r.write(
      "src/a.ts",
      `${"// prologue line\n".repeat(3)}export const MAX = 5;\n`,
    );
    const rep = await runCheck(r.store, { ast: analyzer });
    expect(rep.verdicts[0]?.evidence).toBe("moved");
    expect(rep.verdicts[0]?.doc).toBe("unchanged");
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.summary.warning).toBe(1);
    expect(rep.exitCode).toBe(0);
    expect(rep.verdicts[0]?.remediation?.recommended).toBe("reanchor");
    const warn = await runCheck(r.store, { ast: analyzer, failOn: "warn" });
    expect(warn.exitCode).toBe(2);
  });
});

describe("remediation menus on live verdicts", () => {
  async function seeded() {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write("README.md", "# Doc\n\nRetries are capped at 5 attempts.\n");
    const rec = await record(r, {
      doc: "README.md",
      text: "Retries are capped at 5 attempts",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    return { r, id: rec.claim.id };
  }

  test("evidence changed: recommended update-claim with [update-claim, reanchor, retire]", async () => {
    const { r, id } = await seeded();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const v = (await runCheck(r.store, { ast: analyzer })).verdicts[0];
    expect(v?.evidence).toBe("changed");
    expect(v?.doc).toBe("unchanged");
    expect(v?.remediation?.recommended).toBe("update-claim");
    expect(v?.remediation?.actions.map((a) => a.id)).toEqual([
      "update-claim",
      "reanchor",
      "retire",
    ]);
    expect(v?.remediation?.actions[0]?.command).toBe(`hibi reanchor ${id}`);
  });

  test("doc changed: recommended reverify-doc", async () => {
    const { r } = await seeded();
    await r.write("README.md", "# Doc\n\nRetries are capped at 7 attempts.\n");
    const v = (await runCheck(r.store, { ast: analyzer })).verdicts[0];
    expect(v?.doc).toBe("changed");
    expect(v?.evidence).toBe("unchanged");
    expect(v?.remediation?.recommended).toBe("reverify-doc");
    expect(v?.remediation?.actions.map((a) => a.id)).toEqual([
      "reverify-doc",
      "reanchor",
      "retire",
    ]);
  });

  test("both changed: recommended reconcile", async () => {
    const { r } = await seeded();
    await r.write("README.md", "# Doc\n\nRetries are capped at 7 attempts.\n");
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const v = (await runCheck(r.store, { ast: analyzer })).verdicts[0];
    expect(v?.doc).toBe("changed");
    expect(v?.evidence).toBe("changed");
    expect(v?.remediation?.recommended).toBe("reconcile");
    expect(v?.remediation?.actions.map((a) => a.id)).toEqual([
      "reconcile",
      "reanchor",
      "retire",
    ]);
  });

  test("orphaned: recommended reanchor with --suggest, plus retire and supersede", async () => {
    const { r, id } = await seeded();
    await r.write("src/retry.ts", "export const OTHER = 1;\n");
    const v = (await runCheck(r.store, { ast: analyzer })).verdicts[0];
    expect(v?.evidence).toBe("orphaned");
    expect(v?.remediation?.recommended).toBe("reanchor");
    const ids = v?.remediation?.actions.map((a) => a.id) ?? [];
    expect(ids).toContain("retire");
    expect(ids).toContain("supersede");
    expect(v?.remediation?.actions[0]?.command).toBe(
      `hibi reanchor ${id} --suggest`,
    );
  });
});
