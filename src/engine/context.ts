/**
 * The `claim_context` input of the `supports` judgment (SPEC 8.3): where the
 * sentence sits in its document. Deterministic and bounded:
 *
 *   1. the heading path: the Markdown `#` headings above the sentence, outer
 *      to inner, joined with " > " (headings inside fenced blocks are skipped);
 *   2. a blank line, then the paragraph containing the sentence: the run of
 *      lines around it up to a blank line, a heading, or a fence line.
 *
 * The whole is at most `CONTEXT_MAX_CHARS` characters. The sentence is never
 * cut: the paragraph is trimmed around it first (cut ends marked with "…"),
 * then the heading path from its outer end. A sentence longer than the limit
 * is returned alone.
 */

import type { Claim } from "../core/model.ts";

export const CONTEXT_MAX_CHARS = 600;

const ELLIPSIS = "…";
const HEADING = /^ {0,3}(#{1,6})[ \t]+(.*?)[ \t]*#*[ \t]*$/;
const FENCE = /^ {0,3}(```|~~~)/;

interface Line {
  text: string;
  start: number;
  fenced: boolean;
  heading?: { level: number; title: string };
}

function scanLines(doc: string): Line[] {
  const out: Line[] = [];
  let start = 0;
  let inFence = false;
  for (const text of doc.split("\n")) {
    const fenceLine = FENCE.test(text);
    const fenced = inFence || fenceLine;
    const h = !fenced ? HEADING.exec(text) : null;
    out.push({
      text,
      start,
      fenced,
      ...(h
        ? { heading: { level: (h[1] as string).length, title: h[2] ?? "" } }
        : {}),
    });
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
  if (line.text.trim() === "") return true;
  if (line.heading) return true;
  // A paragraph does not cross a fence boundary in either direction.
  return line.fenced !== fenced || FENCE.test(line.text);
}

/** [start, end) offsets of the paragraph holding the sentence's first and last lines. */
function paragraphBounds(
  lines: Line[],
  first: number,
  last: number,
  doc: string,
): [number, number] {
  const fenced = (lines[first] as Line).fenced;
  let a = first;
  while (a > 0 && !isBreak(lines[a - 1] as Line, fenced)) a -= 1;
  let b = last;
  while (b < lines.length - 1 && !isBreak(lines[b + 1] as Line, fenced)) b += 1;
  const end = Math.min(
    doc.length,
    (lines[b] as Line).start + (lines[b] as Line).text.length,
  );
  return [(lines[a] as Line).start, end];
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

  const lines = scanLines(docText);
  const first = lineIndexAt(lines, at);
  const last = lineIndexAt(lines, at + Math.max(0, sentence.length - 1));
  let path = headingPath(lines, first).join(" > ");

  const [pStart, pEnd] = paragraphBounds(lines, first, last, docText);
  const before = docText.slice(pStart, at).replace(/^\s+/, "");
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
  const lead = path ? path + sep : "";
  return (
    lead + window(before, sentence, after, CONTEXT_MAX_CHARS - lead.length)
  );
}
