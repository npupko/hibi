/**
 * `reanchor`: re-point an existing claim at its current location.
 *
 * Both sides re-resolve against the current working tree and the stored
 * baseline is rewritten so a `moved`/`changed` claim settles back to
 * `unchanged`. Explicit replacement spans (`docSpec` / `code[]`) override;
 * otherwise each side re-localizes through its existing selectors.
 *
 * Safety: a side that resolves `orphaned` and has no explicit replacement is
 * refused; an empty span is never stored. The result carries the before and
 * after quotes per side so the caller can see what moved.
 */

import { fuzzyLocate, regionText } from "../algo/localize.ts";
import { textSimilarity } from "../algo/normalize.ts";
import {
  type AstAnalyzer,
  type ResolveFiles,
  resolveSides,
  type SideResult,
  worstCodeState,
} from "../algo/resolve.ts";
import { newId, propositionFingerprint } from "../core/ids.ts";
import type {
  AnchorState,
  Assertion,
  Proposition,
  Region,
  SelectorBundle,
} from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";
import {
  type AnchorAnalyzer,
  buildCoarseBundle,
  buildSelectorBundle,
  composeAnchor,
  isCoarseBundle,
} from "./anchor.ts";
import { languageForFile } from "./lang.ts";
import {
  type CodeTarget,
  documentIdForPath,
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
  /** Replacement code-side targets; omit to re-localize the existing bundles. */
  code?: CodeTarget[];
  /** The ref to stamp (the caller resolves git HEAD). */
  ref?: string;
  analyzer?: AnchorAnalyzer & AstAnalyzer;
  dryRun?: boolean;
}

export interface SideQuote {
  file: string;
  quote: string;
}

export interface ReanchorResult {
  assertion: Assertion;
  doc: AnchorState;
  code: AnchorState;
  before: { doc: SideQuote; code: SideQuote[] };
  after: { doc: SideQuote; code: SideQuote[] };
  warnings: string[];
}

/** One candidate location for `reanchor --suggest`. */
export interface ReanchorCandidate {
  side: "doc" | "code";
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
  side: "doc" | "code",
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
    code: new Map(Object.entries(contents.codeContents)),
  };
}

function orphanError(id: string, side: string, file: string): Error {
  return new Error(
    `claim ${id}: the ${side} span in ${file} was not found (orphaned). Pass an explicit new span for that side (${side === "doc" ? "--doc-quote/--doc-range" : "--code-file with --code-quote/--code-range"}), run --suggest to find candidates, or retire the claim.`,
  );
}

/** Rebuild one code bundle from an explicit replacement target. */
function bundleFromTarget(
  target: CodeTarget,
  contents: RecordContents,
  analyzer: AnchorAnalyzer | undefined,
): SelectorBundle {
  if (target.coarse) return buildCoarseBundle(target.file);
  if (!target.region) {
    throw new Error(
      `code target ${target.file} has no span; pass --code-quote or --code-range`,
    );
  }
  const content = contents.codeContents[target.file];
  if (content === undefined || content === null) {
    throw new Error(`Code file not found on disk: ${target.file}`);
  }
  const region = resolveRegion(content, target.region);
  return buildSelectorBundle(target.file, content, region, {
    language: target.language ?? languageForFile(target.file),
    analyzer,
  });
}

/** Re-localize an existing code bundle in place from its resolved side result. */
function relocateCodeBundle(
  bundle: SelectorBundle,
  contents: RecordContents,
  side: SideResult,
  analyzer: AnchorAnalyzer | undefined,
  claimId: string,
): SelectorBundle {
  if (isCoarseBundle(bundle)) return bundle;
  const content = contents.codeContents[bundle.file];
  if (content === undefined || content === null || !side.region) {
    throw orphanError(claimId, "code", bundle.file);
  }
  return buildSelectorBundle(bundle.file, content, side.region, {
    language: languageForFile(bundle.file),
    analyzer,
  });
}

