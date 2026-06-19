/**
 * Confidence fusion & grading (§17.3) — turns per-selector results for *one
 * anchor side* into a graded `AnchorState`. The same grader runs on the doc
 * side and the code side (one vocabulary — ADR-001). Tuned for precision over
 * recall: never reports a drifted span as `unchanged`; every missed drift is
 * graded `moved` (re-verify), not clean.
 *
 * `expired` is **not** handled here — it is an orthogonal time flag computed by
 * the resolver, independent of confidence (§17.3).
 */
import type { AnchorState } from "../core/model.ts";
import {
  BANDS,
  MIN_AGREEING_SELECTORS,
  MOVE_AWARENESS_CHARS,
  VALUE_VETO_CONFIDENCE,
  VALUE_VETO_TEXTQUOTE_SIMILARITY,
} from "./params.ts";

export interface ResolvedSelector {
  kind: string;
  found: boolean;
  score: number;
  weight: number;
}

export interface GradeInput {
  selectors: ResolvedSelector[];
  /** Bundle has only coarse (path/glob) selectors — navigational, never drift. */
  coarseOnly: boolean;
  /** The quote matched in more than one place — yields `ambiguous` (§17.1). */
  ambiguous: boolean;
  /** |located start − baseline start|, or null when not localized. */
  startDelta: number | null;
  textQuoteFound: boolean;
  /** Similarity of the located region vs the baseline `exact` (0 if not found). */
  textQuoteSimilarity: number;
  /**
   * The text-quote relocated near-exactly and is the sole resolving selector —
   * trustworthy enough to satisfy the two-selector minimum (§17.3), so a moved
   * prose sentence grades `moved` instead of `orphaned`. Defaults to false.
   */
  soleStrongQuote?: boolean;
  valueFound: boolean;
  /** 0 (changed) or 1 (unchanged). */
  valueScore: number;
}

export interface GradeResult {
  state: AnchorState;
  confidence: number;
  notes: string[];
}

/** Fuse confidence `C = Σ(wᵢ·sᵢ) / Σ(wᵢ)` over the selectors that *resolved*. */
export function fuseConfidence(selectors: ResolvedSelector[]): number {
  const found = selectors.filter((s) => s.found);
  const wsum = found.reduce((a, s) => a + s.weight, 0);
  if (wsum === 0) return 0;
  const num = found.reduce((a, s) => a + s.weight * s.score, 0);
  return num / wsum;
}

/** Band a fused confidence into an anchor state (§17.3). */
export function bandConfidence(c: number): AnchorState {
  if (c >= BANDS.unchanged) return "unchanged";
  if (c >= BANDS.moved) return "moved";
  if (c >= BANDS.changed) return "changed";
  return "orphaned";
}

/** Grade one anchor side from its resolved selectors (§17.3). */
export function grade(input: GradeInput): GradeResult {
  // Coarse anchors are navigational and are never reported as drift (§11.3).
  if (input.coarseOnly) {
    return {
      state: "unchanged",
      confidence: 1,
      notes: ["coarse anchor — navigational, never drift"],
    };
  }

  // Fewer than two agreeing selectors → orphaned, confidence forced to 0 (§17.3) —
  // unless a single near-exact text-quote relocation vouches for the span (the
  // moved-prose case, where text-position drops out once the sentence moves).
  const foundCount = input.selectors.filter((s) => s.found).length;
  if (foundCount < MIN_AGREEING_SELECTORS && !input.soleStrongQuote) {
    return {
      state: "orphaned",
      confidence: 0,
      notes: [
        `only ${foundCount} selector(s) resolved (min ${MIN_AGREEING_SELECTORS})`,
      ],
    };
  }

  // Active value veto (§17.3): value changed (score 0) while text-quote is highly
  // confident we are at the right place → force `changed`.
  if (
    input.valueFound &&
    input.valueScore === 0 &&
    input.textQuoteFound &&
    input.textQuoteSimilarity >= VALUE_VETO_TEXTQUOTE_SIMILARITY
  ) {
    return {
      state: "changed",
      confidence: VALUE_VETO_CONFIDENCE,
      notes: ["value veto — anchored value changed"],
    };
  }

  const c = fuseConfidence(input.selectors);
  let state = bandConfidence(c);
  const notes: string[] = [];

  // Move-awareness: an `unchanged` result whose start drifted > 4 chars → `moved`.
  if (
    state === "unchanged" &&
    input.startDelta !== null &&
    input.startDelta > MOVE_AWARENESS_CHARS
  ) {
    state = "moved";
    notes.push(`located region moved ${input.startDelta} chars from baseline`);
  }

  // A located-but-multiply-matched quote yields `ambiguous` (§17.1/§17.3) — it
  // overrides a clean `unchanged`/`moved`, but a genuine content `changed`/
  // `orphaned` dominates (we cannot trust an ambiguous location either way).
  if (input.ambiguous && (state === "unchanged" || state === "moved")) {
    notes.push("quote matched in multiple places — ambiguous");
    return { state: "ambiguous", confidence: c, notes };
  }

  return { state, confidence: c, notes };
}
