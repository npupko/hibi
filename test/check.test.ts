import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { textSimilarity } from "../src/algo/normalize.ts";
import { SAME_TEXT_SIMILARITY } from "../src/algo/params.ts";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { projectCheckReport } from "../src/cli/render/json.ts";
import { type GateMode, gateVerdict } from "../src/core/gating.ts";
import type {
  Attestation,
  Claim,
  ClaimKind,
  EvidenceState,
  JudgeState,
} from "../src/core/model.ts";
import { computeExitCode, runCheck } from "../src/engine/check.ts";
import type { ConflictPassResult } from "../src/engine/conflict.ts";
import { recordClaim } from "../src/engine/record.ts";
import { retire } from "../src/engine/retire.ts";
import { cacheKey } from "../src/judge/cache.ts";
import { FakeJudge, supportsAnswer } from "../src/judge/fake.ts";
import { JevJudge } from "../src/judge/jev.ts";
import { makeRepo, record, type TempRepo } from "./helpers.ts";

let analyzer: Awaited<ReturnType<typeof getAnalyzer>>;
beforeAll(async () => {
  analyzer = await getAnalyzer();
});

let repos: TempRepo[] = [];
async function repo() {
  const r = await makeRepo();
  repos.push(r);
  return r;
}
afterEach(async () => {
  await Promise.all(repos.map((r) => r.cleanup()));
  repos = [];
});

const check = (r: TempRepo, opts = {}) =>
  runCheck(r.store, { ast: analyzer, ...opts });

describe("exit-code contract", () => {
  test("0 when all flags are clear", () => {
    expect(computeExitCode({ gating: false, warn: false }, "gating")).toBe(0);
  });
  test("2 when a gating verdict is present", () => {
    expect(computeExitCode({ gating: true, warn: false }, "gating")).toBe(2);
  });
  test("0 when warn-only under the default failOn=gating", () => {
    expect(computeExitCode({ gating: false, warn: true }, "gating")).toBe(0);
  });
  test("--fail-on warn escalates a warn-only result to 2", () => {
    expect(computeExitCode({ gating: false, warn: true }, "warn")).toBe(2);
    expect(computeExitCode({ gating: true, warn: false }, "warn")).toBe(2);
  });
  test("--fail-on never always exits 0", () => {
    expect(computeExitCode({ gating: true, warn: true }, "never")).toBe(0);
  });
});

describe("end-to-end drift detection (two-axis)", () => {
  test("an unchanged repo is clean (exit 0, evidence:unchanged)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    const rep = await check(r);
    expect(rep.verdicts[0]?.doc).toBe("unchanged");
    expect(rep.verdicts[0]?.evidence).toBe("unchanged");
    expect(rep.verdicts[0]?.expired).toBe(false);
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.summary.clean).toBe(1);
    expect(rep.summary.retired).toBe(0);
    expect(rep.exitCode).toBe(0);
  });

  test("a changed value (5 to 50) on an enforced claim gates (exit 2)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r);
    expect(rep.verdicts[0]?.evidence).toBe("changed");
    expect(rep.verdicts[0]?.gates).toBe(true);
    expect(rep.exitCode).toBe(2);
  });

  test("the same drift on a suggested claim does not gate (exit 0)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      enforcement: "suggested",
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r);
    // The drift still resolves to evidence:changed; only enforcement decides gating.
    expect(rep.verdicts[0]?.evidence).toBe("changed");
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.exitCode).toBe(0);
  });

  test("a retired claim is listed but counted neither clean nor gating", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      enforcement: "retired",
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r);
    expect(rep.summary.total).toBe(1);
    expect(rep.summary.retired).toBe(1);
    expect(rep.summary.clean).toBe(0);
    expect(rep.summary.gating).toBe(0);
    expect(rep.exitCode).toBe(0);
  });

  test("--fail-on never reports a changed claim without failing (exit 0)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r, { failOn: "never" });
    expect(rep.verdicts[0]?.evidence).toBe("changed");
    expect(rep.exitCode).toBe(0);
  });

  test("a deleted anchored file grades evidence:orphaned and gates on enforced (exit 2)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    const { rm } = await import("node:fs/promises");
    await rm(`${r.root}/src/retry.ts`);
    const rep = await check(r);
    expect(rep.verdicts[0]?.evidence).toBe("orphaned");
    expect(rep.verdicts[0]?.gates).toBe(true);
    expect(rep.exitCode).toBe(2);
  });

  test("a relocated documented sentence grades doc:moved, warns, and exits 0", async () => {
    const r = await repo();
    await r.write("src/x.ts", "export const MAX = 5;\n");
    await r.write(
      "README.md",
      "# Doc\n\nThe limit is five widgets per batch.\n",
    );
    await record(r, {
      doc: "README.md",
      text: "The limit is five widgets per batch",
      file: "src/x.ts",
      quote: "MAX = 5",
    });
    await r.write(
      "README.md",
      `# Doc\n\n${"filler line\n".repeat(40)}The limit is five widgets per batch.\n`,
    );
    const rep = await check(r);
    expect(rep.verdicts[0]?.doc).toBe("moved");
    expect(rep.verdicts[0]?.evidence).toBe("unchanged");
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.summary.warning).toBe(1);
    expect(rep.exitCode).toBe(0);
    expect((await check(r, { failOn: "warn" })).exitCode).toBe(2);
  });

  test("check --doc scopes the verdicts to one document", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("src/b.ts", "export const B = 2;\n");
    await record(r, {
      doc: "a.md",
      text: "A is one here",
      file: "src/a.ts",
      quote: "A = 1",
    });
    await record(r, {
      doc: "b.md",
      text: "B is two here",
      file: "src/b.ts",
      quote: "B = 2",
    });
    const { documentIdForPath } = await import("../src/engine/record.ts");
    const { documentScope } = await import("../src/engine/status.ts");
    const scope = await documentScope(r.store, "a.md");
    expect([...scope.files].sort()).toEqual(["a.md", "src/a.ts"]);
    const rep = await check(r, {
      onlyFiles: scope.files,
      onlyDocument: scope.documentId,
    });
    expect(rep.verdicts).toHaveLength(1);
    expect(rep.verdicts[0]?.documentId).toBe(documentIdForPath("a.md"));
    expect(rep.documents.map((d) => d.path)).toEqual(["a.md"]);
  });
});

