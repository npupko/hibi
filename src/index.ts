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
import { type AstAnalyzer, resolveClaimSides } from "./algo/resolve.ts";
import { documentIdForPath } from "./core/ids.ts";
import type {
  Claim,
  ClaimKind,
  Config,
  Enforcement,
  Procedure,
  Source,
} from "./core/model.ts";
import type { AnchorAnalyzer } from "./engine/anchor.ts";
import { type ArchiveResult, archiveDocument } from "./engine/archive.ts";
import {
  type AckTarget,
  type AttestResult,
  attestClaim,
} from "./engine/attest.ts";
import { type AuditReport, runAudit } from "./engine/audit.ts";
import {
  type CheckOptions,
  type CheckReport,
  type FailOn,
  runCheck,
} from "./engine/check.ts";
import {
  type ConflictPassResult,
  duplicateOf,
  runConflictPass,
} from "./engine/conflict.ts";
import { claimContext } from "./engine/context.ts";
import { type EvidenceReport, submitEvidence } from "./engine/evidence.ts";
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
import { referenceTexts } from "./engine/reference.ts";
import {
  assertAliasKnown,
  isAliasedFile,
  type ResolvedFile,
  resolveFile,
} from "./engine/repos.ts";
import { type RetireResult, retire } from "./engine/retire.ts";
import { documentScope } from "./engine/status.ts";
import {
  type SupersedeInput,
  type SupersedeResult,
  supersede,
} from "./engine/supersede.ts";
import { normalizePath } from "./fs.ts";
import { TypeSafeHttp } from "./judge/http.ts";
import { JevJudge } from "./judge/jev.ts";
import type { Judge } from "./judge/types.ts";
import { ClaimStore, type StoreLocation } from "./store/store.ts";

export type { AstAnalyzer } from "./algo/resolve.ts";
export { gateVerdict } from "./core/gating.ts";
export * from "./core/model.ts";
export type { AnchorAnalyzer } from "./engine/anchor.ts";
export type { ArchiveResult } from "./engine/archive.ts";
export {
  type AckTarget,
  type AttestResult,
  parseAck,
} from "./engine/attest.ts";
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
  type ConflictPassResult,
  conflictReviewsFor,
  conflictsFor,
  duplicateOf,
} from "./engine/conflict.ts";
export {
  type EvidenceItem,
  type EvidenceReport,
  type EvidenceResult,
  EvidenceValidationError,
} from "./engine/evidence.ts";
export {
  type DuplicateRef,
  LIST_STATES,
  type ListResult,
  type ListRow,
  type ListState,
  type RowState,
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
export { normalizePath } from "./fs.ts";
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
  JudgeError,
  type JudgeResponse,
  JudgeUnavailable,
  MAX_EVIDENCE_CHARS,
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
  /**
   * The judge for `check`, `audit`, and `evidence`. Default: Jev over HTTP
   * with `config.judge.model`; without `TYPESAFE_API_KEY` every judgment the
   * cache cannot answer is `pending`.
   */
  judge?: Judge;
}

/** A `record` call: the doc span plus the evidence its kind needs. */
export interface RecordCall {
  docPath: string;
  docQuote?: string;
  docRange?: RegionSpec;
  kind: ClaimKind;
  /**
   * Code spans for `code`, one document span for `reference`, none for
   * `attested`. A file may be aliased (`api:src/x.ts`); the alias must be in
   * `config.repos` and checked out.
   */
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

/** Why `record` or `reanchor` refuses an evidence file it cannot read. */
function recordRefusal(
  file: string,
  resolved: Extract<ResolvedFile, { unavailable: true }>,
): Error {
  if (resolved.escapes) {
    return new Error(
      `${file}: ${resolved.reason}; an evidence file must be inside ${resolved.alias === undefined ? "the anchor root" : `the directory of repo alias "${resolved.alias}"`}.`,
    );
  }
  return new Error(
    `${resolved.reason}; a claim is recorded against text on disk, so check out the sibling repo first.`,
  );
}

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