export async function reanchor(
  store: ClaimStore,
  contents: RecordContents,
  input: ReanchorInput,
): Promise<ReanchorResult> {
  const assertion = await store.getAssertion(input.claimId);
  if (!assertion) throw new Error(`No claim ${input.claimId} in the store.`);
  const proposition = await store.getProposition(assertion.propositionId);
  if (!proposition) {
    throw new Error(
      `Proposition ${assertion.propositionId} missing for claim ${input.claimId}.`,
    );
  }

  const files = toResolveFiles(contents);
  const warnings: string[] = [];

  // Resolve every side once.
  const before = resolveSides(assertion, files, { ast: input.analyzer });

  // Doc side.
  const docFile = input.docPath ?? assertion.anchor.doc.file;
  if (contents.docContent === null) {
    throw new Error(`Document not found on disk: ${docFile}`);
  }
  if (
    input.docPath !== undefined &&
    input.docPath !== assertion.anchor.doc.file &&
    !input.docSpec
  ) {
    throw new Error(
      `relocating claim ${assertion.id} to ${input.docPath} requires an explicit doc span (--doc-quote/--doc-range); the existing selectors describe the old file.`,
    );
  }
  let docRegion: Region;
  if (input.docSpec) {
    docRegion = resolveRegion(contents.docContent, input.docSpec);
  } else {
    if (before.doc.state === "orphaned" || !before.doc.region) {
      throw orphanError(assertion.id, "doc", docFile);
    }
    docRegion = before.doc.region;
  }
  warnings.push(...validateDocQuote(contents.docContent, docRegion, docFile));
  const docBundle = buildSelectorBundle(
    docFile,
    contents.docContent,
    docRegion,
  );
  const confirmed = regionText(contents.docContent, docRegion);

  // Code side.
  let codeBundles: SelectorBundle[];
  if (input.code !== undefined) {
    codeBundles = input.code.map((t) =>
      bundleFromTarget(t, contents, input.analyzer),
    );
  } else {
    codeBundles = assertion.anchor.code.map((bundle, i) =>
      relocateCodeBundle(
        bundle,
        contents,
        before.code[i] as SideResult,
        input.analyzer,
        assertion.id,
      ),
    );
  }

  const documentId = input.docPath
    ? documentIdForPath(input.docPath)
    : assertion.documentId;

  // Proposition refresh, split-on-write when shared.
  const newFingerprint = propositionFingerprint(confirmed);
  const shared = (await store.allAssertions()).some(
    (x) => x.propositionId === proposition.id && x.id !== assertion.id,
  );
  let propositionId = assertion.propositionId;
  if (shared && newFingerprint !== proposition.fingerprint) {
    const fresh: Proposition = {
      id: newId("prop"),
      textCache: confirmed,
      fingerprint: newFingerprint,
    };
    propositionId = fresh.id;
    if (!input.dryRun) await store.putProposition(fresh);
  } else {
    proposition.textCache = confirmed;
    proposition.fingerprint = newFingerprint;
    if (!input.dryRun) await store.putProposition(proposition);
  }

  const next: Assertion = {
    ...assertion,
    documentId,
    propositionId,
    anchor: composeAnchor(docBundle, codeBundles),
    ref: input.ref ?? assertion.ref,
  };
  if (!input.dryRun) await store.putAssertion(next);

  if (!input.dryRun && input.docPath && documentId !== assertion.documentId) {
    const existing = await store.getDocument(documentId);
    if (!existing) {
      await store.putDocument(newDocument(documentId, input.docPath));
    } else if (existing.lifecycle !== "active") {
      await store.putDocument({ ...existing, lifecycle: "active" });
    }
  }

  const after = resolveSides(next, files, { ast: input.analyzer });
  return {
    assertion: next,
    doc: after.doc.state,
    code: worstCodeState(after.code),
    before: {
      doc: sideQuote(assertion.anchor.doc),
      code: assertion.anchor.code.map(sideQuote),
    },
    after: {
      doc: sideQuote(next.anchor.doc),
      code: next.anchor.code.map(sideQuote),
    },
    warnings,
  };
}
