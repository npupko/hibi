/**
 * The hibi library facade: the in-process surface a JS/TS consumer imports
 * instead of shelling out to the CLI.
 *
 * Functional core, imperative shell: `src/engine/*` + `src/store/*` take
 * already-read contents and never touch git or argv. `Engine` is the shell
 * for in-process consumers; the CLI is a sibling shell over the same core, so
 * the verdicts and JSON shapes are identical by construction.
 *
 * Git is a host concern: the library never resolves a ref. A pure consumer
 * gets the defaults (`owner: "unknown"`, `ref: "WORKTREE"`); the CLI resolves
 * git first and passes the values in.
 */

import { readFile } from "node:fs/promises";
import { isAbsolute, join, relative } from "node:path";
import type { AstAnalyzer } from "./algo/resolve.ts";
import type { AnchorAnalyzer } from "./engine/anchor.ts";
import { isCoarseBundle } from "./engine/anchor.ts";
import { type ArchiveResult, archiveDocument } from "./engine/archive.ts";
import {
  type CheckOptions,
  type CheckReport,
  type FailOn,
  runCheck,
  stripEngineOwned,
} from "./engine/check.ts";
import { type CoverageResult, coverage } from "./engine/coverage.ts";
import { languageForFile } from "./engine/lang.ts";
import { type ListResult, type ListState, toListRows } from "./engine/list.ts";
import {
  type ReanchorCandidate,
  type ReanchorInput,
  type ReanchorResult,
  type ReanchorSuggestResult,
  reanchor,
  suggestForBundle,
} from "./engine/reanchor.ts";
import {
  type CodeTarget,
  type RecordContents,
  type RecordInput,
  type RecordResult,
  type RegionSpec,
  recordClaim,
} from "./engine/record.ts";
import { type RetireResult, retire } from "./engine/retire.ts";
import { documentScope } from "./engine/status.ts";
import {
  type SupersedeInput,
  type SupersedeResult,
  supersede,
} from "./engine/supersede.ts";
import { CommandRunnerResolver } from "./resolver/builtin/command-runner.ts";
import { DriftResolver, ResolverRegistry } from "./resolver/registry.ts";
import { ClaimStore, type StoreLocation } from "./store/store.ts";

export type { AstAnalyzer } from "./algo/resolve.ts";
export {
  computeGates,
  isGatingAnchor,
  isWarnAnchor,
  isWarnVerdict,
} from "./core/gating.ts";
export * from "./core/model.ts";
export type { AnchorAnalyzer } from "./engine/anchor.ts";
export type { ArchiveResult } from "./engine/archive.ts";
export {
  type CheckOptions,
  type CheckReport,
  type CheckSummary,
  computeExitCode,
  type DocumentReport,
  FAIL_ON,
  type FailOn,
  type SuspectEntry as CheckSuspectEntry,
} from "./engine/check.ts";
export {
  type CoverageInput,
  type CoverageRegion,
  type CoverageResult,
  type CoverageSummary,
  coverage as docCoverage,
} from "./engine/coverage.ts";
export {
  LIST_STATES,
  type ListResult,
  type ListRow,
  type ListSeverity,
  type ListState,
  toListRows,
} from "./engine/list.ts";
export {
  type ReanchorCandidate,
  type ReanchorInput,
  type ReanchorResult,
  type ReanchorSuggestResult,
  reanchor as reanchorClaim,
} from "./engine/reanchor.ts";
export {
  type CodeTarget,
  documentIdForPath,
  type RecordContents,
  type RecordInput,
  type RecordResult,
  type RegionSpec,
  resolveRegion,
} from "./engine/record.ts";
export { type RetireResult, retire as retireClaim } from "./engine/retire.ts";
export {
  isLiveClaimOn,
  type SupersedeInput,
  type SupersedeResult,
} from "./engine/supersede.ts";
export { ClaimStore, STORE_DIR, type StoreLocation } from "./store/store.ts";

// ── Shared engine wiring ──

