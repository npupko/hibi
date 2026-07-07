/**
 * The change-gate evidence set (§17.6, D14) — the file-level blast-radius a
 * behavioral claim watches. I/O is via an injected `readFile` reader (the
 * `check.ts`/`index.ts` shells own the FS), so this module stays deterministic
 * and testable, like `check.ts`.
 *
 * The evidence set for a behavioral claim = the anchored code file(s) + the files
 * they import to `depth` (default 1, via `ast/imports.ts` — no call graph) +
 * `include` globs − `exclude` globs + resolvable verifier `ref` source paths.
 * `evidenceBaseline` (path → xxHash64) is captured over this set at record time;
 * the gate recomputes the set from *current* imports at check time.
 */

import { dirname, join, normalize } from "node:path";
import type { AstAnalyzer } from "../algo/resolve.ts";
import { hashContent } from "../ast/hash.ts";
import type { Assertion, BehaviorScope, Verifier } from "../core/model.ts";
import { languageForFile } from "./lang.ts";

/**
 * The pieces of a claim the evidence walk needs, independent of an Assertion —
 * so `record`/`reanchor` can capture a baseline from raw code targets before the
 * assertion exists, and `check` can pass a live Assertion. `seeds` are the
 * anchored code files (glob patterns excluded by the caller).
 */
export interface EvidenceScope {
  seeds: string[];
  behaviorScope?: BehaviorScope;
  verifiers?: Verifier[];
}

/** The reader + analyzer + root the evidence walk needs (injected by the shell). */
export interface EvidenceDeps {
  /** Only `extractImports` is used; absent → no import following (anchored files only). */
  analyzer?: Pick<AstAnalyzer, "extractImports">;
  /** Read a repo-relative file; `null` when it is missing. */
  readFile: (rel: string) => Promise<string | null>;
  /** The anchor root the `include`/`exclude` globs scan against. */
  root: string;
}

const RELATIVE = /^\.\.?\//;

/** Normalize a repo-relative path; `null` if it escapes the root (leading `..`). */
function norm(p: string): string | null {
  const n = normalize(p);
  if (n.startsWith("..")) return null;
  return n;
}

/**
 * Candidate on-disk paths a specifier could resolve to, per language (§17.6,
 * D14). Bare/package specifiers (no relative prefix) yield none — they are not
 * file-resolvable and are skipped.
 */
export function importCandidates(
  fromFile: string,
  spec: string,
  language: string,
): string[] {
  const dir = dirname(fromFile);
  if (language === "typescript" || language === "tsx") {
    if (!RELATIVE.test(spec)) return [];
    const base = norm(join(dir, spec));
    if (base === null) return [];
    const exts = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];
    return [
      base,
      ...exts.map((e) => base + e),
      ...exts.map((e) => join(base, `index${e}`)),
    ];
  }
  if (language === "python") {
    // A Python relative import is a run of leading dots (level) + a dotted path.
    if (!spec.startsWith(".")) return [];
    const dots = /^\.+/.exec(spec)?.[0].length ?? 0;
    const rest = spec.slice(dots).replace(/\./g, "/");
    let d = dir;
    for (let i = 1; i < dots; i++) d = join(d, "..");
    const base = norm(rest ? join(d, rest) : d);
    if (base === null) return [];
    return [`${base}.py`, join(base, "__init__.py")];
  }
  if (language === "rust") {
    // `mod foo;` → `foo.rs` or `foo/mod.rs`, relative to the declaring file's dir.
    const base = norm(join(dir, spec));
    if (base === null) return [];
    return [`${base}.rs`, join(base, "mod.rs")];
  }
  return [];
}

/** First candidate for `spec` that exists on disk, or `null` (skip silently). */
async function resolveExisting(
  fromFile: string,
  spec: string,
  language: string,
  readFile: EvidenceDeps["readFile"],
): Promise<string | null> {
  for (const cand of importCandidates(fromFile, spec, language)) {
    if ((await readFile(cand)) !== null) return cand;
  }
  return null;
}

/** The seed code files of a claim: precise/coarse bundles, not glob patterns. */
export function seedFiles(a: Assertion): string[] {
  const out: string[] = [];
  for (const b of a.anchor.code) {
    if (b.selectors.some((s) => s.kind === "glob")) continue;
    out.push(b.file);
  }
  return out;
}