describe("check is fully offline: no git on the verdict path", () => {
  test("verdicts are identical whether or not a git repo exists", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    // The temp dir is not a git repo; check must still succeed deterministically.
    const a = await check(r);
    const b = await check(r);
    expect(a.verdicts[0]?.evidence).toBe("unchanged");
    expect(a.verdicts[0]?.evidence).toBe(b.verdicts[0]?.evidence);
    expect(a.verdicts[0]?.detail.similarity).toBe(
      b.verdicts[0]?.detail.similarity,
    );
    expect(a.ref).toBe("WORKTREE");
  });
});

// ── v4: gating table, judge axis, expiry, conflicts, worklist ────────────────

describe("gating table (SPEC 7.4), every row in both modes", () => {
  const clean = { doc: "unchanged" as const, expired: false };
  const row = (
    over: Partial<{
      doc: EvidenceState;
      evidence: EvidenceState;
      judge: JudgeState;
      expired: boolean;
      conflicts: { claimId: string; probability: number; reason: string }[];
    }>,
    mode: GateMode,
    enforcement: Claim["enforcement"] = "enforced",
  ) => {
    const { gates, warns } = gateVerdict(
      { ...clean, ...over } as Parameters<typeof gateVerdict>[0],
      enforcement,
      { mode, threshold: 0.8 },
    );
    return gates ? "gate" : warns ? "warn" : "clean";
  };
  const conflict = (p: number) => [
    { claimId: "clm_b", probability: p, reason: "29 vs 39" },
  ];

  for (const mode of ["store", "doc"] as const) {
    test(`${mode}: changed / orphaned / ambiguous on either side gate`, () => {
      for (const s of ["changed", "orphaned", "ambiguous"] as const) {
        expect(row({ doc: s }, mode)).toBe("gate");
        expect(row({ evidence: s }, mode)).toBe("gate");
      }
    });
    test(`${mode}: judge refuted gates`, () => {
      expect(row({ judge: "refuted" }, mode)).toBe("gate");
    });
    test(`${mode}: a conflict at or above threshold gates; below warns`, () => {
      expect(row({ conflicts: conflict(0.8) }, mode)).toBe("gate");
      expect(row({ conflicts: conflict(0.79) }, mode)).toBe("warn");
    });
    test(`${mode}: expired ${mode === "doc" ? "gates" : "warns"}`, () => {
      expect(row({ expired: true }, mode)).toBe(
        mode === "doc" ? "gate" : "warn",
      );
    });
    test(`${mode}: moved, review, pending, unavailable warn`, () => {
      expect(row({ doc: "moved" }, mode)).toBe("warn");
      expect(row({ evidence: "moved" }, mode)).toBe("warn");
      expect(row({ judge: "review" }, mode)).toBe("warn");
      expect(row({ judge: "pending" }, mode)).toBe("warn");
      expect(row({ evidence: "unavailable" }, mode)).toBe("warn");
    });
    test(`${mode}: supported, unchanged, and no axes are clean`, () => {
      expect(row({ judge: "supported", evidence: "unchanged" }, mode)).toBe(
        "clean",
      );
      expect(row({}, mode)).toBe("clean");
    });
    test(`${mode}: suggested and retired never gate or warn, and say so`, () => {
      for (const e of ["suggested", "retired"] as const) {
        const all = {
          doc: "changed" as const,
          judge: "refuted" as const,
          expired: true,
          conflicts: conflict(0.99),
        };
        expect(row(all, mode, e)).toBe("clean");
        expect(row({ doc: "moved" }, mode, e)).toBe("clean");
        const g = gateVerdict({ ...clean, doc: "moved" }, e, { mode });
        expect(g.reasons).toContain(`${e} claim: never gates or warns`);
      }
    });
  }

  test("a gating verdict does not also warn; rules carry labels", () => {
    const g = gateVerdict(
      {
        doc: "changed",
        evidence: "moved",
        expired: true,
        conflicts: conflict(0.5),
      },
      "enforced",
      { mode: "store", threshold: 0.8 },
    );
    expect(g).toEqual({
      gates: true,
      warns: false,
      reasons: [
        "possible conflict with clm_b (p=0.50 below threshold 0.8): 29 vs 39",
        "expired warns in a whole-store check and gates under check --doc",
      ],
    });
  });
});

