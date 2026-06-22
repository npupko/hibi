/**
 * The canonical data model — the single source of truth (§5).
 *
 * Defined once in Zod v4. The versioned JSON Schema (`schemas/*.v1.json`), the
 * TypeScript types, and every language SDK are generated from this file (via
 * `z.toJSONSchema`). Claim-store records are validated against it at load.
 *
 * The computed model is **two axes that answer two different questions**
 * (ADR-001, PRD §4/§10/§18-C), each with a borrowed term-of-art vocabulary,
 * plus orthogonal flags:
 *   - Axis 1 — anchor resolution (`AnchorState`): one vocabulary applied per
 *     side (`doc:…` / `code:…`) — "can I find the span, and is it unchanged?"
 *   - Axis 2 — behavioral belief (`BehaviorState`): absent on non-behavioral
 *     claims — "do we still believe the documented behavior holds?"
 *   - `expired` is an orthogonal time flag, never a state.
 *
 * Lineage (§5): Proposition/Assertion ≈ Truth-Maintenance belief/justification;
 * Document edges ≈ ADR `superseded-by` / RFC `Obsoletes`; Anchor selectors ≈ W3C
 * Web Annotation TextQuoteSelector (+ tree-sitter for the structural selector);
 * AnchorState ≈ git status + hypothes.is `orphaned`; BehaviorState ≈ FEVER
 * `supported`/`refuted` + JTMS support-withdrawal.
 */
import * as z from "zod";

/** Schema version stamped into generated artifacts and the store config. */
export const MODEL_VERSION = "v1" as const;

// ── Status: four kinds, never conflated (§4/§10) ─────────────────────────────

/** Authored trust — set by the author, lives on the Proposition/Assertion. */
export const AuthoredTrust = z.enum(["verified", "inferred", "assumed"]);
export type AuthoredTrust = z.infer<typeof AuthoredTrust>;

/**
 * Enforcement — the record's creation-lifecycle, set by the workflow (§4/§9/§10).
 * Only `enforced` can produce a gating verdict or a strong banner; `suggested`
 * is advisory; `retired` is withdrawn; `unanchored-legacy` is an un-reanchorable
 * migrated copy, excluded from strong enforcement.
 */
export const Enforcement = z.enum([
  "suggested",
  "enforced",
  "retired",
  "unanchored-legacy",
]);
export type Enforcement = z.infer<typeof Enforcement>;

/**
 * Computed — Axis 1: anchor resolution. One vocabulary, applied to *each side*
 * (reported `doc:…` / `code:…`). Engine-only, ephemeral, never authored (§10).
 * Borrowed from git (`unchanged`/`moved`), W3C annotation (`ambiguous`), and
 * hypothes.is (`orphaned`). The side is a *separate field*, never baked into the
 * word (ADR-001 parallelism invariant).
 */
export const AnchorState = z.enum([
  "unchanged", // found, identical
  "moved", // found, relocated (same content)
  "changed", // found, content differs
  "ambiguous", // matches in several places
  "orphaned", // span deleted / unresolvable
]);
export type AnchorState = z.infer<typeof AnchorState>;

/**
 * Computed — Axis 2: behavioral belief. Absent on non-behavioral claims (no
 * peer status; displayed `n/a`, never stored). Engine-only, ephemeral (§10).
 * Borrowed from FEVER (`supported`/`refuted`) and reason-maintenance
 * (support-withdrawn → `at-risk`). Shares no member with `AuthoredTrust`.
 * Only `refuted` may gate (§7.4); `at-risk` is advisory.
 */
export const BehaviorState = z.enum([
  "unverified", // behavioral, untested, nothing changed (resting)
  "at-risk", // reachable evidence changed; belief no longer justified
  "supported", // a linked verifier passed
  "refuted", // a linked verifier failed
]);
export type BehaviorState = z.infer<typeof BehaviorState>;

/**
 * Claim kind — the author's declaration that a claim is behavioral, and of what
 * kind (§5/§17.6). Drives Tier-3 classification explicitly; absent, a
 * deterministic keyword heuristic classifies. A label, never a verdict.
 */
export const ClaimKind = z.enum([
  "ordering",
  "retry",
  "complexity",
  "concurrency",
  "caching",
  "validation",
  "error-handling",
]);
export type ClaimKind = z.infer<typeof ClaimKind>;

