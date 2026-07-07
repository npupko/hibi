/**
 * Drift resolution (§6 layered cheapest-first with corroboration; §17).
 *
 * Resolves an Assertion's **bidirectional** Anchor against the *current* working
 * tree, producing the two-axis Verdict. Freshness is computed from (stored
 * Anchor) vs (current files) alone — the engine never reads a historical
 * revision, so `check` stays offline-correct (§6).
 *
 * Order is load-bearing (§6 step 0 / §18-B): the **doc side resolves first**.
 * The documented sentence is the source of truth; a `doc:orphaned`/`doc:changed`
 * result must not let the stale `textCache` be verified as live truth, and a
 * verifier must never certify a claim whose sentence is in flux. Behavioral
 * belief here is the **deterministic baseline** (`unverified`/`at-risk`); a
 * runner resolver upgrades it to `supported`/`refuted` out-of-process (§17.6).
 */

import { hashContent } from "../ast/hash.ts";
import { computeGates } from "../core/gating.ts";
import {
  type AnchorState,
  type Assertion,
  type BehaviorState,
  type ChangedEvidence,
  COARSE_SELECTOR_KINDS,
  type Region,
  type Selector,
  type SelectorBundle,
  type SelectorScore,
  type Verdict,
} from "../core/model.ts";
import { remediationFor } from "../core/remediation.ts";
import { isBehavioral } from "./behavioral.ts";
import { grade, type ResolvedSelector } from "./fusion.ts";
import { localizeTextQuote, positionBias, regionText } from "./localize.ts";
import { collapseWhitespace, textSimilarity } from "./normalize.ts";
import {
  AMBIGUOUS_MIN_QUOTE_LENGTH,
  POSITION_FOUND_SIMILARITY,
  STRONG_TEXTQUOTE_SIMILARITY,
  STRUCTURAL_ONLY_SCORE,
  WEIGHTS,
} from "./params.ts";

/**
 * Tier-2 analyzer hook — implemented with tree-sitter. Given the current text
 * and the localized region, snaps the enclosing named node and reports its
 * two-tier hash and any extracted literal value.
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
  extractValue(
    text: string,
    language: string,
    region: Region,
    nodeKind: string,
  ): string | null;
  /** List the import specifiers of a file (§17.6, D14); [] if unparseable. */
  extractImports(text: string, language: string): string[];
}

/** Current content of every file an anchor points into (null = file missing). */
export interface ResolveFiles {
  /** The doc-side file's current content. */
  doc: string | null;
  /** Each code-side bundle file's current content, keyed by path. */
  code: ReadonlyMap<string, string | null>;
}

export interface ResolveOptions {
  ast?: AstAnalyzer;
  /** Current time for ttl evaluation; defaults to Date.now() at call site. */
  now?: number;
  /**
   * The change-gate evidence (§17.6, D14): current contents of every
   * evidence-set path, keyed by path (null = file missing). Supplied by the
   * engine shell (`check.ts`); absent → the gate falls back to the
   * anchored-node signal only.
   */
  evidence?: ReadonlyMap<string, string | null>;
}

/**
 * The behavioral change-gate (§17.6, D14). `docSide.state` is consulted NOWHERE
 * here — a doc-side edit is Axis 1's job, so the two axes never double-fire on
 * one signal. Fires `at-risk` when the anchored node's semantics changed (an
 * `ast`/`value` change on the code side) or any evidence path drifted from its
 * baseline (a changed hash, or a newly-added import with no baseline entry).
 *
 * Baseline handling is precision-critical (§11.3):
 *   - baseline **exists** but an evidence path is missing from it → that path is
 *     changed evidence (a newly added import);
 *   - baseline **absent entirely** (recorded `--no-ast`, analyzer failure) →
 *     fall back to the anchored-node signal, never flag every path.
 */
