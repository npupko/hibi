/**
 * The canonical data model, the single source of truth for the store, the
 * generated JSON Schema (`schemas/*.v3.json` via `scripts/gen-schemas.ts`),
 * and the TypeScript types. Claim-store records are validated against it at load.
 *
 * Two computed axes:
 *   - `AnchorState`: one vocabulary applied per side (`doc` / `code`): can the
 *     span be found, and is it unchanged?
 *   - `BehaviorState`: present only when a verifier ran (`supported` / `refuted`).
 *   - `expired` is an orthogonal time flag.
 */
import * as z from "zod";

/** Schema version stamped into generated artifacts and the store config. */
export const MODEL_VERSION = "v3" as const;

/**
 * Enforcement: whether the claim may gate. `enforced` is the record default;
 * `suggested` is advisory (`record --suggest`); `retired` is withdrawn.
 */
export const Enforcement = z.enum(["suggested", "enforced", "retired"]);
export type Enforcement = z.infer<typeof Enforcement>;

/** Computed anchor resolution, applied to each side of the anchor. */
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

/**
 * Computed behavioral belief. Absent unless a verifier ran under
 * `check --run-verifiers`. Only `refuted` may gate.
 */
export const BehaviorState = z.enum(["supported", "refuted"]);
export type BehaviorState = z.infer<typeof BehaviorState>;

/** Document lifecycle. */
export const DocumentLifecycle = z.enum(["active", "superseded", "archived"]);
export type DocumentLifecycle = z.infer<typeof DocumentLifecycle>;

// ── Anchor selectors ─────────────────────────────────────────────────────────

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
 * `ast-node` (code side): the smallest enclosing named tree-sitter node with a
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

/** `value` (code side): the first literal inside the quoted span, so `5` to `50` trips. */
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

/** The selectors for one side of an anchor against one file. */
export const SelectorBundle = z
  .strictObject({
    file: z.string(),
    selectors: z.array(Selector).min(1),
  })
  .meta({ id: "SelectorBundle" });
export type SelectorBundle = z.infer<typeof SelectorBundle>;

/**
 * Anchor: a doc-side bundle (the documented sentence) plus zero or more
 * code-side bundles. `code` may be empty only on a `suggested` claim.
 */
export const Anchor = z.strictObject({
  doc: SelectorBundle,
  code: z.array(SelectorBundle).default([]),
});
export type Anchor = z.infer<typeof Anchor>;

// ── Verifiers ────────────────────────────────────────────────────────────────

/**
 * Verifier: a command a runner executes under `check --run-verifiers`.
 * `kind` is a dispatch key matched against runner-declared kinds; the built-in
 * runner handles `command`.
 */
export const Verifier = z
  .strictObject({
    kind: z.string().min(1),
    ref: z.string(),
  })
  .meta({ id: "Verifier" });
export type Verifier = z.infer<typeof Verifier>;

// ── Document edges ───────────────────────────────────────────────────────────

/** `supersedes`: authored on the new document, targets the old document id. */
export const SupersedesEdge = z.strictObject({
  type: z.literal("supersedes"),
  target: z.string(),
});

export const Edge = z.discriminatedUnion("type", [SupersedesEdge]);
export type Edge = z.infer<typeof Edge>;

// ── Entities ─────────────────────────────────────────────────────────────────

export const Document = z.strictObject({
  id: z.string(),
  path: z.string(),
  lifecycle: DocumentLifecycle.default("active"),
  edges: z.array(Edge).default([]),
});
export type Document = z.infer<typeof Document>;

/**
 * Proposition: the sentence, deduplicated by content fingerprint. `textCache`
 * is a non-authoritative copy; the live doc span is the truth.
 */
export const Proposition = z.strictObject({
  id: z.string(),
  textCache: z.string(),
  fingerprint: z.string(),
});
export type Proposition = z.infer<typeof Proposition>;

