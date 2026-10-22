/**
 * The canonical data model (design/SPEC.md section 6): the single source of
 * truth for the store, the generated JSON Schema (`schemas/*.v4.json` via
 * `scripts/gen-schemas.ts`), and the TypeScript types. Store records are
 * validated against it at load.
 */
import * as z from "zod";

/** Schema version stamped into generated artifacts and the store config. */
export const MODEL_VERSION = "v4" as const;

/** The judge model id `init` writes into a new store's config. */
export const DEFAULT_JUDGE_MODEL = "jev-1.14.0";
/** The judge threshold `init` writes into a new store's config (SPEC 8.4). */
export const DEFAULT_JUDGE_THRESHOLD = 0.8;

// ── Ids and actors ───────────────────────────────────────────────────────────

/** `doc_` followed by 16 lowercase hex characters. */
export const DocumentId = z.string().regex(/^doc_[0-9a-f]{16}$/);
/** `clm_` followed by 16 lowercase hex characters. */
export const ClaimId = z.string().regex(/^clm_[0-9a-f]{16}$/);

/** OKF actor form: `human:<id>`, `agent:<name>`, or `process:<name>`. */
export const ACTOR_PATTERN = /^(human|agent|process):.+$/;
export const Actor = z.string().regex(ACTOR_PATTERN).meta({ id: "Actor" });
export type Actor = z.infer<typeof Actor>;

/**
 * An ISO 8601 duration such as `P90D` or `PT12H`: at least one component,
 * integer amounts, and a time component only after `T`.
 */
export const DURATION_PATTERN =
  /^P(?!$)(\d+Y)?(\d+M)?(\d+W)?(\d+D)?(T(?=\d)(\d+H)?(\d+M)?(\d+S)?)?$/;
export const Duration = z.string().regex(DURATION_PATTERN);

/** An ISO 8601 instant with an offset, e.g. `2026-10-22T09:12:00Z`. */
const Instant = z.iso.datetime({ offset: true });

// ── Enumerations ─────────────────────────────────────────────────────────────

/**
 * Enforcement: whether the claim may gate. `enforced` is the record default;
 * `suggested` never gates (`record --suggest`); `retired` is withdrawn.
 */
export const Enforcement = z.enum(["enforced", "suggested", "retired"]);
export type Enforcement = z.infer<typeof Enforcement>;

/** The three recordable claim kinds (SPEC 4). */
export const ClaimKind = z.enum(["code", "reference", "attested"]);
export type ClaimKind = z.infer<typeof ClaimKind>;

/** Document lifecycle. */
export const DocumentLifecycle = z.enum(["active", "superseded", "archived"]);
export type DocumentLifecycle = z.infer<typeof DocumentLifecycle>;

/** Computed anchor resolution of one span (SPEC 7.1). */
export const AnchorState = z
  .enum([
    "unchanged", // found, same text, same place
    "moved", // found, same text, new place
    "changed", // found, text or structure differs
    "ambiguous", // matches in several places equally well
    "orphaned", // not found
  ])
  .meta({ id: "AnchorState" });
export type AnchorState = z.infer<typeof AnchorState>;

/** The evidence axis: the anchor states plus `unavailable` (a cross-repo alias not checked out). */
export const EvidenceState = z
  .enum([...AnchorState.options, "unavailable"])
  .meta({ id: "EvidenceState" });
export type EvidenceState = z.infer<typeof EvidenceState>;

/** The judge axis of a verdict (SPEC 7.2). */
export const JudgeState = z.enum(["supported", "refuted", "review", "pending"]);
export type JudgeState = z.infer<typeof JudgeState>;

// ── Anchor selectors (unchanged from v3) ─────────────────────────────────────

/** `text-quote`: exact + prefix + suffix (W3C TextQuoteSelector). */
export const TextQuoteSelector = z.strictObject({
  kind: z.literal("text-quote"),
  exact: z.string(),
  prefix: z.string().default(""),
  suffix: z.string().default(""),
});

/** `text-position`: char offsets at record time. A locate bias and a move tiebreaker. */
export const TextPositionSelector = z.strictObject({
  kind: z.literal("text-position"),
  start: z.number().int().nonnegative(),
  end: z.number().int().nonnegative(),
});

