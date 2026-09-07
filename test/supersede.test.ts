import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { runCheck } from "../src/engine/check.ts";
import { documentIdForPath } from "../src/engine/record.ts";
import { isLiveClaimOn, supersede } from "../src/engine/supersede.ts";
import { Engine } from "../src/index.ts";
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

/** The lifecycle deps the engine module needs, provided by the facade. */
async function deps(r: TempRepo) {
  const engine = await Engine.open(r.root);
  return {
    readDoc: async (rel: string) => {
      try {
        return await r.read(rel);
      } catch {
        return null;
      }
    },
    reanchor: (
      id: string,
      o: { doc: string; docQuote: string; ref?: string; dryRun?: boolean },
    ) => engine.reanchor(id, o),
  };
}

describe("supersession edges", () => {
  test("supersedes flips the old doc to superseded and authors the edge on the new doc", async () => {
    const r = await repo();
    await r.write("v2.md", "# v2\n");
    const { newDoc, oldDoc, relocated, misses, strandedClaims, dryRun } =
      await supersede(r.store, await deps(r), { from: "v1.md", to: "v2.md" });
    expect(newDoc.edges).toEqual([{ type: "supersedes", target: oldDoc.id }]);
    expect(oldDoc.edges).toEqual([]);
    expect(oldDoc.lifecycle).toBe("superseded");
    expect(newDoc.lifecycle).toBe("active");
    expect(relocated).toEqual([]);
    expect(misses).toEqual([]);
    expect(strandedClaims).toEqual([]);
    expect(dryRun).toBe(false);
    expect((await r.store.getDocument(oldDoc.id))?.lifecycle).toBe(
      "superseded",
    );
  });

  test("from and to must differ, and the new document must exist on disk", async () => {
    const r = await repo();
    await expect(
      supersede(r.store, await deps(r), { from: "v1.md", to: "v1.md" }),
    ).rejects.toThrow("must differ");
    await expect(
      supersede(r.store, await deps(r), { from: "v1.md", to: "missing.md" }),
    ).rejects.toThrow("Document not found on disk: missing.md");
  });

  test("edges are idempotent (re-authoring does not duplicate)", async () => {
    const r = await repo();
    await r.write("v2.md", "# v2\n");
    await supersede(r.store, await deps(r), { from: "v1.md", to: "v2.md" });
    await supersede(r.store, await deps(r), { from: "v1.md", to: "v2.md" });
    const newDoc = await r.store.getDocument(documentIdForPath("v2.md"));
    expect(newDoc?.edges.filter((e) => e.type === "supersedes").length).toBe(1);
  });

  test("dry run reports the result without writing the store", async () => {
    const r = await repo();
    await r.write("v2.md", "# v2\n");
    const res = await supersede(r.store, await deps(r), {
      from: "v1.md",
      to: "v2.md",
      dryRun: true,
    });
    expect(res.dryRun).toBe(true);
    expect(res.oldDoc.lifecycle).toBe("superseded");
    expect(
      await r.store.getDocument(documentIdForPath("v1.md")),
    ).toBeUndefined();
    expect(
      await r.store.getDocument(documentIdForPath("v2.md")),
    ).toBeUndefined();
  });
});