const MODEL = "jev-1.14.0";
const NOW = new Date("2026-06-01T00:00:00Z");

async function setThreshold(r: TempRepo, threshold: number) {
  const path = join(r.store.dir, "config.json");
  const cfg = JSON.parse(await Bun.file(path).text());
  cfg.judge.threshold = threshold;
  await writeFile(path, JSON.stringify(cfg));
}

/** Record a claim of any kind. Reference claims point at a span of `docs/billing.md`. */
async function recordKind(
  r: TempRepo,
  kind: ClaimKind,
  opts: {
    doc?: string;
    sentence: string;
    ref?: { file: string; quote: string };
    ttl?: string;
    by?: string;
    at?: Date;
    enforcement?: Claim["enforcement"];
    procedure?: Claim["procedure"];
    sources?: Claim["sources"];
  },
): Promise<Claim> {
  const docPath = opts.doc ?? "docs/pricing.md";
  let docContent = "";
  try {
    docContent = await r.read(docPath);
  } catch {}
  if (!docContent.includes(opts.sentence)) {
    docContent = `${docContent}${docContent ? "\n" : "# Pricing\n\n"}${opts.sentence}\n`;
    await r.write(docPath, docContent);
  }
  const evidenceContents: Record<string, string | null> = {};
  if (opts.ref) evidenceContents[opts.ref.file] = await r.read(opts.ref.file);
  const { claim } = await recordClaim(
    r.store,
    { docContent, evidenceContents },
    {
      docPath,
      docSpec: { quote: opts.sentence },
      kind,
      evidence: opts.ref
        ? [{ file: opts.ref.file, region: { quote: opts.ref.quote } }]
        : [],
      ttl: opts.ttl,
      by: opts.by ?? "human:alice@example.com",
      owner: "human:alice@example.com",
      ref: "testref",
      enforcement: opts.enforcement,
      procedure: opts.procedure,
      sources: opts.sources,
      now: opts.at ?? new Date("2026-05-01T00:00:00Z"),
    },
  );
  return claim;
}

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

/**
 * The Jev adapter over a scripted transport, so cached answers are Jev-shaped
 * and a second run can read them back (FakeJudge caches its own result shape).
 */
function jevJudge(p: {
  supports: number;
  contradicts: number;
  says_nothing: number;
}) {
  let n = 0;
  const judge = new JevJudge({
    model: MODEL,
    http: {
      async systemOne() {
        n += 1;
        return {
          model: MODEL,
          answers: jevAnswers(p),
          usage: { input_tokens: 0, output_tokens: 0 },
        };
      },
    },
  });
  return { judge, calls: () => n };
}

/** A `supports` cache entry for an attested claim, as `hibi evidence` would store it. */
async function putEvidenceEntry(
  r: TempRepo,
  claimId: string,
  p: { supports: number; contradicts: number; says_nothing: number },
  at = "2026-05-10T00:00:00.000Z",
  summary = "Stripe lists Pro at $29",
) {
  const request = {
    model: MODEL,
    state: { claim: claimId, at },
    questions: {},
  };
  const key = cacheKey(MODEL, request);
  await r.store.judgeCache().put({
    key,
    model: MODEL,
    judgment: "supports",
    subject: { claimId },
    request,
    answers: jevAnswers(p),
    evidence: { hash: "h", source: "stripe", summary, by: "agent:opus", at },
    at,
  });
  return key;
}

const SUPPORTS = supportsAnswer({
  supports: 0.97,
  refutes: 0.02,
  does_not_address: 0.01,
});
const REFUTES = supportsAnswer({
  supports: 0.02,
  refutes: 0.97,
  does_not_address: 0.01,
});

