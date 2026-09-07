import { describe, expect, test } from "bun:test";
import {
  fuzzyLocate,
  localizeTextQuote,
  withinErrorBudget,
} from "../src/algo/localize.ts";
import {
  collapseWhitespace,
  levenshtein,
  normalizeText,
  textSimilarity,
} from "../src/algo/normalize.ts";
import {
  fuzzyErrorBudget,
  MOVE_AWARENESS_CHARS,
  SAME_TEXT_SIMILARITY,
} from "../src/algo/params.ts";
import { resolveAssertion, resolveSide } from "../src/algo/resolve.ts";
import type { SelectorBundle } from "../src/core/model.ts";
import { buildSelectorBundle } from "../src/engine/anchor.ts";
import { MATCH_MAX_BITS, matchMain } from "../src/vendor/bitap.ts";
import { makeRepo, record } from "./helpers.ts";

describe("Bitap matcher (vendored diff-match-patch)", () => {
  test("exact match returns the location", () => {
    expect(matchMain("the quick brown fox", "quick", 0)).toBe(4);
  });
  test("fuzzy match tolerates a small edit", () => {
    const text = "function retryWithBackoff(maxAttempts) {}";
    const at = matchMain(text, "retryWithBackoff", 0); // dropped an 'f'
    expect(at).toBeGreaterThanOrEqual(8);
    expect(at).toBeLessThanOrEqual(10);
  });
  test("returns -1 when nothing is close enough", () => {
    expect(matchMain("aaaaaaaaaa", "zzzz", 0)).toBe(-1);
  });
  test("biases toward loc when pattern appears twice", () => {
    const text = "needle ......................................... needle";
    const near = matchMain(text, "needle", 48);
    expect(near).toBe(49);
  });
  test("throws on patterns longer than the 32-char word size", () => {
    expect(() =>
      matchMain("x".repeat(100), "y".repeat(MATCH_MAX_BITS + 1), 0),
    ).toThrow();
  });
});

describe("text normalization and similarity", () => {
  test("reindent normalizes to identical and scores 1.0", () => {
    const a = "if (x) {\n    return 5;\n}";
    const b = "if (x) {\n        return 5;\n}";
    expect(normalizeText(a)).toBe(normalizeText(b));
    expect(textSimilarity(a, b)).toBe(1);
  });
  test("reflow (line breaks) scores 1.0", () => {
    expect(
      textSimilarity("retries are capped at 5", "retries are\ncapped at 5"),
    ).toBe(1);
  });
  test("a changed constant lowers similarity below 1", () => {
    expect(
      textSimilarity("MAX_ATTEMPTS = 5", "MAX_ATTEMPTS = 50"),
    ).toBeLessThan(1);
  });
  test("levenshtein basics", () => {
    expect(levenshtein("kitten", "sitting")).toBe(3);
    expect(levenshtein("", "abc")).toBe(3);
  });
  test("collapseWhitespace", () => {
    expect(collapseWhitespace("  a   b\tc \n d ")).toBe("a b c d");
  });
});

describe("fixed resolution parameters", () => {
  test("the fuzzy error budget is 40% of the quote length, capped at 256", () => {
    expect(fuzzyErrorBudget(10)).toBe(4);
    expect(fuzzyErrorBudget(1000)).toBe(256);
  });
  test("the same-text floor and the move threshold are the documented values", () => {
    expect(SAME_TEXT_SIMILARITY).toBe(0.9);
    expect(MOVE_AWARENESS_CHARS).toBe(4);
  });
  test("withinErrorBudget accepts a span inside the budget and rejects one outside it", () => {
    expect(withinErrorBudget("abcdefghix", "abcdefghij")).toBe(true);
    expect(withinErrorBudget("zzzzzzzzzz", "abcdefghij")).toBe(false);
  });
});

const tq = (exact: string, prefix = "", suffix = "") =>
  ({ kind: "text-quote", exact, prefix, suffix }) as const;
const tp = (start: number, end: number) =>
  ({ kind: "text-position", start, end }) as const;

