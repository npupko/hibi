/**
 * The region splitter: a fixed fixture document and its exact regions.
 */
import { describe, expect, test } from "bun:test";
import { splitRegions } from "../src/engine/regions.ts";

const DOC = [
  "# Pricing",
  "",
  "Pro costs $29/month. Teams get a discount!",
  "",
  "## Limits ##",
  "",
  "- The API allows 100 requests per minute.",
  "- Run `hibi check` first.",
  "",
  "> Quoted line one.",
  "",
  "| plan | price |",
  "|------|-------|",
  "",
  "```ts",
  "const a = 1;",
  "",
  "const b = 2;",
  "```",
  "",
  "### Deep",
  "",
  "## Support",
  "",
  'He said "done." Then left',
  "",
].join("\n");

describe("splitRegions", () => {
  test("the fixture document splits into these exact regions", () => {
    expect(splitRegions(DOC)).toEqual([
      {
        id: "r0",
        kind: "heading",
        text: "# Pricing",
        headingPath: [],
        line: 1,
        start: 0,
        end: 9,
      },
      {
        id: "r1",
        kind: "sentence",
        text: "Pro costs $29/month.",
        headingPath: ["Pricing"],
        line: 3,
        start: 11,
        end: 31,
      },
      {
        id: "r2",
        kind: "sentence",
        text: "Teams get a discount!",
        headingPath: ["Pricing"],
        line: 3,
        start: 32,
        end: 53,
      },
      {
        id: "r3",
        kind: "heading",
        text: "## Limits ##",
        headingPath: ["Pricing"],
        line: 5,
        start: 55,
        end: 67,
      },
      {
        id: "r4",
        kind: "list-item",
        text: "- The API allows 100 requests per minute.",
        headingPath: ["Pricing", "Limits"],
        line: 7,
        start: 69,
        end: 110,
      },
      {
        id: "r5",
        kind: "list-item",
        text: "- Run `hibi check` first.",
        headingPath: ["Pricing", "Limits"],
        line: 8,
        start: 111,
        end: 136,
      },
      {
        id: "r6",
        kind: "sentence",
        text: "> Quoted line one.",
        headingPath: ["Pricing", "Limits"],
        line: 10,
        start: 138,
        end: 156,
      },
      {
        id: "r7",
        kind: "table-row",
        text: "| plan | price |",
        headingPath: ["Pricing", "Limits"],
        line: 12,
        start: 158,
        end: 174,
      },
      {
        id: "r8",
        kind: "table-row",
        text: "|------|-------|",
        headingPath: ["Pricing", "Limits"],
        line: 13,
        start: 175,
        end: 191,
      },
      {
        id: "r9",
        kind: "fenced",
        text: "```ts\nconst a = 1;\n\nconst b = 2;\n```",
        headingPath: ["Pricing", "Limits"],
        line: 15,
        start: 193,
        end: 229,
      },
      {
        id: "r10",
        kind: "heading",
        text: "### Deep",
        headingPath: ["Pricing", "Limits"],
        line: 21,
        start: 231,
        end: 239,
      },
      {
        id: "r11",
        kind: "heading",
        text: "## Support",
        headingPath: ["Pricing"],
        line: 23,
        start: 241,
        end: 251,
      },
      {
        id: "r12",
        kind: "sentence",
        text: 'He said "done."',
        headingPath: ["Pricing", "Support"],
        line: 25,
        start: 253,
        end: 268,
      },
      {
        id: "r13",
        kind: "sentence",
        text: "Then left",
        headingPath: ["Pricing", "Support"],
        line: 25,
        start: 269,
        end: 278,
      },
    ]);
  });

  test("every region's text is the document slice at its offsets", () => {
    for (const r of splitRegions(DOC)) {
      expect(DOC.slice(r.start, r.end)).toBe(r.text);
    }
  });

  test("a structural line starts a new region without a blank line; continuation lines stay in their piece", () => {
    const text = [
      "Intro sentence that wraps",
      "onto a second line.",
      "- item that wraps",
      "  onto a second line",
      "Version 1.2 ships today.",
    ].join("\n");
    expect(splitRegions(text).map((r) => [r.kind, r.line, r.text])).toEqual([
      ["sentence", 1, "Intro sentence that wraps\nonto a second line."],
      [
        "list-item",
        3,
        "- item that wraps\n  onto a second line\nVersion 1.2 ships today.",
      ],
    ]);
  });

  test("an empty document has no regions", () => {
    expect(splitRegions("")).toEqual([]);
    expect(splitRegions("\n\n  \n")).toEqual([]);
  });
});
