/**
 * The machine-output projection: shapes the serialized JSON by verbosity
 * without mutating the engine's in-memory `Verdict`.
 *
 *   - concise (default): ids, kind, the doc and evidence states, the judge
 *     state, `expired`/`conflicts`/`gates`/`warns`, a one-line `changed`
 *     summary, the `remediation` menu, and `reasons`.
 *   - `--explain`: adds `detail` (regions, changes, similarity).
 *
 * `--no-hints` (`HIBI_ADVICE=0`) drops the `remediation` block.
 */

import { MODEL_VERSION } from "../../core/model.ts";
import type { CheckReport, CheckVerdict } from "../../engine/check.ts";

export interface ProjectionOptions {
  explain: boolean;
  hints: boolean;
}

/** The schema version stamped into every payload. */
export const SCHEMA_VERSION = MODEL_VERSION;

/** The `{ok, action, schemaVersion, ...payload, next?}` envelope every command emits. */
export function envelope(
  action: string,
  payload: Record<string, unknown>,
  next?: string,
): Record<string, unknown> {
  return {
    ok: true,
    action,
    schemaVersion: SCHEMA_VERSION,
    ...payload,
    ...(next !== undefined ? { next } : {}),
  };
}

export function projectVerdict(
  v: CheckVerdict,
  opts: ProjectionOptions,
): Record<string, unknown> {
  const out: Record<string, unknown> = {
    claimId: v.claimId,
    documentId: v.documentId,
    kind: v.kind,
    doc: v.doc,
  };
  if (v.evidence !== undefined) out.evidence = v.evidence;
  if (v.judge !== undefined) out.judge = v.judge;
  out.expired = v.expired;
  out.conflicts = v.conflicts;
  out.gates = v.gates;
  out.warns = v.warns;

  const c = v.detail.changes[0];
  if (c) out.changed = `${c.path} ${c.kind}${c.detail ? `: ${c.detail}` : ""}`;

  if (opts.hints) out.remediation = v.remediation;
  out.reasons = v.reasons;

  if (opts.explain) out.detail = v.detail;
  return out;
}

/** Project a `check` report: `ok`, `action`, `schemaVersion`, context, `exitCode`, `summary`, `verdicts`, `worklist`, `documents`. */
export function projectCheckReport(
  report: CheckReport,
  opts: ProjectionOptions,
  extra?: Record<string, unknown>,
): Record<string, unknown> {
  return envelope("check", {
    ref: report.ref,
    ...extra,
    exitCode: report.exitCode,
    summary: report.summary,
    verdicts: report.verdicts.map((v) => projectVerdict(v, opts)),
    worklist: report.worklist,
    documents: report.documents,
  });
}
