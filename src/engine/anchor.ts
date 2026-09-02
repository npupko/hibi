/**
 * Anchor construction at record time. The anchor is bidirectional: a doc-side
 * bundle (the documented sentence) plus code-side bundles (the code it
 * describes). Each bundle is the baseline snapshot for its side: text-quote
 * (exact + prefix/suffix), text-position, and, with an analyzer, the ast-node
 * fingerprint and the in-span literal value.
 */

import { TEXT_QUOTE_CONTEXT } from "../algo/params.ts";
import type {
  Anchor,
  Region,
  Selector,
  SelectorBundle,
} from "../core/model.ts";

/** Record-time tree-sitter seam (implemented by `src/ast/analyzer.ts`). */
export interface AnchorAnalyzer {
  recordSelectors(
    text: string,
    language: string,
    region: Region,
  ): {
    astNode?: Extract<Selector, { kind: "ast-node" }>;
    value?: Extract<Selector, { kind: "value" }>;
  };
}

export interface BuildBundleOptions {
  /** The structural language for the file (e.g. "typescript"); omit for prose. */
  language?: string;
  analyzer?: AnchorAnalyzer;
}

/**
 * Build a precise bundle for a region of `content` in `file`. Always emits
 * text-quote + text-position; adds ast-node + value when an analyzer and a
 * language are supplied. Refuses an empty span: an empty quote matches
 * everywhere and could never fail again.
 */
export function buildSelectorBundle(
  file: string,
  content: string,
  region: Region,
  opts: BuildBundleOptions = {},
): SelectorBundle {
  const start = Math.max(0, Math.min(region.start, content.length));
  const end = Math.max(start, Math.min(region.end, content.length));

  const exact = content.slice(start, end);
  if (exact.trim().length === 0) {
    throw new Error(
      `refusing to anchor an empty span in ${file}; pass a quote or range that covers text`,
    );
  }
  const prefix = content.slice(Math.max(0, start - TEXT_QUOTE_CONTEXT), start);
  const suffix = content.slice(
    end,
    Math.min(content.length, end + TEXT_QUOTE_CONTEXT),
  );

  const selectors: Selector[] = [
    { kind: "text-quote", exact, prefix, suffix },
    { kind: "text-position", start, end },
  ];

  if (opts.language && opts.analyzer) {
    const { astNode, value } = opts.analyzer.recordSelectors(
      content,
      opts.language,
      { start, end },
    );
    if (astNode) selectors.push(astNode);
    if (value) selectors.push(value);
  }

  return { file, selectors };
}

/** Build a coarse bundle for a file or glob pattern (navigational, never drift). */
export function buildCoarseBundle(pattern: string): SelectorBundle {
  return { file: pattern, selectors: [{ kind: "coarse", pattern }] };
}

/** True when every selector in the bundle is coarse. */
export function isCoarseBundle(bundle: SelectorBundle): boolean {
  return bundle.selectors.every((s) => s.kind === "coarse");
}

/**
 * Does a coarse pattern cover `path`? Exact file, a directory ancestor on a
 * `/` boundary, or a glob match.
 */
export function coarseCovers(pattern: string, path: string): boolean {
  if (pattern === path) return true;
  const dir = pattern.endsWith("/") ? pattern : `${pattern}/`;
  if (path.startsWith(dir)) return true;
  try {
    return new Bun.Glob(pattern).match(path);
  } catch {
    return false;
  }
}

export function composeAnchor(
  doc: SelectorBundle,
  code: SelectorBundle[] = [],
): Anchor {
  return { doc, code };
}
