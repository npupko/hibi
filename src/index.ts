/**
 * The hibi library facade: the in-process surface a JS/TS consumer imports
 * instead of shelling out to the CLI.
 *
 * Functional core, imperative shell: `src/engine/*` + `src/store/*` take
 * already-read contents and never touch git or argv. `Engine` is the shell
 * for in-process consumers; the CLI is a sibling shell over the same core, so
 * the verdicts and JSON shapes are identical by construction.
 *
 * Git is a host concern: the library never resolves a ref or a git user. A
 * pure consumer passes `by` and gets `ref: "WORKTREE"`; the CLI resolves git
 * first and passes the values in.
 */

import { readFile } from "node:fs/promises";
import { isAbsolute, join, relative } from "node:path";
import type { AstAnalyzer } from "./algo/resolve.ts";
import type {
  ClaimKind,
  Enforcement,
  Procedure,
  Source,
} from "./core/model.ts";
import type { AnchorAnalyzer } from "./engine/anchor.ts";
import { isCoarseBundle } from "./engine/anchor.ts";
import { type ArchiveResult, archiveDocument } from "./engine/archive.ts";
import { type AuditReport, runAudit } from "./engine/audit.ts";
import {
  type CheckOptions,
  type CheckReport,
  type FailOn,
  runCheck,
} from "./engine/check.ts";
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
  type EvidenceTarget,
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
import { TypeSafeHttp } from "./judge/http.ts";
import { JevJudge } from "./judge/jev.ts";
import type { Judge } from "./judge/types.ts";
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
  AUDIT_KINDS,
  type AuditInput,
  type AuditKind,
  type AuditReport,
  type AuditRow,
  type AuditSummary,
  auditExitCode,
  runAudit,
} from "./engine/audit.ts";
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
  documentIdForPath,
  type EvidenceTarget,
  type RecordContents,
  type RecordInput,
  type RecordResult,
  type RegionSpec,
  resolveRegion,
} from "./engine/record.ts";
export { type DocRegion, splitRegions } from "./engine/regions.ts";
export { type RetireResult, retire as retireClaim } from "./engine/retire.ts";
export {
  isLiveClaimOn,
  type SupersedeInput,
  type SupersedeResult,
} from "./engine/supersede.ts";
export {
  cacheKey,
  canonicalJson,
  JudgeCache,
} from "./judge/cache.ts";
export {
  type ConflictRelation,
  type ConflictResult,
  type ConflictSide,
  choiceConfidence,
  type EvidenceKind,
  type Judge,
  type JudgeResponse,
  JudgeUnavailable,
  noulConfidence,
  type Region as JudgeRegion,
  type RegionKind,
  type SupportsInput,
  type SupportsRelation,
  type SupportsResult,
  type TriageKind,
  type TriageResult,
} from "./judge/types.ts";
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

export interface EngineOptions {
  /** Skip the tree-sitter analyzer; text drift still runs. */
  noAst?: boolean;
}

/** A `record` call: the doc span plus the evidence its kind needs. */
export interface RecordCall {
  docPath: string;
  docQuote?: string;
  docRange?: RegionSpec;
  /** Default `code`. */
  kind?: ClaimKind;
  /** Code spans for `code`, one document span for `reference`, none for `attested`. */
  evidence?: EvidenceTarget[];
  sources?: Source[];
  procedure?: Procedure | null;
  /** The actor on the initial `vouch` attestation, e.g. `human:alice@example.com`. */
  by: string;
  owner?: string;
  /** Default `"WORKTREE"`; the CLI passes git HEAD. */
  ref?: string;
  /** ISO 8601 duration, e.g. `P90D`. Required on `attested`. */
  ttl?: string;
  /** Default `enforced`; `suggested` never gates. */
  enforcement?: Extract<Enforcement, "enforced" | "suggested">;
}

