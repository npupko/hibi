/**
 * Fixed grading parameters (§10, §17.3). Where any value here differs from a
 * prototype, §17 is authoritative; these constants mirror it exactly.
 */
import type { BitapOptions } from "../vendor/bitap.ts";

/** Selector fusion weights (§10/§17.3). */
export const WEIGHTS = {
  "ast-node": 0.35,
  "text-quote": 0.3,
  value: 0.2,
  "text-position": 0.15,
} as const;

/** Bitap fuzzy-match parameters (§10/§17.1) and the text-quote context window. */
export const BITAP: BitapOptions = {
  matchThreshold: 0.4,
  matchDistance: 100000,
};
export const TEXT_QUOTE_CONTEXT = 48;

/** A minimum of two agreeing selectors is required, else `orphaned` (§10/§17.3). */
export const MIN_AGREEING_SELECTORS = 2;

/**
 * Verdict bands on fused confidence C (§17.3), keyed by the `AnchorState` the
 * band yields. Applied identically on the doc and code sides (one vocabulary).
 */
export const BANDS = {
  unchanged: 0.8, // C ≥ 0.8 → unchanged
  moved: 0.5, // 0.5 ≤ C < 0.8 → moved
  changed: 0.2, // 0.2 ≤ C < 0.5 → changed; C < 0.2 → orphaned
} as const;

/** An `unchanged` result is downgraded to `moved` when the start moved > this (§17.3). */
export const MOVE_AWARENESS_CHARS = 4;

/**
 * Minimum length for a `text-quote` `exact` to be eligible for multiple-match
 * (`ambiguous`) detection — a too-short quote occurs everywhere and is not a
 * meaningful ambiguity signal (§17.1/§17.3).
 */
export const AMBIGUOUS_MIN_QUOTE_LENGTH = 8;

/** text-position is "found" when content at the baseline offset is ≥ this similar (§17.3). */
export const POSITION_FOUND_SIMILARITY = 0.6;

/** Structural-only AST match (rename/whitespace) credits this partial score (§17.3). */
export const STRUCTURAL_ONLY_SCORE = 0.4;

/** Value veto trips when text-quote similarity is ≥ this and value changed (§17.3). */
export const VALUE_VETO_TEXTQUOTE_SIMILARITY = 0.9;
export const VALUE_VETO_CONFIDENCE = 0.3;

/**
 * A near-exact text-quote relocation (similarity ≥ this) is trustworthy on its
 * own, so it satisfies the two-selector minimum even when it is the only
 * selector that resolved (§17.3). This keeps a *moved* prose sentence — whose
 * bundle carries only text-quote + text-position, and whose text-position fails
 * once the sentence relocates — graded `moved`, not `orphaned`. A genuine
 * deletion yields no high-similarity match, so it still falls to `orphaned`.
 */
export const STRONG_TEXTQUOTE_SIMILARITY = 0.9;
