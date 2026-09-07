/**
 * `coverage`: which regions of a document are backed by a tracked claim and
 * which are not. The uncovered regions are the candidates for grounding or
 * removal; hibi reports the structural fact and never judges a region.
 *
 * A region is covered iff a live, code-grounded claim's doc anchor resolves
 * cleanly (`unchanged`/`moved`) into it. Prose is split at sentence level;
 * fenced code blocks stay whole; headings and list items are their own regions.
 */

import type { AstAnalyzer } from "../algo/resolve.ts";
import { resolveSide } from "../algo/resolve.ts";
import type { Region } from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";

export interface CoverageInput {
  docPath: string;
}

export interface CoverageRegion {
  /**
   * Char span in the banner-normalized document, the coordinate space every
   * stored doc anchor uses. Ground a region by quoting its text, never by
   * slicing the raw file at these offsets.
   */
  range: Region;
  preview: string;
  covered: boolean;
  claimIds: string[];
}

export interface CoverageSummary {
  regions: number;
  covered: number;
  uncovered: number;
  /** `covered / regions`; 0 when the document has no regions. */
  coverageRatio: number;
}

export interface CoverageResult {
  regions: CoverageRegion[];
  summary: CoverageSummary;
}

export interface CoverageOptions {
  ast?: AstAnalyzer;
}

const PREVIEW_MAX = 96;

function preview(s: string): string {
  const flat = s.replace(/\s+/g, " ").trim();
  return flat.length > PREVIEW_MAX
    ? `${flat.slice(0, PREVIEW_MAX - 1)}…`
    : flat;
}

const FENCE = /^(`{3,}|~{3,})/;
const LIST_OR_HEADING = /^(?:#{1,6}\s|[-*+]\s|\d+[.)]\s|>\s?|\|)/;
/** A sentence end: terminator, optional closing quote/paren, then whitespace. */
const SENTENCE_END = /[.!?]["')\]]?(?=\s)/g;

interface Block {
  start: number;
  end: number;
  fenced: boolean;
}

/**
 * Segment `text` into blocks on blank-line boundaries; a fence stays one block
 * even with blank lines inside it.
 */
function splitBlocks(text: string): Block[] {
  const out: Block[] = [];
  let start = -1;
  let end = -1;
  let fenceChar = "";
  let fenced = false;
  let cursor = 0;
  const len = text.length;
  while (cursor <= len) {
    let nl = text.indexOf("\n", cursor);
    if (nl === -1) nl = len;
    const line = text.slice(cursor, nl);
    const trimmed = line.trim();
    const fence = FENCE.exec(trimmed);
    if (fence) {
      const marker = fence[0][0] as string;
      if (fenceChar === "") {
        fenceChar = marker;
        fenced = true;
      } else if (marker === fenceChar) {
        fenceChar = "";
      }
    }
    if (trimmed.length === 0 && fenceChar === "") {
      if (start >= 0) {
        out.push({ start, end, fenced });
        start = -1;
        fenced = false;
      }
    } else {
      if (start < 0) start = cursor + (line.length - line.trimStart().length);
      end = cursor + line.trimEnd().length;
    }
    if (nl === len) break;
    cursor = nl + 1;
  }
  if (start >= 0) out.push({ start, end, fenced });
  return out;
}

/** Trim a region to its non-whitespace content. */
function tight(text: string, start: number, end: number): Region | null {
  let s = start;
  let e = end;
  while (s < e && /\s/.test(text[s] as string)) s++;
  while (e > s && /\s/.test(text[e - 1] as string)) e--;
  return s < e ? { start: s, end: e } : null;
}

/**
 * Split a prose block into sentence regions. Lines that open a heading, list
 * item, blockquote, or table row start a new region; inside a run of prose
 * lines, sentence terminators followed by whitespace end a region.
 */
function splitSentences(text: string, block: Block): Region[] {
  if (block.fenced) return [{ start: block.start, end: block.end }];
  const out: Region[] = [];
  let cursor = block.start;
  // First, split on structural line starts.
  const pieces: Region[] = [];
  let pieceStart = block.start;
  while (cursor < block.end) {
    let nl = text.indexOf("\n", cursor);
    if (nl === -1 || nl > block.end) nl = block.end;
    if (cursor !== block.start) {
      const lineText = text.slice(cursor, nl);
      if (LIST_OR_HEADING.test(lineText.trimStart())) {
        pieces.push({ start: pieceStart, end: cursor });
        pieceStart = cursor;
      }
    }
    cursor = nl + 1;
  }
  pieces.push({ start: pieceStart, end: block.end });
  // Then split each piece at sentence ends.
  for (const piece of pieces) {
    const slice = text.slice(piece.start, piece.end);
    let last = 0;
    SENTENCE_END.lastIndex = 0;
    let m = SENTENCE_END.exec(slice);
    while (m) {
      const cut = m.index + m[0].length;
      const r = tight(text, piece.start + last, piece.start + cut);
      if (r) out.push(r);
      last = cut;
      m = SENTENCE_END.exec(slice);
    }
    const tail = tight(text, piece.start + last, piece.end);
    if (tail) out.push(tail);
  }
  return out;
}

/** Sentence-level regions of a document. Exported for tests. */
export function splitRegions(text: string): Region[] {
  return splitBlocks(text).flatMap((b) => splitSentences(text, b));
}

function overlaps(r: Region, s: Region): boolean {
  return r.start < s.end && r.end > s.start;
}

export async function coverage(
  store: ClaimStore,
  docContent: string,
  input: CoverageInput,
  opts: CoverageOptions = {},
): Promise<CoverageResult> {
  const assertions = await store.allAssertions();
  const spans: { claimId: string; region: Region }[] = [];
  for (const a of assertions) {
    if (a.enforcement === "retired") continue;
    if (a.anchor.doc.file !== input.docPath) continue;
    if (a.anchor.code.length === 0) continue;
    const side = resolveSide(a.anchor.doc, docContent, { ast: opts.ast });
    if (!side.region) continue;
    if (side.state !== "unchanged" && side.state !== "moved") continue;
    spans.push({ claimId: a.id, region: side.region });
  }

  const regions: CoverageRegion[] = splitRegions(docContent).map((r) => {
    const claimIds = spans
      .filter((s) => overlaps(s.region, r))
      .map((s) => s.claimId);
    return {
      range: r,
      preview: preview(docContent.slice(r.start, r.end)),
      covered: claimIds.length > 0,
      claimIds,
    };
  });

  const covered = regions.filter((r) => r.covered).length;
  const total = regions.length;
  return {
    regions,
    summary: {
      regions: total,
      covered,
      uncovered: total - covered,
      coverageRatio: total === 0 ? 0 : covered / total,
    },
  };
}
