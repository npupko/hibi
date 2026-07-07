/**
 * `coverage` (§9) — deterministic doc-side coverage: which regions of a document
 * are backed by a tracked claim, and which are not.
 *
 * The inverse of `query --path <doc>` (which lists the claims that live on a doc):
 * `coverage` segments the document into blocks and marks each one **covered** iff a
 * live, code-grounded claim's doc-side anchor *cleanly* resolves into it. It reports
 * a **structural fact** (a block is grounded to code, or it isn't) — it never judges
 * whether an uncovered block *should* carry a claim. That judgment (ground it, or
 * prune it as ungrounded prose) is the agent's; this command just hands the agent
 * the worklist and a grounding ratio that climbs as the doc is grounded or trimmed.
 *
 * "Covered" is deliberately strict: a claim counts only when (a) it pins code
 * (`anchor.code` non-empty — a doc-only `suggested` placeholder grounds nothing) and
 * (b) its doc anchor resolves cleanly to `unchanged`/`moved` (a drifted `changed`/
 * `ambiguous`/`orphaned` anchor is `check`'s concern, and its region may be a stale
 * text-position fallback that would inflate the ratio or mis-attribute the claim to
 * the wrong block — §17.3). Drift surfacing belongs to `check`, not here.
 *
 * Deterministic and read-only: it resolves stored doc anchors against the current
 * doc text via the same `resolveSide` machinery `check` uses, and segments blocks
 * by blank-line boundaries (no sentence-splitting — block is the unit).
 */

import type { AstAnalyzer } from "../algo/resolve.ts";
import { resolveSide } from "../algo/resolve.ts";
import type { Region } from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";

export interface CoverageInput {
  /** Repo-relative path of the document to measure. */
  docPath: string;
}

/** One document block and whether a claim's doc anchor lands in it. */
export interface CoverageRegion {
  /**
   * Char span of the block in the **banner-normalized** document — the same
   * coordinate space every stored doc anchor uses (the HIBI banner is stripped
   * before measuring, §8). These offsets do *not* index the raw on-disk file when
   * it carries a banner; ground a block by quoting its `preview`/sentence text via
   * `record --from-file`, never by byte-slicing the raw file at `range`.
   */
  range: Region;
  /** A collapsed one-line preview of the block, to identify it without a re-read. */
  preview: string;
  /** True iff a live, code-grounded claim's doc anchor resolves cleanly into this block. */
  covered: boolean;
  /**
   * True iff this block is (or contains) a fenced code block whose info string is
   * in {@link EXECUTABLE_INFO_STRINGS}. A structural fact, not a judgment: an
   * uncovered executable block is the highest-value grounding target because it
   * can carry a `command:` verifier and reach `supported`/`refuted` (§17.6, D13) —
   * `coverage` flags it, but never creates a claim (creation stays explicit, D2/D21).
   */
  executable: boolean;
  /** Claim ids whose doc anchor overlaps this block (empty when uncovered). */
  claimIds: string[];
}

export interface CoverageSummary {
  blocks: number;
  coveredBlocks: number;
  uncoveredBlocks: number;
  /** `coveredBlocks / blocks` — 0 when the document has no blocks. */
  coverageRatio: number;
}

export interface CoverageResult {
  regions: CoverageRegion[];
  summary: CoverageSummary;
}

export interface CoverageOptions {
  /** Tier-2 analyzer, threaded through to doc-anchor resolution (optional). */
  ast?: AstAnalyzer;
}

/** A located, claimed span in the current doc text. */
interface ClaimSpan {
  claimId: string;
  region: Region;
}

const PREVIEW_MAX = 96;

/** Collapse whitespace and truncate, so a block reads as one identifying line. */
function preview(s: string): string {
  const flat = s.replace(/\s+/g, " ").trim();
  return flat.length > PREVIEW_MAX
    ? `${flat.slice(0, PREVIEW_MAX - 1)}…`
    : flat;
}

