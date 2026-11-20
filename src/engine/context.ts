/**
 * The `claim_context` input of the `supports` judgment (SPEC 8.3): where the
 * sentence sits in its document. Deterministic and bounded:
 *
 *   1. the heading path: the Markdown `#` headings above the sentence, outer
 *      to inner, joined with " > " (headings inside fenced blocks are skipped);
 *   2. a blank line, then the paragraph containing the sentence: the run of
 *      lines around it up to a blank line, a heading, a fence line, or a
 *      thematic break (a line that is only `---`, `***` or `___`, three or
 *      more characters, outside a fence).
 *
 * Frontmatter of a `.md` or `.mdx` file (a first line `---` through the next
 * `---` line, see `frontmatterEnd`) is never context: its lines are not
 * headings and a paragraph stops at them. The file is `claim.doc.file`.
 *
 * Lead-in rule (list items). When the paragraph's first line opens a list
 * item (`-`, `*`, `+`, `1.` or `1)` and a space), the paragraph is a list and
 * its lead-in joins the context ahead of it. The lead-in is found this way:
 * from the line above the list, skip one or more blank lines; the line
 * reached must not be a heading, a fence line, a fenced line, a frontmatter
 * line, a thematic break, or a line that is only a JSX or HTML tag. The
 * lead-in is that line's paragraph (the run of lines above it up to a blank
 * line, a heading, a fence line, or a thematic break), and it counts only when its first line does not open a list
 * item and does not start with `|`. Otherwise there is no lead-in. The
 * lead-in text, trimmed, then a blank line, is placed directly before the
 * list text that precedes the sentence; together they are the text before
 * the sentence, and the 600-character bound trims them the same way (from
 * the start, so the lead-in gives way before the list items nearest the
 * sentence). A sentence whose paragraph does not open with a list item has
 * no lead-in: its context is unchanged.
 *
 * The whole is at most `CONTEXT_MAX_CHARS` characters. The sentence is never
 * cut: the paragraph is trimmed around it first (cut ends marked with "…"),
 * then the heading path from its outer end. A sentence longer than the limit
 * is returned alone.
 */

import type { Claim } from "../core/model.ts";
import { frontmatterEnd, TAG_LINE, THEMATIC_BREAK } from "./regions.ts";

export const CONTEXT_MAX_CHARS = 600;

