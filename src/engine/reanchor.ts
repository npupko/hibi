/**
 * `reanchor`: re-point an existing claim at its current location.
 *
 * Every span re-resolves against the current working tree and the stored
 * baseline is rewritten so a `moved`/`changed` claim settles back to
 * `unchanged`. Explicit replacement spans (`docSpec` / `evidence[]`) override;
 * otherwise each span re-localizes through its existing selectors. The
 * claim's `sentence` is refreshed from the new doc span.
 *
 * Safety: a side that resolves `orphaned` and has no explicit replacement is
 * refused; an empty span is never stored. An evidence side in a sibling repo
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

export interface ReanchorResult {
  claim: Claim;
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

function orphanError(
  id: string,
  side: "doc" | "evidence",
  file: string,
  kind: ClaimKind,
): Error {
  const flags =
    side === "doc"
      ? "--doc-quote/--doc-range"
      : kind === "reference"
        ? "--ref-file with --ref-quote/--ref-range"
        : "--code-file with --code-quote/--code-range";
  return new Error(
    `claim ${id}: the ${side} span in ${file} was not found (orphaned). Pass an explicit new span for that side (${flags}), run --suggest to find candidates, or retire the claim.`,
  );
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
 * Re-localize an existing evidence bundle in place from its resolved side
 * result. An `unavailable` side (sibling repo not checked out) keeps the
 * stored bundle and adds a warning.
 */
function relocateEvidenceBundle(
  kind: ClaimKind,
  bundle: SelectorBundle,
  contents: RecordContents,
  side: EvidenceSide,
  analyzer: AnchorAnalyzer | undefined,
  claimId: string,
  warnings: string[],
): SelectorBundle {
  if (side.state === "unavailable") {
    warnings.push(
      `evidence side ${bundle.file} kept unchanged: ${side.reasons.join("; ")}`,
    );
    return bundle;
  }
  const content = contents.evidenceContents[bundle.file];
  if (content === undefined || content === null || !side.region) {
    throw orphanError(claimId, "evidence", bundle.file, kind);
  }
  return buildSelectorBundle(bundle.file, content, side.region, {
    language: evidenceLanguage(kind, bundle.file),
    analyzer,
  });
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

  // Resolve every span once.
  const before = resolveClaimSides(claim, files, { ast: input.analyzer });

  // Doc side.
  const docFile = input.docPath ?? claim.doc.file;
  if (contents.docContent === null) {
    throw new Error(`Document not found on disk: ${docFile}`);
  }
  if (
    input.docPath !== undefined &&
    input.docPath !== claim.doc.file &&
    !input.docSpec
  ) {
    throw new Error(
      `relocating claim ${claim.id} to ${input.docPath} requires an explicit doc span (--doc-quote/--doc-range); the existing selectors describe the old file.`,
    );
  }
  let docRegion: Region;
  if (input.docSpec) {
    docRegion = resolveRegion(contents.docContent, input.docSpec);
  } else {
    if (before.doc.state === "orphaned" || !before.doc.region) {
      throw orphanError(claim.id, "doc", docFile, claim.kind);
    }
    docRegion = before.doc.region;
  }
  warnings.push(...validateDocQuote(contents.docContent, docRegion, docFile));
  const docBundle = buildSelectorBundle(
    docFile,
    contents.docContent,
    docRegion,
  );
  const sentence = regionText(contents.docContent, docRegion);

  // Evidence side.
  let evidence: SelectorBundle[];
  if (input.evidence !== undefined) {
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
  } else {
    evidence = claim.evidence.map((bundle, i) =>
      relocateEvidenceBundle(
        claim.kind,
        bundle,
        contents,
        before.evidence[i] as EvidenceSide,
        input.analyzer,
        claim.id,
        warnings,
      ),
    );
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
    ref: input.ref ?? claim.ref,
  };
  if (!input.dryRun) await store.putClaim(next);

  if (!input.dryRun && input.docPath && documentId !== claim.documentId) {
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
