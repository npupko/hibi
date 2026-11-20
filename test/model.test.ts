import { describe, expect, test } from "bun:test";
import * as z from "zod";
import { gateVerdict } from "../src/core/gating.ts";
import { documentIdForPath, newClaimId } from "../src/core/ids.ts";
import {
  AnchorState,
  Attestation,
  Claim,
  Config,
  Document,
  DocumentLifecycle,
  Enforcement,
  EvidenceState,
  JudgeCacheEntry,
  JudgeState,
  MODEL_VERSION,
  Procedure,
  SCHEMAS,
  Selector,
  SelectorBundle,
  Source,
  Verdict,
} from "../src/core/model.ts";

const AT = "2026-10-22T09:12:00Z";

const docBundle = {
  file: "docs/pricing.md",
  selectors: [
    {
      kind: "text-quote",
      exact: "Pro costs $29/month.",
      prefix: "",
      suffix: "",
    },
  ],
};
const codeBundle = {
  file: "src/billing.ts",
  selectors: [
    { kind: "text-quote", exact: "PRO = 29", prefix: "", suffix: "" },
  ],
};

const base = {
  id: "clm_0123456789abcdef",
  documentId: "doc_0123456789abcdef",
  kind: "code",
  sentence: "Pro costs $29/month.",
  doc: docBundle,
  evidence: [codeBundle],
  createdAt: AT,
  ref: "HEAD",
};

function issues(schema: z.ZodType, value: unknown): string[] {
  const r = schema.safeParse(value);
  return r.success ? [] : r.error.issues.map((i) => i.message);
}

describe("canonical model", () => {
  test("MODEL_VERSION is v4", () => {
    expect(MODEL_VERSION).toBe("v4");
  });

  test("every SCHEMAS entry exports to JSON Schema without throwing", () => {
    for (const [name, schema] of Object.entries(SCHEMAS)) {
      expect(
        () => z.toJSONSchema(schema, { target: "draft-2020-12" }),
        name,
      ).not.toThrow();
    }
  });

  test("SCHEMAS names exactly the v4 shapes", () => {
    expect(Object.keys(SCHEMAS).sort()).toEqual(
      [
        "Attestation",
        "Claim",
        "Config",
        "Document",
        "JudgeCacheEntry",
        "Procedure",
        "Remediation",
        "RemediationAction",
        "Selector",
        "SelectorBundle",
        "Source",
        "Verdict",
      ].sort(),
    );
  });

  test("Selector is the four-variant union", () => {
    const kinds = Selector.options.map(
      (o) => (o.shape.kind as z.ZodLiteral<string>).value,
    );
    expect(new Set(kinds)).toEqual(
      new Set(["text-quote", "text-position", "ast-node", "value"]),
    );
    expect(SelectorBundle.safeParse({ file: "a", selectors: [] }).success).toBe(
      false,
    );
  });

  test("enumerations", () => {
    expect(Enforcement.options).toEqual(["enforced", "suggested", "retired"]);
    expect(DocumentLifecycle.options).toEqual([
      "active",
      "superseded",
      "archived",
    ]);
    expect(EvidenceState.options).toEqual([
      ...AnchorState.options,
      "unavailable",
    ]);
    expect(JudgeState.options).toEqual([
      "supported",
      "refuted",
      "review",
      "pending",
    ]);
  });
});

describe("ids", () => {
  test("new claim ids and document ids are prefix plus 16 hex", () => {
    expect(newClaimId()).toMatch(/^clm_[0-9a-f]{16}$/);
    expect(newClaimId()).not.toBe(newClaimId());
    expect(documentIdForPath("docs/a.md")).toMatch(/^doc_[0-9a-f]{16}$/);
    expect(documentIdForPath("docs/a.md")).toBe(documentIdForPath("docs/a.md"));
  });

  test("Claim and Document reject ids of the wrong form", () => {
    expect(
      Claim.safeParse({ ...base, id: "asrt_0123456789abcdef" }).success,
    ).toBe(false);
    expect(Claim.safeParse({ ...base, id: "clm_0123" }).success).toBe(false);
    expect(Claim.safeParse({ ...base, documentId: "doc_1" }).success).toBe(
      false,
    );
    expect(
      Document.safeParse({ id: "doc_XYZ", path: "a.md", createdAt: AT })
        .success,
    ).toBe(false);
  });
});

