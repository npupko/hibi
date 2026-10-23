import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Claim } from "../src/core/model.ts";
import {
  type ConflictPassResult,
  conflictRequestFor,
  conflictReviewsFor,
  conflictsFor,
  decidePair,
  duplicateOf,
  MAX_IN_FLIGHT,
  runConflictPass,
} from "../src/engine/conflict.ts";
import { sentenceHash } from "../src/engine/hash.ts";
import { pairClaims } from "../src/engine/pairing.ts";
import { cacheKey } from "../src/judge/cache.ts";
import {
  conflictAnswer,
  encodeConflict,
  FakeJudge,
  pairKey,
} from "../src/judge/fake.ts";
import { buildConflictRequest } from "../src/judge/jev.ts";
import type {
  ConflictResult,
  ConflictSide,
  Judge,
  JudgeResponse,
} from "../src/judge/types.ts";
import { ClaimStore } from "../src/store/store.ts";

const MODEL = "jev-1.14.0";
const THRESHOLD = 0.8;
const ID_A = "clm_000000000000000a";
const ID_B = "clm_000000000000000b";
const PAIR = `${ID_A}:${ID_B}`;

type Attestation = Claim["attestations"][number];

function claim(
  id: string,
  sentence: string,
  attestations: Attestation[] = [],
): Claim {
  const file = `docs/${id}.md`;
  return {
    id,
    documentId: `doc_${Bun.hash.xxHash64(file).toString(16).padStart(16, "0")}`,
    kind: "attested",
    sentence,
    doc: {
      file,
      selectors: [
        { kind: "text-quote", exact: sentence, prefix: "", suffix: "" },
      ],
    },
    evidence: [],
    sources: [],
    procedure: null,
    ttl: "P90D",
    enforcement: "enforced",
    attestations,
    createdAt: "2026-10-22T00:00:00Z",
    ref: "HEAD",
  };
}

function ack(pairId: string, hashes: [string, string]): Attestation {
  return {
    by: "human:alice@example.com",
    at: "2026-10-22T10:00:00Z",
    basis: "ack",
    note: "both are true for different plans",
    ack: { pairId, sentenceHashes: hashes },
  } as Attestation;
}

const context = (c: Claim) => `context of ${c.id}`;

const GATING = conflictAnswer(
  { consistent: 0.02, contradictory: 0.96, unrelated: 0.02 },
  0.95,
  0.9,
);
const CONSISTENT = conflictAnswer(
  { consistent: 0.95, contradictory: 0.03, unrelated: 0.02 },
  0.92,
  0.05,
);

let dir: string;
let store: ClaimStore;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "hibi-conflict-"));
  store = await ClaimStore.init(dir);
});
afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

function judgeFor(answers: Record<string, ConflictResult>): FakeJudge {
  return new FakeJudge({ conflict: answers });
}

async function run(
  judge: Judge,
  claims: Claim[],
  threshold = THRESHOLD,
): Promise<ConflictPassResult> {
  return runConflictPass({
    store,
    judge,
    claims,
    context,
    threshold,
  });
}

/** Two claims that pair on the rare token `stripe`. */
const A = "Stripe charges the Pro plan monthly.";
const B = "Stripe charges the Pro plan yearly.";
/** Two claims with differing money values that pair on `stripe`. */
const PRICE_A = "Stripe bills Pro at $29/month.";
const PRICE_B = "Stripe bills Pro at $39/month.";

