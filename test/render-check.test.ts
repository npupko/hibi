import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { renderCheck } from "../src/cli/render/check.ts";
import { fileReader } from "../src/cli/render/helpers.ts";
import { resolveMode } from "../src/cli/render/mode.ts";
import { duplicateCount, renderOverview } from "../src/cli/render/status.ts";
import { makeStyle } from "../src/cli/render/style.ts";
import { runCheck } from "../src/engine/check.ts";
import type { CheckReport } from "../src/index.ts";
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
async function ctx(r: TempRepo, report: CheckReport, kind: "rich" | "compact") {
  const mode = resolveMode(
    { color: "never", format: kind === "compact" ? "compact" : "human" },
    { isTTY: false },
  );
  const [assertions, propositions] = await Promise.all([
    r.store.allAssertions(),
    r.store.allPropositions(),
  ]);
  return {
    report,
    assertionsById: new Map(assertions.map((a) => [a.id, a])),
    propsById: new Map(propositions.map((p) => [p.id, p])),
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
    expect(text).toContain("code:changed");
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
    expect(text).toContain("code:changed");
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
    const [assertions, propositions] = await Promise.all([
      r.store.allAssertions(),
      r.store.allPropositions(),
    ]);
    const text = renderOverview({
      report,
      assertions,
      propositions,
      storeVersion: "v3",
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
    expect(text).toContain("Store v3, 0 duplicate propositions.");
    expect(duplicateCount(assertions, propositions)).toBe(0);
  });
});
