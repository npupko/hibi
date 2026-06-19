/**
 * Anchor construction at record time (§4, §6). The Anchor is **bidirectional**:
 * a doc-side bundle (the documented sentence) plus one or more code-side bundles
 * (the code it describes). Each bundle *is* the baseline snapshot for its side:
 * it captures the text-quote (exact + prefix/suffix), text-position, and — when
 * a tree-sitter analyzer is supplied — the ast-node two-tier hash and the
 * extracted value, all as seen at record time. An optional `inline-id` marker
 * stabilizes re-anchoring on owned docs (§4/§8) without restating the claim.
 */

import { TEXT_QUOTE_CONTEXT } from "../algo/params.ts";
import type {
  Anchor,
  Region,
  Selector,
  SelectorBundle,
} from "../core/model.ts";

/** Record-time tree-sitter seam (implemented by the analyzer in `src/ast`). */
export interface AnchorAnalyzer {
  /** Build the ast-node + value selectors for a region, or {} if unparseable. */
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
  /** Optional owned-doc inline marker id that stabilizes re-anchoring (§4/§8). */
  inlineId?: string;
}

/**
 * Build a precise `SelectorBundle` for a region of `content` in `file` — one
 * side of an anchor. Always emits text-quote + text-position; adds ast-node +
 * value when a structural analyzer is supplied (code side or a parseable doc);
 * adds an `inline-id` selector when an owned-doc marker id is given.
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

  if (opts.inlineId) selectors.push({ kind: "inline-id", id: opts.inlineId });

  return { file, selectors };
}

/** Build a coarse path bundle (code-side; navigational, never reported drift). */
export function buildPathBundle(file: string): SelectorBundle {
  return { file, selectors: [{ kind: "path", path: file }] };
}

/** Build a coarse glob bundle (code-side blast-radius). */
export function buildGlobBundle(glob: string): SelectorBundle {
  return { file: glob, selectors: [{ kind: "glob", glob }] };
}

/** Compose the bidirectional Anchor from a doc-side bundle and code-side bundles. */
export function composeAnchor(
  doc: SelectorBundle,
  code: SelectorBundle[] = [],
): Anchor {
  return { doc, code };
}
