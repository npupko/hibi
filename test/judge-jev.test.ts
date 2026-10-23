import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import {
  MAX_IN_FLIGHT,
  parseRetryAfter,
  type SystemOneRequest,
  type SystemOneResponse,
  TypeSafeHttp,
} from "../src/judge/http.ts";
import {
  ADDRESSES_FLOOR,
  buildConflictRequest,
  buildSupportsRequest,
  buildTriageRequests,
  chunkRegions,
  estimateTokens,
  isCodeByConstruction,
  JevJudge,
  mapConflictAnswers,
  mapSupportsAnswers,
  mapTriageAnswers,
  QUESTION_TOKEN_BUDGET,
  REQUEST_TOKEN_BUDGET,
  SAME_SUBJECT_FLOOR,
} from "../src/judge/jev.ts";
import {
  type ConflictSide,
  JudgeError,
  JudgeUnavailable,
  type Region,
  type SupportsInput,
} from "../src/judge/types.ts";

const FIXTURES = join(import.meta.dir, "fixtures", "judge");
const MODEL = "jev-1.13.0";
const KEY_ENV = { TYPESAFE_API_KEY: "test-key" };

interface Inputs {
  triage: { doc: { path: string }; regions: Region[] };
  supports: SupportsInput;
  conflict: { a: ConflictSide; b: ConflictSide };
}

async function fixture<T>(name: string): Promise<T> {
  return Bun.file(join(FIXTURES, name)).json();
}

interface Call {
  url: string;
  init: RequestInit;
  body: unknown;
}

type Reply = Response | Error | (() => Promise<Response>);

/** A fetch that answers from a queue and records every call. */
function fakeFetch(replies: Reply[]): { fetch: typeof fetch; calls: Call[] } {
  const calls: Call[] = [];
  const impl = async (input: unknown, init?: RequestInit) => {
    calls.push({
      url: String(input),
      init: init ?? {},
      body: JSON.parse(String(init?.body)),
    });
    const reply = replies.shift();
    if (reply === undefined) throw new Error("fake fetch: no reply queued");
    if (reply instanceof Error) throw reply;
    if (typeof reply === "function") return reply();
    return reply;
  };
  return { fetch: impl as unknown as typeof fetch, calls };
}

