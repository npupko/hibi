/**
 * End to end: the CLI entry (`bun run src/cli/index.ts`) through the whole
 * loop in a temporary git repository. The judge is the real Jev adapter over
 * HTTP, pointed with `TYPESAFE_BASE_URL` at a local server that answers with
 * `FakeJudge`'s wire-shaped answers, so the CLI decodes them on the one
 * production path. Runs without `TYPESAFE_API_KEY` show the no-key behavior.
 *
 * Assertions are on exit codes and JSON fields, never on human text.
 */
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { INSTRUCTIONS_LINE } from "../src/cli/options.ts";
import type { Claim } from "../src/core/model.ts";
import { addDuration } from "../src/engine/expiry.ts";
import {
  conflictAnswer,
  FakeJudge,
  supportsAnswer,
} from "../src/judge/fake.ts";
import type { SystemOneRequest } from "../src/judge/http.ts";
import {
  type ConflictSide,
  choiceConfidence,
  type Region,
} from "../src/judge/types.ts";

const CLI = join(import.meta.dir, "..", "src", "cli", "index.ts");

const CODE_SENTENCE = "Retries are capped at 5 attempts.";
const REF_SENTENCE = "Refunds are issued within two weeks of a request.";
const PRICE_SENTENCE = "The Pro plan costs $29 per month.";
const FAQ_SENTENCE = "The Pro plan costs $39 per month.";

const SURE_NARRATIVE = {
  factual_claim: 0.02,
  instruction: 0.02,
  opinion: 0.02,
  narrative: 0.94,
};
const SURE_FACT = {
  factual_claim: 0.94,
  instruction: 0.02,
  opinion: 0.02,
  narrative: 0.02,
};
const SURE_EXTERNAL = {
  code: 0.02,
  other_doc: 0.02,
  external: 0.94,
  nothing: 0.02,
};

/** The scripted judge behind the local System One server. */
const judge = new FakeJudge({
  triage: (region: Region) =>
    region.kind === "heading"
      ? {
          kind: "narrative",
          kindProbabilities: SURE_NARRATIVE,
          kindConfidence: choiceConfidence(SURE_NARRATIVE),
          checkable: 0.1,
        }
      : {
          kind: "factual_claim",
          kindProbabilities: SURE_FACT,
          kindConfidence: choiceConfidence(SURE_FACT),
          evidenceKind: "external",
          evidenceProbabilities: SURE_EXTERNAL,
          checkable: 0.9,
        },
  supports: () =>
    supportsAnswer({ supports: 0.95, refutes: 0.03, does_not_address: 0.02 }),
  conflict: () =>
    conflictAnswer(
      { consistent: 0.05, contradictory: 0.9, unrelated: 0.05 },
      0.95,
      0.9,
    ),
});

let server: ReturnType<typeof Bun.serve>;
let root: string;

/** Requests the local System One server answered, by judgment. */
const served = { triage: 0, supports: 0, conflict: 0 };

