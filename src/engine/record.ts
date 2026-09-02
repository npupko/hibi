/**
 * Recording a claim: write a Proposition (deduplicated by content fingerprint
 * of the confirmed doc span), an Assertion, and the baseline Anchor. The doc
 * span's text is the claim; the code targets are the spans it describes.
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
  buildCoarseBundle,
  buildSelectorBundle,
  composeAnchor,
} from "./anchor.ts";
import { languageForFile } from "./lang.ts";

/** Stable document id derived from its repo-relative path. */
export function documentIdForPath(path: string): string {
  return `doc_${Bun.hash.xxHash64(path).toString(16).padStart(16, "0")}`;
}

export function newDocument(id: string, path: string): Document {
  return Document.parse({ id, path });
}

/** How a call locates a span inside a file. */
export interface RegionSpec {
  /** A literal quote… */
  quote?: string;
  /** …or explicit char offsets… */
  start?: number;
  end?: number;
  /** …or an inclusive 1-based line range (`L42:L44`). */
  startLine?: number;
  endLine?: number;
}

/** A code target: a precise span in a file, or a coarse file/glob pattern. */
export interface CodeTarget {
  /** Repo-relative path of the code file, or the pattern when `coarse`. */
  file: string;
  /** The span inside `file`; required unless `coarse`. */
  region?: RegionSpec;
  /** Coarse pattern edge: navigation only, never graded as drift. */
  coarse?: boolean;
  /** Optional language override; else derived from the extension. */
  language?: string;
}

export interface RecordInput {
  docPath: string;
  docSpec?: RegionSpec;
  verified: boolean;
  owner: string;
  ref: string;
  ttl?: string;
  code: CodeTarget[];
  /** Default `enforced`. `suggested` is advisory and allows a coarse or empty code side. */
  enforcement?: Enforcement;
  verifiers?: Verifier[];
  analyzer?: AnchorAnalyzer;
  attrs?: Record<string, unknown>;
}

/** The on-disk content of the doc and each referenced code file. */
export interface RecordContents {
  docContent: string | null;
  codeContents: Record<string, string | null>;
}

export interface RecordResult {
  document: Document;
  proposition: Proposition;
  assertion: Assertion;
  dedupedProposition: boolean;
  /** Other claim ids that already assert the same proposition. */
  existingClaims: string[];
  /** Non-fatal notes, e.g. a doc quote that occurs more than once. */
  warnings: string[];
}