describe("text-quote localization", () => {
  test("a single exact occurrence is returned as an exact hit", () => {
    const text = "line one\nconst MAX = 5;\nline three";
    const r = localizeTextQuote(text, tq("const MAX = 5;"), undefined);
    expect(r.how).toBe("exact");
    expect(r.ambiguous).toBe(false);
    expect(text.slice(r.region?.start, r.region?.end)).toBe("const MAX = 5;");
  });

  test("a quote that moved is still found by its exact text", () => {
    const text = "// a new header line added at the top\nconst MAX = 5;";
    const r = localizeTextQuote(text, tq("const MAX = 5;"), tp(0, 14));
    expect(r.how).toBe("exact");
    expect(text.slice(r.region?.start, r.region?.end)).toBe("const MAX = 5;");
  });

  test("a quote with a small edit is found by the fuzzy cascade", () => {
    const exact =
      "the retry policy caps attempts at five and then gives up entirely";
    const text =
      "prologue\nthe retry policy caps attempts at six and then gives up entirely\nepilogue";
    const r = localizeTextQuote(
      text,
      tq(exact, "prologue\n", "\nepilogue"),
      tp(9, 9 + exact.length),
    );
    expect(r.how).toBe("fuzzy");
    expect(text.slice(r.region?.start, r.region?.end)).toContain(
      "the retry policy caps attempts",
    );
  });

  test("fuzzyLocate returns the region of the nearest fuzzy match", () => {
    const text = "function retryWithBackoff(maxAttempts) {}";
    const r = fuzzyLocate(text, tq("retryWithBackof"), 0);
    expect(r).not.toBeNull();
    expect(r?.start).toBeGreaterThanOrEqual(8);
    expect(r?.start).toBeLessThanOrEqual(10);
  });

  test("a quote that is gone yields no region", () => {
    const r = localizeTextQuote(
      "completely unrelated content here",
      tq("const MAX = 5;"),
      undefined,
    );
    expect(r.region).toBeNull();
    expect(r.how).toBe("none");
  });

  test("a fuzzy candidate over the error budget counts as not found", () => {
    const r = localizeTextQuote(
      "const MAX = 5;\n",
      tq("const LIMIT_TOTAL = 999999;"),
      undefined,
    );
    expect(r.region).toBeNull();
  });

  test("several equally scored occurrences of a long quote are ambiguous", () => {
    const text = "abc\nconst MAX = 5;\nabc\nconst MAX = 5;\nabc\n";
    const r = localizeTextQuote(text, tq("const MAX = 5;"), undefined);
    expect(r.ambiguous).toBe(true);
    expect(r.region).not.toBeNull();
  });

  test("context breaks the tie between occurrences", () => {
    const text = "first:\nconst MAX = 5;\nsecond:\nconst MAX = 5;\nend\n";
    const r = localizeTextQuote(
      text,
      tq("const MAX = 5;", "second:\n", "\nend"),
      undefined,
    );
    expect(r.ambiguous).toBe(false);
    expect(r.region?.start).toBe(text.lastIndexOf("const MAX = 5;"));
  });

  test("position breaks the tie for a short quote", () => {
    const text = "MAX ......................... MAX";
    const r = localizeTextQuote(text, tq("MAX"), tp(30, 33));
    expect(r.ambiguous).toBe(false);
    expect(r.region?.start).toBe(30);
  });
});

/** A doc-side bundle for `sentence` inside `docText`. */
function bundleFor(file: string, docText: string, sentence: string) {
  const start = docText.indexOf(sentence);
  return buildSelectorBundle(file, docText, {
    start,
    end: start + sentence.length,
  });
}