/**
 * `ast-node` (code only): the smallest enclosing named tree-sitter node with a
 * two-tier fingerprint. Used for reason labels only: structural equal and
 * semantic different means a rename, both different means a restructure.
 */
export const AstNodeSelector = z.strictObject({
  kind: z.literal("ast-node"),
  language: z.string(),
  nodeType: z.string(),
  structuralHash: z.string(),
  semanticHash: z.string(),
});

/** `value` (code only): the first literal inside the quoted span, so `5` to `50` trips. */
export const ValueSelector = z.strictObject({
  kind: z.literal("value"),
  language: z.string(),
  nodeKind: z.string(),
  value: z.string(),
});

/** `coarse`: a file or glob pattern. Navigation only, never graded as drift. */
export const CoarseSelector = z.strictObject({
  kind: z.literal("coarse"),
  pattern: z.string(),
});

/** The coarse selector kinds, never reported as drift. */
export const COARSE_SELECTOR_KINDS = ["coarse"] as const;

export const Selector = z
  .discriminatedUnion("kind", [
    TextQuoteSelector,
    TextPositionSelector,
    AstNodeSelector,
    ValueSelector,
    CoarseSelector,
  ])
  .meta({ id: "Selector" });
export type Selector = z.infer<typeof Selector>;
export type SelectorKind = Selector["kind"];

/** The selectors that locate one span in one file. */
export const SelectorBundle = z
  .strictObject({
    file: z.string(),
    selectors: z.array(Selector).min(1),
  })
  .meta({ id: "SelectorBundle" });
export type SelectorBundle = z.infer<typeof SelectorBundle>;

// ── Claim facets ─────────────────────────────────────────────────────────────

/** How an attestation stands behind a claim (SPEC 5). */
export const AttestationBasis = z.enum(["vouch", "evidence", "ack"]);
export type AttestationBasis = z.infer<typeof AttestationBasis>;

/**
 * What an `ack` attestation acknowledges (SPEC 9.4, 8.4): a contradiction
 * pair, pinned to both sentences so the ack lapses when either changes; or
 * the cached `supports` verdict that was in `review`.
 */
export const AttestationAck = z.union([
  z.strictObject({
    /** The two claim ids, sorted, joined with `:`. */
    pairId: z.string().min(1),
    /** sha256 of each normalized sentence, in `pairId` order. */
    sentenceHashes: z
      .tuple([z.string().min(1), z.string().min(1)])
      // Zod emits no length bounds for a tuple; state them for the JSON Schema.
      .meta({ minItems: 2, maxItems: 2 }),
  }),
  z.strictObject({
    /** The judge cache key of the acknowledged `supports` verdict. */
    judgeKey: z.string().min(1),
  }),
]);
export type AttestationAck = z.infer<typeof AttestationAck>;

/**
 * One append-only record that an actor stood behind the claim at a time.
 * `ack` is set exactly when `basis` is `ack`.
 */
export const Attestation = z
  .strictObject({
    by: Actor,
    at: Instant,
    basis: AttestationBasis,
    note: z.string().optional(),
    evidenceHash: z.string().optional(),
    ack: AttestationAck.optional(),
  })
  .superRefine((a, ctx) => {
    if (a.basis === "ack" && a.ack === undefined) {
      ctx.addIssue({
        code: "custom",
        path: ["ack"],
        message: "an ack attestation names what it acknowledges in `ack`",
      });
    }
    if (a.basis !== "ack" && a.ack !== undefined) {
      ctx.addIssue({
        code: "custom",
        path: ["ack"],
        message: `\`ack\` is only valid on an ack attestation (basis is ${a.basis})`,
      });
    }
  })
  .meta({ id: "Attestation" });
export type Attestation = z.infer<typeof Attestation>;

/** An opaque pointer to where a claim's truth lives (SPEC 4.3). Never dereferenced. */
export const Source = z
  .strictObject({
    id: z.string().optional(),
    resource: z.string().min(1),
    title: z.string().optional(),
  })
  .meta({ id: "Source" });
export type Source = z.infer<typeof Source>;

/** One Lobster-compatible step: a hint, never run by hibi (SPEC 4.2). */
export const ProcedureStep = z.strictObject({
  id: z.string().min(1),
  command: z.string().min(1),
  stdin: z.string().optional(),
});
export type ProcedureStep = z.infer<typeof ProcedureStep>;

