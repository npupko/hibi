import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { renderCheck } from "../src/cli/render/check.ts";
import { fileReader } from "../src/cli/render/helpers.ts";
import { resolveMode } from "../src/cli/render/mode.ts";
import { renderOverview } from "../src/cli/render/status.ts";
import { makeStyle } from "../src/cli/render/style.ts";
import { runCheck } from "../src/engine/check.ts";
import { recordClaim } from "../src/engine/record.ts";
import type { CheckReport } from "../src/index.ts";
import { FakeJudge } from "../src/judge/fake.ts";
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

/** Build a render context from a real store + report, with color off. */
async function ctx(
  r: TempRepo,
  report: CheckReport,
  kind: "rich" | "compact",
  explain = false,
) {
  const mode = resolveMode(
    {
      color: "never",
      format: kind === "compact" ? "compact" : "human",
      explain,
    },
    { isTTY: false },
  );
  const claims = await r.store.allClaims();
  return {
    report,
    claimsById: new Map(claims.map((c) => [c.id, c])),
    read: fileReader(r.store.anchorRoot),
    style: makeStyle(false),
    mode,
  };
}

/** A repo with one drifted enforced claim and one clean claim across two docs. */
async function driftedRepo(): Promise<TempRepo> {
  const r = await repo();
  await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
  await r.write("README.md", "# Doc\n\nCapped at 5 attempts here.\n");
  await record(r, {
    doc: "README.md",
    text: "Capped at 5 attempts",
    file: "src/retry.ts",
    quote: "MAX_ATTEMPTS = 5",
  });
  await r.write("docs/api.md", "# API\n\nReturns JSON always here.\n");
  await r.write("src/api.ts", "export const FORMAT = 'json';\n");
  await record(r, {
    doc: "docs/api.md",
    text: "Returns JSON always",
    file: "src/api.ts",
    quote: "FORMAT = 'json'",
  });
  // Drift only the retry constant: README's claim gates, api.md stays clean.
  await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
  return r;
}

