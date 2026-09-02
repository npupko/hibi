/**
 * The machine-output projection: shapes the serialized JSON by verbosity
 * without mutating the engine's in-memory `Verdict`.
 *
 *   - concise (default): ids, the two anchor states, the behavioral state,
 *     `expired`/`gates`, a one-line `changed` summary, the `remediation` menu,
 *     and `notes`.
 *   - `--explain`: adds `evidence`, `advisories`, and the proposition `fingerprint`.
 *
 * `--no-hints` (`HIBI_ADVICE=0`) drops the `remediation` block.
 */

import { MODEL_VERSION, type Verdict } from "../../core/model.ts";
import type { CheckReport } from "../../engine/check.ts";

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
  v: Verdict,
  opts: ProjectionOptions,
  fingerprints?: ReadonlyMap<string, string>,
): Record<string, unknown> {
  const out: Record<string, unknown> = {
    assertionId: v.assertionId,
    propositionId: v.propositionId,
    documentId: v.documentId,
    doc: v.doc,
    code: v.code,
  };
  if (v.behavior !== undefined) out.behavior = v.behavior;
  out.expired = v.expired;
  out.gates = v.gates;

  const c = v.evidence.changedEvidence[0];
  if (c) out.changed = `${c.path} ${c.kind}${c.detail ? `: ${c.detail}` : ""}`;

  if (opts.hints) out.remediation = v.remediation;
  out.notes = v.notes;

  if (opts.explain) {
    out.evidence = v.evidence;
    out.advisories = v.advisories;
    const fp = fingerprints?.get(v.propositionId);
    if (fp) out.fingerprint = fp;
  }
  return out;
}

/** Project a `check` report: `ok`, `action`, `schemaVersion`, context, `exitCode`, `summary`, `verdicts`, `documents`. */
export function projectCheckReport(
  report: CheckReport,
  opts: ProjectionOptions,
  extra?: Record<string, unknown>,
  fingerprints?: ReadonlyMap<string, string>,
): Record<string, unknown> {
  return envelope("check", {
    ref: report.ref,
    ...extra,
    exitCode: report.exitCode,
    summary: report.summary,
    verdicts: report.verdicts.map((v) => projectVerdict(v, opts, fingerprints)),
    documents: report.documents,
  });
}