/** How an agent re-checks an `attested` claim. `instructions` is the contract. */
export const Procedure = z
  .strictObject({
    instructions: z.string().min(1),
    steps: z.array(ProcedureStep).optional(),
  })
  .meta({ id: "Procedure" });
export type Procedure = z.infer<typeof Procedure>;

// ── Entities ─────────────────────────────────────────────────────────────────

export const Document = z
  .strictObject({
    id: DocumentId,
    path: z.string(),
    lifecycle: DocumentLifecycle.default("active"),
    supersededBy: DocumentId.optional(),
    createdAt: Instant,
  })
  .meta({ id: "Document" });
export type Document = z.infer<typeof Document>;

/**
 * Claim: one sentence in one document with its kind, its evidence spans, and
 * its attestations. `evidence` holds code spans for `code` (one or more),
 * exactly one document span for `reference`, and nothing for `attested`.
 */
export const Claim = z
  .strictObject({
    id: ClaimId,
    documentId: DocumentId,
    kind: ClaimKind,
    sentence: z.string().min(1),
    doc: SelectorBundle,
    evidence: z.array(SelectorBundle).default([]),
    sources: z.array(Source).default([]),
    procedure: Procedure.nullable().default(null),
    owner: Actor.optional(),
    ttl: Duration.optional(),
    enforcement: Enforcement.default("enforced"),
    attestations: z.array(Attestation).default([]),
    createdAt: Instant,
    /** The git HEAD the claim was last recorded or reanchored against. */
    ref: z.string(),
  })
  .superRefine((c, ctx) => {
    const n = c.evidence.length;
    if (c.kind === "code" && n < 1) {
      ctx.addIssue({
        code: "custom",
        path: ["evidence"],
        message: "a code claim needs at least one evidence span",
      });
    }
    if (c.kind === "reference" && n !== 1) {
      ctx.addIssue({
        code: "custom",
        path: ["evidence"],
        message: `a reference claim needs exactly one evidence span (got ${n})`,
      });
    }
    if (c.kind === "attested") {
      if (n !== 0) {
        ctx.addIssue({
          code: "custom",
          path: ["evidence"],
          message: "an attested claim has no evidence spans",
        });
      }
      if (c.ttl === undefined) {
        ctx.addIssue({
          code: "custom",
          path: ["ttl"],
          message: "an attested claim requires a ttl",
        });
      }
    }
  })
  .meta({ id: "Claim" });
export type Claim = z.infer<typeof Claim>;

// ── Store config ─────────────────────────────────────────────────────────────

/** A pinned, versioned Jev model id such as `jev-1.14.0`; aliases are rejected. */
export const JUDGE_MODEL_PATTERN = /^jev-\d+\.\d+\.\d+$/;

export const JudgeConfig = z.strictObject({
  /** A pinned, versioned model id; never an alias. */
  model: z.string().regex(JUDGE_MODEL_PATTERN, {
    error: (iss) =>
      `judge.model ${JSON.stringify(iss.input)} is not a pinned versioned id; hibi needs one such as jev-1.14.0, not an alias like jev-latest`,
  }),
  /** Choice-confidence threshold for gate and clean outcomes (SPEC 8.4). */
  threshold: z
    .number({ error: "judge.threshold must be a number in [0, 1]" })
    .min(0, { error: "judge.threshold must be a number in [0, 1]" })
    .max(1, {
      error: "judge.threshold must be a number in [0, 1]",
    }),
});
export type JudgeConfig = z.infer<typeof JudgeConfig>;

export const Config = z
  .strictObject({
    version: z.literal(MODEL_VERSION),
    /** Cross-repository aliases: alias to a path relative to the repository root. */
    repos: z.record(z.string(), z.string()).default({}),
    judge: JudgeConfig,
    /** Globs of documents hibi skips. */
    ignore: z.array(z.string()).default([]),
  })
  .meta({ id: "Config" });
export type Config = z.infer<typeof Config>;

// ── Judge cache ──────────────────────────────────────────────────────────────

export const Judgment = z.enum(["triage", "supports", "conflict"]);
export type Judgment = z.infer<typeof Judgment>;

