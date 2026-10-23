/**
 * `reanchor`: re-point an existing claim at its current location.
 *
 * Every span re-resolves against the current working tree. An explicit
 * replacement span (`docSpec` / `evidence[]`) is stored as given. Without
 * one, a side that reads `moved` is rebuilt at its new location, a side that
 * reads `unchanged` is kept, and a side that reads `changed`, `ambiguous` or
 * `orphaned` is kept as stored and listed in `kept` with a reason naming the
 * flag to pass. The claim's `sentence` is refreshed from a rebuilt doc span. With `docOnly`
 * (`supersede`) the claim moves to the new file (`documentId` and the doc
 * side's file); the text-position selector is rebuilt only when the stored
 * quote occurs verbatim exactly once there. The sentence, the quote, the
 * hashes, evidence bundles, their baselines, and `ref` stay as stored.
 *
 * Safety: an empty span is never stored; nothing is written when no side
 * was rebuilt. An evidence side in a sibling repo
 * that is not checked out keeps its stored bundle unchanged, with a warning. The result carries the before and
 * after quotes per side so the caller can see what moved.
 */

import { fuzzyLocate, regionText } from "../algo/localize.ts";
import { textSimilarity } from "../algo/normalize.ts";
import {
  type AstAnalyzer,
  type EvidenceSide,
  type ResolveFiles,
  resolveClaimSides,
  worstEvidenceState,
} from "../algo/resolve.ts";
import type {
  AnchorState,
  Claim,
  ClaimKind,
  EvidenceState,
  Region,
  SelectorBundle,
} from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";
import { type AnchorAnalyzer, buildSelectorBundle } from "./anchor.ts";
import { languageForFile } from "./lang.ts";
import {
  documentIdForPath,
  type EvidenceTarget,
  newDocument,
  type RecordContents,
  type RegionSpec,
  resolveRegion,
  validateDocQuote,
} from "./record.ts";

export interface ReanchorInput {
  claimId: string;
  /** Replacement doc-side span; omit to re-localize via the existing selectors. */
  docSpec?: RegionSpec;
  /** Re-home the doc anchor to a different file (requires `docSpec`). */
  docPath?: string;
  /** Replacement evidence targets; omit to re-localize the existing bundles. */
  evidence?: EvidenceTarget[];
  /**
   * Move the doc side only (`supersede`): the evidence bundles, their
   * baselines, and `ref` stay as stored, so a changed evidence side stays
   * changed.
   */
  docOnly?: boolean;
  /** The ref to stamp (the caller resolves git HEAD). */
  ref?: string;
  analyzer?: AnchorAnalyzer & AstAnalyzer;
  dryRun?: boolean;
  /** Clock for a newly created document's `createdAt`. Default: now. */
  now?: Date;
}

export interface SideQuote {
  file: string;
  quote: string;
}

/** A side plain `reanchor` left as stored, and why. */
export interface KeptSide {
  side: "doc" | "evidence";
  file: string;
  state: AnchorState;
  /** e.g. `doc: changed; pass --doc-quote to re-anchor`. */
  reason: string;
}

export interface ReanchorResult {
  claim: Claim;
  /** Number of sides rebuilt (explicit spans and `moved` sides). */
  relocated: number;
  /** Sides that needed relocating but were left as stored. */
  kept: KeptSide[];
  doc: AnchorState;
  /** Absent when the claim has no evidence span. */
  evidence?: EvidenceState;
  before: { doc: SideQuote; evidence: SideQuote[] };
  after: { doc: SideQuote; evidence: SideQuote[] };
  warnings: string[];
}

/** One candidate location for `reanchor --suggest`. */
export interface ReanchorCandidate {
  side: "doc" | "evidence";
  file: string;
  start: number;
  end: number;
  similarity: number;
  /** The region text, trimmed to 120 chars. */
  snippet: string;
}

export interface ReanchorSuggestResult {
  action: "reanchor-suggest";
  claimId: string;
  candidates: ReanchorCandidate[];
}

const SUGGEST_MIN_SIMILARITY = 0.5;
const SUGGEST_MAX_CANDIDATES = 5;
const SUGGEST_SNIPPET_MAX = 120;

function quoteOf(bundle: SelectorBundle): string {
  const tq = bundle.selectors.find((s) => s.kind === "text-quote");
  return tq?.kind === "text-quote" ? tq.exact : "";
}

function sideQuote(bundle: SelectorBundle): SideQuote {
  return { file: bundle.file, quote: quoteOf(bundle) };
}

/**
 * Rank candidate locations for one side's stored quote across `files`. Exact
 * occurrences first, then fuzzy matches; candidates below the similarity floor
 * are dropped and the rest sort by similarity, then path, then offset.
 */