describe("Claim validation", () => {
  test("a code claim round-trips with defaults applied", () => {
    const c = Claim.parse(base);
    expect(c.sources).toEqual([]);
    expect(c.procedure).toBeNull();
    expect(c.enforcement).toBe("enforced");
    expect(c.attestations).toEqual([]);
    expect(c.owner).toBeUndefined();
    expect(c.ttl).toBeUndefined();
  });

  test("code needs at least one evidence span", () => {
    expect(issues(Claim, { ...base, evidence: [] })).toEqual([
      "a code claim needs at least one evidence span",
    ]);
    expect(
      Claim.safeParse({ ...base, evidence: [codeBundle, codeBundle] }).success,
    ).toBe(true);
  });

  test("reference needs exactly one evidence span", () => {
    const ref = { ...base, kind: "reference" };
    expect(Claim.safeParse(ref).success).toBe(true);
    expect(issues(Claim, { ...ref, evidence: [] })).toEqual([
      "a reference claim needs exactly one evidence span (got 0)",
    ]);
    expect(
      issues(Claim, { ...ref, evidence: [codeBundle, codeBundle] }),
    ).toEqual(["a reference claim needs exactly one evidence span (got 2)"]);
  });

  test("attested has no evidence and requires a ttl", () => {
    const att = { ...base, kind: "attested", evidence: [], ttl: "P90D" };
    expect(Claim.safeParse(att).success).toBe(true);
    expect(issues(Claim, { ...att, ttl: undefined })).toEqual([
      "an attested claim requires a ttl",
    ]);
    expect(issues(Claim, { ...att, evidence: [codeBundle] })).toEqual([
      "an attested claim has no evidence spans",
    ]);
  });

  test("ttl is an ISO 8601 duration, optional outside attested", () => {
    for (const ok of ["P90D", "P1Y", "P2W", "PT12H", "P1DT2H30M", "PT45S"]) {
      expect(Claim.safeParse({ ...base, ttl: ok }).success, ok).toBe(true);
    }
    for (const bad of ["P", "PT", "90D", "P1.5D", "2027-01-01T00:00:00Z", ""]) {
      expect(Claim.safeParse({ ...base, ttl: bad }).success, bad).toBe(false);
    }
  });

  test("owner and attestation actors must be human:, agent:, or process:", () => {
    for (const ok of ["human:alice@example.com", "agent:opus", "process:ci"]) {
      expect(Claim.safeParse({ ...base, owner: ok }).success, ok).toBe(true);
    }
    for (const bad of ["alice", "human:", "robot:x", "Human:alice"]) {
      expect(Claim.safeParse({ ...base, owner: bad }).success, bad).toBe(false);
      expect(
        Attestation.safeParse({ by: bad, at: AT, basis: "vouch" }).success,
        bad,
      ).toBe(false);
    }
  });

  test("Attestation shape: by, at, basis, optional note and evidenceHash", () => {
    expect(
      Attestation.parse({
        by: "human:alice@example.com",
        at: AT,
        basis: "evidence",
        note: "checked Stripe",
        evidenceHash: "abc",
      }),
    ).toBeDefined();
    expect(
      Attestation.safeParse({ by: "agent:x", at: AT, basis: "trust-me" })
        .success,
    ).toBe(false);
    expect(
      Attestation.safeParse({ by: "agent:x", at: "yesterday", basis: "ack" })
        .success,
    ).toBe(false);
  });

  test("Source requires resource; Procedure requires instructions", () => {
    expect(Source.safeParse({ id: "stripe" }).success).toBe(false);
    expect(Source.parse({ resource: "https://x" })).toEqual({
      resource: "https://x",
    });
    expect(Procedure.safeParse({ steps: [] }).success).toBe(false);
    expect(
      Procedure.parse({
        instructions: "Read the price.",
        steps: [{ id: "price", command: "stripe prices list", stdin: "" }],
      }).steps?.[0]?.id,
    ).toBe("price");
    expect(
      Procedure.safeParse({ instructions: "x", steps: [{ id: "a" }] }).success,
    ).toBe(false);
  });

  test("the removed v3 fields are rejected", () => {
    for (const field of [
      "propositionId",
      "anchor",
      "verified",
      "verifiers",
      "attrs",
      "authoredTrust",
    ]) {
      expect(Claim.safeParse({ ...base, [field]: [] }).success, field).toBe(
        false,
      );
    }
  });
});

describe("Document", () => {
  test("lifecycle defaults to active; supersededBy is an optional doc id", () => {
    const d = Document.parse({
      id: "doc_0123456789abcdef",
      path: "a.md",
      createdAt: AT,
    });
    expect(d.lifecycle).toBe("active");
    expect(d.supersededBy).toBeUndefined();
    expect(
      Document.safeParse({ ...d, supersededBy: "doc_fedcba9876543210" })
        .success,
    ).toBe(true);
    expect(Document.safeParse({ ...d, supersededBy: "v2.md" }).success).toBe(
      false,
    );
    expect(Document.safeParse({ ...d, edges: [] }).success).toBe(false);
    expect(
      Document.safeParse({ id: d.id, path: "a.md" }).success,
      "createdAt is required",
    ).toBe(false);
  });
});