describe("decisions", () => {
  test("contradictory with confidence and same_subject at threshold gates on both claims", async () => {
    const result = await run(judgeFor({ [pairKey(A, B)]: GATING }), [
      claim(ID_A, A),
      claim(ID_B, B),
    ]);
    const reason = "contradictory (p=0.96, same_subject=0.95)";
    expect(result.findings).toEqual([
      {
        claimId: ID_A,
        otherClaimId: ID_B,
        pairId: PAIR,
        probability: 0.96,
        reason,
      },
      {
        claimId: ID_B,
        otherClaimId: ID_A,
        pairId: PAIR,
        probability: 0.96,
        reason,
      },
    ]);
    expect(conflictsFor(result, ID_A)).toEqual([
      { claimId: ID_B, probability: 0.96, reason },
    ]);
    expect(conflictReviewsFor(result, ID_A)).toEqual([]);
    expect(result.duplicates).toEqual([]);
    expect(result.pairs).toEqual([
      {
        pairId: PAIR,
        why: "shared rare tokens: stripe, charges, pro, plan",
        outcome: "conflict",
        reason,
      },
    ]);
    expect(result.requests).toBe(1);
  });

  test("contradictory below threshold is a review finding", async () => {
    const low = conflictAnswer(
      { consistent: 0.2, contradictory: 0.6, unrelated: 0.2 },
      0.9,
      0.8,
    );
    const result = await run(judgeFor({ [pairKey(A, B)]: low }), [
      claim(ID_A, A),
      claim(ID_B, B),
    ]);
    expect(result.findings).toHaveLength(2);
    expect(result.findings.every((f) => f.review === true)).toBe(true);
    expect(result.findings[0]?.reason).toBe(
      "review: contradictory, confidence=0.40 below threshold 0.80",
    );
    expect(conflictsFor(result, ID_A)).toEqual([]);
    expect(conflictReviewsFor(result, ID_B)).toEqual([
      {
        claimId: ID_A,
        probability: 0.6,
        reason: "review: contradictory, confidence=0.40 below threshold 0.80",
      },
    ]);
  });

  test("contradictory with contradicts below 0.5 is a review finding", async () => {
    const disagree = conflictAnswer(
      { consistent: 0.02, contradictory: 0.96, unrelated: 0.02 },
      0.95,
      0.3,
    );
    const result = await run(judgeFor({ [pairKey(A, B)]: disagree }), [
      claim(ID_A, A),
      claim(ID_B, B),
    ]);
    expect(result.findings.map((f) => [f.review, f.reason])).toEqual([
      [true, "review: contradictory, contradicts=0.30 below 0.50"],
      [true, "review: contradictory, contradicts=0.30 below 0.50"],
    ]);
  });

  test("contradictory with same_subject between the floor and the threshold is a review finding", () => {
    const r = conflictAnswer(
      { consistent: 0.02, contradictory: 0.96, unrelated: 0.02 },
      0.6,
      0.9,
    );
    expect(decidePair(A, B, r, THRESHOLD)).toEqual({
      outcome: "review",
      probability: 0.96,
      reason: "review: contradictory, same_subject=0.60 below threshold 0.80",
    });
  });

  test("after same_subject and contradicts pass, differing values are a finding with probability 1 whatever the relation", async () => {
    const a = "Stripe bills Pro at $29/month.";
    const b = "Stripe bills Pro at $39/month.";
    const leaning = conflictAnswer(
      { consistent: 0.95, contradictory: 0.03, unrelated: 0.02 },
      0.92,
      0.6,
    );
    const result = await run(judgeFor({ [pairKey(a, b)]: leaning }), [
      claim(ID_A, a),
      claim(ID_B, b),
    ]);
    expect(
      result.findings.map((f) => [
        f.claimId,
        f.probability,
        f.reason,
        f.review,
      ]),
    ).toEqual([
      [ID_A, 1, "numbers differ: 29 vs 39", undefined],
      [ID_B, 1, "numbers differ: 29 vs 39", undefined],
    ]);
    expect(result.duplicates).toEqual([]);
  });

  test("differing values with contradicts below 0.5 do not fire the value rule; the judge's outcome stands", async () => {
    // Live shape (jev-1.14.0): two exit codes from different situations.
    const a = "`check --doc` on a path with no registered claims exits 1.";
    const b = "exit 2 means a sentence in it is stale.";
    const consistent = conflictAnswer(
      { consistent: 0.8, contradictory: 0.1, unrelated: 0.1 },
      0.85,
      0.34,
    );
    expect(decidePair(a, b, consistent, THRESHOLD)).toEqual({
      outcome: "duplicate",
      probability: 0.8,
      reason: "consistent and same subject (same_subject=0.85)",
    });
    const leansContradictory = conflictAnswer(
      { consistent: 0.13, contradictory: 0.74, unrelated: 0.13 },
      0.93,
      0.34,
    );
    expect(decidePair(a, b, leansContradictory, THRESHOLD)).toEqual({
      outcome: "review",
      probability: 0.74,
      reason:
        "review: contradictory, contradicts=0.34 below 0.50, confidence=0.61 below threshold 0.80",
    });

    const unrelated = conflictAnswer(
      { consistent: 0.3, contradictory: 0.1, unrelated: 0.6 },
      0.85,
      0.34,
    );
    const result = await run(
      judgeFor({ [pairKey(PRICE_A, PRICE_B)]: unrelated }),
      [claim(ID_A, PRICE_A), claim(ID_B, PRICE_B)],
    );
    expect(result.findings).toEqual([]);
    expect(result.pairs[0]).toMatchObject({
      outcome: "clean",
      reason: "unrelated (confidence=0.40, same_subject=0.85)",
    });
  });

  test("differing values with contradicts at or above 0.5 are a conflict with probability 1", async () => {
    // Live shape (jev-1.14.0): two retry limits for the same client.
    const a = "Retries are capped at 3 attempts.";
    const b = "The client retries a failed request up to 5 times.";
    const r = conflictAnswer(
      { consistent: 0.13, contradictory: 0.74, unrelated: 0.13 },
      0.93,
      0.86,
    );
    expect(decidePair(a, b, r, THRESHOLD)).toEqual({
      outcome: "conflict",
      probability: 1,
      reason: "numbers differ: 3 vs 5",
    });
    const result = await run(judgeFor({ [pairKey(a, b)]: r }), [
      claim(ID_A, a),
      claim(ID_B, b),
    ]);
    expect(conflictsFor(result, ID_A)).toEqual([
      { claimId: ID_B, probability: 1, reason: "numbers differ: 3 vs 5" },
    ]);
    expect(conflictsFor(result, ID_B)).toEqual([
      { claimId: ID_A, probability: 1, reason: "numbers differ: 3 vs 5" },
    ]);
  });

  test("the contradicts floor applies to every value group", () => {
    const cases: [string, string, string][] = [
      [
        "Pro allows 10 seats.",
        "Pro allows 20 seats.",
        "numbers differ: 10 vs 20",
      ],
      [
        "Stripe bills Pro at $29/month.",
        "Stripe bills Pro at $39/month.",
        "numbers differ: 29 vs 39",
      ],
      [
        "The CLI requires Bun v1.2.",
        "The CLI requires Bun v1.3.",
        "versions differ: 1.2 vs 1.3",
      ],
      [
        "The migration window closes on 2026-10-01.",
        "The migration window closes on 2026-11-01.",
        "dates differ: 2026-10-01 vs 2026-11-01",
      ],
    ];
    const probabilities = {
      consistent: 0.9,
      contradictory: 0.05,
      unrelated: 0.05,
    };
    for (const [a, b, label] of cases) {
      expect(
        decidePair(a, b, conflictAnswer(probabilities, 0.9, 0.49), THRESHOLD),
      ).toEqual({
        outcome: "duplicate",
        probability: 0.9,
        reason: "consistent and same subject (same_subject=0.90)",
      });
      expect(
        decidePair(a, b, conflictAnswer(probabilities, 0.9, 0.5), THRESHOLD),
      ).toEqual({ outcome: "conflict", probability: 1, reason: label });
    }
  });

  test("the value rule does not run when same_subject is below threshold", () => {
    const r = conflictAnswer(
      { consistent: 0.1, contradictory: 0.1, unrelated: 0.8 },
      0.5,
      0.9,
    );
    expect(
      decidePair("Pro costs $29.", "Team costs $39.", r, THRESHOLD),
    ).toEqual({
      outcome: "clean",
      reason: "unrelated (confidence=0.70, same_subject=0.50)",
    });
  });

  test("consistent with same_subject at threshold is a duplicate candidate in both directions", async () => {
    const result = await run(judgeFor({ [pairKey(A, B)]: CONSISTENT }), [
      claim(ID_A, A),
      claim(ID_B, B),
    ]);
    const reason = "consistent and same subject (same_subject=0.92)";
    expect(result.findings).toEqual([]);
    expect(result.duplicates).toEqual([
      {
        claimId: ID_A,
        duplicateOf: ID_B,
        pairId: PAIR,
        probability: 0.95,
        reason,
      },
      {
        claimId: ID_B,
        duplicateOf: ID_A,
        pairId: PAIR,
        probability: 0.95,
        reason,
      },
    ]);
    expect(duplicateOf(result, ID_A)).toBe(ID_B);
    expect(duplicateOf(result, ID_B)).toBe(ID_A);
    expect(duplicateOf(result, "clm_ffffffffffffffff")).toBeNull();
  });

  test("consistent with same_subject below threshold is clean", () => {
    const r = conflictAnswer(
      { consistent: 0.9, contradictory: 0.05, unrelated: 0.05 },
      0.6,
    );
    expect(decidePair(A, B, r, THRESHOLD).outcome).toBe("clean");
  });

  test("a forced same_subject is labelled and never a finding, even with differing values", async () => {
    const a = "Stripe bills Pro at $29/month.";
    const b = "Stripe bills Pro at $39/month.";
    const forced: ConflictResult = {
      ...conflictAnswer(
        { consistent: 0.02, contradictory: 0.96, unrelated: 0.02 },
        0.12,
        0.9,
      ),
      relation: "unrelated",
      confidence: 0.76,
      forced: "same_subject",
    };
    const result = await run(judgeFor({ [pairKey(a, b)]: forced }), [
      claim(ID_A, a),
      claim(ID_B, b),
    ]);
    expect(result.findings).toEqual([]);
    expect(result.duplicates).toEqual([]);
    expect(result.pairs[0]?.outcome).toBe("unrelated");
    expect(result.pairs[0]?.reason).toBe(
      "not the same subject (same_subject=0.12)",
    );
  });
});