describe("claim relocation on supersede", () => {
  test("a claim whose sentence appears verbatim in the new doc is relocated", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("v1.md", "# V1\n\nA is one.\n");
    await r.write("v2.md", "# V2\n\nIntro.\n\nA is one.\n");
    const claim = await record(r, {
      doc: "v1.md",
      text: "A is one.",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const res = await supersede(r.store, await deps(r), {
      from: "v1.md",
      to: "v2.md",
    });
    expect(res.relocated).toEqual([
      { claimId: claim.assertion.id, doc: "unchanged", code: "unchanged" },
    ]);
    expect(res.misses).toEqual([]);
    expect(res.strandedClaims).toEqual([]);
    const moved = await r.store.getAssertion(claim.assertion.id);
    expect(moved?.documentId).toBe(documentIdForPath("v2.md"));
    expect(moved?.anchor.doc.file).toBe("v2.md");
    expect(moved?.anchor.code[0]?.file).toBe("src/a.ts");
    // The relocated claim resolves clean on the new document.
    const rep = await runCheck(r.store, { ast: analyzer });
    expect(rep.verdicts[0]?.doc).toBe("unchanged");
    expect(rep.exitCode).toBe(0);
  });

  test("a claim whose sentence is absent from the new doc is a miss and stays stranded", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("v1.md", "# V1\n\nA is one.\n");
    await r.write("v2.md", "# V2\n\nSomething else.\n");
    const claim = await record(r, {
      doc: "v1.md",
      text: "A is one.",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const res = await supersede(r.store, await deps(r), {
      from: "v1.md",
      to: "v2.md",
    });
    expect(res.relocated).toEqual([]);
    expect(res.misses.map((m) => m.claimId)).toEqual([claim.assertion.id]);
    expect(res.misses[0]?.reason).toContain("not found in v2.md");
    expect(res.strandedClaims).toEqual([claim.assertion.id]);
    expect((await r.store.getAssertion(claim.assertion.id))?.documentId).toBe(
      documentIdForPath("v1.md"),
    );
  });

  test("a retired claim is neither relocated nor counted as stranded", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("v1.md", "# V1\n\nA is one.\n");
    await r.write("v2.md", "# V2\n\nA is one.\n");
    const claim = await record(r, {
      doc: "v1.md",
      text: "A is one.",
      file: "src/a.ts",
      quote: "A = 1",
    });
    await r.store.putAssertion({ ...claim.assertion, enforcement: "retired" });
    const retired = await r.store.getAssertion(claim.assertion.id);
    if (!retired) throw new Error("claim missing");
    expect(isLiveClaimOn(retired, documentIdForPath("v1.md"))).toBe(false);
    const res = await supersede(r.store, await deps(r), {
      from: "v1.md",
      to: "v2.md",
    });
    expect(res.relocated).toEqual([]);
    expect(res.strandedClaims).toEqual([]);
  });

  test("dry run lists the would-be relocations without moving anything", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("v1.md", "# V1\n\nA is one.\n");
    await r.write("v2.md", "# V2\n\nA is one.\n");
    const claim = await record(r, {
      doc: "v1.md",
      text: "A is one.",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const res = await supersede(r.store, await deps(r), {
      from: "v1.md",
      to: "v2.md",
      dryRun: true,
    });
    expect(res.relocated.map((x) => x.claimId)).toEqual([claim.assertion.id]);
    expect(
      (await r.store.getAssertion(claim.assertion.id))?.anchor.doc.file,
    ).toBe("v1.md");
  });
});

describe("supersession and code drift are surfaced together", () => {
  test("a superseded doc with a separately drifted claim shows both in the banner", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\nexport const B = 2;\n");
    await r.write("doc.md", "# Doc\n\nText.\n");
    await r.write("newer.md", "# Newer\n");
    const a = await record(r, {
      doc: "doc.md",
      text: "A is one",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const b = await record(r, {
      doc: "doc.md",
      text: "B is two",
      file: "src/a.ts",
      quote: "B = 2",
    });

    // Neither sentence appears in newer.md, so both claims stay on doc.md.
    const res = await supersede(r.store, await deps(r), {
      from: "doc.md",
      to: "newer.md",
    });
    expect(res.strandedClaims.sort()).toEqual(
      [a.assertion.id, b.assertion.id].sort(),
    );
    await r.write("src/a.ts", "export const A = 1;\nexport const B = 22;\n");

    const report = await runCheck(r.store, { ast: analyzer, write: true });
    const docReport = report.documents.find((d) => d.path === "doc.md");
    expect(docReport?.lifecycle).toBe("superseded");
    const statuses = docReport?.suspect.map((s) => s.status) ?? [];
    expect(statuses).toContain("code:changed");
    expect(statuses).toContain("superseded");

    const banner = await r.read("doc.md");
    expect(banner).toContain("[superseded]");
    expect(banner).toContain("[code:changed]");
    expect(banner).toContain(b.proposition.id);
    expect(banner).toContain(a.proposition.id);
    expect(banner).toContain("This document has been superseded.");
  });

  test("a claim suspect only because its document is superseded reports the lifecycle status", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("doc.md", "# Doc\n\nText.\n");
    await r.write("newer.md", "# Newer\n");
    const a = await record(r, {
      doc: "doc.md",
      text: "A is one",
      file: "src/a.ts",
      quote: "A = 1",
    });
    await supersede(r.store, await deps(r), { from: "doc.md", to: "newer.md" });

    const report = await runCheck(r.store, { ast: analyzer, write: true });
    const docReport = report.documents.find((d) => d.path === "doc.md");
    const entry = docReport?.suspect.find(
      (s) => s.propositionId === a.proposition.id,
    );
    expect(entry?.status).toBe("superseded");
    // A lifecycle-only suspect never gates.
    expect(report.exitCode).toBe(0);
  });
});
