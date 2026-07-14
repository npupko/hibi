/**
 * The check engine (§6, §9): walk the store's claims → resolve each
 * **bidirectional** anchor against the current working tree → merge with
 * document lifecycle/supersession → optionally stamp banners → emit a JSON
 * report with a meaningful exit code.
 *
 * Verdicts are recomputed live and never persisted (§6). The report leads with
 * the two-axis decision per claim (`doc`/`code` AnchorState, `behavior`
 * BehaviorState, `expired`/`gates`) and a side-tagged banner status vocabulary
 * (`code:changed`, `doc:orphaned`, `behavior:refuted`, …); the words
 * stale/ghost/drift live only in human banner copy, never in this machine text.
 */

import { readFile, writeFile } from "node:fs/promises";
import { extname, join } from "node:path";
import {
  type AstAnalyzer,
  type ResolveFiles,
  resolveAssertion,
} from "../algo/resolve.ts";
import {
  type BannerAction,
  type BannerEntry,
  DEFAULT_HEADLINE,
  DEFAULT_INSTRUCTION_FILES,
  isInstructionFile,
  removeBanner,
  stampBanner,
} from "../banner/banner.ts";
import { setFrontmatterStatus } from "../banner/frontmatter.ts";
import { isWarnVerdict } from "../core/gating.ts";
import type {
  AnchorState,
  BehaviorState,
  Document,
  DocumentLifecycle,
  Enforcement,
  Proposition,
  Region,
  Verdict,
} from "../core/model.ts";
import { remediationFor } from "../core/remediation.ts";
import { exists } from "../fs.ts";
import type { ResolverRegistry } from "../resolver/registry.ts";
import type { ClaimStore } from "../store/store.ts";
import { evidenceSetPaths, readEvidenceContents } from "./evidence.ts";
import { buildTestFileIndex, suggestTests } from "./test-suggest.ts";

/**
 * Side-tagged status precedence for the single-valued frontmatter status (most
 * severe first, §8). The side prefix is part of the machine vocabulary; the
 * lifecycle terms trail it.
 */
const STATUS_PRECEDENCE = [
  "code:orphaned",
  "doc:orphaned",
  "code:ambiguous",
  "doc:ambiguous",
  "code:changed",
  "doc:changed",
  "behavior:refuted",
  "expired",
  "behavior:at-risk",
  "code:moved",
  "doc:moved",
  "retracted",
  "superseded",
  "amended",
];

const MARKDOWN_EXT = new Set([".md", ".markdown", ".mdx"]);

/** Anchor states that surface as a side-tagged banner status (§8). */
const REPORTABLE_ANCHOR: ReadonlySet<AnchorState> = new Set<AnchorState>([
  "orphaned",
  "ambiguous",
  "changed",
  "moved",
]);

export type FailOn = "gating" | "warn" | "tamper" | "never";

export interface CheckOptions {
  ast?: AstAnalyzer;
  /** Resolve through the registry (built-in + external + advisory) instead of inline. */
  registry?: ResolverRegistry;
  /** Apply banner stamps to documents (the write path). Default: read-only. */
  write?: boolean;
  /** Restrict the check to claims touching a file in this set (doc or code side). */
  onlyFiles?: Set<string>;
  failOn?: FailOn;
  now?: number;
  ref?: string;
}

/** A suspect claim's banner entry: its proposition and the side-tagged status string. */
export interface SuspectEntry {
  propositionId: string;
  status: string;
}

export interface DocumentReport {
  id: string;
  path: string;
  lifecycle: DocumentLifecycle;
  suspect: SuspectEntry[];
  bannerAction?: BannerAction;
  tampered?: boolean;
  /** The machine-readable frontmatter status written (markdown only, §8). */
  frontmatterStatus?: string;
}

/**
 * Two-axis check summary (§9): the gating/warning/clean rollup plus a per-state
 * histogram for each axis. `doc`/`code` count the AnchorState enum; `behavior`
 * counts the BehaviorState enum over behavioral claims only.
 */
export interface CheckSummary {
  total: number;
  gating: number;
  warning: number;
  clean: number;
  doc: Record<AnchorState, number>;
  code: Record<AnchorState, number>;
  behavior: Record<BehaviorState, number>;
  expired: number;
}

