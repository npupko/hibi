/**
 * Region splitter regressions from real documents in this repository. The
 * text of each case is copied from the file named in the test (as of commit
 * 1802f47).
 */
import { describe, expect, test } from "bun:test";
import { splitRegions } from "../src/engine/regions.ts";

const shape = (text: string, path?: string) =>
  splitRegions(text, path).map((r) => [r.kind, r.line, r.text]);

describe("splitRegions on real cases", () => {
  test("docs/audit.mdx L29-33: a numbered marker stays with its sentence", () => {
    const text = [
      "The rules, in order:",
      "",
      "1. Blank lines separate blocks.",
      "2. Inside a block, a line that opens a heading, list item, blockquote, or table row starts a new piece.",
      "3. Each piece except a fenced block splits after `.`, `!`, or `?` (optionally followed by a closing quote, parenthesis, or bracket) when whitespace follows.",
    ].join("\n");
    expect(shape(text)).toEqual([
      ["sentence", 1, "The rules, in order:"],
      ["list-item", 3, "1. Blank lines separate blocks."],
      [
        "list-item",
        4,
        "2. Inside a block, a line that opens a heading, list item, blockquote, or table row starts a new piece.",
      ],
      [
        "list-item",
        5,
        "3. Each piece except a fenced block splits after `.`, `!`, or `?` (optionally followed by a closing quote, parenthesis, or bracket) when whitespace follows.",
      ],
    ]);
  });

  test("docs/claude-code.mdx: the skill's 'must not do:' list", () => {
    const text = [
      "The skill also states what an agent must not do:",
      "",
      "- vouch as a human. An agent attests as `agent:<name>`. Its vouch is recorded and does not reset a ttl. Only judged evidence or a `human:` vouch does (see [Attestations](/attestations)).",
      "- reanchor a claim without re-reading the sentence against its evidence.",
      "- delete a file under `.claims/` instead of running `hibi retire <id>`.",
    ].join("\n");
    expect(shape(text)).toEqual([
      ["sentence", 1, "The skill also states what an agent must not do:"],
      ["list-item", 3, "- vouch as a human."],
      ["list-item", 3, "An agent attests as `agent:<name>`."],
      ["list-item", 3, "Its vouch is recorded and does not reset a ttl."],
      [
        "list-item",
        3,
        "Only judged evidence or a `human:` vouch does (see [Attestations](/attestations)).",
      ],
      [
        "list-item",
        4,
        "- reanchor a claim without re-reading the sentence against its evidence.",
      ],
      [
        "list-item",
        5,
        "- delete a file under `.claims/` instead of running `hibi retire <id>`.",
      ],
    ]);
  });

  test("docs/index.mdx: frontmatter and JSX tag lines are no regions", () => {
    const text = [
      "---",
      'title: "Hibi: Find the Sentences in Your Documents That Are No Longer True"',
      'sidebarTitle: "Overview"',
      'description: "Hibi tracks checkable sentences in any document, ties each to code, another document, or an attestation with a ttl, and tells an agent which ones are stale before it trusts the file."',
      'icon: "house"',
      "---",
      "",
      "## Start here",
      "",
      "<CardGroup cols={2}>",
      '  <Card title="Quickstart" icon="rocket" href="/quickstart">',
      "    Install, record one claim of each kind, and run the loop.",
      "  </Card>",
      '  <Card title="Workflows" icon="route" href="/workflows">',
      "    The agent loop, the human loop, and CI.",
      "  </Card>",
      "</CardGroup>",
    ].join("\n");
    const regions = splitRegions(text, "docs/index.mdx");
    expect(regions.map((r) => [r.kind, r.line, r.text])).toEqual([
      ["heading", 8, "## Start here"],
      [
        "sentence",
        12,
        "Install, record one claim of each kind, and run the loop.",
      ],
      ["sentence", 15, "The agent loop, the human loop, and CI."],
    ]);
    expect(regions[1]?.headingPath).toEqual(["Start here"]);
    for (const r of regions) expect(text.slice(r.start, r.end)).toBe(r.text);
  });

  test("frontmatter is skipped only in .md and .mdx files and only when closed; elsewhere its `---` lines are thematic breaks", () => {
    const text = "---\ntitle: x\n---\n\nBody.";
    expect(shape(text, "docs/a.md").map((r) => r[2])).toEqual(["Body."]);
    expect(shape(text, "notes.txt").map((r) => r[2])).toEqual([
      "title: x",
      "Body.",
    ]);
    expect(shape("---\ntitle: x\n\nBody.", "a.mdx").map((r) => r[2])).toEqual([
      "title: x",
      "Body.",
    ]);
  });

  test("docs/claims.mdx: a terminator inside an inline code span does not split; separator rows are no regions", () => {
    const text = [
      "| field | shape |",
      "|---|---|",
      "| `sources` | `[{ id?, resource, title? }]` |",
    ].join("\n");
    expect(shape(text)).toEqual([
      ["table-row", 1, "| field | shape |"],
      ["table-row", 3, "| `sources` | `[{ id?, resource, title? }]` |"],
    ]);
  });

  test("docs/quickstart.mdx: `## 1. Install` is one heading region", () => {
    const text = [
      "## 1. Install",
      "",
      "```sh",
      "# Prebuilt single-file executable (no runtime needed)",
      "```",
    ].join("\n");
    const regions = splitRegions(text, "docs/quickstart.mdx");
    expect(regions.map((r) => [r.kind, r.text])).toEqual([
      ["heading", "## 1. Install"],
      [
        "fenced",
        "```sh\n# Prebuilt single-file executable (no runtime needed)\n```",
      ],
    ]);
    expect(regions[1]?.headingPath).toEqual(["1. Install"]);
  });

  test("docs/lifecycle.mdx: a table row's closing pipe stays with its last sentence", () => {
    const text =
      "| `enforced` | The default. The claim may gate (exit 2) and warn. |";
    expect(shape(text)).toEqual([
      ["table-row", 1, "| `enforced` | The default."],
      ["table-row", 1, "The claim may gate (exit 2) and warn. |"],
    ]);
  });

  test("a heading is one line: the line after it opens a new piece", () => {
    expect(shape("# Title. Part two\nBody line. Second.")).toEqual([
      ["heading", 1, "# Title. Part two"],
      ["sentence", 2, "Body line."],
      ["sentence", 2, "Second."],
    ]);
  });

  test("an inline code span may cross lines; a fence inside a block keeps it whole", () => {
    expect(shape("Run `a. b\nc! d` now. Done.")).toEqual([
      ["sentence", 1, "Run `a. b\nc! d` now."],
      ["sentence", 2, "Done."],
    ]);
    expect(shape("Text. More.\n```\nx. y\n```")).toEqual([
      ["fenced", 1, "Text. More.\n```\nx. y\n```"],
    ]);
  });
});

