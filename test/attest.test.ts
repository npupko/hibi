import { afterEach, describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { renderAttest } from "../src/cli/render/attest.ts";
import { resolveMode } from "../src/cli/render/mode.ts";
import { makeStyle } from "../src/cli/render/style.ts";
import {
  Attestation,
  type Claim,
  type JudgeCacheEntry,
} from "../src/core/model.ts";
import { attestClaim, parseAck } from "../src/engine/attest.ts";
import { Engine } from "../src/index.ts";
import { cacheKey } from "../src/judge/cache.ts";
import { FakeJudge, supportsAnswer } from "../src/judge/fake.ts";
import { makeRepo, type TempRepo } from "./helpers.ts";

const NOW = new Date("2026-10-22T12:00:00.000Z");
const AT = "2026-10-22T09:12:00Z";
const A = "clm_00000000000000a1";
const B = "clm_00000000000000b2";
const RETIRED = "clm_00000000000000c3";
const SENTENCE_A = "  Pro  costs\n$29/month. ";
// "café" with a decomposed é (NFD): the hash must be of the NFC form.
const SENTENCE_B = "Pro costs $39/month at the café.";

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

let repos: TempRepo[] = [];
let tmpRoots: string[] = [];
afterEach(async () => {
  await Promise.all(repos.map((r) => r.cleanup()));
  repos = [];
  await Promise.all(
    tmpRoots.map((d) => rm(d, { recursive: true, force: true })),
  );
  tmpRoots = [];
});

async function setup(): Promise<TempRepo> {
  const repo = await makeRepo();
  repos.push(repo);
  // Written B first so the pair order does not follow insertion order.
  await repo.store.putClaim(attested(B, SENTENCE_B));
  await repo.store.putClaim(attested(A, SENTENCE_A));
  await repo.store.putClaim(
    attested(RETIRED, "Old plan.", { enforcement: "retired" }),
  );
  return repo;
}

function sha256(s: string): string {
  return createHash("sha256").update(s).digest("hex");
}

function supportsEntry(
  request: unknown,
  claimId: string,
  at: string,
  evidenceAt?: string,
): JudgeCacheEntry {
  return {
    key: cacheKey("jev-1.14.0", request),
    model: "jev-1.14.0",
    judgment: "supports",
    subject: { claimId },
    request,
    answers: {},
    ...(evidenceAt !== undefined
      ? {
          evidence: {
            hash: "h",
            source: "s",
            summary: "x",
            by: "agent:opus",
            at: evidenceAt,
          },
        }
      : {}),
    at,
  };
}

describe("attestClaim: vouch", () => {
  test("a human vouch is recorded and counts toward the ttl", async () => {
    const repo = await setup();
    const result = await attestClaim({
      store: repo.store,
      claimId: A,
      by: "human:alice@example.com",
      note: "checked Stripe, still $29",
      now: NOW,
    });
    const attestation: Attestation = {
      by: "human:alice@example.com",
      at: NOW.toISOString(),
      basis: "vouch",
      note: "checked Stripe, still $29",
    };
    expect(result).toEqual({
      claimIds: [A],
      attestation,
      countsTowardTtl: true,
    });
    expect((await repo.store.getClaim(A))?.attestations).toEqual([attestation]);
  });

  test("an agent or process vouch is recorded and does not count toward the ttl", async () => {
    const repo = await setup();
    for (const by of ["agent:opus", "process:ci"]) {
      const result = await attestClaim({
        store: repo.store,
        claimId: A,
        by,
        now: NOW,
      });
      expect(result.countsTowardTtl).toBe(false);
      expect(result.attestation).toEqual({
        by,
        at: NOW.toISOString(),
        basis: "vouch",
      });
    }
    expect((await repo.store.getClaim(A))?.attestations).toHaveLength(2);
  });

  test("the human renderer states that an agent vouch does not reset the ttl", async () => {
    const repo = await setup();
    const mode = resolveMode({ format: "human", color: "never" });
    const agent = await attestClaim({
      store: repo.store,
      claimId: A,
      by: "agent:opus",
    });
    expect(renderAttest(agent, makeStyle(false), mode)).toContain(
      "recorded; does not reset the ttl (agent vouches never do)",
    );
    const human = await attestClaim({
      store: repo.store,
      claimId: A,
      by: "human:alice@example.com",
    });
    expect(renderAttest(human, makeStyle(false), mode)).not.toContain(
      "does not reset the ttl",
    );
  });

  test("errors: bad actor, empty note, unknown claim, retired claim", async () => {
    const repo = await setup();
    const store = repo.store;
    await expect(
      attestClaim({ store, claimId: A, by: "alice" }),
    ).rejects.toThrow("--by must be human:<id>");
    await expect(
      attestClaim({ store, claimId: A, by: "human:a", note: " " }),
    ).rejects.toThrow("--note is empty");
    await expect(
      attestClaim({ store, claimId: "clm_ffffffffffffffff", by: "human:a" }),
    ).rejects.toThrow("No claim clm_ffffffffffffffff");
    await expect(
      attestClaim({ store, claimId: RETIRED, by: "human:a" }),
    ).rejects.toThrow("is retired");
    expect((await store.getClaim(A))?.attestations).toEqual([]);
  });
});

describe("attestClaim: ack", () => {
  test("an ack without a note fails and writes nothing", async () => {
    const repo = await setup();
    for (const ack of [
      { kind: "review" } as const,
      { kind: "conflict", otherClaimId: B } as const,
    ]) {
      await expect(
        attestClaim({ store: repo.store, claimId: A, by: "human:a", ack }),
      ).rejects.toThrow("an ack needs --note");
    }
    expect((await repo.store.getClaim(A))?.attestations).toEqual([]);
    expect((await repo.store.getClaim(B))?.attestations).toEqual([]);
  });

  test("ack conflict writes the same ack to both claims with pair id and sentence hashes", async () => {
    const repo = await setup();
    const result = await attestClaim({
      store: repo.store,
      claimId: B,
      by: "human:alice@example.com",
      note: "different plans in different regions",
      ack: { kind: "conflict", otherClaimId: A },
      now: NOW,
    });
    const expected: Attestation = {
      by: "human:alice@example.com",
      at: NOW.toISOString(),
      basis: "ack",
      note: "different plans in different regions",
      ack: {
        pairId: `${A}:${B}`,
        sentenceHashes: [
          sha256("Pro costs $29/month."),
          sha256("Pro costs $39/month at the café."),
        ],
      },
    };
    expect(result).toEqual({
      claimIds: [A, B],
      attestation: expected,
      countsTowardTtl: false,
    });
    expect((await repo.store.getClaim(A))?.attestations).toEqual([expected]);
    expect((await repo.store.getClaim(B))?.attestations).toEqual([expected]);
  });

  test("ack conflict errors: same claim, unknown or retired other; nothing written", async () => {
    const repo = await setup();
    const base = { store: repo.store, claimId: A, by: "human:a", note: "n" };
    await expect(
      attestClaim({ ...base, ack: { kind: "conflict", otherClaimId: A } }),
    ).rejects.toThrow("names the same claim");
    await expect(
      attestClaim({
        ...base,
        ack: { kind: "conflict", otherClaimId: "clm_ffffffffffffffff" },
      }),
    ).rejects.toThrow("No claim clm_ffffffffffffffff");
    await expect(
      attestClaim({
        ...base,
        ack: { kind: "conflict", otherClaimId: RETIRED },
      }),
    ).rejects.toThrow("is retired");
    expect((await repo.store.getClaim(A))?.attestations).toEqual([]);
  });

  test("ack review pins the newest supports verdict of the claim", async () => {
    const repo = await setup();
    const cache = repo.store.judgeCache();
    const old = supportsEntry({ n: 1 }, A, "2026-10-20T00:00:00.000Z");
    // Asked early, but its evidence was re-submitted most recently.
    const resubmitted = supportsEntry(
      { n: 2 },
      A,
      "2026-10-19T00:00:00.000Z",
      "2026-10-22T00:00:00.000Z",
    );
    const newer = supportsEntry({ n: 3 }, A, "2026-10-21T00:00:00.000Z");
    const otherClaim = supportsEntry({ n: 4 }, B, "2026-10-23T00:00:00.000Z");
    const conflict: JudgeCacheEntry = {
      ...supportsEntry({ n: 5 }, A, "2026-10-24T00:00:00.000Z"),
      judgment: "conflict",
    };
    for (const e of [old, resubmitted, newer, otherClaim, conflict]) {
      await cache.put(e);
    }
    const result = await attestClaim({
      store: repo.store,
      claimId: A,
      by: "agent:opus",
      note: "the doc only covers the US plan",
      ack: { kind: "review" },
      now: NOW,
    });
    expect(result.attestation.ack).toEqual({ judgeKey: resubmitted.key });
    expect(result.claimIds).toEqual([A]);
    expect(result.countsTowardTtl).toBe(false);
    expect((await repo.store.getClaim(A))?.attestations).toEqual([
      result.attestation,
    ]);
  });

  test("ack review without a supports verdict fails", async () => {
    const repo = await setup();
    await expect(
      attestClaim({
        store: repo.store,
        claimId: A,
        by: "human:a",
        note: "n",
        ack: { kind: "review" },
      }),
    ).rejects.toThrow("has no supports verdict to acknowledge");
  });

  test("parseAck accepts conflict:<id> and review only", () => {
    expect(parseAck("review")).toEqual({ kind: "review" });
    expect(parseAck(`conflict:${B}`)).toEqual({
      kind: "conflict",
      otherClaimId: B,
    });
    for (const bad of ["conflict:", "conflict", "reviewed", ""]) {
      expect(() => parseAck(bad), bad).toThrow("--ack expects");
    }
  });
});

describe("Engine.attest --ack review on a reference claim", () => {
  const REVIEW = supportsAnswer({
    supports: 0.6,
    refutes: 0.3,
    does_not_address: 0.1,
  });

  test("names the key check judges for the current texts; check then stops warning", async () => {
    const root = await mkdtemp(join(tmpdir(), "hibi-ackref-"));
    tmpRoots.push(root);
    await writeFile(
      join(root, "README.md"),
      "# Demo\n\nRefunds take two weeks.\n",
    );
    await mkdir(join(root, "docs"), { recursive: true });
    await writeFile(
      join(root, "docs/billing.md"),
      "# Billing\n\nRefunds are issued within 14 days.\n",
    );
    const judge = new FakeJudge({ supports: () => REVIEW });
    const e = await Engine.init(root, { judge, noAst: true });
    const rec = await e.record({
      kind: "reference",
      by: "human:a@example.com",
      docPath: "README.md",
      docQuote: "Refunds take two weeks.",
      evidence: [
        {
          file: "docs/billing.md",
          region: { quote: "Refunds are issued within 14 days." },
        },
      ],
    });
    const id = rec.claim.id;
    const ack = { by: "human:a@example.com", note: "same thing" };

    // Before any check there is no verdict for the current texts.
    await expect(
      e.attest(id, { ...ack, ack: { kind: "review" } }),
    ).rejects.toThrow(
      `claim ${id} has no supports verdict for its current text; run \`hibi check\` first.`,
    );

    const first = await e.check();
    expect(first.verdicts[0]?.judge).toBe("review");
    expect(first.verdicts[0]?.warns).toBe(true);
    const [entry] = await e.store.judgeCache().list({ claimId: id });

    const res = await e.attest(id, { ...ack, ack: { kind: "review" } });
    expect(res.attestation.ack).toEqual({ judgeKey: entry?.key as string });

    const after = await e.check();
    expect(after.verdicts[0]?.judge).toBe("review");
    expect(after.verdicts[0]?.warns).toBe(false);
    expect(after.worklist).toEqual([]);

    // A newer, unrelated supports entry does not change which verdict is current.
    await e.store.judgeCache().put({
      ...(entry as JudgeCacheEntry),
      key: "f".repeat(64),
      at: "2099-01-01T00:00:00.000Z",
    });
    const res2 = await e.attest(id, { ...ack, ack: { kind: "review" } });
    expect(res2.attestation.ack).toEqual({ judgeKey: entry?.key as string });

    // A changed referenced text is a new verdict; the old ack no longer covers it.
    await writeFile(
      join(root, "docs/billing.md"),
      "# Billing\n\nRefunds are issued within 14 days of the request.\n",
    );
    const reanchored = await e.reanchor(id, {
      reference: {
        file: "docs/billing.md",
        region: { quote: "Refunds are issued within 14 days of the request." },
      },
    });
    expect(reanchored.evidence).toBe("unchanged");
    const changed = await e.check();
    expect(changed.verdicts[0]?.warns).toBe(true);
  });
});

describe("Attestation model: ack", () => {
  const pair = { pairId: `${A}:${B}`, sentenceHashes: ["h1", "h2"] };

  test("basis ack requires ack; both ack shapes parse", () => {
    expect(
      Attestation.safeParse({ by: "human:a", at: AT, basis: "ack", note: "n" })
        .success,
    ).toBe(false);
    expect(
      Attestation.safeParse({ by: "human:a", at: AT, basis: "ack", ack: pair })
        .success,
    ).toBe(true);
    expect(
      Attestation.safeParse({
        by: "human:a",
        at: AT,
        basis: "ack",
        ack: { judgeKey: "k" },
      }).success,
    ).toBe(true);
  });

  test("ack is invalid on vouch and evidence attestations", () => {
    for (const basis of ["vouch", "evidence"]) {
      expect(
        Attestation.safeParse({ by: "human:a", at: AT, basis, ack: pair })
          .success,
        basis,
      ).toBe(false);
    }
  });

  test("malformed ack shapes are rejected", () => {
    for (const ack of [
      { pairId: "p", sentenceHashes: ["h1"] },
      { pairId: "p", sentenceHashes: ["h1", "h2", "h3"] },
      { pairId: "p" },
      { judgeKey: "k", pairId: "p", sentenceHashes: ["h1", "h2"] },
      { judgeKey: "" },
      {},
    ]) {
      expect(
        Attestation.safeParse({ by: "human:a", at: AT, basis: "ack", ack })
          .success,
        JSON.stringify(ack),
      ).toBe(false);
    }
  });
});

describe("CLI: attest and evidence", () => {
  const CLI = join(import.meta.dir, "..", "src", "cli", "index.ts");

  async function run(cwd: string, args: string[], stdin?: string) {
    const proc = Bun.spawn(["bun", "run", CLI, ...args, "--json"], {
      cwd,
      stdout: "pipe",
      stderr: "pipe",
      stdin: stdin === undefined ? "ignore" : new TextEncoder().encode(stdin),
      env: { ...process.env, TYPESAFE_API_KEY: "" },
    });
    const stdout = await new Response(proc.stdout).text();
    const code = await proc.exited;
    return { code, json: JSON.parse(stdout.trim()) };
  }

  test("attest records a vouch and reports countsTowardTtl", async () => {
    const repo = await setup();
    const r = await run(repo.root, ["attest", A, "--by", "agent:opus"]);
    expect(r.code).toBe(0);
    expect(r.json).toMatchObject({
      ok: true,
      action: "attest",
      claimIds: [A],
      attestation: { by: "agent:opus", basis: "vouch" },
      countsTowardTtl: false,
    });
  });

  test("attest requires --by; --ack needs --note", async () => {
    const repo = await setup();
    const noBy = await run(repo.root, ["attest", A]);
    expect(noBy.code).toBe(1);
    expect(noBy.json.error).toContain("attest requires --by");
    const noNote = await run(repo.root, [
      "attest",
      A,
      "--by",
      "human:a",
      "--ack",
      `conflict:${B}`,
    ]);
    expect(noNote.code).toBe(1);
    expect(noNote.json.error).toContain("an ack needs --note");
  });

  test("evidence without an API key is pending; --by fills items without by", async () => {
    const repo = await setup();
    const r = await run(
      repo.root,
      ["evidence", "--from-file", "-", "--by", "agent:opus"],
      JSON.stringify({
        claimId: A,
        evidence: "Pro $29",
        source: "stripe",
        summary: "Stripe says $29",
      }),
    );
    expect(r.code).toBe(0);
    expect(r.json).toMatchObject({
      ok: true,
      action: "evidence",
      results: [{ claimId: A, judge: "pending", attested: false }],
      summary: { pending: 1, requests: 0 },
    });
    expect(await repo.store.judgeCache().list()).toEqual([]);
  });

  test("evidence --by is optional when every item carries by; a missing by names the item", async () => {
    const repo = await setup();
    const ok = await run(
      repo.root,
      ["evidence", "--from-file", "-"],
      JSON.stringify([
        {
          claimId: A,
          evidence: "Pro $29",
          source: "stripe",
          summary: "Stripe says $29",
          by: "agent:opus",
        },
      ]),
    );
    expect(ok.code).toBe(0);
    expect(ok.json.results[0].judge).toBe("pending");
    const missing = await run(
      repo.root,
      ["evidence", "--from-file", "-"],
      JSON.stringify([
        {
          claimId: A,
          evidence: "Pro $29",
          source: "stripe",
          summary: "Stripe says $29",
          by: "agent:opus",
        },
        { claimId: B, evidence: "Pro $39", source: "s", summary: "x" },
      ]),
    );
    expect(missing.code).toBe(1);
    expect(missing.json.errors).toEqual([
      `item 1 (${B}): no "by"; pass --by <actor> or a "by" key: human:<id>, agent:<name>, or process:<name>`,
    ]);
  });

  test("evidence --from-file on a missing file prints the JSON error envelope", async () => {
    const repo = await setup();
    const r = await run(repo.root, [
      "evidence",
      "--from-file",
      "nope.json",
      "--by",
      "agent:opus",
    ]);
    expect(r.code).toBe(1);
    expect(r.json).toEqual({
      ok: false,
      error: "evidence --from-file: --from-file nope.json: no such file",
    });
  });

  test("evidence rejects an invalid batch with every error", async () => {
    const repo = await setup();
    const r = await run(
      repo.root,
      ["evidence", "--from-file", "-", "--by", "agent:opus"],
      JSON.stringify([{ claimId: A, evidence: "", source: "s", summary: "" }]),
    );
    expect(r.code).toBe(1);
    expect(r.json.error).toContain('"evidence" is empty');
    expect(r.json.error).toContain('"summary" is empty');
  });
});
