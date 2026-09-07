import { describe, expect, test } from "bun:test";
import * as z from "zod";
import { computeGates, isWarnVerdict } from "../src/core/gating.ts";
import {
  Anchor,
  AnchorState,
  Assertion,
  BehaviorState,
  ChangedEvidenceKind,
  Document,
  DocumentLifecycle,
  Edge,
  Enforcement,
  MODEL_VERSION,
  Proposition,
  RemediationAction,
  SCHEMAS,
  Selector,
  SelectorBundle,
  Verdict,
  Verifier,
} from "../src/core/model.ts";
import { PROTOCOL_SCHEMAS } from "../src/resolver/protocol.ts";

const docOnlyAnchor = {
  doc: {
    file: "README.md",
    selectors: [{ kind: "text-quote", exact: "x", prefix: "", suffix: "" }],
  },
};

const baseAssertion = {
  id: "a",
  propositionId: "p",
  documentId: "d",
  owner: "x",
  ref: "r",
  anchor: docOnlyAnchor,
};

describe("canonical model is the single source of truth", () => {
  test("MODEL_VERSION is v3", () => {
    expect(MODEL_VERSION).toBe("v3");
  });

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

  test("SCHEMAS carries no BehaviorScope entry", () => {
    expect("BehaviorScope" in SCHEMAS).toBe(false);
  });

  // Enum option lists.

  test("Enforcement options", () => {
    expect(Enforcement.options).toEqual(["suggested", "enforced", "retired"]);
  });

  test("AnchorState options: one vocabulary, both sides", () => {
    expect(AnchorState.options).toEqual([
      "unchanged",
      "moved",
      "changed",
      "ambiguous",
      "orphaned",
    ]);
  });

  test("BehaviorState options", () => {
    expect(BehaviorState.options).toEqual(["supported", "refuted"]);
  });

  test("DocumentLifecycle options", () => {
    expect(DocumentLifecycle.options).toEqual([
      "active",
      "superseded",
      "archived",
    ]);
  });

  test("ChangedEvidenceKind options", () => {
    expect(ChangedEvidenceKind.options).toEqual([
      "text",
      "ast",
      "value",
      "verifier-source",
    ]);
  });

  // Selector union.

  test("Selector is a discriminated union of exactly 5 variants", () => {
    const js = z.toJSONSchema(Selector) as {
      anyOf?: unknown[];
      oneOf?: unknown[];
    };
    const variants = js.oneOf ?? js.anyOf;
    expect(Array.isArray(variants)).toBe(true);
    // text-quote, text-position, ast-node, value, coarse
    expect(variants?.length).toBe(5);
  });

  test("Selector accepts the coarse variant", () => {
    expect(
      Selector.safeParse({ kind: "coarse", pattern: "src/**" }).success,
    ).toBe(true);
  });

  test("the removed selector kinds are rejected", () => {
    expect(Selector.safeParse({ kind: "bogus" }).success).toBe(false);
    expect(Selector.safeParse({ kind: "path", path: "src/a.ts" }).success).toBe(
      false,
    );
    expect(Selector.safeParse({ kind: "glob", glob: "src/**" }).success).toBe(
      false,
    );
    expect(
      Selector.safeParse({ kind: "inline-id", id: "hibi:claim:abc" }).success,
    ).toBe(false);
  });

  // Bidirectional Anchor + SelectorBundle round-trip.

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

  test("Anchor defaults code to []", () => {
    const a = Anchor.parse(docOnlyAnchor);
    expect(a.code).toEqual([]);
  });

  test("the old flat anchor shape { file, selectors } does not validate", () => {
    const flat = {
      file: "a.ts",
      selectors: [{ kind: "text-quote", exact: "x" }],
    };
    expect(Anchor.safeParse(flat).success).toBe(false);
  });

  // Assertion.

  test("a valid Assertion round-trips with the bidirectional anchor", () => {
    const a = {
      ...baseAssertion,
      anchor: {
        ...docOnlyAnchor,
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

  test("Assertion defaults: enforcement enforced, verified false, verifiers [], attrs {}", () => {
    const a = Assertion.parse(baseAssertion);
    expect(a.enforcement).toBe("enforced");
    expect(a.verified).toBe(false);
    expect(a.verifiers).toEqual([]);
    expect(a.attrs).toEqual({});
    expect(a.ttl).toBeUndefined();
  });

  test("Assertion rejects an out-of-enum enforcement", () => {
    expect(
      Assertion.safeParse({ ...baseAssertion, enforcement: "mandatory" })
        .success,
    ).toBe(false);
  });

  test("Assertion accepts verified, verifiers, ttl and attrs", () => {
    const a = Assertion.parse({
      ...baseAssertion,
      verified: true,
      verifiers: [{ kind: "command", ref: "bun test" }],
      ttl: "2030-01-01T00:00:00Z",
      attrs: { note: "x" },
    });
    expect(a.verified).toBe(true);
    expect(a.verifiers[0]?.ref).toBe("bun test");
    expect(a.ttl).toBe("2030-01-01T00:00:00Z");
    expect(a.attrs).toEqual({ note: "x" });
  });

  test("Assertion rejects the removed v2 fields", () => {
    for (const extra of [
      { behavioral: true },
      { behaviorScope: { include: ["fixtures/**"] } },
      { evidenceBaseline: { "src/x.ts": "deadbeef" } },
      { suppressed: true },
    ]) {
      expect(Assertion.safeParse({ ...baseAssertion, ...extra }).success).toBe(
        false,
      );
    }
  });

  test("Verifier is {kind, ref} with an open, non-empty kind", () => {
    for (const kind of ["command", "metamorphic", "my-custom-runner"]) {
      expect(Verifier.safeParse({ kind, ref: "x" }).success).toBe(true);
    }
    expect(Verifier.safeParse({ kind: "", ref: "x" }).success).toBe(false);
    expect(
      Verifier.safeParse({ kind: "command", ref: "x", proves: "y" }).success,
    ).toBe(false);
  });

  test("an invalid selector kind inside an anchor bundle is rejected", () => {
    expect(
      Assertion.safeParse({
        ...baseAssertion,
        anchor: { doc: { file: "a.ts", selectors: [{ kind: "bogus" }] } },
      }).success,
    ).toBe(false);
  });

  test("an empty selector bundle on the doc side is rejected (min 1)", () => {
    expect(
      Assertion.safeParse({
        ...baseAssertion,
        anchor: { doc: { file: "a.ts", selectors: [] } },
      }).success,
    ).toBe(false);
  });

  // Document.

  test("Document applies defaults (lifecycle active, edges [])", () => {
    const d = Document.parse({ id: "d", path: "x.md" });
    expect(d.lifecycle).toBe("active");
    expect(d.edges).toEqual([]);
  });

  test("Document rejects pristine and frontmatterStatus", () => {
    expect(
      Document.safeParse({ id: "d", path: "x.md", pristine: false }).success,
    ).toBe(false);
    expect(
      Document.safeParse({ id: "d", path: "x.md", frontmatterStatus: "x" })
        .success,
    ).toBe(false);
  });

  test("Edge is only supersedes {type, target}", () => {
    expect(
      Edge.safeParse({ type: "supersedes", target: "doc_1" }).success,
    ).toBe(true);
    expect(
      Edge.safeParse({ type: "supersedes", target: "doc_1", derived: false })
        .success,
    ).toBe(false);
    expect(
      Edge.safeParse({ type: "superseded-by", source: "doc_1" }).success,
    ).toBe(false);
    expect(
      Edge.safeParse({ type: "amends", target: "doc_1", propositions: [] })
        .success,
    ).toBe(false);
  });

  // Proposition.

  test("Proposition is {id, textCache, fingerprint} and nothing else", () => {
    const p = Proposition.parse({
      id: "p",
      textCache: "the documented sentence",
      fingerprint: "f",
    });
    expect(p.textCache).toBe("the documented sentence");

    expect(
      Proposition.safeParse({ id: "p", text: "t", fingerprint: "f" }).success,
    ).toBe(false);
    expect(
      Proposition.safeParse({
        id: "p",
        textCache: "t",
        fingerprint: "f",
        authoredTrust: "verified",
      }).success,
    ).toBe(false);
  });

  // Remediation.

  test("RemediationAction is {id, title, rationale, command?}", () => {
    expect(
      RemediationAction.safeParse({
        id: "retire",
        title: "Retire the claim",
        rationale: "obsolete",
        command: "hibi retire a",
      }).success,
    ).toBe(true);
    expect(
      RemediationAction.safeParse({
        id: "retire",
        title: "Retire the claim",
        rationale: "obsolete",
        applicability: "manual",
      }).success,
    ).toBe(false);
  });

  // Verdict.

  test("Verdict (ephemeral, two-axis) validates", () => {
    const v: z.infer<typeof Verdict> = {
      assertionId: "a",
      propositionId: "p",
      documentId: "d",
      doc: "unchanged",
      code: "changed",
      behavior: "supported",
      expired: false,
      gates: false,
      remediation: {
        recommended: null,
        actions: [
          {
            id: "retire",
            title: "Retire the claim",
            rationale: "the claim is obsolete",
            command: "hibi retire a",
          },
        ],
      },
      evidence: {
        docRegion: { start: 0, end: 5 },
        codeRegions: [{ start: 10, end: 20 }],
        similarity: 0.3,
        changedEvidence: [
          { path: "a.ts", kind: "ast", detail: "restructured" },
        ],
      },
      notes: [],
      advisories: [],
    };
    expect(() => Verdict.parse(v)).not.toThrow();
  });

  test("Verdict behavior is optional and defaults apply", () => {
    const v = Verdict.parse({
      assertionId: "a",
      propositionId: "p",
      documentId: "d",
      doc: "unchanged",
      code: "unchanged",
      expired: false,
      gates: false,
      remediation: null,
      evidence: { codeRegions: [] },
    });
    expect(v.behavior).toBeUndefined();
    expect(v.evidence.changedEvidence).toEqual([]);
    expect(v.evidence.similarity).toBeUndefined();
    expect(v.remediation).toBeNull();
    expect(v.notes).toEqual([]);
    expect(v.advisories).toEqual([]);
  });

  test("Verdict rejects an out-of-enum anchor state and the removed fields", () => {
    const base = {
      assertionId: "a",
      propositionId: "p",
      documentId: "d",
      doc: "unchanged",
      code: "unchanged",
      expired: false,
      gates: false,
      remediation: null,
      evidence: { codeRegions: [] },
    };
    expect(Verdict.safeParse({ ...base, doc: "fresh" }).success).toBe(false);
    expect(Verdict.safeParse({ ...base, behavior: "at-risk" }).success).toBe(
      false,
    );
    expect(Verdict.safeParse({ ...base, suppressed: false }).success).toBe(
      false,
    );
    expect(
      Verdict.safeParse({
        ...base,
        evidence: { codeRegions: [], confidence: 1 },
      }).success,
    ).toBe(false);
    expect(
      Verdict.safeParse({
        ...base,
        evidence: {
          codeRegions: [],
          changedEvidence: [{ path: "a", kind: "bogus" }],
        },
      }).success,
    ).toBe(false);
  });
});

describe("fitness functions", () => {
  test("AnchorState is exactly {unchanged, moved, changed, ambiguous, orphaned}", () => {
    expect(new Set(AnchorState.options)).toEqual(
      new Set(["unchanged", "moved", "changed", "ambiguous", "orphaned"]),
    );
  });

  test("no AnchorState value carries a doc-/code-/behavior- prefix", () => {
    for (const state of AnchorState.options) {
      expect(state).not.toMatch(/^(doc|code|behavior)-/);
    }
  });

  test("the words drift/stale/ghost/fresh appear in no machine enum", () => {
    const banned = ["drift", "stale", "ghost", "fresh"];
    const allEnumValues = [
      ...AnchorState.options,
      ...BehaviorState.options,
      ...Enforcement.options,
      ...DocumentLifecycle.options,
    ];
    for (const value of allEnumValues) {
      for (const word of banned) {
        expect(value.includes(word)).toBe(false);
      }
    }
  });

  test("only refuted + changed/orphaned/ambiguous/expired gate; moved never gates", () => {
    const base = {
      doc: "unchanged",
      code: "unchanged",
      expired: false,
    } as const;

    for (const state of ["changed", "orphaned", "ambiguous"] as const) {
      expect(computeGates({ ...base, code: state }, "enforced")).toBe(true);
      expect(computeGates({ ...base, doc: state }, "enforced")).toBe(true);
    }
    expect(computeGates({ ...base, expired: true }, "enforced")).toBe(true);
    expect(computeGates({ ...base, behavior: "refuted" }, "enforced")).toBe(
      true,
    );
    expect(computeGates({ ...base, behavior: "supported" }, "enforced")).toBe(
      false,
    );

    expect(computeGates({ ...base, code: "moved" }, "enforced")).toBe(false);
    expect(
      isWarnVerdict({ ...base, code: "moved", gates: false }, "enforced"),
    ).toBe(true);
    expect(
      isWarnVerdict({ ...base, doc: "moved", gates: false }, "enforced"),
    ).toBe(true);
    expect(isWarnVerdict({ ...base, gates: false }, "enforced")).toBe(false);
  });

  test("only enforced claims gate; suggested/retired never gate or warn", () => {
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
      expect(
        isWarnVerdict(
          { doc: "moved", code: "unchanged", expired: false, gates: false },
          e,
        ),
      ).toBe(false);
    }
  });
});
