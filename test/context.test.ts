import { describe, expect, test } from "bun:test";
import { CONTEXT_MAX_CHARS, claimContext } from "../src/engine/context.ts";

const ctx = (doc: string, sentence: string) => claimContext(doc, { sentence });

describe("claimContext", () => {
  test("a sentence at the top of a file: no heading path, just its paragraph", () => {
    const doc =
      "Pro costs $29/month. Billing is monthly.\n\n# Later\n\nOther.\n";
    expect(ctx(doc, "Pro costs $29/month.")).toBe(
      "Pro costs $29/month. Billing is monthly.",
    );
  });

  test("under nested headings: the path joins outer to inner and drops closed siblings", () => {
    const doc = [
      "# Guide",
      "",
      "## Setup",
      "",
      "Intro.",
      "",
      "## Pricing",
      "",
      "### Plans",
      "",
      "We sell two plans.",
      "Pro costs $29/month.",
      "",
      "Unrelated paragraph.",
    ].join("\n");
    expect(ctx(doc, "Pro costs $29/month.")).toBe(
      "Guide > Pricing > Plans\n\nWe sell two plans.\nPro costs $29/month.",
    );
  });

  test("a sentence in a list: the paragraph is the list block", () => {
    const doc = [
      "## Limits",
      "",
      "- Requests are capped at 100 per minute.",
      "- Uploads are capped at 10 MB.",
      "- Exports run nightly.",
      "",
      "After the list.",
    ].join("\n");
    expect(ctx(doc, "Uploads are capped at 10 MB.")).toBe(
      "Limits\n\n- Requests are capped at 100 per minute.\n- Uploads are capped at 10 MB.\n- Exports run nightly.",
    );
  });

  test("headings inside fenced blocks are not headings", () => {
    const doc =
      "# Real\n\n```sh\n# not a heading\necho hi\n```\n\nThe sentence.\n";
    expect(ctx(doc, "The sentence.")).toBe("Real\n\nThe sentence.");
  });

  test("bounded: at most 600 characters, the sentence always whole and cut ends marked", () => {
    const filler = "Lorem ipsum dolor sit amet. ".repeat(40);
    const sentence = "Pro costs $29/month.";
    const doc = `# Pricing\n\n${filler}${sentence} ${filler}\n`;
    const out = ctx(doc, sentence);
    expect(out.length).toBeLessThanOrEqual(CONTEXT_MAX_CHARS);
    expect(out.startsWith("Pricing\n\n…")).toBe(true);
    expect(out.endsWith("…")).toBe(true);
    expect(out).toContain(sentence);
    expect(ctx(doc, sentence)).toBe(out);
  });

  test("a long heading path gives way from its outer end", () => {
    const h = (n: number) =>
      `${"#".repeat(n)} ${"Section ".repeat(30).trim()} ${n}`;
    const sentence = "S.".repeat(200);
    const doc = `${h(1)}\n\n${h(2)}\n\n${h(3)}\n\n${sentence}\n`;
    const out = ctx(doc, sentence);
    expect(out.length).toBeLessThanOrEqual(CONTEXT_MAX_CHARS);
    expect(out.endsWith(sentence)).toBe(true);
    expect(out.startsWith("…")).toBe(true);
    expect(out).toContain("Section 3");
  });

  test("a sentence longer than the bound is returned alone; a missing sentence too", () => {
    const long = "x".repeat(700);
    expect(ctx(`# H\n\n${long}\n`, long)).toBe(long);
    expect(ctx("# H\n\nSomething else.\n", "Not here.")).toBe("Not here.");
  });

  test("with several occurrences, the one nearest the recorded position is used", () => {
    const doc = "# A\n\nSame line.\n\n# B\n\nSame line.\n";
    const second = doc.lastIndexOf("Same line.");
    const out = claimContext(doc, {
      sentence: "Same line.",
      doc: {
        file: "x.md",
        selectors: [{ kind: "text-position", start: second, end: second + 10 }],
      },
    });
    expect(out).toBe("B\n\nSame line.");
  });
});
