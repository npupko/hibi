/**
 * Confidence fusion & grading (§17.3) — turns per-selector results into a graded
 * verdict. Tuned for precision over recall: never reports a drifted claim as
 * `fresh`; every missed drift is graded `moved` (re-verify), not clean.
 */
import type { ComputedState } from "../core/model.ts";
import {
  MIN_AGREEING_SELECTORS,
  BANDS,
  MOVE_AWARENESS_CHARS,
  VALUE_VETO_TEXTQUOTE_SIMILARITY,
  VALUE_VETO_CONFIDENCE,
} from "./params.ts";

export interface ResolvedSelector {
  kind: string;
  found: boolean;
  score: number;
  weight: number;
}

export interface GradeInput {
  selectors: ResolvedSelector[];
  /** ttl elapsed — short-circuits to `expired` before fusion (§17.3). */
  expired: boolean;
  /** Anchor has only coarse (path/glob) selectors — navigational, never stale. */
  coarseOnly: boolean;
  /** |located start − baseline start|, or null when not localized. */
  startDelta: number | null;
  textQuoteFound: boolean;
  /** Similarity of the located region vs the baseline `exact` (0 if not found). */
  textQuoteSimilarity: number;
  valueFound: boolean;
  /** 0 (changed) or 1 (unchanged). */
  valueScore: number;
}

export interface GradeResult {
  state: ComputedState;
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

/** Band a fused confidence into a computed state (§17.3). */
export function bandConfidence(c: number): ComputedState {
  if (c >= BANDS.fresh) return "fresh";
  if (c >= BANDS.moved) return "moved";
  if (c >= BANDS.stale) return "stale";
  return "ghost";
}

export function grade(input: GradeInput): GradeResult {
  const notes: string[] = [];

  // `expired` is determined before fusion, independent of confidence (§17.3).
  if (input.expired) {
    return { state: "expired", confidence: 0, notes: ["past ttl — time-based re-verification"] };
  }

  // Coarse anchors are navigational and are never reported as stale (§11.3).
  if (input.coarseOnly) {
    return { state: "fresh", confidence: 1, notes: ["coarse anchor — navigational, never stale"] };
  }

  // Fewer than two agreeing selectors → ghost, confidence forced to 0 (§17.3).
  const foundCount = input.selectors.filter((s) => s.found).length;
  if (foundCount < MIN_AGREEING_SELECTORS) {
    return {
      state: "ghost",
      confidence: 0,
      notes: [`only ${foundCount} selector(s) resolved (min ${MIN_AGREEING_SELECTORS})`],
    };
  }

  // Active value veto (§17.3): value changed (score 0) while text-quote is highly
  // confident we are at the right place → force stale.
  if (
    input.valueFound &&
    input.valueScore === 0 &&
    input.textQuoteFound &&
    input.textQuoteSimilarity >= VALUE_VETO_TEXTQUOTE_SIMILARITY
  ) {
    return { state: "stale", confidence: VALUE_VETO_CONFIDENCE, notes: ["value veto — anchored value changed"] };
  }

  const c = fuseConfidence(input.selectors);
  let state = bandConfidence(c);

  // Move-awareness: a `fresh` result whose start drifted > 4 chars → `moved` (§17.3).
  if (state === "fresh" && input.startDelta !== null && input.startDelta > MOVE_AWARENESS_CHARS) {
    state = "moved";
    notes.push(`located region moved ${input.startDelta} chars from baseline`);
  }

  return { state, confidence: c, notes };
}
