/**
 * Tier-1 localization (§17.1): re-locate a `text-quote` selector in the current
 * text via the Bitap cascade, biased toward the stored `text-position` start.
 */

import type * as z from "zod";
import type {
  Region,
  TextPositionSelector,
  TextQuoteSelector,
} from "../core/model.ts";
import { MATCH_MAX_BITS, matchMain } from "../vendor/bitap.ts";
import { BITAP } from "./params.ts";

type TextQuote = z.infer<typeof TextQuoteSelector>;
type TextPosition = z.infer<typeof TextPositionSelector>;

/**
 * Locate a `text-quote` in `text` near `bias`, returning the region or null.
 *
 * Implements the §17.1 Bitap cascade with the **32-char Bitap word-size cap**
 * (`MATCH_MAX_BITS`) — the width of one machine word the bitap algorithm matches
 * per pass. This is a matching-algorithm limit and is NOT the 48-char stored
 * text-quote *context window* (`TEXT_QUOTE_CONTEXT` in `params.ts`), which is how
 * much prefix/suffix a selector *stores* at record time. They are independent
 * numbers that happen to both bound text-quote handling.
 *
 *   1. exact ≤ 32 chars → match directly, region `[at, at+len(exact))`.
 *   2. exact > 32 chars → match the first 32 chars to fix the start, set the end
 *      to `at+len(exact)`, then refine the end against up to 32 chars of suffix.
 *   3. fallback → match the last 32 chars of the prefix; begin just after it.
 */
export function localizeTextQuote(
  text: string,
  tq: TextQuote,
  bias: number,
): Region | null {
  if (text.length === 0) return null;
  const clampBias = Math.max(0, Math.min(bias, text.length - 1));
  const exact = tq.exact;

  if (exact.length <= MATCH_MAX_BITS) {
    const at = matchMain(text, exact, clampBias, BITAP);
    if (at !== -1)
      return { start: at, end: Math.min(at + exact.length, text.length) };
  } else {
    const head = exact.slice(0, MATCH_MAX_BITS);
    const at = matchMain(text, head, clampBias, BITAP);
    if (at !== -1) {
      let end = Math.min(at + exact.length, text.length);
      if (tq.suffix && tq.suffix.length > 0) {
        const suf = tq.suffix.slice(0, MATCH_MAX_BITS);
        const sufAt = matchMain(text, suf, end, BITAP);
        if (sufAt !== -1) end = sufAt; // region ends where the suffix begins
      }
      return { start: at, end: Math.max(at, end) };
    }
  }

  // (3) fallback via prefix.
  if (tq.prefix && tq.prefix.length > 0) {
    const pre = tq.prefix.slice(-MATCH_MAX_BITS);
    const preAt = matchMain(text, pre, clampBias, BITAP);
    if (preAt !== -1) {
      const start = preAt + pre.length;
      return { start, end: Math.min(start + exact.length, text.length) };
    }
  }
  return null;
}

/** The bias offset for localization: the stored `text-position` start, or 0. */
export function positionBias(tp: TextPosition | undefined): number {
  return tp ? tp.start : 0;
}

/** Substring of `text` for a region, clamped. */
export function regionText(text: string, region: Region): string {
  return text.slice(
    Math.max(0, region.start),
    Math.min(text.length, region.end),
  );
}
