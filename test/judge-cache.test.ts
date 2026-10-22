import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  DEFAULT_JUDGE_MODEL,
  type JudgeCacheEntry,
} from "../src/core/model.ts";
import { cacheKey, canonicalJson, JudgeCache } from "../src/judge/cache.ts";
import {
  conflictAnswer,
  FakeJudge,
  pairKey,
  supportsAnswer,
} from "../src/judge/fake.ts";
import { buildSupportsRequest, mapSupportsAnswers } from "../src/judge/jev.ts";
import { JudgeUnavailable } from "../src/judge/types.ts";
import { makeRepo } from "./helpers.ts";

const MODEL = "jev-1.14.0";

/** A supports request with its keys deliberately out of order. */
const REQUEST = {
  state: { evidence: "price: 29", claim: "Pro costs $29/month." },
  questions: [
    {
      type: "choice",
      name: "relation",
      options: ["supports", "refutes", "does_not_address"],
    },
  ],
};

describe("canonical request JSON", () => {
  test("object keys are sorted, arrays keep their order, no whitespace", () => {
    expect(canonicalJson(REQUEST)).toBe(
      '{"questions":[{"name":"relation","options":["supports","refutes","does_not_address"],"type":"choice"}],"state":{"claim":"Pro costs $29/month.","evidence":"price: 29"}}',
    );
  });

  test("undefined keys are dropped and undefined array items become null", () => {
    expect(canonicalJson({ b: undefined, a: [1, undefined] })).toBe(
      '{"a":[1,null]}',
    );
  });

  test("values JSON cannot represent are refused", () => {
    expect(() => canonicalJson({ x: Number.NaN })).toThrow("non-finite");
    expect(() => canonicalJson({ x: 1n })).toThrow("bigint");
  });
});

describe("cache key", () => {
  test("fixture: sha256 of `<model>\\n<canonical JSON>`", () => {
    expect(cacheKey(MODEL, REQUEST)).toBe(
      "1caf95c54695fdd9ed556b46d0942fe1e9bd697d88600dcf23d85f726798fc94",
    );
  });

  test("same input, same key, whatever the object key order", () => {
    const reordered = {
      questions: [
        {
          options: ["supports", "refutes", "does_not_address"],
          name: "relation",
          type: "choice",
        },
      ],
      state: { claim: "Pro costs $29/month.", evidence: "price: 29" },
    };
    expect(cacheKey(MODEL, reordered)).toBe(cacheKey(MODEL, REQUEST));
  });

  test("the key changes when option order changes", () => {
    const swapped = structuredClone(REQUEST);
    swapped.questions[0] = {
      type: "choice",
      name: "relation",
      options: ["refutes", "supports", "does_not_address"],
    };
    expect(cacheKey(MODEL, swapped)).not.toBe(cacheKey(MODEL, REQUEST));
  });

  test("the key changes with the model id", () => {
    expect(cacheKey("jev-1.15.0", REQUEST)).not.toBe(cacheKey(MODEL, REQUEST));
  });
});

let dirs: string[] = [];
afterEach(async () => {
  await Promise.all(dirs.map((d) => rm(d, { recursive: true, force: true })));
  dirs = [];
});

function entry(
  over: Partial<JudgeCacheEntry> & { request: unknown },
): JudgeCacheEntry {
  return {
    key: cacheKey(MODEL, over.request),
    model: MODEL,
    judgment: "supports",
    subject: { claimId: "clm_0000000000000001" },
    answers: {
      relation: { supports: 0.95, refutes: 0.03, does_not_address: 0.02 },
    },
    at: "2026-10-22T09:00:00.000Z",
    ...over,
  };
}

