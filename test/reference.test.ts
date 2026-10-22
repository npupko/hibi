import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Claim } from "../src/core/model.ts";
import { buildSelectorBundle } from "../src/engine/anchor.ts";
import { claimContext } from "../src/engine/context.ts";
import { judgeReference, supportsOutcome } from "../src/engine/reference.ts";
import { cacheKey, JudgeCache } from "../src/judge/cache.ts";
import { FakeJudge, supportsAnswer } from "../src/judge/fake.ts";
import { buildSupportsRequest } from "../src/judge/jev.ts";
import type { SupportsResult } from "../src/judge/types.ts";

const MODEL = "jev-1.14.0";
const SENTENCE = "Pro costs $29/month.";
const DOC = `# Pricing\n\n${SENTENCE}\n`;
const REFERENCED = "The Pro plan is billed at $29 per month.";

const claim: Pick<Claim, "id" | "sentence" | "doc" | "evidence"> = {
  id: "clm_00000000000000aa",
  sentence: SENTENCE,
  doc: buildSelectorBundle("docs/pricing.md", DOC, { start: 11, end: 31 }),
  evidence: [
    buildSelectorBundle("docs/billing.md", REFERENCED, { start: 0, end: 40 }),
  ],
};

/** A Jev-shaped `answers` object, as the TypeSafe API returns and the cache stores. */
function jevAnswers(
  p: { supports: number; contradicts: number; says_nothing: number },
  addresses = 0.95,
) {
  const choice = (Object.entries(p) as [string, number][]).reduce((a, b) =>
    b[1] > a[1] ? b : a,
  )[0];
  return {
    relation: { choice, confidence: 0, probabilities: p },
    addresses: { noul: addresses },
  };
}

let dirs: string[] = [];
afterEach(async () => {
  await Promise.all(dirs.map((d) => rm(d, { recursive: true, force: true })));
  dirs = [];
});
async function cache(): Promise<JudgeCache> {
  const d = await mkdtemp(join(tmpdir(), "hibi-ref-"));
  dirs.push(d);
  return new JudgeCache(join(d, "judge"));
}

const request = () =>
  buildSupportsRequest(MODEL, {
    sentence: SENTENCE,
    context: claimContext(DOC, claim),
    evidence: REFERENCED,
    source: "docs/billing.md",
  });

describe("supportsOutcome (SPEC 8.4, one threshold)", () => {
  const r = (over: Partial<SupportsResult>): SupportsResult => ({
    relation: "supports",
    probabilities: { supports: 1, refutes: 0, does_not_address: 0 },
    confidence: 0.9,
    addresses: 0.9,
    ...over,
  });

  test("supports at or above threshold is supported", () => {
    expect(supportsOutcome(r({ confidence: 0.8 }), 0.8)).toEqual({
      state: "supported",
      reasons: [
        "judge: evidence supports the sentence (confidence 0.80 at or above threshold 0.8)",
      ],
    });
  });
  test("refutes at or above threshold is refuted", () => {
    expect(supportsOutcome(r({ relation: "refutes" }), 0.8).state).toBe(
      "refuted",
    );
  });
  test("either relation below threshold is review", () => {
    for (const relation of ["supports", "refutes"] as const) {
      const out = supportsOutcome(r({ relation, confidence: 0.5 }), 0.8);
      expect(out.state).toBe("review");
      expect(out.reasons[0]).toContain(
        "below threshold (confidence 0.50 < 0.8)",
      );
    }
  });
  test("does_not_address is review; a forced one carries the addresses label", () => {
    expect(
      supportsOutcome(
        r({ relation: "does_not_address", confidence: 0.9 }),
        0.8,
      ),
    ).toEqual({
      state: "review",
      reasons: [
        "judge: evidence does not address the sentence (confidence 0.90)",
      ],
    });
    expect(
      supportsOutcome(
        r({
          relation: "does_not_address",
          addresses: 0.12,
          forced: "addresses",
        }),
        0.8,
      ),
    ).toEqual({
      state: "review",
      reasons: ["evidence did not address the sentence (addresses=0.12)"],
    });
  });
  test("the threshold comes from the caller", () => {
    expect(supportsOutcome(r({ confidence: 0.6 }), 0.5).state).toBe(
      "supported",
    );
  });
});

