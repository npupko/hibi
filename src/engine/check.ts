/**
 * The check engine (SPEC 7): walk the store's claims, resolve each one
 * against the current working tree, consult the judge cache, apply the gating
 * table, and emit a report with an exit code. Verdicts are recomputed live and
 * never persisted; only judge verdicts are cached.
 *
 * JSON shape of `hibi check` (the CLI wraps it in `{ ok, action: "check",
 * schemaVersion, ref, [since, changedFiles], [doc], exitCode, summary,
 * verdicts, worklist, documents }`; see `src/cli/render/json.ts`):
 *
 *   summary: {
 *     total, gating, warning, clean, retired,        // claim counts; retired is none of the others
 *     doc: { unchanged, moved, changed, ambiguous, orphaned },
 *     evidence: { …the five, unavailable },          // claims with no evidence side are not counted
 *     judge: { supported, refuted, review, pending },// claims with a judge state
 *     expired,                                       // claims with expired: true
 *     conflicts,                                     // claims with a non-empty conflicts[]
 *     worklist,                                      // worklist.length
 *     conflictPass: { pairs, requests, pending,      // pairs judged, judge calls answered,
 *                     scope: "store" | "doc" },      // pairs pending; doc: only pairs with a
 *                                                    // claim in the --doc document
 *   }
 *   verdicts[]: {
 *     claimId, documentId, kind: "code" | "reference" | "attested",
 *     doc: AnchorState,
 *     evidence?: AnchorState | "unavailable",      // absent when the claim has no evidence side
 *     judge?: "supported" | "refuted" | "review" | "pending", // absent when no judgment applies
 *     expired: boolean,
 *     conflicts: [{ claimId, probability, reason }],
 *     gates: boolean, warns: boolean,              // never both true
 *     changed?: "<path> <text|ast|value>: <detail>", // the first span change, when any
 *     remediation?: { recommended: string | null,  // dropped under --no-hints
 *                     actions: [{ id, title, rationale, command? }] } | null,
 *     reasons: string[],                           // every reason label
 *     detail?: { docRegion?, evidenceRegions[], similarity?, changes[] }, // --explain only
 *   }
 *   worklist[]: {
 *     claimId, sentence, kind, document,           // document: the claim's doc path
 *     procedure: { instructions, steps? } | null,
 *     sources: [{ id?, resource, title? }],
 *     owner: string | null,
 *     lastEvidence?: { summary, source, at, by },  // from the newest supports cache entry with evidence
 *     reason: "expired" | "never-checked" | "review" | "pending",
 *   }
 *   documents[]: { id, path, lifecycle, suspect: [{ claimId, status }] }
 *     // status: the most severe warning, gating, or lifecycle tag, e.g.
 *     // `doc:changed`, `conflict`, `expired`, `judge:review`,
 *     // `conflict:review`, `conflict:pending`, `superseded`; never `unchanged`
 *
 * The contradiction pass (SPEC 9) runs before this module is called
 * (`conflictPass`): over every pair for a whole-store check, and over the
 * pairs with a claim in the document for `--doc`; its findings may name
 * claims outside the scope. On each claim: a gating finding is a
 * `conflicts[]` entry; a `review` finding warns with its label; a pair the
 * judge could not answer warns with `conflict check pending with <id>: judge
 * unavailable` (or `: judge error` when its answer could not be used).
 *
 * Judge outcome labels carry the prefix `judge: ` (see `reference.ts` for the
 * labels themselves).
 *
 * Worklist membership (SPEC 7.5), over claims that are not retired and whose
 * document is active: any claim whose ttl ran out (`expired`); a `reference`
 * or `attested` claim whose judge state is `review` (not acknowledged) or
 * `pending`; and an `attested` claim with a `procedure` or at least one
 * `sources` entry that has no `supports` cache entry (`never-checked`). A
 * pure vouch claim (attested, no procedure, no sources) is listed only when
 * expired. A claim is listed once, with the first matching reason in this
 * order: expired, review, pending, never-checked. The verdict's `reasons[]`
 * carries a `worklist: …` label.
 */