/** Document lifecycle — set by the engine from edges/actions (§4/§10). */
export const DocumentLifecycle = z.enum([
  "active",
  "amended",
  "superseded",
  "archived",
  "retracted",
]);
export type DocumentLifecycle = z.infer<typeof DocumentLifecycle>;

// ── Anchor selectors (discriminated union on `kind` — §4) ────────────────────

/**
 * `text-quote` — exact + prefix + suffix snippet (W3C TextQuoteSelector). The
 * base selector on both sides; always present for a precise anchor. Carries the
 * baseline `exact` captured at record time.
 */
export const TextQuoteSelector = z.object({
  kind: z.literal("text-quote"),
  exact: z.string(),
  prefix: z.string().default(""),
  suffix: z.string().default(""),
});

/** `text-position` — line/char range; a cheap first guess and corroboration hint. */
export const TextPositionSelector = z.object({
  kind: z.literal("text-position"),
  start: z.number().int().nonnegative(),
  end: z.number().int().nonnegative(),
});

/**
 * `ast-node` — the enclosing construct via tree-sitter, snapped to the smallest
 * enclosing *named* node (a code symbol on the code side, or a document
 * structural path on the doc side). Stores the two-tier baseline AST fingerprint
 * (structural + semantic) and the node type, captured at record time.
 */
export const AstNodeSelector = z.object({
  kind: z.literal("ast-node"),
  language: z.string(),
  nodeType: z.string(),
  structuralHash: z.string(),
  semanticHash: z.string(),
});

/**
 * `value` — *(code side)* an extracted structured value so a `5 → 50` change
 * trips even if nothing else moves. Which AST node kinds carry a literal is
 * configured per-grammar (§17.4).
 */
export const ValueSelector = z.object({
  kind: z.literal("value"),
  language: z.string(),
  nodeKind: z.string(),
  value: z.string(),
});

/**
 * `inline-id` — *(optional, owned docs only)* a hidden marker (e.g.
 * `<!-- hibi:claim id=… -->`) that *identifies* the record near the paragraph;
 * it stabilizes re-anchoring but **never restates the claim**, and is never
 * required (§4/§8/§18-B). If marker and prose disagree, the prose wins — so this
 * selector aids localization/disambiguation only and is never a fusion score.
 */
export const InlineIdSelector = z.object({
  kind: z.literal("inline-id"),
  id: z.string(),
});

/** `path` (coarse) — a file → an edge: navigation and blast-radius only. */
export const PathSelector = z.object({
  kind: z.literal("path"),
  path: z.string(),
});

/** `glob` (coarse) — a directory/glob → an edge: navigation and blast-radius only. */
export const GlobSelector = z.object({
  kind: z.literal("glob"),
  glob: z.string(),
});

/** The precise selector kinds — these can be graded into a drift state. */
export const PRECISE_SELECTOR_KINDS = [
  "text-quote",
  "text-position",
  "ast-node",
  "value",
  "inline-id",
] as const;
/** The coarse selector kinds — navigational; never reported as drift (§11.3). */
export const COARSE_SELECTOR_KINDS = ["path", "glob"] as const;

export const Selector = z.discriminatedUnion("kind", [
  TextQuoteSelector,
  TextPositionSelector,
  AstNodeSelector,
  ValueSelector,
  InlineIdSelector,
  PathSelector,
  GlobSelector,
]);
export type Selector = z.infer<typeof Selector>;
export type SelectorKind = Selector["kind"];

/**
 * SelectorBundle — the multi-selector list for *one side* of an anchor (§4). A
 * bundle of redundant, independently-resolvable selectors against one `file`;
 * the engine resolves the most robust available, falls back down the chain, and
 * cross-corroborates. This *is* the baseline snapshot for that side (§6).
 */
export const SelectorBundle = z.object({
  /** The file this side's selectors resolve against. */
  file: z.string(),
  selectors: z.array(Selector).min(1),
});
export type SelectorBundle = z.infer<typeof SelectorBundle>;

/**
 * Anchor (bidirectional, composite — §4). A value-object on the Assertion: a
 * **doc-side** bundle (the documented sentence) plus **one or more code-side**
 * bundles (the code it describes). The current artifact span is authoritative;
 * the stored quote is anchoring material + an audit cache, never the truth
 * (§18-B). `code` may be empty for a doc-only `suggested` claim awaiting a code
 * target; an `enforced` claim requires both sides to resolve (§9, validated at
 * `record`, not in the schema).
 */
