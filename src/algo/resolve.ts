/**
 * Anchor resolution: an ordered cascade per side, no weighted fusion.
 *
 *   1. Locate the stored quote (exact occurrences ranked by prefix/suffix
 *      context, else a fuzzy match within the error budget).
 *   2. Same normalized text at the same offset: `unchanged`.
 *   3. Same text at a new offset: `moved`.
 *   4. Text below the similarity floor, semantic AST hash changed, or the
 *      in-span literal changed: `changed`, with a reason label.
 *   5. Not found: `orphaned`.
 *   6. Several equally good exact matches: `ambiguous`.
 *
 * Position is a tiebreaker only. Both AST hashes are kept as reason labels:
 * structural equal and semantic different is "identifiers or literals renamed";
 * both different is "restructured". Resolution is computed from the stored
 * selectors and the current files alone; `check` never reads git.
 */

import { computeGates } from "../core/gating.ts";
import {
  type AnchorState,
  type Claim,
  COARSE_SELECTOR_KINDS,
  type Region,
  type Selector,
  type SelectorBundle,
  type SpanChange,
  type Verdict,
} from "../core/model.ts";
import { remediationFor } from "../core/remediation.ts";
import { localizeTextQuote, regionText } from "./localize.ts";
import { collapseWhitespace, textSimilarity } from "./normalize.ts";
import { MOVE_AWARENESS_CHARS, SAME_TEXT_SIMILARITY } from "./params.ts";

/** Tree-sitter analyzer hook (check time). */
export interface AstAnalysis {
  nodeType: string;
  structuralHash: string;
  semanticHash: string;
  /** The snapped node's region in the current text. */
  region: Region;
}
export interface AstAnalyzer {
  /** Snap and hash the enclosing named node around `region`; null if unparseable. */
  analyze(text: string, language: string, region: Region): AstAnalysis | null;
  /** The first literal inside `region`; null if none. */
  extractValue(
    text: string,
    language: string,
    region: Region,
    nodeKind?: string,
  ): string | null;
}

/** Current content of every file a claim's spans point into (null = missing). */
export interface ResolveFiles {
  doc: string | null;
  evidence: ReadonlyMap<string, string | null>;
}

export interface ResolveOptions {
  ast?: AstAnalyzer;
}

const STATE_RANK: Record<AnchorState, number> = {
  orphaned: 4,
  ambiguous: 3,
  changed: 2,
  moved: 1,
  unchanged: 0,
};

export interface SideResult {
  state: AnchorState;
  region: Region | null;
  /** Normalized similarity of the located text to the stored quote (0 when not found). */
  similarity: number;
  notes: string[];
  changedEvidence: SpanChange[];
  /** The live text at the located region. */
  liveText: string | null;
}

function bySelectorKind(selectors: Selector[]) {
  const out: Partial<Record<Selector["kind"], Selector>> = {};
  for (const s of selectors) out[s.kind] = s;
  return out;
}

/** Numeric tokens of a prose span, so a `5` to `7` edit in a sentence is caught. */
function numbers(s: string): string {
  return (s.match(/\d+(?:\.\d+)?/g) ?? []).join(",");
}

