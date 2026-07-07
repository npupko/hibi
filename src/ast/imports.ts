/**
 * Per-grammar import extraction (§17.6, D14) — a deterministic walk of the parse
 * tree that lists the *specifiers* a file imports. Sibling of `value-map.ts`:
 * this is the explicit node-kind map the change-gate's file-level reachability
 * rests on. **No call graph** — file-level import edges only.
 *
 * The extraction is intentionally asymmetric and documented as such: TypeScript,
 * Python, and Rust have file-resolvable relative imports; Go and Java package
 * imports do not resolve to files and are skipped (their evidence comes from the
 * scope's `include` globs). An unresolvable specifier is skipped **silently** —
 * the walk never guesses and never throws.
 *
 * The returned specifiers are raw (a JS module string, a Python relative-import
 * dotted string, or a Rust `mod` name); `engine/evidence.ts` resolves them to
 * repo-relative file paths, branching by language.
 */
import type { Node } from "web-tree-sitter";

/** Strip surrounding quotes (and any r/b/u string prefix) off a string literal. */
function unquote(raw: string): string {
  return raw.replace(/^[a-zA-Z]*(["'`])/, "").replace(/["'`]$/, "");
}

/** Depth-first walk collecting import specifiers for one language (§17.6). */
export function extractImportSpecifiers(
  root: Node,
  language: string,
): string[] {
  const out: string[] = [];
  const push = (s: string) => {
    if (s.length > 0) out.push(s);
  };

  const visit = (n: Node): void => {
    collect(n, language, push);
    for (let i = 0; i < n.childCount; i++) {
      const c = n.child(i);
      if (c) visit(c);
    }
  };
  visit(root);
  return out;
}

/** Per-node import collection — the explicit node-kind map from the D14 table. */
function collect(n: Node, language: string, push: (s: string) => void): void {
  switch (language) {
    case "typescript":
    case "tsx": {
      // `import … from "src"` and re-exporting `export … from "src"`: the
      // `source` field is the module string. `require("src")` call expressions.
      if (n.type === "import_statement" || n.type === "export_statement") {
        const src = n.childForFieldName("source");
        if (src?.type === "string") push(unquote(src.text));
      } else if (n.type === "call_expression") {
        const fn = n.childForFieldName("function");
        if (fn?.text === "require") {
          const args = n.childForFieldName("arguments");
          const first = args?.namedChild(0);
          if (first?.type === "string") push(unquote(first.text));
        }
      }
      return;
    }
    case "python": {
      // `from .x import y` / `from ..pkg import y`: only relative imports resolve
      // to files; an absolute `import os` / `from pkg import x` is skipped.
      if (n.type === "import_from_statement") {
        const mod = n.childForFieldName("module_name");
        if (mod?.type === "relative_import") push(mod.text);
      }
      return;
    }
    case "rust": {
      // `mod foo;` (a declaration, no inline body) resolves to `foo.rs` or
      // `foo/mod.rs`. An inline `mod foo { … }` and every `use` path are skipped.
      if (n.type === "mod_item") {
        const name = n.childForFieldName("name");
        const body = n.childForFieldName("body");
        if (name && !body) push(name.text);
      }
      return;
    }
    // go / java: package imports are not file-resolvable — skipped (D14).
    default:
      return;
  }
}
