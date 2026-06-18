/**
 * The hibi library facade — the curated, in-process public surface (§7.5).
 *
 * This is the contract a JS/TS consumer (e.g. atlas) imports instead of shelling
 * out to the CLI. It is deliberately *contract-equal* to the CLI (§9): every
 * method returns the same typed object the CLI serializes to JSON, and the
 * verdict/lifecycle semantics are identical because both paths share the same
 * engine wiring below. Import this module — never `src/engine/*` internals — so a
 * future Rust port (§12/§14-D1) can keep serving consumers through the CLI/JSON
 * contract without breaking them.
 *
 * Scope discipline (§2, §11.4): this exposes hibi's own model and verdicts only.
 * Consumer-specific concepts stay in the consumer; they map *down* onto the
 * Document/Proposition/Assertion/Anchor here, never *into* this core.
 */

import { readFile } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import type { AstAnalyzer } from "./algo/resolve.ts";
import type {
  AuthoredTrust,
  Document,
  DocumentLifecycle,
  Verdict,
} from "./core/model.ts";
import type { AnchorAnalyzer } from "./engine/anchor.ts";
import { type ArchiveResult, archiveDocument } from "./engine/archive.ts";
import {
  type CheckOptions,
  type CheckReport,
  type FailOn,
  runCheck,
} from "./engine/check.ts";
import { type QueryHit, queryByPath } from "./engine/query.ts";
import {
  documentIdForPath,
  type RecordResult,
  recordClaim,
  resolveRegion,
} from "./engine/record.ts";
import {
  retract,
  type SupersedeInput,
  type SupersedeResult,
  supersede,
} from "./engine/supersede.ts";
import { DriftResolver, ResolverRegistry } from "./resolver/registry.ts";
import { ClaimStore, type StoreLocation } from "./store/store.ts";

export type { AstAnalyzer } from "./algo/resolve.ts";
// ── Public re-exports: the §5 data model + the operation contracts ──
export * from "./core/model.ts";
export type { AnchorAnalyzer } from "./engine/anchor.ts";
export type { ArchiveResult } from "./engine/archive.ts";
export type {
  CheckOptions,
  CheckReport,
  DocumentReport,
  FailOn,
} from "./engine/check.ts";
export type { QueryHit } from "./engine/query.ts";
export {
  documentIdForPath,
  type RecordInput,
  type RecordResult,
  resolveRegion,
} from "./engine/record.ts";
export {
  amendedPropositions,
  type SupersedeInput,
  type SupersedeResult,
} from "./engine/supersede.ts";
export { ClaimStore, STORE_DIR, type StoreLocation } from "./store/store.ts";

// ── Shared engine wiring (the single path the CLI also uses) ──

type Analyzer = AstAnalyzer & AnchorAnalyzer;
let analyzerPromise: Promise<Analyzer | undefined> | undefined;

/**
 * Lazily load the tree-sitter analyzer (Tier-2), once per process and shared
 * across every Engine and the CLI. Tier-1 works without it, so a load failure
 * degrades to `undefined` rather than throwing. Loading it from here (a dynamic
 * import) keeps WASM off the path of consumers that never check structurally.
 */
export async function loadAnalyzer(): Promise<Analyzer | undefined> {
  if (!analyzerPromise) {
    analyzerPromise = import("./ast/analyzer.ts")
      .then((m) => m.getAnalyzer() as Promise<Analyzer>)
      .catch(() => undefined);
  }
  return analyzerPromise;
}

/** Build the resolver registry: built-in drift + manifest-gated externals (§7). */
export async function buildRegistry(
  store: ClaimStore,
  analyzer?: AstAnalyzer,
): Promise<ResolverRegistry> {
  const registry = new ResolverRegistry();
  registry.register(new DriftResolver(analyzer));
  await registry.loadFromManifest(store);
  return registry;
}

// ── The high-level in-process engine ──

export interface EngineOptions {
  /** Skip the tree-sitter Tier-2 analyzer; Tier-1 text drift still runs. */
  noAst?: boolean;
}

/** How `Engine.record` locates the region inside the anchored code file. */
export interface RecordCall {
  /** Repo-relative path of the document making the claim. */
  docPath: string;
  /** The proposition text — the timeless meaning. */
  text: string;
  /** Repo-relative path of the anchored code file. */
  codeFile: string;
  /** Locate the region by a literal quote… */
  quote?: string;
  /** …or by explicit char offsets… */
  start?: number;
  end?: number;
  /** …or by a 1-based line number. */
  line?: number;
  /** Record a coarse (navigational, never-stale) path anchor instead. */
  coarse?: boolean;
  /** Default `"inferred"`. `"verified"` requires a precise region + a ref. */
  authoredTrust?: AuthoredTrust;
  /** Default `"unknown"` — the library does no git blame (a CLI concern). */
  owner?: string;
  /** The ref last verified against. Default `"WORKTREE"`. */
  ref?: string;
  ttl?: string;
  attrs?: Record<string, unknown>;
}