type Analyzer = AstAnalyzer &
  AnchorAnalyzer & { loadForFiles(files: Iterable<string>): Promise<void> };
let analyzerPromise: Promise<Analyzer | undefined> | undefined;

/**
 * Lazily load the tree-sitter analyzer, memoized per process. Grammars load
 * per language on demand (`loadForFiles`). A load failure degrades to
 * `undefined` and is not cached, so a later call retries.
 */
export async function loadAnalyzer(): Promise<Analyzer | undefined> {
  if (!analyzerPromise) {
    analyzerPromise = import("./ast/analyzer.ts")
      .then((m) => m.getAnalyzer([]) as Promise<Analyzer>)
      .catch(() => {
        analyzerPromise = undefined;
        return undefined;
      });
  }
  return analyzerPromise;
}

async function buildRegistry(
  store: ClaimStore,
  analyzer?: AstAnalyzer,
  opts: { runVerifiers?: boolean; verifierTimeoutMs?: number } = {},
): Promise<ResolverRegistry> {
  const registry = new ResolverRegistry();
  registry.runVerifiers = opts.runVerifiers ?? false;
  registry.register(new DriftResolver(analyzer));
  registry.register(
    new CommandRunnerResolver(store.anchorRoot, opts.verifierTimeoutMs),
  );
  await registry.loadFromManifest(store);
  return registry;
}

export interface EngineOptions {
  /** Skip the tree-sitter analyzer; text drift still runs. */
  noAst?: boolean;
}

/** A `record` call: the doc span plus zero or more code targets. */
export interface RecordCall {
  docPath: string;
  docQuote?: string;
  docRange?: RegionSpec;
  code?: CodeTarget[];
  /** The author confirmed the code backs the sentence. Default false. */
  verified?: boolean;
  /** Default `"unknown"`. */
  owner?: string;
  /** Default `"WORKTREE"`; the CLI passes git HEAD. */
  ref?: string;
  ttl?: string;
  /** Default `enforced`; `suggested` is advisory. */
  enforcement?: "enforced" | "suggested";
  verifiers?: RecordInput["verifiers"];
  attrs?: Record<string, unknown>;
}

export interface CheckCall {
  write?: boolean;
  failOn?: FailOn;
  onlyFiles?: Iterable<string>;
  /** Scope to one document: its file plus the code files its claims pin. */
  doc?: string;
  ref?: string;
  /** Execute declared verifiers (repo-committed commands). Default false. */
  runVerifiers?: boolean;
  verifierTimeoutMs?: number;
}

const SKIP_DIRS = new Set(["node_modules", ".git", ".claims", "dist"]);

export class Engine {
  private constructor(
    readonly store: ClaimStore,
    private readonly options: EngineOptions,
  ) {}

  static async open(
    location: string | StoreLocation,
    options: EngineOptions = {},
  ): Promise<Engine> {
    return new Engine(await ClaimStore.open(location), options);
  }

  static async init(
    location: string | StoreLocation,
    options: EngineOptions & { nonce?: string } = {},
  ): Promise<Engine> {
    return new Engine(await ClaimStore.init(location, options.nonce), options);
  }

  /** The analyzer with the grammars for `files` loaded, or undefined under `noAst`. */
  private async analyzerFor(
    files: Iterable<string>,
  ): Promise<Analyzer | undefined> {
    if (this.options.noAst) return undefined;
    const analyzer = await loadAnalyzer();
    if (analyzer) await analyzer.loadForFiles(files);
    return analyzer;
  }

  /** Read a repo-relative file, or null when missing. Other I/O errors surface. */
  private async readAnchored(rel: string): Promise<string | null> {
    const abs = isAbsolute(rel) ? rel : join(this.store.anchorRoot, rel);
    try {
      return await readFile(abs, "utf8");
    } catch (e) {
      if ((e as { code?: string }).code === "ENOENT") return null;
      throw e;
    }
  }

  /** Read a document with hibi's own banner stripped, the coordinate space anchors use. */
  private async readDoc(rel: string): Promise<string | null> {
    const raw = await this.readAnchored(rel);
    if (raw === null) return null;
    const nonce = (await this.store.config()).nonce;
    return stripEngineOwned(raw, rel, nonce);
  }

