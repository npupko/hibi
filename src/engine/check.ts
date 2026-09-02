/**
 * The check engine: walk the store's claims, resolve each anchor against the
 * current working tree, merge with document lifecycle, optionally stamp
 * banners, and emit a report with an exit code.
 *
 * Verdicts are recomputed live and never persisted. The report leads with the
 * per-claim decision (`doc`/`code` AnchorState, `behavior`, `expired`/`gates`)
 * and a side-tagged status vocabulary (`code:changed`, `doc:orphaned`,
 * `behavior:refuted`, ...).
 */

import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
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
  Region,
  Verdict,
} from "../core/model.ts";
import { exists } from "../fs.ts";
import type { ResolverRegistry } from "../resolver/registry.ts";
import type { ClaimStore } from "../store/store.ts";

/** Side-tagged status precedence, most severe first. */
const STATUS_PRECEDENCE = [
  "code:orphaned",
  "doc:orphaned",
  "code:ambiguous",
  "doc:ambiguous",
  "code:changed",
  "doc:changed",
  "behavior:refuted",
  "expired",
  "code:moved",
  "doc:moved",
  "superseded",
  "archived",
];

const REPORTABLE_ANCHOR: ReadonlySet<AnchorState> = new Set<AnchorState>([
  "orphaned",
  "ambiguous",
  "changed",
  "moved",
]);

export const FAIL_ON = ["gating", "warn", "never"] as const;
export type FailOn = (typeof FAIL_ON)[number];

export interface CheckOptions {
  ast?: AstAnalyzer;
  /** Resolve through the registry (built-in + external + verifiers) instead of inline. */
  registry?: ResolverRegistry;
  /** Apply banner stamps to documents. Default: read-only. */
  write?: boolean;
  /** Restrict the check to claims touching a file in this set (doc or code side). */
  onlyFiles?: Set<string>;
  /** Restrict the check to claims on this document id. */
  onlyDocument?: string;
  failOn?: FailOn;
  now?: number;
  ref?: string;
}

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
}

export interface CheckSummary {
  total: number;
  gating: number;
  warning: number;
  clean: number;
  /** Retired claims are listed but never clean, gating, or warning. */
  retired: number;
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
  return { supported: 0, refuted: 0 };
}

function liveDocText(
  docContent: string | null,
  region: Region | undefined,
): string | null {
  if (docContent === null || region === undefined) return null;
  return docContent.slice(region.start, region.end);
}

/** The side-tagged status strings a suspect verdict contributes. */
export function verdictStatuses(v: Verdict): string[] {
  const out: string[] = [];
  if (REPORTABLE_ANCHOR.has(v.code)) out.push(`code:${v.code}`);
  if (REPORTABLE_ANCHOR.has(v.doc)) out.push(`doc:${v.doc}`);
  if (v.behavior === "refuted") out.push("behavior:refuted");
  if (v.expired) out.push("expired");
  return out;
}

/** The single most severe status for a verdict, folding in lifecycle tags. */
export function worstStatus(v: Verdict, lcTags: string[] = []): string {
  const statuses = [...verdictStatuses(v), ...lcTags];
  for (const s of STATUS_PRECEDENCE) if (statuses.includes(s)) return s;
  return statuses[0] ?? "unchanged";
}

function lifecycleTags(doc: Document): string[] {
  return doc.lifecycle === "active" ? [] : [doc.lifecycle];
}

function lifecycleEntries(doc: Document): BannerEntry[] {
  if (doc.lifecycle === "superseded") {
    return [
      {
        status: "superseded",
        id: doc.id,
        text: "This document has been superseded.",
      },
    ];
  }
  if (doc.lifecycle === "archived") {
    return [
      {
        status: "archived",
        id: doc.id,
        text: "This document has been archived.",
      },
    ];
  }
  return [];
}

