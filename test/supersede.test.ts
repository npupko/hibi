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
    const { document } = await retract(r.store, "old.md");
    expect(document.lifecycle).toBe("retracted");
  });
});

describe("lifecycle ops report stranded claims (Tier-1 silent-orphan hardening)", () => {
  test("supersede lists the live claims left on the old document", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("v1.md", "# V1\n\nA is one.\n");
    const claim = await record(r, {
      doc: "v1.md",
      text: "A is one.",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const { strandedClaims } = await supersede(r.store, {
      newDocPath: "v2.md",
      oldDocPath: "v1.md",
      type: "supersedes",
    });
    expect(strandedClaims).toEqual([claim.assertion.id]);
  });

  test("retract lists the live claims left on the retracted document", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("gone.md", "# Gone\n\nA is one.\n");
    const claim = await record(r, {
      doc: "gone.md",
      text: "A is one.",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const { strandedClaims } = await retract(r.store, "gone.md");
    expect(strandedClaims).toEqual([claim.assertion.id]);
  });

  test("a retired claim is not counted as stranded", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("v1.md", "# V1\n\nA is one.\n");
    const claim = await record(r, {
      doc: "v1.md",
      text: "A is one.",
      file: "src/a.ts",
      quote: "A = 1",
    });
    // Retire the only claim, then supersede — it must not be reported as stranded.
    await r.store.putAssertion({ ...claim.assertion, enforcement: "retired" });
    const { strandedClaims } = await supersede(r.store, {
      newDocPath: "v2.md",
      oldDocPath: "v1.md",
      type: "supersedes",
    });
    expect(strandedClaims).toEqual([]);
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
    // B is ENFORCED so its code-side drift banners on its own merit (a
    // `code:changed` suspect entry), independent of any document lifecycle.
    const b = await record(r, {
      doc: "doc.md",
      text: "B is two",
      file: "src/a.ts",
      quote: "B = 2",
      trust: "verified",
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

    const report = await runCheck(r.store, { ast: analyzer, write: true });

    // The old doc is amended at the lifecycle level, and B's enforced code side
    // drifts: the machine report carries both signals.
    const docReport = report.documents.find((d) => d.path === "doc.md");
    expect(docReport?.lifecycle).toBe("amended");
    const statuses = docReport?.suspect.map((s) => s.status) ?? [];
    expect(statuses).toContain("code:changed"); // B's enforced drift

    // The rendered banner surfaces both: the human `[amended]` lifecycle copy
    // and the side-tagged code-drift entry, plus both proposition ids.
    const banner = await r.read("doc.md");
    expect(banner).toContain("[amended]"); // human lifecycle copy
    expect(banner).toContain("[code:changed]"); // B's side-tagged drift
    expect(banner).toContain(b.proposition.id); // the drifted claim
    expect(banner).toContain(a.proposition.id); // the amended claim
  });

  // A claim suspect ONLY because its document is amended (otherwise
  // doc:unchanged / code:unchanged, not expired) must report the `amended`
  // lifecycle tag as its suspect status — `worstStatus` folds the document's
  // lifecycle tags in, rather than falling back to the literal `expired`.
  test("an amended-only claim reports the `amended` suspect status, not `expired`", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("doc.md", "# Doc\n\nText.\n");
    const a = await record(r, {
      doc: "doc.md",
      text: "A is one",
      file: "src/a.ts",
      quote: "A = 1",
    });
    await supersede(r.store, {
      newDocPath: "newer.md",
      oldDocPath: "doc.md",
      type: "amends",
      propositions: [a.proposition.id],
    });

    const report = await runCheck(r.store, { ast: analyzer, write: true });
    const docReport = report.documents.find((d) => d.path === "doc.md");
    const entry = docReport?.suspect.find(
      (s) => s.propositionId === a.proposition.id,
    );
    // The claim is neither expired nor anchor-suspect — its status is lifecycle.
    expect(entry?.status).toBe("amended");
  });
});
