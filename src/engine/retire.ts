/**
 * `retire` (§9) — withdraw a single claim so it no longer gates or warns.
 *
 * Flips the Assertion's `enforcement` to `retired` (the model already carries
 * that value) and persists it, rather than deleting the record: the audit trail
 * is kept and the action is reversible. A retired assertion is excluded from
 * gating/warning by the gating policy (`computeGates`/`isWarnVerdict`), so a
 * retired claim is simply ignored by `check`. Idempotent — retiring an
 * already-retired claim is a no-op success (`alreadyRetired: true`).
 */

import type { Assertion } from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";

export interface RetireResult {
  assertion: Assertion;
  /** True when the claim was already `retired` (this call changed nothing). */
  alreadyRetired: boolean;
}

export async function retire(
  store: ClaimStore,
  claimId: string,
  opts: { dryRun?: boolean } = {},
): Promise<RetireResult> {
  const assertion = await store.getAssertion(claimId);
  if (!assertion) throw new Error(`No claim ${claimId} in the store.`);
  if (assertion.enforcement === "retired") {
    return { assertion, alreadyRetired: true };
  }
  const next: Assertion = { ...assertion, enforcement: "retired" };
  // --dry-run: report the would-retire result without persisting it (§9).
  if (!opts.dryRun) await store.putAssertion(next);
  return { assertion: next, alreadyRetired: false };
}