describe("Config", () => {
  const cfg = {
    version: "v4",
    repos: { api: "../api" },
    judge: { model: "jev-1.14.0", threshold: 0.8 },
    ignore: ["docs/generated/**"],
  };

  test("the SPEC example parses", () => {
    expect(Config.parse(cfg)).toEqual(cfg as z.infer<typeof Config>);
  });

  test("only v4 is accepted", () => {
    expect(Config.safeParse({ ...cfg, version: "v3" }).success).toBe(false);
  });

  test("judge.model must be a pinned versioned jev id", () => {
    for (const bad of ["jev-latest", "jev-preview", "jev-1.14", "gpt-4"]) {
      const msgs = issues(Config, {
        ...cfg,
        judge: { ...cfg.judge, model: bad },
      });
      expect(msgs.join(" "), bad).toContain("pinned versioned id");
    }
  });

  test("judge.threshold is a number in [0, 1]", () => {
    for (const t of [0, 0.5, 1]) {
      expect(
        Config.safeParse({ ...cfg, judge: { ...cfg.judge, threshold: t } })
          .success,
      ).toBe(true);
    }
    for (const t of [-0.1, 1.1, "0.8"]) {
      expect(
        issues(Config, { ...cfg, judge: { ...cfg.judge, threshold: t } }),
      ).toEqual(["judge.threshold must be a number in [0, 1]"]);
    }
  });

  test("the removed v3 keys are rejected", () => {
    expect(Config.safeParse({ ...cfg, nonce: "deadbeef" }).success).toBe(false);
    expect(Config.safeParse({ ...cfg, pristine: [] }).success).toBe(false);
  });
});

describe("JudgeCacheEntry", () => {
  test("the SPEC 6.4 shape parses; raw evidence text has no field", () => {
    const entry = {
      key: "a".repeat(64),
      model: "jev-1.14.0",
      judgment: "supports",
      subject: { claimId: "clm_0123456789abcdef" },
      request: { state: {}, questions: [] },
      answers: { relation: { supports: 0.9 } },
      evidence: {
        hash: "h",
        source: "stripe",
        summary: "$29",
        by: "agent:opus",
        at: AT,
      },
      at: AT,
    };
    expect(JudgeCacheEntry.parse(entry)).toEqual(entry as JudgeCacheEntry);
    expect(
      JudgeCacheEntry.safeParse({
        ...entry,
        evidence: { ...entry.evidence, text: "raw" },
      }).success,
    ).toBe(false);
    expect(JudgeCacheEntry.safeParse({ ...entry, key: "short" }).success).toBe(
      false,
    );
    expect(
      JudgeCacheEntry.safeParse({ ...entry, judgment: "verify" }).success,
    ).toBe(false);
  });
});

describe("Verdict", () => {
  test("evidence and judge are optional axes; conflicts default to []", () => {
    const v = Verdict.parse({
      claimId: "clm_0123456789abcdef",
      documentId: "doc_0123456789abcdef",
      kind: "attested",
      doc: "unchanged",
      expired: false,
      gates: false,
      warns: false,
      detail: {},
    });
    expect(v.evidence).toBeUndefined();
    expect(v.judge).toBeUndefined();
    expect(v.conflicts).toEqual([]);
    expect(v.reasons).toEqual([]);
    expect(
      Verdict.safeParse({ ...v, notes: [] }).success,
      "notes is gone",
    ).toBe(false);
  });
});

describe("gating fitness functions", () => {
  const clean = {
    doc: "unchanged",
    evidence: "unchanged",
    expired: false,
  } as const;
  const gates = (
    v: Parameters<typeof gateVerdict>[0],
    e: Parameters<typeof gateVerdict>[1] = "enforced",
    mode: "store" | "doc" = "store",
  ) => gateVerdict(v, e, { mode }).gates;
  const warns = (
    v: Parameters<typeof gateVerdict>[0],
    e: Parameters<typeof gateVerdict>[1] = "enforced",
  ) => gateVerdict(v, e).warns;

  test("changed/orphaned/ambiguous on either side or refuted gate; moved and unavailable never do", () => {
    for (const state of ["changed", "orphaned", "ambiguous"] as const) {
      expect(gates({ ...clean, evidence: state })).toBe(true);
      expect(gates({ ...clean, doc: state })).toBe(true);
    }
    expect(gates({ ...clean, judge: "refuted" })).toBe(true);
    expect(gates({ ...clean, judge: "supported" })).toBe(false);
    expect(gates({ ...clean, evidence: "moved" })).toBe(false);
    expect(gates({ ...clean, evidence: "unavailable" })).toBe(false);
    expect(
      gates({ doc: "unchanged", expired: false }),
      "no evidence axis",
    ).toBe(false);
    expect(warns({ ...clean, evidence: "moved" })).toBe(true);
  });

  test("expired warns in a whole-store check and gates under check --doc", () => {
    expect(gates({ ...clean, expired: true })).toBe(false);
    expect(warns({ ...clean, expired: true })).toBe(true);
    expect(gates({ ...clean, expired: true }, "enforced", "doc")).toBe(true);
  });

  test("only enforced claims gate; suggested and retired never gate or warn", () => {
    const gating = {
      doc: "changed",
      evidence: "orphaned",
      expired: true,
      judge: "refuted",
    } as const;
    expect(gates(gating)).toBe(true);
    for (const e of ["suggested", "retired"] as const) {
      expect(gates(gating, e)).toBe(false);
      expect(warns({ ...clean, doc: "moved" }, e)).toBe(false);
    }
  });
});