/** Assertion: one claim. Carries the anchor and the authored facets. */
export const Assertion = z.strictObject({
  id: z.string(),
  propositionId: z.string(),
  documentId: z.string(),
  owner: z.string(),
  /** The commit the claim was last recorded or reanchored against. */
  ref: z.string(),
  anchor: Anchor,
  enforcement: Enforcement.default("enforced"),
  /** The author confirmed the code backs the sentence (`record --verified`). */
  verified: z.boolean().default(false),
  verifiers: z.array(Verifier).default([]),
  /** ISO-8601 instant; past it the computed `expired` flag is set. */
  ttl: z.string().optional(),
  /** Open key/value bag for resolver-specific metadata the core does not interpret. */
  attrs: z.record(z.string(), z.unknown()).default({}),
});
export type Assertion = z.infer<typeof Assertion>;

// ── Verdict (ephemeral, never persisted) ─────────────────────────────────────

export const Region = z
  .strictObject({
    start: z.number().int().nonnegative(),
    end: z.number().int().nonnegative(),
  })
  .meta({ id: "Region" });
export type Region = z.infer<typeof Region>;

export const ChangedEvidenceKind = z.enum([
  "text",
  "ast",
  "value",
  "verifier-source",
]);
export type ChangedEvidenceKind = z.infer<typeof ChangedEvidenceKind>;

/** What changed and where. */
export const ChangedEvidence = z.strictObject({
  path: z.string(),
  kind: ChangedEvidenceKind,
  detail: z.string().optional(),
});
export type ChangedEvidence = z.infer<typeof ChangedEvidence>;

/** Advisory note from an advisory resolver. Advises, never gates. */
export const Advisory = z
  .strictObject({
    resolver: z.string(),
    message: z.string(),
    /** Required from a `modelBacked` resolver; the registry drops advisories without it. */
    provenance: z
      .strictObject({
        model: z.string(),
        promptHash: z.string(),
        contextHash: z.string(),
        params: z.record(z.string(), z.unknown()).optional(),
      })
      .optional(),
  })
  .meta({ id: "Advisory" });
export type Advisory = z.infer<typeof Advisory>;

export const VerdictEvidence = z.strictObject({
  docRegion: Region.optional(),
  codeRegions: z.array(Region).default([]),
  /** Normalized text similarity of the located primary span to its stored quote. */
  similarity: z.number().optional(),
  changedEvidence: z.array(ChangedEvidence).default([]),
});
export type VerdictEvidence = z.infer<typeof VerdictEvidence>;

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

/** Verdict: the per-claim result. Decision fields first, evidence last. */
export const Verdict = z.strictObject({
  assertionId: z.string(),
  propositionId: z.string(),
  documentId: z.string(),
  doc: AnchorState,
  code: AnchorState,
  behavior: BehaviorState.optional(),
  expired: z.boolean(),
  /** True iff enforced and (a side is changed/orphaned/ambiguous, or expired, or refuted). */
  gates: z.boolean(),
  remediation: Remediation.nullable().default(null),
  evidence: VerdictEvidence,
  /** Reason labels, e.g. "identifiers or literals renamed". */
  notes: z.array(z.string()).default([]),
  advisories: z.array(Advisory).default([]),
});
export type Verdict = z.infer<typeof Verdict>;

// ── Store config ─────────────────────────────────────────────────────────────

export const StoreConfig = z.strictObject({
  version: z.string().default(MODEL_VERSION),
  /** Short random identifier generated once per repository at store init. */
  nonce: z.string(),
  /** Instruction files that get the one-line banner. Defaults: CLAUDE.md, AGENTS.md, editor rule files. */
  instructionFiles: z.array(z.string()).optional(),
  /** Globs for documents `check --write` never stamps. */
  pristine: z.array(z.string()).optional(),
});
export type StoreConfig = z.infer<typeof StoreConfig>;

export const SCHEMAS = {
  Selector,
  SelectorBundle,
  Anchor,
  Verifier,
  Edge,
  Document,
  Proposition,
  Assertion,
  RemediationAction,
  Remediation,
  Verdict,
  StoreConfig,
} as const;
