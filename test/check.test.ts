import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
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
      "judge: pending: network down",
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
  test("a fresh human vouch with no evidence is clean but never-checked", async () => {
    const r = await repo();
    const c = await recordKind(r, "attested", {
      sentence: "Pro costs $29/month.",
      ttl: "P90D",
    });
    const rep = await runCheck(r.store, { now: NOW });
    const v = rep.verdicts[0];
    expect(v?.judge).toBeUndefined();
    expect(v?.expired).toBe(false);
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
      conflictPass: { pairs: 0, requests: 0, pending: 0 },
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