describe("cache", () => {
  test("a second run makes no judge calls and gives the same result", async () => {
    const judge = judgeFor({ [pairKey(A, B)]: GATING });
    const claims = [claim(ID_A, A), claim(ID_B, B)];
    const first = await run(judge, claims);
    expect(judge.calls).toHaveLength(1);
    const second = await run(judge, claims);
    expect(judge.calls).toHaveLength(1);
    expect(second.requests).toBe(0);
    expect(second.findings).toEqual(first.findings);

    const entries = await store.judgeCache().list({ pairId: PAIR });
    expect(entries).toHaveLength(1);
    expect(entries[0]?.judgment).toBe("conflict");
    expect(entries[0]?.subject).toEqual({ pairId: PAIR });
    const sideA = {
      text: A,
      doc: `docs/${ID_A}.md`,
      context: `context of ${ID_A}`,
    };
    const sideB = {
      text: B,
      doc: `docs/${ID_B}.md`,
      context: `context of ${ID_B}`,
    };
    expect(entries[0]?.key).toBe(
      cacheKey(MODEL, buildConflictRequest(MODEL, sideA, sideB)),
    );
  });

  test("a changed sentence is a new key and asks again", async () => {
    const b2 = "Stripe charges the Pro plan weekly.";
    const judge = judgeFor({
      [pairKey(A, B)]: GATING,
      [pairKey(A, b2)]: GATING,
    });
    await run(judge, [claim(ID_A, A), claim(ID_B, B)]);
    await run(judge, [claim(ID_A, A), claim(ID_B, b2)]);
    expect(judge.calls).toHaveLength(2);
  });

  test("a cached entry holding raw Jev answers is mapped by the adapter", async () => {
    const claims = [claim(ID_A, A), claim(ID_B, B)];
    const [pair] = pairClaims(claims);
    if (!pair) throw new Error("expected a pair");
    const { request, key } = conflictRequestFor(MODEL, pair, context);
    await store.judgeCache().put({
      key,
      model: MODEL,
      judgment: "conflict",
      subject: { pairId: PAIR },
      request,
      answers: {
        same_subject: { noul: 0.95 },
        relation: {
          choice: "contradictory",
          confidence: 0.94,
          probabilities: {
            consistent: 0.02,
            contradictory: 0.96,
            unrelated: 0.02,
          },
        },
        contradicts: { noul: 0.9 },
      },
      at: "2026-10-22T00:00:00Z",
    });
    const judge = judgeFor({});
    const result = await run(judge, claims);
    expect(judge.calls).toHaveLength(0);
    expect(conflictsFor(result, ID_A)).toEqual([
      {
        claimId: ID_B,
        probability: 0.96,
        reason: "contradictory (p=0.96, same_subject=0.95)",
      },
    ]);
  });
});

