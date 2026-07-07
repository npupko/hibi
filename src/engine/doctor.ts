/**
 * `doctor` (§9, Tier-1 silent-orphan hardening) — the store-health projection.
 *
 * `check` answers "does the build gate?"; it is silent about non-gating rot — an
 * orphaned `suggested` claim, a claim stranded on a superseded document, two
 * propositions that drifted into duplicate fingerprints. `doctor` surfaces
 * exactly that dead state as a categorized, purely informational report (it never
 * gates; the CLI always exits 0). It is a *pure projection* over a live
 * `CheckReport` joined to the store's authored entities — no I/O, no second
 * verdict computation.
 */

import type { Assertion, Document, Proposition } from "../core/model.ts";
import type { CheckReport } from "./check.ts";
import { isCoarseBundle } from "./query.ts";

/** A side of a claim's anchor that resolved to `orphaned` (deleted/unresolvable). */
export interface OrphanedAnchorRow {
  claimId: string;
  side: "doc" | "code";
  path: string;
}

/** A `suggested` claim with no precise code anchor — it can never be promoted to gating as-is. */
export interface SuggestedNoCodeRow {
  claimId: string;
  docPath: string | null;
}

/** A live claim stranded on a document that has left the read path. */
export interface StaleDocClaimRow {
  claimId: string;
  docPath: string | null;
  lifecycle: Document["lifecycle"];
}

/** Propositions that collapsed onto the same content fingerprint (the dedup unit, §5). */
export interface DuplicatePropositionRow {
  fingerprint: string;
  propositionIds: string[];
  claimIds: string[];
}

/**
 * Rate metrics from the read-only resolution pass (§17.6/D14/D19). Observability,
 * not gates: the behavioral flag-rate carries the research's >30% tighten-the-gate
 * trigger; the doc-side rates carry D19's orphan-rate kill-switch (>30% → require
 * inline IDs). All are shares in [0,1]; 0 when there are no claims.
 */
export interface DoctorRates {
  /** Share of behavioral claims currently `at-risk` or `refuted` (>30% → tighten the gate). */
  behavioralFlagRate: number;
  /** Share of all claims whose doc side is `orphaned` (>30% → require inline IDs). */
  docOrphanedRate: number;
  /** Share of all claims whose doc side is `moved`. */
  docMovedRate: number;
  /** Share of all claims whose doc side is `changed`. */
  docChangedRate: number;
}

export interface DoctorReport {
  orphanedAnchors: OrphanedAnchorRow[];
  suggestedNoCode: SuggestedNoCodeRow[];
  staleDocClaims: StaleDocClaimRow[];
  duplicatePropositions: DuplicatePropositionRow[];
  counts: {
    orphanedAnchors: number;
    suggestedNoCode: number;
    staleDocClaims: number;
    duplicatePropositions: number;
  };
  /** Observability rates (never gate) — the tighten-the-gate / kill-switch triggers. */
  rates: DoctorRates;
  /** True iff every category is empty — the store has no hidden rot. */
  healthy: boolean;
}

/** Compute the observability rates over a live check's verdicts (§17.6/D14/D19). */
function computeRates(report: CheckReport): DoctorRates {
  const verdicts = report.verdicts;
  const total = verdicts.length;
  const share = (n: number, d: number) => (d === 0 ? 0 : n / d);

  const behavioral = verdicts.filter((v) => v.behavior !== undefined);
  const flagged = behavioral.filter(
    (v) => v.behavior === "at-risk" || v.behavior === "refuted",
  );

  return {
    behavioralFlagRate: share(flagged.length, behavioral.length),
    docOrphanedRate: share(
      verdicts.filter((v) => v.doc === "orphaned").length,
      total,
    ),
    docMovedRate: share(
      verdicts.filter((v) => v.doc === "moved").length,
      total,
    ),
    docChangedRate: share(
      verdicts.filter((v) => v.doc === "changed").length,
      total,
    ),
  };
}

/** True when no bundle on the code side carries a precise (gradeable) selector. */
function lacksPreciseCode(assertion: Assertion): boolean {
  // Reuse query's coarse-bundle test so "precise vs navigational" is defined once.
  return (
    assertion.anchor.code.length === 0 ||
    assertion.anchor.code.every(isCoarseBundle)
  );
}

/**
 * The code-side files that actually went orphaned for a verdict, derived from the
 * change evidence (a missing file / a deleted span), so a multi-bundle claim
 * points at the bundle that disappeared — never blindly at `code[0]`. Falls back
 * to the first code file only when the evidence names none.
 */