export const Anchor = z.object({
  doc: SelectorBundle,
  code: z.array(SelectorBundle).default([]),
});
export type Anchor = z.infer<typeof Anchor>;

// ── Behavioral evidence (§5/§17.6) ───────────────────────────────────────────

/**
 * Verifier — an executable-evidence link that upgrades behavioral risk to a real
 * verdict (§5/§17.6). If a verifier runs and fails → `refuted`; if none is
 * declared, a claim is never marked `supported`. Executed by an out-of-process
 * runner resolver (§7), never in core.
 */
export const Verifier = z.object({
  kind: z.enum([
    "example",
    "snapshot",
    "contract",
    "property",
    "formal",
    "command",
  ]),
  /** Names a test/command to run. */
  ref: z.string(),
  /** Optional human note on what this verifier proves. */
  proves: z.string().optional(),
});
export type Verifier = z.infer<typeof Verifier>;

/**
 * BehaviorScope — the deterministic blast-radius the change-gate watches for a
 * behavioral claim (§5/§17.6). Absent → the change-gate falls back to the
 * anchored node + its file.
 */
export const BehaviorScope = z.object({
  rootSymbols: z.array(z.string()).default([]),
  /** Transitive-callee depth the change-gate walks (default 2 — §17.6). */
  reachableDepth: z.number().int().nonnegative().default(2),
  include: z.array(z.string()).default([]),
  exclude: z.array(z.string()).default([]),
});
export type BehaviorScope = z.infer<typeof BehaviorScope>;

// ── Document edges (forward-authored, reverse-derived — §4, §6) ──────────────

/** `supersedes` (full) → targets a Document; the old doc → `superseded`. */
export const SupersedesEdge = z.object({
  type: z.literal("supersedes"),
  /** documentId of the superseded (old) document. */
  target: z.string(),
  derived: z.boolean().default(false),
});

/** Reverse of `supersedes`, derived by the engine onto the old document. */
export const SupersededByEdge = z.object({
  type: z.literal("superseded-by"),
  /** documentId of the superseding (new) document. */
  source: z.string(),
  derived: z.boolean().default(true),
});

/** `amends` (partial) → targets one or more Propositions in a Document. */
export const AmendsEdge = z.object({
  type: z.literal("amends"),
  /** documentId of the amended (old) document. */
  target: z.string(),
  /** propositionIds amended within the target document. */
  propositions: z.array(z.string()).min(1),
  derived: z.boolean().default(false),
});

/** Reverse of `amends`, derived by the engine onto the old document. */
export const AmendedByEdge = z.object({
  type: z.literal("amended-by"),
  source: z.string(),
  propositions: z.array(z.string()).min(1),
  derived: z.boolean().default(true),
});

export const Edge = z.discriminatedUnion("type", [
  SupersedesEdge,
  SupersededByEdge,
  AmendsEdge,
  AmendedByEdge,
]);
export type Edge = z.infer<typeof Edge>;

// ── Entities (§5) ────────────────────────────────────────────────────────────

/** Document — a file. Owns lifecycle and supersession edges. */
export const Document = z.object({
  id: z.string(),
  path: z.string(),
  lifecycle: DocumentLifecycle.default("active"),
  edges: z.array(Edge).default([]),
  frontmatterStatus: z.string().optional(),
});
export type Document = z.infer<typeof Document>;

/**
 * Proposition — the timeless meaning; the target of `amends`; the dedup unit.
 * `textCache` is a **non-authoritative** copy of the documented sentence, kept
 * only for audit, diffing, and `orphaned`-claim recovery; **the authoritative
 * text is the current doc span**, re-read at `check` time via the doc-side
 * anchor (§4, §8, §18-B). Identity is authored/explicit (`id` / content
 * `fingerprint` of the confirmed text), never similarity-computed (§5).
 */
export const Proposition = z.object({
  id: z.string(),
  /** Non-authoritative cache of the documented sentence (§5/§18-B). */
  textCache: z.string(),
  authoredTrust: AuthoredTrust,
  fingerprint: z.string(),
});
export type Proposition = z.infer<typeof Proposition>;

