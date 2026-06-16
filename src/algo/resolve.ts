/**
 * Drift resolution (§6 layered cheapest-first with corroboration; §17).
 *
 * Given an Assertion's Anchor and the *current* file text (plus an optional
 * tree-sitter analyzer for Tier-2 selectors), localize each selector, score it,
 * and fuse into a graded Verdict. Freshness is computed from (stored Anchor) vs
 * (current working tree) alone — the engine never reads a historical revision.
 */
import type { Anchor, Assertion, Verdict, Selector, Region } from "../core/model.ts";
import { COARSE_SELECTOR_KINDS } from "../core/model.ts";
import { WEIGHTS, POSITION_FOUND_SIMILARITY, STRUCTURAL_ONLY_SCORE } from "./params.ts";
import { localizeTextQuote, positionBias, regionText } from "./localize.ts";
import { textSimilarity, collapseWhitespace } from "./normalize.ts";
import { grade, type ResolvedSelector } from "./fusion.ts";

/**
 * Tier-2 analyzer hook — implemented with tree-sitter in Layer 5. Given the
 * current text and the localized region, snaps the enclosing named node and
 * reports its two-tier hash and any extracted literal value.
 */
export interface AstAnalysis {
  nodeType: string;
  structuralHash: string;
  semanticHash: string;
  /** The snapped node's region in the current text. */
  region: Region;
}
export interface AstAnalyzer {
  /** Snap & hash the enclosing named node around `region`; null if none. */
  analyze(text: string, language: string, region: Region): AstAnalysis | null;
  /** Extract the first matching literal value within `region`; null if none. */
  extractValue(text: string, language: string, region: Region, nodeKind: string): string | null;
}

export interface ResolveOptions {
  ast?: AstAnalyzer;
  /** Current time for ttl evaluation; defaults to Date.now() at call site. */
  now?: number;
}

function bySelectorKind(anchor: Anchor) {
  const out: Partial<Record<Selector["kind"], Selector>> = {};
  for (const s of anchor.selectors) out[s.kind] = s;
  return out;
}

/** Resolve a single Assertion against the current text of its anchored file. */
export function resolveAssertion(
  assertion: Assertion,
  currentText: string,
  opts: ResolveOptions = {},
): Verdict {
  const anchor = assertion.anchor;
  const sel = bySelectorKind(anchor);
  const now = opts.now ?? Date.now();

  const base = {
    assertionId: assertion.id,
    propositionId: assertion.propositionId,
    documentId: assertion.documentId,
    ref: assertion.ref,
  };

  // ttl → expired, before fusion (§17.3).
  const expired = assertion.ttl !== undefined && Date.parse(assertion.ttl) <= now;

  // Coarse-only anchors are navigational and never stale (§11.3).
  const coarseOnly = anchor.selectors.every((s) =>
    (COARSE_SELECTOR_KINDS as readonly string[]).includes(s.kind),
  );
  if (coarseOnly || expired) {
    const g = grade({
      selectors: [],
      expired,
      coarseOnly,
      startDelta: null,
      textQuoteFound: false,
      textQuoteSimilarity: 0,
      valueFound: false,
      valueScore: 0,
    });
    return { ...base, state: g.state, confidence: g.confidence, selectorScores: [], notes: g.notes, advisories: [] };
  }

  const tq = sel["text-quote"]?.kind === "text-quote" ? sel["text-quote"] : undefined;
  const tp = sel["text-position"]?.kind === "text-position" ? sel["text-position"] : undefined;
  const astSel = sel["ast-node"]?.kind === "ast-node" ? sel["ast-node"] : undefined;
  const valSel = sel["value"]?.kind === "value" ? sel["value"] : undefined;

  // ── Localize (text-quote cascade, biased by text-position) ──
  const bias = positionBias(tp);
  let region: Region | null = null;
  if (tq) region = localizeTextQuote(currentText, tq, bias);
  if (!region && tp) region = { start: tp.start, end: tp.end };

  const baselineExact = tq?.exact ?? "";
  const textQuoteFound = tq !== undefined && region !== null;
  const textQuoteSimilarity =
    region !== null && tq ? textSimilarity(regionText(currentText, region), baselineExact) : 0;

  // ── text-position found-check (§17.3): content at baseline offset ≥ 0.6 ──
  let positionFound = false;
  let positionScore = 0;
  if (tp) {
    const atOffset = currentText.slice(tp.start, tp.end);
    positionScore = textSimilarity(atOffset, baselineExact || atOffset);
    positionFound = positionScore >= POSITION_FOUND_SIMILARITY;
  }

  const resolved: ResolvedSelector[] = [];

  if (tq) {
    resolved.push({
      kind: "text-quote",
      found: textQuoteFound,
      score: textQuoteSimilarity,
      weight: WEIGHTS["text-quote"],
    });
  }
  if (tp) {
    resolved.push({
      kind: "text-position",
      found: positionFound,
      score: positionScore,
      weight: WEIGHTS["text-position"],
    });
  }

  // ── Tier-2 structural (ast-node) ──
  let astScore = 0;
  let astFound = false;
  const notes: string[] = [];
  if (astSel && opts.ast && region) {
    const analysis = opts.ast.analyze(currentText, astSel.language, region);
    if (analysis) {
      if (analysis.semanticHash === astSel.semanticHash && analysis.structuralHash === astSel.structuralHash) {
        astScore = 1.0;
      } else if (analysis.structuralHash === astSel.structuralHash) {
        astScore = STRUCTURAL_ONLY_SCORE; // rename/whitespace — keep out of `stale` band
        notes.push("structural-only AST match (rename/whitespace)");
      } else {
        astScore = 0;
      }
    }
    // A positive match is always found; a total mismatch counts as found only
    // if text-position corroborates (the ghost-detection mechanism, §17.3).
    astFound = astScore > 0 ? true : positionFound;
    resolved.push({ kind: "ast-node", found: astFound, score: astScore, weight: WEIGHTS["ast-node"] });
  }

  // ── value tier ──
  let valueScore = 0;
  let valueFound = false;
  if (valSel && opts.ast && region) {
    const extracted = opts.ast.extractValue(currentText, valSel.language, region, valSel.nodeKind);
    if (extracted !== null && collapseWhitespace(extracted) === collapseWhitespace(valSel.value)) {
      valueScore = 1;
    } else {
      valueScore = 0;
    }
    valueFound = valueScore > 0 ? true : positionFound;
    resolved.push({ kind: "value", found: valueFound, score: valueScore, weight: WEIGHTS["value"] });
  }

  const startDelta = region && tp ? Math.abs(region.start - tp.start) : null;

  const g = grade({
    selectors: resolved,
    expired: false,
    coarseOnly: false,
    startDelta,
    textQuoteFound,
    textQuoteSimilarity,
    valueFound,
    valueScore,
  });

  return {
    ...base,
    state: g.state,
    confidence: g.confidence,
    region: region ?? undefined,
    selectorScores: resolved.map((r) => ({ kind: r.kind, found: r.found, score: r.score, weight: r.weight })),
    notes: [...notes, ...g.notes],
    advisories: [],
  };
}