export function suggestForBundle(
  side: "doc" | "evidence",
  bundle: SelectorBundle,
  files: { path: string; content: string }[],
): ReanchorCandidate[] {
  const tq = bundle.selectors.find((s) => s.kind === "text-quote");
  if (tq?.kind !== "text-quote" || tq.exact.length === 0) return [];
  const out: ReanchorCandidate[] = [];
  for (const { path, content } of files) {
    const seen = new Set<number>();
    const push = (region: Region) => {
      if (seen.has(region.start)) return;
      seen.add(region.start);
      const text = regionText(content, region);
      const similarity = textSimilarity(text, tq.exact);
      if (similarity < SUGGEST_MIN_SIMILARITY) return;
      out.push({
        side,
        file: path,
        start: region.start,
        end: region.end,
        similarity,
        snippet: text.slice(0, SUGGEST_SNIPPET_MAX),
      });
    };
    let i = content.indexOf(tq.exact);
    while (i !== -1) {
      push({ start: i, end: i + tq.exact.length });
      i = content.indexOf(tq.exact, i + tq.exact.length);
    }
    const fuzzy = fuzzyLocate(content, tq, 0);
    if (fuzzy) push(fuzzy);
  }
  out.sort(
    (a, b) =>
      b.similarity - a.similarity ||
      (a.file < b.file ? -1 : a.file > b.file ? 1 : 0) ||
      a.start - b.start,
  );
  return out.slice(0, SUGGEST_MAX_CANDIDATES);
}

function toResolveFiles(contents: RecordContents): ResolveFiles {
  return {
    doc: contents.docContent,
    evidence: new Map(Object.entries(contents.evidenceContents)),
    unavailable: contents.unavailable,
  };
}

function keptSide(
  side: "doc" | "evidence",
  file: string,
  state: AnchorState,
  kind: ClaimKind,
): KeptSide {
  const flag =
    side === "doc"
      ? "--doc-quote"
      : kind === "reference"
        ? "--ref-quote"
        : "--code-quote";
  return {
    side,
    file,
    state,
    reason: `${side}: ${state}; pass ${flag} to re-anchor`,
  };
}

/** The structural language of an evidence file: code claims only. */
function evidenceLanguage(
  kind: ClaimKind,
  file: string,
  override?: string,
): string | undefined {
  return kind === "code" ? (override ?? languageForFile(file)) : undefined;
}

/** Rebuild one evidence bundle from an explicit replacement target. */
function bundleFromTarget(
  kind: ClaimKind,
  target: EvidenceTarget,
  contents: RecordContents,
  analyzer: AnchorAnalyzer | undefined,
): SelectorBundle {
  if (!target.region) {
    throw new Error(
      `evidence target ${target.file} has no span; pass ${kind === "reference" ? "--ref-quote or --ref-range" : "--code-quote or --code-range"}`,
    );
  }
  const content = contents.evidenceContents[target.file];
  if (content === undefined || content === null) {
    throw new Error(`Evidence file not found on disk: ${target.file}`);
  }
  const region = resolveRegion(content, target.region);
  return buildSelectorBundle(target.file, content, region, {
    language: evidenceLanguage(kind, target.file, target.language),
    analyzer,
  });
}

/**
 * Plain `reanchor` on one existing evidence bundle: rebuilt when it reads
 * `moved`, kept when it reads `unchanged`, kept with a warning when
 * `unavailable` (sibling repo not checked out), and kept with a `kept` entry
 * when it reads `changed`, `ambiguous` or `orphaned`.
 */
function relocateEvidenceBundle(
  kind: ClaimKind,
  bundle: SelectorBundle,
  contents: RecordContents,
  side: EvidenceSide,
  analyzer: AnchorAnalyzer | undefined,
  warnings: string[],
  kept: KeptSide[],
): { bundle: SelectorBundle; relocated: boolean } {
  if (side.state === "unavailable") {
    warnings.push(
      `evidence side ${bundle.file} kept unchanged: ${side.reasons.join("; ")}`,
    );
    return { bundle, relocated: false };
  }
  if (side.state === "unchanged") return { bundle, relocated: false };
  const content = contents.evidenceContents[bundle.file];
  if (
    side.state !== "moved" ||
    content === undefined ||
    content === null ||
    !side.region
  ) {
    kept.push(keptSide("evidence", bundle.file, side.state, kind));
    return { bundle, relocated: false };
  }
  return {
    bundle: buildSelectorBundle(bundle.file, content, side.region, {
      language: evidenceLanguage(kind, bundle.file),
      analyzer,
    }),
    relocated: true,
  };
}

/**
 * The doc selectors for `supersede` in a new file: the text-position selector
 * moved to the stored quote's only verbatim occurrence, or the selectors
 * unchanged when the quote occurs zero or several times.
 */
function repositionDoc(
  selectors: SelectorBundle["selectors"],
  content: string,
): SelectorBundle["selectors"] {
  const quote = selectors.find((s) => s.kind === "text-quote");
  if (quote?.kind !== "text-quote" || quote.exact.length === 0)
    return selectors;
  const at = content.indexOf(quote.exact);
  if (at < 0 || content.indexOf(quote.exact, at + 1) >= 0) return selectors;
  return selectors.map((s) =>
    s.kind === "text-position"
      ? { ...s, start: at, end: at + quote.exact.length }
      : s,
  );
}

