/**
 * The machine-output projection (§9) — shapes the *serialized* JSON by verbosity
 * without ever mutating the engine's in-memory `Verdict` (the human renderer
 * still needs its full `evidence`). Two explicit tiers:
 *
 *   - **concise (default):** decision-first and lean — ids, the two anchor
 *     states, the behavioral state, `expired`/`gates`, the `remediation` menu,
 *     and short `notes`. The bulky located evidence is dropped from the hot path.
 *   - **`--explain`:** adds `evidence{…}` (selectorScores, codeRegions,
 *     changedEvidence, confidence), `advisories`, and the proposition
 *     `fingerprint`.
 *
 * A behavioral carve-out keeps a 1-line `changed` summary (path + kind) on the
 * concise path for `at-risk`/`refuted` verdicts, so an agent learns *what*
 * changed in one round-trip rather than re-querying with `--explain`.
 *
 * `--no-hints` (`HIBI_ADVICE=0`) drops the whole `remediation` block — the
 * documented escape hatch for noise-sensitive harnesses (git `advice.*` precedent).
 */

import { MODEL_VERSION, type Verdict } from "../../core/model.ts";
import type { CheckReport } from "../../index.ts";

/** The output-shaping axes a projection reads (resolved from flags + env). */
export interface ProjectionOptions {
  explain: boolean;
  hints: boolean;
}

/** The schema version stamped into every payload (not only the schema filename). */
export const SCHEMA_VERSION = MODEL_VERSION;

/**
 * Project one verdict. Key order is decision-first: the handles, the two anchor
 * states, the behavioral state, `expired`/`gates`, the `changed` carve-out, the
 * `remediation` menu, then `notes` — and only under `--explain` the bulky tail.
 */
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

  // Behavioral carve-out: a 1-line "what changed" summary survives concise output.
  if (v.behavior === "at-risk" || v.behavior === "refuted") {
    const c = v.evidence.changedEvidence[0];
    if (c) out.changed = `${c.path} ${c.kind}`;
  }

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

/**
 * Project a full `check`/`diff` report into the decision-first envelope: `ok`,
 * `action`, `schemaVersion` lead, then the context (`ref`, plus any `extra` like
 * `diff`'s `since`/`changedFiles`), then `exitCode`, `summary`, `verdicts`, and
 * `documents`. `documents` carry no bulky evidence, so they pass through as-is.
 */
export function projectCheckReport(
  action: string,
  report: CheckReport,
  opts: ProjectionOptions,
  extra?: Record<string, unknown>,
  fingerprints?: ReadonlyMap<string, string>,
): Record<string, unknown> {
  return {
    ok: true,
    action,
    schemaVersion: SCHEMA_VERSION,
    ref: report.ref,
    ...extra,
    exitCode: report.exitCode,
    summary: report.summary,
    verdicts: report.verdicts.map((v) => projectVerdict(v, opts, fingerprints)),
    documents: report.documents,
  };
}
