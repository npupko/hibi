/**
 * Judge context: the lead-in rule for list items and frontmatter. Text is
 * copied from this repository's documents (as of commit 1802f47).
 */
import { describe, expect, test } from "bun:test";
import { claimContext } from "../src/engine/context.ts";

const at = (file: string) => ({
  file,
  selectors: [] as never[],
});

describe("claimContext lead-in", () => {
  const MUST_NOT = [
    "## Skill",
    "",
    "The skill also states what an agent must not do:",
    "",
    "- vouch as a human. An agent attests as `agent:<name>`. Its vouch is recorded and does not reset a ttl. Only judged evidence or a `human:` vouch does (see [Attestations](/attestations)).",
    "- reanchor a claim without re-reading the sentence against its evidence.",
    "- delete a file under `.claims/` instead of running `hibi retire <id>`.",
    "",
    "## Session hooks",
  ].join("\n");

  test("docs/claude-code.mdx: a list item's context starts with the list's lead-in", () => {
    const sentence =
      "reanchor a claim without re-reading the sentence against its evidence.";
    expect(claimContext(MUST_NOT, { sentence })).toBe(
      [
        "Skill",
        "",
        "The skill also states what an agent must not do:",
        "",
        "- vouch as a human. An agent attests as `agent:<name>`. Its vouch is recorded and does not reset a ttl. Only judged evidence or a `human:` vouch does (see [Attestations](/attestations)).",
        "- reanchor a claim without re-reading the sentence against its evidence.",
        "- delete a file under `.claims/` instead of running `hibi retire <id>`.",
      ].join("\n"),
    );
  });

  test("docs/audit.mdx L29-33: the lead-in may span lines; a sentence outside the list is unchanged", () => {
    const doc = [
      "## Regions",
      "",
      "The splitter turns any text file into regions.",
      "The rules, in order:",
      "",
      "1. Blank lines separate blocks.",
      "2. Inside a block, a line that opens a heading, list item, blockquote, or table row starts a new piece.",
    ].join("\n");
    expect(
      claimContext(doc, { sentence: "1. Blank lines separate blocks." }),
    ).toBe(
      "Regions\n\nThe splitter turns any text file into regions.\nThe rules, in order:\n\n1. Blank lines separate blocks.\n2. Inside a block, a line that opens a heading, list item, blockquote, or table row starts a new piece.",
    );
    expect(claimContext(doc, { sentence: "The rules, in order:" })).toBe(
      "Regions\n\nThe splitter turns any text file into regions.\nThe rules, in order:",
    );
  });

  test("no lead-in above a heading, another list, a table, or a tag line", () => {
    const sentence = "Uploads are capped.";
    for (const above of ["## Limits", "- other list", "| a | b |", "</Card>"]) {
      const doc = `${above}\n\n- ${sentence}\n`;
      expect(claimContext(doc, { sentence })).toBe(
        above.startsWith("#") ? `Limits\n\n- ${sentence}` : `- ${sentence}`,
      );
    }
  });

  test("a list that follows its lead-in without a blank line already holds it", () => {
    const doc = "Limits:\n- Uploads are capped.\n";
    expect(claimContext(doc, { sentence: "Uploads are capped." })).toBe(
      "Limits:\n- Uploads are capped.",
    );
  });
});

describe("claimContext frontmatter", () => {
  test("docs/index.mdx: frontmatter is not context in .md and .mdx files", () => {
    const doc = [
      "---",
      "# not a heading",
      'sidebarTitle: "Overview"',
      "---",
      "Hibi runs nothing and fetches nothing.",
    ].join("\n");
    const sentence = "Hibi runs nothing and fetches nothing.";
    expect(claimContext(doc, { sentence, doc: at("docs/index.mdx") })).toBe(
      sentence,
    );
    expect(claimContext(doc, { sentence, doc: at("notes.txt") })).toBe(
      "not a heading\n\nHibi runs nothing and fetches nothing.",
    );
  });
});

describe("claimContext thematic breaks", () => {
  test("plugins/hibi-cli/skills/hibi/references/cookbook.md: a `---` line ends a paragraph and stops the lead-in search", () => {
    const sentence = "Ids are examples.";
    const doc = `---\n${sentence} Commands assume \`hibi\` is on PATH.\n`;
    expect(claimContext(doc, { sentence })).toBe(
      `${sentence} Commands assume \`hibi\` is on PATH.`,
    );
    const list = "Intro:\n---\n\n- Uploads are capped.\n";
    expect(claimContext(list, { sentence: "Uploads are capped." })).toBe(
      "- Uploads are capped.",
    );
  });
});