export async function recordClaim(
  store: ClaimStore,
  contents: RecordContents,
  input: RecordInput,
): Promise<RecordResult> {
  const { docContent, codeContents } = contents;
  const warnings: string[] = [];

  // Doc side.
  if (!input.docSpec) {
    throw new Error(
      "A claim requires a doc span (--doc-quote or --doc-range).",
    );
  }
  if (docContent === null) {
    throw new Error(`Document not found on disk: ${input.docPath}`);
  }
  const docRegion = resolveRegion(docContent, input.docSpec);
  const confirmedText = regionText(docContent, docRegion);
  warnings.push(...validateDocQuote(docContent, docRegion, input.docPath));

  // Code side.
  const enforcement: Enforcement = input.enforcement ?? "enforced";
  const codeBundles: SelectorBundle[] = [];
  let hasPreciseCode = false;
  for (const target of input.code) {
    if (target.coarse) {
      codeBundles.push(buildCoarseBundle(target.file));
      continue;
    }
    if (target.region === undefined) {
      throw new Error(
        `code target ${target.file} has no span; pass --code-quote or --code-range, or --glob for a coarse edge.`,
      );
    }
    const content = codeContents[target.file];
    if (content === undefined || content === null) {
      throw new Error(`Code file not found on disk: ${target.file}`);
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

  if (enforcement === "enforced" && !hasPreciseCode) {
    throw new Error(
      "an enforced claim needs a precise code span (--code-file with --code-quote or --code-range); pass --suggest for an advisory claim with a coarse or empty code side.",
    );
  }

  // Document (upsert by path).
  const docId = documentIdForPath(input.docPath);
  let document = await store.getDocument(docId);
  if (!document) {
    document = newDocument(docId, input.docPath);
    await store.putDocument(document);
  }

  // Proposition (dedup by fingerprint of the confirmed text).
  const fingerprint = propositionFingerprint(confirmedText);
  let proposition = await store.findPropositionByFingerprint(fingerprint);
  const deduped = proposition !== undefined;
  if (!proposition) {
    proposition = { id: newId("prop"), textCache: confirmedText, fingerprint };
    await store.putProposition(proposition);
  }

  const docBundle = buildSelectorBundle(input.docPath, docContent, docRegion);
  const anchor: Anchor = composeAnchor(docBundle, codeBundles);

  // One claim per (proposition, document): re-recording is idempotent.
  const allAssertions = await store.allAssertions();
  const sharingProposition = allAssertions.filter(
    (x) => x.propositionId === proposition.id,
  );
  const existing = sharingProposition.find((x) => x.documentId === docId);
  if (existing) {
    const updated: Assertion = {
      ...existing,
      anchor: input.code.length > 0 ? anchor : existing.anchor,
      ref: input.ref,
      enforcement: input.enforcement ?? existing.enforcement,
      verified: input.verified || existing.verified,
      ...(input.verifiers !== undefined && input.verifiers.length > 0
        ? { verifiers: input.verifiers }
        : {}),
      ...(input.ttl !== undefined ? { ttl: input.ttl } : {}),
    };
    await store.putAssertion(updated);
    return {
      document,
      proposition,
      assertion: updated,
      dedupedProposition: deduped,
      existingClaims: sharingProposition
        .filter((x) => x.id !== existing.id)
        .map((x) => x.id),
      warnings,
    };
  }

  const assertion: Assertion = {
    id: newId("asrt"),
    propositionId: proposition.id,
    documentId: docId,
    owner: input.owner,
    ref: input.ref,
    anchor,
    enforcement,
    verified: input.verified,
    verifiers: input.verifiers ?? [],
    ttl: input.ttl,
    attrs: input.attrs ?? {},
  };
  await store.putAssertion(assertion);

  return {
    document,
    proposition,
    assertion,
    dedupedProposition: deduped,
    existingClaims: sharingProposition.map((x) => x.id),
    warnings,
  };
}

/**
 * Record-time doc-quote guard. Throws on a quote shorter than the length floor
 * or one whose context cannot select a single occurrence; returns a warning
 * when the sentence occurs more than once but the context disambiguates it.
 */
export function validateDocQuote(
  docContent: string,
  region: Region,
  docPath: string,
): string[] {
  const quote = regionText(docContent, region);
  if (quote.length < AMBIGUOUS_MIN_QUOTE_LENGTH) {
    throw new Error(
      `doc quote is shorter than ${AMBIGUOUS_MIN_QUOTE_LENGTH} characters, too short to anchor reliably. Record a wider span (--doc-range) that covers the full sentence.`,
    );
  }

  let count = 0;
  for (
    let j = docContent.indexOf(quote);
    j !== -1;
    j = docContent.indexOf(quote, j + quote.length)
  ) {
    count += 1;
  }
  if (count <= 1) return [];

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
      `doc quote occurs ${count} times in ${docPath} and the surrounding context does not select a single occurrence. Record a wider span (--doc-range).`,
    );
  }
  return [
    `doc quote occurs ${count} times in ${docPath}; the stored context selects this occurrence. A wider span (--doc-range) would be more robust.`,
  ];
}

/** Char offset of the first character of a 1-based `line` in `lines`. */
function lineStartOffset(lines: string[], line: number): number {
  let off = 0;
  for (let i = 0; i < line - 1 && i < lines.length; i++)
    off += (lines[i] ?? "").length + 1;
  return off;
}

/** Resolve a region from a quote, char offsets, or an inclusive 1-based line range. */
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
    if (
      !Number.isFinite(spec.startLine) ||
      !Number.isFinite(spec.endLine) ||
      spec.startLine < 1 ||
      spec.endLine < spec.startLine
    )
      throw new Error(
        `Malformed line range: ${spec.startLine}:${spec.endLine}`,
      );
    const lines = content.split("\n");
    if (spec.startLine > lines.length)
      throw new Error(
        `Line ${spec.startLine} is past the end of the file (${lines.length} lines)`,
      );
    const start = lineStartOffset(lines, spec.startLine);
    const end =
      lineStartOffset(lines, spec.endLine) +
      (lines[spec.endLine - 1] ?? "").length;
    return { start, end: Math.max(start, end) };
  }
  throw new Error("A span requires a quote, a char range, or a line range.");
}
