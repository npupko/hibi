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

/** A minimum of two agreeing selectors is required, else `ghost` (§10/§17.3). */
export const MIN_AGREEING_SELECTORS = 2;

/** Verdict bands on fused confidence C (§17.3). */
export const BANDS = {
  fresh: 0.8, // C ≥ 0.8 → fresh
  moved: 0.5, // 0.5 ≤ C < 0.8 → moved
  stale: 0.2, // 0.2 ≤ C < 0.5 → stale; C < 0.2 → ghost
} as const;

/** A `fresh` result is downgraded to `moved` when the start moved > this (§17.3). */
export const MOVE_AWARENESS_CHARS = 4;

/** text-position is "found" when content at the baseline offset is ≥ this similar (§17.3). */
export const POSITION_FOUND_SIMILARITY = 0.6;

/** Structural-only AST match (rename/whitespace) credits this partial score (§17.3). */
export const STRUCTURAL_ONLY_SCORE = 0.4;

/** Value veto trips when text-quote similarity is ≥ this and value changed (§17.3). */
export const VALUE_VETO_TEXTQUOTE_SIMILARITY = 0.9;
export const VALUE_VETO_CONFIDENCE = 0.3;
