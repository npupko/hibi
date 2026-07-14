/**
 * Recording a new claim (§9 `record`): write a Proposition (deduped by content
 * fingerprint of the *confirmed doc span*), an Assertion, and the composite
 * baseline Anchor to the store. Span-first and bidirectional (§4/§18-B): the
 * documented sentence's own span is the doc-side anchor, and zero or more code
 * targets form the code-side bundles. Agent-authored — the engine never
 * NLP-extracts claims (D2).
 */

import { regionText } from "../algo/localize.ts";
import { textSimilarity } from "../algo/normalize.ts";
import {
  AMBIGUOUS_MIN_QUOTE_LENGTH,
  TEXT_QUOTE_CONTEXT,
} from "../algo/params.ts";
import { newId, propositionFingerprint } from "../core/ids.ts";
import type {
  Anchor,
  Assertion,
  AuthoredTrust,
  BehaviorScope,
  Enforcement,
  Proposition,
  Region,
  SelectorBundle,
  Verifier,
} from "../core/model.ts";
import { Document } from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";
import {
  type AnchorAnalyzer,
  buildGlobBundle,
  buildPathBundle,
  buildSelectorBundle,
  composeAnchor,
} from "./anchor.ts";
import { languageForFile } from "./lang.ts";

/** Stable document id derived from its repo-relative path. */
export function documentIdForPath(path: string): string {
  return `doc_${Bun.hash.xxHash64(path).toString(16).padStart(16, "0")}`;
}

/**
 * A fresh active Document with schema defaults applied once (§4). Routing every
 * construction site through `Document.parse` keeps a defaulted field (e.g.
 * `pristine`) defined in exactly one place — the schema — instead of repeated in
 * each object literal, so adding a field is not an N-call-site edit.
 */
export function newDocument(
  id: string,
  path: string,
  pristine = false,
): Document {
  return Document.parse({ id, path, pristine });
}

/** How a record call locates a span inside a file (doc side or code side). */
export interface RegionSpec {
  /** A literal quote to find… */
  quote?: string;
  /** …or explicit char offsets… */
  start?: number;
  end?: number;
  /** …or a 1-based line number… */
  line?: number;
  /** …or an inclusive 1-based line range (`L42:L44`). */
  startLine?: number;
  endLine?: number;
}

/**
 * A code target this claim describes (§4). One bundle on the code side: a
 * precise region (resolved from the file's content) or a coarse path/glob edge
 * (navigation and blast-radius only, never reported as drift — §11.3).
 */
export interface CodeTarget {
  /** Repo-relative path of the code file (or the glob's notional file). */
  file: string;
  /** The span inside `file`; omit for a coarse target. */
  region?: RegionSpec;
  /** true → coarse path bundle (`buildPathBundle`). */
  coarse?: boolean;
  /** set → coarse glob bundle (`buildGlobBundle`). */
  glob?: string;
  /** Optional structural-language override; else `languageForFile(file)`. */
  language?: string;
}

export interface RecordInput {
  /** Repo-relative path of the document making the claim. */
  docPath: string;
  /** The documented sentence's span (preferred — the doc side is span-first). */
  docSpec?: RegionSpec;
  /** Optional owned-doc inline marker id that stabilizes re-anchoring (§4/§8). */
  inlineId?: string;
  authoredTrust: AuthoredTrust;
  owner: string;
  ref: string;
  ttl?: string;
  /** Zero or more code targets the claim describes. */
  code: CodeTarget[];
  /** Explicit enforcement override; else derived below (§9). */
  enforcement?: Enforcement;
  /** Mark the document pristine — `check --write` never stamps it (§8, D17). */
  pristine?: boolean;
  /** Author's behavioral declaration (§17.6, D12); undefined → heuristic decides. */
  behavioral?: boolean;
  verifiers?: Verifier[];
  behaviorScope?: BehaviorScope;
  /**
   * The change-gate evidence baseline (§17.6, D14), computed by the shell (which
   * has FS + analyzer) and stored on the Assertion. Undefined → not captured.
   */
  evidenceBaseline?: Record<string, string>;
  analyzer?: AnchorAnalyzer;
  attrs?: Record<string, unknown>;
}

/** The on-disk content of the doc and each referenced code file (read by the shell). */
export interface RecordContents {
  docContent: string | null;
  codeContents: Record<string, string | null>;
}