export interface CheckReport {
  ref: string;
  verdicts: Verdict[];
  documents: DocumentReport[];
  summary: CheckSummary;
  exitCode: number;
}

function emptyAnchorHistogram(): Record<AnchorState, number> {
  return { unchanged: 0, moved: 0, changed: 0, ambiguous: 0, orphaned: 0 };
}

function emptyBehaviorHistogram(): Record<BehaviorState, number> {
  return { unverified: 0, "at-risk": 0, supported: 0, refuted: 0 };
}

/** Slice the live documented span out of the doc content for a banner entry (§8/§18-B). */
function liveDocText(
  docContent: string | null,
  region: Region | undefined,
): string | null {
  if (docContent === null || region === undefined) return null;
  return docContent.slice(region.start, region.end);
}

/**
 * The worst-per-side, side-tagged status strings a suspect verdict contributes
 * (§8). Each side reports at most its single worst reportable AnchorState; the
 * behavioral axis and the `expired` flag add their own tags. Returns the
 * machine vocabulary only — banner headline copy lives elsewhere.
 */
export function verdictStatuses(v: Verdict): string[] {
  const out: string[] = [];
  if (REPORTABLE_ANCHOR.has(v.code)) out.push(`code:${v.code}`);
  if (REPORTABLE_ANCHOR.has(v.doc)) out.push(`doc:${v.doc}`);
  if (v.behavior === "refuted") out.push("behavior:refuted");
  else if (v.behavior === "at-risk") out.push("behavior:at-risk");
  if (v.expired) out.push("expired");
  return out;
}

/**
 * The single most-severe status for a suspect verdict (the banner entry's tag).
 * A claim suspect *only* because its document changed lifecycle (e.g. amended)
 * has no verdict status of its own, so the document's lifecycle tags are folded
 * in — otherwise the status would wrongly default to `expired`.
 */
export function worstStatus(v: Verdict, lcTags: string[] = []): string {
  const statuses = [...verdictStatuses(v), ...lcTags];
  for (const s of STATUS_PRECEDENCE) if (statuses.includes(s)) return s;
  return statuses[0] ?? "unchanged";
}

/** Lifecycle status tags a document carries from its edges/lifecycle (§6). */
function lifecycleTags(doc: Document): string[] {
  const tags: string[] = [];
  if (doc.lifecycle === "superseded") tags.push("superseded");
  if (doc.lifecycle === "retracted") tags.push("retracted");
  if (doc.lifecycle === "amended") tags.push("amended");
  for (const e of doc.edges) {
    if (e.type === "amended-by" && !tags.includes("amended"))
      tags.push("amended");
  }
  return tags;
}

/** Banner entries contributed by a document's lifecycle (§6 remediation). */
function lifecycleEntries(
  doc: Document,
  propsById: Map<string, Proposition>,
): BannerEntry[] {
  const entries: BannerEntry[] = [];
  const amended = new Set<string>();
  for (const e of doc.edges) {
    if (e.type === "amended-by") for (const p of e.propositions) amended.add(p);
  }
  for (const propId of amended) {
    const p = propsById.get(propId);
    if (p) entries.push({ status: "amended", id: propId, text: p.textCache });
  }
  if (doc.lifecycle === "superseded") {
    entries.push({
      status: "superseded",
      id: doc.id,
      text: `This document has been superseded.`,
    });
  }
  if (doc.lifecycle === "retracted") {
    entries.push({
      status: "retracted",
      id: doc.id,
      text: `The author withdrew this document.`,
    });
  }
  return entries;
}

