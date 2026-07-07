/**
 * Structural snapping & the two-tier AST fingerprint (§17.1 snap, §17.2 hash).
 * Operates on tree-sitter nodes; deterministic and language-universal except for
 * the value-extraction map (§17.4).
 */
import type { Node } from "web-tree-sitter";
import { collapseWhitespace } from "../algo/normalize.ts";
import type { Region } from "../core/model.ts";
import { valueClass } from "./value-map.ts";

/** Content-literal kinds (§17.2, verbatim) — some grammars hide a literal body. */
const CONTENT_LITERAL = new Set([
  "string",
  "string_literal",
  "interpreted_string_literal",
  "raw_string_literal",
  "char_literal",
  "rune_literal",
  "number",
  "integer",
  "float",
  "integer_literal",
  "float_literal",
  "int_literal",
  "imaginary_literal",
]);

const SEP = "";

// SEP (above) is the US control char (\x1f): a delimiter between serialized nodes
// so adjacent kinds/tokens cannot concatenate into a colliding stream (§17.2).
function xx(s: string): string {
  return Bun.hash.xxHash64(s).toString(16).padStart(16, "0");
}

/**
 * xxHash64 of a file's content, as 16-hex (§17.6, D14). The single hashing
 * function behind the change-gate's evidence baselines — used identically at
 * `record`/`reanchor` (to store) and at `check` (to compare), so they never drift.
 */
export function hashContent(content: string): string {
  return xx(content);
}

/**
 * Snap a region to the smallest enclosing *named* node (§17.1). Trim leading and
 * trailing whitespace off the span first (if it collapses, keep one character) —
 * this is what makes the chosen node invariant to re-indentation.
 */
export function snapNamedNode(
  root: Node,
  text: string,
  region: Region,
): Node | null {
  let ts = Math.max(0, region.start);
  let te = Math.min(text.length, region.end);
  const span = text.slice(ts, te);
  const lead = span.length - span.replace(/^\s+/, "").length;
  const trail = span.length - span.replace(/\s+$/, "").length;
  ts += lead;
  te -= trail;
  if (ts >= te) te = ts + 1; // collapsed → keep one character

  let node = root.descendantForIndex(ts, Math.max(ts, te - 1));
  while (node && !node.isNamed) node = node.parent;
  return node ?? root;
}

export interface AstFingerprint {
  nodeType: string;
  structuralHash: string;
  semanticHash: string;
}

/**
 * Two-tier fingerprint (§17.2): pre-order DFS over ALL children (including
 * anonymous token nodes), source order, no sorting, no trivia dropping.
 *   - structural: the `type` of every node (invariant under renames/literals/ws).
 *   - semantic:   leaf → `type:text`; internal → `type`; content-literal kinds
 *                 additionally `=<whitespace-collapsed text>`.
 */
export function fingerprintNode(node: Node): AstFingerprint {
  const struct: string[] = [];
  const sem: string[] = [];

  const visit = (n: Node): void => {
    struct.push(n.type);
    const leaf = n.childCount === 0;
    sem.push(leaf ? `${n.type}:${n.text}` : n.type);
    if (CONTENT_LITERAL.has(n.type)) sem.push(`=${collapseWhitespace(n.text)}`);
    for (let i = 0; i < n.childCount; i++) {
      const c = n.child(i);
      if (c) visit(c);
    }
  };
  visit(node);

  return {
    nodeType: node.type,
    structuralHash: xx(struct.join(SEP)),
    semanticHash: xx(sem.join(SEP)),
  };
}

/**
 * Extract a literal value from within `node` (§17.4): pre-order DFS over named
 * children, take the first matching literal and stop. Collections strip all
 * whitespace; scalars/strings are whitespace-collapsed.
 */
export function extractValueFrom(
  node: Node,
  language: string,
): { nodeKind: string; value: string } | null {
  let found: { nodeKind: string; value: string } | null = null;

  const visit = (n: Node): void => {
    if (found) return;
    const cls = valueClass(language, n.type);
    if (cls) {
      const raw = n.text;
      const value =
        cls === "collection"
          ? raw.replace(/\s+/g, "")
          : collapseWhitespace(raw);
      found = { nodeKind: n.type, value };
      return;
    }
    for (let i = 0; i < n.namedChildCount; i++) {
      const c = n.namedChild(i);
      if (c) visit(c);
      if (found) return;
    }
  };
  visit(node);
  return found;
}
