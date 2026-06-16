/**
 * The canonical data model — the single source of truth (§5).
 *
 * Defined once in Zod v4. The versioned JSON Schema (`schemas/*.v1.json`), the
 * TypeScript types, and every language SDK are generated from this file (via
 * `z.toJSONSchema`). Claim-store records are validated against it at load.
 *
 * Lineage (§5): Proposition/Assertion ≈ Truth-Maintenance belief/justification;
 * Document edges ≈ ADR `superseded-by` / RFC `Obsoletes`; Anchor selectors ≈ W3C
 * Web Annotation TextQuoteSelector (+ tree-sitter for the structural selector).
 */
import * as z from "zod";

/** Schema version stamped into generated artifacts and the store config. */
export const MODEL_VERSION = "v1" as const;

// ── Status enums (final — §10) ──────────────────────────────────────────────

/** Authored trust — set by the author, lives on Proposition/Assertion. */
export const AuthoredTrust = z.enum(["verified", "inferred", "assumed"]);
export type AuthoredTrust = z.infer<typeof AuthoredTrust>;

/** Computed state — set by the engine, never authored, ephemeral. */
export const ComputedState = z.enum(["fresh", "moved", "stale", "ghost", "expired"]);
export type ComputedState = z.infer<typeof ComputedState>;

/** Document lifecycle — set by the engine from edges/actions. */
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
 * base selector; always present for a precise anchor. Carries the baseline
 * `exact` captured at record time.
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
 * enclosing *named* node. Stores the two-tier baseline AST fingerprint
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
 * `value` — an extracted structured value so a `5 → 50` change trips even if
 * nothing else moves. Which AST node kinds carry a literal is configured
 * per-grammar (§17.4).
 */
export const ValueSelector = z.object({
  kind: z.literal("value"),
  language: z.string(),
  nodeKind: z.string(),
  value: z.string(),
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

/** The precise selector kinds — these can be graded stale. */
export const PRECISE_SELECTOR_KINDS = ["text-quote", "text-position", "ast-node", "value"] as const;
/** The coarse selector kinds — navigational; never reported as stale (§11.3). */
export const COARSE_SELECTOR_KINDS = ["path", "glob"] as const;

export const Selector = z.discriminatedUnion("kind", [
  TextQuoteSelector,
  TextPositionSelector,
  AstNodeSelector,
  ValueSelector,
  PathSelector,
  GlobSelector,
]);
export type Selector = z.infer<typeof Selector>;
export type SelectorKind = Selector["kind"];

/**
 * Anchor (composite, multi-selector — §4). A value-object on the Assertion: a
 * bundle of redundant, independently-resolvable selectors. The Anchor *is* the
 * baseline snapshot (§6) — freshness is computed from (stored Anchor) vs
 * (current working tree) alone.
 */
export const Anchor = z.object({
  /** The file the precise selectors resolve against (the anchored code file). */
  file: z.string(),
  selectors: z.array(Selector).min(1),
});
export type Anchor = z.infer<typeof Anchor>;

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
 * Identity is authored/explicit (`id`/content `fingerprint`), never
 * similarity-computed (§5).
 */
export const Proposition = z.object({
  id: z.string(),
  text: z.string(),
  authoredTrust: AuthoredTrust,
  fingerprint: z.string(),
});
export type Proposition = z.infer<typeof Proposition>;

/** Assertion — one verification instance. Carries the composite Anchor. */
export const Assertion = z.object({
  id: z.string(),
  propositionId: z.string(),
  documentId: z.string(),
  owner: z.string(),
  /** The `@ref` (commit) last verified against. */
  ref: z.string(),
  anchor: Anchor,
  /** Optional ISO-8601 instant; past it the computed state is `expired`. */
  ttl: z.string().optional(),
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

/** Advisory note from a quarantined Tier-3 resolver — advises, never gates (§7.4). */
export const Advisory = z.object({
  resolver: z.string(),
  message: z.string(),
  /** Free-form confidence the advisor reports; never folded into the verdict. */
  confidence: z.number().optional(),
});
export type Advisory = z.infer<typeof Advisory>;

/**
 * Verdict — the engine's per-Assertion result, recomputed live, never stored.
 * Means "suspect — re-verify", never "the claim is false" (§11).
 */
export const Verdict = z.object({
  assertionId: z.string(),
  propositionId: z.string(),
  documentId: z.string(),
  state: ComputedState,
  confidence: z.number(),
  region: Region.optional(),
  /** Which selectors agreed, and how. */
  selectorScores: z.array(SelectorScore).default([]),
  /** The ref the assertion was verified against. */
  ref: z.string().optional(),
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
});
export type StoreConfig = z.infer<typeof StoreConfig>;

/** The full set of schemas exported to JSON Schema by `scripts/gen-schemas.ts`. */
export const SCHEMAS = {
  Selector,
  Anchor,
  Edge,
  Document,
  Proposition,
  Assertion,
  Verdict,
  StoreConfig,
} as const;