export async function runCheck(
  store: ClaimStore,
  options: CheckOptions = {},
): Promise<CheckReport> {
  const root = store.anchorRoot;
  const ref = options.ref ?? "WORKTREE";
  const documents = await store.allDocuments();
  const propositions = await store.allPropositions();
  const assertions = await store.allAssertions();

  const propsById = new Map(propositions.map((p) => [p.id, p]));

  // assertionId → enforcement, so the warn predicate can re-read the policy
  // for any verdict without re-fetching the assertion (§9).
  const enforcementById = new Map<string, Enforcement>(
    assertions.map((a) => [a.id, a.enforcement]),
  );

  // The per-repo banner nonce, used to strip hibi's own banner out of a document
  // before resolving its claims (see the doc-side read below).
  const nonce = (await store.config()).nonce;

  // Cache file reads (a regenerable optimization; never affects the verdict).
  const fileCache = new Map<string, string | null>();
  const readFileText = async (rel: string): Promise<string | null> => {
    if (fileCache.has(rel)) return fileCache.get(rel) ?? null;
    const abs = join(root, rel);
    const text = (await exists(abs)) ? await readFile(abs, "utf8") : null;
    fileCache.set(rel, text);
    return text;
  };

  const verdicts: Verdict[] = [];
  /** Doc content per document id, for slicing the live span into banner entries. */
  const docContentById = new Map<string, string | null>();
  /** Document ids that were actually evaluated (≥1 in-scope assertion). */
  const evaluatedDocs = new Set<string>();

  for (const a of assertions) {
    const codeFiles = a.anchor.code.map((b) => b.file);
    if (options.onlyFiles) {
      const touches =
        options.onlyFiles.has(a.anchor.doc.file) ||
        codeFiles.some((f) => options.onlyFiles?.has(f));
      if (!touches) continue;
    }
    evaluatedDocs.add(a.documentId);

    // Read both sides into ResolveFiles (file-missing → orphaned is handled in resolve).
    // Strip hibi's own banner first: the engine-owned banner restates the suspect
    // sentence verbatim, and leaving it in would let the doc-side text-quote
    // re-anchor onto the stamped copy and self-orphan on re-check — the banner
    // must never poison re-anchoring (§8/§18-B). Code files carry no banner.
    const rawDoc = await readFileText(a.anchor.doc.file);
    const docContent =
      rawDoc === null
        ? null
        : removeBanner(rawDoc, a.anchor.doc.file, nonce).content;
    docContentById.set(a.documentId, docContent);
    const code = new Map<string, string | null>();
    for (const f of codeFiles) code.set(f, await readFileText(f));
    const files: ResolveFiles = { doc: docContent, code };

    // Change-gate evidence (§17.6, D14): the current contents of every
    // evidence-set path. The gate reads this map ONLY for a claim carrying a
    // stored baseline or an authored suppression; every other claim (non-
    // behavioral, or behavioral with no baseline → the anchored-node-only
    // fallback) discards it. So skip the whole import walk + globs + reads for
    // those — a large saving on the common check path.
    let evidence: Map<string, string | null> | undefined;
    if (a.evidenceBaseline !== undefined || a.suppressed !== undefined) {
      const paths = await evidenceSetPaths(a, {
        analyzer: options.ast,
        readFile: readFileText,
        root,
      });
      evidence = await readEvidenceContents(paths, readFileText);
    }

    const verdict = options.registry
      ? await options.registry.resolve(
          a,
          files,
          propsById.get(a.propositionId),
          { evidence },
        )
      : resolveAssertion(a, files, {
          ast: options.ast,
          now: options.now,
          evidence,
        });
    verdicts.push(verdict);
  }

  // ── D26 — advisory reverse-import test suggestions ──
  // For a behavioral claim that is `at-risk`/`refuted` and has NO declared
  // verifier, list test files that exercise the anchored code and fold them into
  // the declare-a-verifier remediation rationale. Built lazily: the test-file
  // import index is computed at most once per run, and only when at least one
  // verdict qualifies. Never touches verdicts, states, exit codes, or the store.
  const assertById = new Map(assertions.map((a) => [a.id, a]));
  const qualifies = (v: Verdict): boolean => {
    if (v.behavior !== "at-risk" && v.behavior !== "refuted") return false;
    const a = assertById.get(v.assertionId);
    return a !== undefined && a.verifiers.length === 0;
  };
  if (verdicts.some(qualifies)) {
    const index = await buildTestFileIndex({
      analyzer: options.ast,
      readFile: readFileText,
      root,
    });
    for (const v of verdicts) {
      if (!qualifies(v)) continue;
      const a = assertById.get(v.assertionId);
      if (!a) continue;
      // Union the suggestions across the claim's precise (non-coarse) code files.
      const seen = new Set<string>();
      const tests: string[] = [];
      for (const bundle of a.anchor.code) {
        if (
          bundle.selectors.every((s) => s.kind === "path" || s.kind === "glob")
        )
          continue;
        for (const t of suggestTests(bundle.file, index)) {
          if (!seen.has(t)) {
            seen.add(t);
            tests.push(t);
          }
        }
      }
      tests.sort();
      v.remediation = remediationFor({
        assertionId: v.assertionId,
        doc: v.doc,
        code: v.code,
        behavior: v.behavior,
        expired: v.expired,
        changedEvidence: v.evidence.changedEvidence,
        suggestedTests: tests.slice(0, 3),
      });
    }
  }

  // ── Histograms & rollups ──
  const docHist = emptyAnchorHistogram();
  const codeHist = emptyAnchorHistogram();
  const behaviorHist = emptyBehaviorHistogram();
  let expiredCount = 0;
  let gatingCount = 0;
  let warningCount = 0;

  for (const v of verdicts) {
    docHist[v.doc] += 1;
    codeHist[v.code] += 1;
    if (v.behavior !== undefined) behaviorHist[v.behavior] += 1;
    if (v.expired) expiredCount += 1;
    if (v.gates) gatingCount += 1;
    else if (
      isWarnVerdict(v, enforcementById.get(v.assertionId) ?? "suggested")
    ) {
      warningCount += 1;
    }
  }

  // ── Per-document banner payloads & lifecycle ──
  const verdictsByDoc = new Map<string, Verdict[]>();
  for (const v of verdicts) {
    const list = verdictsByDoc.get(v.documentId) ?? [];
    list.push(v);
    verdictsByDoc.set(v.documentId, list);
  }

  const docReports: DocumentReport[] = [];
  let sawGating = false;
  let sawWarn = false;
  let sawTamper = false;

  for (const doc of documents) {
    // A scoped check (the `diff --since` / write-time loop) only evaluated the
    // documents touching a changed file. Leave every other document — and its
    // existing valid banner — untouched, rather than stripping a banner we never
    // re-verified (§6 write-time loop).
    if (options.onlyFiles && !evaluatedDocs.has(doc.id)) continue;

    const dv = verdictsByDoc.get(doc.id) ?? [];
    const lcTags = lifecycleTags(doc);
    const docContent = docContentById.get(doc.id) ?? null;

    // A claim is suspect (gets a banner entry) iff it gates, warns, or its
    // document carries a lifecycle flag (§8).
    const docHasLifecycle = lcTags.length > 0;
    const suspectVerdicts = dv.filter(
      (v) =>
        v.gates ||
        isWarnVerdict(v, enforcementById.get(v.assertionId) ?? "suggested") ||
        docHasLifecycle,
    );

    const suspectEntries: BannerEntry[] = suspectVerdicts.map((v) => {
      const region = v.evidence.docRegion;
      const text =
        liveDocText(docContent, region) ??
        propsById.get(v.propositionId)?.textCache ??
        "(unknown proposition)";
      return { status: worstStatus(v, lcTags), id: v.propositionId, text };
    });
    const lcEntries = lifecycleEntries(doc, propsById);
    const allEntries = [...suspectEntries, ...lcEntries];

    if (dv.some((v) => v.gates)) sawGating = true;
    if (
      dv.some((v) =>
        isWarnVerdict(v, enforcementById.get(v.assertionId) ?? "suggested"),
      )
    ) {
      sawWarn = true;
    }

    const suspect: SuspectEntry[] = suspectVerdicts
      .map((v) => ({
        propositionId: v.propositionId,
        status: worstStatus(v, lcTags),
      }))
      .sort((a, b) =>
        a.propositionId < b.propositionId
          ? -1
          : a.propositionId > b.propositionId
            ? 1
            : 0,
      );

    const report: DocumentReport = {
      id: doc.id,
      path: doc.path,
      lifecycle: doc.lifecycle,
      suspect,
    };

    // Worst single status for the optional frontmatter field (§8): over both
    // the side-tagged verdict statuses and the lifecycle tags.
    const severities = [
      ...suspectVerdicts.flatMap((v) => verdictStatuses(v)),
      ...lcTags,
    ];
    const statusValue: string | null =
      severities.length > 0
        ? (STATUS_PRECEDENCE.find((s) => severities.includes(s)) ??
          severities[0] ??
          null)
        : null;

    // D17 — pristine docs are never stamped. Evaluated at stamp time: the
    // per-document flag OR a `StoreConfig.pristine` glob matching now — so adding
    // a glob later protects already-recorded docs. Verdicts are still computed
    // and emitted (JSON/status/exit codes); only the banner/frontmatter writes
    // are skipped for a doc hibi does not own.
    const cfg = await store.config();
    const pristine =
      doc.pristine === true ||
      (cfg.pristine ?? []).some((g) => new Bun.Glob(g).match(doc.path));

    if (options.write && !pristine) {
      const abs = join(root, doc.path);
      if (await exists(abs)) {
        const original = await readFile(abs, "utf8");
        const nonce = cfg.nonce;
        const headline =
          suspectEntries.length === 0 && lcEntries.length > 0
            ? `DOCUMENT STATUS — ${lcEntries.length} notice(s) — re-verify before trusting.`
            : DEFAULT_HEADLINE(allEntries.length);

        // D18 — instruction files get the single-line compact banner instead of
        // the full block (attention budget: every extra byte dilutes following).
        const compact = isInstructionFile(
          doc.path,
          cfg.instructionFiles ?? [...DEFAULT_INSTRUCTION_FILES],
        );

        let content = original;
        if (allEntries.length === 0) {
          // Nothing suspect → ensure no lingering banner remains.
          const res = removeBanner(content, doc.path, nonce);
          content = res.content;
          report.bannerAction = res.action;
        } else {
          const res = stampBanner(
            content,
            doc.path,
            { headline, entries: allEntries },
            nonce,
            {
              failOnTamper: options.failOn === "tamper",
              ...(compact
                ? { compact: { count: allEntries.length, docPath: doc.path } }
                : {}),
            },
          );
          if (res.tampered) {
            sawTamper = true;
            report.tampered = true;
          }
          report.bannerAction = res.action;
          // Honor --fail-on tamper: refuse to overwrite a hand-edited banner.
          content =
            res.tampered && options.failOn === "tamper"
              ? original
              : res.content;
        }

        // Optional markdown frontmatter status (§8): only where frontmatter exists.
        if (MARKDOWN_EXT.has(extname(doc.path).toLowerCase())) {
          content = setFrontmatterStatus(content, statusValue);
          report.frontmatterStatus = statusValue ?? undefined;
        }

        if (content !== original) await writeFile(abs, content);
      }
    }

    docReports.push(report);
  }

  const failOn = options.failOn ?? "gating";
  const exitCode = computeExitCode(
    { gating: sawGating, warn: sawWarn, tamper: sawTamper },
    failOn,
  );

  const summary: CheckSummary = {
    total: verdicts.length,
    gating: gatingCount,
    warning: warningCount,
    clean: verdicts.length - gatingCount - warningCount,
    doc: docHist,
    code: codeHist,
    behavior: behaviorHist,
    expired: expiredCount,
  };

  return {
    ref,
    verdicts,
    documents: docReports,
    summary,
    exitCode,
  };
}

/**
 * Exit-code contract (§9): 0 clean · 2 gating · 3 warn-only · 1 op error.
 *
 * `failOn` selects the threshold before the flags are consulted: `never` always
 * passes; otherwise a `gating` verdict always fails (exit 2); a warn-only result
 * fails (exit 2) when `failOn==="warn"` and is otherwise a soft exit 3; a tamper
 * fails (exit 2) only under `failOn==="tamper"`. Operational errors are exit 1,
 * raised by the CLI, not here.
 */
export function computeExitCode(
  flags: { gating: boolean; warn: boolean; tamper: boolean },
  failOn: FailOn,
): number {
  if (failOn === "never") return 0;
  if (flags.gating) return 2;
  if (flags.warn) return failOn === "warn" ? 2 : 3;
  if (flags.tamper && failOn === "tamper") return 2;
  return 0;
}