describe("reference claims: the judge axis", () => {
  async function refRepo() {
    const r = await repo();
    await r.write(
      "docs/billing.md",
      "# Billing\n\nThe Pro plan is billed at $29 per month.\n",
    );
    const c = await recordKind(r, "reference", {
      sentence: "Pro costs $29/month.",
      ref: {
        file: "docs/billing.md",
        quote: "The Pro plan is billed at $29 per month.",
      },
    });
    return { r, c };
  }

  test("supported is clean; the verdict is cached and a second run asks nothing", async () => {
    const { r } = await refRepo();
    const { judge, calls } = jevJudge({
      supports: 0.97,
      contradicts: 0.02,
      says_nothing: 0.01,
    });
    const a = await runCheck(r.store, { judge, now: NOW });
    expect(a.verdicts[0]?.judge).toBe("supported");
    expect(a.verdicts[0]?.gates).toBe(false);
    expect(a.verdicts[0]?.warns).toBe(false);
    expect(a.worklist).toEqual([]);
    const b = await runCheck(r.store, { judge, now: NOW });
    expect(calls()).toBe(1);
    expect(b.verdicts[0]?.judge).toBe("supported");
    expect(b.verdicts[0]?.reasons).toEqual(a.verdicts[0]?.reasons);
  });

  test("refuted gates (exit 2) and recommends fixing, never reanchoring", async () => {
    const { r } = await refRepo();
    const judge = new FakeJudge({ supports: () => REFUTES });
    const rep = await runCheck(r.store, { judge, now: NOW });
    const v = rep.verdicts[0];
    expect(v?.judge).toBe("refuted");
    expect(v?.gates).toBe(true);
    expect(rep.exitCode).toBe(2);
    expect(v?.remediation?.actions.map((a) => a.id)).toEqual([
      "fix-evidence",
      "fix-claim",
    ]);
    expect(rep.documents[0]?.suspect[0]?.status).toBe("judge:refuted");
  });

  test("the threshold comes from config.judge.threshold", async () => {
    const { r } = await refRepo();
    const { judge, calls } = jevJudge({
      supports: 0.7,
      contradicts: 0.2,
      says_nothing: 0.1,
    });
    expect(
      (await runCheck(r.store, { judge, now: NOW })).verdicts[0]?.judge,
    ).toBe("review");
    await setThreshold(r, 0.5);
    // A new store handle reads the new config; the cached verdict is not re-asked.
    const { ClaimStore } = await import("../src/store/store.ts");
    const rep = await runCheck(await ClaimStore.open(r.root), {
      judge,
      now: NOW,
    });
    expect(rep.verdicts[0]?.judge).toBe("supported");
    expect(calls()).toBe(1);
  });

  test("review warns and is listed; pending warns and is listed", async () => {
    const { r, c } = await refRepo();
    const unsure = supportsAnswer({
      supports: 0.5,
      refutes: 0.3,
      does_not_address: 0.2,
    });
    const review = await runCheck(r.store, {
      judge: new FakeJudge({ supports: () => unsure }),
      now: NOW,
    });
    expect(review.verdicts[0]?.judge).toBe("review");
    expect(review.verdicts[0]?.warns).toBe(true);
    expect(review.worklist.map((w) => [w.claimId, w.reason])).toEqual([
      [c.id, "review"],
    ]);
    expect(review.verdicts[0]?.reasons).toContain(
      "worklist: the judge verdict is review",
    );

    const { r: r2, c: c2 } = await refRepo();
    const down = new FakeJudge();
    down.setUnavailable("network down");
    const pending = await runCheck(r2.store, { judge: down, now: NOW });
    expect(pending.verdicts[0]?.judge).toBe("pending");
    expect(pending.verdicts[0]?.warns).toBe(true);
    expect(pending.verdicts[0]?.reasons).toContain(
      "judge: judge unavailable: network down",
    );
    expect(pending.worklist.map((w) => [w.claimId, w.reason])).toEqual([
      [c2.id, "pending"],
    ]);
    expect(pending.exitCode).toBe(0);
  });

  test("a changed side means no judge call; the anchor state is reported", async () => {
    const { r } = await refRepo();
    await r.write(
      "docs/billing.md",
      "# Billing\n\nThe Pro plan is billed at $39 per month.\n",
    );
    const judge = new FakeJudge({ supports: () => SUPPORTS });
    const rep = await runCheck(r.store, { judge, now: NOW });
    const v = rep.verdicts[0];
    expect(judge.calls).toHaveLength(0);
    expect(v?.evidence).toBe("changed");
    expect(v?.judge).toBeUndefined();
    expect(v?.gates).toBe(true);
    expect(v?.reasons).toContain(
      "judge: not consulted while the evidence side is changed",
    );
  });

  test("a retired reference claim is never judged", async () => {
    const r = await repo();
    await r.write(
      "docs/billing.md",
      "The Pro plan is billed at $29 per month.\n",
    );
    await recordKind(r, "reference", {
      sentence: "Pro costs $29/month.",
      ref: {
        file: "docs/billing.md",
        quote: "The Pro plan is billed at $29 per month.",
      },
      enforcement: "retired",
    });
    const judge = new FakeJudge({ supports: () => SUPPORTS });
    const rep = await runCheck(r.store, { judge, now: NOW });
    expect(judge.calls).toHaveLength(0);
    expect(rep.verdicts[0]?.judge).toBeUndefined();
    expect(rep.worklist).toEqual([]);
  });
});

