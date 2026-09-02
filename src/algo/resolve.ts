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
 * both different is "restructured". Freshness is computed from the stored
 * anchor and the current files alone; `check` never reads git.
 */

import { computeGates } from "../core/gating.ts";
import {
  type AnchorState,
  type Assertion,
  type ChangedEvidence,
  COARSE_SELECTOR_KINDS,
  type Region,
  type Selector,
  type SelectorBundle,
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

/** Current content of every file an anchor points into (null = missing). */
export interface ResolveFiles {
  doc: string | null;
  code: ReadonlyMap<string, string | null>;
}

export interface ResolveOptions {
  ast?: AstAnalyzer;
  /** Current time for ttl evaluation; defaults to Date.now(). */
  now?: number;
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
  changedEvidence: ChangedEvidence[];
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
  const changedEvidence: ChangedEvidence[] = [];

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

/** TTL expiry. A datetime with no timezone suffix is read as UTC. */
function parseTtl(
  ttl: string,
  now: number,
): { expired: boolean; invalid: boolean } {
  const normalized =
    ttl.includes("T") && !/(Z|[+-]\d{2}:?\d{2})$/i.test(ttl) ? `${ttl}Z` : ttl;
  const at = Date.parse(normalized);
  if (Number.isNaN(at)) return { expired: true, invalid: true };
  return { expired: at <= now, invalid: false };
}

export interface SidesResult {
  doc: SideResult;
  /** Index-aligned with `assertion.anchor.code`. */
  code: SideResult[];
}

/** Resolve every side once. The doc side resolves first. */
export function resolveSides(
  assertion: Assertion,
  files: ResolveFiles,
  opts: ResolveOptions = {},
): SidesResult {
  const anchor = assertion.anchor;
  const doc = resolveSide(anchor.doc, files.doc, opts);
  const code = anchor.code.map((bundle) =>
    resolveSide(bundle, files.code.get(bundle.file) ?? null, opts),
  );
  return { doc, code };
}

/** Worst state over the code-side bundles. */
export function worstCodeState(code: SideResult[]): AnchorState {
  let state: AnchorState = "unchanged";
  for (const side of code) {
    if (STATE_RANK[side.state] > STATE_RANK[state]) state = side.state;
  }
  return state;
}

/** Resolve a single assertion against the current working tree. */
export function resolveAssertion(
  assertion: Assertion,
  files: ResolveFiles,
  opts: ResolveOptions = {},
): Verdict {
  const now = opts.now ?? Date.now();
  const sides = resolveSides(assertion, files, opts);
  const code = worstCodeState(sides.code);
  const primaryCode = sides.code.find((s) => s.state === code) ?? sides.code[0];
  const primary = primaryCode ?? sides.doc;

  const ttl =
    assertion.ttl !== undefined ? parseTtl(assertion.ttl, now) : undefined;
  const expired = ttl?.expired ?? false;

  const gates = computeGates(
    { doc: sides.doc.state, code, expired },
    assertion.enforcement,
  );

  const notes = [
    ...sides.doc.notes.map((n) => `doc: ${n}`),
    ...sides.code.flatMap((s) => s.notes.map((n) => `code: ${n}`)),
    ttl?.invalid
      ? `unparseable ttl "${assertion.ttl}": treated as expired (fix or clear the ttl)`
      : "",
  ].filter(Boolean);

  const changedEvidence = [
    ...sides.doc.changedEvidence,
    ...sides.code.flatMap((s) => s.changedEvidence),
  ];

  return {
    assertionId: assertion.id,
    propositionId: assertion.propositionId,
    documentId: assertion.documentId,
    doc: sides.doc.state,
    code,
    behavior: undefined,
    expired,
    gates,
    remediation: remediationFor({
      assertionId: assertion.id,
      doc: sides.doc.state,
      code,
      expired,
      changedEvidence,
    }),
    evidence: {
      docRegion: sides.doc.region ?? undefined,
      codeRegions: sides.code
        .map((s) => s.region)
        .filter((r): r is Region => r !== null),
      similarity: primary.similarity,
      changedEvidence,
    },
    notes,
    advisories: [],
  };
}
