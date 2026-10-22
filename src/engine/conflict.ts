/**
 * The contradiction and duplicate pass (SPEC 9.2 to 9.4, decisions per 8.4
 * and 8.5). Code pairs claims (`pairing.ts`), the judge answers each pair
 * once per model and pair texts (cached in the store), and code decides.
 *
 * Every pair ends with one outcome and one reason label:
 *
 * | outcome     | rule                                                                 | example label |
 * |-------------|----------------------------------------------------------------------|---------------|
 * | `unrelated` | the judge's `same_subject` floor forced `unrelated`                   | `not the same subject (same_subject=0.12)` |
 * | `conflict`  | `same_subject` >= threshold and the sentences' values differ          | `numbers differ: 29 vs 39` |
 * | `conflict`  | `contradictory`, confidence and `same_subject` >= threshold, `contradicts` >= 0.5 | `contradictory (p=0.93, same_subject=0.95)` |
 * | `review`    | `contradictory` with any of those below its bar                      | `review: contradictory, confidence=0.55 below threshold 0.80` |
 * | `duplicate` | `consistent` with `same_subject` >= threshold                         | `consistent and same subject (same_subject=0.95)` |
 * | `clean`     | anything else                                                        | `unrelated (confidence=0.80, same_subject=0.40)` |
 * | `acked`     | a `conflict` or `review` that an `ack` on either claim covers         | `acknowledged: numbers differ: 29 vs 39` |
 * | `pending`   | the judge was unavailable                                             | `judge unavailable: no API key` |
 *
 * `conflict` and `review` become findings on both claims; `duplicate` becomes
 * a duplicate candidate in both directions.
 */
import type { Claim, JudgeCacheEntry } from "../core/model.ts";
import type { JudgeCache } from "../judge/cache.ts";
import { cacheKey } from "../judge/cache.ts";
import type { Answer } from "../judge/http.ts";
import { buildConflictRequest, mapConflictAnswers } from "../judge/jev.ts";
import {
  type ConflictResult,
  type ConflictSide,
  type Judge,
  JudgeUnavailable,
} from "../judge/types.ts";
import { sentenceHash } from "./hash.ts";
import { type Pair, pairClaims } from "./pairing.ts";
import { compareValues, extractValues } from "./values.ts";

/** At most this many judge calls in flight during one pass. */
export const MAX_IN_FLIGHT = 8;

/** `contradicts` below this, with a `contradictory` relation, is a review case (SPEC 8.3). */
export const CONTRADICTS_FLOOR = 0.5;

/** One side of a contradiction: a finding on `claimId` naming `otherClaimId`. */
export interface ConflictFinding {
  claimId: string;
  otherClaimId: string;
  pairId: string;
  /** p(contradictory) from the judge, or 1 when the values differ. */
  probability: number;
  reason: string;
  /** Set on findings that warn instead of gate. */
  review?: true;
}

export interface DuplicateCandidate {
  claimId: string;
  duplicateOf: string;
  pairId: string;
  /** p(consistent) from the judge. */
  probability: number;
  reason: string;
}

export type PairOutcomeKind =
  | "conflict"
  | "review"
  | "duplicate"
  | "clean"
  | "unrelated"
  | "acked"
  | "pending";

/** What happened to one pair, with the label of the rule that decided it. */
export interface PairOutcome {
  pairId: string;
  /** Why the pair was formed (from pairing). */
  why: string;
  outcome: PairOutcomeKind;
  reason: string;
}

export interface ConflictPassResult {
  /** Both sides of every `conflict` and `review` pair, sorted by claim then other claim. */
  findings: ConflictFinding[];
  /** Both directions of every `duplicate` pair, sorted the same way. */
  duplicates: DuplicateCandidate[];
  /** Pair ids the judge could not answer, sorted. */
  pending: string[];
  /** Judge calls that returned an answer; cache hits cost none. */
  requests: number;
  /** Every pair formed, sorted by pair id. */
  pairs: PairOutcome[];
}

export interface ConflictPassOptions {
  store: { judgeCache(): JudgeCache };
  judge: Judge;
  claims: Claim[];
  /** A claim's context for the judge: heading path and neighbouring sentences. */
  context: (claim: Claim) => string;
  /** `judge.threshold` from config. */
  threshold: number;
  /** `judge.model` from config; part of every cache key. */
  model: string;
}

type Decision =
  | { outcome: "conflict" | "review"; probability: number; reason: string }
  | { outcome: "duplicate"; probability: number; reason: string }
  | { outcome: "clean" | "unrelated"; reason: string };

const f2 = (p: number) => p.toFixed(2);

