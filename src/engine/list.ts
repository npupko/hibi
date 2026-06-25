/**
 * `list` (§9) — the triage command. A flat array of one row per claim with just
 * the decision fields and the handles the next step needs (the claim id, the
 * doc/code paths, the worst status, the severity, and the recommended action) —
 * so an agent can answer "what needs attention?" without parsing a full `check`
 * report. Built by projecting a live `CheckReport`, so it shares the exact
 * verdict/gating semantics `check` uses (never a second computation).
 */

import { isWarnVerdict } from "../core/gating.ts";
import type { Assertion, Verdict } from "../core/model.ts";
import type { CheckReport, DocumentReport } from "./check.ts";
import { worstStatus } from "./check.ts";

/** The triage severity buckets, matching the `check` summary vocabulary. */
export type ListSeverity = "gating" | "warning" | "clean";

/**
 * Which claims to include. `all` (default) lists every tracked claim; the
 * severity buckets (`gating`/`warning`/`clean`) filter by `check` severity; the
 * two health filters cut across severity — `orphaned` selects claims with an
 * un-relocatable side, `suggested` selects non-gating advisory claims.
 */
export type ListState =
  | "all"
  | "gating"
  | "warning"
  | "clean"
  | "orphaned"
  | "suggested";

/** One lean triage row — decision fields + the handles for the next command. */
export interface ListRow {
  claimId: string;
  propositionId: string;
  documentPath: string | null;
  codePath: string | null;
  /** Worst side-tagged status (`code:changed`, …), or `unchanged` when clean. */
  status: string;
  severity: ListSeverity;
  gates: boolean;
  /** The recommended remediation action id, or `null` when intent is ambiguous. */
  recommended: string | null;
}

export interface ListResult {
  state: ListState;
  count: number;
  claims: ListRow[];
}

function severityOf(
  v: Verdict,
  enforcement: Assertion["enforcement"],
): ListSeverity {
  if (v.gates) return "gating";
  if (isWarnVerdict(v, enforcement)) return "warning";
  return "clean";
}

/**
 * Does this row match the requested `state`? The severity buckets compare against
 * the computed severity; the two health filters cut across it — `orphaned` keys
 * off an un-relocatable side from the live verdict, `suggested` off the authored
 * enforcement (a `retired` claim carries enforcement `retired`, so it is
 * naturally excluded from `suggested`).
 */
function matchesState(
  state: ListState,
  v: Verdict,
  enforcement: Assertion["enforcement"],
  severity: ListSeverity,
): boolean {
  if (state === "all") return true;
  if (state === "orphaned")
    return v.doc === "orphaned" || v.code === "orphaned";
  if (state === "suggested") return enforcement === "suggested";
  return severity === state;
}

/** Lifecycle status tags a document carries (for the worst-status string). */
function lifecycleTagsOf(lifecycle: DocumentReport["lifecycle"]): string[] {
  return lifecycle === "active" ? [] : [lifecycle];
}

/**
 * The code file most relevant to a verdict's status: the changed-evidence path
 * that belongs to one of the claim's code bundles (so a multi-target claim points
 * at the bundle that actually drifted), else the first code bundle's file.
 */
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
  /** Emit the `recommended` action; off under `--no-hints` / `HIBI_ADVICE=0`. */
  hints?: boolean;
}

/**
 * Project a `CheckReport` (joined to its assertions and the report's own document
 * reports) into triage rows, filtered by `state`. Pure — the caller supplies the
 * live report. Document *lifecycle* is folded into the status (so a claim on a
 * superseded/retracted document reads `superseded`/`retracted`, not `unchanged`),
 * and a `retired` claim is reported as `retired` rather than as live drift.
 */
export function toListRows(
  report: CheckReport,
  assertions: Assertion[],
  documents: DocumentReport[],
  opts: ToListRowsOptions = {},
): ListResult {
  const state = opts.state ?? "all";
  const hints = opts.hints ?? true;
  const assertById = new Map(assertions.map((a) => [a.id, a]));
  const docById = new Map(documents.map((d) => [d.id, d]));

  const rows: ListRow[] = [];
  for (const v of report.verdicts) {
    const assertion = assertById.get(v.assertionId);
    const enforcement = assertion?.enforcement ?? "suggested";
    const doc = docById.get(v.documentId);
    const retired = enforcement === "retired";

    const severity = retired ? "clean" : severityOf(v, enforcement);
    if (!matchesState(state, v, enforcement, severity)) continue;

    // A retired claim is withdrawn — report it as such, never as live drift.
    const status = retired
      ? "retired"
      : worstStatus(v, lifecycleTagsOf(doc?.lifecycle ?? "active"));
    const recommended =
      hints && !retired ? (v.remediation?.recommended ?? null) : null;

    rows.push({
      claimId: v.assertionId,
      propositionId: v.propositionId,
      documentPath: doc?.path ?? null,
      codePath: relevantCodePath(v, assertion),
      status,
      severity,
      gates: v.gates,
      recommended,
    });
  }

  // Most-severe-first so the rows an agent must act on lead.
  const rank = (s: ListSeverity) =>
    s === "gating" ? 0 : s === "warning" ? 1 : 2;
  rows.sort((a, b) => rank(a.severity) - rank(b.severity));

  return { state, count: rows.length, claims: rows };
}
