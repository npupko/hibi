import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import {
  getFrontmatterStatus,
  setFrontmatterStatus,
  splitFrontmatter,
} from "../src/banner/frontmatter.ts";
import { buildGlobBundle, composeAnchor } from "../src/engine/anchor.ts";
import { archiveDocument } from "../src/engine/archive.ts";
import { runCheck } from "../src/engine/check.ts";
import { queryByPath } from "../src/engine/query.ts";
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

describe("optional markdown frontmatter status (§8)", () => {
  test("setFrontmatterStatus only acts where frontmatter exists; round-trips", () => {
    const withFm = "---\ntitle: Doc\n---\n\n# Body\n";
    // Use a side-tagged machine status, the new vocabulary (§8).
    const set = setFrontmatterStatus(withFm, "code:changed");
    expect(getFrontmatterStatus(set)).toBe("code:changed");
    expect(splitFrontmatter(set).body).toBe("\n# Body\n"); // body untouched
    const cleared = setFrontmatterStatus(set, null);
    expect(getFrontmatterStatus(cleared)).toBeUndefined();
    expect(cleared).toBe(withFm); // exact restore

    const noFm = "# Body only\n";
    expect(setFrontmatterStatus(noFm, "code:changed")).toBe(noFm); // never creates frontmatter
  });

  test("never clobbers an author's own status key", () => {
    const t = "---\nstatus: published\n---\nbody\n";
    const set = setFrontmatterStatus(t, "code:changed");
    expect(set).toContain("status: published");
    expect(getFrontmatterStatus(set)).toBe("code:changed");
  });

  test("check --write sets the side-tagged frontmatter status on a doc whose code went orphaned", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("doc.md", "---\ntitle: Doc\n---\n\n# Doc\n");
    // Enforced so the orphaned code side gates and the doc becomes suspect.
    await record(r, {
      doc: "doc.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
      trust: "verified",
    });
    // Delete the code file outright → the code side resolves `orphaned` (was 'ghost').
    await r.write(
      "src/a.ts",
      "// gone — the documented construct is deleted\n",
    );
    const rep = await runCheck(r.store, { ast: analyzer, write: true });
    const docReport = rep.documents.find((d) => d.path === "doc.md");
    if (docReport === undefined) throw new Error("doc.md report not found");
    expect(docReport.frontmatterStatus).toBe("code:orphaned");
    expect(getFrontmatterStatus(await r.read("doc.md"))).toBe("code:orphaned");
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
    expect(await exists(join(r.root, "archive", "old.md"))).toBe(true);
  });
});

describe("coarse glob blast-radius in query (§4, §9)", () => {
  test("a glob code-side anchor matches files under it (navigational, never drift)", async () => {
    const r = await repo();
    // Record a coarse/assumed claim — stays suggested & navigational (§11.3).
    await record(r, {
      doc: "arch.md",
      text: "Decisions about the auth module",
      file: "src/auth/**",
      glob: "src/auth/**",
      trust: "assumed",
    });
    // The assumed/coarse claim never gets enforced.
    const a = (await r.store.allAssertions())[0];
    if (a === undefined) throw new Error("expected a recorded assertion");
    expect(a.enforcement).toBe("suggested");
    // Code side is a single glob bundle (buildGlobBundle); doc side is precise.
    expect(a.anchor.code.length).toBe(1);
    expect(a.anchor.code[0]?.selectors[0]?.kind).toBe("glob");

    // queryByPath hits the code side via the glob, flagged coarse.
    const hits = await queryByPath(r.store, "src/auth/login.ts");
    expect(hits.length).toBe(1);
    expect(hits[0]?.coarse).toBe(true);
    expect(hits[0]?.side).toBe("code");
  });

  test("buildGlobBundle + composeAnchor: a swapped-in glob code side still queries coarse", async () => {
    const r = await repo();
    await record(r, {
      doc: "arch.md",
      text: "Decisions about the payments module",
      file: "src/payments/**",
      glob: "src/payments/**",
      trust: "assumed",
    });
    // Recompose the anchor's code side with a fresh glob bundle directly.
    const a = (await r.store.allAssertions())[0];
    if (a === undefined) throw new Error("expected a recorded assertion");
    a.anchor = composeAnchor(a.anchor.doc, [
      buildGlobBundle("src/payments/**"),
    ]);
    await r.store.putAssertion(a);

    const hits = await queryByPath(r.store, "src/payments/charge.ts");
    expect(hits.length).toBe(1);
    expect(hits[0]?.coarse).toBe(true);
    expect(hits[0]?.side).toBe("code");
  });

  test("a coarse `path` edge covers on a `/` boundary, not a bare prefix (§11.3)", async () => {
    const r = await repo();
    // A coarse path edge anchored to the directory `src`.
    await record(r, {
      doc: "arch.md",
      text: "Decisions about the src tree",
      file: "src",
      coarse: true,
      trust: "assumed",
    });
    // Covers files under `src/…` …
    expect((await queryByPath(r.store, "src/main.ts")).length).toBe(1);
    // …but NOT a sibling whose name merely starts with `src`.
    expect((await queryByPath(r.store, "src2/main.ts")).length).toBe(0);
  });
});

describe("moved-only verdict yields exit code 3 end-to-end (§9)", () => {
  test("a position shift with intact content grades code moved (doc unchanged) → exit 3 on an enforced claim", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const MAX = 5;\n");
    // Enforced (verified trust) so the moved code side can warn at exit 3.
    await record(r, {
      doc: "doc.md",
      text: "Max is 5",
      file: "src/a.ts",
      quote: "MAX = 5",
      trust: "verified",
    });
    const a = (await r.store.allAssertions())[0];
    if (a === undefined) throw new Error("expected a recorded assertion");
    expect(a.enforcement).toBe("enforced");
    // Prepend a large intact prologue so the region relocates far (moved), but
    // nothing about the anchored construct changes.
    await r.write(
      "src/a.ts",
      `${"// prologue line\n".repeat(3)}export const MAX = 5;\n`,
    );
    const rep = await runCheck(r.store, { ast: analyzer });
    expect(rep.verdicts[0]?.code).toBe("moved");
    expect(rep.verdicts[0]?.doc).toBe("unchanged");
    // moved never gates; an enforced moved verdict warns at exit 3 (§9).
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.exitCode).toBe(3);
  });
});