/** Strip hibi's own banner and the legacy `hibi-status:` frontmatter line. */
export function stripEngineOwned(
  raw: string,
  path: string,
  nonce: string,
): string {
  return setFrontmatterStatus(removeBanner(raw, path, nonce).content, null);
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
  const enforcementById = new Map<string, Enforcement>(
    assertions.map((a) => [a.id, a.enforcement]),
  );
  const cfg = await store.config();
  const nonce = cfg.nonce;

  const fileCache = new Map<string, string | null>();
  const readFileText = async (rel: string): Promise<string | null> => {
    if (fileCache.has(rel)) return fileCache.get(rel) ?? null;
    const abs = join(root, rel);
    const text = (await exists(abs)) ? await readFile(abs, "utf8") : null;
    fileCache.set(rel, text);
    return text;
  };

  const verdicts: Verdict[] = [];
  const docContentById = new Map<string, string | null>();
  const evaluatedDocs = new Set<string>();

  for (const a of assertions) {
    if (options.onlyDocument && a.documentId !== options.onlyDocument) continue;
    const codeFiles = a.anchor.code.map((b) => b.file);
    if (options.onlyFiles) {
      const touches =
        options.onlyFiles.has(a.anchor.doc.file) ||
        codeFiles.some((f) => options.onlyFiles?.has(f));
      if (!touches) continue;
    }
    evaluatedDocs.add(a.documentId);

    // The engine-owned banner restates the suspect sentence verbatim; strip it
    // before resolving so the doc-side quote never latches onto the copy.
    const rawDoc = await readFileText(a.anchor.doc.file);
    const docContent =
      rawDoc === null
        ? null
        : stripEngineOwned(rawDoc, a.anchor.doc.file, nonce);
    docContentById.set(a.documentId, docContent);
    const code = new Map<string, string | null>();
    for (const f of codeFiles) code.set(f, await readFileText(f));
    const files: ResolveFiles = { doc: docContent, code };

    const verdict = options.registry
      ? await options.registry.resolve(a, files, propsById.get(a.propositionId))
      : resolveAssertion(a, files, { ast: options.ast, now: options.now });
    verdicts.push(verdict);
  }

  // Histograms and rollups.
  const docHist = emptyAnchorHistogram();
  const codeHist = emptyAnchorHistogram();
  const behaviorHist = emptyBehaviorHistogram();
  let expiredCount = 0;
  let gatingCount = 0;
  let warningCount = 0;
  let retiredCount = 0;

  for (const v of verdicts) {
    docHist[v.doc] += 1;
    codeHist[v.code] += 1;
    if (v.behavior !== undefined) behaviorHist[v.behavior] += 1;
    if (v.expired) expiredCount += 1;
    const enforcement = enforcementById.get(v.assertionId) ?? "suggested";
    if (enforcement === "retired") retiredCount += 1;
    else if (v.gates) gatingCount += 1;
    else if (isWarnVerdict(v, enforcement)) warningCount += 1;
  }

  const verdictsByDoc = new Map<string, Verdict[]>();
  for (const v of verdicts) {
    const list = verdictsByDoc.get(v.documentId) ?? [];
    list.push(v);
    verdictsByDoc.set(v.documentId, list);
  }

  const docReports: DocumentReport[] = [];
  for (const doc of documents) {
    if (options.onlyDocument && doc.id !== options.onlyDocument) continue;
    // A scoped check only evaluated the documents touching a changed file.
    // Leave every other document, and its banner, untouched.
    if (options.onlyFiles && !evaluatedDocs.has(doc.id)) continue;

    const dv = verdictsByDoc.get(doc.id) ?? [];
    const lcTags = lifecycleTags(doc);
    const docContent = docContentById.get(doc.id) ?? null;
    const docHasLifecycle = lcTags.length > 0;
    const suspectVerdicts = dv.filter((v) => {
      const enforcement = enforcementById.get(v.assertionId) ?? "suggested";
      if (enforcement === "retired") return false;
      return v.gates || isWarnVerdict(v, enforcement) || docHasLifecycle;
    });

    const suspectEntries: BannerEntry[] = suspectVerdicts.map((v) => {
      const text =
        liveDocText(docContent, v.evidence.docRegion) ??
        propsById.get(v.propositionId)?.textCache ??
        "(unknown proposition)";
      return { status: worstStatus(v, lcTags), id: v.propositionId, text };
    });
    const lcEntries = lifecycleEntries(doc);
    const allEntries = [...suspectEntries, ...lcEntries];

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

    const pristine = (cfg.pristine ?? []).some((g) =>
      new Bun.Glob(g).match(doc.path),
    );

    if (options.write && !pristine) {
      const abs = join(root, doc.path);
      if (await exists(abs)) {
        const original = await readFile(abs, "utf8");
        const headline =
          suspectEntries.length === 0 && lcEntries.length > 0
            ? `DOCUMENT STATUS — ${lcEntries.length} notice(s) — re-verify before trusting.`
            : DEFAULT_HEADLINE(allEntries.length);
        const compact = isInstructionFile(
          doc.path,
          cfg.instructionFiles ?? [...DEFAULT_INSTRUCTION_FILES],
        );

        let content = original;
        if (allEntries.length === 0) {
          const res = removeBanner(content, doc.path, nonce);
          content = res.content;
          report.bannerAction = res.action;
        } else {
          const res = stampBanner(
            content,
            doc.path,
            { headline, entries: allEntries },
            nonce,
            compact
              ? { compact: { count: allEntries.length, docPath: doc.path } }
              : {},
          );
          report.bannerAction = res.action;
          content = res.content;
        }
        // Clear a legacy `hibi-status:` frontmatter line written by older versions.
        content = setFrontmatterStatus(content, null);
        if (content !== original) await writeFile(abs, content);
      }
    }

    docReports.push(report);
  }

  const failOn = options.failOn ?? "gating";
  const exitCode = computeExitCode(
    { gating: gatingCount > 0, warn: warningCount > 0 },
    failOn,
  );

  const summary: CheckSummary = {
    total: verdicts.length,
    gating: gatingCount,
    warning: warningCount,
    clean: verdicts.length - gatingCount - warningCount - retiredCount,
    retired: retiredCount,
    doc: docHist,
    code: codeHist,
    behavior: behaviorHist,
    expired: expiredCount,
  };

  return { ref, verdicts, documents: docReports, summary, exitCode };
}

/**
 * Exit-code contract: 0 clean, 2 gating, 1 operational error (raised by the
 * CLI, not here). `--fail-on never` always passes; a gating verdict is exit 2;
 * a warning (`moved`) is exit 2 only under `--fail-on warn`.
 */
export function computeExitCode(
  flags: { gating: boolean; warn: boolean },
  failOn: FailOn,
): number {
  if (failOn === "never") return 0;
  if (flags.gating) return 2;
  if (flags.warn && failOn === "warn") return 2;
  return 0;
}
