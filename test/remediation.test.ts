import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { access } from "node:fs/promises";
import { join } from "node:path";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import {
  getFrontmatterStatus,
  setFrontmatterStatus,
  splitFrontmatter,
} from "../src/banner/frontmatter.ts";
import { buildGlobAnchor } from "../src/engine/anchor.ts";
import { archiveDocument } from "../src/engine/archive.ts";
import { runCheck } from "../src/engine/check.ts";
import { queryByPath } from "../src/engine/query.ts";
import { recordClaim } from "../src/engine/record.ts";
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
async function fileExists(p: string) {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}

describe("optional markdown frontmatter status (§8)", () => {
  test("setFrontmatterStatus only acts where frontmatter exists; round-trips", () => {
    const withFm = "---\ntitle: Doc\n---\n\n# Body\n";
    const set = setFrontmatterStatus(withFm, "stale");
    expect(getFrontmatterStatus(set)).toBe("stale");
    expect(splitFrontmatter(set).body).toBe("\n# Body\n"); // body untouched
    const cleared = setFrontmatterStatus(set, null);
    expect(getFrontmatterStatus(cleared)).toBeUndefined();
    expect(cleared).toBe(withFm); // exact restore

    const noFm = "# Body only\n";
    expect(setFrontmatterStatus(noFm, "stale")).toBe(noFm); // never creates frontmatter
  });

  test("never clobbers an author's own status key", () => {
    const t = "---\nstatus: published\n---\nbody\n";
    const set = setFrontmatterStatus(t, "stale");
    expect(set).toContain("status: published");
    expect(getFrontmatterStatus(set)).toBe("stale");
  });

  test("check --write sets the frontmatter status on a drifted markdown doc with frontmatter", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("doc.md", "---\ntitle: Doc\n---\n\n# Doc\n");
    await record(r, {
      doc: "doc.md",
      text: "A is 1",
      file: "src/a.ts",
      quote: "A = 1",
    });
    await r.write("src/a.ts", "// gone\n");
    const rep = await runCheck(r.store, { ast: analyzer, write: true });
    const docReport = rep.documents.find((d) => d.path === "doc.md");
    if (docReport === undefined) throw new Error("doc.md report not found");
    expect(docReport.frontmatterStatus).toBe("ghost");
    expect(getFrontmatterStatus(await r.read("doc.md"))).toBe("ghost");
  });
});

describe("archival remediation (§6)", () => {
  test("moves the doc out of the read path, writes a tombstone, sets lifecycle archived", async () => {
    const r = await repo();
    await r.write("old.md", "# Old policy\n\nOriginal content.\n");
    const result = await archiveDocument(r.store, "old.md", "new.md");
    expect(result.document.lifecycle).toBe("archived");
    expect(result.archivedTo).toBe(join("archive", "old.md"));
    // Original content preserved in the archive.
    expect(await r.read(join("archive", "old.md"))).toContain(
      "Original content.",
    );
    // Tombstone at the original path, redirecting to the successor.
    const tomb = await r.read("old.md");
    expect(tomb).toContain("# Archived");
    expect(tomb).toContain("new.md");
    expect(getFrontmatterStatus(tomb)).toBe("archived");
    expect(await fileExists(join(r.root, "archive", "old.md"))).toBe(true);
  });
});

describe("coarse glob blast-radius in query (§4, §9)", () => {
  test("a glob anchor matches files under it (navigational, never stale)", async () => {
    const r = await repo();
    // Record a coarse glob claim directly.
    await recordClaim(r.store, null, {
      docPath: "arch.md",
      text: "Decisions about the auth module",
      authoredTrust: "assumed",
      owner: "x",
      ref: "r",
      codeFile: "src/auth/**",
      coarse: true,
    });
    // Replace the path anchor with a glob anchor for the test.
    const a = (await r.store.allAssertions())[0];
    if (a === undefined) throw new Error("expected a recorded assertion");
    a.anchor = buildGlobAnchor("src/auth/**");
    await r.store.putAssertion(a);

    const hits = await queryByPath(r.store, "src/auth/login.ts");
    expect(hits.length).toBe(1);
    expect(hits[0]?.coarse).toBe(true);
  });
});

describe("moved-only verdict yields exit code 3 end-to-end (§9)", () => {
  test("a position shift with intact content grades moved → exit 3", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const MAX = 5;\n");
    await record(r, {
      doc: "doc.md",
      text: "Max is 5",
      file: "src/a.ts",
      quote: "MAX = 5",
    });
    // Prepend a large intact prologue so the region relocates far (moved), but
    // nothing about the anchored construct changes.
    await r.write(
      "src/a.ts",
      `${"// prologue line\n".repeat(3)}export const MAX = 5;\n`,
    );
    const rep = await runCheck(r.store, { ast: analyzer });
    expect(rep.verdicts[0]?.state).toBe("moved");
    expect(rep.exitCode).toBe(3);
  });
});