describe("attested claims: expiry, cached evidence, worklist", () => {
  test("a fresh pure vouch (no procedure, no sources) is clean and not in the worklist", async () => {
    const r = await repo();
    await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P90D",
    });
    const rep = await runCheck(r.store, { now: NOW });
    const v = rep.verdicts[0];
    expect(v?.judge).toBeUndefined();
    expect(v?.expired).toBe(false);
    expect(v?.warns).toBe(false);
    expect(rep.worklist).toEqual([]);
  });

  test("a fresh human vouch with a source and no evidence is clean but never-checked", async () => {
    const r = await repo();
    const c = await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P90D",
      sources: [{ id: "stripe", resource: "https://stripe.example/p" }],
    });
    const rep = await runCheck(r.store, { now: NOW });
    const v = rep.verdicts[0];
    expect(v?.warns).toBe(false);
    expect(rep.worklist.map((w) => [w.claimId, w.reason])).toEqual([
      [c.id, "never-checked"],
    ]);
    expect(v?.reasons).toContain(
      "worklist: never checked; no evidence has been judged for this claim",
    );
  });

  test("an agent vouch does not count: expired, warns in check, gates under --doc", async () => {
    const r = await repo();
    const c = await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P90D",
      by: "agent:opus",
    });
    const rep = await runCheck(r.store, { now: NOW });
    expect(rep.verdicts[0]?.expired).toBe(true);
    expect(rep.verdicts[0]?.warns).toBe(true);
    expect(rep.exitCode).toBe(0);
    expect(rep.worklist[0]?.reason).toBe("expired");
    const scoped = await runCheck(r.store, {
      now: NOW,
      onlyDocument: c.documentId,
    });
    expect(scoped.verdicts[0]?.gates).toBe(true);
    expect(scoped.exitCode).toBe(2);
    expect(scoped.verdicts[0]?.reasons).toContain(
      "expired gates under check --doc",
    );
  });

  test("an elapsed ttl expires; judged evidence resets it and shows as last evidence", async () => {
    const r = await repo();
    const c = await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P10D",
      at: new Date("2026-05-01T00:00:00Z"),
    });
    const stale = await runCheck(r.store, { now: NOW });
    expect(stale.verdicts[0]?.expired).toBe(true);
    expect(stale.worklist[0]?.reason).toBe("expired");

    await putEvidenceEntry(
      r,
      c.id,
      { supports: 0.97, contradicts: 0.02, says_nothing: 0.01 },
      "2026-05-28T00:00:00.000Z",
    );
    const stored = await r.store.getClaim(c.id);
    if (!stored) throw new Error("claim missing");
    const evidence: Attestation = {
      by: "agent:opus",
      at: "2026-05-28T00:00:00.000Z",
      basis: "evidence",
      evidenceHash: "h",
    };
    await r.store.putClaim({
      ...stored,
      attestations: [...stored.attestations, evidence],
    });
    const fresh = await runCheck(r.store, { now: NOW });
    expect(fresh.verdicts[0]?.expired).toBe(false);
    expect(fresh.verdicts[0]?.judge).toBe("supported");
    expect(fresh.worklist).toEqual([]);
  });

  test("the newest cached verdict decides; refuted gates; review warns and lists last evidence", async () => {
    const r = await repo();
    const c = await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P90D",
    });
    await putEvidenceEntry(
      r,
      c.id,
      { supports: 0.97, contradicts: 0.02, says_nothing: 0.01 },
      "2026-05-02T00:00:00.000Z",
    );
    await putEvidenceEntry(
      r,
      c.id,
      { supports: 0.02, contradicts: 0.97, says_nothing: 0.01 },
      "2026-05-03T00:00:00.000Z",
      "Stripe lists Pro at $39",
    );
    const refuted = await runCheck(r.store, { now: NOW });
    expect(refuted.verdicts[0]?.judge).toBe("refuted");
    expect(refuted.verdicts[0]?.gates).toBe(true);
    expect(refuted.worklist).toEqual([]);

    const key = await putEvidenceEntry(
      r,
      c.id,
      { supports: 0.4, contradicts: 0.3, says_nothing: 0.3 },
      "2026-05-04T00:00:00.000Z",
      "dashboard was ambiguous",
    );
    const review = await runCheck(r.store, { now: NOW });
    expect(review.verdicts[0]?.judge).toBe("review");
    expect(review.verdicts[0]?.warns).toBe(true);
    expect(review.worklist).toEqual([
      {
        claimId: c.id,
        sentence: "Pro costs $29/month.",
        kind: "attested",
        document: "docs/pricing.md",
        procedure: null,
        sources: [],
        owner: "human:alice@example.com",
        lastEvidence: {
          summary: "dashboard was ambiguous",
          source: "stripe",
          at: "2026-05-04T00:00:00.000Z",
          by: "agent:opus",
        },
        reason: "review",
      },
    ]);

    // An ack naming that verdict stops the warning and the listing.
    const stored = await r.store.getClaim(c.id);
    if (!stored) throw new Error("claim missing");
    const ack = {
      by: "human:alice@example.com",
      at: "2026-05-05T00:00:00.000Z",
      basis: "ack",
      note: "the dashboard is the source; fine",
      ack: { judgeKey: key },
    } as Attestation;
    await r.store.putClaim({
      ...stored,
      attestations: [...stored.attestations, ack],
    });
    const acked = await runCheck(r.store, { now: NOW });
    expect(acked.verdicts[0]?.judge).toBe("review");
    expect(acked.verdicts[0]?.warns).toBe(false);
    expect(acked.worklist).toEqual([]);
    expect(acked.verdicts[0]?.reasons).toContain(
      "review acknowledged by human:alice@example.com at 2026-05-05T00:00:00.000Z: the dashboard is the source; fine; it no longer warns",
    );
  });

  test("a suggested attested claim is listed but never warns; a retired one is not listed", async () => {
    const r = await repo();
    await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P90D",
      by: "agent:x",
      enforcement: "suggested",
    });
    await recordKind(r, "attested", {
      sentence: "Team costs $99/month.",
      ttl: "P90D",
      by: "agent:x",
      enforcement: "retired",
    });
    const rep = await runCheck(r.store, { now: NOW });
    expect(rep.summary.warning).toBe(0);
    expect(rep.worklist.map((w) => w.reason)).toEqual(["expired"]);
  });
});

