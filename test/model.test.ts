import { describe, expect, test } from "bun:test";
import * as z from "zod";
import { computeGates, isWarnVerdict } from "../src/core/gating.ts";
import {
  Anchor,
  AnchorState,
  Assertion,
  AuthoredTrust,
  BehaviorState,
  Document,
  DocumentLifecycle,
  Enforcement,
  Proposition,
  SCHEMAS,
  Selector,
  SelectorBundle,
  Verdict,
  Verifier,
} from "../src/core/model.ts";
import { PROTOCOL_SCHEMAS } from "../src/resolver/protocol.ts";

describe("canonical model is the single source of truth (§5)", () => {
  test("every SCHEMAS + PROTOCOL_SCHEMAS entry exports to JSON Schema without throwing", () => {
    for (const [name, schema] of Object.entries({
      ...SCHEMAS,
      ...PROTOCOL_SCHEMAS,
    })) {
      expect(
        () =>
          z.toJSONSchema(schema, { target: "draft-2020-12", reused: "ref" }),
        name,
      ).not.toThrow();
    }
  });

  // ── Enum option lists (§10) ────────────────────────────────────────────────

  test("AuthoredTrust options (§10)", () => {
    expect(AuthoredTrust.options).toEqual(["verified", "inferred", "assumed"]);
  });

  test("Enforcement options (§4/§9/§10)", () => {
    expect(Enforcement.options).toEqual(["suggested", "enforced", "retired"]);
  });

  test("AnchorState options — one vocabulary, both sides (§10, ADR-001)", () => {
    expect(AnchorState.options).toEqual([
      "unchanged",
      "moved",
      "changed",
      "ambiguous",
      "orphaned",
    ]);
  });

  test("BehaviorState options (§10/§17.6)", () => {
    expect(BehaviorState.options).toEqual([
      "unverified",
      "at-risk",
      "supported",
      "refuted",
    ]);
  });

  test("DocumentLifecycle options (§10)", () => {
    expect(DocumentLifecycle.options).toEqual([
      "active",
      "amended",
      "superseded",
      "archived",
      "retracted",
    ]);
  });

  // ── Selector union: 7 variants now (added inline-id) ───────────────────────

  test("Selector is a discriminated union of exactly 7 variants (§4)", () => {
    const js = z.toJSONSchema(Selector) as {
      anyOf?: unknown[];
      oneOf?: unknown[];
    };
    const variants = js.oneOf ?? js.anyOf;
    expect(Array.isArray(variants)).toBe(true);
    // text-quote, text-position, ast-node, value, inline-id, path, glob
    expect(variants?.length).toBe(7);
  });

  test("Selector accepts the inline-id variant (owned-doc localization marker)", () => {
    expect(
      Selector.safeParse({ kind: "inline-id", id: "hibi:claim:abc" }).success,
    ).toBe(true);
  });

  test("an invalid selector kind is rejected", () => {
    expect(Selector.safeParse({ kind: "bogus" }).success).toBe(false);
  });

  // ── Bidirectional Anchor + SelectorBundle round-trip ───────────────────────

  test("SelectorBundle is { file, selectors[] } with min 1 selector", () => {
    const ok = SelectorBundle.safeParse({
      file: "a.ts",
      selectors: [{ kind: "text-quote", exact: "x", prefix: "", suffix: "" }],
    });
    expect(ok.success).toBe(true);

    const empty = SelectorBundle.safeParse({ file: "a.ts", selectors: [] });
    expect(empty.success).toBe(false);
  });

  test("Anchor is bidirectional { doc: SelectorBundle, code: SelectorBundle[] }", () => {
    const a = Anchor.parse({
      doc: {
        file: "README.md",
        selectors: [
          { kind: "text-quote", exact: "the sentence", prefix: "", suffix: "" },
        ],
      },
      code: [
        {
          file: "src/x.ts",
          selectors: [
            { kind: "text-quote", exact: "fn()", prefix: "", suffix: "" },
          ],
        },
      ],
    });
    expect(a.doc.file).toBe("README.md");
    expect(a.code).toHaveLength(1);
    expect(a.code[0]?.file).toBe("src/x.ts");
  });

  test("Anchor defaults code to [] (doc-only suggested claim awaiting a target, §9)", () => {
    const a = Anchor.parse({
      doc: {
        file: "README.md",
        selectors: [{ kind: "text-quote", exact: "s", prefix: "", suffix: "" }],
      },
    });
    expect(a.code).toEqual([]);
  });

  test("the OLD flat anchor shape { file, selectors } no longer validates", () => {
    const flat = {
      file: "a.ts",
      selectors: [{ kind: "text-quote", exact: "x" }],
    };
    expect(Anchor.safeParse(flat).success).toBe(false);
  });

  // ── Assertion: requires/derives enforcement ────────────────────────────────

  test("a valid Assertion round-trips with the bidirectional anchor", () => {
    const a = {
      id: "asrt_1",
      propositionId: "prop_1",
      documentId: "doc_1",
      owner: "x",
      ref: "r",
      anchor: {
        doc: {
          file: "README.md",
          selectors: [
            { kind: "text-quote", exact: "x", prefix: "", suffix: "" },
          ],
        },
        code: [
          {
            file: "a.ts",
            selectors: [
              { kind: "text-quote", exact: "y", prefix: "", suffix: "" },
            ],
          },
        ],
      },
    };
    expect(() => Assertion.parse(a)).not.toThrow();
  });

  test("Assertion enforcement defaults to 'suggested'", () => {
    const a = Assertion.parse({
      id: "a",
      propositionId: "p",
      documentId: "d",
      owner: "x",
      ref: "r",
      anchor: {
        doc: {
          file: "README.md",
          selectors: [
            { kind: "text-quote", exact: "x", prefix: "", suffix: "" },
          ],
        },
      },
    });
    expect(a.enforcement).toBe("suggested");
    expect(a.verifiers).toEqual([]);
  });

  test("Assertion rejects an out-of-enum enforcement", () => {
    expect(
      Assertion.safeParse({
        id: "a",
        propositionId: "p",
        documentId: "d",
        owner: "x",
        ref: "r",
        anchor: {
          doc: {
            file: "README.md",
            selectors: [
              { kind: "text-quote", exact: "x", prefix: "", suffix: "" },
            ],
          },
        },
        enforcement: "mandatory",
      }).success,
    ).toBe(false);
  });

  test("Assertion accepts behavioral, verifiers and the redefined behaviorScope", () => {
    const a = Assertion.parse({
      id: "a",
      propositionId: "p",
      documentId: "d",
      owner: "x",
      ref: "r",
      anchor: {
        doc: {
          file: "README.md",
          selectors: [
            { kind: "text-quote", exact: "x", prefix: "", suffix: "" },
          ],
        },
      },
      behavioral: true,
      verifiers: [{ kind: "command", ref: "bun test" }],
      behaviorScope: { include: ["fixtures/**"] },
      evidenceBaseline: { "src/x.ts": "deadbeef" },
    });
    expect(a.behavioral).toBe(true);
    expect(a.verifiers[0]?.ref).toBe("bun test");
    expect(a.behaviorScope?.depth).toBe(1); // default
    expect(a.evidenceBaseline?.["src/x.ts"]).toBe("deadbeef");
  });

  test("Verifier.kind is an open string (no closed taxonomy — D13)", () => {
    // Any non-empty string is accepted; there is no enum of kinds.
    for (const kind of ["command", "metamorphic", "my-custom-runner"]) {
      expect(Verifier.safeParse({ kind, ref: "x" }).success).toBe(true);
    }
    // Empty kind is still rejected (min length 1).
    expect(Verifier.safeParse({ kind: "", ref: "x" }).success).toBe(false);
    // The model exports no ClaimKind / behavioral-kind enum.
    const model = z as unknown as Record<string, unknown>;
    expect("ClaimKind" in model).toBe(false);
  });

  test("no-contradiction invariant: behavioral:false + verifiers[] is rejected (D12)", () => {
    const base = {
      id: "a",
      propositionId: "p",
      documentId: "d",
      owner: "x",
      ref: "r",
      anchor: {
        doc: {
          file: "README.md",
          selectors: [
            { kind: "text-quote", exact: "x", prefix: "", suffix: "" },
          ],
        },
      },
    } as const;

    const bad = Assertion.safeParse({
      ...base,
      behavioral: false,
      verifiers: [{ kind: "command", ref: "bun test" }],
    });
    expect(bad.success).toBe(false);
    // The record-time error names BOTH legitimate noise levers.
    const message = bad.success ? "" : (bad.error.issues[0]?.message ?? "");
    expect(message).toContain("behaviorScope");
    expect(message).toContain("hibi ignore");

    // behavioral:false with an EMPTY verifiers[] is fine (the opt-out path).
    expect(
      Assertion.safeParse({ ...base, behavioral: false, verifiers: [] })
        .success,
    ).toBe(true);
    // behavioral:true with verifiers[] is fine.
    expect(
      Assertion.safeParse({
        ...base,
        behavioral: true,
        verifiers: [{ kind: "command", ref: "bun test" }],
      }).success,
    ).toBe(true);
  });

  test("an invalid selector kind inside an anchor bundle is rejected", () => {
    expect(
      Assertion.safeParse({
        id: "a",
        propositionId: "p",
        documentId: "d",
        owner: "x",
        ref: "r",
        anchor: {
          doc: { file: "a.ts", selectors: [{ kind: "bogus" }] },
        },
      }).success,
    ).toBe(false);
  });

  test("an empty selector bundle on the doc side is rejected (min 1)", () => {
    expect(
      Assertion.safeParse({
        id: "a",
        propositionId: "p",
        documentId: "d",
        owner: "x",
        ref: "r",
        anchor: { doc: { file: "a.ts", selectors: [] } },
      }).success,
    ).toBe(false);
  });

  // ── Document defaults ──────────────────────────────────────────────────────

  test("Document applies defaults (lifecycle active, edges [])", () => {
    const d = Document.parse({ id: "d", path: "x.md" });
    expect(d.lifecycle).toBe("active");
    expect(d.edges).toEqual([]);
  });

  // ── Proposition uses textCache (was .text) ─────────────────────────────────

  test("Proposition uses textCache and requires authoredTrust from the enum", () => {
    const p = Proposition.parse({
      id: "p",
      textCache: "the documented sentence",
      authoredTrust: "verified",
      fingerprint: "f",
    });
    expect(p.textCache).toBe("the documented sentence");

    // Old `.text` key no longer satisfies the schema (textCache is required).
    expect(
      Proposition.safeParse({
        id: "p",
        text: "t",
        authoredTrust: "verified",
        fingerprint: "f",
      }).success,
    ).toBe(false);

    // Out-of-enum authoredTrust is rejected.
    expect(
      Proposition.safeParse({
        id: "p",
        textCache: "t",
        authoredTrust: "nope",
        fingerprint: "f",
      }).success,
    ).toBe(false);
  });

  // ── Two-axis Verdict ───────────────────────────────────────────────────────

  test("Verdict (ephemeral, two-axis) validates", () => {
    const v: z.infer<typeof Verdict> = {
      assertionId: "a",
      propositionId: "p",
      documentId: "d",
      doc: "unchanged",
      code: "changed",
      behavior: "at-risk",
      expired: false,
      gates: false,
      suppressed: false,
      remediation: {
        recommended: null,
        actions: [
          {
            id: "retire",
            title: "Retire the claim",
            applicability: "manual",
            effect: "deterministic",
            rationale: "the claim is obsolete",
            command: "hibi retire a",
          },
        ],
      },
      evidence: {
        docRegion: { start: 0, end: 5 },
        codeRegions: [{ start: 10, end: 20 }],
        confidence: 0.3,
        selectorScores: [],
        changedEvidence: [],
        ref: "abc",
      },
      notes: [],
      advisories: [],
    };
    expect(() => Verdict.parse(v)).not.toThrow();
  });

  test("Verdict behavior is optional (absent on non-behavioral claims)", () => {
    const v = Verdict.parse({
      assertionId: "a",
      propositionId: "p",
      documentId: "d",
      doc: "unchanged",
      code: "unchanged",
      expired: false,
      gates: false,
      remediation: null,
      evidence: { confidence: 1, codeRegions: [], selectorScores: [] },
    });
    expect(v.behavior).toBeUndefined();
    expect(v.evidence.changedEvidence).toEqual([]); // default
    expect(v.remediation).toBeNull(); // null on a clean verdict
  });

  test("Verdict rejects an out-of-enum anchor state", () => {
    expect(
      Verdict.safeParse({
        assertionId: "a",
        propositionId: "p",
        documentId: "d",
        doc: "fresh", // old word — no longer a state
        code: "unchanged",
        expired: false,
        gates: false,
        remediation: null,
        evidence: { confidence: 1, codeRegions: [], selectorScores: [] },
      }).success,
    ).toBe(false);
  });
});

