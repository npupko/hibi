/**
 * Recording a claim: write the Document (upserted by path) and the Claim with
 * its doc span, its evidence spans, and one initial `vouch` attestation by the
 * recording actor. The doc span's text is the claim's sentence.
 */

import {
  contextScore,
  exactOccurrences,
  regionText,
} from "../algo/localize.ts";
import { normalizeText } from "../algo/normalize.ts";
import {
  AMBIGUOUS_MIN_QUOTE_LENGTH,
  TEXT_QUOTE_CONTEXT,
} from "../algo/params.ts";
import { documentIdForPath, newClaimId } from "../core/ids.ts";
import {
  ACTOR_PATTERN,
  type Claim,
  type ClaimKind,
  Document,
  DURATION_PATTERN,
  type Enforcement,
  type Procedure,
  type Region,
  type Selector,
  type SelectorBundle,
  type Source,
} from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";
import { type AnchorAnalyzer, buildSelectorBundle } from "./anchor.ts";
import { languageForFile } from "./lang.ts";

export { documentIdForPath };

export function newDocument(id: string, path: string, now: Date): Document {
  return Document.parse({ id, path, createdAt: now.toISOString() });
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

/** An evidence target: a span in a file. The file may be aliased (`api:src/x.ts`, SPEC 4.1). */
export interface EvidenceTarget {
  /** Repo-relative path of the evidence file, or `<alias>:<path>` for a sibling repo. */
  file: string;
  /** The span inside `file`. */
  region: RegionSpec;
  /** Optional language override; else derived from the extension. */
  language?: string;
}

export interface RecordInput {
  docPath: string;
  docSpec?: RegionSpec;
  kind: ClaimKind;
  /** Code spans for `code`, one document span for `reference`, none for `attested`. */
  evidence: EvidenceTarget[];
  sources?: Source[];
  procedure?: Procedure | null;
  owner?: string;
  ttl?: string;
  /** The actor recorded on the initial `vouch` attestation. */
  by: string;
  ref: string;
  /** Default `enforced`. `suggested` never gates. */
  enforcement?: Enforcement;
  analyzer?: AnchorAnalyzer;
  /** Clock for `createdAt` and the attestation time. Default: now. */
  now?: Date;
}

/**
 * The on-disk content of the doc and each referenced evidence file, keyed by
 * the target's `file` as given (an aliased file is keyed by its alias form).
 */
export interface RecordContents {
  docContent: string | null;
  evidenceContents: Record<string, string | null>;
}

export interface RecordResult {
  document: Document;
  claim: Claim;
  /** True when an existing claim on the same sentence in the same document was updated. */
  updated: boolean;
  /** Other claim ids, in other documents, whose sentence is the same text. */
  existingClaims: string[];
  /** Non-fatal notes, e.g. a doc quote that occurs more than once. */
  warnings: string[];
}

function sameSentence(a: string, b: string): boolean {
  return normalizeText(a) === normalizeText(b);
}

/** Build the evidence bundles for a claim kind, enforcing the per-kind shape. */
function evidenceBundles(
  kind: ClaimKind,
  targets: EvidenceTarget[],
  contents: Record<string, string | null>,
  analyzer: AnchorAnalyzer | undefined,
): SelectorBundle[] {
  if (kind === "attested") {
    if (targets.length > 0) {
      throw new Error(
        "an attested claim has no evidence spans; use sources and a procedure to say where its truth lives.",
      );
    }
    return [];
  }
  if (kind === "reference" && targets.length !== 1) {
    throw new Error(
      `a reference claim needs exactly one span in another document (got ${targets.length}).`,
    );
  }
  if (kind === "code" && targets.length === 0) {
    throw new Error(
      "a code claim needs at least one code span (--code-file with --code-quote or --code-range).",
    );
  }
  return targets.map((target) => {
    const content = contents[target.file];
    if (content === undefined || content === null) {
      throw new Error(`Evidence file not found on disk: ${target.file}`);
    }
    const region = resolveRegion(content, target.region);
    // A reference span is located like a doc span: text selectors only.
    return kind === "code"
      ? buildSelectorBundle(target.file, content, region, {
          language: target.language ?? languageForFile(target.file),
          analyzer,
        })
      : buildSelectorBundle(target.file, content, region);
  });
}

export async function recordClaim(
  store: ClaimStore,
  contents: RecordContents,
  input: RecordInput,
): Promise<RecordResult> {
  const { docContent, evidenceContents } = contents;
  const warnings: string[] = [];
  const now = input.now ?? new Date();
  const kind: ClaimKind = input.kind;
  const enforcement: Enforcement = input.enforcement ?? "enforced";

  if (!ACTOR_PATTERN.test(input.by)) {
    throw new Error(
      `the recording actor must be human:<id>, agent:<name>, or process:<name> (got ${JSON.stringify(input.by)}).`,
    );
  }
  if (input.owner !== undefined && !ACTOR_PATTERN.test(input.owner)) {
    throw new Error(
      `owner must be human:<id>, agent:<name>, or process:<name> (got ${JSON.stringify(input.owner)}).`,
    );
  }
  if (input.ttl !== undefined && !DURATION_PATTERN.test(input.ttl)) {
    throw new Error(
      `ttl must be an ISO 8601 duration such as P90D (got ${JSON.stringify(input.ttl)}).`,
    );
  }
  if (kind === "attested" && input.ttl === undefined) {
    throw new Error("an attested claim requires a ttl (e.g. P90D).");
  }

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
  const sentence = regionText(docContent, docRegion);
  warnings.push(...validateDocQuote(docContent, docRegion, input.docPath));

  const evidence = evidenceBundles(
    kind,
    input.evidence,
    evidenceContents,
    input.analyzer,
  );

  // Document (upsert by path).
  const docId = documentIdForPath(input.docPath);
  let document = await store.getDocument(docId);
  if (!document) {
    document = newDocument(docId, input.docPath, now);
    await store.putDocument(document);
  }

  const doc = buildSelectorBundle(input.docPath, docContent, docRegion);
  const vouch = {
    by: input.by,
    at: now.toISOString(),
    basis: "vouch" as const,
  };

  // One claim per (sentence, document): re-recording updates it in place.
  const all = await store.allClaims();
  const sharing = all.filter((c) => sameSentence(c.sentence, sentence));
  const existing = sharing.find((c) => c.documentId === docId);
  if (existing) {
    const updated: Claim = {
      ...existing,
      kind,
      sentence,
      doc,
      evidence,
      ref: input.ref,
      // The same value the per-kind span rules above validated against.
      enforcement,
      ...(input.sources !== undefined ? { sources: input.sources } : {}),
      ...(input.procedure !== undefined ? { procedure: input.procedure } : {}),
      ...(input.owner !== undefined ? { owner: input.owner } : {}),
      ...(input.ttl !== undefined ? { ttl: input.ttl } : {}),
      attestations: [...existing.attestations, vouch],
    };
    await store.putClaim(updated);
    return {
      document,
      claim: updated,
      updated: true,
      existingClaims: sharing
        .filter((c) => c.id !== existing.id)
        .map((c) => c.id),
      warnings,
    };
  }

  const claim: Claim = {
    id: newClaimId(),
    documentId: docId,
    kind,
    sentence,
    doc,
    evidence,
    sources: input.sources ?? [],
    procedure: input.procedure ?? null,
    ...(input.owner !== undefined ? { owner: input.owner } : {}),
    ...(input.ttl !== undefined ? { ttl: input.ttl } : {}),
    enforcement,
    attestations: [vouch],
    createdAt: now.toISOString(),
    ref: input.ref,
  };
  await store.putClaim(claim);

  return {
    document,
    claim,
    updated: false,
    existingClaims: sharing.map((c) => c.id),
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

  const hits = exactOccurrences(docContent, quote);
  const count = hits.length;
  if (count <= 1) return [];

  // Score every occurrence exactly the way `localizeTextQuote` will at check
  // time, so a quote that records clean also resolves to this same occurrence.
  const stored: Extract<Selector, { kind: "text-quote" }> = {
    kind: "text-quote",
    exact: quote,
    prefix: docContent.slice(
      Math.max(0, region.start - TEXT_QUOTE_CONTEXT),
      region.start,
    ),
    suffix: docContent.slice(region.end, region.end + TEXT_QUOTE_CONTEXT),
  };
  const scores = hits.map((at) =>
    contextScore(docContent, at, quote.length, stored),
  );
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
