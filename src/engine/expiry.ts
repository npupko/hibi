/**
 * Expiry (SPEC 5). A claim with a `ttl` expires `ttl` after its newest
 * counting attestation. Only two bases count: `evidence` (the judge returned
 * `supports` for submitted evidence), and `vouch` by a `human:` actor. An
 * `agent:` or `process:` vouch is recorded and does not count; an `ack` never
 * counts. A claim with a `ttl` and no counting attestation is expired.
 */

import {
  type Attestation,
  type Claim,
  DURATION_PATTERN,
} from "../core/model.ts";

/** True when an attestation resets the ttl clock (SPEC 5, counting rule). */
export function counts(a: Attestation): boolean {
  if (a.basis === "evidence") return true;
  return a.basis === "vouch" && a.by.startsWith("human:");
}

/**
 * The newest counting attestation, or undefined. Newest is the latest `at`;
 * on equal times the later entry in the append-only list wins.
 */
export function countingAttestation(
  claim: Pick<Claim, "attestations">,
): Attestation | undefined {
  let best: Attestation | undefined;
  for (const a of claim.attestations) {
    if (!counts(a)) continue;
    if (best === undefined || Date.parse(a.at) >= Date.parse(best.at)) best = a;
  }
  return best;
}

/**
 * Add an ISO 8601 duration (`PnYnMnWnDTnHnMnS`, integer amounts) to an
 * instant. Years and months add calendar units in UTC; when the target month
 * is shorter, the day is clamped to its last day (Jan 31 + P1M is Feb 28 or
 * 29). Weeks, days, hours, minutes, and seconds add a fixed number of seconds
 * (a day is 86400). Throws on a string that is not such a duration.
 */
export function addDuration(start: Date, duration: string): Date {
  const m = DURATION_PATTERN.exec(duration);
  if (m === null) {
    throw new Error(
      `not an ISO 8601 duration: ${JSON.stringify(duration)} (expected e.g. P90D or PT12H)`,
    );
  }
  const n = (g: string | undefined): number =>
    g === undefined ? 0 : Number.parseInt(g, 10);
  const [, y, mo, w, d, , h, mi, s] = m;
  const months = n(y) * 12 + n(mo);
  const seconds =
    n(w) * 7 * 86_400 + n(d) * 86_400 + n(h) * 3_600 + n(mi) * 60 + n(s);

  const out = new Date(start.getTime());
  if (months > 0) {
    const day = out.getUTCDate();
    out.setUTCDate(1);
    out.setUTCMonth(out.getUTCMonth() + months);
    const lastDay = new Date(
      Date.UTC(out.getUTCFullYear(), out.getUTCMonth() + 1, 0),
    ).getUTCDate();
    out.setUTCDate(Math.min(day, lastDay));
  }
  return new Date(out.getTime() + seconds * 1000);
}

/** When the claim expires, or undefined when it has no `ttl` or no counting attestation. */
export function expiresAt(
  claim: Pick<Claim, "ttl" | "attestations">,
): Date | undefined {
  if (claim.ttl === undefined) return undefined;
  const a = countingAttestation(claim);
  if (a === undefined) return undefined;
  return addDuration(new Date(a.at), claim.ttl);
}

/** True when `ttl` is set and `now >= at(counting) + ttl`, or when there is no counting attestation. */
export function isExpired(
  claim: Pick<Claim, "ttl" | "attestations">,
  now: Date,
): boolean {
  if (claim.ttl === undefined) return false;
  const until = expiresAt(claim);
  return until === undefined || now.getTime() >= until.getTime();
}

/** The reason label for an expired claim; undefined when it is not expired. */
export function expiryLabel(
  claim: Pick<Claim, "ttl" | "attestations">,
  now: Date,
): string | undefined {
  if (!isExpired(claim, now)) return undefined;
  const a = countingAttestation(claim);
  if (a === undefined) {
    return `expired: ttl ${claim.ttl} is set and no attestation counts (only evidence or a human vouch resets ttl)`;
  }
  const until = expiresAt(claim)?.toISOString();
  return `expired: ttl ${claim.ttl} ran out at ${until}, counted from the ${a.basis} by ${a.by} at ${a.at}`;
}
