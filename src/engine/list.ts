/**
 * `list`: one lean row per claim with the decision fields and the handles the
 * next command needs. Built by projecting a live `CheckReport`, so it shares
 * the verdict/gating semantics `check` uses. `--path` restricts the rows to
 * claims anchored to or covering a file on either side.
 */

import { isWarnVerdict } from "../core/gating.ts";
import type {
  Claim,
  ClaimKind,
  SelectorBundle,
  Verdict,
} from "../core/model.ts";
import { coarseCovers } from "./anchor.ts";
import type { CheckReport, DocumentReport } from "./check.ts";
import { worstStatus } from "./check.ts";

export type ListSeverity = "gating" | "warning" | "clean";

export const LIST_STATES = [
  "all",
  "gating",
  "warning",
  "clean",
  "orphaned",
  "suggested",
  "stranded",
] as const;
export type ListState = (typeof LIST_STATES)[number];

export interface ListRow {
  claimId: string;
  kind: ClaimKind;
  sentence: string;
  documentPath: string | null;
  evidencePath: string | null;
  /** Worst side-tagged status, `retired`, or `unchanged` when clean. */
  status: string;
  severity: ListSeverity;
  gates: boolean;
  enforcement: Claim["enforcement"];
  recommended: string | null;
  /** Under `--path`: which side matched. */
  side?: "doc" | "evidence";
}

export interface ListResult {
  state: ListState;
  path?: string;
  count: number;
  claims: ListRow[];
}

function severityOf(
  v: Verdict,
  enforcement: Claim["enforcement"],
): ListSeverity {
  if (v.gates) return "gating";
  if (isWarnVerdict(v, enforcement)) return "warning";
  return "clean";
}

function bundleMatches(bundle: SelectorBundle, path: string): boolean {
  if (bundle.file === path) return true;
  for (const s of bundle.selectors) {
    if (s.kind === "coarse" && coarseCovers(s.pattern, path)) return true;
  }
  return false;
}

/** Which side of a claim targets or covers `path`, if any. */
export function matchedSide(
  c: Claim,
  path: string,
): "doc" | "evidence" | undefined {
  if (c.doc.file === path) return "doc";
  if (c.evidence.some((b) => bundleMatches(b, path))) return "evidence";
  return undefined;
}

function relevantEvidencePath(v: Verdict, claim: Claim): string | null {
  const files = claim.evidence.map((b) => b.file);
  const changed = v.detail.changes.find((c) => files.includes(c.path));
  return changed?.path ?? files[0] ?? null;
}

export interface ToListRowsOptions {
  state?: ListState;
  path?: string;
  hints?: boolean;
}

export function toListRows(
  report: CheckReport,
  claims: Claim[],
  documents: DocumentReport[],
  opts: ToListRowsOptions = {},
): ListResult {
  const state = opts.state ?? "all";
  const hints = opts.hints ?? true;
  const claimById = new Map(claims.map((c) => [c.id, c]));
  const docById = new Map(documents.map((d) => [d.id, d]));

  const rows: ListRow[] = [];
  for (const v of report.verdicts) {
    const claim = claimById.get(v.claimId);
    if (!claim) continue;
    const enforcement = claim.enforcement;
    const doc = docById.get(v.documentId);
    const retired = enforcement === "retired";
    const severity = retired ? "clean" : severityOf(v, enforcement);
    const lifecycle = doc?.lifecycle ?? "active";

    let side: "doc" | "evidence" | undefined;
    if (opts.path !== undefined) {
      side = matchedSide(claim, opts.path);
      if (!side) continue;
    }

    const matches = (() => {
      switch (state) {
        case "all":
          return true;
        case "orphaned":
          return (
            !retired && (v.doc === "orphaned" || v.evidence === "orphaned")
          );
        case "suggested":
          return enforcement === "suggested";
        case "stranded":
          return !retired && lifecycle !== "active";
        default:
          return !retired && severity === state;
      }
    })();
    if (!matches) continue;

    const status = retired
      ? "retired"
      : worstStatus(v, lifecycle === "active" ? [] : [lifecycle]);
    const recommended =
      hints && !retired ? (v.remediation?.recommended ?? null) : null;

    rows.push({
      claimId: v.claimId,
      kind: claim.kind,
      sentence: claim.sentence,
      documentPath: doc?.path ?? null,
      evidencePath: relevantEvidencePath(v, claim),
      status,
      severity,
      gates: v.gates,
      enforcement,
      recommended,
      ...(side ? { side } : {}),
    });
  }

  const rank = (s: ListSeverity) =>
    s === "gating" ? 0 : s === "warning" ? 1 : 2;
  rows.sort((a, b) => rank(a.severity) - rank(b.severity));

  return {
    state,
    ...(opts.path !== undefined ? { path: opts.path } : {}),
    count: rows.length,
    claims: rows,
  };
}