function json(body: unknown, status = 200, headers = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

function httpWith(
  replies: Reply[],
  env: Record<string, string | undefined> = KEY_ENV,
) {
  const { fetch, calls } = fakeFetch(replies);
  const sleeps: number[] = [];
  const http = new TypeSafeHttp({
    env,
    fetch,
    sleep: async (ms) => {
      sleeps.push(ms);
    },
    random: () => 0,
  });
  return { http, calls, sleeps };
}

/** A client that answers every request with fixed answers. */
function stubClient(answers: SystemOneResponse["answers"]) {
  const requests: SystemOneRequest[] = [];
  return {
    requests,
    client: {
      async systemOne(body: SystemOneRequest): Promise<SystemOneResponse> {
        requests.push(body);
        return {
          model: MODEL,
          answers,
          usage: { input_tokens: 1, output_tokens: 1 },
        };
      },
    },
  };
}

function region(id: string, text: string, kind = "sentence"): Region {
  return { id, kind: kind as Region["kind"], text, headingPath: ["Doc"] };
}

describe("request shapes match the fixtures", () => {
  test("triage", async () => {
    const inputs = await fixture<Inputs>("inputs.json");
    const expected = await fixture<SystemOneRequest>("triage.request.json");
    const response = await fixture<SystemOneResponse>("triage.response.json");
    const { http, calls } = httpWith([json(response)]);
    const judge = new JevJudge({ model: MODEL, http });

    const out = await judge.triage(inputs.triage.doc, inputs.triage.regions);

    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe("https://api.typesafe.ai/v1/systemone");
    expect(calls[0]?.body).toEqual(expected);
    const headers = calls[0]?.init.headers as Record<string, string>;
    expect(headers.authorization).toBe("Bearer test-key");
    expect(out.request).toEqual([expected]);
    expect(out.answers).toEqual([response.answers]);
    expect(out.model).toBe("jev-1.13.0");

    const byId = Object.fromEntries(out.result.map((r) => [r.regionId, r]));
    expect(byId.r0?.kind).toBe("narrative");
    expect(byId.r0?.evidenceKind).toBeUndefined();
    // r1 names a path: code by construction, no evidenceKind question asked.
    expect(byId.r1?.kind).toBe("factual_claim");
    expect(byId.r1?.evidenceKind).toBe("code");
    expect(byId.r1?.evidenceProbabilities).toEqual({
      code: 1,
      other_doc: 0,
      external: 0,
      nothing: 0,
    });
    expect(byId.r2?.evidenceKind).toBe("other_doc");
    expect(byId.r2?.evidenceProbabilities?.external).toBe(0.25);
    expect(byId.r3?.kind).toBe("instruction");
    expect(byId.r3?.evidenceKind).toBeUndefined();
    expect(byId.r4?.kind).toBe("opinion");
    expect(byId.r4?.evidenceKind).toBeUndefined();
    // (1 - 1/4) / (1 - 1/4)
    expect(byId.r1?.kindConfidence).toBeCloseTo(1, 4);
    // Triage asks kind and evidenceKind only; checkable was dropped.
    expect(
      Object.keys(expected.questions).filter((k) => k.startsWith("checkable")),
    ).toEqual([]);
    expect("checkable" in (byId.r1 ?? {})).toBe(false);
  });

  test("supports", async () => {
    const inputs = await fixture<Inputs>("inputs.json");
    const expected = await fixture<SystemOneRequest>("supports.request.json");
    const response = await fixture<SystemOneResponse>("supports.response.json");
    const { http, calls } = httpWith([json(response)]);
    const judge = new JevJudge({ model: MODEL, http });

    const out = await judge.supports(inputs.supports);

    expect(calls[0]?.body).toEqual(expected);
    expect(out.request).toEqual(expected);
    expect(out.answers).toEqual(response.answers);
    expect(out.model).toBe("jev-1.13.0");
    expect(out.result).toEqual({
      relation: "supports",
      probabilities: { supports: 0.99, refutes: 0, does_not_address: 0.01 },
      confidence: expect.closeTo(0.985, 4),
      addresses: 0.97,
    });
  });

  test("conflict", async () => {
    const inputs = await fixture<Inputs>("inputs.json");
    const expected = await fixture<SystemOneRequest>("conflict.request.json");
    const response = await fixture<SystemOneResponse>("conflict.response.json");
    const { http, calls } = httpWith([json(response)]);
    const judge = new JevJudge({ model: MODEL, http });

    const out = await judge.conflict(inputs.conflict.a, inputs.conflict.b);

    expect(calls[0]?.body).toEqual(expected);
    expect(out.request).toEqual(expected);
    expect(out.answers).toEqual(response.answers);
    expect(out.result).toEqual({
      relation: "contradictory",
      probabilities: { consistent: 0.05, contradictory: 0.95, unrelated: 0 },
      confidence: expect.closeTo(0.925, 4),
      sameSubject: 0.74,
      contradicts: 0.65,
    });
  });
});

describe("code rules", () => {
  const supportsInput = {
    sentence: "s",
    context: "c",
    evidence: "e",
    source: "x",
  };
  const supportsAnswers = (addresses: number) => ({
    relation: {
      choice: "supports",
      confidence: 0.85,
      probabilities: { supports: 0.9, contradicts: 0.05, says_nothing: 0.05 },
    },
    addresses: { noul: addresses },
  });

  test(`supports: addresses below ${ADDRESSES_FLOOR} forces does_not_address`, async () => {
    const { client } = stubClient(supportsAnswers(0.29));
    const out = await new JevJudge({ model: MODEL, http: client }).supports(
      supportsInput,
    );
    expect(out.result.relation).toBe("does_not_address");
    expect(out.result.forced).toBe("addresses");
    // Noul confidence of addresses: |2 * 0.29 - 1|
    expect(out.result.confidence).toBeCloseTo(0.42, 10);
    expect(out.result.probabilities).toEqual({
      supports: 0.9,
      refutes: 0.05,
      does_not_address: 0.05,
    });
    expect(out.result.addresses).toBe(0.29);
  });

  test("supports: addresses at the floor keeps the Choice", async () => {
    const { client } = stubClient(supportsAnswers(ADDRESSES_FLOOR));
    const out = await new JevJudge({ model: MODEL, http: client }).supports(
      supportsInput,
    );
    expect(out.result.relation).toBe("supports");
    expect(out.result.forced).toBeUndefined();
    // Choice confidence: (0.9 - 1/3) / (1 - 1/3)
    expect(out.result.confidence).toBeCloseTo(0.85, 10);
  });

  const side = { text: "t", doc: "d.md", context: "c" };
  const conflictAnswers = (sameSubject: number) => ({
    same_subject: { noul: sameSubject },
    relation: {
      choice: "contradictory",
      confidence: 0.85,
      probabilities: { consistent: 0.05, contradictory: 0.9, unrelated: 0.05 },
    },
    contradicts: { noul: 0.8 },
  });

  test(`conflict: same_subject below ${SAME_SUBJECT_FLOOR} forces unrelated`, async () => {
    const { client } = stubClient(conflictAnswers(0.1));
    const out = await new JevJudge({ model: MODEL, http: client }).conflict(
      side,
      side,
    );
    expect(out.result.relation).toBe("unrelated");
    expect(out.result.forced).toBe("same_subject");
    // Noul confidence of same_subject: |2 * 0.1 - 1|
    expect(out.result.confidence).toBeCloseTo(0.8, 10);
    expect(out.result.probabilities.contradictory).toBe(0.9);
    expect(out.result.sameSubject).toBe(0.1);
  });

  test("conflict: same_subject at the floor keeps the Choice", async () => {
    const { client } = stubClient(conflictAnswers(SAME_SUBJECT_FLOOR));
    const out = await new JevJudge({ model: MODEL, http: client }).conflict(
      side,
      side,
    );
    expect(out.result.relation).toBe("contradictory");
    expect(out.result.forced).toBeUndefined();
    expect(out.result.confidence).toBeCloseTo(0.85, 10);
  });
});

describe("pure builders and mappers", () => {
  test("builders return the fixture bodies without a network call", async () => {
    const inputs = await fixture<Inputs>("inputs.json");
    expect(
      buildTriageRequests(MODEL, inputs.triage.doc, inputs.triage.regions),
    ).toEqual([await fixture<SystemOneRequest>("triage.request.json")]);
    expect(buildSupportsRequest(MODEL, inputs.supports)).toEqual(
      await fixture<SystemOneRequest>("supports.request.json"),
    );
    expect(
      buildConflictRequest(MODEL, inputs.conflict.a, inputs.conflict.b),
    ).toEqual(await fixture<SystemOneRequest>("conflict.request.json"));
    expect(buildTriageRequests(MODEL, { path: "empty.md" }, [])).toEqual([]);
  });

  test("mappers turn cached answers into the same results the judge returns", async () => {
    const inputs = await fixture<Inputs>("inputs.json");
    for (const name of ["triage", "supports", "conflict"] as const) {
      const response = await fixture<SystemOneResponse>(
        `${name}.response.json`,
      );
      const { client } = stubClient(response.answers);
      const judge = new JevJudge({ model: MODEL, http: client });
      if (name === "triage") {
        const requests = buildTriageRequests(
          MODEL,
          inputs.triage.doc,
          inputs.triage.regions,
        );
        const live = await judge.triage(
          inputs.triage.doc,
          inputs.triage.regions,
        );
        expect(mapTriageAnswers(requests, [response.answers])).toEqual(
          live.result,
        );
      } else if (name === "supports") {
        const live = await judge.supports(inputs.supports);
        expect(mapSupportsAnswers(response.answers)).toEqual(live.result);
      } else {
        const live = await judge.conflict(inputs.conflict.a, inputs.conflict.b);
        expect(mapConflictAnswers(response.answers)).toEqual(live.result);
      }
    }
  });
});

describe("code by construction", () => {
  const cases: Array<[string, boolean, Region["kind"]?]> = [
    ["hibi stores its cache in .hibi/cache.json.", true],
    ["Settings live in hibi.toml.", true],
    ["Copy the .env file first.", true],
    ["Run `hibi check --doc README.md` first.", true],
    ["The `src/judge` folder holds the adapter.", true],
    ["Pass `--threshold` to change it.", true],
    ["const x = 1;", true, "fenced"],
    ["Use ```ts blocks``` for code.", true],
    ["The `Judge` interface has three methods.", false],
    ["Bun supports Node.js streams and/or Web streams.", false],
    ["See https://docs.typesafe.ai/api.md for details.", false],
    ["Refunds are processed within five business days, e.g. on Monday.", false],
  ];
  for (const [text, expected, kind] of cases) {
    test(`${expected ? "code" : "not code"}: ${text}`, () => {
      expect(isCodeByConstruction(region("r", text, kind))).toBe(expected);
    });
  }
});

describe("chunking", () => {
  const cost = (state: number, questions: number, largestQuestion = 0) => ({
    state,
    questions,
    largestQuestion,
  });
  const budgets = { request: 100, question: 60 };

  test("packs consecutive regions under the request budget", () => {
    const costs = [cost(5, 25), cost(5, 25), cost(5, 25), cost(5, 45)];
    // fixed 10: 10 + 15 + 75 = 100 fits; the fourth region starts a new chunk.
    expect(chunkRegions(costs, 10, budgets)).toEqual([[0, 1, 2], [3]]);
    expect(chunkRegions([], 10, budgets)).toEqual([]);
  });

  test("closes a chunk when its state plus one question would pass the question budget", () => {
    const costs = [cost(20, 10, 10), cost(20, 10, 10), cost(20, 10, 10)];
    // fixed 10: 10 + 40 + 10 = 60 fits; a third region makes 70.
    expect(chunkRegions(costs, 10, budgets)).toEqual([[0, 1], [2]]);
  });

  test("throws when one region cannot fit in a request on its own", () => {
    expect(() => chunkRegions([cost(10, 95)], 10, budgets)).toThrow(
      "too large",
    );
    expect(() => chunkRegions([cost(45, 10, 10)], 10, budgets)).toThrow(
      "too large",
    );
  });

  test("a long document is split; each request holds doc plus only its regions", async () => {
    const regions = Array.from({ length: 400 }, (_, i) =>
      region(`r${i}`, `Sentence number ${i} states a fact about the system.`),
    );
    const answers: SystemOneResponse["answers"] = {};
    const kindAnswer = {
      choice: "narrative",
      confidence: 1,
      probabilities: {
        factual_claim: 0,
        instruction: 0,
        opinion: 0,
        narrative: 1,
      },
    };
    for (let i = 0; i < regions.length; i++) {
      answers[`kind_${i}`] = kindAnswer;
    }
    const { client, requests } = stubClient(answers);
    const out = await new JevJudge({ model: MODEL, http: client }).triage(
      { path: "big.md" },
      regions,
    );

    expect(requests.length).toBeGreaterThan(1);
    const seen: string[] = [];
    for (const request of requests) {
      const state = request.state as { doc: unknown; regions: Region[] };
      expect(state.doc).toEqual({ path: "big.md" });
      seen.push(...state.regions.map((r) => r.id));
      // Question ids and paths use the index inside this request.
      expect(Object.keys(request.questions)).toHaveLength(
        state.regions.length * 2,
      );
      expect(request.questions[`kind_${state.regions.length}`]).toBeUndefined();
      expect(estimateTokens(request)).toBeLessThanOrEqual(REQUEST_TOKEN_BUDGET);
      const largest = Math.max(
        ...Object.values(request.questions).map(estimateTokens),
      );
      expect(estimateTokens(request.state) + largest).toBeLessThanOrEqual(
        QUESTION_TOKEN_BUDGET,
      );
    }
    expect(seen).toEqual(regions.map((r) => r.id));
    expect(out.result.map((r) => r.regionId)).toEqual(seen);
    expect(out.request).toEqual(requests);
  });
});

describe("http client", () => {
  const body: SystemOneRequest = { model: MODEL, state: "s", questions: {} };
  const ok = {
    model: MODEL,
    answers: {},
    usage: { input_tokens: 1, output_tokens: 0 },
  };

  test("retries a 429 after the retry-after delay", async () => {
    const { http, calls, sleeps } = httpWith([
      json({ detail: "slow down" }, 429, { "retry-after": "2" }),
      json(ok),
    ]);
    expect(await http.systemOne(body)).toEqual(ok);
    expect(calls).toHaveLength(2);
    expect(sleeps).toEqual([2000]);
  });

  test("a retry-after over 60 s is clamped to 60 s", async () => {
    const { http, sleeps } = httpWith([
      json({}, 529, { "retry-after": "120" }),
      json({}, 503),
      json(ok),
    ]);
    await http.systemOne(body);
    expect(sleeps).toEqual([60_000, 1000]);
  });

  test("missing or blank key throws JudgeUnavailable before any request", async () => {
    for (const env of [{}, { TYPESAFE_API_KEY: "  " }]) {
      const { http, calls } = httpWith([json(ok)], env);
      await expect(http.systemOne(body)).rejects.toBeInstanceOf(
        JudgeUnavailable,
      );
      expect(calls).toHaveLength(0);
    }
  });

  test("401 throws JudgeUnavailable naming the env var", async () => {
    const { http, calls } = httpWith([json({ detail: "bad key" }, 401)]);
    const error = await http.systemOne(body).catch((e) => e);
    expect(error).toBeInstanceOf(JudgeUnavailable);
    expect(error.message).toContain("TYPESAFE_API_KEY");
    expect(calls).toHaveLength(1);
  });

  test("exhausted retries throw JudgeUnavailable", async () => {
    const { http, calls } = httpWith([
      json({}, 503),
      json({}, 503),
      json({}, 503),
    ]);
    await expect(http.systemOne(body)).rejects.toBeInstanceOf(JudgeUnavailable);
    expect(calls).toHaveLength(3);
  });

  test("connection errors retry, then throw JudgeUnavailable", async () => {
    const { http, calls } = httpWith([
      new TypeError("connection refused"),
      new TypeError("connection refused"),
      new TypeError("connection refused"),
    ]);
    const error = await http.systemOne(body).catch((e) => e);
    expect(error).toBeInstanceOf(JudgeUnavailable);
    expect(error.message).toContain("connection refused");
    expect(calls).toHaveLength(3);
  });

  test("400 and 422 throw JudgeError with the body and do not retry", async () => {
    for (const status of [400, 404, 422]) {
      const { http, calls } = httpWith([
        json({ error_type: "max_tokens_exceeded" }, status),
      ]);
      const error = await http.systemOne(body).catch((e) => e);
      expect(error).toBeInstanceOf(JudgeError);
      expect(error).not.toBeInstanceOf(JudgeUnavailable);
      expect(error.message).toBe(
        `TypeSafe returned HTTP ${status}: {"error_type":"max_tokens_exceeded"}`,
      );
      expect(calls).toHaveLength(1);
    }
  });

  test("403 is JudgeUnavailable, not JudgeError", async () => {
    const { http } = httpWith([json({}, 403)]);
    const error = await http.systemOne(body).catch((e) => e);
    expect(error).toBeInstanceOf(JudgeUnavailable);
  });

  test("a 200 body that is not JSON throws JudgeError", async () => {
    const { http, calls } = httpWith([
      new Response("<html>gateway</html>", { status: 200 }),
    ]);
    const error = await http.systemOne(body).catch((e) => e);
    expect(error).toBeInstanceOf(JudgeError);
    expect(error.message).toContain(
      "TypeSafe returned a body that is not JSON",
    );
    expect(error.message).toContain("<html>gateway</html>");
    expect(calls).toHaveLength(1);
  });

  test("a 200 body without an answers object and a model string throws JudgeError", async () => {
    for (const bad of [
      { model: MODEL },
      { answers: {} },
      { model: MODEL, answers: [] },
      { model: 1, answers: {} },
      [],
      null,
    ]) {
      const { http } = httpWith([json(bad)]);
      const error = await http.systemOne(body).catch((e) => e);
      expect(error).toBeInstanceOf(JudgeError);
      expect(error.message).toContain(
        'TypeSafe returned JSON without an "answers" object and a "model" string',
      );
    }
  });

  test("an answer the mappers cannot read throws JudgeError from the judge", async () => {
    const { http } = httpWith([json({ ...ok, answers: { relation: {} } })]);
    const judge = new JevJudge({ model: MODEL, http });
    const error = await judge
      .supports({ sentence: "s", context: "c", evidence: "e", source: "x" })
      .catch((e) => e);
    expect(error).toBeInstanceOf(JudgeError);
  });

  test("parseRetryAfter clamps to 60 s and reads HTTP dates", () => {
    expect(parseRetryAfter("2")).toBe(2000);
    expect(parseRetryAfter("120")).toBe(60_000);
    expect(parseRetryAfter("-5")).toBe(0);
    expect(parseRetryAfter("soon")).toBeNull();
    expect(parseRetryAfter(null)).toBeNull();
    const now = Date.parse("2026-10-22T00:00:00Z");
    expect(parseRetryAfter("Thu, 22 Oct 2026 00:10:00 GMT", now)).toBe(60_000);
    expect(parseRetryAfter("Thu, 22 Oct 2026 00:00:30 GMT", now)).toBe(30_000);
  });

  test("TYPESAFE_BASE_URL overrides the endpoint", async () => {
    const { fetch, calls } = fakeFetch([json(ok)]);
    const http = new TypeSafeHttp({
      env: { ...KEY_ENV, TYPESAFE_BASE_URL: "http://localhost:9/" },
      fetch,
    });
    await http.systemOne(body);
    expect(calls[0]?.url).toBe("http://localhost:9/v1/systemone");
  });

  test(`at most ${MAX_IN_FLIGHT} requests are in flight`, async () => {
    let inFlight = 0;
    let peak = 0;
    const slow = async () => {
      inFlight++;
      peak = Math.max(peak, inFlight);
      await new Promise((resolve) => setTimeout(resolve, 5));
      inFlight--;
      return json(ok);
    };
    const { http } = httpWith(Array.from({ length: 20 }, () => slow));
    await Promise.all(Array.from({ length: 20 }, () => http.systemOne(body)));
    expect(peak).toBe(MAX_IN_FLIGHT);
  });
});

describe("live", () => {
  test.skipIf(!process.env.TYPESAFE_API_KEY?.trim())(
    "supports on an obvious pair",
    async () => {
      const judge = new JevJudge({ model: MODEL, http: new TypeSafeHttp() });
      const out = await judge.supports({
        sentence: "The default port is 8080.",
        context: "Server > Configuration",
        evidence: "const DEFAULT_PORT = 8080; // used when PORT is unset",
        source: "src/server/config.ts:3-3",
      });
      expect(out.result.relation).toBe("supports");
    },
    30_000,
  );
});