/** A fenced-code opener/closer: 3+ backticks or tildes at the line start. */
const FENCE = /^(`{3,}|~{3,})/;

/**
 * The info-string language tokens that mark a fenced code block as an **executable
 * shell** block — the one region kind that can carry a `command:` verifier and reach
 * `supported`/`refuted` (§17.6, D13). This exact set is the WHOLE detection rule for
 * `CoverageRegion.executable`; there are no other heuristics (no prose keywords, no
 * `$`-prompt sniffing). Adding a language here is a deliberate product decision — it
 * changes which blocks `coverage` steers agents to ground with a verifier — not a
 * formatting tweak, so change it by editing this constant, with a test.
 */
export const EXECUTABLE_INFO_STRINGS: ReadonlySet<string> = new Set([
  "sh",
  "bash",
  "zsh",
  "shell",
  "console",
]);

/**
 * True iff `blockText` is (or contains) a fenced code block whose info-string
 * language token is in {@link EXECUTABLE_INFO_STRINGS}. Scans fence *openers* only
 * (a bare closing fence carries no info string), using the same marker-char fence
 * tracking as `splitBlocks` so the two agree on where fences open and close.
 */
function hasExecutableFence(blockText: string): boolean {
  let fenceChar = ""; // the open fence's marker char, or "" when outside a fence
  for (const rawLine of blockText.split("\n")) {
    const trimmed = rawLine.trim();
    const fence = FENCE.exec(trimmed);
    if (!fence) continue;
    const marker = fence[0][0] as string;
    if (fenceChar === "") {
      // Opener: the info string is the token right after the run of markers.
      fenceChar = marker;
      const lang = trimmed.slice(fence[0].length).trim().split(/\s+/)[0] ?? "";
      if (EXECUTABLE_INFO_STRINGS.has(lang.toLowerCase())) return true;
    } else if (marker === fenceChar) {
      fenceChar = ""; // Closer: same marker char re-closes the open fence.
    }
  }
  return false;
}

/**
 * Segment `text` into blocks on blank-line boundaries, preserving each block's
 * char span (tight — leading/trailing whitespace excluded). A block is a run of
 * consecutive non-blank lines (a paragraph, heading, list, or fenced block);
 * whitespace-only lines separate blocks. Block — not sentence — is the unit, so
 * this never reintroduces sentence-splitting.
 *
 * Fenced code blocks are kept whole: a blank line *inside* a ```/~~~ fence does
 * not split the block, so a code example with blank lines stays one block (and
 * one coverage region) instead of being over-counted as several.
 */
function splitBlocks(text: string): { start: number; end: number }[] {
  const out: { start: number; end: number }[] = [];
  let start = -1; // content start of the open block, or -1 when between blocks
  let end = -1; // content end (exclusive) of the last non-blank line seen
  let fenceChar = ""; // the open fence's marker char ("`"/"~"), or "" when outside
  let cursor = 0;
  const len = text.length;
  while (cursor <= len) {
    let nl = text.indexOf("\n", cursor);
    if (nl === -1) nl = len;
    const line = text.slice(cursor, nl);
    const trimmed = line.trim();
    const fence = FENCE.exec(trimmed);
    if (fence) {
      // A fence line is block content; it opens a fence, or closes the open one.
      const marker = fence[0][0] as string;
      if (fenceChar === "") fenceChar = marker;
      else if (marker === fenceChar) fenceChar = "";
    }
    if (trimmed.length === 0 && fenceChar === "") {
      if (start >= 0) {
        out.push({ start, end });
        start = -1;
      }
    } else {
      if (start < 0) start = cursor + (line.length - line.trimStart().length);
      end = cursor + line.trimEnd().length;
    }
    if (nl === len) break;
    cursor = nl + 1;
  }
  if (start >= 0) out.push({ start, end });
  return out;
}

/** Do a claim span and a block overlap? */
function overlaps(r: Region, blockStart: number, blockEnd: number): boolean {
  return r.start < blockEnd && r.end > blockStart;
}

/**
 * Measure doc-side coverage of `docContent`: segment it into blocks and mark each
 * one covered iff a live, code-grounded claim anchored on `docPath` resolves
 * cleanly (`unchanged`/`moved`) into it. Deterministic; reads only the store and
 * the provided text.
 */
export async function coverage(
  store: ClaimStore,
  docContent: string,
  input: CoverageInput,
  opts: CoverageOptions = {},
): Promise<CoverageResult> {
  // Live, code-grounded claims documented on this doc. Stricter than query.ts's
  // doc-side selection on purpose (see file header): coverage measures *grounding*,
  // so it drops retired claims, doc-only placeholders with no code pin, and claims
  // whose doc anchor has drifted off `unchanged`/`moved`.
  const assertions = await store.allAssertions();
  const spans: ClaimSpan[] = [];
  for (const a of assertions) {
    if (a.enforcement === "retired") continue;
    if (a.anchor.doc.file !== input.docPath) continue;
    // No code pin → grounds nothing (a doc-only `suggested` placeholder).
    if (a.anchor.code.length === 0) continue;
    const side = resolveSide(a.anchor.doc, docContent, { ast: opts.ast });
    // Count only a clean, located resolution. A `changed`/`ambiguous`/`orphaned`
    // anchor either has no region or a stale text-position fallback (resolve.ts) —
    // either way it would inflate the ratio or mis-attribute the claim's block.
    if (!side.region) continue;
    if (side.state !== "unchanged" && side.state !== "moved") continue;
    spans.push({ claimId: a.id, region: side.region });
  }

  const blocks = splitBlocks(docContent);
  const regions: CoverageRegion[] = blocks.map((b) => {
    const blockText = docContent.slice(b.start, b.end);
    const claimIds = spans
      .filter((s) => overlaps(s.region, b.start, b.end))
      .map((s) => s.claimId);
    return {
      range: { start: b.start, end: b.end },
      preview: preview(blockText),
      covered: claimIds.length > 0,
      executable: hasExecutableFence(blockText),
      claimIds,
    };
  });

  const coveredBlocks = regions.filter((r) => r.covered).length;
  const total = regions.length;
  return {
    regions,
    summary: {
      blocks: total,
      coveredBlocks,
      uncoveredBlocks: total - coveredBlocks,
      coverageRatio: total === 0 ? 0 : coveredBlocks / total,
    },
  };
}