beforeAll(async () => {
  server = Bun.serve({
    port: 0,
    async fetch(req) {
      if (req.headers.get("authorization") !== "Bearer test-key") {
        return new Response("unauthorized", { status: 401 });
      }
      const body = (await req.json()) as SystemOneRequest;
      const state = body.state as Record<string, unknown>;
      let answers: unknown;
      if ("addresses" in body.questions) {
        served.supports++;
        answers = (
          await judge.supports({
            sentence: state.claim as string,
            context: state.claim_context as string,
            evidence: state.evidence as string,
            source: state.evidence_source as string,
          })
        ).answers;
      } else if ("same_subject" in body.questions) {
        served.conflict++;
        answers = (
          await judge.conflict(state.a as ConflictSide, state.b as ConflictSide)
        ).answers;
      } else {
        served.triage++;
        const res = await judge.triage(
          state.doc as { path: string },
          state.regions as Region[],
        );
        answers = (res.answers as unknown[])[0];
      }
      return Response.json({
        model: body.model,
        answers,
        usage: { input_tokens: 1, output_tokens: 1 },
      });
    },
  });

  root = await mkdtemp(join(tmpdir(), "hibi-e2e-"));
  const files: Record<string, string> = {
    "src/retry.ts": "export const MAX_ATTEMPTS = 5;\n",
    "docs/billing.md": "# Billing\n\nRefunds are issued within 14 days.\n",
    "README.md": `# Demo\n\n${CODE_SENTENCE}\n\n${REF_SENTENCE}\n`,
    "docs/pricing.md": `# Pricing\n\n${PRICE_SENTENCE}\n`,
    "AGENTS.md": "# Agents\n\nRun the tests before committing.\n",
  };
  for (const [rel, text] of Object.entries(files)) {
    await mkdir(dirname(join(root, rel)), { recursive: true });
    await writeFile(join(root, rel), text);
  }
  for (const args of [
    ["init", "-q"],
    ["config", "user.email", "dev@example.com"],
    ["config", "user.name", "dev"],
  ]) {
    await Bun.spawn(["git", ...args], { cwd: root }).exited;
  }
});

afterAll(async () => {
  server.stop(true);
  await rm(root, { recursive: true, force: true });
});

interface Run {
  code: number;
  json: Record<string, unknown>;
  stdout: string;
}

/** Run the CLI. `judge: true` points it at the local server with a key; false runs with no key. */
async function hibi(
  args: string[],
  opts: { judge?: boolean; stdin?: unknown } = {},
): Promise<Run> {
  const env: Record<string, string | undefined> = { ...process.env };
  delete env.TYPESAFE_API_KEY;
  delete env.TYPESAFE_BASE_URL;
  if (opts.judge) {
    env.TYPESAFE_API_KEY = "test-key";
    env.TYPESAFE_BASE_URL = `http://localhost:${server.port}`;
  }
  const proc = Bun.spawn(["bun", "run", CLI, ...args, "--json"], {
    cwd: root,
    env,
    stdout: "pipe",
    stderr: "pipe",
    stdin:
      opts.stdin === undefined
        ? "ignore"
        : new TextEncoder().encode(JSON.stringify(opts.stdin)),
  });
  const stdout = await new Response(proc.stdout).text();
  const code = await proc.exited;
  let json: Record<string, unknown> = {};
  try {
    json = JSON.parse(stdout.trim()) as Record<string, unknown>;
  } catch {
    /* not JSON */
  }
  return { code, json, stdout };
}

interface Verdict {
  claimId: string;
  judge?: string;
  gates: boolean;
  warns: boolean;
  conflicts: { claimId: string; probability: number; reason: string }[];
  reasons: string[];
}
interface WorkItem {
  claimId: string;
  reason: string;
  sources: unknown[];
  procedure: unknown;
}
interface ListRow {
  claimId: string;
  expiresAt: string | null;
  state: string;
}

const verdicts = (r: Run) => r.json.verdicts as Verdict[];
const verdictOf = (r: Run, id: string) =>
  verdicts(r).find((v) => v.claimId === id) as Verdict;
const worklist = (r: Run) => r.json.worklist as WorkItem[];

async function claimFile(id: string): Promise<Claim> {
  return JSON.parse(
    await readFile(join(root, ".claims", "claims", `${id}.json`), "utf8"),
  ) as Claim;
}

async function expiresAtOf(id: string): Promise<string | null> {
  const rows = (await hibi(["list"])).json.claims as ListRow[];
  return rows.find((r) => r.claimId === id)?.expiresAt ?? null;
}