describe("conflicts from the contradiction pass", () => {
  test("at or above threshold gates and names the other claim; below warns", async () => {
    const r = await repo();
    const a = await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P90D",
    });
    const b = await recordKind(r, "attested", {
      doc: "docs/faq.md",
      sentence: "Pro costs $39/month.",
      ttl: "P90D",
    });
    const pairId = [a.id, b.id].sort().join(":");
    const conflictPass: ConflictPassResult = {
      findings: [
        {
          claimId: a.id,
          otherClaimId: b.id,
          pairId,
          probability: 1,
          reason: "numbers differ: 29 vs 39",
        },
        {
          claimId: b.id,
          otherClaimId: a.id,
          pairId,
          probability: 0.6,
          reason: "review: contradictory below threshold",
          review: true,
        },
      ],
      duplicates: [],
      pending: [],
      requests: 0,
      pairs: [],
      scope: "store",
    };
    const rep = await runCheck(r.store, { now: NOW, conflictPass });
    const va = rep.verdicts.find((v) => v.claimId === a.id);
    const vb = rep.verdicts.find((v) => v.claimId === b.id);
    expect(va?.gates).toBe(true);
    expect(va?.remediation?.recommended).toBe("reconcile");
    expect(va?.remediation?.actions[0]?.title).toBe(`Reconcile with ${b.id}`);
    expect(va?.reasons).toContain(
      `conflicts with ${b.id} (p=1.00 at or above threshold 0.8): numbers differ: 29 vs 39`,
    );
    expect(vb?.gates).toBe(false);
    expect(vb?.warns).toBe(true);
    expect(vb?.conflicts).toEqual([]);
    expect(vb?.reasons).toContain(
      `possible conflict with ${a.id}: review: contradictory below threshold`,
    );
    expect(rep.summary.conflicts).toBe(1);
    expect(rep.exitCode).toBe(2);
  });
});