// ── ADR-001 fitness functions (executable architecture invariants) ───────────

describe("ADR-001 fitness functions", () => {
  test("AnchorState is exactly {unchanged, moved, changed, ambiguous, orphaned}", () => {
    expect(new Set(AnchorState.options)).toEqual(
      new Set(["unchanged", "moved", "changed", "ambiguous", "orphaned"]),
    );
  });

  test("no AnchorState value carries a doc-/code-/behavior- prefix (parallelism invariant)", () => {
    for (const state of AnchorState.options) {
      expect(state).not.toMatch(/^(doc|code|behavior)-/);
    }
  });

  test("BehaviorState ∩ AuthoredTrust = ∅", () => {
    const behavior = new Set<string>(BehaviorState.options);
    for (const trust of AuthoredTrust.options) {
      expect(behavior.has(trust)).toBe(false);
    }
  });

  test("the words drift/stale/ghost/fresh appear in NO machine enum", () => {
    const banned = ["drift", "stale", "ghost", "fresh"];
    const allEnumValues = [
      ...AnchorState.options,
      ...BehaviorState.options,
      ...AuthoredTrust.options,
      ...Enforcement.options,
      ...DocumentLifecycle.options,
    ];
    for (const value of allEnumValues) {
      for (const word of banned) {
        expect(value.includes(word)).toBe(false);
      }
    }
  });

  test("only refuted + changed/orphaned/ambiguous/expired gate; moved/at-risk never gate", () => {
    const base = {
      doc: "unchanged",
      code: "unchanged",
      expired: false,
    } as const;

    // Gating anchor states gate an enforced claim, either side.
    for (const state of ["changed", "orphaned", "ambiguous"] as const) {
      expect(computeGates({ ...base, code: state }, "enforced")).toBe(true);
      expect(computeGates({ ...base, doc: state }, "enforced")).toBe(true);
    }
    // expired gates an enforced claim.
    expect(computeGates({ ...base, expired: true }, "enforced")).toBe(true);
    // refuted gates an enforced claim.
    expect(computeGates({ ...base, behavior: "refuted" }, "enforced")).toBe(
      true,
    );

    // moved never gates — it warns.
    expect(computeGates({ ...base, code: "moved" }, "enforced")).toBe(false);
    expect(
      isWarnVerdict({ ...base, code: "moved", gates: false }, "enforced"),
    ).toBe(true);
    // at-risk never gates — it warns.
    expect(computeGates({ ...base, behavior: "at-risk" }, "enforced")).toBe(
      false,
    );
    expect(
      isWarnVerdict({ ...base, behavior: "at-risk", gates: false }, "enforced"),
    ).toBe(true);
  });

  test("only ENFORCED claims gate; suggested/retired never gate", () => {
    const gating = {
      doc: "changed",
      code: "orphaned",
      expired: true,
      behavior: "refuted",
    } as const;
    expect(computeGates(gating, "enforced")).toBe(true);
    for (const e of ["suggested", "retired"] as const) {
      expect(computeGates(gating, e)).toBe(false);
      expect(isWarnVerdict({ ...gating, gates: false }, e)).toBe(false);
    }
  });
});