export interface RecordResult {
  document: Document;
  proposition: Proposition;
  assertion: Assertion;
  dedupedProposition: boolean;
  /**
   * Other claim ids that already assert the *same proposition* (this proposition
   * was reached by fingerprint dedup), excluding the one just recorded/returned.
   * Empty when the proposition is brand new. Surfaced so the author can spot a
   * re-record of an existing claim and reach for `reanchor` instead (§9).
   */
  existingClaims: string[];
}

export async function recordClaim(
  store: ClaimStore,
  contents: RecordContents,
  input: RecordInput,
): Promise<RecordResult> {
  const { docContent, codeContents } = contents;

  // ── Doc side: resolve the documented sentence's span (§18-B). ──
  // The confirmed text is the live doc span — the only source of the claim text
  // (there is no side-channel override; D16).
  let docRegion: Region | undefined;
  if (input.docSpec) {
    if (docContent === null)
      throw new Error(`Document not found on disk: ${input.docPath}`);
    docRegion = resolveRegion(docContent, input.docSpec);
  }

  const confirmedText =
    docRegion !== undefined && docContent !== null
      ? regionText(docContent, docRegion)
      : undefined;
  if (confirmedText === undefined)
    throw new Error(
      "A claim requires a doc span (--doc-quote/--doc-range/--doc-line).",
    );

  // D23 — reject a doc-side quote that cannot anchor reliably (too short, or a
  // repeated span the stored context cannot disambiguate). docRegion + docContent
  // are both defined here (confirmedText required them).
  if (docRegion !== undefined && docContent !== null)
    validateDocQuote(docContent, docRegion, input.docPath);

  // ── Code side: a bundle per target. ──
  const codeBundles: SelectorBundle[] = [];
  /** Whether every *precise* code target resolved uniquely (enforcement gate). */
  let allCodeResolved = true;
  /** Whether at least one *precise* code anchor resolved — coarse edges don't count. */
  let hasPreciseCode = false;
  for (const target of input.code) {
    if (target.glob !== undefined) {
      codeBundles.push(buildGlobBundle(target.glob));
      continue;
    }
    if (target.coarse) {
      codeBundles.push(buildPathBundle(target.file));
      continue;
    }
    // A precise target with no locator is a caller error, not a coarse edge —
    // surface it rather than silently degrading (use `--coarse` for an edge).
    if (target.region === undefined) {
      throw new Error(
        `code target ${target.file} has no locator — pass --code-quote/--code-range/--code-line, or --coarse for a navigational edge.`,
      );
    }
    const content = codeContents[target.file];
    if (content === undefined || content === null) {
      // The file is missing on disk → it cannot be located precisely. Keep a
      // coarse path edge so navigation survives, and mark the target unresolved
      // so a `verified`/`enforced` claim is refused below.
      allCodeResolved = false;
      codeBundles.push(buildPathBundle(target.file));
      continue;
    }
    const region = resolveRegion(content, target.region);
    hasPreciseCode = true;
    codeBundles.push(
      buildSelectorBundle(target.file, content, region, {
        language: target.language ?? languageForFile(target.file),
        analyzer: input.analyzer,
      }),
    );
  }

  // Doc resolved uniquely iff a real span (not a legacy text-only override) was found.
  const docResolved = docRegion !== undefined;

  // `verified` authored trust requires a resolvable anchor + @ref (§10),
  // regardless of how the record is enforced.
  if (input.authoredTrust === "verified" && (!docResolved || !input.ref)) {
    throw new Error(
      "`verified` trust requires a resolvable doc span and a @ref.",
    );
  }

  // ── Enforcement (§9). Explicit override wins; else derive. ──
  const enforcement: Enforcement =
    input.enforcement ??
    (input.authoredTrust === "verified" &&
    input.ref &&
    docResolved &&
    allCodeResolved &&
    hasPreciseCode
      ? "enforced"
      : "suggested");

  // ── Refusal (§9/§11.3/§18-B): an `enforced` (gating-eligible) claim must
  // resolve BOTH sides to a precise span — whatever set that enforcement, an
  // explicit `--enforce` included. Coarse path/glob edges are navigational only.
  if (
    enforcement === "enforced" &&
    (!docResolved || !input.ref || !hasPreciseCode || !allCodeResolved)
  ) {
    throw new Error(
      "an `enforced` claim requires a resolvable doc span, a @ref, and a precise code anchor (coarse path/glob edges are navigational only).",
    );
  }

  // ── Document (upsert by path). ── `--pristine` persists the per-document flag
  // (§8, D17): set it on a new document, and promote an existing one to pristine
  // when the flag is passed (never silently un-pristine it).
  const docId = documentIdForPath(input.docPath);
  let document = await store.getDocument(docId);
  if (!document) {
    document = newDocument(docId, input.docPath, input.pristine ?? false);
    await store.putDocument(document);
  } else if (input.pristine && !document.pristine) {
    document = { ...document, pristine: true };
    await store.putDocument(document);
  }

  // ── Proposition (dedup by fingerprint of the confirmed text — §5). ──
  const fingerprint = propositionFingerprint(confirmedText);
  let proposition = await store.findPropositionByFingerprint(fingerprint);
  const deduped = proposition !== undefined;
  if (!proposition) {
    proposition = {
      id: newId("prop"),
      textCache: confirmedText,
      authoredTrust: input.authoredTrust,
      fingerprint,
    };
    await store.putProposition(proposition);
  }

  // Dedup the assertion: one verification instance per (proposition, document).
  // Re-running `record` on unchanged content must be idempotent (§6) —
  // otherwise each run would accumulate a duplicate assertion in the store.
  const allAssertions = await store.allAssertions();
  /** Claims already asserting this proposition (the duplicate-proposition signal). */
  const sharingProposition = allAssertions.filter(
    (x) => x.propositionId === proposition.id,
  );
  const existing = sharingProposition.find((x) => x.documentId === docId);
  if (existing) {
    return {
      document,
      proposition,
      assertion: existing,
      dedupedProposition: deduped,
      // Exclude the returned (existing) claim — the rest already share its proposition.
      existingClaims: sharingProposition
        .filter((x) => x.id !== existing.id)
        .map((x) => x.id),
    };
  }

  // ── Anchor (bidirectional, composite — §4). ──
  const docBundle: SelectorBundle = docRegion
    ? buildSelectorBundle(input.docPath, docContent ?? "", docRegion, {
        inlineId: input.inlineId,
      })
    : buildPathBundle(input.docPath);
  const anchor: Anchor = composeAnchor(docBundle, codeBundles);

  const assertion: Assertion = {
    id: newId("asrt"),
    propositionId: proposition.id,
    documentId: docId,
    owner: input.owner,
    ref: input.ref,
    anchor,
    enforcement,
    behavioral: input.behavioral,
    verifiers: input.verifiers ?? [],
    behaviorScope: input.behaviorScope,
    evidenceBaseline: input.evidenceBaseline,
    ttl: input.ttl,
    attrs: input.attrs ?? {},
  };
  await store.putAssertion(assertion);

  return {
    document,
    proposition,
    assertion,
    dedupedProposition: deduped,
    // The fresh assertion is not yet in `sharingProposition`; every id there is a
    // pre-existing claim on the same (deduped) proposition.
    existingClaims: sharingProposition.map((x) => x.id),
  };
}