describe("cached answers and the value rule", () => {
  async function putRaw(claims: Claim[], contradicts: number): Promise<void> {
    const [pair] = pairClaims(claims);
    if (!pair) throw new Error("expected a pair");
    const { request, key } = conflictRequestFor(MODEL, pair, context);
    await store.judgeCache().put({
      key,
      model: MODEL,
      judgment: "conflict",
      subject: { pairId: PAIR },
      request,
      answers: {
        same_subject: { noul: 0.93 },
        relation: {
          choice: "consistent",
          confidence: 0.85,
          probabilities: {
            consistent: 0.9,
            contradictory: 0.05,
            unrelated: 0.05,
          },
        },
        contradicts: { noul: contradicts },
      },
      at: "2026-10-22T00:00:00Z",
    });
  }

  test("a cached verdict with contradicts below 0.5 gives no value finding and asks nothing", async () => {
    const claims = [claim(ID_A, PRICE_A), claim(ID_B, PRICE_B)];
    await putRaw(claims, 0.2);
    const judge = judgeFor({});
    const result = await run(judge, claims);
    expect(judge.calls).toHaveLength(0);
    expect(result.requests).toBe(0);
    expect(result.findings).toEqual([]);
    expect(result.pairs[0]?.outcome).toBe("duplicate");
  });

  test("a cached verdict with contradicts at or above 0.5 gives the value finding and asks nothing", async () => {
    const claims = [claim(ID_A, PRICE_A), claim(ID_B, PRICE_B)];
    await putRaw(claims, 0.7);
    const judge = judgeFor({});
    const result = await run(judge, claims);
    expect(judge.calls).toHaveLength(0);
    expect(result.requests).toBe(0);
    expect(conflictsFor(result, ID_A)).toEqual([
      { claimId: ID_B, probability: 1, reason: "numbers differ: 29 vs 39" },
    ]);
  });
});