describe("the whole loop through the CLI", () => {
  const ids = { code: "", reference: "", attested: "", faq: "" };

  test("init --instructions writes the store and appends the check line once", async () => {
    const res = await hibi(["init", "--instructions", "AGENTS.md"]);
    expect(res.code).toBe(0);
    expect(res.json.next).toBe("hibi audit --doc <file>");
    expect(res.json.instructions).toEqual({
      file: "AGENTS.md",
      appended: true,
    });
    expect(res.json.judge).toEqual({ model: "jev-1.14.0", threshold: 0.8 });
    const config = JSON.parse(
      await readFile(join(root, ".claims", "config.json"), "utf8"),
    );
    expect(config).toEqual({
      version: "v4",
      repos: {},
      judge: { model: "jev-1.14.0", threshold: 0.8 },
      ignore: [],
    });
    const agents = await readFile(join(root, "AGENTS.md"), "utf8");
    expect(agents.split(INSTRUCTIONS_LINE)).toHaveLength(2);
  });

  test("record one claim of each kind", async () => {
    const code = await hibi([
      "record",
      "--doc",
      "README.md",
      "--doc-quote",
      CODE_SENTENCE,
      "--kind",
      "code",
      "--code-file",
      "src/retry.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
    ]);
    expect(code.code).toBe(0);
    expect(code.json.kind).toBe("code");
    ids.code = code.json.id as string;

    const reference = await hibi([
      "record",
      "--doc",
      "README.md",
      "--doc-quote",
      REF_SENTENCE,
      "--kind",
      "reference",
      "--ref-file",
      "docs/billing.md",
      "--ref-quote",
      "Refunds are issued within 14 days.",
    ]);
    expect(reference.code).toBe(0);
    expect(reference.json.evidence).toBe("docs/billing.md");
    ids.reference = reference.json.id as string;

    const attested = await hibi([
      "record",
      "--doc",
      "docs/pricing.md",
      "--doc-quote",
      PRICE_SENTENCE,
      "--kind",
      "attested",
      "--ttl",
      "P90D",
      "--owner",
      "human:alice@example.com",
      "--source",
      "stripe=https://dashboard.stripe.com/products/prod_123",
      "--instructions",
      "Open the Stripe dashboard and report the Pro monthly price.",
    ]);
    expect(attested.code).toBe(0);
    ids.attested = attested.json.id as string;

    const stored = await claimFile(ids.attested);
    expect(stored.kind).toBe("attested");
    expect(stored.evidence).toEqual([]);
    expect(stored.sources).toEqual([
      {
        id: "stripe",
        resource: "https://dashboard.stripe.com/products/prod_123",
      },
    ]);
    expect(stored.procedure).toEqual({
      instructions:
        "Open the Stripe dashboard and report the Pro monthly price.",
    });
    expect(stored.attestations).toHaveLength(1);
    expect(stored.attestations[0]?.by).toBe("human:dev@example.com");
  });

  test("an unknown sibling-repo alias is refused at record time", async () => {
    const res = await hibi([
      "record",
      "--doc",
      "README.md",
      "--doc-quote",
      CODE_SENTENCE,
      "--kind",
      "code",
      "--code-file",
      "api:src/limits.ts",
      "--code-quote",
      "x",
    ]);
    expect(res.code).toBe(1);
    expect(res.json.errors).toEqual([
      'unknown repo alias "api" in api:src/limits.ts; add it to "repos" in .claims/config.json',
    ]);
  });

  test("check without a key: attested is in the worklist, reference is pending", async () => {
    const res = await hibi(["check"]);
    expect(res.code).toBe(0);
    expect(verdictOf(res, ids.code).gates).toBe(false);
    expect(verdictOf(res, ids.code).warns).toBe(false);
    expect(verdictOf(res, ids.reference).judge).toBe("pending");
    expect(verdictOf(res, ids.reference).warns).toBe(true);
    const items = worklist(res);
    expect(items.map((w) => [w.claimId, w.reason]).sort()).toEqual(
      [
        [ids.attested, "never-checked"],
        [ids.reference, "pending"],
      ].sort(),
    );
    const att = items.find((w) => w.claimId === ids.attested);
    expect(att?.sources).toHaveLength(1);
    expect(att?.procedure).not.toBeNull();
    expect(res.json.summary).toMatchObject({
      worklist: 2,
      conflictPass: { pairs: 0, requests: 0, pending: 0 },
    });

    const pending = await hibi(["list", "--state", "pending"]);
    expect(pending.code).toBe(0);
    expect(
      (pending.json.claims as ListRow[]).map((r) => r.claimId).sort(),
    ).toEqual([ids.attested, ids.reference].sort());
  });

  test("evidence with supporting text attests the claim", async () => {
    const res = await hibi(
      ["evidence", "--from-file", "-", "--by", "agent:x"],
      {
        judge: true,
        stdin: [
          {
            claimId: ids.attested,
            evidence: "Pro plan: $29.00 per month, billed monthly.",
            source: "stripe",
            summary: "Stripe lists Pro at $29/month",
          },
        ],
      },
    );
    expect(res.code).toBe(0);
    const [result] = res.json.results as {
      judge: string;
      attested: boolean;
      reasons: string[];
    }[];
    expect(result?.judge).toBe("supported");
    expect(result?.attested).toBe(true);
    expect(result?.reasons).toContain(
      'source matches declared source "stripe"',
    );
  });

  test("check shows supported, and the ttl counts from the evidence attestation", async () => {
    const res = await hibi(["check"], { judge: true });
    expect(res.code).toBe(0);
    expect(verdictOf(res, ids.attested).judge).toBe("supported");
    expect(verdictOf(res, ids.reference).judge).toBe("supported");
    expect(worklist(res)).toEqual([]);

    const stored = await claimFile(ids.attested);
    const evidence = stored.attestations.at(-1);
    expect(evidence?.basis).toBe("evidence");
    expect(evidence?.by).toBe("agent:x");
    const expected = addDuration(
      new Date(evidence?.at as string),
      "P90D",
    ).toISOString();
    expect(await expiresAtOf(ids.attested)).toBe(expected);

    // Cached: a second run with no key asks nothing and answers the same.
    const again = await hibi(["check"]);
    expect(verdictOf(again, ids.reference).judge).toBe("supported");
    expect(verdictOf(again, ids.attested).judge).toBe("supported");
  });

  test("an agent vouch does not reset the ttl; a human vouch does", async () => {
    const before = await expiresAtOf(ids.attested);
    const agent = await hibi(["attest", ids.attested, "--by", "agent:x"]);
    expect(agent.code).toBe(0);
    expect(agent.json.countsTowardTtl).toBe(false);
    expect(await expiresAtOf(ids.attested)).toBe(before);

    await Bun.sleep(5);
    const human = await hibi([
      "attest",
      ids.attested,
      "--by",
      "human:alice@example.com",
      "--note",
      "checked Stripe, still $29",
    ]);
    expect(human.code).toBe(0);
    expect(human.json.countsTowardTtl).toBe(true);
    const after = await expiresAtOf(ids.attested);
    expect(after).not.toBe(before);
    const vouch = (await claimFile(ids.attested)).attestations.at(-1);
    expect(after).toBe(
      addDuration(new Date(vouch?.at as string), "P90D").toISOString(),
    );
  });

  test("audit --doc: covered rows name their claim; uncovered rows are triaged or pending", async () => {
    const offline = await hibi(["audit", "--doc", "docs/pricing.md"]);
    expect(offline.code).toBe(0);
    const offRows = offline.json.rows as {
      claimId: string | null;
      kind: string;
    }[];
    expect(offRows.map((r) => [r.claimId, r.kind])).toEqual([
      [null, "pending"],
      [ids.attested, "attested"],
    ]);

    const res = await hibi(["audit", "--doc", "README.md"], { judge: true });
    expect(res.code).toBe(0);
    const rows = res.json.rows as {
      claimId: string | null;
      kind: string;
      duplicateOf: string | null;
    }[];
    expect(rows.map((r) => [r.claimId, r.kind])).toEqual([
      [null, "not-a-claim"],
      [ids.code, "code"],
      [ids.reference, "reference"],
    ]);
    expect(rows.every((r) => r.duplicateOf === null)).toBe(true);
  });

  test("list --state pending matches the check worklist", async () => {
    const check = await hibi(["check"]);
    const pending = await hibi(["list", "--state", "pending"]);
    expect(pending.code).toBe(0);
    expect((pending.json.claims as ListRow[]).map((r) => r.claimId)).toEqual(
      worklist(check).map((w) => w.claimId),
    );
  });

  test("a contradiction gates in check and is cleared by attest --ack conflict", async () => {
    await writeFile(join(root, "docs/faq.md"), `# FAQ\n\n${FAQ_SENTENCE}\n`);
    const faq = await hibi([
      "record",
      "--doc",
      "docs/faq.md",
      "--doc-quote",
      FAQ_SENTENCE,
      "--kind",
      "attested",
      "--ttl",
      "P30D",
    ]);
    expect(faq.code).toBe(0);
    ids.faq = faq.json.id as string;

    // Without a key the new pair cannot be judged: a warning on both claims.
    const offline = await hibi(["check"]);
    expect(offline.code).toBe(0);
    expect(verdictOf(offline, ids.attested).reasons).toContain(
      `conflict check pending with ${ids.faq}: judge unavailable`,
    );
    expect(offline.json.summary).toMatchObject({
      conflictPass: { pairs: 1, requests: 0, pending: 1 },
    });

    const res = await hibi(["check"], { judge: true });
    expect(res.code).toBe(2);
    expect(res.json.summary).toMatchObject({
      conflicts: 2,
      conflictPass: { pairs: 1, requests: 1, pending: 0 },
    });
    const v = verdictOf(res, ids.attested);
    expect(v.gates).toBe(true);
    // The label lists the values in pair order: the smaller claim id first.
    // Claim ids are random, so the expected order follows the ids.
    const values = ids.attested < ids.faq ? "29 vs 39" : "39 vs 29";
    expect(v.conflicts).toEqual([
      {
        claimId: ids.faq,
        probability: 1,
        reason: `numbers differ: ${values}`,
      },
    ]);
    const conflict = await hibi(["list", "--state", "conflict"]);
    expect(
      (conflict.json.claims as ListRow[]).map((r) => r.claimId).sort(),
    ).toEqual([ids.attested, ids.faq].sort());

    // --doc scopes the verdicts but still sees the finding across documents.
    const scoped = await hibi(["check", "--doc", "docs/pricing.md"]);
    expect(scoped.code).toBe(2);
    expect(verdicts(scoped).map((x) => x.claimId)).toEqual([ids.attested]);

    const ack = await hibi([
      "attest",
      ids.attested,
      "--by",
      "human:alice@example.com",
      "--ack",
      `conflict:${ids.faq}`,
      "--note",
      "the FAQ describes the annual plan",
    ]);
    expect(ack.code).toBe(0);
    expect((ack.json.claimIds as string[]).sort()).toEqual(
      [ids.attested, ids.faq].sort(),
    );
    const cleared = await hibi(["check"]);
    expect(cleared.code).toBe(0);
    expect(verdictOf(cleared, ids.attested).conflicts).toEqual([]);
    expect(verdictOf(cleared, ids.faq).conflicts).toEqual([]);
  });

  test("retire withdraws a claim", async () => {
    const res = await hibi(["retire", ids.faq]);
    expect(res.code).toBe(0);
    expect(res.json.alreadyRetired).toBe(false);
    const check = await hibi(["check"]);
    expect(check.code).toBe(0);
    expect(check.json.summary).toMatchObject({ retired: 1 });
    expect(worklist(check).map((w) => w.claimId)).not.toContain(ids.faq);
  });

  test("the local server answered every judgment the loop needed", () => {
    expect(served.supports).toBeGreaterThanOrEqual(2);
    expect(served.conflict).toBe(1);
    expect(served.triage).toBe(1);
  });
});
