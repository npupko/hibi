import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { hasBanner } from "../src/banner/banner.ts";
import {
  getFrontmatterStatus,
  setFrontmatterStatus,
  splitFrontmatter,
} from "../src/banner/frontmatter.ts";
import {
  buildCoarseBundle,
  coarseCovers,
  composeAnchor,
} from "../src/engine/anchor.ts";
import { archiveDocument } from "../src/engine/archive.ts";
import { runCheck } from "../src/engine/check.ts";
import { matchedSide } from "../src/engine/list.ts";
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

describe("frontmatter helper (used only to strip a legacy hibi-status line)", () => {
  test("setFrontmatterStatus only acts where frontmatter exists; round-trips", () => {
    const withFm = "---\ntitle: Doc\n---\n\n# Body\n";
    const set = setFrontmatterStatus(withFm, "code:changed");
    expect(getFrontmatterStatus(set)).toBe("code:changed");
    expect(splitFrontmatter(set).body).toBe("\n# Body\n");
    const cleared = setFrontmatterStatus(set, null);
    expect(getFrontmatterStatus(cleared)).toBeUndefined();
    expect(cleared).toBe(withFm);

    const noFm = "# Body only\n";
    expect(setFrontmatterStatus(noFm, "code:changed")).toBe(noFm);
  });

  test("never clobbers an author's own status key", () => {
    const t = "---\nstatus: published\n---\nbody\n";
    const set = setFrontmatterStatus(t, "code:changed");
    expect(set).toContain("status: published");
    expect(getFrontmatterStatus(set)).toBe("code:changed");
  });

  test("check --write strips a legacy hibi-status line and never writes one", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write(
      "doc.md",
      "---\ntitle: Doc\nhibi-status: code:changed\n---\n\n# Doc\n",
    );
    await record(r, {
      doc: "doc.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
    });
    await r.write("src/a.ts", "// gone\n");
    const rep = await runCheck(r.store, { ast: analyzer, write: true });
    const docReport = rep.documents.find((d) => d.path === "doc.md");
    expect(docReport?.suspect.map((s) => s.status)).toEqual(["code:orphaned"]);
    const doc = await r.read("doc.md");
    expect(getFrontmatterStatus(doc)).toBeUndefined();
    expect(doc.startsWith("---\ntitle: Doc\n---")).toBe(true);
    expect(hasBanner(doc, "doc.md", "deadbeef")).toBe(true);
  });
});

describe("archival", () => {
  test("moves the doc out of the read path, writes a tombstone, sets lifecycle archived", async () => {
    const r = await repo();
    await r.write("old.md", "# Old policy\n\nOriginal content.\n");
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
    const result = await archiveDocument(r.store, "old.md", undefined, {
      dryRun: true,
    });
    expect(result.dryRun).toBe(true);
    expect(await r.read("old.md")).toBe("# Old policy\n");
    expect(await exists(join(r.root, "archive", "old.md"))).toBe(false);
    expect(await r.store.getDocument(result.document.id)).toBeUndefined();
  });
});

describe("coarse code targets", () => {
  test("a glob code-side anchor records as suggested with a coarse selector", async () => {
    const r = await repo();
    await record(r, {
      doc: "arch.md",
      text: "Decisions about the auth module",
      file: "src/auth/**",
      glob: "src/auth/**",
    });
    const a = (await r.store.allAssertions())[0];
    if (a === undefined) throw new Error("expected a recorded assertion");
    expect(a.enforcement).toBe("suggested");
    expect(a.anchor.code.length).toBe(1);
    expect(a.anchor.code[0]?.selectors).toEqual([
      { kind: "coarse", pattern: "src/auth/**" },
    ]);
    expect(matchedSide(a, "src/auth/login.ts")).toBe("code");
    expect(matchedSide(a, "arch.md")).toBe("doc");
    expect(matchedSide(a, "src/other.ts")).toBeUndefined();
  });

  test("buildCoarseBundle + composeAnchor: a swapped-in coarse code side still matches", async () => {
    const r = await repo();
    await record(r, {
      doc: "arch.md",
      text: "Decisions about the payments module",
      file: "src/payments/**",
      glob: "src/payments/**",
    });
    const a = (await r.store.allAssertions())[0];
    if (a === undefined) throw new Error("expected a recorded assertion");
    a.anchor = composeAnchor(a.anchor.doc, [
      buildCoarseBundle("src/payments/**"),
    ]);
    await r.store.putAssertion(a);
    const reloaded = await r.store.getAssertion(a.id);
    if (reloaded === undefined) throw new Error("assertion did not persist");
    expect(matchedSide(reloaded, "src/payments/charge.ts")).toBe("code");
  });

  test("a coarse path covers on a `/` boundary, not a bare prefix", () => {
    expect(coarseCovers("src", "src/main.ts")).toBe(true);
    expect(coarseCovers("src", "src2/main.ts")).toBe(false);
    expect(coarseCovers("src/a.ts", "src/a.ts")).toBe(true);
    expect(coarseCovers("src/**/*.ts", "src/x/y.ts")).toBe(true);
  });
});

describe("moved-only verdict warns and exits 0", () => {
  test("a position shift with intact content grades code moved (doc unchanged)", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const MAX = 5;\n");
    await record(r, {
      doc: "doc.md",
      text: "Max is 5",
      file: "src/a.ts",
      quote: "MAX = 5",
    });
    const a = (await r.store.allAssertions())[0];
    if (a === undefined) throw new Error("expected a recorded assertion");
    expect(a.enforcement).toBe("enforced");
    await r.write(
      "src/a.ts",
      `${"// prologue line\n".repeat(3)}export const MAX = 5;\n`,
    );
    const rep = await runCheck(r.store, { ast: analyzer });
    expect(rep.verdicts[0]?.code).toBe("moved");
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
    return { r, id: rec.assertion.id };
  }

  test("code changed: recommended update-claim with [update-claim, reanchor, retire]", async () => {
    const { r, id } = await seeded();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const v = (await runCheck(r.store, { ast: analyzer })).verdicts[0];
    expect(v?.code).toBe("changed");
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
    expect(v?.code).toBe("unchanged");
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
    expect(v?.code).toBe("changed");
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
    expect(v?.code).toBe("orphaned");
    expect(v?.remediation?.recommended).toBe("reanchor");
    const ids = v?.remediation?.actions.map((a) => a.id) ?? [];
    expect(ids).toContain("retire");
    expect(ids).toContain("supersede");
    expect(v?.remediation?.actions[0]?.command).toBe(
      `hibi reanchor ${id} --suggest`,
    );
  });
});
