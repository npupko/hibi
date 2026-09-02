/**
 * Locating a stored `text-quote` in the current text.
 *
 * Exact occurrences are found first and ranked by the stored prefix/suffix
 * context; when none exists the Bitap cascade (vendored diff-match-patch)
 * finds the nearest fuzzy match to the stored position and the fuzzy error
 * budget decides whether that candidate counts as found.
 */

import type * as z from "zod";
import type {
  Region,
  TextPositionSelector,
  TextQuoteSelector,
} from "../core/model.ts";
import { MATCH_MAX_BITS, matchMain } from "../vendor/bitap.ts";
import { levenshtein, normalizeText, textSimilarity } from "./normalize.ts";
import {
  AMBIGUOUS_MIN_QUOTE_LENGTH,
  BITAP,
  fuzzyErrorBudget,
  TEXT_QUOTE_CONTEXT,
} from "./params.ts";

type TextQuote = z.infer<typeof TextQuoteSelector>;
type TextPosition = z.infer<typeof TextPositionSelector>;

export interface LocateResult {
  region: Region | null;
  /** True when several exact occurrences scored equally on context. */
  ambiguous: boolean;
  /** Whether the region came from an exact occurrence or a fuzzy match. */
  how: "exact" | "fuzzy" | "none";
}

/** Every exact occurrence of `needle` in `text`. */
export function exactOccurrences(text: string, needle: string): number[] {
  const out: number[] = [];
  if (!needle) return out;
  let i = text.indexOf(needle);
  while (i !== -1) {
    out.push(i);
    i = text.indexOf(needle, i + needle.length);
  }
  return out;
}

/** Context score of an occurrence at `at`: prefix + suffix similarity to the stored context. */
export function contextScore(
  text: string,
  at: number,
  length: number,
  tq: TextQuote,
): number {
  const pre = text.slice(Math.max(0, at - TEXT_QUOTE_CONTEXT), at);
  const suf = text.slice(at + length, at + length + TEXT_QUOTE_CONTEXT);
  return textSimilarity(pre, tq.prefix) + textSimilarity(suf, tq.suffix);
}

/**
 * The end offset at which `text.slice(at, end)` has the same whitespace-
 * collapsed length as `exact`, so a span reformatted with extra whitespace
 * still covers the whole quote. Linear in the quote length.
 */
function collapsedEnd(text: string, at: number, exact: string): number {
  const target = normalizeText(exact).length;
  const limit = Math.min(text.length, at + exact.length * 2 + 16);
  let collapsed = 0;
  let prevWs = true;
  for (let i = at; i < limit; i++) {
    const ws = /\s/.test(text[i] as string);
    if (ws) {
      if (!prevWs) collapsed++;
    } else {
      collapsed++;
    }
    prevWs = ws;
    if (!ws && collapsed >= target) return i + 1;
  }
  return Math.min(text.length, at + exact.length);
}

/** Pick the better of two candidate ends by similarity to the stored quote. */
function betterEnd(
  text: string,
  at: number,
  exact: string,
  a: number,
  b: number,
): number {
  if (a === b) return a;
  const sa = textSimilarity(text.slice(at, a), exact);
  const sb = textSimilarity(text.slice(at, b), exact);
  return sb > sa ? b : a;
}

/**
 * Fuzzy-locate `tq.exact` near `bias` with the Bitap cascade:
 *   1. exact ≤ 32 chars: match directly.
 *   2. exact > 32 chars: match the first 32 chars to fix the start, then refine
 *      the end against the stored suffix.
 *   3. fallback: match the last 32 chars of the prefix and begin after it.
 * The 32-char cap is the Bitap word size, independent of the stored context.
 */
export function fuzzyLocate(
  text: string,
  tq: TextQuote,
  bias: number,
): Region | null {
  if (text.length === 0) return null;
  const clampBias = Math.max(0, Math.min(bias, text.length - 1));
  const exact = tq.exact;

  if (exact.length <= MATCH_MAX_BITS) {
    const at = matchMain(text, exact, clampBias, BITAP);
    if (at !== -1) {
      const naive = Math.min(at + exact.length, text.length);
      const end = betterEnd(
        text,
        at,
        exact,
        naive,
        collapsedEnd(text, at, exact),
      );
      return { start: at, end };
    }
  } else {
    const head = exact.slice(0, MATCH_MAX_BITS);
    const at = matchMain(text, head, clampBias, BITAP);
    if (at !== -1) {
      const naive = Math.min(at + exact.length, text.length);
      let end = betterEnd(
        text,
        at,
        exact,
        naive,
        collapsedEnd(text, at, exact),
      );
      if (tq.suffix && tq.suffix.length > 0) {
        const suf = tq.suffix.slice(0, MATCH_MAX_BITS);
        const sufAt = matchMain(text, suf, end, BITAP);
        if (sufAt !== -1) end = betterEnd(text, at, exact, end, sufAt);
      }
      return { start: at, end: Math.max(at, end) };
    }
  }

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

/** True when the located text is within the fuzzy error budget of the stored quote. */
export function withinErrorBudget(located: string, exact: string): boolean {
  const a = normalizeText(located);
  const b = normalizeText(exact);
  if (a === b) return true;
  return levenshtein(a, b) <= fuzzyErrorBudget(b.length);
}

/**
 * Locate a text-quote: exact occurrences ranked by context, else a fuzzy match
 * near the stored position that lies within the error budget. Position is a
 * tiebreaker only: it picks among equally-scored short quotes and biases the
 * fuzzy search, but never selects among long equal candidates (those are
 * reported `ambiguous`).
 */
export function localizeTextQuote(
  text: string,
  tq: TextQuote,
  tp: TextPosition | undefined,
): LocateResult {
  const bias = tp ? tp.start : 0;
  const hits = exactOccurrences(text, tq.exact);
  if (hits.length === 1) {
    const at = hits[0] as number;
    return {
      region: { start: at, end: at + tq.exact.length },
      ambiguous: false,
      how: "exact",
    };
  }
  if (hits.length > 1) {
    const scored = hits
      .map((at) => ({ at, score: contextScore(text, at, tq.exact.length, tq) }))
      .sort((a, b) => b.score - a.score || a.at - b.at);
    const best = scored[0] as { at: number; score: number };
    const second = scored[1] as { at: number; score: number };
    let at = best.at;
    let ambiguous = false;
    if (!(best.score > second.score)) {
      if (tq.exact.length < AMBIGUOUS_MIN_QUOTE_LENGTH) {
        // Short quote: position breaks the tie.
        at = scored
          .filter((s) => s.score === best.score)
          .sort((a, b) => Math.abs(a.at - bias) - Math.abs(b.at - bias))[0]
          ?.at as number;
      } else {
        ambiguous = true;
      }
    }
    return {
      region: { start: at, end: at + tq.exact.length },
      ambiguous,
      how: "exact",
    };
  }
  const region = fuzzyLocate(text, tq, bias);
  if (!region) return { region: null, ambiguous: false, how: "none" };
  if (!withinErrorBudget(text.slice(region.start, region.end), tq.exact)) {
    return { region: null, ambiguous: false, how: "none" };
  }
  return { region, ambiguous: false, how: "fuzzy" };
}

/** Substring of `text` for a region, clamped. */
export function regionText(text: string, region: Region): string {
  return text.slice(
    Math.max(0, region.start),
    Math.min(text.length, region.end),
  );
}