describe("pending", () => {
  test("an unavailable judge leaves the pair pending and caches nothing", async () => {
    const judge = judgeFor({ [pairKey(A, B)]: GATING });
    judge.setUnavailable("no API key");
    const claims = [claim(ID_A, A), claim(ID_B, B)];
    const result = await run(judge, claims);
    expect(result.pending).toEqual([PAIR]);
    expect(result.findings).toEqual([]);
    expect(result.requests).toBe(0);
    expect(result.pairs[0]).toMatchObject({
      outcome: "pending",
      reason: "judge unavailable: no API key",
    });
    expect(await store.judgeCache().list()).toEqual([]);

    judge.setUnavailable(null);
    const again = await run(judge, claims);
    expect(again.pending).toEqual([]);
    expect(again.requests).toBe(1);
  });

  test("a judge error leaves the pair pending with its label and never aborts the pass", async () => {
    const judge = judgeFor({ [pairKey(A, B)]: GATING });
    judge.setError("TypeSafe returned HTTP 400: bad request");
    const claims = [claim(ID_A, A), claim(ID_B, B)];
    const result = await run(judge, claims);
    expect(result.pending).toEqual([PAIR]);
    expect(result.findings).toEqual([]);
    expect(result.pairs[0]).toMatchObject({
      outcome: "pending",
      reason: "judge error: TypeSafe returned HTTP 400: bad request",
    });
    expect(await store.judgeCache().list()).toEqual([]);
  });

  test("the cache key uses the judge's model", async () => {
    const judge = new FakeJudge(
      { conflict: { [pairKey(A, B)]: GATING } },
      "jev-9.9.9",
    );
    await run(judge, [claim(ID_A, A), claim(ID_B, B)]);
    const [entry] = await store.judgeCache().list();
    const pair = pairClaims([claim(ID_A, A), claim(ID_B, B)])[0];
    if (pair === undefined) throw new Error("expected a pair");
    expect(entry?.key).toBe(conflictRequestFor("jev-9.9.9", pair, context).key);
  });
});