export interface CheckCall {
  failOn?: FailOn;
  onlyFiles?: Iterable<string>;
  /** Scope to one document: its file plus the evidence files its claims pin. */
  doc?: string;
  ref?: string;
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
    options: EngineOptions = {},
  ): Promise<Engine> {
    return new Engine(await ClaimStore.init(location), options);
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

  /** Verify every claim against the working tree. Read-only. */
  async check(opts: CheckCall = {}): Promise<CheckReport> {
    const claims = await this.store.allClaims();
    const evidenceFiles = claims.flatMap((c) =>
      c.evidence.filter((b) => !isCoarseBundle(b)).map((b) => b.file),
    );
    const analyzer = await this.analyzerFor(evidenceFiles);
    let onlyFiles = opts.onlyFiles ? new Set(opts.onlyFiles) : undefined;
    let onlyDocument: string | undefined;
    if (opts.doc !== undefined) {
      const scope = await documentScope(this.store, opts.doc);
      onlyFiles = scope.files;
      onlyDocument = scope.documentId;
    }
    const options: CheckOptions = {
      ast: analyzer,
      failOn: opts.failOn,
      onlyFiles,
      onlyDocument,
      ref: opts.ref,
    };
    return runCheck(this.store, options);
  }

  /** Record a claim. The doc span's text is the claim's sentence. */
  async record(call: RecordCall): Promise<RecordResult> {
    const evidence = call.evidence ?? [];
    const docContent = await this.readAnchored(call.docPath);
    const evidenceContents: Record<string, string | null> = {};
    for (const target of evidence) {
      if (target.coarse) continue;
      if (target.file in evidenceContents) continue;
      evidenceContents[target.file] = await this.readAnchored(target.file);
    }
    const analyzer =
      (call.kind ?? "code") === "code"
        ? await this.analyzerFor(
            evidence.filter((t) => !t.coarse).map((t) => t.file),
          )
        : undefined;
    const contents: RecordContents = { docContent, evidenceContents };
    const input: RecordInput = {
      docPath: call.docPath,
      docSpec:
        call.docQuote !== undefined ? { quote: call.docQuote } : call.docRange,
      kind: call.kind,
      evidence,
      sources: call.sources,
      procedure: call.procedure,
      by: call.by,
      owner: call.owner,
      ref: call.ref ?? "WORKTREE",
      ttl: call.ttl,
      enforcement: call.enforcement,
      analyzer,
    };
    return recordClaim(this.store, contents, input);
  }

  /**
   * Audit a document (SPEC 10). Throws when the document is missing on disk
   * or matches an ignore glob. The default judge is Jev over HTTP with the
   * configured model.
   */
  async audit(
    docPath: string,
    opts: {
      judge?: Judge;
      duplicateOf?: (claimId: string) => string | null;
    } = {},
  ): Promise<AuditReport> {
    const text = await this.readAnchored(docPath);
    if (text === null) {
      throw new Error(`Document not found on disk: ${docPath}`);
    }
    const config = await this.store.config();
    const judge =
      opts.judge ??
      new JevJudge({ model: config.judge.model, http: new TypeSafeHttp() });
    return runAudit({
      store: this.store,
      judge,
      docPath,
      text,
      claims: await this.store.allClaims(),
      config,
      duplicateOf: opts.duplicateOf,
    });
  }

  /** Re-anchor an existing claim against current content. */
  async reanchor(
    claimId: string,
    opts: {
      doc?: string;
      docQuote?: string;
      docRange?: RegionSpec;
      evidence?: EvidenceTarget[];
      ref?: string;
      dryRun?: boolean;
    } = {},
  ): Promise<ReanchorResult> {
    const claim = await this.store.getClaim(claimId);
    if (!claim) throw new Error(`No claim ${claimId} in the store.`);
    const document = await this.store.getDocument(claim.documentId);

    const docFile = opts.doc ?? (claim.doc.file || document?.path);
    const docContent = docFile ? await this.readAnchored(docFile) : null;
    if (opts.doc !== undefined && docContent === null) {
      throw new Error(`Document not found on disk: ${opts.doc}`);
    }
    const evidenceContents: Record<string, string | null> = {};
    for (const bundle of claim.evidence) {
      if (bundle.file in evidenceContents) continue;
      evidenceContents[bundle.file] = await this.readAnchored(bundle.file);
    }
    for (const target of opts.evidence ?? []) {
      if (target.coarse) continue;
      if (target.file in evidenceContents) continue;
      evidenceContents[target.file] = await this.readAnchored(target.file);
    }
    const analyzer = await this.analyzerFor(Object.keys(evidenceContents));

    const contents: RecordContents = { docContent, evidenceContents };
    const input: ReanchorInput = {
      claimId,
      docPath: opts.doc,
      docSpec:
        opts.docQuote !== undefined ? { quote: opts.docQuote } : opts.docRange,
      evidence: opts.evidence,
      ref: opts.ref,
      analyzer,
      dryRun: opts.dryRun,
    };
    return reanchor(this.store, contents, input);
  }

  /**
   * Candidate locations for a claim's stored quotes: the doc quote across every
   * registered document, and each evidence quote across files of the same
   * extension (and language, for code). Read-only.
   */
  async reanchorSuggest(claimId: string): Promise<ReanchorSuggestResult> {
    const claim = await this.store.getClaim(claimId);
    if (!claim) throw new Error(`No claim ${claimId} in the store.`);
    const docs: { path: string; content: string }[] = [];
    for (const d of await this.store.allDocuments()) {
      const content = await this.readAnchored(d.path);
      if (content === null) continue;
      docs.push({ path: d.path, content });
    }
    const candidates: ReanchorCandidate[] = suggestForBundle(
      "doc",
      claim.doc,
      docs,
    );
    for (const bundle of claim.evidence) {
      if (isCoarseBundle(bundle)) continue;
      const language = languageForFile(bundle.file);
      const files = await this.sameLanguageFiles(language, bundle.file);
      candidates.push(...suggestForBundle("evidence", bundle, files));
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
        readDoc: (rel) => this.readAnchored(rel),
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
    const report = await this.check({ ref: opts.ref });
    const claims = await this.store.allClaims();
    return toListRows(report, claims, report.documents, {
      state: opts.state ?? "all",
      path: opts.path,
      hints: opts.hints ?? true,
    });
  }
}