import { readFile } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import {
  type AstAnalyzer,
  assembleVerdict,
  type ClaimFindings,
  type ClaimVerdict,
  resolveClaimSides,
} from "../algo/resolve.ts";
import type { GateMode } from "../core/gating.ts";
import type {
  AnchorState,
  Claim,
  ClaimKind,
  Config,
  Document,
  DocumentLifecycle,
  EvidenceState,
  JudgeCacheEntry,
  JudgeState,
  Procedure,
  Source,
} from "../core/model.ts";
import { exists } from "../fs.ts";
import type { Judge } from "../judge/types.ts";
import type { ClaimStore } from "../store/store.ts";
import {
  type ConflictPassResult,
  conflictReviewsFor,
  conflictsFor,
} from "./conflict.ts";
import {
  cachedSupportsOutcome,
  type JudgeOutcome,
  judgeReference,
  newestSupports,
  referenceTexts,
} from "./reference.ts";
import { resolveFile } from "./repos.ts";

/** Side-tagged status precedence, most severe first. */
const STATUS_PRECEDENCE = [
  "evidence:orphaned",
  "doc:orphaned",
  "evidence:ambiguous",
  "doc:ambiguous",
  "evidence:changed",
  "doc:changed",
  "judge:refuted",
  "conflict",
  "expired",
  "evidence:moved",
  "doc:moved",
  "judge:review",
  "judge:pending",
  "conflict:review",
  "conflict:pending",
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

const REPORTABLE_JUDGE: ReadonlySet<JudgeState> = new Set<JudgeState>([
  "refuted",
  "review",
  "pending",
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
  /** `doc` (`check --doc`) makes `expired` gate. Default: `doc` when `onlyDocument` is set, else `store`. */
  mode?: GateMode;
  /** Clock for expiry and new cache entries. Default: now. */
  now?: Date;
  /** Answers `supports` for `reference` claims on a cache miss. Absent: a miss is `pending`. */
  judge?: Judge;
  /** The contradiction pass over the whole store. Default: no pass ran. */
  conflictPass?: ConflictPassResult;
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
  /** Claims with no judge state are not counted here. */
  judge: Record<JudgeState, number>;
  expired: number;
  /** Claims with at least one conflict finding. */
  conflicts: number;
  worklist: number;
  /**
   * The contradiction pass: pairs judged, judge calls answered, pairs pending,
   * and whether it judged every pair (`store`) or only pairs with a claim in
   * the scoped document (`doc`).
   */
  conflictPass: {
    pairs: number;
    requests: number;
    pending: number;
    scope: "store" | "doc";
  };
}

export const WORKLIST_REASONS = [
  "expired",
  "review",
  "pending",
  "never-checked",
] as const;
export type WorklistReason = (typeof WORKLIST_REASONS)[number];

export interface WorklistItem {
  claimId: string;
  sentence: string;
  kind: ClaimKind;
  /** The claim's document path. */
  document: string;
  procedure: Procedure | null;
  sources: Source[];
  owner: string | null;
  lastEvidence?: { summary: string; source: string; at: string; by: string };
  reason: WorklistReason;
}

/** A `check` verdict. */
export type CheckVerdict = ClaimVerdict;

export interface CheckReport {
  ref: string;
  verdicts: CheckVerdict[];
  worklist: WorklistItem[];
  documents: DocumentReport[];
  summary: CheckSummary;
  exitCode: number;
}

/**
 * One label per worklist reason. `hibi evidence` takes `attested` claims only,
 * so an expired `code` or `reference` claim gets `EXPIRED_NO_EVIDENCE_LABEL`.
 */
const WORKLIST_LABEL: Record<WorklistReason, string> = {
  expired:
    "worklist: expired; re-check the claim and submit evidence, or have a human attest",
  review: "worklist: the judge verdict is review",
  pending: "worklist: the judge verdict is pending",
  "never-checked":
    "worklist: never checked; no evidence has been judged for this claim",
};

const EXPIRED_NO_EVIDENCE_LABEL =
  "worklist: expired; have a human attest, or re-record after re-verifying";

function worklistLabel(reason: WorklistReason, kind: ClaimKind): string {
  return reason === "expired" && kind !== "attested"
    ? EXPIRED_NO_EVIDENCE_LABEL
    : WORKLIST_LABEL[reason];
}

function emptyAnchorHistogram(): Record<AnchorState, number> {
  return { unchanged: 0, moved: 0, changed: 0, ambiguous: 0, orphaned: 0 };
}

function emptyEvidenceHistogram(): Record<EvidenceState, number> {
  return { ...emptyAnchorHistogram(), unavailable: 0 };
}

function emptyJudgeHistogram(): Record<JudgeState, number> {
  return { supported: 0, refuted: 0, review: 0, pending: 0 };
}

/** The side-tagged status strings a suspect verdict contributes. */
export function verdictStatuses(
  v: Pick<ClaimVerdict, "doc" | "evidence" | "judge" | "expired"> & {
    conflicts?: readonly unknown[];
  },
): string[] {
  const out: string[] = [];
  if (v.evidence !== undefined && REPORTABLE_STATE.has(v.evidence))
    out.push(`evidence:${v.evidence}`);
  if (REPORTABLE_STATE.has(v.doc)) out.push(`doc:${v.doc}`);
  if (v.judge !== undefined && REPORTABLE_JUDGE.has(v.judge))
    out.push(`judge:${v.judge}`);
  if ((v.conflicts?.length ?? 0) > 0) out.push("conflict");
  if (v.expired) out.push("expired");
  return out;
}

/** Statuses ordered most severe first; unknown tags keep their order at the end. */
export function orderStatuses(statuses: string[]): string[] {
  const rank = (s: string): number => {
    const i = STATUS_PRECEDENCE.indexOf(s);
    return i < 0 ? STATUS_PRECEDENCE.length : i;
  };
  return statuses
    .map((s, i) => ({ s, i }))
    .sort((a, b) => rank(a.s) - rank(b.s) || a.i - b.i)
    .map((x) => x.s);
}

/**
 * The most severe of `statuses` (warning, gating, and lifecycle tags). A
 * suspect always has at least one; `warning` is a fallback that names no
 * rule and should not occur.
 */
export function firstStatus(statuses: string[]): string {
  for (const s of STATUS_PRECEDENCE) if (statuses.includes(s)) return s;
  return statuses[0] ?? "warning";
}

/** The single most severe status for a verdict, folding in lifecycle tags. */
export function worstStatus(
  v: Parameters<typeof verdictStatuses>[0],
  lcTags: string[] = [],
): string {
  return firstStatus([...verdictStatuses(v), ...lcTags]);
}

function lifecycleTags(doc: Document): string[] {
  return doc.lifecycle === "active" ? [] : [doc.lifecycle];
}

/** Newest-last `supports` cache entries per claim id. */
async function supportsEntriesByClaim(
  store: ClaimStore,
): Promise<Map<string, JudgeCacheEntry[]>> {
  const out = new Map<string, JudgeCacheEntry[]>();
  for (const e of await store.judgeCache().list()) {
    const id = e.subject.claimId;
    if (e.judgment !== "supports" || id === undefined) continue;
    const list = out.get(id) ?? [];
    list.push(e);
    out.set(id, list);
  }
  return out;
}

/** The `ack` attestation that names `key`, if any. */
function ackFor(claim: Claim, key: string | undefined) {
  if (key === undefined) return undefined;
  return claim.attestations.findLast(
    (a) =>
      a.basis === "ack" &&
      a.ack !== undefined &&
      "judgeKey" in a.ack &&
      a.ack.judgeKey === key,
  );
}

function worklistReason(
  claim: Claim,
  v: ClaimVerdict,
  acknowledged: boolean,
  hasSupports: boolean,
): WorklistReason | undefined {
  if (claim.enforcement === "retired") return undefined;
  if (v.expired) return "expired";
  if (claim.kind === "code") return undefined;
  if (v.judge === "review" && !acknowledged) return "review";
  if (v.judge === "pending") return "pending";
  const checkable = claim.procedure !== null || claim.sources.length > 0;
  if (claim.kind === "attested" && checkable && !hasSupports)
    return "never-checked";
  return undefined;
}

export async function runCheck(
  store: ClaimStore,
  options: CheckOptions = {},
): Promise<CheckReport> {
  const root = store.anchorRoot;
  const ref = options.ref ?? "WORKTREE";
  const now = options.now ?? new Date();
  const mode: GateMode =
    options.mode ?? (options.onlyDocument !== undefined ? "doc" : "store");
  const config: Config = await store.config();
  const threshold = config.judge.threshold;
  // Cache keys use the judge's model; the configured model only when no judge is given.
  const model = options.judge?.model ?? config.judge.model;
  const cache = store.judgeCache();
  const pass = options.conflictPass;
  const pendingWith = new Map<string, { other: string; why: string }[]>();
  for (const pair of pass?.pairs ?? []) {
    if (pair.outcome !== "pending") continue;
    const [a, b] = pair.pairId.split(":") as [string, string];
    const why = pair.reason.startsWith("judge error")
      ? "judge error"
      : "judge unavailable";
    pendingWith.set(a, [...(pendingWith.get(a) ?? []), { other: b, why }]);
    pendingWith.set(b, [...(pendingWith.get(b) ?? []), { other: a, why }]);
  }
  const documents = await store.allDocuments();
  const claims = await store.allClaims();
  const lifecycleById = new Map(documents.map((d) => [d.id, d.lifecycle]));
  const supports = await supportsEntriesByClaim(store);
  const enforcementById = new Map(claims.map((c) => [c.id, c.enforcement]));

  const fileCache = new Map<string, string | null>();
  const readFileText = async (rel: string): Promise<string | null> => {
    if (fileCache.has(rel)) return fileCache.get(rel) ?? null;
    // An aliased evidence file may resolve to an absolute path (SPEC 4.1).
    const abs = isAbsolute(rel) ? rel : join(root, rel);
    const text = (await exists(abs)) ? await readFile(abs, "utf8") : null;
    fileCache.set(rel, text);
    return text;
  };

  const verdicts: ClaimVerdict[] = [];
  const statusTags = new Map<string, string[]>();
  const worklist: WorklistItem[] = [];
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
    const unavailable = new Map<string, string>();
    for (const f of evidenceFiles) {
      const resolved = resolveFile(config, f, root);
      if ("unavailable" in resolved) unavailable.set(f, resolved.reason);
      else evidence.set(f, await readFileText(resolved.path));
    }
    const docText = await readFileText(c.doc.file);
    const files = { doc: docText, evidence, unavailable };
    const sides = resolveClaimSides(c, files, { ast: options.ast });

    const entries = supports.get(c.id) ?? [];
    let outcome: JudgeOutcome | undefined;
    const findingReasons: string[] = [];
    if (c.enforcement !== "retired") {
      if (c.kind === "reference") {
        const judgeable = referenceTexts(c, sides, docText);
        if ("reason" in judgeable) {
          findingReasons.push(`judge: ${judgeable.reason}`);
        } else {
          outcome = await judgeReference({
            claim: c,
            ...judgeable.texts,
            judge: options.judge,
            cache,
            model,
            threshold,
            now,
          });
        }
      } else if (c.kind === "attested") {
        const newest = newestSupports(entries);
        if (newest !== undefined) {
          outcome = cachedSupportsOutcome(newest, threshold, c);
        }
      }
    }

    const ack =
      outcome?.state === "review" ? ackFor(c, outcome.key) : undefined;
    if (ack !== undefined) {
      findingReasons.push(
        `review acknowledged by ${ack.by} at ${ack.at}${ack.note ? `: ${ack.note}` : ""}; it no longer warns`,
      );
    }
    const live = c.enforcement !== "retired";
    const reviews =
      live && pass !== undefined ? conflictReviewsFor(pass, c.id) : [];
    const pendingPairs = live ? (pendingWith.get(c.id) ?? []) : [];
    const warnings = [
      ...reviews.map((f) => `possible conflict with ${f.claimId}: ${f.reason}`),
      ...pendingPairs.map(
        (p) => `conflict check pending with ${p.other}: ${p.why}`,
      ),
    ];
    const findings: ClaimFindings = {
      judge: outcome?.state,
      conflicts: live && pass !== undefined ? conflictsFor(pass, c.id) : [],
      warnings,
      reasons: [
        ...(outcome?.reasons ?? []).map((r) => `judge: ${r}`),
        ...findingReasons,
      ],
      reviewAcknowledged: ack !== undefined,
    };
    const v = assembleVerdict(c, sides, {
      ...findings,
      now,
      mode,
      threshold,
    });
    const tags = verdictStatuses({
      ...v,
      judge: ack !== undefined ? undefined : v.judge,
    });
    if (reviews.length > 0) tags.push("conflict:review");
    if (pendingPairs.length > 0) tags.push("conflict:pending");
    statusTags.set(c.id, tags);

    const active = (lifecycleById.get(c.documentId) ?? "active") === "active";
    const reason = active
      ? worklistReason(c, v, ack !== undefined, entries.length > 0)
      : undefined;
    if (reason !== undefined) {
      v.reasons.push(worklistLabel(reason, c.kind));
      const withEvidence = entries.filter((e) => e.evidence !== undefined);
      const last = withEvidence.at(-1)?.evidence;
      worklist.push({
        claimId: c.id,
        sentence: c.sentence,
        kind: c.kind,
        document: c.doc.file,
        procedure: c.procedure,
        sources: c.sources,
        owner: c.owner ?? null,
        ...(last !== undefined
          ? {
              lastEvidence: {
                summary: last.summary,
                source: last.source,
                at: last.at,
                by: last.by,
              },
            }
          : {}),
        reason,
      });
    }
    verdicts.push(v);
  }

  // Histograms and rollups.
  const docHist = emptyAnchorHistogram();
  const evidenceHist = emptyEvidenceHistogram();
  const judgeHist = emptyJudgeHistogram();
  let expiredCount = 0;
  let conflictCount = 0;
  let gatingCount = 0;
  let warningCount = 0;
  let retiredCount = 0;

  for (const v of verdicts) {
    docHist[v.doc] += 1;
    if (v.evidence !== undefined) evidenceHist[v.evidence] += 1;
    if (v.judge !== undefined) judgeHist[v.judge] += 1;
    if (v.expired) expiredCount += 1;
    if (v.conflicts.length > 0) conflictCount += 1;
    const enforcement = enforcementById.get(v.claimId) ?? "suggested";
    if (enforcement === "retired") retiredCount += 1;
    else if (v.gates) gatingCount += 1;
    else if (v.warns) warningCount += 1;
  }

  const verdictsByDoc = new Map<string, ClaimVerdict[]>();
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
        return v.gates || v.warns || docHasLifecycle;
      })
      .map((v) => ({
        claimId: v.claimId,
        status: firstStatus([...(statusTags.get(v.claimId) ?? []), ...lcTags]),
      }))
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
    judge: judgeHist,
    expired: expiredCount,
    conflicts: conflictCount,
    worklist: worklist.length,
    conflictPass: {
      pairs: pass?.pairs.length ?? 0,
      requests: pass?.requests ?? 0,
      pending: pass?.pending.length ?? 0,
      scope: pass?.scope ?? "store",
    },
  };

  return {
    ref,
    verdicts,
    worklist,
    documents: docReports,
    summary,
    exitCode,
  };
}

/**
 * Exit-code contract: 0 clean or warnings, 2 gating, 1 operational error
 * (raised by the CLI, not here). `--fail-on never` always passes; a gating
 * verdict is exit 2; a warning is exit 2 only under `--fail-on warn`.
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