/**
 * The per-pair decision, before acknowledgements. Pure. `a` and `b` are the
 * pair's sentences; the result is the judge's answer for them.
 */
export function decidePair(
  a: string,
  b: string,
  r: ConflictResult,
  threshold: number,
): Decision {
  if (r.forced === "same_subject") {
    return {
      outcome: "unrelated",
      reason: `not the same subject (same_subject=${f2(r.sameSubject)})`,
    };
  }
  if (r.sameSubject >= threshold) {
    // Numeric rule (SPEC 8.5): only after the judge says same subject.
    const values = compareValues(extractValues(a), extractValues(b));
    if (values.differs && values.label !== undefined) {
      return { outcome: "conflict", probability: 1, reason: values.label };
    }
  }
  const pContra = r.probabilities.contradictory;
  if (r.relation === "contradictory") {
    const below: string[] = [];
    if (r.contradicts < CONTRADICTS_FLOOR) {
      below.push(
        `contradicts=${f2(r.contradicts)} below ${f2(CONTRADICTS_FLOOR)}`,
      );
    }
    if (r.confidence < threshold) {
      below.push(
        `confidence=${f2(r.confidence)} below threshold ${f2(threshold)}`,
      );
    }
    if (r.sameSubject < threshold) {
      below.push(
        `same_subject=${f2(r.sameSubject)} below threshold ${f2(threshold)}`,
      );
    }
    if (below.length === 0) {
      return {
        outcome: "conflict",
        probability: pContra,
        reason: `contradictory (p=${f2(pContra)}, same_subject=${f2(r.sameSubject)})`,
      };
    }
    return {
      outcome: "review",
      probability: pContra,
      reason: `review: contradictory, ${below.join(", ")}`,
    };
  }
  if (r.relation === "consistent" && r.sameSubject >= threshold) {
    return {
      outcome: "duplicate",
      probability: r.probabilities.consistent,
      reason: `consistent and same subject (same_subject=${f2(r.sameSubject)})`,
    };
  }
  return {
    outcome: "clean",
    reason: `${r.relation} (confidence=${f2(r.confidence)}, same_subject=${f2(r.sameSubject)})`,
  };
}

interface PairAck {
  pairId: string;
  sentenceHashes: [string, string];
}

function isPairAck(x: unknown): x is PairAck {
  if (x === null || typeof x !== "object") return false;
  const o = x as Record<string, unknown>;
  return (
    typeof o.pairId === "string" &&
    Array.isArray(o.sentenceHashes) &&
    o.sentenceHashes.length === 2
  );
}

/**
 * Whether an attestation on either claim acknowledges this pair as the two
 * sentences read now (SPEC 9.4). Hashes are in pair id order; changing either
 * sentence ends the suppression.
 */
export function isAcked(pair: Pair): boolean {
  const hashes = [sentenceHash(pair.a.sentence), sentenceHash(pair.b.sentence)];
  for (const claim of [pair.a, pair.b]) {
    for (const att of claim.attestations) {
      const ack = (att as { ack?: unknown }).ack;
      if (
        isPairAck(ack) &&
        ack.pairId === pair.pairId &&
        ack.sentenceHashes[0] === hashes[0] &&
        ack.sentenceHashes[1] === hashes[1]
      ) {
        return true;
      }
    }
  }
  return false;
}

function side(claim: Claim, context: (claim: Claim) => string): ConflictSide {
  return { text: claim.sentence, doc: claim.doc.file, context: context(claim) };
}

/** The exact request and cache key for a pair. Sides are in pair id order. */
export function conflictRequestFor(
  model: string,
  pair: Pair,
  context: (claim: Claim) => string,
): { a: ConflictSide; b: ConflictSide; request: unknown; key: string } {
  const [first, second] =
    pair.a.id < pair.b.id ? [pair.a, pair.b] : [pair.b, pair.a];
  const a = side(first, context);
  const b = side(second, context);
  const request = buildConflictRequest(model, a, b);
  return { a, b, request, key: cacheKey(model, request) };
}

async function pool<T>(
  items: T[],
  limit: number,
  fn: (item: T, index: number) => Promise<void>,
): Promise<void> {
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const i = next++;
      await fn(items[i] as T, i);
    }
  };
  await Promise.all(
    Array.from({ length: Math.min(limit, items.length) }, worker),
  );
}

const byIds = <T extends { claimId: string }>(
  other: (t: T) => string,
): ((x: T, y: T) => number) => {
  return (x, y) => {
    const kx = `${x.claimId}:${other(x)}`;
    const ky = `${y.claimId}:${other(y)}`;
    return kx < ky ? -1 : kx > ky ? 1 : 0;
  };
};

