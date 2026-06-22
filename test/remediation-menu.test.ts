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

describe("remediationFor (the verdict→action menu)", () => {
  test("a clean verdict has no remediation", () => {
    expect(remediationFor(input({}))).toBeNull();
  });

  test("orphan is handled before refuted, so an orphaned+refuted claim keeps a withdraw path", () => {
    const rem = remediationFor(
      input({ code: "orphaned", behavior: "refuted" }),
    );
    expect(rem?.recommended).toBe("retire");
    expect(rem?.actions.map((a) => a.id)).toContain("retire");
  });

  test("a refuted claim with intact anchors offers fix-code/fix-claim and never reanchor", () => {
    const rem = remediationFor(input({ code: "changed", behavior: "refuted" }));
    expect(rem?.recommended).toBeNull();
    expect(rem?.actions.map((a) => a.id)).toEqual(["fix-code", "fix-claim"]);
  });

  test("expired on a moved verdict promotes reverify-and-rerecord (a bare reanchor cannot clear expiry)", () => {
    const rem = remediationFor(input({ code: "moved", expired: true }));
    expect(rem?.recommended).toBe("reverify-and-rerecord");
    expect(rem?.actions.map((a) => a.id)).toContain("reverify-and-rerecord");
    expect(rem?.actions.map((a) => a.id)).toContain("reanchor");
  });

  test("expired on a clean verdict recommends reverify-and-rerecord", () => {
    expect(remediationFor(input({ expired: true }))?.recommended).toBe(
      "reverify-and-rerecord",
    );
  });

  test("expired keeps retire recommended for an orphan", () => {
    const rem = remediationFor(input({ code: "orphaned", expired: true }));
    expect(rem?.recommended).toBe("retire");
    expect(rem?.actions.map((a) => a.id)).toContain("reverify-and-rerecord");
  });

  test("expired keeps null recommended for an intent-ambiguous change", () => {
    const rem = remediationFor(input({ code: "changed", expired: true }));
    expect(rem?.recommended).toBeNull();
    expect(rem?.actions.map((a) => a.id)).toContain("reverify-and-rerecord");
  });

  test("moved/ambiguous recommend reanchor; orphan offers supersede", () => {
    expect(remediationFor(input({ code: "moved" }))?.recommended).toBe(
      "reanchor",
    );
    expect(remediationFor(input({ doc: "ambiguous" }))?.recommended).toBe(
      "reanchor",
    );
    expect(ids({ code: "orphaned" })).toContain("supersede");
  });
});

describe("topAction", () => {
  test("returns the recommended action when set", () => {
    expect(topAction(remediationFor(input({ code: "moved" })))?.id).toBe(
      "reanchor",
    );
  });

  test("falls back to the safest/first action when recommended is null", () => {
    const rem = remediationFor(input({ code: "changed" }));
    expect(rem?.recommended).toBeNull();
    expect(topAction(rem)?.id).toBe("retire");
  });

  test("returns null for a clean verdict", () => {
    expect(topAction(null)).toBeNull();
  });
});