describe("check JSON shape: one claim of each kind", () => {
  test("verdicts, worklist, and summary", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write(
      "docs/billing.md",
      "The Pro plan is billed at $29 per month.\n",
    );
    const code = await record(r, {
      doc: "README.md",
      text: "Retries are capped at 5.",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    const ref = await recordKind(r, "reference", {
      sentence: "Pro costs $29/month.",
      ref: {
        file: "docs/billing.md",
        quote: "The Pro plan is billed at $29 per month.",
      },
    });
    const att = await recordKind(r, "attested", {
      doc: "docs/plans.md",
      sentence: "Team costs $99/month.",
      ttl: "P90D",
      procedure: {
        instructions: "Open the Stripe dashboard and read the Team price.",
      },
      sources: [
        {
          id: "stripe",
          resource: "https://dashboard.stripe.com/products/prod_team",
        },
      ],
    });
    const rep = await runCheck(r.store, {
      ast: analyzer,
      judge: new FakeJudge({ supports: () => SUPPORTS }),
      now: NOW,
    });
    const json = projectCheckReport(rep, { explain: false, hints: true }) as {
      verdicts: Record<string, unknown>[];
      worklist: unknown[];
      summary: unknown;
    };
    const byId = (id: string) => json.verdicts.find((v) => v.claimId === id);

    expect(byId(code.claim.id)).toEqual({
      claimId: code.claim.id,
      documentId: code.claim.documentId,
      kind: "code",
      doc: "unchanged",
      evidence: "unchanged",
      expired: false,
      conflicts: [],
      gates: false,
      warns: false,
      remediation: null,
      reasons: [],
    });
    expect(byId(ref.id)).toEqual({
      claimId: ref.id,
      documentId: ref.documentId,
      kind: "reference",
      doc: "unchanged",
      evidence: "unchanged",
      judge: "supported",
      expired: false,
      conflicts: [],
      gates: false,
      warns: false,
      remediation: null,
      reasons: [
        "judge: evidence supports the sentence (confidence 0.95 >= threshold 0.8)",
      ],
    });
    expect(byId(att.id)).toEqual({
      claimId: att.id,
      documentId: att.documentId,
      kind: "attested",
      doc: "unchanged",
      expired: false,
      conflicts: [],
      gates: false,
      warns: false,
      remediation: null,
      reasons: [
        "worklist: never checked; no evidence has been judged for this claim",
      ],
    });
    expect(json.worklist).toEqual([
      {
        claimId: att.id,
        sentence: "Team costs $99/month.",
        kind: "attested",
        document: "docs/plans.md",
        procedure: {
          instructions: "Open the Stripe dashboard and read the Team price.",
        },
        sources: [
          {
            id: "stripe",
            resource: "https://dashboard.stripe.com/products/prod_team",
          },
        ],
        owner: "human:alice@example.com",
        reason: "never-checked",
      },
    ]);
    expect(json.summary).toEqual({
      total: 3,
      gating: 0,
      warning: 0,
      clean: 3,
      retired: 0,
      doc: { unchanged: 3, moved: 0, changed: 0, ambiguous: 0, orphaned: 0 },
      evidence: {
        unchanged: 2,
        moved: 0,
        changed: 0,
        ambiguous: 0,
        orphaned: 0,
        unavailable: 0,
      },
      judge: { supported: 1, refuted: 0, review: 0, pending: 0 },
      expired: 0,
      conflicts: 0,
      worklist: 1,
      conflictPass: { pairs: 0, requests: 0, pending: 0, scope: "store" },
    });
    const explained = projectCheckReport(rep, {
      explain: true,
      hints: false,
    }) as {
      verdicts: Record<string, unknown>[];
    };
    expect(Object.keys(explained.verdicts[0] ?? {})).toContain("detail");
    expect(Object.keys(explained.verdicts[0] ?? {})).not.toContain(
      "remediation",
    );
  });
});

describe("worklist, suspects, and pending pairs", () => {
  test("a code claim with a ttl expires: listed as expired, warns, and gates under --doc", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    const c = await record(r, {
      doc: "README.md",
      text: "Retries are capped at 5.",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      ttl: "P1D",
    });
    const later = new Date("2030-01-01T00:00:00Z");
    const store = await runCheck(r.store, { ast: analyzer, now: later });
    expect(store.verdicts[0]?.expired).toBe(true);
    expect(store.verdicts[0]?.warns).toBe(true);
    expect(store.worklist.map((w) => [w.claimId, w.reason])).toEqual([
      [c.claim.id, "expired"],
    ]);
    // `hibi evidence` takes attested claims only: nothing points a code claim at it.
    expect(store.verdicts[0]?.reasons).toContain(
      `worklist: expired: re-verify, then hibi attest ${c.claim.id} --by human:<name>`,
    );
    const worklistLabels =
      store.verdicts[0]?.reasons.filter((l) => l.startsWith("worklist:")) ?? [];
    expect(worklistLabels.join(" ")).not.toContain("evidence");
    const actions = store.verdicts[0]?.remediation?.actions ?? [];
    expect(actions.map((a) => a.id)).not.toContain("evidence");
    expect(JSON.stringify(actions)).not.toContain("evidence");
    expect(store.documents[0]?.suspect).toEqual([
      { claimId: c.claim.id, status: "expired" },
    ]);
    const doc = await runCheck(r.store, {
      ast: analyzer,
      now: later,
      onlyDocument: c.claim.documentId,
    });
    expect(doc.verdicts[0]?.gates).toBe(true);
    expect(doc.exitCode).toBe(2);
  });

  test("a pending pair warns with its cause and is the suspect status; a review finding is conflict:review", async () => {
    const r = await repo();
    const a = await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P90D",
    });
    const b = await recordKind(r, "attested", {
      doc: "docs/faq.md",
      sentence: "Pro costs $39/month.",
      ttl: "P90D",
    });
    const pairId = [a.id, b.id].sort().join(":");
    const pending: ConflictPassResult = {
      findings: [],
      duplicates: [],
      pending: [pairId],
      requests: 0,
      pairs: [
        {
          pairId,
          why: "shared rare tokens: pro",
          outcome: "pending",
          reason: "judge error: TypeSafe returned HTTP 400: bad",
        },
      ],
      scope: "store",
    };
    const rep = await runCheck(r.store, { now: NOW, conflictPass: pending });
    const va = rep.verdicts.find((v) => v.claimId === a.id);
    expect(va?.warns).toBe(true);
    expect(va?.reasons).toContain("conflict checks pending: 1");
    const statuses = rep.documents.flatMap((d) => d.suspect);
    expect(statuses.map((s) => s.status)).toEqual([
      "conflict:pending",
      "conflict:pending",
    ]);
    expect(statuses.every((s) => s.status !== "unchanged")).toBe(true);

    const review: ConflictPassResult = {
      ...pending,
      pending: [],
      pairs: [],
      findings: [
        {
          claimId: a.id,
          otherClaimId: b.id,
          pairId,
          probability: 0.6,
          reason: "review: contradictory below threshold",
          review: true,
        },
        {
          claimId: b.id,
          otherClaimId: a.id,
          pairId,
          probability: 0.6,
          reason: "review: contradictory below threshold",
          review: true,
        },
      ],
    };
    const rev = await runCheck(r.store, { now: NOW, conflictPass: review });
    expect(
      rev.documents.flatMap((d) => d.suspect).map((s) => s.status),
    ).toEqual(["conflict:review", "conflict:review"]);
    expect(rev.summary.conflictPass.scope).toBe("store");
  });
});

describe("B.1 newest evidence", () => {
  test("B.1: lastEvidence and the verdict use the same newest entry, by the later of entry at and evidence at", async () => {
    const r = await repo();
    const c = await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P90D",
    });
    const review = { supports: 0.4, contradicts: 0.3, says_nothing: 0.3 };
    const a = await putEvidenceEntry(
      r,
      c.id,
      review,
      "2026-05-02T00:00:00.000Z",
      "first submission",
    );
    await putEvidenceEntry(
      r,
      c.id,
      review,
      "2026-05-03T00:00:00.000Z",
      "second submission",
    );
    // The first request is submitted again later: a cache hit rewrites its evidence block.
    const entries = await r.store.judgeCache().list({ claimId: c.id });
    const first = entries.find((e) => e.key === a);
    if (!first?.evidence) throw new Error("entry missing");
    await r.store.judgeCache().put({
      ...first,
      evidence: {
        ...first.evidence,
        summary: "resubmitted",
        at: "2026-05-06T00:00:00.000Z",
      },
    });
    const out = await runCheck(r.store, { now: NOW });
    expect(out.worklist[0]?.lastEvidence).toEqual({
      summary: "resubmitted",
      source: "stripe",
      at: "2026-05-06T00:00:00.000Z",
      by: "agent:opus",
    });
  });
});