describe("scope", () => {
  // Shares the rare token `monthly` with A only.
  const C = "Invoices go out monthly for every seat.";
  const ID_C = "clm_000000000000000c";

  test("with a scope only pairs with a claim in it are judged; pairing still sees every claim", async () => {
    const judge = new FakeJudge({ conflict: () => GATING });
    const claims = [claim(ID_A, A), claim(ID_B, B), claim(ID_C, C)];
    const all = await runConflictPass({
      store,
      judge,
      claims,
      context,
      threshold: THRESHOLD,
    });
    expect(all.scope).toBe("store");
    expect(all.pairs.map((p) => p.pairId)).toEqual([PAIR, `${ID_A}:${ID_C}`]);
    const scoped = await runConflictPass({
      store,
      judge,
      claims,
      context,
      threshold: THRESHOLD,
      scope: new Set([ID_C]),
    });
    expect(scoped.scope).toBe("doc");
    expect(scoped.pairs.map((p) => p.pairId)).toEqual([`${ID_A}:${ID_C}`]);
  });
});

describe("ack suppression", () => {
  const hashes: [string, string] = [sentenceHash(A), sentenceHash(B)];

  test("an ack on either claim for the current sentences drops the finding", async () => {
    for (const on of ["a", "b"] as const) {
      const judge = judgeFor({ [pairKey(A, B)]: GATING });
      const result = await run(judge, [
        claim(ID_A, A, on === "a" ? [ack(PAIR, hashes)] : []),
        claim(ID_B, B, on === "b" ? [ack(PAIR, hashes)] : []),
      ]);
      expect(result.findings).toEqual([]);
      expect(result.pairs[0]).toMatchObject({
        outcome: "acked",
        reason: "acknowledged: contradictory (p=0.96, same_subject=0.95)",
      });
    }
  });

  test("hashes ignore whitespace differences", async () => {
    const result = await run(judgeFor({ [pairKey(A, B)]: GATING }), [
      claim(ID_A, A, [
        ack(PAIR, [sentenceHash(`  ${A.replaceAll(" ", "\n ")} `), hashes[1]]),
      ]),
      claim(ID_B, B),
    ]);
    expect(result.findings).toEqual([]);
  });

  test("a changed sentence ends the suppression", async () => {
    const b2 = "Stripe charges the Pro plan weekly.";
    const result = await run(judgeFor({ [pairKey(A, b2)]: GATING }), [
      claim(ID_A, A, [ack(PAIR, hashes)]),
      claim(ID_B, b2),
    ]);
    expect(result.findings).toHaveLength(2);
  });

  test("hashes in the wrong order, or for another pair, do not suppress", async () => {
    for (const a of [
      ack(PAIR, [hashes[1], hashes[0]]),
      ack(`${ID_A}:clm_000000000000000c`, hashes),
    ]) {
      const result = await run(judgeFor({ [pairKey(A, B)]: GATING }), [
        claim(ID_A, A, [a]),
        claim(ID_B, B),
      ]);
      expect(result.findings).toHaveLength(2);
    }
  });

  test("an ack drops review findings too", async () => {
    const low = conflictAnswer(
      { consistent: 0.2, contradictory: 0.6, unrelated: 0.2 },
      0.9,
      0.8,
    );
    const result = await run(judgeFor({ [pairKey(A, B)]: low }), [
      claim(ID_A, A, [ack(PAIR, hashes)]),
      claim(ID_B, B),
    ]);
    expect(result.findings).toEqual([]);
  });

  test("an ack does not drop duplicate candidates", async () => {
    const result = await run(judgeFor({ [pairKey(A, B)]: CONSISTENT }), [
      claim(ID_A, A, [ack(PAIR, hashes)]),
      claim(ID_B, B),
    ]);
    expect(result.duplicates).toHaveLength(2);
  });
});