/**
 * Record-time doc-quote guard (§17.1, D23). After the doc span is resolved
 * (span-first), reject a doc-side `text-quote` that cannot anchor reliably:
 *
 *   1. Length floor — a quote shorter than `AMBIGUOUS_MIN_QUOTE_LENGTH` occurs
 *      everywhere and is not a meaningful anchor.
 *   2. Uniqueness — if the exact quote occurs more than once, score each
 *      occurrence with the stored 48-char prefix/suffix context (the same text
 *      similarity the text-quote cascade uses); if the recorded occurrence is not
 *      *strictly* the best, the context cannot select a single span → reject.
 *
 * Prevents a bad anchor at birth, in `record`, `record --from-file`, and
 * `reanchor` alike (the failure to prevent is a degenerate span, not thin
 * context — D23).
 */
export function validateDocQuote(
  docContent: string,
  region: Region,
  docPath: string,
): void {
  const quote = regionText(docContent, region);
  if (quote.length < AMBIGUOUS_MIN_QUOTE_LENGTH) {
    throw new Error(
      "doc quote is shorter than 8 characters — too short to anchor reliably. Record a wider span (--doc-range) that covers the full sentence.",
    );
  }

  // Count exact occurrences of the quote in the document text.
  let count = 0;
  for (
    let j = docContent.indexOf(quote);
    j !== -1;
    j = docContent.indexOf(quote, j + quote.length)
  ) {
    count += 1;
  }
  if (count <= 1) return;

  // The stored 48-char context around the recorded span (buildSelectorBundle
  // captures exactly this window). Score each occurrence's live context against
  // it; the recorded occurrence must win outright.
  const storedPrefix = docContent.slice(
    Math.max(0, region.start - TEXT_QUOTE_CONTEXT),
    region.start,
  );
  const storedSuffix = docContent.slice(
    region.end,
    region.end + TEXT_QUOTE_CONTEXT,
  );
  const scores: number[] = [];
  for (
    let j = docContent.indexOf(quote);
    j !== -1;
    j = docContent.indexOf(quote, j + quote.length)
  ) {
    const pre = docContent.slice(Math.max(0, j - TEXT_QUOTE_CONTEXT), j);
    const suf = docContent.slice(
      j + quote.length,
      j + quote.length + TEXT_QUOTE_CONTEXT,
    );
    scores.push(
      textSimilarity(pre, storedPrefix) + textSimilarity(suf, storedSuffix),
    );
  }
  scores.sort((a, b) => b - a);
  const best = scores[0] ?? 0;
  const second = scores[1] ?? 0;
  if (!(best > second)) {
    throw new Error(
      `doc quote occurs ${count} times in ${docPath} and the surrounding context does not select a single occurrence. Record a wider span (--doc-range), or add an inline ID and re-record.`,
    );
  }
}

