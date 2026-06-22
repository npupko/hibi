/**
 * Shared join/derivation helpers for the human renderers. The `CheckReport`
 * carries verdicts + per-document rollups but not the authored facets (`owner`,
 * `ref`, `ttl`) — those live on the `Assertion` (§5). These helpers bridge that
 * gap with data the CLI shell already has in hand, and never touch the verdict.
 */

import { readFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import type { Assertion, Proposition, Verdict } from "../../core/model.ts";

/** A throw-free synchronous file reader rooted at the anchor root. */
export type FileRead = (rel: string) => string | null;

/** A cached, throw-free synchronous file reader rooted at the anchor root. */
export function fileReader(anchorRoot: string): FileRead {
  const cache = new Map<string, string | null>();
  return (rel: string): string | null => {
    if (cache.has(rel)) return cache.get(rel) ?? null;
    const abs = isAbsolute(rel) ? rel : join(anchorRoot, rel);
    let text: string | null = null;
    try {
      text = readFileSync(abs, "utf8");
    } catch {
      text = null;
    }
    cache.set(rel, text);
    return text;
  };
}

/** 1-based line number of a char offset in `content` (newline count + 1). */
export function lineOfOffset(content: string, offset: number): number {
  let line = 1;
  const end = Math.min(offset, content.length);
  for (let i = 0; i < end; i++) if (content[i] === "\n") line += 1;
  return line;
}

/**
 * The code anchor `path:line` for a verdict, or just the file when the line is
 * unknown. Pairs the first located `codeRegion` (char offsets) with the
 * assertion's first code bundle file and converts the offset to a line.
 */
export function codeAnchor(
  verdict: Verdict,
  assertion: Assertion | undefined,
  read: (rel: string) => string | null,
): string | null {
  const file = assertion?.anchor.code[0]?.file;
  if (!file) return null;
  const region = verdict.evidence.codeRegions[0];
  if (!region) return file;
  const content = read(file);
  if (content === null) return file;
  return `${file}:${lineOfOffset(content, region.start)}`;
}

/**
 * Freshness crumb for a claim: the short verified `ref` plus its time state.
 * There is no stored `verifiedAt` (§5), so true relative age is a nice-to-have;
 * the ref + `expired`/`ttl` state is the deterministic, always-available signal.
 */
export function freshness(
  verdict: Verdict,
  assertion: Assertion | undefined,
): string {
  const parts: string[] = [];
  const ref = assertion?.ref;
  if (ref && ref !== "WORKTREE") parts.push(`ref ${ref.slice(0, 7)}`);
  else if (ref === "WORKTREE") parts.push("worktree");
  if (verdict.expired) parts.push("expired");
  else if (assertion?.ttl) parts.push(`ttl ${assertion.ttl}`);
  return parts.join(", ") || "—";
}

/** The documented sentence for a verdict (the non-authoritative cache, §5/§18-B). */
export function docSentence(
  verdict: Verdict,
  propsById: Map<string, Proposition>,
): string {
  return (
    propsById.get(verdict.propositionId)?.textCache ?? "(unknown proposition)"
  );
}

/** Collapse a string to a single line, trimmed and capped for terminal display. */
export function oneLine(s: string, max = 72): string {
  const flat = s.replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
}