function computeBehaviorRisk(
  assertion: Assertion,
  codeChanged: ChangedEvidence[],
  evidence: ReadonlyMap<string, string | null> | undefined,
): { state: BehaviorState; changed: ChangedEvidence[] } {
  const changed: ChangedEvidence[] = [];

  // (1) Anchored-node signal: a semantic (ast) or literal (value) change of the
  //     anchored span — a rename/whitespace-only edit is structural-only and
  //     surfaces neither, so it correctly never fires.
  for (const c of codeChanged) {
    if (c.kind === "ast" || c.kind === "value") changed.push(c);
  }

  // (2) Evidence-set drift — only when a baseline exists (baseline-absent → (1)).
  const baseline = assertion.evidenceBaseline;
  if (baseline && evidence) {
    for (const [path, content] of evidence) {
      const cur = content === null ? null : hashContent(content);
      const base = baseline[path];
      if (base === undefined) {
        changed.push({ path, kind: "import", detail: "new evidence path" });
      } else if (cur !== base) {
        changed.push({ path, kind: "import", detail: "evidence file changed" });
      }
    }
  }

  // Dedupe by (path, kind) so one file never contributes two identical entries.
  const seen = new Set<string>();
  const uniq = changed.filter((c) => {
    const key = `${c.path}|${c.kind}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });

  return uniq.length > 0
    ? { state: "at-risk", changed: uniq }
    : { state: "unverified", changed: [] };
}

/**
 * Whether an authored `hibi ignore` suppression is still active (§17.6, D14).
 * Active iff every acknowledged path still hashes to its acknowledged value AND
 * no currently-changed evidence path lies outside the acknowledged set (a new
 * path appearing lapses it). Any mismatch → lapsed, and the at-risk resurfaces.
 */
function suppressionActive(
  suppressed: Assertion["suppressed"],
  evidence: ReadonlyMap<string, string | null> | undefined,
  behaviorChanged: ChangedEvidence[],
): boolean {
  if (!suppressed) return false;
  for (const [path, ackHash] of Object.entries(suppressed.paths)) {
    const content = evidence?.get(path);
    const cur = content == null ? null : hashContent(content);
    if (cur !== ackHash) return false; // an acknowledged path moved → lapsed
  }
  for (const c of behaviorChanged) {
    if (!(c.path in suppressed.paths)) return false; // a new path → lapsed
  }
  return true;
}

/** Worst-wins precedence when aggregating code-side bundle states. */
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
  confidence: number;
  selectorScores: SelectorScore[];
  notes: string[];
  changedEvidence: ChangedEvidence[];
  /** The live text at the located region (the authoritative span when found). */
  liveText: string | null;
}

/** Dedupe changed-evidence entries by (path, kind, detail). */
function dedupeEvidence(entries: ChangedEvidence[]): ChangedEvidence[] {
  const seen = new Set<string>();
  return entries.filter((c) => {
    const key = `${c.path}|${c.kind}|${c.detail ?? ""}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function bySelectorKind(selectors: Selector[]) {
  const out: Partial<Record<Selector["kind"], Selector>> = {};
  for (const s of selectors) out[s.kind] = s;
  return out;
}

function countOccurrences(haystack: string, needle: string): number {
  if (!needle) return 0;
  let n = 0;
  let i = haystack.indexOf(needle);
  while (i !== -1) {
    n++;
    i = haystack.indexOf(needle, i + needle.length);
  }
  return n;
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Locate an owned-doc `<!-- hibi:claim id=… -->` marker; -1 if absent. */
function locateInlineId(text: string, id: string): number {
  const re = new RegExp(`hibi:claim\\s+id=["']?${escapeRegExp(id)}["']?`);
  const m = re.exec(text);
  return m ? m.index : -1;
}

/**
 * Detect a unique-but-multiply-matched quote (§17.1) — `ambiguous`. Guarded by a
 * minimum quote length, and disambiguated by prefix context so a genuinely
 * unique anchor with a common substring is not falsely flagged.
 */
function isAmbiguous(
  text: string,
  tq: Extract<Selector, { kind: "text-quote" }>,
): boolean {
  if (tq.exact.length < AMBIGUOUS_MIN_QUOTE_LENGTH) return false;
  if (countOccurrences(text, tq.exact) < 2) return false;
  // Disambiguate with both prefix and suffix context — a span at offset 0 (or
  // any anchor with no captured prefix) still has suffix context to fall back on,
  // so a genuinely-unique quote with a common substring is not falsely flagged.
  const ctx = tq.prefix.slice(-16) + tq.exact + tq.suffix.slice(0, 16);
  if (ctx.length > tq.exact.length && countOccurrences(text, ctx) < 2) {
    return false;
  }
  return true;
}

/**
 * Resolve a single anchor side (doc or code) against its current file text.
 * Exported so `reanchor` can re-localize each bundle independently and in order
 * (the aggregated `evidence.codeRegions` drops null regions, so it is not
 * index-aligned with `anchor.code`).
 */
export function resolveSide(
  bundle: SelectorBundle,
  currentText: string | null,
  opts: ResolveOptions = {},
): SideResult {
  // File missing → the span is unresolvable (§17.1).
  if (currentText === null) {
    return {
      state: "orphaned",
      region: null,
      confidence: 0,
      selectorScores: [],
      notes: [`file not found: ${bundle.file}`],
      changedEvidence: [
        { path: bundle.file, kind: "text", detail: "file missing" },
      ],
      liveText: null,
    };
  }

  const sel = bySelectorKind(bundle.selectors);

  // Coarse-only bundles are navigational and never reported as drift (§11.3).
  const coarseOnly = bundle.selectors.every((s) =>
    (COARSE_SELECTOR_KINDS as readonly string[]).includes(s.kind),
  );
  if (coarseOnly) {
    const g = grade({
      selectors: [],
      coarseOnly: true,
      ambiguous: false,
      startDelta: null,
      textQuoteFound: false,
      textQuoteSimilarity: 0,
      soleStrongQuote: false,
      valueFound: false,
      valueScore: 0,
    });
    return {
      state: g.state,
      region: null,
      confidence: g.confidence,
      selectorScores: [],
      notes: g.notes,
      changedEvidence: [],
      liveText: null,
    };
  }

  const tq =
    sel["text-quote"]?.kind === "text-quote" ? sel["text-quote"] : undefined;
  const tp =
    sel["text-position"]?.kind === "text-position"
      ? sel["text-position"]
      : undefined;
  const astSel =
    sel["ast-node"]?.kind === "ast-node" ? sel["ast-node"] : undefined;
  const valSel = sel.value?.kind === "value" ? sel.value : undefined;
  const inlineSel =
    sel["inline-id"]?.kind === "inline-id" ? sel["inline-id"] : undefined;

  // An owned-doc inline marker, when present, biases localization and resolves
  // ambiguity — but never restates the claim, so prose still decides the state.
  const inlineAt = inlineSel ? locateInlineId(currentText, inlineSel.id) : -1;
  const inlineFound = inlineAt >= 0;

  // ── Localize (text-quote cascade, biased by inline marker or text-position) ──
  const bias = inlineFound ? inlineAt : positionBias(tp);
  let region: Region | null = null;
  if (tq) region = localizeTextQuote(currentText, tq, bias);
  if (!region && tp) region = { start: tp.start, end: tp.end };

  const baselineExact = tq?.exact ?? "";
  const textQuoteFound = tq !== undefined && region !== null;
  const textQuoteSimilarity =
    region !== null && tq
      ? textSimilarity(regionText(currentText, region), baselineExact)
      : 0;
  const ambiguous = tq && !inlineFound ? isAmbiguous(currentText, tq) : false;

  // ── text-position found-check (§17.3): content at baseline offset ≥ 0.6 ──
  let positionFound = false;
  let positionScore = 0;
  if (tp) {
    const atOffset = currentText.slice(tp.start, tp.end);
    positionScore = textSimilarity(atOffset, baselineExact || atOffset);
    positionFound = positionScore >= POSITION_FOUND_SIMILARITY;
  }

  const resolved: ResolvedSelector[] = [];
  const changedEvidence: ChangedEvidence[] = [];
  const notes: string[] = [];

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
  if (astSel && opts.ast && region) {
    let astScore = 0;
    const analysis = opts.ast.analyze(currentText, astSel.language, region);
    if (analysis) {
      if (
        analysis.semanticHash === astSel.semanticHash &&
        analysis.structuralHash === astSel.structuralHash
      ) {
        astScore = 1.0;
      } else if (analysis.structuralHash === astSel.structuralHash) {
        astScore = STRUCTURAL_ONLY_SCORE; // rename/whitespace — keep out of `changed`
        notes.push("structural-only AST match (rename/whitespace)");
      } else {
        astScore = 0;
        changedEvidence.push({
          path: bundle.file,
          kind: "ast",
          detail: "enclosing node structure changed",
        });
      }
    }
    // A positive match is always found; a total mismatch counts as found only
    // if text-position corroborates (the orphan-detection mechanism, §17.3).
    const astFound = astScore > 0 ? true : positionFound;
    resolved.push({
      kind: "ast-node",
      found: astFound,
      score: astScore,
      weight: WEIGHTS["ast-node"],
    });
  }

  // ── value tier ──
  let valueScore = 0;
  let valueFound = false;
  if (valSel && opts.ast && region) {
    const extracted = opts.ast.extractValue(
      currentText,
      valSel.language,
      region,
      valSel.nodeKind,
    );
    if (
      extracted !== null &&
      collapseWhitespace(extracted) === collapseWhitespace(valSel.value)
    ) {
      valueScore = 1;
    } else {
      valueScore = 0;
      changedEvidence.push({
        path: bundle.file,
        kind: "value",
        detail: `anchored value changed (was \`${valSel.value}\`)`,
      });
    }
    valueFound = valueScore > 0 ? true : positionFound;
    resolved.push({
      kind: "value",
      found: valueFound,
      score: valueScore,
      weight: WEIGHTS.value,
    });
  }

  const startDelta = region && tp ? Math.abs(region.start - tp.start) : null;

  // A near-exact relocation that is the *only* resolving selector still vouches
  // for the span (the moved-prose case — §17.3) — keeps it `moved`, not `orphaned`.
  const foundCount = resolved.filter((r) => r.found).length;
  const soleStrongQuote =
    foundCount < 2 &&
    textQuoteFound &&
    textQuoteSimilarity >= STRONG_TEXTQUOTE_SIMILARITY;

  const g = grade({
    selectors: resolved,
    coarseOnly: false,
    ambiguous,
    startDelta,
    textQuoteFound,
    textQuoteSimilarity,
    soleStrongQuote,
    valueFound,
    valueScore,
  });

  // A `text`-level changed/orphaned verdict on the quote is itself evidence.
  if ((g.state === "changed" || g.state === "orphaned") && tq) {
    changedEvidence.push({
      path: bundle.file,
      kind: "text",
      detail: `documented span ${g.state}`,
    });
  }

  return {
    state: g.state,
    region,
    confidence: g.confidence,
    selectorScores: resolved.map((r) => ({
      kind: r.kind,
      found: r.found,
      score: r.score,
      weight: r.weight,
    })),
    notes: [...notes, ...g.notes],
    changedEvidence,
    liveText: region ? regionText(currentText, region) : null,
  };
}

/** Resolve a single Assertion against the current working tree (two-axis). */
export function resolveAssertion(
  assertion: Assertion,
  files: ResolveFiles,
  opts: ResolveOptions = {},
): Verdict {
  const now = opts.now ?? Date.now();
  const anchor = assertion.anchor;

  // ── Step 0 — resolve the doc side first (§6 / §18-B) ──
  const docSide = resolveSide(anchor.doc, files.doc, opts);

  // ── Resolve each code-side bundle; aggregate worst-wins ──
  const codeSides = anchor.code.map((bundle) =>
    resolveSide(bundle, files.code.get(bundle.file) ?? null, opts),
  );
  let code: AnchorState = "unchanged";
  let primaryCode: SideResult | undefined;
  for (const side of codeSides) {
    if (!primaryCode || STATE_RANK[side.state] > STATE_RANK[code]) {
      code = side.state;
      primaryCode = side;
    }
  }
  const codeRegions = codeSides
    .map((s) => s.region)
    .filter((r): r is Region => r !== null);
  const codeChanged = codeSides.flatMap((s) => s.changedEvidence);
  const codeNotes = codeSides.flatMap((s) => s.notes);

  // ── Behavioral risk routing (deterministic baseline — §17.6) ──
  // The live documented span is authoritative for classification. A claim that
  // links executable verifiers is behavioral by construction, even if neither an
  // author `behavioral` flag nor the keyword heuristic classifies it — so the
  // verifier-dispatch path and this inline path agree on the behavior axis (§10).
  const behavioral = isBehavioral(
    assertion.behavioral,
    docSide.liveText,
    assertion.verifiers.length > 0,
  );
  let behavior: BehaviorState | undefined;
  let behaviorChanged: ChangedEvidence[] = [];
  let suppressed = false;
  if (behavioral) {
    // Change-gate v2 (§17.6, D14): the anchored-node signal + evidence-set drift.
    // A linked verifier may later upgrade this to supported/refuted (§17.6).
    const risk = computeBehaviorRisk(assertion, codeChanged, opts.evidence);
    behavior = risk.state;
    behaviorChanged = risk.changed;
    // An authored `hibi ignore` neutralizes an active at-risk (D14): still
    // surfaced (`suppressed: true`), but contributes nothing to exit codes.
    if (behavior === "at-risk") {
      suppressed = suppressionActive(
        assertion.suppressed,
        opts.evidence,
        behaviorChanged,
      );
    }
  }

  const expired =
    assertion.ttl !== undefined && Date.parse(assertion.ttl) <= now;

  const gates = computeGates(
    { doc: docSide.state, code, behavior, expired },
    assertion.enforcement,
  );

  // Primary evidence side: code (the precision-critical side) when present.
  const primary = primaryCode ?? docSide;

  const notes = [
    ...docSide.notes.map((n) => `doc: ${n}`),
    ...codeNotes,
    behavioral ? "behavioral claim" : "",
  ].filter(Boolean);

  // Merge the behavioral evidence-path changes (imports/verifier sources) in,
  // deduped by (path, kind) — so each behavioral banner line names the changed
  // evidence path even when the anchored span itself is untouched (D14).
  const changedEvidence = dedupeEvidence([
    ...docSide.changedEvidence,
    ...codeChanged,
    ...behaviorChanged,
  ]);

  return {
    assertionId: assertion.id,
    propositionId: assertion.propositionId,
    documentId: assertion.documentId,
    doc: docSide.state,
    code,
    behavior,
    expired,
    gates,
    suppressed,
    // Deterministic next-action menu derived from the computed states (§9).
    remediation: remediationFor({
      assertionId: assertion.id,
      doc: docSide.state,
      code,
      behavior,
      expired,
      changedEvidence,
    }),
    evidence: {
      docRegion: docSide.region ?? undefined,
      codeRegions,
      confidence: primary.confidence,
      selectorScores: primary.selectorScores,
      changedEvidence,
      ref: assertion.ref,
    },
    notes,
    advisories: [],
  };
}
