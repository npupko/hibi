import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { runCheck } from "../src/engine/check.ts";
import { documentIdForPath } from "../src/engine/record.ts";
import {
  amendedPropositions,
  retract,
  supersede,
} from "../src/engine/supersede.ts";
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

describe("supersession edges: forward-authored, reverse-derived (§4, §6)", () => {
  test("supersedes (full) flips the old doc to superseded and derives the reverse edge", async () => {
    const r = await repo();
    const { newDoc, oldDoc } = await supersede(r.store, {
      newDocPath: "v2.md",
      oldDocPath: "v1.md",
      type: "supersedes",
    });
    expect(newDoc.edges).toContainEqual({
      type: "supersedes",
      target: oldDoc.id,
      derived: false,
    });
    expect(oldDoc.edges).toContainEqual({
      type: "superseded-by",
      source: newDoc.id,
      derived: true,
    });
    expect(oldDoc.lifecycle).toBe("superseded");
  });

  test("amends (partial) keeps the doc active-but-amended and flips named propositions", async () => {
    const r = await repo();
    const { oldDoc } = await supersede(r.store, {
      newDocPath: "v2.md",
      oldDocPath: "v1.md",
      type: "amends",
      propositions: ["prop_a", "prop_b"],
    });
    expect(oldDoc.lifecycle).toBe("amended");
    expect([...amendedPropositions(oldDoc)].sort()).toEqual([
      "prop_a",
      "prop_b",
    ]);
  });

  test("amends requires proposition ids", async () => {
    const r = await repo();
    await expect(
      supersede(r.store, {
        newDocPath: "v2.md",
        oldDocPath: "v1.md",
        type: "amends",
      }),
    ).rejects.toThrow();
  });

  test("edges are idempotent (re-authoring does not duplicate)", async () => {
    const r = await repo();
    await supersede(r.store, {
      newDocPath: "v2.md",
      oldDocPath: "v1.md",
      type: "supersedes",
    });
    await supersede(r.store, {
      newDocPath: "v2.md",
      oldDocPath: "v1.md",
      type: "supersedes",
    });
    const newDoc = await r.store.getDocument(documentIdForPath("v2.md"));
    expect(newDoc?.edges.filter((e) => e.type === "supersedes").length).toBe(1);
  });

  test("retract marks lifecycle retracted (§10)", async () => {
    const r = await repo();
    const doc = await retract(r.store, "old.md");
    expect(doc.lifecycle).toBe("retracted");
  });
});

describe("supersession and code-drift are surfaced together (§4, §6)", () => {
  test("an amended doc with a separately-drifted claim shows both in the banner", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\nexport const B = 2;\n");
    await r.write("doc.md", "# Doc\n\nText.\n");
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

    // Amend proposition A via a newer doc.
    await supersede(r.store, {
      newDocPath: "newer.md",
      oldDocPath: "doc.md",
      type: "amends",
      propositions: [a.proposition.id],
    });
    // Drift proposition B by changing its value.
    await r.write("src/a.ts", "export const A = 1;\nexport const B = 22;\n");

    await runCheck(r.store, { ast: analyzer, write: true });
    const banner = await r.read("doc.md");
    expect(banner).toContain("[amended]");
    expect(banner).toContain(b.proposition.id); // the drifted claim
    expect(banner).toContain(a.proposition.id); // the amended claim
  });
});