function orphanedCodeFiles(
  v: CheckReport["verdicts"][number],
  a?: Assertion,
): string[] {
  const codeFiles = new Set(a?.anchor.code.map((b) => b.file) ?? []);
  const hit = [
    ...new Set(
      v.evidence.changedEvidence
        .filter(
          (c) =>
            codeFiles.has(c.path) &&
            (c.detail === "file missing" ||
              (c.detail?.includes("orphaned") ?? false)),
        )
        .map((c) => c.path),
    ),
  ];
  return hit.length > 0 ? hit : [a?.anchor.code[0]?.file ?? "?"];
}

/**
 * Build the store-health report. Pure: the caller supplies a live `CheckReport`
 * (for the per-side anchor states) plus the authored entities the report does not
 * carry verbatim (assertions for the anchor paths, documents for lifecycle/path,
 * propositions for the fingerprint grouping).
 */
export function buildDoctorReport(
  report: CheckReport,
  assertions: Assertion[],
  documents: Document[],
  propositions: Proposition[],
): DoctorReport {
  const assertById = new Map(assertions.map((a) => [a.id, a]));
  const docById = new Map(documents.map((d) => [d.id, d]));

  // ── orphanedAnchors: a side whose span is deleted/unresolvable (from the live check). ──
  // A retired claim is withdrawn — its orphaned anchor is inert and must NOT be
  // counted, or retiring an orphan (the recommended remedy) would never let the
  // store report healthy and the `--state orphaned` cleanup loop would never drain.
  const orphanedAnchors: OrphanedAnchorRow[] = [];
  for (const v of report.verdicts) {
    const assertion = assertById.get(v.assertionId);
    if (assertion?.enforcement === "retired") continue;
    if (v.doc === "orphaned") {
      orphanedAnchors.push({
        claimId: v.assertionId,
        side: "doc",
        path: assertion?.anchor.doc.file ?? "?",
      });
    }
    if (v.code === "orphaned") {
      for (const path of orphanedCodeFiles(v, assertion)) {
        orphanedAnchors.push({ claimId: v.assertionId, side: "code", path });
      }
    }
  }

  // ── suggestedNoCode: a `suggested` claim with no precise code anchor to promote. ──
  const suggestedNoCode: SuggestedNoCodeRow[] = [];
  // ── staleDocClaims: a live claim whose document has left the read path. ──
  const staleDocClaims: StaleDocClaimRow[] = [];
  for (const a of assertions) {
    if (a.enforcement === "retired") continue;
    const doc = docById.get(a.documentId);
    if (a.enforcement === "suggested" && lacksPreciseCode(a)) {
      suggestedNoCode.push({ claimId: a.id, docPath: doc?.path ?? null });
    }
    // Any non-`active` lifecycle means the document has left the read path — use
    // the codebase's `!== "active"` convention so a future lifecycle value is
    // caught automatically (the exact silent-orphan rot doctor exists to surface).
    if (doc && doc.lifecycle !== "active") {
      staleDocClaims.push({
        claimId: a.id,
        docPath: doc.path,
        lifecycle: doc.lifecycle,
      });
    }
  }

  // ── duplicatePropositions: distinct propositions sharing one content fingerprint. ──
  // Count only LIVE (non-retired) claims, and only propositions that still have
  // one — so retiring the redundant claim drops the group below 2 and the finding
  // clears. (The advertised remedy must actually make `doctor` report healthy; a
  // proposition whose every claim is retired is inert and never a live duplicate.)
  const liveClaimsByProp = new Map<string, string[]>();
  for (const a of assertions) {
    if (a.enforcement === "retired") continue;
    const list = liveClaimsByProp.get(a.propositionId) ?? [];
    list.push(a.id);
    liveClaimsByProp.set(a.propositionId, list);
  }
  const livePropsByFingerprint = new Map<string, Proposition[]>();
  for (const p of propositions) {
    if (!liveClaimsByProp.has(p.id)) continue;
    const list = livePropsByFingerprint.get(p.fingerprint) ?? [];
    list.push(p);
    livePropsByFingerprint.set(p.fingerprint, list);
  }
  const duplicatePropositions: DuplicatePropositionRow[] = [];
  for (const [fingerprint, group] of livePropsByFingerprint) {
    if (group.length < 2) continue;
    duplicatePropositions.push({
      fingerprint,
      propositionIds: group.map((p) => p.id),
      claimIds: group.flatMap((p) => liveClaimsByProp.get(p.id) ?? []),
    });
  }

  const counts = {
    orphanedAnchors: orphanedAnchors.length,
    suggestedNoCode: suggestedNoCode.length,
    staleDocClaims: staleDocClaims.length,
    duplicatePropositions: duplicatePropositions.length,
  };
  const healthy = Object.values(counts).every((n) => n === 0);

  return {
    orphanedAnchors,
    suggestedNoCode,
    staleDocClaims,
    duplicatePropositions,
    counts,
    rates: computeRates(report),
    healthy,
  };
}