const ELLIPSIS = "…";
/** An ATX heading opener: up to 3 spaces, 1-6 `#`, then a space or tab. */
const HEADING_OPEN = /^ {0,3}(#{1,6})[ \t]/;
/** Characters `.` does not match; a heading line holding one is not a heading. */
const LINE_TERMINATOR = /[\n\r\u2028\u2029]/;
const FENCE = /^ {0,3}(```|~~~)/;
const LIST_ITEM = /^\s*(?:[-*+]|\d+[.)])\s/;

interface Line {
  text: string;
  start: number;
  fenced: boolean;
  /** Inside the frontmatter. */
  front: boolean;
  heading?: { level: number; title: string };
}

function isSpaceOrTab(code: number): boolean {
  return code === 0x20 || code === 0x09;
}

/**
 * The level and title of an ATX heading line, or null. The title drops the
 * leading and trailing spaces and tabs and any run of `#` at its end, so
 * `## Title ##` and `# C#` give `Title` and `C`. Scans the line once.
 */
export function parseHeading(
  text: string,
): { level: number; title: string } | null {
  const open = HEADING_OPEN.exec(text);
  if (!open) return null;
  let start = open[0].length;
  if (LINE_TERMINATOR.test(text.slice(start))) return null;
  while (start < text.length && isSpaceOrTab(text.charCodeAt(start))) start++;
  let end = text.length;
  while (end > start && isSpaceOrTab(text.charCodeAt(end - 1))) end--;
  while (end > start && text.charCodeAt(end - 1) === 0x23) end--;
  while (end > start && isSpaceOrTab(text.charCodeAt(end - 1))) end--;
  return { level: (open[1] as string).length, title: text.slice(start, end) };
}

function scanLines(doc: string, bodyStart: number): Line[] {
  const out: Line[] = [];
  let start = 0;
  let inFence = false;
  for (const text of doc.split("\n")) {
    const front = start < bodyStart;
    const fenceLine = !front && FENCE.test(text);
    const fenced = inFence || fenceLine;
    const heading = !fenced && !front ? parseHeading(text) : null;
    out.push({ text, start, fenced, front, ...(heading ? { heading } : {}) });
    if (fenceLine) inFence = !inFence;
    start += text.length + 1;
  }
  return out;
}

/** Offset of the sentence in `doc`, nearest the recorded position when it occurs more than once. */
function locate(
  doc: string,
  claim: Pick<Claim, "sentence"> & { doc?: Claim["doc"] },
): number {
  const hint = claim.doc?.selectors.find((s) => s.kind === "text-position");
  const near = hint?.kind === "text-position" ? hint.start : 0;
  let best = -1;
  for (
    let i = doc.indexOf(claim.sentence);
    i >= 0;
    i = doc.indexOf(claim.sentence, i + 1)
  ) {
    if (best < 0 || Math.abs(i - near) < Math.abs(best - near)) best = i;
  }
  return best;
}

function lineIndexAt(lines: Line[], offset: number): number {
  let idx = 0;
  for (let i = 0; i < lines.length; i++) {
    if ((lines[i] as Line).start <= offset) idx = i;
    else break;
  }
  return idx;
}

function headingPath(lines: Line[], upTo: number): string[] {
  const stack: { level: number; title: string }[] = [];
  for (let i = 0; i < upTo; i++) {
    const h = (lines[i] as Line).heading;
    if (!h) continue;
    while (
      stack.length > 0 &&
      (stack[stack.length - 1] as { level: number }).level >= h.level
    )
      stack.pop();
    stack.push(h);
  }
  return stack.map((h) => h.title);
}

function isBreak(line: Line, fenced: boolean): boolean {
  if (line.front || line.text.trim() === "") return true;
  // A thematic break (`---`, `***`, `___`) outside a fence ends a paragraph.
  if (!line.fenced && THEMATIC_BREAK.test(line.text.trim())) return true;
  if (line.heading) return true;
  // A paragraph does not cross a fence boundary in either direction.
  return line.fenced !== fenced || FENCE.test(line.text);
}

/** [first, last] line indices of the paragraph holding lines `first` to `last`. */
function paragraphLines(
  lines: Line[],
  first: number,
  last: number,
): [number, number] {
  const fenced = (lines[first] as Line).fenced;
  let a = first;
  while (a > 0 && !isBreak(lines[a - 1] as Line, fenced)) a -= 1;
  let b = last;
  while (b < lines.length - 1 && !isBreak(lines[b + 1] as Line, fenced)) b += 1;
  return [a, b];
}

/** End offset (exclusive) of line `i`'s text. */
function lineEnd(lines: Line[], i: number, doc: string): number {
  const line = lines[i] as Line;
  return Math.min(doc.length, line.start + line.text.length);
}

/** The lead-in of a list whose first line is `a`, or "" (see the lead-in rule above). */
function leadIn(lines: Line[], a: number, doc: string): string {
  const head = lines[a] as Line;
  if (head.fenced || !LIST_ITEM.test(head.text)) return "";
  let k = a - 1;
  while (
    k >= 0 &&
    !(lines[k] as Line).front &&
    (lines[k] as Line).text.trim() === ""
  ) {
    k -= 1;
  }
  if (k < 0 || k === a - 1) return "";
  const above = lines[k] as Line;
  if (isBreak(above, false) || TAG_LINE.test(above.text.trim())) return "";
  const [c] = paragraphLines(lines, k, k);
  const top = (lines[c] as Line).text;
  if (LIST_ITEM.test(top) || top.trimStart().startsWith("|")) return "";
  return doc.slice((lines[c] as Line).start, lineEnd(lines, k, doc)).trim();
}

/** Trim `before` + sentence + `after` to `budget` chars, keeping the sentence whole. */
function window(
  before: string,
  sentence: string,
  after: string,
  budget: number,
): string {
  if (before.length + sentence.length + after.length <= budget) {
    return before + sentence + after;
  }
  if (budget - sentence.length < 2 * ELLIPSIS.length) return sentence;
  // Reserve one ellipsis per side; the sides share what is left, nearest text first.
  const room = Math.max(0, budget - sentence.length - 2 * ELLIPSIS.length);
  let keepBefore = Math.min(before.length, Math.floor(room / 2));
  const keepAfter = Math.min(after.length, room - keepBefore);
  keepBefore = Math.min(before.length, room - keepAfter);
  const head =
    keepBefore < before.length
      ? ELLIPSIS + before.slice(before.length - keepBefore)
      : before;
  const tail =
    keepAfter < after.length ? after.slice(0, keepAfter) + ELLIPSIS : after;
  return head + sentence + tail;
}

/** The bounded context string for a claim's sentence in `docText`. */
export function claimContext(
  docText: string,
  claim: Pick<Claim, "sentence"> & { doc?: Claim["doc"] },
): string {
  const sentence = claim.sentence;
  if (sentence.length >= CONTEXT_MAX_CHARS) return sentence;
  const at = locate(docText, claim);
  if (at < 0) return sentence;

  const lines = scanLines(docText, frontmatterEnd(docText, claim.doc?.file));
  const first = lineIndexAt(lines, at);
  const last = lineIndexAt(lines, at + Math.max(0, sentence.length - 1));
  let path = headingPath(lines, first).join(" > ");

  const [a, b] = paragraphLines(lines, first, last);
  const pStart = (lines[a] as Line).start;
  const pEnd = lineEnd(lines, b, docText);
  const lead = leadIn(lines, a, docText);
  const listBefore = docText.slice(pStart, at).replace(/^\s+/, "");
  const before = lead ? `${lead}\n\n${listBefore}` : listBefore;
  const after = docText
    .slice(at + sentence.length, Math.max(pEnd, at + sentence.length))
    .replace(/\s+$/, "");

  // The heading path gives way to the sentence, from its outer end.
  const sep = "\n\n";
  const maxPath = CONTEXT_MAX_CHARS - sentence.length - sep.length;
  if (path.length > maxPath) {
    path =
      maxPath > ELLIPSIS.length
        ? ELLIPSIS + path.slice(path.length - (maxPath - ELLIPSIS.length))
        : "";
  }
  const top = path ? path + sep : "";
  return top + window(before, sentence, after, CONTEXT_MAX_CHARS - top.length);
}