/** What a cached verdict is about. Every field is optional; at least one is set. */
export const JudgeSubject = z.strictObject({
  claimId: z.string().optional(),
  /** `clm_a:clm_b` for a contradiction pair. */
  pairId: z.string().optional(),
  docId: z.string().optional(),
});
export type JudgeSubject = z.infer<typeof JudgeSubject>;

/** The evidence a `supports` verdict was asked about. Raw text is never stored. */
export const CachedEvidence = z.strictObject({
  hash: z.string(),
  source: z.string(),
  summary: z.string(),
  by: Actor,
  at: Instant,
});
export type CachedEvidence = z.infer<typeof CachedEvidence>;

/** One cached judge verdict at `.claims/judge/<key>.json` (SPEC 6.4). */
export const JudgeCacheEntry = z
  .strictObject({
    /** sha256 hex of the model id and the canonical request JSON. */
    key: z.string().regex(/^[0-9a-f]{64}$/),
    model: z.string(),
    judgment: Judgment,
    subject: JudgeSubject,
    /** The exact state and questions sent. */
    request: z.unknown(),
    /** The exact probabilities returned. */
    answers: z.unknown(),
    evidence: CachedEvidence.optional(),
    at: Instant,
  })
  .meta({ id: "JudgeCacheEntry" });
export type JudgeCacheEntry = z.infer<typeof JudgeCacheEntry>;

// ── Verdict (computed at check time, never stored) ───────────────────────────

/** A char span in a file. */
export const Region = z
  .strictObject({
    start: z.number().int().nonnegative(),
    end: z.number().int().nonnegative(),
  })
  .meta({ id: "Region" });
export type Region = z.infer<typeof Region>;

export const SpanChangeKind = z.enum(["text", "ast", "value"]);
export type SpanChangeKind = z.infer<typeof SpanChangeKind>;

/** What changed in a span and where. */
export const SpanChange = z.strictObject({
  path: z.string(),
  kind: SpanChangeKind,
  detail: z.string().optional(),
});
export type SpanChange = z.infer<typeof SpanChange>;

/** A contradiction finding against another claim (SPEC 7.2). */
export const ConflictFinding = z.strictObject({
  claimId: z.string(),
  probability: z.number(),
  reason: z.string(),
});
export type ConflictFinding = z.infer<typeof ConflictFinding>;

/** Where each span was located and what changed in it. */
export const VerdictDetail = z.strictObject({
  docRegion: Region.optional(),
  evidenceRegions: z.array(Region).default([]),
  /** Normalized text similarity of the span that decided the verdict to its stored quote. */
  similarity: z.number().optional(),
  changes: z.array(SpanChange).default([]),
});
export type VerdictDetail = z.infer<typeof VerdictDetail>;

// ── Remediation menu ─────────────────────────────────────────────────────────

export const RemediationAction = z.strictObject({
  /** Stable token, e.g. `reanchor`, `retire`. */
  id: z.string(),
  title: z.string(),
  rationale: z.string(),
  /** Ready-to-run command with the claim id filled in, when one exists. */
  command: z.string().optional(),
});
export type RemediationAction = z.infer<typeof RemediationAction>;

export const Remediation = z.strictObject({
  recommended: z.string().nullable(),
  actions: z.array(RemediationAction).default([]),
});
export type Remediation = z.infer<typeof Remediation>;

/** Verdict: the per-claim result of `check`. Decision fields first, detail last. */
export const Verdict = z
  .strictObject({
    claimId: z.string(),
    documentId: z.string(),
    kind: ClaimKind,
    doc: AnchorState,
    /** Worst state over the evidence spans; absent when the claim has none. */
    evidence: EvidenceState.optional(),
    /** Absent when no judgment applies. */
    judge: JudgeState.optional(),
    expired: z.boolean(),
    conflicts: z.array(ConflictFinding).default([]),
    gates: z.boolean(),
    remediation: Remediation.nullable().default(null),
    detail: VerdictDetail,
    /** Reason labels, e.g. "identifiers or literals renamed". */
    notes: z.array(z.string()).default([]),
  })
  .meta({ id: "Verdict" });
export type Verdict = z.infer<typeof Verdict>;

export const SCHEMAS = {
  Selector,
  SelectorBundle,
  Attestation,
  Source,
  Procedure,
  Document,
  Claim,
  Config,
  JudgeCacheEntry,
  RemediationAction,
  Remediation,
  Verdict,
} as const;