describe("renderCheck: rich", () => {
  test("a gating drift produces a grouped diagnostic block, no ANSI", async () => {
    const r = await driftedRepo();
    const report = await runCheck(r.store, { ast: analyzer, ref: "WORKTREE" });
    const text = renderCheck(await ctx(r, report, "rich"));

    // biome-ignore lint/suspicious/noControlCharactersInRegex: assert no ANSI leaked.
    expect(text).not.toMatch(/\x1b\[/);
    expect(text).toContain("✖ README.md");
    expect(text).toContain("evidence:changed");
    expect(text).toContain('"Capped at 5 attempts"');
    expect(text).toContain("src/retry.ts:1");
    expect(text).toContain("help:");
    expect(text).toContain("reason:");
    // The clean doc collapses to a count, not a block.
    expect(text).toContain("✓ docs/api.md");
    expect(text).toContain("1 claim clean");
    expect(text).toMatch(/Found 1 gating, 0 warning .* exit 2/);
  });

  test("an all-clean repo collapses every doc to a count", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("doc.md", "# Doc\n\nA is one here.\n");
    await record(r, {
      doc: "doc.md",
      text: "A is one",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const report = await runCheck(r.store, { ast: analyzer, ref: "WORKTREE" });
    const text = renderCheck(await ctx(r, report, "rich"));
    expect(text).toContain("✓ doc.md");
    expect(text).toMatch(/Found 0 gating, 0 warning .* exit 0/);
    expect(text).not.toContain("help:");
  });

  test("an empty store says so", async () => {
    const r = await repo();
    const report = await runCheck(r.store, { ast: analyzer, ref: "WORKTREE" });
    const text = renderCheck(await ctx(r, report, "rich"));
    expect(text).toContain("No claims recorded");
    expect(text).toMatch(/exit 0/);
  });
});

describe("renderCheck: compact", () => {
  test("one line per suspect claim under each doc header", async () => {
    const r = await driftedRepo();
    const report = await runCheck(r.store, { ast: analyzer, ref: "WORKTREE" });
    const text = renderCheck(await ctx(r, report, "compact"));
    // biome-ignore lint/suspicious/noControlCharactersInRegex: assert no ANSI leaked.
    expect(text).not.toMatch(/\x1b\[/);
    expect(text).toContain("✖ README.md");
    expect(text).toContain("evidence:changed");
    expect(text).toContain("src/retry.ts:1");
    expect(text).not.toContain("help:");
  });
});

describe("renderOverview: repo-wide table", () => {
  test("one row per document with counts, owner, and lifecycle", async () => {
    const r = await driftedRepo();
    const report = await runCheck(r.store, { ast: analyzer, ref: "WORKTREE" });
    const mode = resolveMode(
      { color: "never", format: "human" },
      { isTTY: false },
    );
    const claims = await r.store.allClaims();
    const text = renderOverview({
      report,
      claims,
      storeVersion: "v4",
      style: makeStyle(false),
      mode,
    });
    // biome-ignore lint/suspicious/noControlCharactersInRegex: assert no ANSI leaked.
    expect(text).not.toMatch(/\x1b\[/);
    expect(text).toContain("Document");
    expect(text).toContain("README.md");
    expect(text).toContain("docs/api.md");
    expect(text).toContain("tester"); // owner column
    expect(text).toContain("active"); // lifecycle column
    // The gating doc sorts above the clean one.
    expect(text.indexOf("README.md")).toBeLessThan(text.indexOf("docs/api.md"));
    expect(text).toMatch(/Tracking 2 documents.* exit 2/);
    expect(text).toContain("Store v4.");
  });
});

describe("renderCheck: v4 tags, reasons, and worklist", () => {
  const NOW = new Date("2026-06-01T00:00:00Z");

  /** A code claim in conflict, a reference claim left pending, an expired attested claim. */
  async function v4Repo() {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write("README.md", "# Doc\n\nRetries are capped at 5.\n");
    const code = await record(r, {
      doc: "README.md",
      text: "Retries are capped at 5.",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    const pricing =
      "# Pricing\n\nPro costs $29/month.\n\nTeam costs $99/month.\n";
    const billing = "The Pro plan is billed at $29 per month.\n";
    await r.write("docs/pricing.md", pricing);
    await r.write("docs/billing.md", billing);
    const base = {
      docPath: "docs/pricing.md",
      owner: "human:alice@example.com",
      ref: "testref",
      now: new Date("2026-05-01T00:00:00Z"),
    };
    await recordClaim(
      r.store,
      { docContent: pricing, evidenceContents: { "docs/billing.md": billing } },
      {
        ...base,
        kind: "reference",
        docSpec: { quote: "Pro costs $29/month." },
        evidence: [
          {
            file: "docs/billing.md",
            region: { quote: "The Pro plan is billed at $29 per month." },
          },
        ],
        by: "human:alice@example.com",
      },
    );
    const att = await recordClaim(
      r.store,
      { docContent: pricing, evidenceContents: {} },
      {
        ...base,
        kind: "attested",
        docSpec: { quote: "Team costs $99/month." },
        evidence: [],
        ttl: "P10D",
        by: "agent:opus",
        procedure: {
          instructions: "Open the Stripe dashboard and read the Team price.",
        },
      },
    );
    const judge = new FakeJudge();
    judge.setUnavailable("no TYPESAFE_API_KEY");
    const report = await runCheck(r.store, {
      ast: analyzer,
      ref: "WORKTREE",
      judge,
      now: NOW,
      conflictPass: {
        findings: [
          {
            claimId: code.claim.id,
            otherClaimId: att.claim.id,
            pairId: [code.claim.id, att.claim.id].sort().join(":"),
            probability: 0.95,
            reason: "numbers differ",
          },
        ],
        duplicates: [],
        pending: [],
        requests: 0,
        pairs: [],
        scope: "store",
      },
    });
    return { r, report, code: code.claim, att: att.claim };
  }

  test("rich: judge and conflict tags in the status column, then the worklist", async () => {
    const { r, report, att } = await v4Repo();
    const text = renderCheck(await ctx(r, report, "rich"));
    expect(text).toContain(`conflict:${att.id}`);
    expect(text).toContain("judge:pending");
    expect(text).toContain("expired");
    expect(text).toContain("Worklist (2)");
    expect(text).toContain(`expired       ${att.id}   docs/pricing.md`);
    expect(text).toContain(
      "procedure: Open the Stripe dashboard and read the Team price.",
    );
    expect(text).toMatch(/Found 1 gating, 2 warning, 2 in worklist .* exit 2/);
    // Without --explain, judge and expiry labels stay out of the block.
    expect(text).not.toContain("reason: expired: ttl");
    expect(text.indexOf("Worklist")).toBeGreaterThan(text.indexOf("README.md"));
  });

  test("--explain prints every reason label", async () => {
    const { r, report } = await v4Repo();
    const text = renderCheck(await ctx(r, report, "rich", true));
    expect(text).toContain(
      "reason: expired: ttl P10D is set and no attestation counts (only evidence or a human vouch resets ttl)",
    );
    expect(text).toContain("reason: judge: pending: no TYPESAFE_API_KEY");
    expect(text).toMatch(
      /reason: conflicts with clm_[0-9a-f]{16} \(p=0\.95 at or above threshold 0\.8\): numbers differ/,
    );
  });

  test("compact: one line per worklist item", async () => {
    const { r, report, att } = await v4Repo();
    const text = renderCheck(await ctx(r, report, "compact"));
    const worklist = text.slice(text.indexOf("Worklist (2)"));
    const lines = worklist.split("\n").filter((l) => l.includes(att.id));
    expect(lines).toHaveLength(1);
    expect(lines[0]).toContain(`expired       ${att.id}   docs/pricing.md`);
    expect(lines[0]).toContain('"Team costs $99/month."');
  });

  test("the overview counts warnings from the verdict's warns", async () => {
    const { r, report } = await v4Repo();
    const text = renderOverview({
      report,
      claims: await r.store.allClaims(),
      storeVersion: "v4",
      style: makeStyle(false),
      mode: resolveMode({ color: "never", format: "human" }, { isTTY: false }),
    });
    expect(text).toContain("docs/pricing.md");
    expect(text).toMatch(/1 gating, 2 warning/);
  });
});
