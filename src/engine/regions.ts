/**
 * The region splitter (SPEC 3): a document becomes an ordered list of regions.
 *
 * 1. Frontmatter: in a `.md` or `.mdx` file whose first line is `---`, the
 *    lines up to and including the next `---` line are skipped. They are no
 *    region. Without a closing `---` line there is no frontmatter.
 * 2. Blocks are separated by blank lines. A line that is only a JSX or HTML
 *    tag (`<Card title="x">`, `</Card>`, `<Tabs>`, `<br/>`) also separates
 *    blocks and is no region, and so does a thematic break (a line that is
 *    only `---`, `***` or `___`, three or more characters, outside a fence
 *    and not frontmatter). A fenced block stays one block even with blank
 *    lines or tag lines inside it, and is one `fenced` region.
 * 3. Inside a block, a line that opens a heading, list item, blockquote, or
 *    table row starts a new piece, and so does the line after a heading: a
 *    heading is always one line.
 * 4. A table separator row (`|---|:---:|`) is no region.
 * 5. A heading piece is one region; it is not split into sentences.
 * 6. Every other piece splits at a sentence terminator (`.`, `!`, `?`,
 *    optionally followed by a closing quote, paren, or bracket) followed by
 *    whitespace, except: a terminator inside an inline code span (a backtick
 *    run up to the next run of the same length, which may cross lines) does
 *    not split, and in a list item the marker (`-`, `*`, `+`, `1.`, `1)`)
 *    and the whitespace after it do not split, so the marker stays with the
 *    first sentence. In a table row, a terminator followed only by
 *    whitespace and `|` to the end of the row does not split, so the closing
 *    pipe stays with the last sentence.
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
/** A list marker and the whitespace after it, at the start of a trimmed line. */
const LIST_MARKER = /^(?:[-*+]|\d+[.)])\s+/;
/** A line that is only one JSX or HTML tag: `<Card title="x">`, `</Card>`, `<br/>`. */
export const TAG_LINE = /^<\/?[A-Za-z][\w.:-]*(?:\s[^<>]*)?\/?>$/;
/** A thematic break: a line that is only `---`, `***` or `___` (three or more). */
export const THEMATIC_BREAK = /^(?:-{3,}|\*{3,}|_{3,})$/;
/** A table separator row: cells of dashes with optional alignment colons. */
const TABLE_SEPARATOR = /^\|?(?:\s*:?-+:?\s*\|)+\s*(?::?-+:?\s*)?$/;
/** A sentence end: terminator, optional closing quote/paren, then whitespace. */
const SENTENCE_END = /[.!?]["')\]]?(?=\s)/g;

/**
 * Segment `text` into blocks on blank-line boundaries; a fence stays one block
 * even with blank lines inside it.
 */
function splitBlocks(text: string, from: number): Block[] {
  const out: Block[] = [];
  let start = -1;
  let end = -1;
  let fenceChar = "";
  let fenced = false;
  let cursor = from;
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
    const boundary =
      fenceChar === "" &&
      (trimmed.length === 0 ||
        TAG_LINE.test(trimmed) ||
        THEMATIC_BREAK.test(trimmed));
    if (boundary) {
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

/**
 * Where the body of a `.md` or `.mdx` file starts: after its frontmatter (a
 * first line `---` through the next `---` line), else 0. Shared with the
 * judge context builder so neither treats frontmatter as text.
 */
export function frontmatterEnd(text: string, path?: string): number {
  if (path === undefined || !/\.mdx?$/i.test(path)) return 0;
  const first = text.indexOf("\n");
  if (first < 0 || text.slice(0, first).trimEnd() !== "---") return 0;
  let cursor = first + 1;
  while (cursor <= text.length) {
    let nl = text.indexOf("\n", cursor);
    if (nl === -1) nl = text.length;
    if (text.slice(cursor, nl).trimEnd() === "---")
      return Math.min(nl + 1, text.length);
    if (nl === text.length) break;
    cursor = nl + 1;
  }
  return 0;
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
    const line = text.slice(cursor, nl).trimStart();
    if (cursor !== block.start && LIST_OR_HEADING.test(line)) {
      if (starts.at(-1) !== cursor) starts.push(cursor);
    }
    // A heading is one line: the next line opens a new piece.
    if (HEADING.test(line) && nl < block.end) starts.push(nl + 1);
    cursor = nl + 1;
  }
  const out: Piece[] = [];
  starts.forEach((start, i) => {
    const end = starts[i + 1] ?? block.end;
    let nl = text.indexOf("\n", start);
    if (nl === -1 || nl > end) nl = end;
    const first = text.slice(start, nl).trim();
    if (TABLE_SEPARATOR.test(first)) return;
    out.push({ start, end, kind: pieceKind(first) });
  });
  return out;
}

/**
 * [start, end) offsets, relative to `slice`, of its inline code spans: a run
 * of n backticks up to the next run of exactly n backticks. A run with no
 * closing run is literal text.
 */
function codeSpans(slice: string): Span[] {
  const out: Span[] = [];
  const runs = [...slice.matchAll(/`+/g)].map((m) => ({
    at: m.index,
    n: m[0].length,
  }));
  for (let i = 0; i < runs.length; i++) {
    const open = runs[i] as { at: number; n: number };
    const close = runs.findIndex((r, j) => j > i && r.n === open.n);
    if (close < 0) continue;
    const c = runs[close] as { at: number; n: number };
    out.push({ start: open.at, end: c.at + c.n });
    i = close;
  }
  return out;
}

/**
 * Split a piece at sentence ends. A fenced or heading piece stays whole; a
 * terminator inside an inline code span or inside a list marker does not split.
 */
function splitSentences(text: string, piece: Piece): Span[] {
  if (piece.kind === "fenced" || piece.kind === "heading") {
    const whole = tight(text, piece.start, piece.end);
    return whole ? [whole] : [];
  }
  const out: Span[] = [];
  const slice = text.slice(piece.start, piece.end);
  const code = codeSpans(slice);
  let markerEnd = 0;
  if (piece.kind === "list-item") {
    const indent = slice.length - slice.trimStart().length;
    markerEnd =
      indent + (LIST_MARKER.exec(slice.slice(indent))?.[0].length ?? 0);
  }
  let last = 0;
  for (const m of slice.matchAll(SENTENCE_END)) {
    if (m.index < markerEnd) continue;
    if (code.some((c) => m.index >= c.start && m.index < c.end)) continue;
    const cut = m.index + m[0].length;
    // A table row's closing pipe stays with the row's last sentence.
    if (piece.kind === "table-row" && /^[\s|]*$/.test(slice.slice(cut))) {
      continue;
    }
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
export function splitRegions(text: string, path?: string): DocRegion[] {
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
  for (const block of splitBlocks(text, frontmatterEnd(text, path))) {
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
