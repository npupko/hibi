/**
 * Fixed resolution parameters. Every constant here changes a verdict, so each
 * is named and documented; there are no hidden weights.
 */
import type { BitapOptions } from "../vendor/bitap.ts";

/**
 * The fuzzy error budget for locating a stored quote (Hypothesis uses
 * `min(256, quote.length / 2)`). Bitap accepts a candidate whose error rate on
 * the matched pattern is at most `FUZZY_ERROR_RATIO`; the located span is then
 * accepted only when its edit distance to the stored quote is at most
 * `min(FUZZY_ERROR_CAP, floor(quote.length * FUZZY_ERROR_RATIO))`. A candidate
 * over the budget is treated as not found (`orphaned`).
 */
export const FUZZY_ERROR_RATIO = 0.4;
export const FUZZY_ERROR_CAP = 256;

export function fuzzyErrorBudget(quoteLength: number): number {
  return Math.min(FUZZY_ERROR_CAP, Math.floor(quoteLength * FUZZY_ERROR_RATIO));
}

/** Bitap options derived from the error budget; `matchDistance` keeps position a weak bias. */
export const BITAP: BitapOptions = {
  matchThreshold: FUZZY_ERROR_RATIO,
  matchDistance: 100000,
};

/** How many characters of prefix and suffix a text-quote stores. */
export const TEXT_QUOTE_CONTEXT = 48;

/**
 * Normalized text similarity at or above this counts as the same text (then
 * `unchanged` or `moved`, subject to the AST and value checks). Below it the
 * span is `changed`.
 */
export const SAME_TEXT_SIMILARITY = 0.9;

/** A same-text span whose start moved by more than this many chars is `moved`. */
export const MOVE_AWARENESS_CHARS = 4;

/**
 * Minimum quote length for multiple-match (`ambiguous`) detection. A shorter
 * quote occurs everywhere; its candidates are picked by position instead.
 */
export const AMBIGUOUS_MIN_QUOTE_LENGTH = 8;
