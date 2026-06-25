/**
 * `doctor` (§9, Tier-1 silent-orphan hardening) — the store-health projection
 * surfaces the dead state `check` hides: orphaned anchors, `suggested` claims
 * with no precise code side, claims stranded on a lifecycle-flagged document, and
 * propositions that collapsed onto one fingerprint.
 */

import { afterEach, describe, expect, test } from "bun:test";
import { rm } from "node:fs/promises";
import { join } from "node:path";
import type { CheckReport } from "../src/engine/check.ts";
import { buildDoctorReport } from "../src/engine/doctor.ts";
import { documentIdForPath } from "../src/engine/record.ts";
import { supersede } from "../src/engine/supersede.ts";
import type {
  Assertion,
  Document,
  Proposition,
  Verdict,
} from "../src/index.ts";
import { Engine } from "../src/index.ts";
import { makeRepo, record, type TempRepo } from "./helpers.ts";

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

/** Minimal verdict carrying only the fields the doctor projection reads. */
function verdict(over: Partial<Verdict>): Verdict {
  return {
    assertionId: "asrt_x",
    propositionId: "prop_x",
    documentId: "doc_x",
    doc: "unchanged",
    code: "unchanged",
    expired: false,
    gates: false,
    remediation: null,
    evidence: {
      codeRegions: [],
      confidence: 1,
      selectorScores: [],
      changedEvidence: [],
    },
    notes: [],
    advisories: [],
    ...over,
  } as Verdict;
}

/** Minimal assertion carrying only the fields the doctor projection reads. */
function assertion(over: Partial<Assertion>): Assertion {
  return {
    id: "asrt_x",
    propositionId: "prop_x",
    documentId: "doc_x",
    owner: "tester",
    ref: "r",
    anchor: {
      doc: {
        file: "a.md",
        selectors: [{ kind: "text-quote", exact: "x", prefix: "", suffix: "" }],
      },
      code: [
        {
          file: "src/a.ts",
          selectors: [
            { kind: "text-quote", exact: "y", prefix: "", suffix: "" },
          ],
        },
      ],
    },
    enforcement: "enforced",
    verifiers: [],
    attrs: {},
    ...over,
  } as Assertion;
}

function doc(over: Partial<Document>): Document {
  return {
    id: "doc_x",
    path: "a.md",
    lifecycle: "active",
    edges: [],
    ...over,
  } as Document;
}

function proposition(over: Partial<Proposition>): Proposition {
  return {
    id: "prop_x",
    textCache: "x",
    authoredTrust: "inferred",
    fingerprint: "fp",
    ...over,
  } as Proposition;
}

describe("buildDoctorReport (pure projection)", () => {
  test("each category is populated and `healthy` is false", () => {
    const report = {
      ref: "WORKTREE",
      verdicts: [
        verdict({ assertionId: "asrt_orphan", code: "orphaned" }),
        verdict({ assertionId: "asrt_sugg" }),
        verdict({ assertionId: "asrt_stale" }),
        verdict({ assertionId: "asrt_dup1" }),
        verdict({ assertionId: "asrt_dup2" }),
      ],
      documents: [],
      summary: {} as CheckReport["summary"],
      exitCode: 0,
    } satisfies CheckReport;

    const assertions: Assertion[] = [
      assertion({ id: "asrt_orphan", propositionId: "p_o" }),
      // suggested with a coarse-only (path) code side → no precise code.
      assertion({
        id: "asrt_sugg",
        propositionId: "p_s",
        documentId: documentIdForPath("s.md"),
        enforcement: "suggested",
        anchor: {
          doc: {
            file: "s.md",
            selectors: [
              { kind: "text-quote", exact: "x", prefix: "", suffix: "" },
            ],
          },
          code: [
            {
              file: "src/a.ts",
              selectors: [{ kind: "path", path: "src/a.ts" }],
            },
          ],
        },
      }),
      // a live claim on a superseded document.
      assertion({
        id: "asrt_stale",
        propositionId: "p_st",
        documentId: documentIdForPath("old.md"),
      }),
      // two claims on two propositions that share a fingerprint.
      assertion({ id: "asrt_dup1", propositionId: "p_d1" }),
      assertion({ id: "asrt_dup2", propositionId: "p_d2" }),
    ];

    const documents: Document[] = [
      doc({
        id: documentIdForPath("old.md"),
        path: "old.md",
        lifecycle: "superseded",
      }),
    ];

    const propositions: Proposition[] = [
      proposition({ id: "p_o", fingerprint: "f_o" }),
      proposition({ id: "p_s", fingerprint: "f_s" }),
      proposition({ id: "p_st", fingerprint: "f_st" }),
      proposition({ id: "p_d1", fingerprint: "shared" }),
      proposition({ id: "p_d2", fingerprint: "shared" }),
    ];

    const out = buildDoctorReport(report, assertions, documents, propositions);
    expect(out.orphanedAnchors.map((o) => o.claimId)).toEqual(["asrt_orphan"]);
    expect(out.suggestedNoCode.map((s) => s.claimId)).toEqual(["asrt_sugg"]);
    expect(out.staleDocClaims.map((s) => s.claimId)).toEqual(["asrt_stale"]);
    expect(out.staleDocClaims[0]?.lifecycle).toBe("superseded");
    expect(out.duplicatePropositions).toHaveLength(1);
    expect(out.duplicatePropositions[0]?.claimIds.sort()).toEqual([
      "asrt_dup1",
      "asrt_dup2",
    ]);
    expect(out.healthy).toBe(false);
  });

  test("an empty store is healthy", () => {
    const report = {
      ref: "WORKTREE",
      verdicts: [],
      documents: [],
      summary: {} as CheckReport["summary"],
      exitCode: 0,
    } satisfies CheckReport;
    const out = buildDoctorReport(report, [], [], []);
    expect(out.healthy).toBe(true);
    expect(out.counts).toEqual({
      orphanedAnchors: 0,
      suggestedNoCode: 0,
      staleDocClaims: 0,
      duplicatePropositions: 0,
    });
  });
});

describe("Engine.doctor (§9 integration)", () => {
  test("surfaces an orphan + a stranded claim from a live store", async () => {
    const r = await repo();
    await r.write("src/gone.ts", "export const X = 1;\n");
    await r.write("o.md", "# O\n\nOrphan claim here.\n");
    const orphan = await record(r, {
      doc: "o.md",
      text: "Orphan claim here.",
      file: "src/gone.ts",
      quote: "X = 1",
    });
    // Delete the code file → the code side orphans on the live check.
    await rm(join(r.root, "src/gone.ts"));

    // A claim left on a superseded document (stranded).
    await r.write("src/a.ts", "export const Y = 2;\n");
    await r.write("old.md", "# Old\n\nStranded claim.\n");
    const stranded = await record(r, {
      doc: "old.md",
      text: "Stranded claim.",
      file: "src/a.ts",
      quote: "Y = 2",
    });
    await supersede(r.store, {
      newDocPath: "new.md",
      oldDocPath: "old.md",
      type: "supersedes",
    });

    const engine = await Engine.open(r.root);
    const report = await engine.doctor();
    expect(report.healthy).toBe(false);
    expect(
      report.orphanedAnchors.some((o) => o.claimId === orphan.assertion.id),
    ).toBe(true);
    expect(
      report.staleDocClaims.some((s) => s.claimId === stranded.assertion.id),
    ).toBe(true);
  });
});
