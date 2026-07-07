/**
 * `ignore` (§17.6, D14) — acknowledge a behavioral `at-risk` you re-verified by
 * hand. Writes `Assertion.suppressed`: the acknowledged `{path → hash}` map of
 * the currently-changed evidence, plus the required reason.
 *
 * A change-set can touch several evidence paths, so a single `--until <hash>`
 * would be ambiguous; the suppression covers exactly the acknowledged hashes and
 * lapses automatically when any acknowledged path's current hash differs (or a
 * new evidence path appears — checked in `resolve.ts`). While active, the at-risk
 * contributes nothing to exit codes and is surfaced as `suppressed: true`.
 *
 * The shell computes the acknowledged `{path → hash}` map (it owns FS + analyzer)
 * and hands it here; this stays a thin store write, mirroring `retire.ts`.
 */

import type { Assertion } from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";

export interface IgnoreResult {
  assertion: Assertion;
  /** The acknowledged `{path → hash}` map recorded on the claim. */
  paths: Record<string, string>;
  reason: string;
}

export async function ignoreClaim(
  store: ClaimStore,
  claimId: string,
  reason: string,
  acknowledged: Record<string, string>,
  opts: { dryRun?: boolean } = {},
): Promise<IgnoreResult> {
  if (!reason || reason.trim().length === 0) {
    throw new Error(
      "hibi ignore requires --reason <text> — an unexplained suppression tells the next reader nothing.",
    );
  }
  const assertion = await store.getAssertion(claimId);
  if (!assertion) throw new Error(`No claim ${claimId} in the store.`);

  const next: Assertion = {
    ...assertion,
    suppressed: { paths: acknowledged, reason },
  };
  if (!opts.dryRun) await store.putAssertion(next);
  return { assertion: next, paths: acknowledged, reason };
}
