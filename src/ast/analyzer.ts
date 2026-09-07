/**
 * The tree-sitter analyzer. Implements both the check-time `AstAnalyzer` and
 * the record-time `AnchorAnalyzer` seams. Grammars are the official prebuilt
 * wasm, embedded so the compiled binary stays offline, and loaded lazily per
 * language: `load(languages)` must be awaited before the synchronous
 * `analyze`/`extractValue`/`recordSelectors` see that language.
 */
import { Language, Parser } from "web-tree-sitter";
import runtimeWasm from "web-tree-sitter/web-tree-sitter.wasm" with {
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
import { languageForFile } from "../engine/lang.ts";
import { extractValueFrom, fingerprintNode, snapNamedNode } from "./hash.ts";

const WASM: Record<string, string> = {
  typescript: tsWasm,
  tsx: tsxWasm,
  python: pyWasm,
  rust: rsWasm,
  go: goWasm,
  java: javaWasm,
};

export class TreeSitterAnalyzer implements AstAnalyzer, AnchorAnalyzer {
  private parsers = new Map<string, Parser>();
  private loading = new Map<string, Promise<void>>();

  /** Load the grammars for these languages (unknown names are ignored). Cached per process. */
  async load(languages: Iterable<string | undefined>): Promise<void> {
    const pending: Promise<void>[] = [];
    for (const name of languages) {
      if (!name || this.parsers.has(name) || !WASM[name]) continue;
      let p = this.loading.get(name);
      if (!p) {
        p = Language.load(WASM[name] as string).then((lang) => {
          const parser = new Parser();
          parser.setLanguage(lang);
          this.parsers.set(name, parser);
        });
        this.loading.set(name, p);
      }
      pending.push(p);
    }
    await Promise.all(pending);
  }

  /** Load the grammars for a set of file paths, by extension. */
  async loadForFiles(files: Iterable<string>): Promise<void> {
    const langs = new Set<string | undefined>();
    for (const f of files) langs.add(languageForFile(f));
    await this.load(langs);
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

  extractValue(
    text: string,
    language: string,
    region: Region,
    nodeKind?: string,
  ): string | null {
    const root = this.parse(text, language);
    if (!root) return null;
    const node = snapNamedNode(root, text, region);
    if (!node) return null;
    return extractValueFrom(node, language, region, nodeKind)?.value ?? null;
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
    const v = extractValueFrom(node, language, region);
    const value: Extract<Selector, { kind: "value" }> | undefined = v
      ? { kind: "value", language, nodeKind: v.nodeKind, value: v.value }
      : undefined;
    return { astNode, value };
  }
}

let cached: Promise<TreeSitterAnalyzer> | undefined;

/**
 * The shared analyzer, initialized once per process with no grammars loaded.
 * Pass `languages` to preload grammars in the same call.
 */
export async function getAnalyzer(
  languages: Iterable<string | undefined> = Object.keys(WASM),
): Promise<TreeSitterAnalyzer> {
  if (!cached) {
    cached = (async () => {
      await Parser.init({ locateFile: () => runtimeWasm } as Parameters<
        typeof Parser.init
      >[0]);
      return new TreeSitterAnalyzer();
    })();
  }
  const analyzer = await cached;
  await analyzer.load(languages);
  return analyzer;
}
