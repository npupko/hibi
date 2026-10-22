import { afterEach, describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { evidenceNext } from "../src/cli/render/evidence.ts";
import type { Claim, Config } from "../src/core/model.ts";
import { runCheck } from "../src/engine/check.ts";
import {
  type EvidenceItem,
  type EvidenceReport,
  type EvidenceResult,
  EvidenceValidationError,
  MAX_EVIDENCE_CHARS,
  type SubmitEvidenceInput,
  submitEvidence,
} from "../src/engine/evidence.ts";
import { cacheKey } from "../src/judge/cache.ts";
import { FakeJudge, supportsAnswer } from "../src/judge/fake.ts";
import { buildSupportsRequest } from "../src/judge/jev.ts";
import { noulConfidence, type SupportsResult } from "../src/judge/types.ts";
import { makeRepo, type TempRepo } from "./helpers.ts";

const NOW = new Date("2026-10-22T12:00:00.000Z");
const LATER = new Date("2026-10-23T08:30:00.000Z");
const CONFIG: Config = {
  version: "v4",
  repos: {},
  judge: { model: "jev-1.14.0", threshold: 0.8 },
  ignore: [],
};

const PRICE = "Pro costs $29/month.";
const SEATS = "Pro includes 5 seats.";
const ID_PRICE = "clm_00000000000000a1";
const ID_SEATS = "clm_00000000000000b2";
const ID_RETIRED = "clm_00000000000000c3";
const ID_CODE = "clm_00000000000000d4";
const ID_REF = "clm_00000000000000e5";

function attested(id: string, sentence: string, over: Partial<Claim> = {}) {
  return {
    id,
    documentId: "doc_0000000000000001",
    kind: "attested",
    sentence,
    doc: {
      file: "docs/pricing.md",
      selectors: [
        { kind: "text-quote", exact: sentence, prefix: "", suffix: "" },
      ],
    },
    evidence: [],
    sources: [],
    procedure: null,
    ttl: "P90D",
    enforcement: "enforced",
    attestations: [],
    createdAt: "2026-10-01T00:00:00.000Z",
    ref: "r",
    ...over,
  } satisfies Claim;
}

const SUPPORTED = supportsAnswer({
  supports: 0.96,
  refutes: 0.02,
  does_not_address: 0.02,
});
const REFUTED = supportsAnswer({
  supports: 0.02,
  refutes: 0.95,
  does_not_address: 0.03,
});

function item(over: Partial<EvidenceItem> = {}): EvidenceItem {
  return {
    claimId: ID_PRICE,
    evidence: "Pro plan, monthly: USD 29.00",
    source: "stripe prices list --product prod_123",
    summary: "Stripe lists Pro at $29/month",
    by: "agent:opus",
    ...over,
  };
}

let repos: TempRepo[] = [];
afterEach(async () => {
  await Promise.all(repos.map((r) => r.cleanup()));
  repos = [];
});

async function setup(): Promise<TempRepo> {
  const repo = await makeRepo();
  repos.push(repo);
  await repo.store.putClaim(attested(ID_PRICE, PRICE));
  await repo.store.putClaim(attested(ID_SEATS, SEATS));
  await repo.store.putClaim(
    attested(ID_RETIRED, "Old plan.", { enforcement: "retired" }),
  );
  const span = (file: string) => ({
    file,
    selectors: [
      { kind: "text-quote" as const, exact: "x = 1", prefix: "", suffix: "" },
    ],
  });
  await repo.store.putClaim({
    ...attested(ID_CODE, "Retries are capped at 5."),
    kind: "code",
    ttl: undefined,
    evidence: [span("src/retry.ts")],
  });
  await repo.store.putClaim({
    ...attested(ID_REF, "Refunds take two weeks."),
    kind: "reference",
    ttl: undefined,
    evidence: [span("docs/billing.md")],
  });
  return repo;
}

function submit(
  repo: TempRepo,
  judge: FakeJudge,
  items: unknown[],
  over: Partial<SubmitEvidenceInput> = {},
) {
  return submitEvidence({
    store: repo.store,
    judge,
    items: items as EvidenceItem[],
    context: (c) => `context of ${c.id}`,
    config: CONFIG,
    now: NOW,
    ...over,
  });
}

/** Every file under .claims, path to content. */
async function snapshot(repo: TempRepo): Promise<Map<string, string>> {
  const out = new Map<string, string>();
  const walk = async (d: string) => {
    for (const e of await readdir(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) await walk(p);
      else out.set(p, await readFile(p, "utf8"));
    }
  };
  await walk(repo.store.dir);
  return out;
}

function sha256(s: string): string {
  return createHash("sha256").update(s).digest("hex");
}

describe("submitEvidence validation", () => {
  const cases: [string, unknown, string][] = [
    [
      "unknown claim",
      item({ claimId: "clm_ffffffffffffffff" }),
      "no claim clm_ffffffffffffffff",
    ],
    ["retired claim", item({ claimId: ID_RETIRED }), "is retired"],
    [
      "code claim",
      item({ claimId: ID_CODE }),
      `claim ${ID_CODE} is a code claim; evidence applies to attested claims, and reference claims are judged from the referenced document`,
    ],
    [
      "reference claim",
      item({ claimId: ID_REF }),
      `claim ${ID_REF} is a reference claim; evidence applies to attested claims, and reference claims are judged from the referenced document`,
    ],
    ["empty evidence", item({ evidence: "  " }), '"evidence" is empty'],
    [
      "evidence over the limit",
      item({ evidence: "x".repeat(MAX_EVIDENCE_CHARS + 1) }),
      "Trim it to the part that bears on the sentence, or summarize it",
    ],
    ["empty source", item({ source: "" }), '"source" is empty'],
    ["empty summary", item({ summary: " " }), '"summary" is empty'],
    ["bad actor", item({ by: "opus" }), '"by" must be human:<id>'],
    ["missing field", { ...item(), by: undefined }, '"by" must be a string'],
    [
      "non-string field",
      { ...item(), evidence: 29 },
      '"evidence" must be a string',
    ],
    ["unknown key", { ...item(), sumary: "x" }, 'unknown key "sumary"'],
    ["not an object", "clm_00000000000000a1", "is not an object"],
  ];
  for (const [name, bad, message] of cases) {
    test(`${name} rejects the batch before any judge call`, async () => {
      const repo = await setup();
      const judge = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
      const before = await snapshot(repo);
      const err = await submit(repo, judge, [bad]).catch((e) => e);
      expect(err).toBeInstanceOf(EvidenceValidationError);
      expect((err as Error).message).toContain(message);
      expect(judge.calls).toHaveLength(0);
      expect(await snapshot(repo)).toEqual(before);
    });
  }

  test("evidence exactly at the limit is accepted", async () => {
    const repo = await setup();
    const judge = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
    const report = await submit(repo, judge, [
      item({ evidence: "x".repeat(MAX_EVIDENCE_CHARS) }),
    ]);
    expect(report.results[0]?.judge).toBe("supported");
  });

  test("an empty batch is rejected", async () => {
    const repo = await setup();
    await expect(submit(repo, new FakeJudge(), [])).rejects.toThrow(
      "the batch has no items",
    );
  });

  test("one bad item rejects the whole batch; every bad item is named", async () => {
    const repo = await setup();
    const judge = new FakeJudge({
      supports: { [PRICE]: SUPPORTED, [SEATS]: SUPPORTED },
    });
    const before = await snapshot(repo);
    const err = (await submit(repo, judge, [
      item(),
      item({ claimId: ID_SEATS, summary: "" }),
      item({ by: "nobody" }),
    ]).catch((e) => e)) as EvidenceValidationError;
    expect(err).toBeInstanceOf(EvidenceValidationError);
    expect(err.errors).toEqual([
      `item 1 (${ID_SEATS}): "summary" is empty`,
      `item 2 (${ID_PRICE}): "by" must be human:<id>, agent:<name>, or process:<name> (got "nobody")`,
    ]);
    expect(judge.calls).toHaveLength(0);
    expect(await snapshot(repo)).toEqual(before);
  });

  test("a judge error other than unavailable aborts the batch before any write", async () => {
    const repo = await setup();
    // No scripted answer for SEATS: FakeJudge throws a plain Error.
    const judge = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
    const before = await snapshot(repo);
    await expect(
      submit(repo, judge, [item(), item({ claimId: ID_SEATS })]),
    ).rejects.toThrow("no scripted supports answer");
    expect(await snapshot(repo)).toEqual(before);
  });
});

describe("submitEvidence outcomes", () => {
  test("supported appends an evidence attestation and caches the verdict with an evidence block", async () => {
    const repo = await setup();
    const judge = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
    const it = item();
    const report = await submit(repo, judge, [it]);

    const hash = sha256(it.evidence);
    expect(report.results).toEqual([
      {
        claimId: ID_PRICE,
        judge: "supported",
        relation: "supports",
        confidence: SUPPORTED.confidence,
        probabilities: SUPPORTED.probabilities,
        reasons: [
          "evidence supports the sentence (confidence 0.94 >= threshold 0.8)",
        ],
        attested: true,
        cached: false,
      },
    ]);
    expect(report.summary).toEqual({
      supported: 1,
      refuted: 0,
      review: 0,
      pending: 0,
      requests: 1,
    });

    const claim = await repo.store.getClaim(ID_PRICE);
    expect(claim?.attestations).toEqual([
      {
        by: "agent:opus",
        at: NOW.toISOString(),
        basis: "evidence",
        note: it.summary,
        evidenceHash: hash,
      },
    ]);

    // The judge was asked with the claim's sentence and injected context.
    expect(judge.calls[0]?.request).toEqual({
      sentence: PRICE,
      context: `context of ${ID_PRICE}`,
      evidence: it.evidence,
      source: it.source,
    });

    const sent = buildSupportsRequest(CONFIG.judge.model, {
      sentence: PRICE,
      context: `context of ${ID_PRICE}`,
      evidence: it.evidence,
      source: it.source,
    });
    const key = cacheKey(CONFIG.judge.model, sent);
    const entry = await repo.store.judgeCache().get(key);
    expect(entry).toBeDefined();
    expect(entry?.judgment).toBe("supports");
    expect(entry?.subject).toEqual({ claimId: ID_PRICE });
    expect(entry?.model).toBe(judge.model);
    expect(entry?.evidence).toEqual({
      hash,
      source: it.source,
      summary: it.summary,
      by: "agent:opus",
      at: NOW.toISOString(),
      values: [{ kind: "money", normalized: "29", unit: "USD" }],
    });
    expect(
      (entry?.request as { state: Record<string, unknown> }).state,
    ).toEqual({
      claim: PRICE,
      claim_context: `context of ${ID_PRICE}`,
      evidence: `sha256:${hash}`,
      evidence_source: it.source,
    });

    // Raw evidence text is nowhere in the store.
    for (const [path, content] of await snapshot(repo)) {
      expect(content.includes(it.evidence), path).toBe(false);
    }
  });

  test("refuted at or above threshold appends nothing and gates", async () => {
    const repo = await setup();
    const judge = new FakeJudge({ supports: { [PRICE]: REFUTED } });
    const report = await submit(repo, judge, [item()]);
    expect(report.results[0]).toMatchObject({
      judge: "refuted",
      relation: "refutes",
      attested: false,
      reasons: [
        "evidence refutes the sentence (confidence 0.92 >= threshold 0.8)",
      ],
    });
    expect(report.summary.refuted).toBe(1);
    expect((await repo.store.getClaim(ID_PRICE))?.attestations).toEqual([]);
    expect(await repo.store.judgeCache().list()).toHaveLength(1);
  });

  test("value rule: evidence with a different price is refuted with one label, and check agrees", async () => {
    const repo = await setup();
    const judge = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
    const report = await submit(repo, judge, [
      item({ evidence: "Pro plan, monthly: USD 39.00" }),
    ]);
    expect(report.results[0]).toMatchObject({
      judge: "refuted",
      relation: "supports",
      attested: false,
      reasons: ["value differs: numbers differ: 29 vs 39"],
    });
    expect((await repo.store.getClaim(ID_PRICE))?.attestations).toEqual([]);
    // check has no evidence text; it applies the rule to the stored values.
    const check = await runCheck(repo.store, { now: NOW });
    const v = check.verdicts.find((x) => x.claimId === ID_PRICE);
    expect(v?.judge).toBe("refuted");
    expect(v?.gates).toBe(true);
    expect(v?.reasons).toContain(
      "judge: value differs: numbers differ: 29 vs 39",
    );
  });

  test("a source matching a declared source adds a label and changes nothing else", async () => {
    const repo = await setup();
    await repo.store.putClaim(
      attested(ID_PRICE, PRICE, {
        sources: [{ id: "stripe", resource: "https://stripe.example/p/1" }],
      }),
    );
    const judge = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
    const report = await submit(repo, judge, [
      item({ source: "https://stripe.example/p/1" }),
    ]);
    expect(report.results[0]?.judge).toBe("supported");
    expect(report.results[0]?.reasons).toEqual([
      "evidence supports the sentence (confidence 0.94 >= threshold 0.8)",
      'source matches declared source "https://stripe.example/p/1"',
    ]);
  });

  test("supports or refutes below threshold is review and appends nothing", async () => {
    const repo = await setup();
    const leanSupports = supportsAnswer({
      supports: 0.7,
      refutes: 0.2,
      does_not_address: 0.1,
    });
    const leanRefutes = supportsAnswer({
      supports: 0.2,
      refutes: 0.7,
      does_not_address: 0.1,
    });
    const judge = new FakeJudge({
      supports: { [PRICE]: leanSupports, [SEATS]: leanRefutes },
    });
    const report = await submit(repo, judge, [
      item(),
      item({ claimId: ID_SEATS }),
    ]);
    expect(report.results.map((r) => [r.judge, r.relation, r.reasons])).toEqual(
      [
        [
          "review",
          "supports",
          ["evidence leans supports, confidence 0.55 below threshold 0.8"],
        ],
        [
          "review",
          "refutes",
          ["evidence leans refutes, confidence 0.55 below threshold 0.8"],
        ],
      ],
    );
    expect(report.summary.review).toBe(2);
    for (const id of [ID_PRICE, ID_SEATS]) {
      expect((await repo.store.getClaim(id))?.attestations).toEqual([]);
    }
  });

  test("the threshold comes from config.judge.threshold", async () => {
    const repo = await setup();
    const lean = supportsAnswer({
      supports: 0.7,
      refutes: 0.2,
      does_not_address: 0.1,
    });
    const judge = new FakeJudge({ supports: { [PRICE]: lean } });
    const report = await submit(repo, judge, [item()], {
      config: { ...CONFIG, judge: { ...CONFIG.judge, threshold: 0.5 } },
    });
    expect(report.results[0]?.judge).toBe("supported");
    expect(report.results[0]?.attested).toBe(true);
  });

  test("does_not_address is review with a reason label and appends nothing", async () => {
    const repo = await setup();
    const choice = supportsAnswer({
      supports: 0.05,
      refutes: 0.05,
      does_not_address: 0.9,
    });
    const forced: SupportsResult = {
      relation: "does_not_address",
      probabilities: { supports: 0.9, refutes: 0.05, does_not_address: 0.05 },
      confidence: noulConfidence(0.12),
      addresses: 0.12,
      forced: "addresses",
    };
    const judge = new FakeJudge({
      supports: { [PRICE]: choice, [SEATS]: forced },
    });
    const report = await submit(repo, judge, [
      item(),
      item({ claimId: ID_SEATS }),
    ]);
    expect(report.results.map((r) => [r.judge, r.relation, r.reasons])).toEqual(
      [
        [
          "review",
          "does_not_address",
          ["evidence did not address the sentence"],
        ],
        [
          "review",
          "does_not_address",
          ["evidence did not address the sentence (addresses=0.12)"],
        ],
      ],
    );
    for (const id of [ID_PRICE, ID_SEATS]) {
      expect((await repo.store.getClaim(id))?.attestations).toEqual([]);
    }
  });

  test("a cache hit asks nothing, updates the evidence block, and attests again", async () => {
    const repo = await setup();
    const first = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
    await submit(repo, first, [item()]);

    // An unscripted judge throws if asked.
    const second = new FakeJudge();
    const report = await submit(
      repo,
      second,
      [item({ summary: "re-checked Stripe", by: "agent:sonnet" })],
      { now: LATER },
    );
    expect(second.calls).toHaveLength(0);
    expect(report.results[0]).toMatchObject({
      judge: "supported",
      cached: true,
      attested: true,
    });
    expect(report.summary.requests).toBe(0);

    const entries = await repo.store.judgeCache().list({ claimId: ID_PRICE });
    expect(entries).toHaveLength(1);
    expect(entries[0]?.at).toBe(NOW.toISOString());
    expect(entries[0]?.evidence).toMatchObject({
      summary: "re-checked Stripe",
      by: "agent:sonnet",
      at: LATER.toISOString(),
    });
    const claim = await repo.store.getClaim(ID_PRICE);
    expect(claim?.attestations.map((a) => [a.by, a.at])).toEqual([
      ["agent:opus", NOW.toISOString()],
      ["agent:sonnet", LATER.toISOString()],
    ]);
  });

  test("a different context is a different request and asks again", async () => {
    const repo = await setup();
    const judge = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
    await submit(repo, judge, [item()]);
    await submit(repo, judge, [item()], { context: () => "edited paragraph" });
    expect(judge.calls).toHaveLength(2);
  });

  test("judge unavailable is pending and writes nothing", async () => {
    const repo = await setup();
    const judge = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
    judge.setUnavailable("TYPESAFE_API_KEY is not set");
    const before = await snapshot(repo);
    const report = await submit(repo, judge, [item()]);
    expect(report.results).toEqual([
      {
        claimId: ID_PRICE,
        judge: "pending",
        relation: null,
        confidence: null,
        probabilities: null,
        reasons: ["judge unavailable: TYPESAFE_API_KEY is not set"],
        attested: false,
        cached: false,
      },
    ]);
    expect(report.summary).toEqual({
      supported: 0,
      refuted: 0,
      review: 0,
      pending: 1,
      requests: 0,
    });
    expect(await snapshot(repo)).toEqual(before);
  });

  test("a judge error is pending with its label, writes nothing, and the batch goes on", async () => {
    const repo = await setup();
    const judge = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
    judge.setError("TypeSafe returned HTTP 400: bad");
    const before = await snapshot(repo);
    const report = await submit(repo, judge, [item()]);
    expect(report.results[0]).toMatchObject({
      judge: "pending",
      relation: null,
      reasons: ["judge error: TypeSafe returned HTTP 400: bad"],
      attested: false,
    });
    expect(report.summary.pending).toBe(1);
    expect(await snapshot(repo)).toEqual(before);
  });

  test("the cache key uses the judge's model", async () => {
    const repo = await setup();
    const judge = new FakeJudge(
      { supports: { [PRICE]: SUPPORTED } },
      "jev-9.9.9",
    );
    await submit(repo, judge, [item()]);
    const [entry] = await repo.store.judgeCache().list();
    const request = buildSupportsRequest("jev-9.9.9", {
      sentence: PRICE,
      context: `context of ${ID_PRICE}`,
      evidence: item().evidence,
      source: item().source,
    });
    expect(entry?.key).toBe(cacheKey("jev-9.9.9", request));
  });

  test("two supported items on one claim append two attestations", async () => {
    const repo = await setup();
    const judge = new FakeJudge({ supports: { [PRICE]: SUPPORTED } });
    await submit(repo, judge, [
      item(),
      item({ evidence: "Pricing page: Pro $29", source: "pricing page" }),
    ]);
    const claim = await repo.store.getClaim(ID_PRICE);
    expect(claim?.attestations.map((a) => a.evidenceHash)).toEqual([
      sha256("Pro plan, monthly: USD 29.00"),
      sha256("Pricing page: Pro $29"),
    ]);
  });
});

describe("the next hint", () => {
  const result = (judge: EvidenceResult["judge"], reasons: string[]) =>
    ({
      claimId: "clm_0000000000000001",
      judge,
      relation: null,
      confidence: null,
      probabilities: null,
      reasons,
      attested: false,
      cached: false,
    }) satisfies EvidenceResult;
  const report = (results: EvidenceResult[]): EvidenceReport => ({
    results,
    summary: {
      supported: 0,
      refuted: 0,
      review: 0,
      pending: results.filter((r) => r.judge === "pending").length,
      requests: 0,
    },
  });

  test("nothing pending points at check", () => {
    expect(evidenceNext(report([result("supported", [])]))).toBe("hibi check");
  });

  test("an unavailable judge names the key and the network", () => {
    expect(
      evidenceNext(report([result("pending", ["judge unavailable: no key"])])),
    ).toBe(
      "set TYPESAFE_API_KEY or restore the network, then re-submit the pending items",
    );
  });

  test("a judge error says so and does not mention the key", () => {
    const next = evidenceNext(
      report([result("pending", ["judge error: TypeSafe returned HTTP 400"])]),
    );
    expect(next).toContain("judge error");
    expect(next).not.toContain("TYPESAFE_API_KEY");
  });

  test("both causes are both named", () => {
    const next = evidenceNext(
      report([
        result("pending", ["judge unavailable: no key"]),
        result("pending", ["judge error: bad answer"]),
      ]),
    );
    expect(next).toContain("TYPESAFE_API_KEY");
    expect(next).toContain("judge error");
  });
});
