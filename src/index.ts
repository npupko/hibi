/**
 * The hibi library facade (§7.5) — the in-process surface a JS/TS consumer
 * (e.g. atlas) imports instead of shelling out to the CLI.
 *
 * Architecture — functional core, imperative shell:
 *   • The functional core is `src/engine/*` + `src/store/*`: pure-ish operations
 *     (`planRecord`, `recordClaim`, `runCheck`, `suggest`, `reanchor`, …) that
 *     take already-resolved values and never touch git or argv.
 *   • `Engine` below is the imperative shell for in-process consumers; the CLI
 *     (`src/cli`) is a second, sibling shell. Both are thin and sit on the SAME
 *     core, so the verdict/lifecycle semantics and the returned JSON shapes are
 *     identical by construction, not by convention.
 *
 * Git is a host concern, kept OUT of this core (Mark Seemann's "Dependency
 * Rejection"): the library never blames or resolves a ref, and imports nothing
 * from `src/git/*`. A pure consumer gets the documented defaults (`owner:
 * "unknown"`, `ref: "WORKTREE"`); the CLI shell resolves git first and passes the
 * values in, so the git seam stays clean for a future Rust port (§12/§14-D1).
 * (Out-of-process *resolvers* — §7, default-deny — are a separate, opt-in concern
 * that may spawn a declared subprocess via the resolver client; that is not git,
 * and never runs for a consumer with no resolver manifest.)
 *
 * The model is two-axis (ADR-001): a claim resolves on a doc side AND a code side
 * (`AnchorState`), with an optional behavioral belief (`BehaviorState`). A
 * document is "current" iff none of its verdicts gate — never a single rollup
 * state (§4/§10/§18-C).
 *
 * Scope discipline (§2, §11.4): this exposes hibi's own model and verdicts only.
 * Consumer-specific concepts stay in the consumer; they map *down* onto the
 * Document/Proposition/Assertion/Anchor here, never *into* this core.
 */

