import { describe, expect, test } from "bun:test";
import type { RemediationInput } from "../src/core/remediation.ts";
import { remediationFor, topAction } from "../src/core/remediation.ts";

/** Minimal RemediationInput with everything clean unless overridden. */
function input(over: Partial<RemediationInput>): RemediationInput {
  return {
    claimId: "clm_x",
    doc: "unchanged",
    evidence: "unchanged",
    expired: false,
    ...over,
  };
}

const ids = (over: Partial<RemediationInput>) =>
  (remediationFor(input(over))?.actions ?? []).map((a) => a.id);

describe("remediationFor (the verdict-to-action menu)", () => {
  test("a clean verdict has no remediation", () => {
    expect(remediationFor(input({}))).toBeNull();
  });

  test("code changed recommends update-claim over reanchor and retire", () => {
    const rem = remediationFor(input({ evidence: "changed" }));
    expect(rem?.recommended).toBe("update-claim");
    expect(rem?.actions.map((a) => a.id)).toEqual([
      "update-claim",
      "reanchor",
      "retire",
    ]);
    expect(rem?.actions[0]?.command).toBe("hibi reanchor clm_x");
    expect(rem?.actions[2]?.command).toBe("hibi retire clm_x");
  });

  test("doc changed recommends reverify-doc", () => {
    const rem = remediationFor(input({ doc: "changed" }));
    expect(rem?.recommended).toBe("reverify-doc");
    expect(rem?.actions.map((a) => a.id)).toEqual([
      "reverify-doc",
      "reanchor",
      "retire",
    ]);
  });

  test("both sides changed recommends reconcile", () => {
    const rem = remediationFor(input({ doc: "changed", evidence: "changed" }));
    expect(rem?.recommended).toBe("reconcile");
    expect(rem?.actions.map((a) => a.id)).toEqual([
      "reconcile",
      "reanchor",
      "retire",
    ]);
  });

  test("an orphan recommends reanchor with --suggest and offers retire and supersede", () => {
    const rem = remediationFor(input({ evidence: "orphaned" }));
    expect(rem?.recommended).toBe("reanchor");
    expect(rem?.actions.map((a) => a.id)).toEqual([
      "reanchor",
      "retire",
      "supersede",
    ]);
    expect(rem?.actions[0]?.command).toBe("hibi reanchor clm_x --suggest");
    expect(rem?.actions[2]?.command).toBeUndefined();
    expect(ids({ doc: "orphaned" })).toEqual([
      "reanchor",
      "retire",
      "supersede",
    ]);
  });

  test("orphan is handled before refuted, so an orphaned+refuted claim keeps the orphan menu", () => {
    const rem = remediationFor(
      input({ evidence: "orphaned", judge: "refuted" }),
    );
    expect(rem?.recommended).toBe("reanchor");
    expect(rem?.actions.map((a) => a.id)).toContain("retire");
  });

  test("a refuted claim with intact anchors offers fix-evidence/fix-claim and never reanchor", () => {
    const rem = remediationFor(
      input({ evidence: "changed", judge: "refuted" }),
    );
    expect(rem?.recommended).toBeNull();
    expect(rem?.actions.map((a) => a.id)).toEqual([
      "fix-evidence",
      "fix-claim",
    ]);
  });

  test("moved recommends reanchor; ambiguous recommends a wider reanchor", () => {
    const moved = remediationFor(input({ evidence: "moved" }));
    expect(moved?.recommended).toBe("reanchor");
    expect(moved?.actions.map((a) => a.id)).toEqual(["reanchor"]);
    expect(moved?.actions[0]?.title).toBe("Reanchor to the new position");

    const amb = remediationFor(input({ doc: "ambiguous" }));
    expect(amb?.recommended).toBe("reanchor");
    expect(amb?.actions[0]?.title).toBe("Reanchor to a unique span");
  });

  test("expired on a clean verdict offers a human attest or evidence, no recommendation", () => {
    const rem = remediationFor(input({ expired: true }));
    expect(rem?.recommended).toBeNull();
    expect(rem?.actions.map((a) => a.id)).toEqual(["attest", "evidence"]);
    expect(rem?.actions[0]?.command).toBe(
      'hibi attest clm_x --by human:<you> --note "<what you checked>"',
    );
    expect(rem?.actions[1]?.command).toBe(
      'echo \'[{"claimId":"clm_x","evidence":"<text>","source":"<where>","summary":"<one line>"}]\' | hibi evidence --from-file - --by agent:<name>',
    );
  });

  test("evidence is offered only for attested claims", () => {
    for (const kind of ["code", "reference"] as const) {
      const expired = remediationFor(input({ kind, expired: true }));
      expect(expired?.actions.map((a) => a.id)).toEqual(["attest"]);
      const review = remediationFor(input({ kind, judge: "review" }));
      expect(review?.actions.map((a) => a.id)).toEqual(["ack"]);
    }
    const attested = remediationFor(
      input({ kind: "attested", judge: "review" }),
    );
    expect(attested?.actions.map((a) => a.id)).toEqual(["ack", "evidence"]);
  });

  test("expired outranks moved: no recommendation, reanchor kept last", () => {
    const rem = remediationFor(input({ evidence: "moved", expired: true }));
    expect(rem?.recommended).toBeNull();
    expect(rem?.actions.map((a) => a.id)).toEqual([
      "attest",
      "evidence",
      "reanchor",
    ]);
  });

  test("an orphan outranks expired", () => {
    const rem = remediationFor(input({ evidence: "orphaned", expired: true }));
    expect(rem?.recommended).toBe("reanchor");
    expect(rem?.actions.map((a) => a.id)).toContain("attest");
    expect(rem?.actions.map((a) => a.id)).toContain("supersede");
  });

  test("expired keeps update-claim recommended for a code change", () => {
    const rem = remediationFor(input({ evidence: "changed", expired: true }));
    expect(rem?.recommended).toBe("update-claim");
    expect(rem?.actions.map((a) => a.id)).toContain("attest");
  });

  test("review offers ack then evidence", () => {
    const rem = remediationFor(input({ judge: "review" }));
    expect(rem?.recommended).toBeNull();
    expect(rem?.actions.map((a) => a.id)).toEqual(["ack", "evidence"]);
    expect(rem?.actions[0]?.command).toBe(
      'hibi attest clm_x --by <actor> --ack review --note "<why>"',
    );
  });

  test("conflicts recommend reconcile naming the other claim, then one ack per other claim", () => {
    const rem = remediationFor(
      input({
        conflicts: [{ claimId: "clm_other" }, { claimId: "clm_third" }],
      }),
    );
    expect(rem?.recommended).toBe("reconcile");
    expect(rem?.actions.map((a) => a.id)).toEqual(["reconcile", "ack", "ack"]);
    expect(rem?.actions[0]?.title).toBe("Reconcile with clm_other, clm_third");
    expect(rem?.actions.slice(1).map((a) => a.command)).toEqual([
      'hibi attest clm_x --by <actor> --ack conflict:clm_other --note "<why>"',
      'hibi attest clm_x --by <actor> --ack conflict:clm_third --note "<why>"',
    ]);
  });

  test("pending and unavailable add no action", () => {
    expect(remediationFor(input({ judge: "pending" }))).toBeNull();
    expect(remediationFor(input({ evidence: "unavailable" }))).toBeNull();
  });
});

describe("topAction", () => {
  test("returns the recommended action when set", () => {
    expect(topAction(remediationFor(input({ evidence: "moved" })))?.id).toBe(
      "reanchor",
    );
    expect(topAction(remediationFor(input({ evidence: "changed" })))?.id).toBe(
      "update-claim",
    );
  });

  test("falls back to the first action when recommended is null", () => {
    const rem = remediationFor(
      input({ evidence: "changed", judge: "refuted" }),
    );
    expect(rem?.recommended).toBeNull();
    expect(topAction(rem)?.id).toBe("fix-evidence");
  });

  test("returns null for a clean verdict", () => {
    expect(topAction(null)).toBeNull();
  });
});