/** Assertion — one verification instance. Carries the bidirectional Anchor. */
export const Assertion = z.object({
  id: z.string(),
  propositionId: z.string(),
  documentId: z.string(),
  owner: z.string(),
  /** The `@ref` (commit) last verified against. */
  ref: z.string(),
  anchor: Anchor,
  /** The record's creation-lifecycle (§4/§9). */
  enforcement: Enforcement.default("suggested"),
  /** Author's behavioral-kind declaration; drives Tier-3 routing (§17.6). */
  claimKind: ClaimKind.optional(),
  /** Executable-evidence links that upgrade behavioral risk (§5/§17.6). */
  verifiers: z.array(Verifier).default([]),
  /** Deterministic blast-radius for the behavioral change-gate (§5/§17.6). */
  behaviorScope: BehaviorScope.optional(),
  /** Optional ISO-8601 instant; past it the computed `expired` flag is set. */
  ttl: z.string().optional(),
  /** Open key/value bag for resolver-specific metadata the core does not interpret. */
  attrs: z.record(z.string(), z.unknown()).default({}),
});
export type Assertion = z.infer<typeof Assertion>;

// ── Verdict (ephemeral — never persisted, §5/§6) ─────────────────────────────

/** Per-selector contribution to the fused confidence (§17.3). */
export const SelectorScore = z.object({
  kind: z.string(),
  /** Whether the selector resolved ("found") per §17.3. */
  found: z.boolean(),
  /** The selector's similarity score in [0,1]. */
  score: z.number(),
  /** The fusion weight for this selector kind. */
  weight: z.number(),
});
export type SelectorScore = z.infer<typeof SelectorScore>;

/** A located region in the current text. */
export const Region = z.object({
  start: z.number().int().nonnegative(),
  end: z.number().int().nonnegative(),
});
export type Region = z.infer<typeof Region>;

/**
 * ChangedEvidence — what reachable evidence changed and triggered behavioral
 * risk / a `changed` anchor state (§5/§17.6). Each entry names the changed path
 * and the kind of evidence that moved.
 */
export const ChangedEvidence = z.object({
  /** The file (or selector locus) whose evidence changed. */
  path: z.string(),
  /** What kind of evidence changed: `value`, `ast`, `text`, `callee`, `import`, `verifier-source`. */
  kind: z.string(),
  /** Optional human-readable detail. */
  detail: z.string().optional(),
});
export type ChangedEvidence = z.infer<typeof ChangedEvidence>;

/** Advisory note from a quarantined Tier-3 resolver — advises, never gates (§7.4). */
export const Advisory = z.object({
  resolver: z.string(),
  message: z.string(),
  /** Free-form confidence the advisor reports; never folded into the verdict. */
  confidence: z.number().optional(),
});
export type Advisory = z.infer<typeof Advisory>;

/** Bulky located evidence — trails the decision fields in the JSON shape (§9). */
export const VerdictEvidence = z.object({
  /** The located doc-side region (the documented sentence), when found. */
  docRegion: Region.optional(),
  /** The located code-side region per code bundle, when found. */
  codeRegions: z.array(Region).default([]),
  /** Fused confidence of the primary (code, else doc) side (§17.3). */
  confidence: z.number(),
  /** Which selectors agreed, and how (primary side). */
  selectorScores: z.array(SelectorScore).default([]),
  /** Reachable evidence that changed, triggering `changed`/`at-risk` (§17.6). */
  changedEvidence: z.array(ChangedEvidence).default([]),
  /** The ref the assertion was verified against. */
  ref: z.string().optional(),
});
export type VerdictEvidence = z.infer<typeof VerdictEvidence>;

// ── Remediation menu (deterministic verdict→action lookup, §9) ───────────────

/**
 * How safely an action can be applied — Rust `Applicability`-style (§9):
 *   - `auto`         — safe to apply mechanically (e.g. a pure relocation).
 *   - `needs-review` — apply but review the result (the anchor/value may differ).
 *   - `manual`       — a human/agent must decide intent before acting.
 */
export const RemediationApplicability = z.enum([
  "auto",
  "needs-review",
  "manual",
]);
export type RemediationApplicability = z.infer<typeof RemediationApplicability>;

/**
 * What kind of work the action is:
 *   - `deterministic` — hibi performs it via a `command` (e.g. `reanchor`/`retire`).
 *   - `prose`         — a human/agent must rewrite the doc or code (no command).
 */
export const RemediationEffect = z.enum(["deterministic", "prose"]);
export type RemediationEffect = z.infer<typeof RemediationEffect>;