export async function reanchor(
  store: ClaimStore,
  contents: RecordContents,
  input: ReanchorInput,
): Promise<ReanchorResult> {
  const claim = await store.getClaim(input.claimId);
  if (!claim) throw new Error(`No claim ${input.claimId} in the store.`);

  const files = toResolveFiles(contents);
  const warnings: string[] = [];
  const kept: KeptSide[] = [];
  let relocated = 0;

  // Resolve every span once.
  const before = resolveClaimSides(claim, files, { ast: input.analyzer });

  // Doc side.
  const docFile = input.docPath ?? claim.doc.file;
  if (contents.docContent === null) {
    throw new Error(`Document not found on disk: ${docFile}`);
  }
  let docBundle: SelectorBundle;
  let sentence: string;
  if (input.docOnly) {
    // `supersede`: the claim moves to the new file. When the stored quote
    // occurs verbatim exactly once there, the text-position selector is
    // rebuilt at that occurrence; otherwise every selector stays as stored,
    // so the next check reports `changed`, `ambiguous`, or `orphaned`. The
    // sentence, the quote, and the hashes never change.
    docBundle = {
      file: docFile,
      selectors: repositionDoc(claim.doc.selectors, contents.docContent),
    };
    sentence = claim.sentence;
  } else {
    if (
      input.docPath !== undefined &&
      input.docPath !== claim.doc.file &&
      !input.docSpec
    ) {
      throw new Error(
        `relocating claim ${claim.id} to ${input.docPath} requires an explicit doc span (--doc-quote/--doc-range); the existing selectors describe the old file.`,
      );
    }
    let docRegion: Region | undefined;
    if (input.docSpec) {
      docRegion = resolveRegion(contents.docContent, input.docSpec);
    } else if (before.doc.state === "moved" && before.doc.region) {
      docRegion = before.doc.region;
    } else if (before.doc.state !== "unchanged") {
      kept.push(keptSide("doc", docFile, before.doc.state, claim.kind));
    }
    if (docRegion) {
      warnings.push(
        ...validateDocQuote(contents.docContent, docRegion, docFile),
      );
      docBundle = buildSelectorBundle(docFile, contents.docContent, docRegion);
      sentence = regionText(contents.docContent, docRegion);
      relocated++;
    } else {
      docBundle = claim.doc;
      sentence = claim.sentence;
    }
  }

  // Evidence side.
  let evidence: SelectorBundle[];
  if (input.docOnly) {
    evidence = claim.evidence;
  } else if (input.evidence !== undefined) {
    if (claim.kind === "attested" && input.evidence.length > 0) {
      throw new Error(
        `claim ${claim.id} is attested and has no evidence spans.`,
      );
    }
    if (claim.kind === "reference" && input.evidence.length !== 1) {
      throw new Error(
        `claim ${claim.id} is a reference claim and needs exactly one evidence span.`,
      );
    }
    evidence = input.evidence.map((t) =>
      bundleFromTarget(claim.kind, t, contents, input.analyzer),
    );
    relocated += evidence.length;
  } else {
    evidence = claim.evidence.map((bundle, i) => {
      const r = relocateEvidenceBundle(
        claim.kind,
        bundle,
        contents,
        before.evidence[i] as EvidenceSide,
        input.analyzer,
        warnings,
        kept,
      );
      if (r.relocated) relocated++;
      return r.bundle;
    });
  }

  const documentId = input.docPath
    ? documentIdForPath(input.docPath)
    : claim.documentId;

  const next: Claim = {
    ...claim,
    documentId,
    sentence,
    doc: docBundle,
    evidence,
    ref:
      input.docOnly || relocated === 0 ? claim.ref : (input.ref ?? claim.ref),
  };
  // `supersede` (docOnly) always writes: the claim moves to the new file.
  const write = !input.dryRun && (input.docOnly || relocated > 0);
  if (write) await store.putClaim(next);

  if (write && input.docPath && documentId !== claim.documentId) {
    const existing = await store.getDocument(documentId);
    if (!existing) {
      await store.putDocument(
        newDocument(documentId, input.docPath, input.now ?? new Date()),
      );
    } else if (existing.lifecycle !== "active") {
      await store.putDocument({ ...existing, lifecycle: "active" });
    }
  }

  const after = resolveClaimSides(next, files, { ast: input.analyzer });
  return {
    claim: next,
    relocated,
    kept,
    doc: after.doc.state,
    ...(after.evidence.length > 0
      ? { evidence: worstEvidenceState(after.evidence) }
      : {}),
    before: {
      doc: sideQuote(claim.doc),
      evidence: claim.evidence.map(sideQuote),
    },
    after: {
      doc: sideQuote(next.doc),
      evidence: next.evidence.map(sideQuote),
    },
    warnings,
  };
}