describe("B.6 expired code and reference claims", () => {
  test("B.6: an expired reference claim's worklist label asks for a human vouch, as its remediation does", async () => {
    const r = await repo();
    await r.write(
      "docs/billing.md",
      "# Billing\n\nThe Pro plan is billed at $29 per month.\n",
    );
    const c = await recordKind(r, "reference", {
      sentence: "Pro costs $29/month.",
      ref: {
        file: "docs/billing.md",
        quote: "The Pro plan is billed at $29 per month.",
      },
      ttl: "P1D",
    });
    const out = await runCheck(r.store, { now: NOW });
    const v = out.verdicts[0];
    expect(v?.expired).toBe(true);
    expect(out.worklist.map((w) => [w.claimId, w.reason])).toEqual([
      [c.id, "expired"],
    ]);
    expect(v?.reasons).toContain(
      `worklist: expired: re-verify, then hibi attest ${c.id} --by human:<name>`,
    );
    const actions = v?.remediation?.actions ?? [];
    const attest = actions.find((a) => a.id === "attest");
    expect(attest?.command).toStartWith(`hibi attest ${c.id} --by human:`);
    expect(actions.map((a) => a.id)).not.toContain("evidence");
  });
});

describe("B.8 pending conflict checks", () => {
  test("B.8: a claim with several pending pairs gets one counted reason; the pairs keep their own", async () => {
    const r = await repo();
    const a = await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P90D",
    });
    const b = await recordKind(r, "attested", {
      sentence: "Pro is billed monthly.",
      ttl: "P90D",
    });
    const c = await recordKind(r, "attested", {
      sentence: "Pro includes priority support.",
      ttl: "P90D",
    });
    const ab = [a.id, b.id].sort().join(":");
    const ac = [a.id, c.id].sort().join(":");
    const pass: ConflictPassResult = {
      findings: [],
      duplicates: [],
      pending: [ab, ac],
      requests: 0,
      pairs: [
        {
          pairId: ab,
          why: "shared rare tokens: pro",
          outcome: "pending",
          reason: "judge unavailable: TYPESAFE_API_KEY is not set",
        },
        {
          pairId: ac,
          why: "shared rare tokens: pro",
          outcome: "pending",
          reason: "judge error: TypeSafe returned HTTP 400: bad",
        },
      ],
      scope: "store",
    };
    const rep = await runCheck(r.store, { now: NOW, conflictPass: pass });
    const reasons = (id: string) =>
      rep.verdicts.find((v) => v.claimId === id)?.reasons ?? [];
    expect(reasons(a.id).filter((l) => l.startsWith("conflict check"))).toEqual(
      ["conflict checks pending: 2"],
    );
    expect(reasons(b.id)).toContain("conflict checks pending: 1");
    expect(reasons(c.id)).toContain("conflict checks pending: 1");
    expect(
      rep.verdicts
        .flatMap((v) => v.reasons)
        .some((l) => l.startsWith("conflict check pending with")),
    ).toBe(false);
    expect(rep.summary.conflictPass?.pending).toBe(2);
    expect(rep.verdicts.find((v) => v.claimId === a.id)?.warns).toBe(true);
  });
});

describe("B.9 retired claims in the summary", () => {
  test("B.9: a retired claim counts only in retired, not in expired or the axis counts", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    const live = await record(r, {
      doc: "README.md",
      text: "Retries are capped at 5.",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    const gone = await record(r, {
      doc: "README.md",
      text: "The attempt cap is five.",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      ttl: "P1D",
    });
    await retire(r.store, gone.claim.id);
    const later = new Date("2030-01-01T00:00:00Z");
    const out = await runCheck(r.store, { ast: analyzer, now: later });
    expect(out.verdicts.map((v) => v.claimId).sort()).toEqual(
      [live.claim.id, gone.claim.id].sort(),
    );
    expect(out.verdicts.find((v) => v.claimId === gone.claim.id)?.expired).toBe(
      true,
    );
    expect(out.summary.retired).toBe(1);
    expect(out.summary.expired).toBe(0);
    expect(out.summary.doc.unchanged).toBe(1);
    expect(out.summary.evidence.unchanged).toBe(1);
    expect(out.summary.clean).toBe(1);
    expect(out.summary.total).toBe(2);
  });
});

describe("B.11 ast-node semantic hash", () => {
  test("B.11: a changed semantic hash alone makes the evidence span changed", async () => {
    const r = await repo();
    const before =
      "export function retryPolicy() {\n  return { attempts: limitOfAttemptsForTheClient, backoff: exponentialBackoffStrategy };\n}\n";
    await r.write("src/policy.ts", before);
    await record(r, {
      doc: "README.md",
      text: "The retry policy uses exponential backoff.",
      file: "src/policy.ts",
      quote: before.trimEnd(),
    });
    // One identifier character differs: the text counts as the same text, the structure is the same.
    const after = before.replace(
      "limitOfAttemptsForTheClient",
      "limitOfAttemptsForTheClients",
    );
    expect(
      textSimilarity(before.trimEnd(), after.trimEnd()),
    ).toBeGreaterThanOrEqual(SAME_TEXT_SIMILARITY);
    await r.write("src/policy.ts", after);
    const out = await check(r);
    expect(out.verdicts[0]?.evidence).toBe("changed");
    expect(out.verdicts[0]?.reasons).toContain(
      "evidence: identifiers or literals renamed",
    );
  });
});