  /** Verify every claim against the working tree. Banners are stamped only when `write` is set. */
  async check(opts: CheckCall = {}): Promise<CheckReport> {
    const assertions = await this.store.allAssertions();
    const codeFiles = assertions.flatMap((a) =>
      a.anchor.code.filter((b) => !isCoarseBundle(b)).map((b) => b.file),
    );
    const analyzer = await this.analyzerFor(codeFiles);
    const registry = await buildRegistry(this.store, analyzer, {
      runVerifiers: opts.runVerifiers ?? false,
      verifierTimeoutMs: opts.verifierTimeoutMs,
    });
    try {
      let onlyFiles = opts.onlyFiles ? new Set(opts.onlyFiles) : undefined;
      let onlyDocument: string | undefined;
      if (opts.doc !== undefined) {
        const scope = await documentScope(this.store, opts.doc);
        onlyFiles = scope.files;
        onlyDocument = scope.documentId;
      }
      const options: CheckOptions = {
        ast: analyzer,
        registry,
        write: opts.write ?? false,
        failOn: opts.failOn,
        onlyFiles,
        onlyDocument,
        ref: opts.ref,
      };
      return await runCheck(this.store, options);
    } finally {
      registry.dispose();
    }
  }

  /** Record a claim. The doc span's text is the claim; code targets pin the code it describes. */
  async record(call: RecordCall): Promise<RecordResult> {
    const code = call.code ?? [];
    const docContent = await this.readDoc(call.docPath);
    const codeContents: Record<string, string | null> = {};
    for (const target of code) {
      if (target.coarse) continue;
      if (target.file in codeContents) continue;
      codeContents[target.file] = await this.readAnchored(target.file);
    }
    const analyzer = await this.analyzerFor(
      code.filter((t) => !t.coarse).map((t) => t.file),
    );
    const contents: RecordContents = { docContent, codeContents };
    const input: RecordInput = {
      docPath: call.docPath,
      docSpec:
        call.docQuote !== undefined ? { quote: call.docQuote } : call.docRange,
      verified: call.verified ?? false,
      owner: call.owner ?? "unknown",
      ref: call.ref ?? "WORKTREE",
      ttl: call.ttl,
      code,
      enforcement: call.enforcement,
      verifiers: call.verifiers,
      analyzer,
      attrs: call.attrs,
    };
    return recordClaim(this.store, contents, input);
  }

  /** Doc-side coverage of a document. Throws when the document is missing on disk. */
  async coverage(docPath: string): Promise<CoverageResult> {
    const docContent = await this.readDoc(docPath);
    if (docContent === null) {
      throw new Error(`Document not found on disk: ${docPath}`);
    }
    return coverage(this.store, docContent, { docPath });
  }

  /** Re-anchor an existing claim against current content. */
  async reanchor(
    claimId: string,
    opts: {
      doc?: string;
      docQuote?: string;
      docRange?: RegionSpec;
      code?: CodeTarget[];
      ref?: string;
      dryRun?: boolean;
    } = {},
  ): Promise<ReanchorResult> {
    const assertion = await this.store.getAssertion(claimId);
    if (!assertion) throw new Error(`No claim ${claimId} in the store.`);
    const document = await this.store.getDocument(assertion.documentId);

    const docFile = opts.doc ?? (assertion.anchor.doc.file || document?.path);
    const docContent = docFile ? await this.readDoc(docFile) : null;
    if (opts.doc !== undefined && docContent === null) {
      throw new Error(`Document not found on disk: ${opts.doc}`);
    }
    const codeContents: Record<string, string | null> = {};
    for (const bundle of assertion.anchor.code) {
      if (bundle.file in codeContents) continue;
      codeContents[bundle.file] = await this.readAnchored(bundle.file);
    }
    for (const target of opts.code ?? []) {
      if (target.coarse) continue;
      if (target.file in codeContents) continue;
      codeContents[target.file] = await this.readAnchored(target.file);
    }
    const analyzer = await this.analyzerFor(Object.keys(codeContents));

    const contents: RecordContents = { docContent, codeContents };
    const input: ReanchorInput = {
      claimId,
      docPath: opts.doc,
      docSpec:
        opts.docQuote !== undefined ? { quote: opts.docQuote } : opts.docRange,
      code: opts.code,
      ref: opts.ref,
      analyzer,
      dryRun: opts.dryRun,
    };
    return reanchor(this.store, contents, input);
  }

