/**
 * `suggest` (§9) — deterministically propose anchorable claims from a document.
 *
 * A read-only scan of the doc text that surfaces **atomic, anchorable,
 * verifiable** sentences and records each as a `suggested`, doc-side-only claim
 * for an author to later confirm and point at code (via `reanchor`). The scan is
 * pure pattern-matching — the engine never NLP-extracts meaning (D2); it only
 * picks sentences that *look* checkable (RFC-2119 normative verbs, backticked
 * identifiers, numeric/literal defaults, code/CLI examples) and skips prose that
 * reads as rationale, opinion, or background.
 *
 * A suggested record is doc-anchored only (`code: []`), `authoredTrust:
 * "inferred"`, `enforcement: "suggested"` — so it never gates and never claims a
 * code target it has not earned (§4/§9). Dedup is by the proposition fingerprint,
 * handled in `recordClaim`.
 */

import type { ClaimStore } from "../store/store.ts";
import {
  type RecordContents,
  type RecordResult,
  recordClaim,
} from "./record.ts";

export interface SuggestInput {
  /** Repo-relative path of the document to scan. */
  docPath: string;
  /** Reserved: a git ref the caller scopes the scan to (line selection by caller). */
  since?: string;
}

export interface SuggestResult {
  created: RecordResult[];
}

/** A located candidate sentence and the char span it occupies in the doc. */
interface Candidate {
  text: string;
  start: number;
  end: number;
}

/** RFC-2119 normative keywords — a sentence carrying one states a checkable rule. */
const NORMATIVE =
  /\b(MUST|MUST NOT|SHALL|SHALL NOT|SHOULD|SHOULD NOT|REQUIRED)\b/;

/** A backticked code identifier / inline code span. */
const BACKTICKED = /`[^`\n]+`/;

/** A numeric or literal default a claim can be anchored to (e.g. "default 2", "5ms"). */
const NUMERIC_LITERAL = /\b\d+(?:\.\d+)?\b/;

/**
 * Background / opinion / rationale openers to skip — these read as prose about
 * *why*, not a checkable rule about *what*. Conservative on purpose: a missed
 * suggestion is cheaper than a noisy one.
 */
const RATIONALE_OPENER =
  /^(?:because|this is|we believe|in our (?:opinion|view)|historically|for context|note that|ideally|arguably|in theory|the rationale|originally)\b/i;

/**
 * Split `text` into sentence-like candidates, preserving each one's char span so
 * the doc-side anchor can localize it later. Splits on sentence terminators
 * (`.`/`!`/`?`) followed by whitespace, and on hard line breaks, so list items
 * and headings each become their own candidate.
 */
function splitSentences(text: string): Candidate[] {
  const out: Candidate[] = [];
  // Boundaries: a terminator + whitespace, or one-or-more newlines.
  const boundary = /(?<=[.!?])\s+|\n+/g;
  let cursor = 0;
  let m: RegExpExecArray | null = boundary.exec(text);
  const push = (start: number, end: number) => {
    const slice = text.slice(start, end);
    const trimmedStart = start + (slice.length - slice.trimStart().length);
    const trimmedEnd = end - (slice.length - slice.trimEnd().length);
    if (trimmedEnd > trimmedStart) {
      out.push({
        text: text.slice(trimmedStart, trimmedEnd),
        start: trimmedStart,
        end: trimmedEnd,
      });
    }
  };
  while (m !== null) {
    push(cursor, m.index);
    cursor = m.index + m[0].length;
    m = boundary.exec(text);
  }
  push(cursor, text.length);
  return out;
}

/** Strip leading Markdown markers (list bullets, headings, blockquotes). */
function stripMarkup(s: string): string {
  return s.replace(/^[\s>#*\-+]+/, "").replace(/^\d+[.)]\s+/, "");
}

/**
 * Is this sentence an atomic, anchorable, verifiable candidate? A checkable rule
 * carries a normative verb, a backticked identifier, a numeric/literal default,
 * or a fenced/CLI example — and does not open as rationale or opinion.
 */
function isCandidate(text: string): boolean {
  const body = stripMarkup(text).trim();
  if (body.length === 0) return false;
  if (RATIONALE_OPENER.test(body)) return false;

  const hasCode = BACKTICKED.test(body);
  const hasNormative = NORMATIVE.test(body);
  const hasNumericLiteral = NUMERIC_LITERAL.test(body);

  // A bare number alone (no code and no rule) is too weak — require it to ride
  // alongside a normative verb or a code identifier to count as a default.
  if (hasNormative) return true;
  if (hasCode) return true;
  if (
    hasNumericLiteral &&
    /\b(default|defaults|limit|timeout|max|min|retr)/i.test(body)
  ) {
    return true;
  }
  return false;
}

/**
 * Scan `docContent` for candidate sentences and record each as a `suggested`,
 * doc-side-only claim. Deterministic and side-effecting only through the store;
 * dedup (by fingerprint) lives in `recordClaim`.
 */
export async function suggest(
  store: ClaimStore,
  docContent: string,
  input: SuggestInput,
): Promise<SuggestResult> {
  const contents: RecordContents = { docContent, codeContents: {} };
  const created: RecordResult[] = [];

  for (const cand of splitSentences(docContent)) {
    if (!isCandidate(cand.text)) continue;
    const result = await recordClaim(store, contents, {
      docPath: input.docPath,
      docSpec: { start: cand.start, end: cand.end },
      authoredTrust: "inferred",
      owner: "unknown",
      ref: "WORKTREE",
      code: [],
      enforcement: "suggested",
    });
    // A re-run over an unchanged doc dedups against the existing proposition;
    // surface only the genuinely new suggestions.
    if (!result.dedupedProposition) created.push(result);
  }

  return { created };
}
