/**
 * Copy each official `tree-sitter-LANG/*.wasm` into a tracked `grammars/`
 * directory (§16 grammar-acquisition plan, step 2). This keeps `check` fully
 * offline and survives `bun build --compile` into the single binary.
 */
import { access, copyFile, mkdir } from "node:fs/promises";
import { join } from "node:path";

const ROOT = join(import.meta.dir, "..");
const OUT = join(ROOT, "grammars");

/** grammar name → source wasm path within node_modules. */
const SOURCES: Record<string, string> = {
  typescript: "tree-sitter-typescript/tree-sitter-typescript.wasm",
  tsx: "tree-sitter-typescript/tree-sitter-tsx.wasm",
  python: "tree-sitter-python/tree-sitter-python.wasm",
  rust: "tree-sitter-rust/tree-sitter-rust.wasm",
  go: "tree-sitter-go/tree-sitter-go.wasm",
  java: "tree-sitter-java/tree-sitter-java.wasm",
};

async function exists(p: string): Promise<boolean> {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}

export async function copyGrammars(): Promise<string[]> {
  await mkdir(OUT, { recursive: true });
  const written: string[] = [];
  for (const [name, rel] of Object.entries(SOURCES)) {
    const src = join(ROOT, "node_modules", rel);
    if (!(await exists(src))) {
      throw new Error(`Missing grammar wasm: ${src} (run \`bun install\`).`);
    }
    const dest = join(OUT, `tree-sitter-${name}.wasm`);
    await copyFile(src, dest);
    written.push(dest);
  }
  return written;
}

if (import.meta.main) {
  const written = await copyGrammars();
  console.log(`Copied ${written.length} grammar(s) to ${OUT}`);
}
