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

import { type GateMode, gateVerdict } from "../core/gating.ts";
import {
  type AnchorState,
  type Claim,
  type ConflictFinding,
  DEFAULT_JUDGE_THRESHOLD,
  type EvidenceState,
  type JudgeState,
  type Region,
  type Selector,
  type SelectorBundle,
  type SpanChange,
  type Verdict,
} from "../core/model.ts";
import { remediationFor } from "../core/remediation.ts";
import { expiryLabel } from "../engine/expiry.ts";
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
  /**
   * Evidence files in a sibling repo that is not checked out (SPEC 4.1), with
   * the reason label. Those sides resolve `unavailable`.
   */
  unavailable?: ReadonlyMap<string, string>;
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
  reasons: string[];
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

/**
 * The located text widened to whole numbers at its edges, so a quote `= 5`
 * that now matches the start of `= 50` reads `= 50`.
 */
function withWholeNumbers(text: string, region: Region): string {
  const digit = (i: number) => /\d/.test(text[i] ?? "");
  let start = region.start;
  let end = region.end;
  if (digit(start)) while (digit(start - 1)) start--;
  if (digit(end - 1)) {
    for (;;) {
      if (digit(end)) end++;
      else if (text[end] === "." && digit(end + 1)) end += 2;
      else break;
    }
  }
  return text.slice(start, end);
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
      reasons: [`file not found: ${bundle.file}`],
      changedEvidence: [
        { path: bundle.file, kind: "text", detail: "file missing" },
      ],
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
      reasons: ["no text-quote selector"],
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
      reasons: ["quote not found"],
      changedEvidence: [
        { path: bundle.file, kind: "text", detail: "documented span orphaned" },
      ],
      liveText: null,
    };
  }
  const region = located.region;
  const liveText = regionText(currentText, region);
  const similarity = textSimilarity(liveText, tq.exact);
  const reasons: string[] = [];
  const changedEvidence: SpanChange[] = [];

  // 6. Several equally good matches.
  if (located.ambiguous) {
    reasons.push("quote matched in several places equally well");
    return {
      state: "ambiguous",
      region,
      similarity,
      reasons,
      changedEvidence,
      liveText,
    };
  }

  // AST and value checks run only when the analyzer can parse the span: the
  // analyzer is on and a grammar for the language is loaded.
  const language = astSel?.language ?? valSel?.language;
  const analysis =
    language !== undefined && opts.ast
      ? opts.ast.analyze(currentText, language, region)
      : null;
  const structural = analysis !== null;

  // AST reason label (code side).
  let astLabel: string | undefined;
  if (astSel && analysis && analysis.semanticHash !== astSel.semanticHash) {
    astLabel =
      analysis.structuralHash === astSel.structuralHash
        ? "identifiers or literals renamed"
        : "restructured";
  }

  // In-span literal check (code side).
  let valueLabel: string | undefined;
  if (valSel && structural && opts.ast) {
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

  // Prose, or code without structural checks (--no-ast, or no grammar for
  // the language): a numeric token changed inside the span.
  let numberLabel: string | undefined;
  if (
    !structural &&
    numbers(withWholeNumbers(currentText, region)) !== numbers(tq.exact)
  ) {
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
    for (const c of changedEvidence) if (c.detail) reasons.push(c.detail);
    return {
      state: "changed",
      region,
      similarity,
      reasons,
      changedEvidence,
      liveText,
    };
  }

  // 2 and 3. Unchanged or moved.
  const delta = tp ? Math.abs(region.start - tp.start) : 0;
  if (delta > MOVE_AWARENESS_CHARS) {
    reasons.push(`span moved ${delta} chars`);
    return {
      state: "moved",
      region,
      similarity,
      reasons,
      changedEvidence,
      liveText,
    };
  }
  return {
    state: "unchanged",
    region,
    similarity,
    reasons,
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

// ── Claim level ──────────────────────────────────────────────────────────────

/** An evidence side: an anchor result, or `unavailable` when its repo is not checked out. */
export type EvidenceSide =
  | SideResult
  | (Omit<SideResult, "state"> & { state: "unavailable" });

export interface ClaimSides {
  doc: SideResult;
  /** Index-aligned with `claim.evidence`. */
  evidence: EvidenceSide[];
}

/** `unavailable` ranks above `moved` and below the gating states. */
const EVIDENCE_RANK: Record<EvidenceState, number> = {
  orphaned: 5,
  ambiguous: 4,
  changed: 3,
  unavailable: 2,
  moved: 1,
  unchanged: 0,
};

/** Resolve the doc span and every evidence span, marking sibling-repo sides `unavailable`. */
export function resolveClaimSides(
  claim: Pick<Claim, "doc" | "evidence">,
  files: ResolveFiles,
  opts: ResolveOptions = {},
): ClaimSides {
  const doc = resolveSide(claim.doc, files.doc, opts);
  const evidence = claim.evidence.map((bundle): EvidenceSide => {
    const label = files.unavailable?.get(bundle.file);
    if (label !== undefined) {
      return {
        state: "unavailable",
        region: null,
        similarity: 0,
        reasons: [label],
        changedEvidence: [],
        liveText: null,
      };
    }
    return resolveSide(bundle, files.evidence.get(bundle.file) ?? null, opts);
  });
  return { doc, evidence };
}

/** Worst state over evidence sides that may include `unavailable`. */
export function worstEvidenceState(sides: EvidenceSide[]): EvidenceState {
  let state: EvidenceState = "unchanged";
  for (const side of sides) {
    if (EVIDENCE_RANK[side.state] > EVIDENCE_RANK[state]) state = side.state;
  }
  return state;
}

/** What later passes found about a claim: the judge axis and the contradiction pass. */
export interface ClaimFindings {
  judge?: JudgeState;
  conflicts?: ConflictFinding[];
  /** Warn-only findings, each a reason label (a conflict in review, a pending pair). */
  warnings?: string[];
  /** Reason labels from those passes (judge outcome, why the judge was not asked). */
  reasons?: string[];
  /** A `review` verdict an `ack` attestation names; it stays `review` and no longer warns. */
  reviewAcknowledged?: boolean;
}

export interface ResolveClaimOptions extends ResolveOptions, ClaimFindings {
  /** Clock for expiry. Default: now. */
  now?: Date;
  /** `doc` for `check --doc`, where `expired` gates. Default `store`. */
  mode?: GateMode;
  /** `config.judge.threshold`. */
  threshold?: number;
}

/** The per-claim result of `check`. */
export type ClaimVerdict = Verdict;

/** Build the verdict for resolved sides plus the later passes' findings. */
export function assembleVerdict(
  claim: Claim,
  sides: ClaimSides,
  opts: ResolveClaimOptions = {},
): ClaimVerdict {
  const evidence =
    sides.evidence.length > 0 ? worstEvidenceState(sides.evidence) : undefined;
  // The reported similarity describes the located side that decided the
  // verdict, so a `doc:changed` claim never reports an untouched evidence
  // side's 1.0, and an unavailable side reports nothing.
  const located = [
    sides.doc,
    ...sides.evidence.filter((s): s is SideResult => s.state !== "unavailable"),
  ];
  const primary = located.reduce((worst, side) =>
    STATE_RANK[side.state] > STATE_RANK[worst.state] ? side : worst,
  );

  const now = opts.now ?? new Date();
  const expiry = expiryLabel(claim, now);
  const expired = expiry !== undefined;
  const conflicts = opts.conflicts ?? [];
  const gatingJudge =
    opts.judge === "review" && opts.reviewAcknowledged ? undefined : opts.judge;
  const gate = gateVerdict(
    {
      doc: sides.doc.state,
      evidence,
      judge: gatingJudge,
      expired,
      conflicts,
      warnings: opts.warnings,
    },
    claim.enforcement,
    { mode: opts.mode, threshold: opts.threshold ?? DEFAULT_JUDGE_THRESHOLD },
  );

  const reasons = [
    ...sides.doc.reasons.map((n) => `doc: ${n}`),
    ...sides.evidence.flatMap((s) => s.reasons.map((n) => `evidence: ${n}`)),
    ...(expiry !== undefined ? [expiry] : []),
    ...(opts.reasons ?? []),
    ...gate.reasons,
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
    ...(opts.judge !== undefined ? { judge: opts.judge } : {}),
    expired,
    conflicts,
    gates: gate.gates,
    warns: gate.warns,
    remediation: remediationFor({
      claimId: claim.id,
      kind: claim.kind,
      doc: sides.doc.state,
      evidence,
      judge: gatingJudge,
      expired,
      conflicts,
    }),
    detail: {
      docRegion: sides.doc.region ?? undefined,
      evidenceRegions: sides.evidence
        .map((s) => s.region)
        .filter((r): r is Region => r !== null),
      similarity: primary.similarity,
      changes,
    },
    reasons,
  };
}

/**
 * Resolve a single claim against the current working tree: anchors, expiry
 * (SPEC 5 counting rule), and gating with whatever judge and conflict
 * findings the caller passes. The `evidence` axis is absent when the claim
 * has no evidence span; `judge` is absent unless passed.
 */
export function resolveClaim(
  claim: Claim,
  files: ResolveFiles,
  opts: ResolveClaimOptions = {},
): ClaimVerdict {
  return assembleVerdict(claim, resolveClaimSides(claim, files, opts), opts);
}