/**
 * Plan a record: resolve a span AND the 1-based line it starts on, in one pure
 * step (no I/O, no git). The line is the value a host needs to attribute the
 * anchor (e.g. `git blame`); deriving it lives HERE so the CLI shell and the
 * library shell can never drift on the off-by-one (§7.5, functional core).
 */
export function planRecord(
  content: string,
  spec: RegionSpec,
): { region: Region; line: number } {
  const region = resolveRegion(content, spec);
  const line = content.slice(0, region.start).split("\n").length;
  return { region, line };
}

/** Char offset of the first character of a 1-based `line` in `lines`. */
function lineStartOffset(lines: string[], line: number): number {
  let off = 0;
  for (let i = 0; i < line - 1 && i < lines.length; i++)
    off += (lines[i] ?? "").length + 1;
  return off;
}

/**
 * Resolve a region from a literal quote, char offsets, a 1-based line, or an
 * inclusive 1-based line range. Throws on a malformed numeric spec (e.g. NaN).
 */
export function resolveRegion(content: string, spec: RegionSpec): Region {
  if (spec.quote !== undefined) {
    const idx = content.indexOf(spec.quote);
    if (idx === -1)
      throw new Error(
        `Quote not found in file: ${JSON.stringify(spec.quote.slice(0, 40))}…`,
      );
    return { start: idx, end: idx + spec.quote.length };
  }
  if (spec.start !== undefined && spec.end !== undefined) {
    if (!Number.isFinite(spec.start) || !Number.isFinite(spec.end))
      throw new Error(`Malformed char range: ${spec.start}:${spec.end}`);
    return { start: spec.start, end: spec.end };
  }
  if (spec.startLine !== undefined && spec.endLine !== undefined) {
    if (!Number.isFinite(spec.startLine) || !Number.isFinite(spec.endLine))
      throw new Error(
        `Malformed line range: ${spec.startLine}:${spec.endLine}`,
      );
    const lines = content.split("\n");
    const start = lineStartOffset(lines, spec.startLine);
    const end =
      lineStartOffset(lines, spec.endLine) +
      (lines[spec.endLine - 1] ?? "").length;
    return { start, end: Math.max(start, end) };
  }
  if (spec.line !== undefined) {
    if (!Number.isFinite(spec.line))
      throw new Error(`Malformed line number: ${spec.line}`);
    const lines = content.split("\n");
    const off = lineStartOffset(lines, spec.line);
    return { start: off, end: off + (lines[spec.line - 1] ?? "").length };
  }
  throw new Error(
    "A region requires one of: --quote, --start/--end, --line, or a line range.",
  );
}
