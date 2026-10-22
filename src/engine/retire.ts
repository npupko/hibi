/**
 * `retire` (§9) — withdraw a single claim so it no longer gates or warns.
 *
 * Flips the claim's `enforcement` to `retired` (the model already carries
 * that value) and persists it, rather than deleting the record: the audit trail
 * is kept and the action is reversible. A retired assertion is excluded from
 * gating/warning by the gating policy (`computeGates`/`isWarnVerdict`), so a
 * retired claim is simply ignored by `check`. Idempotent — retiring an
 * already-retired claim is a no-op success (`alreadyRetired: true`).
 */

import type { Claim } from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";

export interface RetireResult {
  claim: Claim;
  /** True when the claim was already `retired` (this call changed nothing). */
  alreadyRetired: boolean;
}

export async function retire(
  store: ClaimStore,
  claimId: string,
  opts: { dryRun?: boolean } = {},
): Promise<RetireResult> {
  const claim = await store.getClaim(claimId);
  if (!claim) throw new Error(`No claim ${claimId} in the store.`);
  if (claim.enforcement === "retired") {
    return { claim, alreadyRetired: true };
  }
  const next: Claim = { ...claim, enforcement: "retired" };
  // --dry-run: report the would-retire result without persisting it.
  if (!opts.dryRun) await store.putClaim(next);
  return { claim: next, alreadyRetired: false };
}
