/**
 * `list`: one lean row per claim with the decision fields and the handles the
 * next command needs. Built by projecting a live `CheckReport`, so it shares
 * the verdict/gating semantics `check` uses. `--path` restricts the rows to
 * claims anchored to or covering a file on either side.
 */

import { isWarnVerdict } from "../core/gating.ts";
import type {
  Assertion,
  Proposition,
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
  "duplicate",
] as const;
export type ListState = (typeof LIST_STATES)[number];

export interface ListRow {
  claimId: string;
  propositionId: string;
  /** The documented sentence (non-authoritative cache). */
  text: string;
  documentPath: string | null;
  codePath: string | null;
  /** Worst side-tagged status, `retired`, or `unchanged` when clean. */
  status: string;
  severity: ListSeverity;
  gates: boolean;
  enforcement: Assertion["enforcement"];
  recommended: string | null;
  /** Under `--path`: which side matched. */
  side?: "doc" | "code";
}

export interface ListResult {
  state: ListState;
  path?: string;
  count: number;
  claims: ListRow[];
}

/**
 * How many live (non-retired) claims assert each proposition fingerprint. The
 * single source for both the `duplicate` list state and the overview footer.
 */
export function liveFingerprintCounts(
  assertions: Assertion[],
  propositions: Proposition[],
): Map<string, number> {
  const fpByProp = new Map(propositions.map((p) => [p.id, p.fingerprint]));
  const counts = new Map<string, number>();
  for (const a of assertions) {
    if (a.enforcement === "retired") continue;
    const fp = fpByProp.get(a.propositionId);
    if (!fp) continue;
    counts.set(fp, (counts.get(fp) ?? 0) + 1);
  }
  return counts;
}

function severityOf(
  v: Verdict,
  enforcement: Assertion["enforcement"],
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

/** Which side of a claim's anchor targets or covers `path`, if any. */
export function matchedSide(
  a: Assertion,
  path: string,
): "doc" | "code" | undefined {
  if (a.anchor.doc.file === path) return "doc";
  if (a.anchor.code.some((b) => bundleMatches(b, path))) return "code";
  return undefined;
}

function relevantCodePath(
  v: Verdict,
  assertion: Assertion | undefined,
): string | null {
  const codeFiles = assertion?.anchor.code.map((b) => b.file) ?? [];
  const changed = v.evidence.changedEvidence.find((c) =>
    codeFiles.includes(c.path),
  );
  return changed?.path ?? codeFiles[0] ?? null;
}

export interface ToListRowsOptions {
  state?: ListState;
  path?: string;
  hints?: boolean;
}

export function toListRows(
  report: CheckReport,
  assertions: Assertion[],
  propositions: Proposition[],
  documents: DocumentReport[],
  opts: ToListRowsOptions = {},
): ListResult {
  const state = opts.state ?? "all";
  const hints = opts.hints ?? true;
  const assertById = new Map(assertions.map((a) => [a.id, a]));
  const propById = new Map(propositions.map((p) => [p.id, p]));
  const docById = new Map(documents.map((d) => [d.id, d]));

  const liveByFingerprint = liveFingerprintCounts(assertions, propositions);

  const rows: ListRow[] = [];
  for (const v of report.verdicts) {
    const assertion = assertById.get(v.assertionId);
    if (!assertion) continue;
    const enforcement = assertion.enforcement;
    const doc = docById.get(v.documentId);
    const retired = enforcement === "retired";
    const severity = retired ? "clean" : severityOf(v, enforcement);
    const lifecycle = doc?.lifecycle ?? "active";

    let side: "doc" | "code" | undefined;
    if (opts.path !== undefined) {
      side = matchedSide(assertion, opts.path);
      if (!side) continue;
    }

    const fp = propById.get(assertion.propositionId)?.fingerprint;
    const matches = (() => {
      switch (state) {
        case "all":
          return true;
        case "orphaned":
          return !retired && (v.doc === "orphaned" || v.code === "orphaned");
        case "suggested":
          return enforcement === "suggested";
        case "stranded":
          return !retired && lifecycle !== "active";
        case "duplicate":
          return (
            !retired && fp !== undefined && (liveByFingerprint.get(fp) ?? 0) > 1
          );
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
      claimId: v.assertionId,
      propositionId: v.propositionId,
      text: propById.get(v.propositionId)?.textCache ?? "",
      documentPath: doc?.path ?? null,
      codePath: relevantCodePath(v, assertion),
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
