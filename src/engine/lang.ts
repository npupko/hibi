/**
 * Map a file path to a tree-sitter grammar name (§16). The five first-party
 * grammars: TypeScript, Python, Rust, Go, Java.
 */
import { extname } from "node:path";

const BY_EXT: Record<string, string> = {
  ".ts": "typescript",
  ".tsx": "tsx",
  ".js": "typescript",
  ".jsx": "tsx",
  ".mjs": "typescript",
  ".cjs": "typescript",
  ".py": "python",
  ".pyi": "python",
  ".rs": "rust",
  ".go": "go",
  ".java": "java",
};

export function languageForFile(path: string): string | undefined {
  return BY_EXT[extname(path).toLowerCase()];
}
