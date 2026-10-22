/**
 * `audit` (SPEC 10): region-to-claim matching, triage kind mapping, the
 * unsure and pending rows, the judge cache, ignore globs, `--fail-uncovered`,
 * and the human renderer's advice mapping.
 *
 * Answered triage runs through the real Jev adapter over a scripted HTTP stub,
 * because `audit` maps cached raw answers with `mapTriageAnswers`; the
 * FakeJudge serves the unavailable case.
 */
import { afterEach, describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { adviceFor, renderAudit } from "../src/cli/render/audit.ts";
import type { OutputMode } from "../src/cli/render/mode.ts";
import { makeStyle } from "../src/cli/render/style.ts";
import type { Config } from "../src/core/model.ts";
import {
  type AuditReport,
  type AuditRow,
  auditExitCode,
  ignoredBy,
  runAudit,
} from "../src/engine/audit.ts";
import { splitRegions } from "../src/engine/regions.ts";
import { Engine } from "../src/index.ts";
import { FakeJudge } from "../src/judge/fake.ts";
import type {
  Answer,
  SystemOneRequest,
  SystemOneResponse,
} from "../src/judge/http.ts";
import { buildTriageRequests, JevJudge } from "../src/judge/jev.ts";
import {
  choiceConfidence,
  type EvidenceKind,
  type Region,
  type TriageKind,
} from "../src/judge/types.ts";

const CLI = join(import.meta.dir, "..", "src", "cli", "index.ts");

let dirs: string[] = [];
afterEach(async () => {
  await Promise.all(dirs.map((d) => rm(d, { recursive: true, force: true })));
  dirs = [];
});

async function tmp(): Promise<string> {
  const d = await mkdtemp(join(tmpdir(), "ce-audit-"));
  dirs.push(d);
  return d;
}

async function write(root: string, rel: string, content: string) {
  const abs = join(root, rel);
  await mkdir(dirname(abs), { recursive: true });
  await writeFile(abs, content);
}

// ── Scripted triage over the real Jev adapter ────────────────────────────────

interface Scripted {
  kind: Record<TriageKind, number>;
  evidence?: Record<EvidenceKind, number>;
}

const TRIAGE_KINDS: TriageKind[] = [
  "factual_claim",
  "instruction",
  "opinion",
  "narrative",
];
const EVIDENCE_KINDS: EvidenceKind[] = [
  "code",
  "other_doc",
  "external",
  "nothing",
];

function sure<K extends string>(options: K[], pick: K): Record<K, number> {
  return Object.fromEntries(
    options.map((o) => [o, o === pick ? 0.97 : 0.01]),
  ) as Record<K, number>;
}

function fact(evidence: EvidenceKind): Scripted {
  return {
    kind: sure(TRIAGE_KINDS, "factual_claim"),
    evidence: sure(EVIDENCE_KINDS, evidence),
  };
}

function notFact(kind: Exclude<TriageKind, "factual_claim">): Scripted {
  return { kind: sure(TRIAGE_KINDS, kind) };
}

const UNSURE_KIND = {
  factual_claim: 0.4,
  instruction: 0.05,
  opinion: 0.5,
  narrative: 0.05,
};

const UNSURE_FACT = {
  factual_claim: 0.6,
  instruction: 0.05,
  opinion: 0.3,
  narrative: 0.05,
};

const UNSURE_FACT_EVIDENCE = {
  code: 0.1,
  other_doc: 0.2,
  external: 0.6,
  nothing: 0.1,
};

/** A confident factual claim whose evidence kind is split: Choice confidence 0.6. */
const SPLIT_EVIDENCE = {
  code: 0.1,
  other_doc: 0.1,
  external: 0.7,
  nothing: 0.1,
};

const FENCE = "```ts\nconst RETRY = 5;\n```";

const SCRIPT: Record<string, Scripted> = {
  "# Billing": notFact("narrative"),
  "Refund rules are described in the billing guide.": fact("other_doc"),
  "Pro costs $29/month.": fact("external"),
  "The CLI exits with code 2 on a gating claim.": fact("code"),
  "Customers love the product.": fact("nothing"),
  "Always run the tests first.": notFact("instruction"),
  "Our support is the best.": notFact("opinion"),
  "This page covers billing.": notFact("narrative"),
  "It might be fast.": { kind: UNSURE_KIND },
  "It handles most traffic.": {
    kind: UNSURE_FACT,
    evidence: UNSURE_FACT_EVIDENCE,
  },
  "Deploys take ten minutes.": {
    kind: sure(TRIAGE_KINDS, "factual_claim"),
    evidence: SPLIT_EVIDENCE,
  },
  [FENCE]: { kind: sure(TRIAGE_KINDS, "factual_claim") },
};

function argmax(p: Record<string, number>): string {
  return Object.entries(p).sort((a, b) => b[1] - a[1])[0]?.[0] ?? "";
}

function choice(p: Record<string, number>): Answer {
  return {
    choice: argmax(p),
    confidence: choiceConfidence(p),
    probabilities: p,
  };
}

function scriptedJev(model: string) {
  const requests: SystemOneRequest[] = [];
  const judge = new JevJudge({
    model,
    http: {
      async systemOne(body: SystemOneRequest): Promise<SystemOneResponse> {
        requests.push(body);
        const regions = (body.state as { regions: Region[] }).regions;
        const answers: Record<string, Answer> = {};
        regions.forEach((region, i) => {
          const s = SCRIPT[region.text];
          if (s === undefined) {
            throw new Error(`no scripted triage for ${region.text}`);
          }
          answers[`kind_${i}`] = choice(s.kind);
          if (`evidenceKind_${i}` in body.questions) {
            answers[`evidenceKind_${i}`] = choice(
              s.evidence ?? sure(EVIDENCE_KINDS, "nothing"),
            );
          }
        });
        return { model, answers, usage: { input_tokens: 1, output_tokens: 1 } };
      },
    },
  });
  return { judge, requests };
}

function unavailable(): FakeJudge {
  const judge = new FakeJudge();
  judge.setUnavailable("no key");
  return judge;
}

// ── Fixture repository ───────────────────────────────────────────────────────

const DOC_PATH = "docs/billing.md";

const DOC = [
  "# Billing",
  "",
  "The retry limit is five attempts. Payments settle within two days.",
  "",
  "Refund rules are described in the billing guide. Pro costs $29/month.",
  "The CLI exits with code 2 on a gating claim. Customers love the product.",
  "",
  "Always run the tests first. Our support is the best. This page covers billing.",
  "",
  "It might be fast. It handles most traffic.",
  "",
  FENCE,
  "",
].join("\n");

const BY = "human:tester@example.com";

interface Fixture {
  root: string;
  engine: Engine;
  config: Config;
  codeClaim: string;
  attestedClaim: string;
}

async function fixture(): Promise<Fixture> {
  const root = await tmp();
  await write(root, "src/retry.ts", "export const RETRY = 5;\n");
  await write(root, DOC_PATH, DOC);
  const engine = await Engine.init(root);
  const code = await engine.record({
    docPath: DOC_PATH,
    docQuote: "The retry limit is five attempts.",
    kind: "code",
    evidence: [{ file: "src/retry.ts", region: { quote: "RETRY = 5" } }],
    by: BY,
  });
  const attested = await engine.record({
    docPath: DOC_PATH,
    docQuote: "Payments settle within two days.",
    kind: "attested",
    ttl: "P30D",
    by: BY,
  });
  const retired = await engine.record({
    docPath: DOC_PATH,
    docQuote: "Our support is the best.",
    kind: "attested",
    ttl: "P30D",
    by: BY,
  });
  await engine.retire(retired.claim.id);
  return {
    root,
    engine,
    config: await engine.store.config(),
    codeClaim: code.claim.id,
    attestedClaim: attested.claim.id,
  };
}

async function audit(
  f: Fixture,
  opts: {
    judge?: Parameters<typeof runAudit>[0]["judge"];
    text?: string;
    config?: Config;
    duplicateOf?: (id: string) => string | null;
    docPath?: string;
  } = {},
): Promise<AuditReport> {
  return runAudit({
    store: f.engine.store,
    judge: opts.judge ?? scriptedJev(f.config.judge.model).judge,
    docPath: opts.docPath ?? DOC_PATH,
    text: opts.text ?? DOC,
    claims: await f.engine.store.allClaims(),
    config: opts.config ?? f.config,
    duplicateOf: opts.duplicateOf,
    now: () => new Date("2026-10-22T00:00:00.000Z"),
  });
}

function rowFor(report: AuditReport, text: string): AuditRow {
  const row = report.rows.find((r) => r.region.text === text);
  if (!row) throw new Error(`no row for ${text}`);
  return row;
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe("covered and uncovered regions", () => {
  test("a live claim's located doc span covers its region; retired claims cover nothing", async () => {
    const f = await fixture();
    const { judge, requests } = scriptedJev(f.config.judge.model);
    const report = await audit(f, { judge });

    expect(rowFor(report, "The retry limit is five attempts.")).toEqual({
      region: {
        id: "r1",
        line: 3,
        kind: "sentence",
        text: "The retry limit is five attempts.",
      },
      claimId: f.codeClaim,
      kind: "code",
      duplicateOf: null,
    });
    expect(rowFor(report, "Payments settle within two days.")).toMatchObject({
      claimId: f.attestedClaim,
      kind: "attested",
    });
    // The retired claim's sentence is uncovered and triaged.
    expect(rowFor(report, "Our support is the best.")).toMatchObject({
      claimId: null,
      kind: "not-a-claim",
    });

    // Covered regions are never sent to the judge.
    const sent = requests.flatMap((r) =>
      (r.state as { regions: Region[] }).regions.map((x) => x.text),
    );
    expect(sent).not.toContain("The retry limit is five attempts.");
    expect(sent).not.toContain("Payments settle within two days.");
    expect(sent).toContain("Our support is the best.");

    expect(report.summary).toEqual({
      regions: 13,
      covered: 2,
      uncovered: 11,
      byKind: {
        code: 3,
        reference: 1,
        attested: 2,
        unverifiable: 1,
        "not-a-claim": 4,
        unsure: 2,
        pending: 0,
      },
      requests: 1,
    });
  });

  test("a claim whose doc sentence was edited still covers its region", async () => {
    const f = await fixture();
    const edited = DOC.replace(
      "The retry limit is five attempts.",
      "The retry limit is six attempts.",
    );
    const report = await audit(f, {
      text: edited,
      judge: unavailable(),
    });
    expect(rowFor(report, "The retry limit is six attempts.")).toMatchObject({
      claimId: f.codeClaim,
      kind: "code",
    });
  });

  test("duplicateOf is asked for covered rows only", async () => {
    const f = await fixture();
    const asked: string[] = [];
    const report = await audit(f, {
      duplicateOf: (id) => {
        asked.push(id);
        return id === f.codeClaim ? f.attestedClaim : null;
      },
    });
    expect(asked.sort()).toEqual([f.codeClaim, f.attestedClaim].sort());
    expect(
      rowFor(report, "The retry limit is five attempts.").duplicateOf,
    ).toBe(f.attestedClaim);
    expect(
      rowFor(report, "Payments settle within two days.").duplicateOf,
    ).toBeNull();
    expect(report.rows.filter((r) => r.duplicateOf !== null)).toHaveLength(1);
  });
});

describe("triage kind mapping (SPEC 8.3)", () => {
  test("every confident triage answer maps to its audit kind", async () => {
    const f = await fixture();
    const report = await audit(f);
    const kinds = Object.fromEntries(
      report.rows
        .filter((r) => r.claimId === null)
        .map((r) => [r.region.text, r.kind]),
    );
    expect(kinds).toEqual({
      "# Billing": "not-a-claim",
      "Refund rules are described in the billing guide.": "reference",
      "Pro costs $29/month.": "attested",
      "The CLI exits with code 2 on a gating claim.": "code",
      "Customers love the product.": "unverifiable",
      "Always run the tests first.": "not-a-claim",
      "Our support is the best.": "not-a-claim",
      "This page covers billing.": "not-a-claim",
      "It might be fast.": "unsure",
      "It handles most traffic.": "unsure",
      [FENCE]: "code",
    });
    expect(rowFor(report, "Pro costs $29/month.")).toEqual({
      region: {
        id: "r4",
        line: 5,
        kind: "sentence",
        text: "Pro costs $29/month.",
      },
      claimId: null,
      kind: "attested",
      probability: 0.97,
      duplicateOf: null,
    });
    // A fenced region is code by construction: no evidence question is asked.
    expect(rowFor(report, FENCE)).toMatchObject({
      kind: "code",
      probability: 0.97,
      region: { kind: "fenced", line: 12 },
    });
  });

  test("below the threshold the row is unsure and carries the probabilities, not a probability", async () => {
    const f = await fixture();
    const report = await audit(f);
    expect(rowFor(report, "It might be fast.")).toEqual({
      region: {
        id: "r10",
        line: 10,
        kind: "sentence",
        text: "It might be fast.",
      },
      claimId: null,
      kind: "unsure",
      unsure: "kind",
      probabilities: { kind: UNSURE_KIND },
      duplicateOf: null,
    });
    expect(rowFor(report, "It handles most traffic.")).toMatchObject({
      kind: "unsure",
      unsure: "kind",
      probabilities: { kind: UNSURE_FACT, evidenceKind: UNSURE_FACT_EVIDENCE },
    });
    expect(
      rowFor(report, "It handles most traffic.").probability,
    ).toBeUndefined();
  });

  test("the threshold is the only knob: confidence equal to it decides the row", async () => {
    const f = await fixture();
    const config = {
      ...f.config,
      judge: { ...f.config.judge, threshold: choiceConfidence(UNSURE_FACT) },
    };
    const report = await audit(f, { config });
    expect(rowFor(report, "It handles most traffic.")).toMatchObject({
      kind: "attested",
      probability: 0.6,
    });
    // Lowered further, the opinion-leaning region decides too.
    const low = {
      ...f.config,
      judge: { ...f.config.judge, threshold: 0 },
    };
    expect(
      rowFor(await audit(f, { config: low }), "It might be fast."),
    ).toMatchObject({
      kind: "not-a-claim",
      probability: 0.5,
    });
  });
});

describe("evidence-kind confidence", () => {
  const TEXT = "Deploys take ten minutes.\n";
  const at = (threshold: number, f: Fixture) =>
    audit(f, {
      text: TEXT,
      config: { ...f.config, judge: { ...f.config.judge, threshold } },
    }).then((r) => r.rows[0]);

  test("a confident factual_claim with a split evidence kind is unsure on the evidence kind, with both maps", async () => {
    const f = await fixture();
    const row = await audit(f, { text: TEXT }).then((r) => r.rows[0]);
    expect(row).toEqual({
      region: {
        id: "r0",
        line: 1,
        kind: "sentence",
        text: "Deploys take ten minutes.",
      },
      claimId: null,
      kind: "unsure",
      unsure: "evidenceKind",
      probabilities: {
        kind: sure(TRIAGE_KINDS, "factual_claim"),
        evidenceKind: SPLIT_EVIDENCE,
      },
      duplicateOf: null,
    });
    expect(adviceFor(row as AuditRow)).toBe("unsure: evidence kind");
    expect(
      renderAudit(await audit(f, { text: TEXT }), makeStyle(false), MODE),
    ).toContain("unsure: evidence kind");
  });

  test("boundary: evidence confidence equal to the threshold decides; just above it does not", async () => {
    const f = await fixture();
    const edge = choiceConfidence(SPLIT_EVIDENCE);
    expect(edge).toBeCloseTo(0.6, 10);
    expect(await at(edge, f)).toMatchObject({
      kind: "attested",
      probability: 0.97,
    });
    expect(await at(edge + 1e-9, f)).toMatchObject({
      kind: "unsure",
      unsure: "evidenceKind",
    });
  });
});

describe("judge unavailable", () => {
  test("uncovered rows are pending with no probability; covered rows and the report still render", async () => {
    const f = await fixture();
    const report = await audit(f, { judge: unavailable() });
    const uncovered = report.rows.filter((r) => r.claimId === null);
    expect(uncovered.every((r) => r.kind === "pending")).toBe(true);
    expect(uncovered.every((r) => r.probability === undefined)).toBe(true);
    expect(uncovered.every((r) => r.probabilities === undefined)).toBe(true);
    expect(
      uncovered.every((r) => r.reason?.startsWith("judge unavailable: ")),
    ).toBe(true);
    expect(report.rows.filter((r) => r.claimId !== null)).toSatisfy((rows) =>
      rows.every((r) => r.reason === undefined),
    );
    expect(report.summary.byKind.pending).toBe(11);
    expect(report.summary.covered).toBe(2);
    expect(report.summary.requests).toBe(0);
    expect(await f.engine.store.judgeCache().list()).toEqual([]);
    expect(renderAudit(report, makeStyle(false), MODE)).toContain(
      "pending: judge unavailable",
    );
  });
});

describe("judge error", () => {
  test("an unusable answer leaves uncovered rows pending and never aborts the audit", async () => {
    const f = await fixture();
    const judge = new FakeJudge();
    judge.setError("TypeSafe returned HTTP 400: bad");
    const report = await audit(f, { judge });
    const uncovered = report.rows.filter((r) => r.claimId === null);
    expect(uncovered.every((r) => r.kind === "pending")).toBe(true);
    expect(
      uncovered.every(
        (r) => r.reason === "judge error: TypeSafe returned HTTP 400: bad",
      ),
    ).toBe(true);
    expect(report.summary.covered).toBe(2);
    expect(await f.engine.store.judgeCache().list()).toEqual([]);
    const out = renderAudit(report, makeStyle(false), MODE);
    expect(out).toContain("pending: judge error");
    expect(out).not.toContain("pending: judge unavailable");
  });
});

describe("judge cache", () => {
  test("the second run over the same text asks the judge nothing and returns the same rows", async () => {
    const f = await fixture();
    const first = scriptedJev(f.config.judge.model);
    const a = await audit(f, { judge: first.judge });
    expect(first.requests).toHaveLength(1);
    expect(a.summary.requests).toBe(1);

    const second = scriptedJev(f.config.judge.model);
    const b = await audit(f, { judge: second.judge });
    expect(second.requests).toHaveLength(0);
    expect(b.summary.requests).toBe(0);
    expect(b.rows).toEqual(a.rows);

    // Once cached, the judge's availability no longer matters.
    expect((await audit(f, { judge: unavailable() })).rows).toEqual(a.rows);
  });

  test("entries are triage verdicts about the document, keyed by the exact request", async () => {
    const f = await fixture();
    await audit(f);
    const entries = await f.engine.store.judgeCache().list();
    expect(entries).toHaveLength(1);
    const entry = entries[0];
    const report = await audit(f);
    const uncovered = report.rows
      .filter((r) => r.claimId === null)
      .map((r) => r.region.id);
    const regions = splitRegions(DOC).filter((r) => uncovered.includes(r.id));
    const [request] = buildTriageRequests(
      f.config.judge.model,
      { path: DOC_PATH },
      regions,
    );
    expect(entry).toMatchObject({
      key: f.engine.store.judgeCache().key(f.config.judge.model, request),
      model: f.config.judge.model,
      judgment: "triage",
      subject: { docId: (await f.engine.store.allClaims())[0]?.documentId },
      request,
      at: "2026-10-22T00:00:00.000Z",
    });
  });

  test("editing an uncovered region sends its chunk to the judge again", async () => {
    const f = await fixture();
    await audit(f);
    const again = scriptedJev(f.config.judge.model);
    const edited = DOC.replace(
      "Customers love the product.",
      "Always run the tests first.",
    );
    await audit(f, { judge: again.judge, text: edited });
    expect(again.requests).toHaveLength(1);
  });
});

describe("ignore globs", () => {
  test("a document matching config.ignore is refused with the glob named", async () => {
    const f = await fixture();
    const config = { ...f.config, ignore: ["docs/generated/**", "docs/*.md"] };
    expect(ignoredBy(config, "docs/generated/api.md")).toBe(
      "docs/generated/**",
    );
    expect(ignoredBy(config, "README.md")).toBeNull();
    await expect(audit(f, { config })).rejects.toThrow(
      'docs/billing.md matches the ignore glob "docs/*.md"',
    );
  });
});

describe("--fail-uncovered", () => {
  function row(kind: AuditRow["kind"], claimId: string | null): AuditRow {
    return {
      region: { id: "r0", line: 1, kind: "sentence", text: "x" },
      claimId,
      kind,
      duplicateOf: null,
    };
  }
  function report(rows: AuditRow[]): AuditReport {
    return {
      doc: "d.md",
      rows,
      summary: {
        regions: rows.length,
        covered: 0,
        uncovered: 0,
        byKind: {
          code: 0,
          reference: 0,
          attested: 0,
          unverifiable: 0,
          "not-a-claim": 0,
          unsure: 0,
          pending: 0,
        },
        requests: 0,
      },
    };
  }

  test("exit 2 only for an uncovered code, reference, or attested row", () => {
    for (const kind of ["code", "reference", "attested"] as const) {
      expect(
        auditExitCode(report([row(kind, null)]), { failUncovered: true }),
      ).toBe(2);
      expect(auditExitCode(report([row(kind, null)]))).toBe(0);
      expect(
        auditExitCode(report([row(kind, "clm_0000000000000001")]), {
          failUncovered: true,
        }),
      ).toBe(0);
    }
    for (const kind of [
      "unverifiable",
      "not-a-claim",
      "unsure",
      "pending",
    ] as const) {
      expect(
        auditExitCode(report([row(kind, null)]), { failUncovered: true }),
      ).toBe(0);
    }
  });
});

describe("renderer", () => {
  test("adviceFor is the fixed to-do mapping", () => {
    const base = {
      region: { id: "r0", line: 1, kind: "sentence" as const, text: "x" },
      duplicateOf: null,
    };
    const advice = (kind: AuditRow["kind"], claimId: string | null = null) =>
      adviceFor({ ...base, kind, claimId });
    expect(advice("code")).toBe("record");
    expect(advice("reference")).toBe("record");
    expect(advice("attested")).toBe("record");
    expect(advice("unverifiable")).toBe(
      "rewrite as an attributed opinion, or remove",
    );
    expect(advice("not-a-claim")).toBeNull();
    expect(advice("unsure")).toBe("unsure: read it yourself");
    expect(
      adviceFor({ ...base, kind: "unsure", unsure: "kind", claimId: null }),
    ).toBe("unsure: read it yourself");
    expect(
      adviceFor({
        ...base,
        kind: "unsure",
        unsure: "evidenceKind",
        claimId: null,
      }),
    ).toBe("unsure: evidence kind");
    expect(advice("pending")).toBe("pending: judge unavailable");
    for (const kind of ["code", "reference", "attested"] as const) {
      expect(advice(kind, "clm_0000000000000001")).toBeNull();
    }
  });

  test("one line per region with line, kind, probability, and to-do, then the summary", async () => {
    const f = await fixture();
    const out = renderAudit(await audit(f), makeStyle(false), MODE);
    const lines = out.trimEnd().split("\n");
    expect(lines[0]).toBe(DOC_PATH);
    expect(lines).toHaveLength(1 + 13 + 1);
    expect(lines.find((l) => l.includes("Pro costs"))).toMatch(
      /^ {2}L5 +attested +0\.97 +record +"Pro costs \$29\/month\."$/,
    );
    expect(lines.find((l) => l.includes("Customers love"))).toContain(
      "rewrite as an attributed opinion, or remove",
    );
    expect(lines.find((l) => l.includes("It might be fast"))).toContain(
      "unsure: read it yourself",
    );
    const covered = lines.find((l) => l.includes("retry limit")) ?? "";
    expect(covered).toContain(f.codeClaim);
    expect(covered).not.toContain("record");
    expect(lines.at(-1)).toBe(
      "13 regions, 2 covered, 11 uncovered; code 3, reference 1, attested 2, unverifiable 1, not-a-claim 4, unsure 2; 1 judge request",
    );
    // The kind column fits `unverifiable`: the probability column lines up.
    const probAt = (needle: string) => {
      const line = lines.find((l) => l.includes(needle)) ?? "";
      return line.search(/\d\.\d\d/);
    };
    expect(probAt("Customers love")).toBe(probAt("Pro costs"));
  });
});

const MODE: OutputMode = {
  kind: "rich",
  color: false,
  unicode: false,
  explain: false,
  hints: true,
};

// ── CLI ──────────────────────────────────────────────────────────────────────

async function cli(cwd: string, args: string[]) {
  const env: Record<string, string | undefined> = { ...process.env };
  delete env.TYPESAFE_API_KEY;
  const proc = Bun.spawn(["bun", "run", CLI, ...args], {
    cwd,
    env,
    stdout: "pipe",
    stderr: "pipe",
  });
  const stdout = await new Response(proc.stdout).text();
  const code = await proc.exited;
  return { code, json: JSON.parse(stdout.trim()) as Record<string, unknown> };
}

describe("hibi audit", () => {
  test("serves cached verdicts, exits 2 under --fail-uncovered, and 1 for a missing or ignored doc", async () => {
    const f = await fixture();
    // Seed the cache in-process; the CLI has no API key and must not need one.
    await (
      await Engine.open(f.root, {
        judge: scriptedJev(f.config.judge.model).judge,
      })
    ).audit(DOC_PATH);

    const plain = await cli(f.root, ["audit", "--doc", DOC_PATH]);
    expect(plain.code).toBe(0);
    expect(plain.json).toMatchObject({
      ok: true,
      action: "audit",
      doc: DOC_PATH,
    });
    expect((plain.json.summary as { requests: number }).requests).toBe(0);
    expect((plain.json.rows as AuditRow[])[4]).toEqual({
      region: {
        id: "r4",
        line: 5,
        kind: "sentence",
        text: "Pro costs $29/month.",
      },
      claimId: null,
      kind: "attested",
      probability: 0.97,
      duplicateOf: null,
    });

    const failing = await cli(f.root, [
      "audit",
      "--doc",
      DOC_PATH,
      "--fail-uncovered",
    ]);
    expect(failing.code).toBe(2);

    const missing = await cli(f.root, ["audit", "--doc", "nope.md"]);
    expect(missing.code).toBe(1);
    expect(String(missing.json.error)).toContain("not found");

    const configPath = join(f.root, ".claims", "config.json");
    const config = JSON.parse(await readFile(configPath, "utf8"));
    await writeFile(
      configPath,
      JSON.stringify({ ...config, ignore: ["docs/**"] }),
    );
    const ignored = await cli(f.root, ["audit", "--doc", DOC_PATH]);
    expect(ignored.code).toBe(1);
    expect(String(ignored.json.error)).toContain(
      'matches the ignore glob "docs/**"',
    );
  });

  test("without an API key and without a cache the rows are pending and --fail-uncovered passes", async () => {
    const f = await fixture();
    const res = await cli(f.root, [
      "audit",
      "--doc",
      DOC_PATH,
      "--fail-uncovered",
    ]);
    expect(res.code).toBe(0);
    const summary = res.json.summary as AuditReport["summary"];
    expect(summary.byKind.pending).toBe(11);
    expect(summary.covered).toBe(2);
  });
});
