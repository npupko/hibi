/**
 * Shared join/derivation helpers for the human renderers. The `CheckReport`
 * carries verdicts + per-document rollups but not the authored facets (`owner`,
 * `ref`, `ttl`, `sentence`); those live on the `Claim`. These helpers bridge
 * that gap with data the CLI shell already has in hand, and never touch the
 * verdict.
 */

import { readFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import type { Claim, Verdict } from "../../core/model.ts";

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
 * The evidence anchor `path:line` for a verdict, or just the file when the
 * line is unknown. Pairs the first located evidence region (char offsets) with
 * the claim's first evidence bundle file and converts the offset to a line.
 */
export function evidenceAnchor(
  verdict: Verdict,
  claim: Claim | undefined,
  read: (rel: string) => string | null,
): string | null {
  const file = claim?.evidence[0]?.file;
  if (!file) return null;
  const region = verdict.detail.evidenceRegions[0];
  if (!region) return file;
  const content = read(file);
  if (content === null) return file;
  return `${file}:${lineOfOffset(content, region.start)}`;
}

/** Freshness crumb for a claim: the short recorded `ref` plus its time state. */
export function freshness(verdict: Verdict, claim: Claim | undefined): string {
  const parts: string[] = [];
  const ref = claim?.ref;
  if (ref && ref !== "WORKTREE") parts.push(`ref ${ref.slice(0, 7)}`);
  else if (ref === "WORKTREE") parts.push("worktree");
  if (verdict.expired) parts.push("expired");
  else if (claim?.ttl) parts.push(`ttl ${claim.ttl}`);
  return parts.join(", ") || "—";
}

/** Collapse a string to a single line, trimmed and capped for terminal display. */
export function oneLine(s: string, max = 72): string {
  const flat = s.replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
}
