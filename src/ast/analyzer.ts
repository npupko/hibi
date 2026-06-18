/**
 * The tree-sitter analyzer (Tier-2, §6, §16). Implements both the check-time
 * `AstAnalyzer` and the record-time `AnchorAnalyzer` seams. Grammars are the
 * official prebuilt wasm, embedded so the compiled single binary stays offline.
 *
 * web-tree-sitter parsing is synchronous once a Language is loaded; grammars are
 * preloaded asynchronously by `getAnalyzer()`, after which `analyze`/`extractValue`
 * are synchronous (as the resolver fusion path requires).
 */
import { Language, Parser } from "web-tree-sitter";
// Embedded grammar wasm — `with { type: "file" }` yields a path that resolves in
// dev (`bun run`) and is embedded by `bun build --compile` (§16, §12).
// The web-tree-sitter runtime wasm must also be embedded and located explicitly,
// otherwise the compiled binary cannot find it (`/$bunfs/root/tree-sitter.wasm`).
import runtimeWasm from "web-tree-sitter/tree-sitter.wasm" with {
  type: "file",
};
import goWasm from "../../grammars/tree-sitter-go.wasm" with { type: "file" };
import javaWasm from "../../grammars/tree-sitter-java.wasm" with {
  type: "file",
};
import pyWasm from "../../grammars/tree-sitter-python.wasm" with {
  type: "file",
};
import rsWasm from "../../grammars/tree-sitter-rust.wasm" with { type: "file" };
import tsxWasm from "../../grammars/tree-sitter-tsx.wasm" with { type: "file" };
import tsWasm from "../../grammars/tree-sitter-typescript.wasm" with {
  type: "file",
};
import type { AstAnalysis, AstAnalyzer } from "../algo/resolve.ts";
import type { Region, Selector } from "../core/model.ts";
import type { AnchorAnalyzer } from "../engine/anchor.ts";
import { extractValueFrom, fingerprintNode, snapNamedNode } from "./hash.ts";

const WASM: Record<string, string> = {
  typescript: tsWasm,
  tsx: tsxWasm,
  python: pyWasm,
  rust: rsWasm,
  go: goWasm,
  java: javaWasm,
};

class TreeSitterAnalyzer implements AstAnalyzer, AnchorAnalyzer {
  private parsers = new Map<string, Parser>();

  constructor(private languages: Map<string, Language>) {
    for (const [name, lang] of languages) {
      const p = new Parser();
      p.setLanguage(lang);
      this.parsers.set(name, p);
    }
  }

  private parse(text: string, language: string) {
    const parser = this.parsers.get(language);
    if (!parser) return null;
    const tree = parser.parse(text);
    return tree?.rootNode ?? null;
  }

  analyze(text: string, language: string, region: Region): AstAnalysis | null {
    const root = this.parse(text, language);
    if (!root) return null;
    const node = snapNamedNode(root, text, region);
    if (!node) return null;
    const fp = fingerprintNode(node);
    return {
      nodeType: fp.nodeType,
      structuralHash: fp.structuralHash,
      semanticHash: fp.semanticHash,
      region: { start: node.startIndex, end: node.endIndex },
    };
  }

  extractValue(text: string, language: string, region: Region): string | null {
    const root = this.parse(text, language);
    if (!root) return null;
    const node = snapNamedNode(root, text, region);
    if (!node) return null;
    return extractValueFrom(node, language)?.value ?? null;
  }

  recordSelectors(
    text: string,
    language: string,
    region: Region,
  ): {
    astNode?: Extract<Selector, { kind: "ast-node" }>;
    value?: Extract<Selector, { kind: "value" }>;
  } {
    const root = this.parse(text, language);
    if (!root) return {};
    const node = snapNamedNode(root, text, region);
    if (!node) return {};
    const fp = fingerprintNode(node);
    const astNode: Extract<Selector, { kind: "ast-node" }> = {
      kind: "ast-node",
      language,
      nodeType: fp.nodeType,
      structuralHash: fp.structuralHash,
      semanticHash: fp.semanticHash,
    };
    const v = extractValueFrom(node, language);
    const value: Extract<Selector, { kind: "value" }> | undefined = v
      ? { kind: "value", language, nodeKind: v.nodeKind, value: v.value }
      : undefined;
    return { astNode, value };
  }
}

let cached: Promise<TreeSitterAnalyzer> | undefined;

/** Load all grammars once and return the shared analyzer. */
export async function getAnalyzer(): Promise<TreeSitterAnalyzer> {
  if (!cached) {
    cached = (async () => {
      await Parser.init({ locateFile: () => runtimeWasm } as Parameters<
        typeof Parser.init
      >[0]);
      const langs = new Map<string, Language>();
      for (const [name, path] of Object.entries(WASM)) {
        langs.set(name, await Language.load(path));
      }
      return new TreeSitterAnalyzer(langs);
    })();
  }
  return cached;
}
