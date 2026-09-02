import { describe, expect, test } from "bun:test";
import type { RemediationInput } from "../src/core/remediation.ts";
import { remediationFor, topAction } from "../src/core/remediation.ts";

/** Minimal RemediationInput with everything clean unless overridden. */
function input(over: Partial<RemediationInput>): RemediationInput {
  return {
    assertionId: "asrt_x",
    doc: "unchanged",
    code: "unchanged",
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
    const rem = remediationFor(input({ code: "changed" }));
    expect(rem?.recommended).toBe("update-claim");
    expect(rem?.actions.map((a) => a.id)).toEqual([
      "update-claim",
      "reanchor",
      "retire",
    ]);
    expect(rem?.actions[0]?.command).toBe("hibi reanchor asrt_x");
    expect(rem?.actions[2]?.command).toBe("hibi retire asrt_x");
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
    const rem = remediationFor(input({ doc: "changed", code: "changed" }));
    expect(rem?.recommended).toBe("reconcile");
    expect(rem?.actions.map((a) => a.id)).toEqual([
      "reconcile",
      "reanchor",
      "retire",
    ]);
  });

  test("an orphan recommends reanchor with --suggest and offers retire and supersede", () => {
    const rem = remediationFor(input({ code: "orphaned" }));
    expect(rem?.recommended).toBe("reanchor");
    expect(rem?.actions.map((a) => a.id)).toEqual([
      "reanchor",
      "retire",
      "supersede",
    ]);
    expect(rem?.actions[0]?.command).toBe("hibi reanchor asrt_x --suggest");
    expect(rem?.actions[2]?.command).toBeUndefined();
    expect(ids({ doc: "orphaned" })).toEqual([
      "reanchor",
      "retire",
      "supersede",
    ]);
  });

  test("orphan is handled before refuted, so an orphaned+refuted claim keeps the orphan menu", () => {
    const rem = remediationFor(
      input({ code: "orphaned", behavior: "refuted" }),
    );
    expect(rem?.recommended).toBe("reanchor");
    expect(rem?.actions.map((a) => a.id)).toContain("retire");
  });

  test("a refuted claim with intact anchors offers fix-code/fix-claim and never reanchor", () => {
    const rem = remediationFor(input({ code: "changed", behavior: "refuted" }));
    expect(rem?.recommended).toBeNull();
    expect(rem?.actions.map((a) => a.id)).toEqual(["fix-code", "fix-claim"]);
  });

  test("moved recommends reanchor; ambiguous recommends a wider reanchor", () => {
    const moved = remediationFor(input({ code: "moved" }));
    expect(moved?.recommended).toBe("reanchor");
    expect(moved?.actions.map((a) => a.id)).toEqual(["reanchor"]);
    expect(moved?.actions[0]?.title).toBe("Reanchor to the new position");

    const amb = remediationFor(input({ doc: "ambiguous" }));
    expect(amb?.recommended).toBe("reanchor");
    expect(amb?.actions[0]?.title).toBe("Reanchor to a unique span");
  });

  test("expired on a clean verdict recommends reverify-and-rerecord", () => {
    const rem = remediationFor(input({ expired: true }));
    expect(rem?.recommended).toBe("reverify-and-rerecord");
    expect(rem?.actions.map((a) => a.id)).toEqual(["reverify-and-rerecord"]);
  });

  test("expired on a moved verdict promotes reverify-and-rerecord and keeps reanchor", () => {
    const rem = remediationFor(input({ code: "moved", expired: true }));
    expect(rem?.recommended).toBe("reverify-and-rerecord");
    expect(rem?.actions.map((a) => a.id)).toEqual([
      "reanchor",
      "reverify-and-rerecord",
    ]);
  });

  test("expired on an orphan promotes reverify-and-rerecord", () => {
    const rem = remediationFor(input({ code: "orphaned", expired: true }));
    expect(rem?.recommended).toBe("reverify-and-rerecord");
    expect(rem?.actions.map((a) => a.id)).toContain("reverify-and-rerecord");
    expect(rem?.actions.map((a) => a.id)).toContain("supersede");
  });

  test("expired keeps update-claim recommended for a code change", () => {
    const rem = remediationFor(input({ code: "changed", expired: true }));
    expect(rem?.recommended).toBe("update-claim");
    expect(rem?.actions.map((a) => a.id)).toContain("reverify-and-rerecord");
  });
});

describe("topAction", () => {
  test("returns the recommended action when set", () => {
    expect(topAction(remediationFor(input({ code: "moved" })))?.id).toBe(
      "reanchor",
    );
    expect(topAction(remediationFor(input({ code: "changed" })))?.id).toBe(
      "update-claim",
    );
  });

  test("falls back to the first action when recommended is null", () => {
    const rem = remediationFor(input({ code: "changed", behavior: "refuted" }));
    expect(rem?.recommended).toBeNull();
    expect(topAction(rem)?.id).toBe("fix-code");
  });

  test("returns null for a clean verdict", () => {
    expect(topAction(null)).toBeNull();
  });
});