describe("thematic breaks", () => {
  test("plugins/hibi-cli/skills/hibi/references/cookbook.md: a `---` line is no region", () => {
    const text = [
      "Shared setup: a knowledge repo `handbook/` with `docs/pricing.md`, `docs/billing-policy.md`, and `AGENTS.md`, and a sibling code repo `../api` with `src/limits.ts` (`export const RATE_LIMIT_PER_MIN = 100;`).",
      "",
      "---",
      "",
      "## 1. Initialize a knowledge repo",
    ].join("\n");
    expect(shape(text, "references/cookbook.md")).toEqual([
      [
        "sentence",
        1,
        "Shared setup: a knowledge repo `handbook/` with `docs/pricing.md`, `docs/billing-policy.md`, and `AGENTS.md`, and a sibling code repo `../api` with `src/limits.ts` (`export const RATE_LIMIT_PER_MIN = 100;`).",
      ],
      ["heading", 5, "## 1. Initialize a knowledge repo"],
    ]);
  });

  test("`***` and `___` lines separate blocks even without blank lines; inside a fence they stay", () => {
    expect(shape("One.\n***\nTwo.\n___\nThree.")).toEqual([
      ["sentence", 1, "One."],
      ["sentence", 3, "Two."],
      ["sentence", 5, "Three."],
    ]);
    expect(shape("```\n---\n```")).toEqual([["fenced", 1, "```\n---\n```"]]);
  });
});
