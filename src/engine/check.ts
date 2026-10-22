/**
 * The check engine: walk the store's claims, resolve each one against the
 * current working tree, merge with document lifecycle, and emit a report with
 * an exit code.
 *
 * Verdicts are recomputed live and never persisted. The report leads with the
 * per-claim decision (`doc` / `evidence` state, `expired` / `gates`) and a
 * side-tagged status vocabulary (`evidence:changed`, `doc:orphaned`, ...).
 */

import { readFile } from "node:fs/promises";
import { join } from "node:path";
import {
  type AstAnalyzer,
  type ResolveFiles,
  resolveClaim,
} from "../algo/resolve.ts";
import { isWarnVerdict } from "../core/gating.ts";
import type {
  AnchorState,
  Document,
  DocumentLifecycle,
  Enforcement,
  EvidenceState,
  Verdict,
} from "../core/model.ts";
import { exists } from "../fs.ts";
import type { ClaimStore } from "../store/store.ts";

/** Side-tagged status precedence, most severe first. */
const STATUS_PRECEDENCE = [
  "evidence:orphaned",
  "doc:orphaned",
  "evidence:ambiguous",
  "doc:ambiguous",
  "evidence:changed",
  "doc:changed",
  "judge:refuted",
  "expired",
  "evidence:moved",
  "doc:moved",
  "evidence:unavailable",
  "superseded",
  "archived",
];

const REPORTABLE_STATE: ReadonlySet<EvidenceState> = new Set<EvidenceState>([
  "orphaned",
  "ambiguous",
  "changed",
  "moved",
  "unavailable",
]);

export const FAIL_ON = ["gating", "warn", "never"] as const;
export type FailOn = (typeof FAIL_ON)[number];

export interface CheckOptions {
  ast?: AstAnalyzer;
  /** Restrict the check to claims touching a file in this set (doc or evidence side). */
  onlyFiles?: Set<string>;
  /** Restrict the check to claims on this document id. */
  onlyDocument?: string;
  failOn?: FailOn;
  ref?: string;
}

export interface SuspectEntry {
  claimId: string;
  status: string;
}

export interface DocumentReport {
  id: string;
  path: string;
  lifecycle: DocumentLifecycle;
  suspect: SuspectEntry[];
}

export interface CheckSummary {
  total: number;
  gating: number;
  warning: number;
  clean: number;
  /** Retired claims are listed but never clean, gating, or warning. */
  retired: number;
  doc: Record<AnchorState, number>;
  /** Claims with no evidence span are not counted here. */
  evidence: Record<EvidenceState, number>;
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

function emptyEvidenceHistogram(): Record<EvidenceState, number> {
  return { ...emptyAnchorHistogram(), unavailable: 0 };
}

/** The side-tagged status strings a suspect verdict contributes. */
export function verdictStatuses(v: Verdict): string[] {
  const out: string[] = [];
  if (v.evidence !== undefined && REPORTABLE_STATE.has(v.evidence))
    out.push(`evidence:${v.evidence}`);
  if (REPORTABLE_STATE.has(v.doc)) out.push(`doc:${v.doc}`);
  if (v.judge === "refuted") out.push("judge:refuted");
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

export async function runCheck(
  store: ClaimStore,
  options: CheckOptions = {},
): Promise<CheckReport> {
  const root = store.anchorRoot;
  const ref = options.ref ?? "WORKTREE";
  const documents = await store.allDocuments();
  const claims = await store.allClaims();
  const enforcementById = new Map<string, Enforcement>(
    claims.map((c) => [c.id, c.enforcement]),
  );

  const fileCache = new Map<string, string | null>();
  const readFileText = async (rel: string): Promise<string | null> => {
    if (fileCache.has(rel)) return fileCache.get(rel) ?? null;
    const abs = join(root, rel);
    const text = (await exists(abs)) ? await readFile(abs, "utf8") : null;
    fileCache.set(rel, text);
    return text;
  };

  const verdicts: Verdict[] = [];
  const evaluatedDocs = new Set<string>();

  for (const c of claims) {
    if (options.onlyDocument && c.documentId !== options.onlyDocument) continue;
    const evidenceFiles = c.evidence.map((b) => b.file);
    if (options.onlyFiles) {
      const touches =
        options.onlyFiles.has(c.doc.file) ||
        evidenceFiles.some((f) => options.onlyFiles?.has(f));
      if (!touches) continue;
    }
    evaluatedDocs.add(c.documentId);

    const evidence = new Map<string, string | null>();
    for (const f of evidenceFiles) evidence.set(f, await readFileText(f));
    const files: ResolveFiles = {
      doc: await readFileText(c.doc.file),
      evidence,
    };
    verdicts.push(resolveClaim(c, files, { ast: options.ast }));
  }

  // Histograms and rollups.
  const docHist = emptyAnchorHistogram();
  const evidenceHist = emptyEvidenceHistogram();
  let expiredCount = 0;
  let gatingCount = 0;
  let warningCount = 0;
  let retiredCount = 0;

  for (const v of verdicts) {
    docHist[v.doc] += 1;
    if (v.evidence !== undefined) evidenceHist[v.evidence] += 1;
    if (v.expired) expiredCount += 1;
    const enforcement = enforcementById.get(v.claimId) ?? "suggested";
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
    if (options.onlyFiles && !evaluatedDocs.has(doc.id)) continue;

    const dv = verdictsByDoc.get(doc.id) ?? [];
    const lcTags = lifecycleTags(doc);
    const docHasLifecycle = lcTags.length > 0;
    const suspect: SuspectEntry[] = dv
      .filter((v) => {
        const enforcement = enforcementById.get(v.claimId) ?? "suggested";
        if (enforcement === "retired") return false;
        return v.gates || isWarnVerdict(v, enforcement) || docHasLifecycle;
      })
      .map((v) => ({ claimId: v.claimId, status: worstStatus(v, lcTags) }))
      .sort((a, b) =>
        a.claimId < b.claimId ? -1 : a.claimId > b.claimId ? 1 : 0,
      );

    docReports.push({
      id: doc.id,
      path: doc.path,
      lifecycle: doc.lifecycle,
      suspect,
    });
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
    evidence: evidenceHist,
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