/**
 * One entry in a verdict's remediation menu (§9). A `deterministic` action
 * carries a ready-to-run `command` with the claim id pre-filled; a `prose`
 * action carries none, because it needs a human/agent to edit text. A command
 * is NEVER pre-filled when it cannot succeed (e.g. a bare `reanchor` on an
 * orphan, which has no span to relocate to).
 */
export const RemediationAction = z.object({
  /** Stable kebab token, machine-stable across releases (e.g. `reanchor`). */
  id: z.string(),
  /** One-line human label. */
  title: z.string(),
  applicability: RemediationApplicability,
  effect: RemediationEffect,
  /** Why this action applies, derived from the verdict's states/evidence. */
  rationale: z.string(),
  /** Ready-to-run command (deterministic, runnable actions only). */
  command: z.string().optional(),
});
export type RemediationAction = z.infer<typeof RemediationAction>;

/**
 * The deterministic remediation menu for a verdict (§9). A *menu*, not a single
 * prescription: hibi routes attention but cannot know developer intent (was the
 * code change deliberate?), so `recommended` is set only when the next step is
 * unambiguous, and `actions` is ordered safest/most-severe-first. The
 * verdict→action mapping is a fixed lookup table (`remediationFor`), never a
 * model decision (§7/§11).
 */
export const Remediation = z.object({
  /** The single best action id, or `null` when intent is ambiguous. */
  recommended: z.string().nullable(),
  actions: z.array(RemediationAction).default([]),
});
export type Remediation = z.infer<typeof Remediation>;

/**
 * Verdict — the engine's per-Assertion result, recomputed live, never stored.
 * **Verdict-first** (§9): leads with the decision (the two per-side anchor
 * states, the behavioral state, the `expired`/`gates` flags, and the
 * `remediation` menu) and trails the bulky `evidence`, so a truncated read still
 * surfaces the verdict and what to do about it. Means "suspect — re-verify",
 * never "the claim is false" (§11).
 */
export const Verdict = z.object({
  assertionId: z.string(),
  propositionId: z.string(),
  documentId: z.string(),
  /** Axis 1 — anchor resolution, doc side. */
  doc: AnchorState,
  /** Axis 1 — anchor resolution, code side (aggregated worst over code bundles). */
  code: AnchorState,
  /** Axis 2 — behavioral belief; absent on non-behavioral claims. */
  behavior: BehaviorState.optional(),
  /** Orthogonal time flag: past the Assertion's `ttl`. */
  expired: z.boolean(),
  /**
   * Whether this verdict gates the build (exit 2): true iff the claim is
   * `enforced` and (doc or code ∈ {changed, orphaned, ambiguous} | `expired` |
   * `behavior === "refuted"`). `moved`/`at-risk` never gate (§9/§17.3).
   */
  gates: z.boolean(),
  /**
   * Deterministic verdict→action menu (§9): a decision field that leads
   * alongside `gates`. `null` on a clean verdict (nothing to remediate). hibi's
   * own resolvers always set it; the default keeps the registry tolerant of a
   * wire verdict from an external resolver that omits it (the registry recomputes
   * the menu from the computed states regardless — §7.4).
   */
  remediation: Remediation.nullable().default(null),
  evidence: VerdictEvidence,
  /** Human-readable explanation crumbs (e.g. "value veto", "structural-only"). */
  notes: z.array(z.string()).default([]),
  /** Non-gating advice from Tier-3 resolvers. */
  advisories: z.array(Advisory).default([]),
});
export type Verdict = z.infer<typeof Verdict>;

// ── Store config ─────────────────────────────────────────────────────────────

/** `.claims/config.json` — holds the per-repository banner nonce (§17.5). */
export const StoreConfig = z.object({
  version: z.string().default(MODEL_VERSION),
  /** Short random identifier generated once per repository at store init. */
  nonce: z.string(),
  /**
   * Attention-budget instruction files that get the compact single-line banner
   * (§8). Defaults applied by the engine when absent: CLAUDE.md, AGENTS.md,
   * editor rule files.
   */
  instructionFiles: z.array(z.string()).optional(),
});
export type StoreConfig = z.infer<typeof StoreConfig>;

/** The full set of schemas exported to JSON Schema by `scripts/gen-schemas.ts`. */
export const SCHEMAS = {
  Selector,
  SelectorBundle,
  Anchor,
  Verifier,
  BehaviorScope,
  Edge,
  Document,
  Proposition,
  Assertion,
  RemediationAction,
  Remediation,
  Verdict,
  StoreConfig,
} as const;
