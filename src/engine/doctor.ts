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

import {
  type Assertion,
  COARSE_SELECTOR_KINDS,
  type Document,
  type Proposition,
} from "../core/model.ts";
import type { CheckReport } from "./check.ts";

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
  /** True iff every category is empty — the store has no hidden rot. */
  healthy: boolean;
}

/** True when no bundle on the code side carries a precise (gradeable) selector. */
function lacksPreciseCode(assertion: Assertion): boolean {
  if (assertion.anchor.code.length === 0) return true;
  return assertion.anchor.code.every((bundle) =>
    bundle.selectors.every((s) =>
      (COARSE_SELECTOR_KINDS as readonly string[]).includes(s.kind),
    ),
  );
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
  const orphanedAnchors: OrphanedAnchorRow[] = [];
  for (const v of report.verdicts) {
    const assertion = assertById.get(v.assertionId);
    if (v.doc === "orphaned") {
      orphanedAnchors.push({
        claimId: v.assertionId,
        side: "doc",
        path: assertion?.anchor.doc.file ?? "?",
      });
    }
    if (v.code === "orphaned") {
      orphanedAnchors.push({
        claimId: v.assertionId,
        side: "code",
        path: assertion?.anchor.code[0]?.file ?? "?",
      });
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
  const claimsByProp = new Map<string, string[]>();
  for (const a of assertions) {
    const list = claimsByProp.get(a.propositionId) ?? [];
    list.push(a.id);
    claimsByProp.set(a.propositionId, list);
  }
  const propsByFingerprint = new Map<string, Proposition[]>();
  for (const p of propositions) {
    const list = propsByFingerprint.get(p.fingerprint) ?? [];
    list.push(p);
    propsByFingerprint.set(p.fingerprint, list);
  }
  const duplicatePropositions: DuplicatePropositionRow[] = [];
  for (const [fingerprint, group] of propsByFingerprint) {
    if (group.length < 2) continue;
    duplicatePropositions.push({
      fingerprint,
      propositionIds: group.map((p) => p.id),
      claimIds: group.flatMap((p) => claimsByProp.get(p.id) ?? []),
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
    healthy,
  };
}