/**
 * The current evidence-set paths for a behavioral claim (§17.6, D14): anchored
 * code files + imports to `depth` + `include` globs − `exclude` globs +
 * resolvable verifier sources. Recomputed from current imports at check time.
 */
export async function evidenceSetPaths(
  a: Assertion,
  deps: EvidenceDeps,
): Promise<Set<string>> {
  return collectEvidencePaths(
    {
      seeds: seedFiles(a),
      behaviorScope: a.behaviorScope,
      verifiers: a.verifiers,
    },
    deps,
  );
}

/** The evidence-set walk over an {@link EvidenceScope} (see `evidenceSetPaths`). */
export async function collectEvidencePaths(
  ev: EvidenceScope,
  deps: EvidenceDeps,
): Promise<Set<string>> {
  const scope = ev.behaviorScope;
  const depth = scope?.depth ?? 1;
  const include = scope?.include ?? [];
  const exclude = scope?.exclude ?? [];

  const paths = new Set<string>(ev.seeds);

  // BFS import edges out to `depth` (no call graph — file-level only).
  let frontier = [...paths];
  for (let d = 0; d < depth && deps.analyzer; d++) {
    const next: string[] = [];
    for (const f of frontier) {
      const content = await deps.readFile(f);
      if (content === null) continue;
      const lang = languageForFile(f);
      if (!lang) continue;
      for (const spec of deps.analyzer.extractImports(content, lang)) {
        const resolved = await resolveExisting(f, spec, lang, deps.readFile);
        if (resolved && !paths.has(resolved)) {
          paths.add(resolved);
          next.push(resolved);
        }
      }
    }
    frontier = next;
  }

  // `include` globs fold in dependencies no import edge reaches (config, fixtures).
  for (const g of include) {
    for await (const rel of new Bun.Glob(g).scan({
      cwd: deps.root,
      onlyFiles: true,
    })) {
      paths.add(rel);
    }
  }

  // Resolvable verifier source paths (a `ref` that is itself a file).
  for (const v of ev.verifiers ?? []) {
    if (v.ref && (await deps.readFile(v.ref)) !== null) paths.add(v.ref);
  }

  // `exclude` globs win, applied last.
  if (exclude.length > 0) {
    const globs = exclude.map((g) => new Bun.Glob(g));
    for (const p of [...paths]) {
      if (globs.some((g) => g.match(p))) paths.delete(p);
    }
  }

  return paths;
}

/** Read the current content of every evidence path (for the gate's `opts.evidence`). */
export async function readEvidenceContents(
  paths: Iterable<string>,
  readFile: EvidenceDeps["readFile"],
): Promise<Map<string, string | null>> {
  const out = new Map<string, string | null>();
  for (const p of paths) out.set(p, await readFile(p));
  return out;
}

/**
 * Capture the evidence baseline for a claim at record/reanchor time (§17.6,
 * D14): each evidence-set path → the xxHash64 of its current content. A missing
 * evidence file is omitted (nothing to hash).
 */
export async function buildEvidenceBaseline(
  a: Assertion,
  deps: EvidenceDeps,
): Promise<Record<string, string>> {
  return baselineOverPaths(await evidenceSetPaths(a, deps), deps.readFile);
}

/**
 * Capture the evidence baseline from raw code targets (the `record`/`reanchor`
 * shell path, where no Assertion exists yet). `seeds` are the precise/coarse
 * anchored code files (glob patterns excluded by the caller).
 */
export async function buildEvidenceBaselineFor(
  ev: EvidenceScope,
  deps: EvidenceDeps,
): Promise<Record<string, string>> {
  return baselineOverPaths(await collectEvidencePaths(ev, deps), deps.readFile);
}

async function baselineOverPaths(
  paths: Set<string>,
  readFile: EvidenceDeps["readFile"],
): Promise<Record<string, string>> {
  const baseline: Record<string, string> = {};
  for (const p of paths) {
    const content = await readFile(p);
    if (content !== null) baseline[p] = hashContent(content);
  }
  return baseline;
}
