/**
 * Anchor construction at record time (§4, §6). The Anchor *is* the baseline
 * snapshot: it captures the text-quote (exact + prefix/suffix), text-position,
 * and — when a tree-sitter analyzer is supplied — the ast-node two-tier hash and
 * the extracted value, all as seen at record time.
 */
import type { Anchor, Selector, Region } from "../core/model.ts";
import { TEXT_QUOTE_CONTEXT } from "../algo/params.ts";

/** Record-time tree-sitter seam (implemented in Layer 5). */
export interface AnchorAnalyzer {
  /** Build the ast-node + value selectors for a region, or {} if unparseable. */
  recordSelectors(
    text: string,
    language: string,
    region: Region,
  ): { astNode?: Extract<Selector, { kind: "ast-node" }>; value?: Extract<Selector, { kind: "value" }> };
}

export interface BuildAnchorOptions {
  /** The structural language for the anchored file (e.g. "typescript"). */
  language?: string;
  analyzer?: AnchorAnalyzer;
}

/** Build a precise composite Anchor for a region of `content` in `file`. */
export function buildAnchor(
  file: string,
  content: string,
  region: Region,
  opts: BuildAnchorOptions = {},
): Anchor {
  const start = Math.max(0, Math.min(region.start, content.length));
  const end = Math.max(start, Math.min(region.end, content.length));

  const exact = content.slice(start, end);
  const prefix = content.slice(Math.max(0, start - TEXT_QUOTE_CONTEXT), start);
  const suffix = content.slice(end, Math.min(content.length, end + TEXT_QUOTE_CONTEXT));

  const selectors: Selector[] = [
    { kind: "text-quote", exact, prefix, suffix },
    { kind: "text-position", start, end },
  ];

  if (opts.language && opts.analyzer) {
    const { astNode, value } = opts.analyzer.recordSelectors(content, opts.language, { start, end });
    if (astNode) selectors.push(astNode);
    if (value) selectors.push(value);
  }

  return { file, selectors };
}

/** Build a coarse path anchor (navigational; never reported stale). */
export function buildPathAnchor(file: string): Anchor {
  return { file, selectors: [{ kind: "path", path: file }] };
}

/** Build a coarse glob anchor (blast-radius). */
export function buildGlobAnchor(glob: string): Anchor {
  return { file: glob, selectors: [{ kind: "glob", glob }] };
}