/** Resolve one anchor side against its current file text. */
export function resolveSide(
  bundle: SelectorBundle,
  currentText: string | null,
  opts: ResolveOptions = {},
): SideResult {
  if (currentText === null) {
    return {
      state: "orphaned",
      region: null,
      similarity: 0,
      notes: [`file not found: ${bundle.file}`],
      changedEvidence: [
        { path: bundle.file, kind: "text", detail: "file missing" },
      ],
      liveText: null,
    };
  }

  const coarseOnly = bundle.selectors.every((s) =>
    (COARSE_SELECTOR_KINDS as readonly string[]).includes(s.kind),
  );
  if (coarseOnly) {
    return {
      state: "unchanged",
      region: null,
      similarity: 1,
      notes: ["coarse anchor: navigational, never drift"],
      changedEvidence: [],
      liveText: null,
    };
  }

  const sel = bySelectorKind(bundle.selectors);
  const tq =
    sel["text-quote"]?.kind === "text-quote" ? sel["text-quote"] : undefined;
  const tp =
    sel["text-position"]?.kind === "text-position"
      ? sel["text-position"]
      : undefined;
  const astSel =
    sel["ast-node"]?.kind === "ast-node" ? sel["ast-node"] : undefined;
  const valSel = sel.value?.kind === "value" ? sel.value : undefined;

  if (!tq) {
    return {
      state: "orphaned",
      region: null,
      similarity: 0,
      notes: ["no text-quote selector"],
      changedEvidence: [],
      liveText: null,
    };
  }

  // 1. Locate.
  const located = localizeTextQuote(currentText, tq, tp);
  if (!located.region) {
    return {
      state: "orphaned",
      region: null,
      similarity: 0,
      notes: ["quote not found"],
      changedEvidence: [
        { path: bundle.file, kind: "text", detail: "documented span orphaned" },
      ],
      liveText: null,
    };
  }
  const region = located.region;
  const liveText = regionText(currentText, region);
  const similarity = textSimilarity(liveText, tq.exact);
  const notes: string[] = [];
  const changedEvidence: SpanChange[] = [];

  // 6. Several equally good matches.
  if (located.ambiguous) {
    notes.push("quote matched in several places equally well");
    return {
      state: "ambiguous",
      region,
      similarity,
      notes,
      changedEvidence,
      liveText,
    };
  }

  // AST reason label (code side, analyzer present).
  let astLabel: string | undefined;
  if (astSel && opts.ast) {
    const analysis = opts.ast.analyze(currentText, astSel.language, region);
    if (analysis) {
      if (analysis.semanticHash !== astSel.semanticHash) {
        astLabel =
          analysis.structuralHash === astSel.structuralHash
            ? "identifiers or literals renamed"
            : "restructured";
      }
    }
  }

  // In-span literal check (code side, analyzer present).
  let valueLabel: string | undefined;
  if (valSel && opts.ast) {
    const extracted = opts.ast.extractValue(
      currentText,
      valSel.language,
      region,
      valSel.nodeKind,
    );
    if (
      extracted === null ||
      collapseWhitespace(extracted) !== collapseWhitespace(valSel.value)
    ) {
      valueLabel = `value changed (was \`${valSel.value}\`)`;
    }
  }

  // Prose: a numeric token changed inside the span.
  let numberLabel: string | undefined;
  if (!astSel && !valSel && numbers(liveText) !== numbers(tq.exact)) {
    numberLabel = "a number in the sentence changed";
  }

  // 4. Changed.
  const sameText = similarity >= SAME_TEXT_SIMILARITY;
  if (!sameText || astLabel || valueLabel || numberLabel) {
    if (valueLabel) {
      changedEvidence.push({
        path: bundle.file,
        kind: "value",
        detail: valueLabel,
      });
    }
    if (astLabel) {
      changedEvidence.push({
        path: bundle.file,
        kind: "ast",
        detail: astLabel,
      });
    }
    if (!valueLabel && !astLabel) {
      changedEvidence.push({
        path: bundle.file,
        kind: "text",
        detail:
          numberLabel ??
          `text changed (${Math.round(similarity * 100)}% similar)`,
      });
    }
    for (const c of changedEvidence) if (c.detail) notes.push(c.detail);
    return {
      state: "changed",
      region,
      similarity,
      notes,
      changedEvidence,
      liveText,
    };
  }

  // 2 and 3. Unchanged or moved.
  const delta = tp ? Math.abs(region.start - tp.start) : 0;
  if (delta > MOVE_AWARENESS_CHARS) {
    notes.push(`span moved ${delta} chars`);
    return {
      state: "moved",
      region,
      similarity,
      notes,
      changedEvidence,
      liveText,
    };
  }
  return {
    state: "unchanged",
    region,
    similarity,
    notes,
    changedEvidence,
    liveText,
  };
}

export interface SidesResult {
  doc: SideResult;
  /** Index-aligned with `claim.evidence`. */
  evidence: SideResult[];
}

/** Resolve every span once. The doc span resolves first. */
export function resolveSides(
  claim: Pick<Claim, "doc" | "evidence">,
  files: ResolveFiles,
  opts: ResolveOptions = {},
): SidesResult {
  const doc = resolveSide(claim.doc, files.doc, opts);
  const evidence = claim.evidence.map((bundle) =>
    resolveSide(bundle, files.evidence.get(bundle.file) ?? null, opts),
  );
  return { doc, evidence };
}

/** Worst state over the evidence spans. */
export function worstState(sides: SideResult[]): AnchorState {
  let state: AnchorState = "unchanged";
  for (const side of sides) {
    if (STATE_RANK[side.state] > STATE_RANK[state]) state = side.state;
  }
  return state;
}

/**
 * Resolve a single claim against the current working tree. The `evidence`
 * axis is absent when the claim has no evidence span. Expiry and the judge
 * axis are not computed here.
 */
export function resolveClaim(
  claim: Claim,
  files: ResolveFiles,
  opts: ResolveOptions = {},
): Verdict {
  const sides = resolveSides(claim, files, opts);
  const evidence =
    sides.evidence.length > 0 ? worstState(sides.evidence) : undefined;
  // The reported similarity describes the side that decided the verdict, so a
  // `doc:changed` claim never reports an untouched evidence side's 1.0.
  const primary = [sides.doc, ...sides.evidence].reduce((worst, side) =>
    STATE_RANK[side.state] > STATE_RANK[worst.state] ? side : worst,
  );

  const expired = false;
  const gates = computeGates(
    { doc: sides.doc.state, evidence, expired },
    claim.enforcement,
  );

  const notes = [
    ...sides.doc.notes.map((n) => `doc: ${n}`),
    ...sides.evidence.flatMap((s) => s.notes.map((n) => `evidence: ${n}`)),
  ];

  const changes = [
    ...sides.doc.changedEvidence,
    ...sides.evidence.flatMap((s) => s.changedEvidence),
  ];

  return {
    claimId: claim.id,
    documentId: claim.documentId,
    kind: claim.kind,
    doc: sides.doc.state,
    ...(evidence !== undefined ? { evidence } : {}),
    expired,
    conflicts: [],
    gates,
    remediation: remediationFor({
      claimId: claim.id,
      doc: sides.doc.state,
      evidence,
      expired,
    }),
    detail: {
      docRegion: sides.doc.region ?? undefined,
      evidenceRegions: sides.evidence
        .map((s) => s.region)
        .filter((r): r is Region => r !== null),
      similarity: primary.similarity,
      changes,
    },
    notes,
  };
}