import { readFile } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { isBehavioral } from "./algo/behavioral.ts";
import { regionText } from "./algo/localize.ts";
import { type AstAnalyzer, resolveSide } from "./algo/resolve.ts";
import { hashContent } from "./ast/hash.ts";
import { removeBanner } from "./banner/banner.ts";
import type {
  AuthoredTrust,
  BehaviorScope,
  DocumentLifecycle,
  Enforcement,
  Verdict,
  Verifier,
} from "./core/model.ts";
import type { AnchorAnalyzer } from "./engine/anchor.ts";
import { type ArchiveResult, archiveDocument } from "./engine/archive.ts";
import {
  type CheckOptions,
  type CheckReport,
  type FailOn,
  runCheck,
} from "./engine/check.ts";
import { buildDoctorReport, type DoctorReport } from "./engine/doctor.ts";
import {
  buildEvidenceBaselineFor,
  evidenceSetPaths,
  readEvidenceContents,
} from "./engine/evidence.ts";
import { type IgnoreResult, ignoreClaim } from "./engine/ignore.ts";
import { type ListResult, type ListState, toListRows } from "./engine/list.ts";
import { type QueryHit, queryByPath } from "./engine/query.ts";
import {
  type ReanchorInput,
  type ReanchorResult,
  reanchor,
} from "./engine/reanchor.ts";
import {
  type CodeTarget,
  documentIdForPath,
  type RecordContents,
  type RecordInput,
  type RecordResult,
  recordClaim,
  resolveRegion,
} from "./engine/record.ts";
import { planRelocation, type RelocateResult } from "./engine/relocate.ts";
import { type RetireResult, retire } from "./engine/retire.ts";
import {
  type SuggestInput,
  type SuggestResult,
  suggest,
} from "./engine/suggest.ts";
import {
  isLiveClaimOn,
  type RetractResult,
  retract,
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
// ── Public re-exports: the §5 data model + the operation contracts ──
export * from "./core/model.ts";
export type { AnchorAnalyzer } from "./engine/anchor.ts";
export type { ArchiveResult } from "./engine/archive.ts";
export {
  type CheckOptions,
  type CheckReport,
  type CheckSummary,
  computeExitCode,
  type DocumentReport,
  type FailOn,
  type SuspectEntry as CheckSuspectEntry,
} from "./engine/check.ts";
export {
  buildDoctorReport,
  type DoctorReport,
} from "./engine/doctor.ts";
export { type IgnoreResult, ignoreClaim } from "./engine/ignore.ts";
export {
  type ListResult,
  type ListRow,
  type ListSeverity,
  type ListState,
  toListRows,
} from "./engine/list.ts";
export type { QueryHit } from "./engine/query.ts";
export {
  type ReanchorInput,
  type ReanchorResult,
  reanchor as reanchorClaim,
} from "./engine/reanchor.ts";
export {
  type CodeTarget,
  documentIdForPath,
  planRecord,
  type RecordContents,
  type RecordInput,
  type RecordResult,
  type RegionSpec,
  resolveRegion,
} from "./engine/record.ts";
export {
  planRelocation,
  type RelocateResult,
  type RelocationMatch,
  type RelocationMiss,
  type RelocationPlan,
} from "./engine/relocate.ts";
export { type RetireResult, retire as retireClaim } from "./engine/retire.ts";
export {
  type SuggestInput,
  type SuggestResult,
  suggest as suggestClaims,
} from "./engine/suggest.ts";
export {
  amendedPropositions,
  isLiveClaimOn,
  liveClaimsOnDocument,
  type RetractResult,
  type SupersedeInput,
  type SupersedeResult,
} from "./engine/supersede.ts";
export { ClaimStore, STORE_DIR, type StoreLocation } from "./store/store.ts";

// ── Shared engine wiring (the single path the CLI also uses) ──

type Analyzer = AstAnalyzer & AnchorAnalyzer;
let analyzerPromise: Promise<Analyzer | undefined> | undefined;

/**
 * Lazily load the tree-sitter analyzer (Tier-2), memoized per process and shared
 * across every Engine and the CLI. Tier-1 works without it, so a load failure
 * degrades to `undefined` rather than throwing — and is NOT cached, so a later
 * call retries (a transient cold-load hiccup must not strand a long-lived
 * consumer in Tier-1 for the life of the process). The dynamic import keeps WASM
 * off the path of consumers that never check structurally.
 */
export async function loadAnalyzer(): Promise<Analyzer | undefined> {
  if (!analyzerPromise) {
    analyzerPromise = import("./ast/analyzer.ts")
      .then((m) => m.getAnalyzer() as Promise<Analyzer>)
      .catch(() => {
        analyzerPromise = undefined; // let the next call retry
        return undefined;
      });
  }
  return analyzerPromise;
}

/**
 * Build the resolver registry: built-in drift + the built-in command verifier
 * runner + manifest-gated externals (§7/§17.6). `runVerifiers` (default false)
 * gates whether verifiers are dispatched at all — the command runner is always
 * *registered*, but the registry only *invokes* a verifier under the explicit
 * `check --run-verifiers` opt-in (D13 security model).
 */
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

// ── The high-level in-process engine ──

export interface EngineOptions {
  /** Skip the tree-sitter Tier-2 analyzer; Tier-1 text drift still runs. */
  noAst?: boolean;
}

/**
 * A span-first record call (§9 `record`). The documented sentence is located by
 * its own span (`docQuote`/`docRange`/`docLine`) — the doc side of the anchor —
 * and zero or more code targets pin the code it describes. The current artifact
 * span is authoritative; there is no side-channel text override (§18-B, D16).
 */
export interface RecordCall {
  /** Repo-relative path of the document making the claim. */
  docPath: string;
  /** Locate the documented sentence by a literal quote… */
  docQuote?: string;
  /** …or by explicit char offsets / a 1-based line / a 1-based line range. */
  docRange?: {
    start?: number;
    end?: number;
    line?: number;
    startLine?: number;
    endLine?: number;
  };
  /** Optional owned-doc marker id that stabilizes re-anchoring (§4/§8). */
  inlineId?: string;
  /** Zero or more code targets the claim pins. */
  code?: {
    file: string;
    quote?: string;
    start?: number;
    end?: number;
    line?: number;
    startLine?: number;
    endLine?: number;
    /** Coarse (navigational) path anchor instead of a precise region. */
    coarse?: boolean;
    /** Coarse glob anchor (blast-radius). */
    glob?: string;
  }[];
  /** Default `"inferred"`. `"verified"` requires both sides to resolve + a ref. */
  authoredTrust?: AuthoredTrust;
  /** Default `"unknown"` — the library does no git (attribution is a host concern). */
  owner?: string;
  /** The ref last verified against. Default `"WORKTREE"` — the host resolves git. */
  ref?: string;
  ttl?: string;
  /** Explicit enforcement override; else derived (enforced iff verified + resolved). */
  enforcement?: Enforcement;
  /** Author's behavioral declaration (§17.6, D12); undefined → heuristic decides. */
  behavioral?: boolean;
  /** Executable-evidence links that upgrade behavioral risk (§5/§17.6). */
  verifiers?: Verifier[];
  /** Deterministic blast-radius for the behavioral change-gate (§5/§17.6). */
  behaviorScope?: BehaviorScope;
  /** Mark the document pristine — hibi never stamps it (§8, D17). */
  pristine?: boolean;
  attrs?: Record<string, unknown>;
}

/** One suspect claim in a status read — side-tagged banner status (§9). */
export interface SuspectEntry {
  propositionId: string;
  status: string;
}

/**
 * Read-time view of one document — the `status` verb as data. `current` is the
 * two-axis answer: a document is current iff NONE of its verdicts gate. There is
 * no single rollup state (ADR-001); the per-side answers live in `verdicts`.
 */
export interface StatusResult {
  doc: string;
  found: boolean;
  lifecycle: DocumentLifecycle | null;
  /** No verdict gates the build (§9). */
  current: boolean;
  suspect: SuspectEntry[];
  verdicts: Verdict[];
}

/** Build a RegionSpec from a doc/code locator, or undefined when none is given. */
function toRegionSpec(c: {
  quote?: string;
  start?: number;
  end?: number;
  line?: number;
  startLine?: number;
  endLine?: number;
}): CodeTarget["region"] {
  if (
    c.quote === undefined &&
    c.start === undefined &&
    c.end === undefined &&
    c.line === undefined &&
    c.startLine === undefined &&
    c.endLine === undefined
  ) {
    return undefined;
  }
  return {
    quote: c.quote,
    start: c.start,
    end: c.end,
    line: c.line,
    startLine: c.startLine,
    endLine: c.endLine,
  };
}

/**
 * An open claim store with the engine wiring attached — the in-process shell over
 * the functional core. Methods mirror the §9 verbs and return the same shapes the
 * CLI emits. Git-derived inputs (`owner`, `ref`) are accepted as values; omitted,
 * they take the documented git-free defaults (the library never shells out to git).
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

  /** Read a repo-relative file from the anchor root, or `null` if it's missing. */
  private async readAnchored(rel: string): Promise<string | null> {
    const abs = isAbsolute(rel) ? rel : join(this.store.anchorRoot, rel);
    try {
      return await readFile(abs, "utf8");
    } catch (e) {
      // Only a missing file degrades to null (the resolve layer maps that to
      // `orphaned`); surface EISDIR/EACCES/etc. so the real cause is never masked.
      if ((e as { code?: string }).code === "ENOENT") return null;
      throw e;
    }
  }

  /**
   * Read a *document* with hibi's own banner stripped out. Anchoring (record /
   * reanchor) and candidate scanning (suggest) must see the real prose, never the
   * stamped banner — which restates the documented sentence verbatim and would
   * otherwise let a re-anchored quote latch onto the banner copy and self-orphan
   * on the next check (the same hazard `check` guards against — §8/§18-B).
   */
  private async readDoc(rel: string): Promise<string | null> {
    const raw = await this.readAnchored(rel);
    if (raw === null) return null;
    const nonce = (await this.store.config()).nonce;
    return removeBanner(raw, rel, nonce).content;
  }

  /**
   * Verify every claim against the working tree (§9 `check`). Banners are stamped
   * into documents only when `write` is set; otherwise this is a pure read that
   * returns verdicts as data — the mode a consumer rendering its own status uses.
   * Pass `onlyFiles` to scope the check (the write-time loop / `diff`).
   */
  async check(
    opts: {
      write?: boolean;
      failOn?: FailOn;
      onlyFiles?: Iterable<string>;
      ref?: string;
      /**
       * Execute declared verifiers (§17.6, D13). Default false. Only the
       * `check --run-verifiers` path sets it; `status`/`query`/`list`/`doctor`
       * never do, so no verifier process spawns outside this opt-in.
       */
      runVerifiers?: boolean;
      /** Per-verifier timeout in ms (default 120s). */
      verifierTimeoutMs?: number;
    } = {},
  ): Promise<CheckReport> {
    const analyzer = await this.analyzer();
    const registry = await buildRegistry(this.store, analyzer, {
      runVerifiers: opts.runVerifiers ?? false,
      verifierTimeoutMs: opts.verifierTimeoutMs,
    });
    try {
      const options: CheckOptions = {
        // The analyzer is also handed to `check` directly: it drives the
        // change-gate's import extraction (§17.6, D14), separately from the
        // registry's anchor-resolution analyzer.
        ast: analyzer,
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

  /**
   * Read-time "is this current?" gate for one document (§9 `status`). Scoped to
   * the document's own anchored files — both the doc file and every code file its
   * claims pin — so answering it costs work proportional to the one document, not
   * the whole store. `current` is true iff no verdict gates (ADR-001 two-axis;
   * never a rollup state).
   */
  async status(
    docPath: string,
    opts: { ref?: string } = {},
  ): Promise<StatusResult> {
    const docId = documentIdForPath(docPath);
    const doc = await this.store.getDocument(docId);
    const assertions = await this.store.allAssertions();
    const onlyFiles = new Set<string>([docPath]);
    for (const a of assertions) {
      if (a.documentId !== docId) continue;
      onlyFiles.add(a.anchor.doc.file);
      for (const bundle of a.anchor.code) onlyFiles.add(bundle.file);
    }
    const report = await this.check({ write: false, ref: opts.ref, onlyFiles });
    const docReport = report.documents.find((d) => d.id === docId);
    const verdicts = report.verdicts.filter((v) => v.documentId === docId);
    return {
      doc: docPath,
      found: Boolean(doc),
      lifecycle: doc?.lifecycle ?? null,
      current: !verdicts.some((v) => v.gates),
      suspect: docReport?.suspect ?? [],
      verdicts,
    };
  }

  /** What claims are anchored to / cover this path? (§9 `query`, before-edit lookup.) */
  async query(path: string): Promise<QueryHit[]> {
    return queryByPath(this.store, path);
  }

  /**
   * Record a claim (§9 `record`). Span-first: the documented sentence is located
   * by its own span on the doc side, and each code target pins the code it
   * describes. The doc file and every code file are read from the store's anchor
   * root into `RecordContents`; the core `recordClaim` resolves the regions,
   * composes the bidirectional anchor, and derives enforcement. Attribution
   * (`owner`) and the verifying `ref` are caller-supplied values; this shell never
   * derives them from git.
   */
  async record(call: RecordCall): Promise<RecordResult> {
    const code: CodeTarget[] = (call.code ?? []).map((c) => ({
      file: c.file,
      region: toRegionSpec(c),
      coarse: c.coarse,
      glob: c.glob,
    }));

    // Read the doc (banner stripped) + every non-coarse, non-glob code file from
    // disk into the RecordContents the core consumes (file-missing → null).
    const docContent = await this.readDoc(call.docPath);
    const codeContents: Record<string, string | null> = {};
    for (const target of code) {
      if (target.coarse || target.glob) continue;
      if (target.file in codeContents) continue;
      codeContents[target.file] = await this.readAnchored(target.file);
    }

    const analyzer = await this.analyzer();

    // Capture the change-gate baseline for a behavioral claim (§17.6, D14): the
    // shell owns FS + analyzer, so it computes the evidence set here and hands
    // the baseline to the pure core. Non-behavioral claims carry no baseline.
    const docText = this.docTextForClassify(docContent, call);
    const hasVerifiers = (call.verifiers?.length ?? 0) > 0;
    const seeds = code.filter((t) => !t.glob).map((t) => t.file);
    const evidenceBaseline =
      seeds.length > 0 && isBehavioral(call.behavioral, docText, hasVerifiers)
        ? await buildEvidenceBaselineFor(
            {
              seeds,
              behaviorScope: call.behaviorScope,
              verifiers: call.verifiers,
            },
            {
              analyzer,
              readFile: (rel) => this.readAnchored(rel),
              root: this.store.anchorRoot,
            },
          )
        : undefined;

    const contents: RecordContents = { docContent, codeContents };
    const input: RecordInput = {
      docPath: call.docPath,
      docSpec:
        call.docQuote !== undefined ? { quote: call.docQuote } : call.docRange,
      inlineId: call.inlineId,
      authoredTrust: call.authoredTrust ?? "inferred",
      owner: call.owner ?? "unknown",
      ref: call.ref ?? "WORKTREE",
      ttl: call.ttl,
      code,
      enforcement: call.enforcement,
      pristine: call.pristine,
      behavioral: call.behavioral,
      verifiers: call.verifiers,
      behaviorScope: call.behaviorScope,
      evidenceBaseline,
      // A precise code anchor consults the analyzer; coarse/glob ignore it, but
      // passing it is harmless and lets a mixed call resolve its precise targets.
      analyzer,
      attrs: call.attrs,
    };
    return recordClaim(this.store, contents, input);
  }

  /**
   * Best-effort documented text for behavioral classification at record time: the
   * quote when given, else the resolved range text. A malformed range surfaces
   * later in `recordClaim` (the real error path), so here it just skips capture.
   */
  private docTextForClassify(
    docContent: string | null,
    call: RecordCall,
  ): string | null {
    if (call.docQuote !== undefined) return call.docQuote;
    if (docContent !== null && call.docRange) {
      try {
        return regionText(docContent, resolveRegion(docContent, call.docRange));
      } catch {
        return null;
      }
    }
    return null;
  }

  /**
   * Suggest `suggested` doc-side records for the anchorable, verifiable sentences
   * in a document (§9 `suggest`). Doc-only, never-gating; the agent later pins
   * code targets via `reanchor`. The doc file is read from the anchor root.
   */
  async suggest(docPath: string): Promise<SuggestResult> {
    const docContent = (await this.readDoc(docPath)) ?? "";
    const input: SuggestInput = { docPath };
    return suggest(this.store, docContent, input);
  }

  /**
   * Re-anchor an existing claim against current content (§9 `reanchor`). Provided
   * specs override; else the existing selectors re-localize. Reads the doc + each
   * code file from the anchor root. Returns the post-reanchor per-side states.
   */
  async reanchor(
    claimId: string,
    opts: {
      /** Re-home the doc anchor to a different file (symmetric with --code-file). */
      doc?: string;
      docQuote?: string;
      docRange?: {
        start?: number;
        end?: number;
        line?: number;
        startLine?: number;
        endLine?: number;
      };
      code?: RecordCall["code"];
      ref?: string;
      /** Preview only — compute the result without persisting any write (§9). */
      dryRun?: boolean;
    } = {},
  ): Promise<ReanchorResult> {
    const assertion = await this.store.getAssertion(claimId);
    if (!assertion) throw new Error(`No claim ${claimId} in the store.`);
    const document = await this.store.getDocument(assertion.documentId);

    const code: CodeTarget[] | undefined = opts.code?.map((c) => ({
      file: c.file,
      region: toRegionSpec(c),
      coarse: c.coarse,
      glob: c.glob,
    }));

    // Read the doc side. `--doc` re-homes the anchor onto a new file; otherwise
    // read the assertion's current document. Plus every code file the claim
    // already pins and any replacements.
    const docFile = opts.doc ?? (assertion.anchor.doc.file || document?.path);
    const docContent = docFile ? await this.readDoc(docFile) : null;
    // A relocation target (`--doc`) missing from disk is a wrong path, not an
    // orphan — say so plainly, rather than letting the null read surface
    // downstream as a misleading "orphaned" error.
    if (opts.doc !== undefined && docContent === null) {
      throw new Error(`Document not found on disk: ${opts.doc}`);
    }
    const codeContents: Record<string, string | null> = {};
    for (const bundle of assertion.anchor.code) {
      if (bundle.file in codeContents) continue;
      codeContents[bundle.file] = await this.readAnchored(bundle.file);
    }
    for (const target of code ?? []) {
      if (target.coarse || target.glob) continue;
      if (target.file in codeContents) continue;
      codeContents[target.file] = await this.readAnchored(target.file);
    }

    const analyzer = await this.analyzer();

    // D14/D15 — refresh the change-gate baseline for a behavioral claim (one that
    // has a baseline, or is behavioral by declaration/verifier). Seeds are the
    // new code files (replacement targets if given, else the current ones).
    const isGated =
      assertion.evidenceBaseline !== undefined ||
      assertion.behavioral === true ||
      assertion.verifiers.length > 0;
    const seeds = code
      ? code.filter((t) => !t.glob).map((t) => t.file)
      : assertion.anchor.code
          .filter((b) => !b.selectors.some((s) => s.kind === "glob"))
          .map((b) => b.file);
    const evidenceBaseline =
      isGated && seeds.length > 0
        ? await buildEvidenceBaselineFor(
            {
              seeds,
              behaviorScope: assertion.behaviorScope,
              verifiers: assertion.verifiers,
            },
            {
              analyzer,
              readFile: (rel) => this.readAnchored(rel),
              root: this.store.anchorRoot,
            },
          )
        : undefined;

    const contents: RecordContents = { docContent, codeContents };
    const input: ReanchorInput = {
      claimId,
      docPath: opts.doc,
      docSpec:
        opts.docQuote !== undefined ? { quote: opts.docQuote } : opts.docRange,
      code,
      ref: opts.ref,
      evidenceBaseline,
      analyzer,
      dryRun: opts.dryRun,
    };
    return reanchor(this.store, contents, input);
  }

  /**
   * Re-home every live claim stranded on `fromDoc` to `toDoc` in one pass (§9
   * `relocate`, Tier-1 silent-orphan hardening). A claim is re-homed when its
   * current documented sentence appears verbatim in the destination; the rest are
   * reported as misses for manual `reanchor`/`retire`. Each match rides the same
   * `reanchor` machinery (code-side re-localization, file reads, `--dry-run`).
   */
  async relocate(
    fromDoc: string,
    toDoc: string,
    opts: { dryRun?: boolean; ref?: string } = {},
  ): Promise<RelocateResult> {
    if (fromDoc === toDoc) {
      throw new Error("relocate --from and --to must differ.");
    }
    const toContent = await this.readDoc(toDoc);
    if (toContent === null) {
      throw new Error(`Document not found on disk: ${toDoc}`);
    }

    const fromId = documentIdForPath(fromDoc);
    const fromContent = await this.readDoc(fromDoc);
    const assertions = await this.store.allAssertions();
    const live = assertions.filter((a) => isLiveClaimOn(a, fromId));

    // Each claim's CURRENT documented text: the live span on `--from` if that
    // file still exists, else the proposition's cached sentence (robust to a
    // consolidation that already deleted the old doc).
    const claims = await Promise.all(
      live.map(async (a) => {
        let text: string | undefined;
        if (fromContent !== null) {
          const located = resolveSide(a.anchor.doc, fromContent).region;
          if (located) text = regionText(fromContent, located);
        }
        if (text === undefined) {
          const prop = await this.store.getProposition(a.propositionId);
          text = prop?.textCache ?? "";
        }
        return { claimId: a.id, text };
      }),
    );

    const plan = planRelocation(claims, toContent, toDoc);

    const relocated: RelocateResult["relocated"] = [];
    const misses: RelocateResult["misses"] = plan.misses.map((m) => ({
      claimId: m.claimId,
      reason: m.reason,
    }));

    // Phase 1 — classify every planned match with a NON-writing dry-run reanchor.
    // A claim that would throw (e.g. an orphaned code side) becomes a miss here,
    // before any write. This keeps a real relocate consistent: the commit phase
    // only runs reanchors that already previewed clean, so a single un-relocatable
    // claim can never leave the store partially relocated (and is never silently
    // dropped — it carries its message into `misses`).
    const previews: {
      quote: string;
      claimId: string;
      doc: string;
      code: string;
    }[] = [];
    for (const match of plan.matches) {
      try {
        const result = await this.reanchor(match.claimId, {
          doc: toDoc,
          docQuote: match.quote,
          ref: opts.ref,
          dryRun: true,
        });
        previews.push({
          quote: match.quote,
          claimId: match.claimId,
          doc: result.doc,
          code: result.code,
        });
      } catch (e) {
        misses.push({ claimId: match.claimId, reason: (e as Error).message });
      }
    }

    // A dry-run stops at the preview — report what would move, write nothing.
    if (opts.dryRun) {
      for (const p of previews) {
        relocated.push({ claimId: p.claimId, doc: p.doc, code: p.code });
      }
      return { from: fromDoc, to: toDoc, relocated, misses, dryRun: true };
    }

    // Phase 2 — commit only the claims that previewed clean.
    for (const p of previews) {
      const result = await this.reanchor(p.claimId, {
        doc: toDoc,
        docQuote: p.quote,
        ref: opts.ref,
      });
      relocated.push({
        claimId: p.claimId,
        doc: result.doc,
        code: result.code,
      });
    }

    return { from: fromDoc, to: toDoc, relocated, misses, dryRun: false };
  }

  /** Author an `amends`/`supersedes` edge and derive its reverse (§9 `supersede`). */
  async supersede(input: SupersedeInput): Promise<SupersedeResult> {
    return supersede(this.store, input);
  }

  /**
   * Acknowledge a behavioral `at-risk` you re-verified by hand (§17.6, D14
   * `ignore`). Computes the acknowledged `{path → hash}` map — the current
   * hashes of the currently-changed evidence — and records it plus the required
   * reason on the claim. The suppression lapses automatically when any
   * acknowledged path's hash moves again or a new evidence path appears.
   */
  async ignore(claimId: string, reason: string): Promise<IgnoreResult> {
    const assertion = await this.store.getAssertion(claimId);
    if (!assertion) throw new Error(`No claim ${claimId} in the store.`);
    const analyzer = await this.analyzer();
    const readFile = (rel: string) => this.readAnchored(rel);
    const paths = await evidenceSetPaths(assertion, {
      analyzer,
      readFile,
      root: this.store.anchorRoot,
    });
    const evidence = await readEvidenceContents(paths, readFile);
    // The acknowledged set: every evidence path whose current hash differs from
    // its baseline entry (or that has none) — exactly the currently-changed
    // evidence the at-risk is firing on.
    const baseline = assertion.evidenceBaseline ?? {};
    const acknowledged: Record<string, string> = {};
    for (const [p, content] of evidence) {
      if (content === null) continue;
      const cur = hashContent(content);
      if (baseline[p] === undefined || cur !== baseline[p]) {
        acknowledged[p] = cur;
      }
    }
    return ignoreClaim(this.store, claimId, reason, acknowledged);
  }

  /** Mark a document retracted — the author withdrew it (§9 `retract`). */
  async retract(
    docPath: string,
    opts: { dryRun?: boolean } = {},
  ): Promise<RetractResult> {
    return retract(this.store, docPath, opts);
  }

  /** Move an obsolete document out of the read path, leaving a tombstone (§9 `archive`). */
  async archive(
    docPath: string,
    successorPath?: string,
    opts: { dryRun?: boolean } = {},
  ): Promise<ArchiveResult> {
    return archiveDocument(this.store, docPath, successorPath, opts);
  }

  /**
   * Retire a single claim (§9 `retire`): flip its enforcement to `retired` so it
   * no longer gates/warns. Idempotent — a second call is a no-op success.
   */
  async retire(
    claimId: string,
    opts: { dryRun?: boolean } = {},
  ): Promise<RetireResult> {
    return retire(this.store, claimId, opts);
  }

  /**
   * Store-health report (§9 `doctor`, Tier-1 silent-orphan hardening): the dead
   * state `check` hides — orphaned anchors, `suggested` claims with no precise
   * code side, claims stranded on a lifecycle-flagged document, duplicate
   * propositions. Purely informational; the CLI always exits 0.
   */
  async doctor(opts: { ref?: string } = {}): Promise<DoctorReport> {
    const report = await this.check({ write: false, ref: opts.ref });
    const [assertions, documents, propositions] = await Promise.all([
      this.store.allAssertions(),
      this.store.allDocuments(),
      this.store.allPropositions(),
    ]);
    return buildDoctorReport(report, assertions, documents, propositions);
  }

  /**
   * Triage list (§9 `list`): every tracked claim as a lean row (handle + status
   * + severity + recommended action), filtered by `state`. Built from a live
   * check so it shares the verdict/gating semantics exactly (never cached). The
   * report already carries the per-document lifecycle, so only the assertions
   * need a separate read. `hints: false` (—-no-hints) drops the recommendation.
   */
  async list(
    opts: { state?: ListState; ref?: string; hints?: boolean } = {},
  ): Promise<ListResult> {
    const report = await this.check({ write: false, ref: opts.ref });
    const assertions = await this.store.allAssertions();
    return toListRows(report, assertions, report.documents, {
      state: opts.state ?? "all",
      hints: opts.hints ?? true,
    });
  }
}