describe("resolveSide: the ordered cascade", () => {
  const original = "# Doc\n\nRetries are capped at 5 attempts.\n";
  const sentence = "Retries are capped at 5 attempts.";
  const bundle = bundleFor("README.md", original, sentence);

  test("same text at the same offset is unchanged", () => {
    const side = resolveSide(bundle, original);
    expect(side.state).toBe("unchanged");
    expect(side.similarity).toBe(1);
    expect(side.liveText).toBe(sentence);
  });

  test("same text within the move threshold is still unchanged", () => {
    const side = resolveSide(bundle, `# Doc\n\n  ${sentence}\n`);
    expect(side.state).toBe("unchanged");
  });

  test("same text at a new offset past the move threshold is moved", () => {
    const side = resolveSide(
      bundle,
      `# Doc\n\nA new paragraph.\n\n${sentence}\n`,
    );
    expect(side.state).toBe("moved");
    expect(side.notes.join(" ")).toContain("span moved");
  });

  test("a changed number in a sentence is changed even at high similarity", () => {
    const side = resolveSide(
      bundle,
      "# Doc\n\nRetries are capped at 7 attempts.\n",
    );
    expect(side.state).toBe("changed");
    expect(side.notes.join(" ")).toContain("a number in the sentence changed");
    expect(side.changedEvidence[0]?.kind).toBe("text");
  });

  test("text below the similarity floor is changed", () => {
    const side = resolveSide(
      bundle,
      "# Doc\n\nRetries are limited at 5 attempts.\n",
    );
    expect(side.state).toBe("changed");
    expect(side.similarity).toBeLessThan(SAME_TEXT_SIMILARITY);
    expect(side.notes.join(" ")).toContain("text changed");
  });

  test("a quote that is not found is orphaned", () => {
    const side = resolveSide(bundle, "# Doc\n\nSomething else entirely.\n");
    expect(side.state).toBe("orphaned");
    expect(side.region).toBeNull();
    expect(side.changedEvidence[0]?.detail).toBe("documented span orphaned");
  });

  test("a missing file is orphaned with a file-not-found note", () => {
    const side = resolveSide(bundle, null);
    expect(side.state).toBe("orphaned");
    expect(side.notes[0]).toBe("file not found: README.md");
    expect(side.changedEvidence[0]?.detail).toBe("file missing");
  });

  test("several equal exact matches are ambiguous", () => {
    // No stored context, so the two occurrences score the same.
    const noContext: SelectorBundle = {
      file: "README.md",
      selectors: [
        { kind: "text-quote", exact: sentence, prefix: "", suffix: "" },
      ],
    };
    const side = resolveSide(noContext, `${sentence}\n\n${sentence}\n`);
    expect(side.state).toBe("ambiguous");
    expect(side.notes[0]).toContain("several places");
  });

  test("context selects one of several exact matches", () => {
    const side = resolveSide(bundle, `# Doc\n\n${sentence}\n\n${sentence}\n`);
    expect(["unchanged", "moved"]).toContain(side.state);
  });

  test("a coarse-only bundle is unchanged and navigational", () => {
    const coarse: SelectorBundle = {
      file: "src/**",
      selectors: [{ kind: "coarse", pattern: "src/**" }],
    };
    const side = resolveSide(coarse, "anything at all");
    expect(side.state).toBe("unchanged");
    expect(side.notes[0]).toContain("coarse anchor");
  });
});

describe("expired is a resolve-level flag, orthogonal to the anchor states", () => {
  test("resolveAssertion sets verdict.expired without disturbing doc/code", async () => {
    const repo = await makeRepo();
    try {
      await repo.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
      const { assertion } = await record(repo, {
        doc: "README.md",
        text: "Retries are capped at five attempts.",
        file: "src/retry.ts",
        quote: "export const MAX_ATTEMPTS = 5;",
        verified: true,
        ttl: "2000-01-01T00:00:00.000Z",
      });

      const docContent = await repo.read("README.md");
      const codeContent = await repo.read("src/retry.ts");
      const verdict = resolveAssertion(assertion, {
        doc: docContent,
        code: new Map([["src/retry.ts", codeContent]]),
      });

      expect(verdict.expired).toBe(true);
      expect(verdict.doc).toBe("unchanged");
      expect(verdict.code).toBe("unchanged");
      expect(verdict.gates).toBe(true);
    } finally {
      await repo.cleanup();
    }
  });

  test("an un-expired claim leaves verdict.expired false", async () => {
    const repo = await makeRepo();
    try {
      await repo.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
      const { assertion } = await record(repo, {
        doc: "README.md",
        text: "Retries are capped at five attempts.",
        file: "src/retry.ts",
        quote: "export const MAX_ATTEMPTS = 5;",
        verified: true,
      });
      const verdict = resolveAssertion(assertion, {
        doc: await repo.read("README.md"),
        code: new Map([["src/retry.ts", await repo.read("src/retry.ts")]]),
      });
      expect(verdict.expired).toBe(false);
      expect(verdict.behavior).toBeUndefined();
    } finally {
      await repo.cleanup();
    }
  });

  test("an unparseable ttl is treated as expired and noted", async () => {
    const repo = await makeRepo();
    try {
      await repo.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
      const { assertion } = await record(repo, {
        doc: "README.md",
        text: "Retries are capped at five attempts.",
        file: "src/retry.ts",
        quote: "export const MAX_ATTEMPTS = 5;",
        ttl: "not-a-date",
      });
      const verdict = resolveAssertion(assertion, {
        doc: await repo.read("README.md"),
        code: new Map([["src/retry.ts", await repo.read("src/retry.ts")]]),
      });
      expect(verdict.expired).toBe(true);
      expect(verdict.notes.join(" ")).toContain("unparseable ttl");
    } finally {
      await repo.cleanup();
    }
  });
});