/** Read-time view of one document — the `status` verb as data. */
export interface StatusResult {
  doc: string;
  found: boolean;
  lifecycle: DocumentLifecycle | null;
  current: boolean;
  suspect: { propositionId: string; state: string }[];
  verdicts: Verdict[];
}

/**
 * An open claim store with the engine wiring attached — the in-process equivalent
 * of the CLI. Methods mirror the §9 verbs and return the same shapes the CLI emits.
 */
export class Engine {
  private constructor(
    readonly store: ClaimStore,
    private readonly options: EngineOptions,
  ) {}

  /** Open an existing store. `location` is the anchor root, or `{anchorRoot, storeDir}`. */
  static async open(
    location: string | StoreLocation,
    options: EngineOptions = {},
  ): Promise<Engine> {
    return new Engine(await ClaimStore.open(location), options);
  }

  /** Initialize a store (idempotent) and open it. */
  static async init(
    location: string | StoreLocation,
    options: EngineOptions & { nonce?: string } = {},
  ): Promise<Engine> {
    return new Engine(await ClaimStore.init(location, options.nonce), options);
  }

  private analyzer(): Promise<Analyzer | undefined> {
    return this.options.noAst ? Promise.resolve(undefined) : loadAnalyzer();
  }

  /**
   * Verify every claim against the working tree (§9 `check`). Banners are stamped
   * into documents only when `write` is set; otherwise this is a pure read that
   * returns verdicts as data — the mode a consumer rendering its own status uses.
   */
  async check(
    opts: {
      write?: boolean;
      failOn?: FailOn;
      onlyFiles?: Iterable<string>;
      ref?: string;
    } = {},
  ): Promise<CheckReport> {
    const analyzer = await this.analyzer();
    const registry = await buildRegistry(this.store, analyzer);
    try {
      const options: CheckOptions = {
        registry,
        write: opts.write ?? false,
        failOn: opts.failOn,
        onlyFiles: opts.onlyFiles ? new Set(opts.onlyFiles) : undefined,
        ref: opts.ref,
      };
      return await runCheck(this.store, options);
    } finally {
      registry.dispose();
    }
  }

  /** Read-time "is this current?" gate for one document (§9 `status`). */
  async status(
    docPath: string,
    opts: { ref?: string } = {},
  ): Promise<StatusResult> {
    const docId = documentIdForPath(docPath);
    const doc = await this.store.getDocument(docId);
    const report = await this.check({ write: false, ref: opts.ref });
    const docReport = report.documents.find((d) => d.id === docId);
    return {
      doc: docPath,
      found: Boolean(doc),
      lifecycle: doc?.lifecycle ?? null,
      current: !docReport || docReport.suspect.length === 0,
      suspect: docReport?.suspect ?? [],
      verdicts: report.verdicts.filter((v) => v.documentId === docId),
    };
  }

  /** What claims are anchored to / cover this path? (§9 `query`, before-edit lookup.) */
  async query(path: string): Promise<QueryHit[]> {
    return queryByPath(this.store, path);
  }

  /**
   * Record a code-anchored claim (§9 `record`). The anchored file is read from the
   * store's anchor root and the region resolved from quote / offsets / line.
   */
  async record(call: RecordCall): Promise<RecordResult> {
    const coarse = call.coarse ?? false;
    let codeContent: string | null = null;
    let region: ReturnType<typeof resolveRegion> | undefined;
    if (!coarse) {
      // Mirror the CLI guard: a precise anchor needs a real file, else the
      // join below resolves to the anchor-root dir and readFile throws EISDIR.
      if (!call.codeFile)
        throw new Error("record requires a codeFile (or set coarse: true)");
      const abs = isAbsolute(call.codeFile)
        ? call.codeFile
        : join(this.store.anchorRoot, call.codeFile);
      codeContent = await readFile(abs, "utf8");
      region = resolveRegion(codeContent, {
        quote: call.quote,
        start: call.start,
        end: call.end,
        line: call.line,
      });
    }
    return recordClaim(this.store, codeContent, {
      docPath: call.docPath,
      text: call.text,
      authoredTrust: call.authoredTrust ?? "inferred",
      owner: call.owner ?? "unknown",
      ref: call.ref ?? "WORKTREE",
      ttl: call.ttl,
      codeFile: coarse ? call.codeFile || call.docPath : call.codeFile,
      region,
      coarse,
      // A coarse (path) anchor never consults the analyzer (buildPathAnchor
      // ignores it), so don't pay the tree-sitter WASM load for it.
      analyzer: coarse ? undefined : await this.analyzer(),
      attrs: call.attrs,
    });
  }

  /** Author an `amends`/`supersedes` edge and derive its reverse (§9 `supersede`). */
  async supersede(input: SupersedeInput): Promise<SupersedeResult> {
    return supersede(this.store, input);
  }

  /** Mark a document retracted — the author withdrew it (§9 `retract`). */
  async retract(docPath: string): Promise<Document> {
    return retract(this.store, docPath);
  }

  /** Move an obsolete document out of the read path, leaving a tombstone (§9 `archive`). */
  async archive(
    docPath: string,
    successorPath?: string,
  ): Promise<ArchiveResult> {
    return archiveDocument(this.store, docPath, successorPath);
  }
}