export async function runConflictPass(
  opts: ConflictPassOptions,
): Promise<ConflictPassResult> {
  const cache = opts.store.judgeCache();
  const pairs = pairClaims(opts.claims);
  const outcomes: PairOutcome[] = new Array(pairs.length);
  const findings: ConflictFinding[] = [];
  const duplicates: DuplicateCandidate[] = [];
  const pending: string[] = [];
  let requests = 0;

  await pool(pairs, MAX_IN_FLIGHT, async (pair, index) => {
    const { a, b, request, key } = conflictRequestFor(
      opts.model,
      pair,
      opts.context,
    );
    let result: ConflictResult;
    const hit = await cache.get(key);
    if (hit !== undefined) {
      result = mapConflictAnswers(hit.answers as Record<string, Answer>);
    } else {
      let response: Awaited<ReturnType<Judge["conflict"]>>;
      try {
        response = await opts.judge.conflict(a, b);
      } catch (e) {
        if (!(e instanceof JudgeUnavailable)) throw e;
        pending.push(pair.pairId);
        outcomes[index] = {
          pairId: pair.pairId,
          why: pair.why,
          outcome: "pending",
          reason: `judge unavailable: ${e.message}`,
        };
        return;
      }
      requests++;
      const entry: JudgeCacheEntry = {
        key,
        model: response.model,
        judgment: "conflict",
        subject: { pairId: pair.pairId },
        request,
        answers: response.answers,
        at: new Date().toISOString(),
      };
      await cache.put(entry);
      result = mapConflictAnswers(response.answers as Record<string, Answer>);
    }

    const decision = decidePair(
      pair.a.sentence,
      pair.b.sentence,
      result,
      opts.threshold,
    );
    let outcome: PairOutcomeKind = decision.outcome;
    let reason = decision.reason;
    if (decision.outcome === "conflict" || decision.outcome === "review") {
      if (isAcked(pair)) {
        outcome = "acked";
        reason = `acknowledged: ${decision.reason}`;
      } else {
        for (const [self, other] of [
          [pair.a, pair.b],
          [pair.b, pair.a],
        ] as const) {
          const finding: ConflictFinding = {
            claimId: self.id,
            otherClaimId: other.id,
            pairId: pair.pairId,
            probability: decision.probability,
            reason: decision.reason,
          };
          if (decision.outcome === "review") finding.review = true;
          findings.push(finding);
        }
      }
    } else if (decision.outcome === "duplicate") {
      for (const [self, other] of [
        [pair.a, pair.b],
        [pair.b, pair.a],
      ] as const) {
        duplicates.push({
          claimId: self.id,
          duplicateOf: other.id,
          pairId: pair.pairId,
          probability: decision.probability,
          reason: decision.reason,
        });
      }
    }
    outcomes[index] = { pairId: pair.pairId, why: pair.why, outcome, reason };
  });

  return {
    findings: findings.sort(byIds((f) => f.otherClaimId)),
    duplicates: duplicates.sort(byIds((d) => d.duplicateOf)),
    pending: pending.sort(),
    requests,
    pairs: outcomes,
  };
}

/**
 * The gating findings on one claim, in the verdict's `conflicts[]` shape
 * (SPEC 7.2). Review findings are left out; see `conflictReviewsFor`.
 */
export function conflictsFor(
  result: ConflictPassResult,
  claimId: string,
): { claimId: string; probability: number; reason: string }[] {
  return result.findings
    .filter((f) => f.claimId === claimId && f.review !== true)
    .map((f) => ({
      claimId: f.otherClaimId,
      probability: f.probability,
      reason: f.reason,
    }));
}

/** The review (warn-only) findings on one claim, same shape as `conflictsFor`. */
export function conflictReviewsFor(
  result: ConflictPassResult,
  claimId: string,
): { claimId: string; probability: number; reason: string }[] {
  return result.findings
    .filter((f) => f.claimId === claimId && f.review === true)
    .map((f) => ({
      claimId: f.otherClaimId,
      probability: f.probability,
      reason: f.reason,
    }));
}

/**
 * The claim this one most likely duplicates: the candidate with the highest
 * probability, ties broken by the smaller claim id. `null` when none.
 */
export function duplicateOf(
  result: ConflictPassResult,
  claimId: string,
): string | null {
  let best: DuplicateCandidate | null = null;
  for (const d of result.duplicates) {
    if (d.claimId !== claimId) continue;
    if (
      best === null ||
      d.probability > best.probability ||
      (d.probability === best.probability && d.duplicateOf < best.duplicateOf)
    ) {
      best = d;
    }
  }
  return best?.duplicateOf ?? null;
}