describe("side order", () => {
  test("swapping a and b yields the same request and cache key", () => {
    const ca = claim(ID_A, A);
    const cb = claim(ID_B, B);
    const forward = conflictRequestFor(
      MODEL,
      { a: ca, b: cb, pairId: PAIR, why: "" },
      context,
    );
    const swapped = conflictRequestFor(
      MODEL,
      { a: cb, b: ca, pairId: PAIR, why: "" },
      context,
    );
    expect(swapped.key).toBe(forward.key);
    expect(swapped.request).toEqual(forward.request);
    expect(forward.a.text).toBe(A);
  });

  test("the judge sees the smaller claim id as side a whatever the input order", async () => {
    // B's text sorts before A's, and the claims are passed in reverse.
    const judge = judgeFor({ [pairKey(A, B)]: GATING });
    await run(judge, [claim(ID_B, A), claim(ID_A, B)]);
    const sent = judge.calls[0]?.request as {
      a: ConflictSide;
      b: ConflictSide;
    };
    expect(sent.a.text).toBe(B);
    expect(sent.a.doc).toBe(`docs/${ID_A}.md`);
  });
});

test("retired claims are not judged", async () => {
  const judge = judgeFor({ [pairKey(A, B)]: GATING });
  const retired = { ...claim(ID_B, B), enforcement: "retired" as const };
  const result = await run(judge, [claim(ID_A, A), retired]);
  expect(result.pairs).toEqual([]);
  expect(judge.calls).toHaveLength(0);
});

test(`at most ${MAX_IN_FLIGHT} judge calls are in flight`, async () => {
  let inFlight = 0;
  let peak = 0;
  const judge: Judge = {
    model: "slow-1.0.0",
    triage: () => Promise.reject(new Error("unused")),
    supports: () => Promise.reject(new Error("unused")),
    async conflict(a, b): Promise<JudgeResponse<ConflictResult>> {
      inFlight++;
      peak = Math.max(peak, inFlight);
      await Bun.sleep(5);
      inFlight--;
      return {
        model: "slow-1.0.0",
        request: { a, b },
        answers: encodeConflict(CONSISTENT),
        result: CONSISTENT,
      };
    },
  };
  // 30 claims sharing one rare-enough token pair by pair through unique tokens.
  const claims = Array.from({ length: 30 }, (_, i) =>
    claim(
      `clm_${(i + 1).toString(16).padStart(16, "0")}`,
      `Topic${"x".repeat(Math.floor(i / 2) + 1)} value.`,
    ),
  );
  const result = await run(judge, claims);
  expect(result.requests).toBe(15);
  expect(peak).toBeLessThanOrEqual(MAX_IN_FLIGHT);
  expect(peak).toBeGreaterThan(1);
});

test("sentenceHash: sha256 of the NFC-normalized, whitespace-collapsed, trimmed text", () => {
  const sha = (s: string) =>
    new Bun.CryptoHasher("sha256").update(s).digest("hex");
  expect(sentenceHash("  Pro\n costs\t$29. ")).toBe(sha("Pro costs $29."));
  expect(sentenceHash("Café opens.")).toBe(sha("Café opens."));
});
