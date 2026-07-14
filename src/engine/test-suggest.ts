/**
 * Deterministic reverse-import test suggestion (§9, D26) — advisory only.
 *
 * A static reverse-dependency walk (the industry-validated primitive: Jest
 * `--findRelatedTests`, Vitest `--changed`, Nx affected): a candidate test file
 * "exercises" an anchored code file when that file is in the test's import
 * closure at depth ≤ 2 (the test imports it directly, or imports a file that
 * imports it). Computed fresh from the working tree at check time — no committed
 * coverage artifact (the deferred D26 resolver). Reuses the analyzer's import
 * extraction (`extractImportSpecifiers` via `AstAnalyzer.extractImports`) and the
 * specifier→file resolution from `evidence.ts` (`importCandidates`); it writes no
 * new parser or resolver, and never touches verdicts, exit codes, or the store.
 */

import type { AstAnalyzer } from "../algo/resolve.ts";
import { importCandidates } from "./evidence.ts";
import { languageForFile } from "./lang.ts";

/** The reader + analyzer + root the reverse-import walk needs (injected by the shell). */
export interface TestSuggestDeps {
  /** Only `extractImports` is used; absent → no candidate ever matches (no import graph). */
  analyzer?: Pick<AstAnalyzer, "extractImports">;
  /** Read a repo-relative file; `null` when it is missing. */
  readFile: (rel: string) => Promise<string | null>;
  /** The anchor root the candidate globs scan against. */
  root: string;
}

/** Import hops from a test file to follow when building its closure (D26: depth ≤ 2). */
const MAX_DEPTH = 2;
/** Never surface more than this many test suggestions (D26). */
const MAX_SUGGESTIONS = 3;

/** Globs selecting candidate test files (`*.test.*` / `*.spec.*`, and `test/`-style dirs). */
const TEST_GLOBS = [
  "**/*.test.*",
  "**/*.spec.*",
  "**/test/**",
  "**/tests/**",
  "**/__tests__/**",
] as const;

/** Directory names never walked (regenerable / vendored / hibi's own store). */
const IGNORE_SEGMENTS = new Set([
  "node_modules",
  ".git",
  ".claims",
  "dist",
  "target",
]);

function isIgnored(rel: string): boolean {
  return rel.split("/").some((seg) => IGNORE_SEGMENTS.has(seg));
}

/** One candidate test file and the set of files it reaches within depth ≤ 2. */
export type TestFileIndex = { path: string; closure: Set<string> }[];

/** First on-disk candidate a specifier resolves to (the `evidence.ts` resolution), or null. */
async function resolveSpecifier(
  fromFile: string,
  spec: string,
  language: string,
  readFile: TestSuggestDeps["readFile"],
): Promise<string | null> {
  for (const cand of importCandidates(fromFile, spec, language)) {
    if ((await readFile(cand)) !== null) return cand;
  }
  return null;
}

/** The set of files `seed` imports within `MAX_DEPTH` hops (file-level; no call graph). */
async function importClosure(
  seed: string,
  deps: TestSuggestDeps,
): Promise<Set<string>> {
  const reached = new Set<string>();
  if (!deps.analyzer) return reached;
  let frontier = [seed];
  for (let d = 0; d < MAX_DEPTH; d++) {
    const next: string[] = [];
    for (const f of frontier) {
      const content = await deps.readFile(f);
      if (content === null) continue;
      const lang = languageForFile(f);
      if (!lang) continue;
      for (const spec of deps.analyzer.extractImports(content, lang)) {
        const resolved = await resolveSpecifier(f, spec, lang, deps.readFile);
        if (resolved && !reached.has(resolved)) {
          reached.add(resolved);
          next.push(resolved);
        }
      }
    }
    frontier = next;
  }
  return reached;
}

/**
 * Build the test-file import index once per check run: every candidate test file
 * mapped to its depth-≤2 import closure. Deterministic; reads only the working
 * tree through `deps.readFile`.
 */
export async function buildTestFileIndex(
  deps: TestSuggestDeps,
): Promise<TestFileIndex> {
  const candidates = new Set<string>();
  for (const g of TEST_GLOBS) {
    for await (const rel of new Bun.Glob(g).scan({
      cwd: deps.root,
      onlyFiles: true,
    })) {
      if (!isIgnored(rel)) candidates.add(rel);
    }
  }
  const index: TestFileIndex = [];
  for (const path of candidates) {
    index.push({ path, closure: await importClosure(path, deps) });
  }
  return index;
}

/**
 * The test files that exercise `anchoredFile` (§9, D26): candidate tests whose
 * import closure reaches it, sorted lexicographically, capped at 3. A pure lookup
 * over the pre-built index — no I/O — so the walk runs at most once per check run.
 */
export function suggestTests(
  anchoredFile: string,
  index: TestFileIndex,
): string[] {
  const hits = index
    .filter((t) => t.closure.has(anchoredFile))
    .map((t) => t.path);
  hits.sort();
  return hits.slice(0, MAX_SUGGESTIONS);
}
