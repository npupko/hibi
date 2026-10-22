/**
 * The region splitter (SPEC 3): a document becomes an ordered list of regions.
 *
 * - Blocks are separated by blank lines; a fenced block stays one block even
 *   with blank lines inside it, and is one `fenced` region.
 * - Inside a block, a line that opens a heading, list item, blockquote, or
 *   table row starts a new piece.
 * - Each piece splits at a sentence terminator (`.`, `!`, `?`, optionally
 *   followed by a closing quote, paren, or bracket) followed by whitespace.
 *
 * Offsets are string indices (UTF-16 code units), the same unit the anchor
 * resolver and `text-position` selectors use, so spans compare directly.
 */

import type { Region, RegionKind } from "../judge/types.ts";

/** A judge region plus where it sits in the document. */
export type DocRegion = Region & {
  /** 1-based line of the region's first character. */
  line: number;
  /** Start offset, inclusive. */
  start: number;
  /** End offset, exclusive. */
  end: number;
};

interface Span {
  start: number;
  end: number;
}

interface Block extends Span {
  fenced: boolean;
}

interface Piece extends Span {
  kind: RegionKind;
}

const FENCE = /^(`{3,}|~{3,})/;
const LIST_OR_HEADING = /^(?:#{1,6}\s|[-*+]\s|\d+[.)]\s|>\s?|\|)/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const LIST_ITEM = /^(?:[-*+]\s|\d+[.)]\s)/;
/** A sentence end: terminator, optional closing quote/paren, then whitespace. */
const SENTENCE_END = /[.!?]["')\]]?(?=\s)/g;

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

/** Trim a span to its non-whitespace content. */
function tight(text: string, start: number, end: number): Span | null {
  let s = start;
  let e = end;
  while (s < e && /\s/.test(text[s] as string)) s++;
  while (e > s && /\s/.test(text[e - 1] as string)) e--;
  return s < e ? { start: s, end: e } : null;
}

/** The region kind of a piece, from its first line. Blockquotes and prose are sentences. */
function pieceKind(firstLine: string): RegionKind {
  const line = firstLine.trimStart();
  if (HEADING.test(line)) return "heading";
  if (LIST_ITEM.test(line)) return "list-item";
  if (line.startsWith("|")) return "table-row";
  return "sentence";
}

/** Split a prose block into pieces at lines that open a heading, list item, blockquote, or table row. */
function splitPieces(text: string, block: Block): Piece[] {
  if (block.fenced) return [{ ...block, kind: "fenced" }];
  const starts: number[] = [block.start];
  let cursor = block.start;
  while (cursor < block.end) {
    let nl = text.indexOf("\n", cursor);
    if (nl === -1 || nl > block.end) nl = block.end;
    if (
      cursor !== block.start &&
      LIST_OR_HEADING.test(text.slice(cursor, nl).trimStart())
    ) {
      starts.push(cursor);
    }
    cursor = nl + 1;
  }
  return starts.map((start, i) => {
    const end = starts[i + 1] ?? block.end;
    let nl = text.indexOf("\n", start);
    if (nl === -1 || nl > end) nl = end;
    return { start, end, kind: pieceKind(text.slice(start, nl)) };
  });
}

/** Split a piece at sentence ends; a fenced piece stays whole. */
function splitSentences(text: string, piece: Piece): Span[] {
  if (piece.kind === "fenced") return [{ start: piece.start, end: piece.end }];
  const out: Span[] = [];
  const slice = text.slice(piece.start, piece.end);
  let last = 0;
  for (const m of slice.matchAll(SENTENCE_END)) {
    const cut = m.index + m[0].length;
    const r = tight(text, piece.start + last, piece.start + cut);
    if (r) out.push(r);
    last = cut;
  }
  const tail = tight(text, piece.start + last, piece.end);
  if (tail) out.push(tail);
  return out;
}

/** Heading level and text (without `#` markers or a closing `#` run). */
function parseHeading(line: string): { level: number; text: string } | null {
  const m = HEADING.exec(line.trimStart());
  if (!m) return null;
  const marks = m[1] as string;
  const body = (m[2] as string).replace(/\s+#+\s*$/, "").trim();
  return { level: marks.length, text: body };
}

/**
 * The regions of a document, in order. Ids are `r<index>`. `headingPath` is
 * the stack of Markdown headings above the region; a heading's own path holds
 * its ancestors, not itself.
 */
export function splitRegions(text: string): DocRegion[] {
  const out: DocRegion[] = [];
  const stack: { level: number; text: string }[] = [];
  let line = 1;
  let lineCursor = 0;
  const lineAt = (offset: number): number => {
    for (; lineCursor < offset; lineCursor++) {
      if (text[lineCursor] === "\n") line++;
    }
    return line;
  };
  for (const block of splitBlocks(text)) {
    for (const piece of splitPieces(text, block)) {
      let heading: { level: number; text: string } | null = null;
      if (piece.kind === "heading") {
        heading = parseHeading(
          text.slice(piece.start, piece.end).split("\n")[0] ?? "",
        );
        if (heading) {
          const level = heading.level;
          while (stack.length > 0 && (stack.at(-1)?.level ?? 0) >= level) {
            stack.pop();
          }
        }
      }
      const headingPath = stack.map((h) => h.text);
      for (const span of splitSentences(text, piece)) {
        out.push({
          id: `r${out.length}`,
          kind: piece.kind,
          text: text.slice(span.start, span.end),
          headingPath: [...headingPath],
          line: lineAt(span.start),
          start: span.start,
          end: span.end,
        });
      }
      if (heading) stack.push(heading);
    }
  }
  return out;
}