  /** The one judge factory: the injected judge, else Jev over HTTP with the configured model. */
  private judgeFor(config: Config): Judge {
    return (
      this.options.judge ??
      new JevJudge({ model: config.judge.model, http: new TypeSafeHttp() })
    );
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

  /**
   * The `claim_context` function for a set of claims (SPEC 8.3): each claim's
   * heading path and paragraph, read from its document's current text. A
   * claim whose document is missing gets its sentence alone.
   */
  private async contextFor(claims: Claim[]): Promise<(claim: Claim) => string> {
    const texts = new Map<string, string | null>();
    for (const c of claims) {
      if (!texts.has(c.doc.file)) {
        texts.set(c.doc.file, await this.readAnchored(c.doc.file));
      }
    }
    return (claim) => {
      const text = texts.get(claim.doc.file) ?? null;
      return text === null ? claim.sentence : claimContext(text, claim);
    };
  }

  /**
   * The contradiction and duplicate pass (SPEC 9) over the live claims of
   * active documents. Cached pairs cost no judge call. With `documentId`,
   * only pairs with at least one claim on that document are judged
   * (`check --doc`, `audit`); pairing still sees every live claim.
   */
  async conflictPass(documentId?: string): Promise<ConflictPassResult> {
    const config = await this.store.config();
    const active = new Set(
      (await this.store.allDocuments())
        .filter((d) => d.lifecycle === "active")
        .map((d) => d.id),
    );
    const claims = (await this.store.allClaims()).filter(
      (c) => c.enforcement !== "retired" && active.has(c.documentId),
    );
    const scope =
      documentId === undefined
        ? undefined
        : new Set(
            claims.filter((c) => c.documentId === documentId).map((c) => c.id),
          );
    return runConflictPass({
      store: this.store,
      judge: this.judgeFor(config),
      claims,
      context: await this.contextFor(claims),
      threshold: config.judge.threshold,
      scope,
    });
  }

  /** A user-supplied path as the store keys it (`normalizePath`). */
  private norm(p: string): string {
    return normalizePath(this.store.anchorRoot, p);
  }

  /**
   * Verify every claim against the working tree. The contradiction pass runs
   * over the whole store; `doc` and `onlyFiles` scope only the verdicts.
   * Writes nothing but judge cache entries.
   */
  async check(opts: CheckCall = {}): Promise<CheckReport> {
    return (await this.checkWithPass(opts)).report;
  }

  private async checkWithPass(
    opts: CheckCall,
  ): Promise<{ report: CheckReport; pass: ConflictPassResult }> {
    const config = await this.store.config();
    const claims = await this.store.allClaims();
    const evidenceFiles = claims.flatMap((c) => c.evidence.map((b) => b.file));
    const analyzer = await this.analyzerFor(evidenceFiles);
    let onlyFiles = opts.onlyFiles
      ? new Set([...opts.onlyFiles].map((f) => this.norm(f)))
      : undefined;
    let onlyDocument: string | undefined;
    if (opts.doc !== undefined) {
      const doc = this.norm(opts.doc);
      const scope = await documentScope(this.store, doc);
      if (!claims.some((c) => c.documentId === scope.documentId)) {
        throw new Error(
          `no claims are registered for ${doc}; run hibi audit --doc ${doc}`,
        );
      }
      onlyFiles = scope.files;
      onlyDocument = scope.documentId;
    }
    const pass = await this.conflictPass(onlyDocument);
    const options: CheckOptions = {
      ast: analyzer,
      failOn: opts.failOn,
      onlyFiles,
      onlyDocument,
      ref: opts.ref,
      judge: this.judgeFor(config),
      conflictPass: pass,
    };
    return { report: await runCheck(this.store, options), pass };
  }

  /** Record a claim. The doc span's text is the claim's sentence. */
  async record(call: RecordCall): Promise<RecordResult> {
    const config = await this.store.config();
    const evidence = (call.evidence ?? []).map((t) => ({
      ...t,
      file: this.norm(t.file),
    }));
    const docPath = this.norm(call.docPath);
    const docContent = await this.readAnchored(docPath);
    const evidenceContents: Record<string, string | null> = {};
    for (const target of evidence) {
      if (target.file in evidenceContents) continue;
      assertAliasKnown(config, target.file);
      const resolved = resolveFile(config, target.file, this.store.anchorRoot);
      if ("unavailable" in resolved) throw recordRefusal(target.file, resolved);
      evidenceContents[target.file] = await this.readAnchored(resolved.path);
    }
    const analyzer =
      call.kind === "code"
        ? await this.analyzerFor(evidence.map((t) => t.file))
        : undefined;
    const contents: RecordContents = { docContent, evidenceContents };
    const input: RecordInput = {
      docPath,
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
   * or matches an ignore glob. A covered row's `duplicateOf` comes from the
   * contradiction pass, run here on demand.
   */
  async audit(path: string): Promise<AuditReport> {
    const docPath = this.norm(path);
    const text = await this.readAnchored(docPath);
    if (text === null) {
      throw new Error(`Document not found on disk: ${docPath}`);
    }
    const config = await this.store.config();
    const pass = await this.conflictPass(documentIdForPath(docPath));
    return runAudit({
      store: this.store,
      judge: this.judgeFor(config),
      docPath,
      text,
      claims: await this.store.allClaims(),
      config,
      duplicateOf: (id) => duplicateOf(pass, id),
    });
  }

  /**
   * Submit evidence an agent collected (`hibi evidence`): the judge answers
   * `supports` for each item, with the claim's document context.
   */
  async evidence(items: unknown[]): Promise<EvidenceReport> {
    const config = await this.store.config();
    return submitEvidence({
      store: this.store,
      judge: this.judgeFor(config),
      items,
      context: await this.contextFor(await this.store.allClaims()),
      config,
    });
  }

  /** Vouch for a claim, or acknowledge a `review` or `conflict` finding (`hibi attest`). */
  async attest(
    claimId: string,
    opts: { by: string; note?: string; ack?: AckTarget },
  ): Promise<AttestResult> {
    const claim = await this.store.getClaim(claimId);
    let reference: Parameters<typeof attestClaim>[0]["reference"];
    if (claim?.kind === "reference" && opts.ack?.kind === "review") {
      // The verdict `check` judges now: same sides, same request.
      const config = await this.store.config();
      const docText = await this.readAnchored(claim.doc.file);
      const evidence = new Map<string, string | null>();
      const unavailable = new Map<string, string>();
      for (const b of claim.evidence) {
        const r = resolveFile(config, b.file, this.store.anchorRoot);
        if ("unavailable" in r) unavailable.set(b.file, r.reason);
        else evidence.set(b.file, await this.readAnchored(r.path));
      }
      const sides = resolveClaimSides(claim, {
        doc: docText,
        evidence,
        unavailable,
      });
      reference = {
        model: this.judgeFor(config).model,
        current: referenceTexts(claim, sides, docText),
      };
    }
    return attestClaim({ store: this.store, claimId, ...opts, reference });
  }

  /**
   * Re-anchor an existing claim against current content. `evidence` replaces
   * a `code` claim's spans; `reference` replaces a `reference` claim's span.
   * An evidence side in a sibling repo that is not checked out keeps its
   * stored bundle, with a warning.
   */
  async reanchor(
    claimId: string,
    opts: {
      doc?: string;
      docQuote?: string;
      docRange?: RegionSpec;
      evidence?: EvidenceTarget[];
      reference?: EvidenceTarget;
      ref?: string;
      dryRun?: boolean;
    } = {},
  ): Promise<ReanchorResult> {
    const claim = await this.store.getClaim(claimId);
    if (!claim) throw new Error(`No claim ${claimId} in the store.`);
    if (opts.evidence !== undefined && claim.kind !== "code") {
      throw new Error(
        `claim ${claim.id} is a ${claim.kind} claim; --code-file, --code-quote, and --code-range are for code claims${claim.kind === "reference" ? "; pass --ref-file with --ref-quote or --ref-range" : ""}.`,
      );
    }
    if (opts.reference !== undefined && claim.kind !== "reference") {
      throw new Error(
        `claim ${claim.id} is a ${claim.kind} claim; --ref-file, --ref-quote, and --ref-range are for reference claims.`,
      );
    }
    const document = await this.store.getDocument(claim.documentId);

    const newDoc = opts.doc === undefined ? undefined : this.norm(opts.doc);
    const docFile = newDoc ?? (claim.doc.file || document?.path);
    const docContent = docFile ? await this.readAnchored(docFile) : null;
    if (newDoc !== undefined && docContent === null) {
      throw new Error(`Document not found on disk: ${newDoc}`);
    }
    const config = await this.store.config();
    const targets = (
      opts.reference !== undefined ? [opts.reference] : opts.evidence
    )?.map((t) => ({ ...t, file: this.norm(t.file) }));
    const evidenceContents: Record<string, string | null> = {};
    const unavailable = new Map<string, string>();
    if (targets === undefined) {
      for (const bundle of claim.evidence) {
        if (bundle.file in evidenceContents) continue;
        const r = resolveFile(config, bundle.file, this.store.anchorRoot);
        if ("unavailable" in r) {
          unavailable.set(bundle.file, r.reason);
          evidenceContents[bundle.file] = null;
        } else {
          evidenceContents[bundle.file] = await this.readAnchored(r.path);
        }
      }
    }
    for (const target of targets ?? []) {
      if (target.file in evidenceContents) continue;
      assertAliasKnown(config, target.file);
      const r = resolveFile(config, target.file, this.store.anchorRoot);
      if ("unavailable" in r) throw recordRefusal(target.file, r);
      evidenceContents[target.file] = await this.readAnchored(r.path);
    }
    const analyzer = await this.analyzerFor(Object.keys(evidenceContents));

    const contents: RecordContents = {
      docContent,
      evidenceContents,
      unavailable,
    };
    const input: ReanchorInput = {
      claimId,
      docPath: newDoc,
      docSpec:
        opts.docQuote !== undefined ? { quote: opts.docQuote } : opts.docRange,
      evidence: targets,
      ref: opts.ref,
      analyzer,
      dryRun: opts.dryRun,
    };
    return reanchor(this.store, contents, input);
  }

  /**
   * Candidate locations for a claim's stored quotes: the doc quote across every
   * registered document, and each evidence quote across files of the same
   * extension (and language, for code) in this repository. An evidence side
   * in a sibling repo (`alias:path`) gets no candidates. Read-only.
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
      if (isAliasedFile(bundle.file)) continue;
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
      { ...input, from: this.norm(input.from), to: this.norm(input.to) },
    );
  }

  /** Move an obsolete document out of the read path, leaving a tombstone. */
  async archive(
    docPath: string,
    successorPath?: string,
    opts: { dryRun?: boolean } = {},
  ): Promise<ArchiveResult> {
    return archiveDocument(
      this.store,
      this.norm(docPath),
      successorPath === undefined ? undefined : this.norm(successorPath),
      opts,
    );
  }

  /** Withdraw a claim: enforcement → `retired`. Idempotent. */
  async retire(
    claimId: string,
    opts: { dryRun?: boolean } = {},
  ): Promise<RetireResult> {
    return retire(this.store, claimId, opts);
  }

  /** One row per claim from a live check, filtered by state and path. */
  async list(
    opts: { state?: ListState; path?: string; ref?: string } = {},
  ): Promise<ListResult> {
    const { report, pass } = await this.checkWithPass({ ref: opts.ref });
    return toListRows({
      report,
      claims: await this.store.allClaims(),
      pass,
      lifecycle: new Map(
        (await this.store.allDocuments()).map((d) => [d.id, d.lifecycle]),
      ),
      state: opts.state ?? "all",
      path: opts.path === undefined ? undefined : this.norm(opts.path),
    });
  }
}