describe("judgeReference", () => {
  test("a cache hit answers without calling the judge", async () => {
    const c = await cache();
    const req = request();
    const key = cacheKey(MODEL, req);
    await c.put({
      key,
      model: MODEL,
      judgment: "supports",
      subject: { claimId: claim.id },
      request: req,
      answers: jevAnswers({
        supports: 0.1,
        contradicts: 0.85,
        says_nothing: 0.05,
      }),
      at: "2026-01-01T00:00:00.000Z",
    });
    const judge = new FakeJudge();
    const out = await judgeReference({
      claim,
      docText: DOC,
      referencedText: REFERENCED,
      judge,
      cache: c,
      model: MODEL,
      threshold: 0.7,
    });
    expect(judge.calls).toHaveLength(0);
    expect(out.state).toBe("refuted");
    expect(out.key).toBe(key);
  });

  test("a miss calls supports once and caches the verdict with no evidence block", async () => {
    const c = await cache();
    const judge = new FakeJudge({
      supports: {
        [SENTENCE]: supportsAnswer({
          supports: 0.97,
          refutes: 0.02,
          does_not_address: 0.01,
        }),
      },
    });
    const out = await judgeReference({
      claim,
      docText: DOC,
      referencedText: REFERENCED,
      judge,
      cache: c,
      model: MODEL,
      threshold: 0.8,
      now: new Date("2026-02-03T04:05:06Z"),
    });
    expect(out.state).toBe("supported");
    expect(judge.calls).toHaveLength(1);
    expect(judge.calls[0]?.request).toEqual({
      sentence: SENTENCE,
      context: "Pricing\n\nPro costs $29/month.",
      evidence: REFERENCED,
      source: "docs/billing.md",
    });
    const entries = await c.list({ claimId: claim.id });
    expect(entries).toHaveLength(1);
    const entry = entries[0];
    expect(entry?.key).toBe(cacheKey(MODEL, request()));
    expect(entry?.judgment).toBe("supports");
    expect(entry?.subject).toEqual({ claimId: claim.id });
    expect(entry?.request).toEqual(request());
    expect(entry?.evidence).toBeUndefined();
    expect(entry?.at).toBe("2026-02-03T04:05:06.000Z");
  });

  test("JudgeUnavailable is pending and caches nothing", async () => {
    const c = await cache();
    const judge = new FakeJudge();
    judge.setUnavailable("no TYPESAFE_API_KEY");
    const out = await judgeReference({
      claim,
      docText: DOC,
      referencedText: REFERENCED,
      judge,
      cache: c,
      model: MODEL,
      threshold: 0.8,
    });
    expect(out).toEqual({
      state: "pending",
      reasons: ["judge: pending: no TYPESAFE_API_KEY"],
    });
    expect(await c.list()).toHaveLength(0);
  });

  test("no judge at all is pending", async () => {
    const out = await judgeReference({
      claim,
      docText: DOC,
      referencedText: REFERENCED,
      cache: await cache(),
      model: MODEL,
      threshold: 0.8,
    });
    expect(out.state).toBe("pending");
    expect(out.reasons[0]).toBe(
      "judge: pending: no judge is configured (set TYPESAFE_API_KEY)",
    );
  });

  test("a forced does_not_address is review with the addresses label", async () => {
    const judge = new FakeJudge({
      supports: {
        [SENTENCE]: {
          relation: "does_not_address",
          probabilities: {
            supports: 0.9,
            refutes: 0.05,
            does_not_address: 0.05,
          },
          confidence: 0.6,
          addresses: 0.2,
          forced: "addresses",
        },
      },
    });
    const out = await judgeReference({
      claim,
      docText: DOC,
      referencedText: REFERENCED,
      judge,
      cache: await cache(),
      model: MODEL,
      threshold: 0.8,
    });
    expect(out.state).toBe("review");
    expect(out.reasons).toEqual([
      "evidence did not address the sentence (addresses=0.20)",
    ]);
  });

  test("a changed referenced text is a new request", async () => {
    const c = await cache();
    const judge = new FakeJudge({
      supports: () =>
        supportsAnswer({
          supports: 0.97,
          refutes: 0.02,
          does_not_address: 0.01,
        }),
    });
    const base = {
      claim,
      docText: DOC,
      judge,
      cache: c,
      model: MODEL,
      threshold: 0.8,
    };
    await judgeReference({ ...base, referencedText: REFERENCED });
    await judgeReference({
      ...base,
      referencedText: `${REFERENCED} Annual is $290.`,
    });
    expect(judge.calls).toHaveLength(2);
    expect(await c.list()).toHaveLength(2);
  });
});