  /**
   * Candidate locations for a claim's stored quotes: the doc quote across every
   * registered document, and each code quote across files of the same language.
   * Read-only.
   */
  async reanchorSuggest(claimId: string): Promise<ReanchorSuggestResult> {
    const assertion = await this.store.getAssertion(claimId);
    if (!assertion) throw new Error(`No claim ${claimId} in the store.`);
    const docs: { path: string; content: string }[] = [];
    for (const d of await this.store.allDocuments()) {
      const content = await this.readDoc(d.path);
      if (content === null) continue;
      docs.push({ path: d.path, content });
    }
    const candidates: ReanchorCandidate[] = suggestForBundle(
      "doc",
      assertion.anchor.doc,
      docs,
    );
    for (const bundle of assertion.anchor.code) {
      if (isCoarseBundle(bundle)) continue;
      const language = languageForFile(bundle.file);
      const files = await this.sameLanguageFiles(language, bundle.file);
      candidates.push(...suggestForBundle("code", bundle, files));
    }
    return { action: "reanchor-suggest", claimId, candidates };
  }

  /** Files under the anchor root that share `language` (by extension), read into memory. */
  private async sameLanguageFiles(
    language: string | undefined,
    sample: string,
  ): Promise<{ path: string; content: string }[]> {
    const out: { path: string; content: string }[] = [];
    const ext = sample.includes(".")
      ? sample.slice(sample.lastIndexOf("."))
      : "";
    if (ext === "") return out;
    const glob = new Bun.Glob(`**/*${ext}`);
    for await (const abs of glob.scan({
      cwd: this.store.anchorRoot,
      absolute: true,
      onlyFiles: true,
      dot: false,
    })) {
      const rel = relative(this.store.anchorRoot, abs);
      if (rel.split("/").some((seg) => SKIP_DIRS.has(seg))) continue;
      if (language !== undefined && languageForFile(rel) !== language) continue;
      const content = await this.readAnchored(rel);
      if (content !== null) out.push({ path: rel, content });
    }
    return out;
  }

  /** Supersede a document and relocate its live claims to the successor. */
  async supersede(input: SupersedeInput): Promise<SupersedeResult> {
    return supersede(
      this.store,
      {
        readDoc: (rel) => this.readDoc(rel),
        reanchor: (id, o) => this.reanchor(id, o),
      },
      input,
    );
  }

  /** Move an obsolete document out of the read path, leaving a tombstone. */
  async archive(
    docPath: string,
    successorPath?: string,
    opts: { dryRun?: boolean } = {},
  ): Promise<ArchiveResult> {
    return archiveDocument(this.store, docPath, successorPath, opts);
  }

  /** Withdraw a claim: enforcement → `retired`. Idempotent. */
  async retire(
    claimId: string,
    opts: { dryRun?: boolean } = {},
  ): Promise<RetireResult> {
    return retire(this.store, claimId, opts);
  }

  /** Triage rows from a live check, filtered by state and path. */
  async list(
    opts: {
      state?: ListState;
      path?: string;
      ref?: string;
      hints?: boolean;
    } = {},
  ): Promise<ListResult> {
    const report = await this.check({ write: false, ref: opts.ref });
    const [assertions, propositions] = await Promise.all([
      this.store.allAssertions(),
      this.store.allPropositions(),
    ]);
    return toListRows(report, assertions, propositions, report.documents, {
      state: opts.state ?? "all",
      path: opts.path,
      hints: opts.hints ?? true,
    });
  }
}