describe("JudgeCache", () => {
  test("put writes judge/<key>.json and get reads it back", async () => {
    const dir = await mkdtemp(join(tmpdir(), "ce-judge-"));
    dirs.push(dir);
    const cache = new JudgeCache(join(dir, "judge"));
    const e = entry({ request: REQUEST });
    expect(await cache.get(e.key)).toBeUndefined();
    await cache.put(e);
    expect(await readdir(join(dir, "judge"))).toEqual([`${e.key}.json`]);
    expect(await cache.get(cache.key(MODEL, REQUEST))).toEqual(e);
  });

  test("put refuses an entry that does not match the schema", async () => {
    const dir = await mkdtemp(join(tmpdir(), "ce-judge-"));
    dirs.push(dir);
    const cache = new JudgeCache(dir);
    await expect(
      cache.put({ ...entry({ request: REQUEST }), key: "nope" }),
    ).rejects.toThrow();
  });

  test("list filters by every subject field given, oldest first", async () => {
    const dir = await mkdtemp(join(tmpdir(), "ce-judge-"));
    dirs.push(dir);
    const cache = new JudgeCache(dir);
    const a = entry({
      request: { n: 1 },
      subject: { claimId: "clm_a" },
      at: "2026-10-22T10:00:00.000Z",
    });
    const b = entry({
      request: { n: 2 },
      subject: { claimId: "clm_a", docId: "doc_x" },
      at: "2026-10-22T09:00:00.000Z",
    });
    const c = entry({
      request: { n: 3 },
      judgment: "conflict",
      subject: { pairId: "clm_a:clm_b" },
    });
    for (const e of [a, b, c]) await cache.put(e);
    expect((await cache.list({ claimId: "clm_a" })).map((e) => e.key)).toEqual([
      b.key,
      a.key,
    ]);
    expect(
      (await cache.list({ claimId: "clm_a", docId: "doc_x" })).map(
        (e) => e.key,
      ),
    ).toEqual([b.key]);
    expect(
      (await cache.list({ pairId: "clm_a:clm_b" })).map((e) => e.key),
    ).toEqual([c.key]);
    expect(await cache.list()).toHaveLength(3);
  });

  test("a corrupt entry names its file", async () => {
    const dir = await mkdtemp(join(tmpdir(), "ce-judge-"));
    dirs.push(dir);
    await writeFile(join(dir, "bad.json"), "{}");
    await expect(new JudgeCache(dir).list()).rejects.toThrow("judge/bad.json");
  });

  test("the store exposes its cache at .claims/judge", async () => {
    const repo = await makeRepo();
    try {
      const cache = repo.store.judgeCache();
      expect(cache.dir).toBe(join(repo.store.dir, "judge"));
      await cache.put(entry({ request: REQUEST }));
      expect(await cache.list()).toHaveLength(1);
    } finally {
      await repo.cleanup();
    }
  });
});

describe("FakeJudge", () => {
  test("answers supports by sentence text and records the request", async () => {
    const answer = supportsAnswer({
      supports: 0.9,
      refutes: 0.05,
      does_not_address: 0.05,
    });
    const judge = new FakeJudge({ supports: { "Pro costs $29.": answer } });
    const input = {
      sentence: "Pro costs $29.",
      context: "",
      evidence: "price 29",
      source: "stripe",
    };
    const res = await judge.supports(input);
    expect(res.result.relation).toBe("supports");
    expect(res.result.confidence).toBeCloseTo((0.9 - 1 / 3) / (1 - 1 / 3), 10);
    expect(res.request).toEqual(
      buildSupportsRequest(DEFAULT_JUDGE_MODEL, input),
    );
    expect(mapSupportsAnswers(res.answers as never)).toEqual(res.result);
    expect(res.model).toBe(DEFAULT_JUDGE_MODEL);
    expect(judge.calls).toEqual([{ judgment: "supports", request: input }]);
    await expect(
      judge.supports({ ...input, sentence: "unscripted" }),
    ).rejects.toThrow("no scripted supports answer");
  });

  test("answers conflict by pair key in either order, or by function", async () => {
    const answer = conflictAnswer(
      { consistent: 0.1, contradictory: 0.85, unrelated: 0.05 },
      0.95,
    );
    const judge = new FakeJudge({
      conflict: { [pairKey("Pro is $29.", "Pro is $39.")]: answer },
    });
    const side = (text: string) => ({ text, doc: "a.md", context: "" });
    const res = await judge.conflict(side("Pro is $39."), side("Pro is $29."));
    expect(res.result.relation).toBe("contradictory");
    expect(res.result.sameSubject).toBe(0.95);

    const fn = new FakeJudge({ conflict: () => answer });
    expect((await fn.conflict(side("x"), side("y"))).result).toEqual(answer);
  });

  test("answers triage per region and fills regionId", async () => {
    const judge = new FakeJudge({
      triage: (region) => ({
        kind: region.kind === "heading" ? "narrative" : "factual_claim",
        kindProbabilities: {
          factual_claim: 0.7,
          instruction: 0.1,
          opinion: 0.1,
          narrative: 0.1,
        },
        kindConfidence: 0.6,
      }),
    });
    const res = await judge.triage({ path: "a.md" }, [
      { id: "r1", kind: "heading", text: "# A", headingPath: [] },
      { id: "r2", kind: "sentence", text: "A is 1.", headingPath: ["A"] },
    ]);
    expect(res.result.map((r) => [r.regionId, r.kind])).toEqual([
      ["r1", "narrative"],
      ["r2", "factual_claim"],
    ]);
  });

  test("throws JudgeUnavailable when told to, and recovers", async () => {
    const answer = supportsAnswer({
      supports: 0.9,
      refutes: 0.05,
      does_not_address: 0.05,
    });
    const judge = new FakeJudge({ supports: () => answer });
    judge.setUnavailable("no API key");
    const input = { sentence: "s", context: "", evidence: "e", source: "x" };
    const err = await judge.supports(input).catch((e) => e);
    expect(err).toBeInstanceOf(JudgeUnavailable);
    expect(err.message).toBe("no API key");
    expect(judge.calls).toEqual([]);
    judge.setUnavailable(null);
    expect((await judge.supports(input)).result).toEqual(answer);
  });
});
