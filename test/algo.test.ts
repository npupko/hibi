import { describe, expect, test } from "bun:test";
import { bandConfidence, fuseConfidence, grade } from "../src/algo/fusion.ts";
import { localizeTextQuote } from "../src/algo/localize.ts";
import {
  collapseWhitespace,
  levenshtein,
  normalizeText,
  textSimilarity,
} from "../src/algo/normalize.ts";
import { WEIGHTS } from "../src/algo/params.ts";
import { resolveAssertion } from "../src/algo/resolve.ts";
import { MATCH_MAX_BITS, matchMain } from "../src/vendor/bitap.ts";
import { fnv1a32hex } from "../src/vendor/fnv1a.ts";
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

describe("FNV-1a 32-bit checksum", () => {
  // Canonical FNV-1a test vectors (offset 0x811c9dc5, prime 0x01000193).
  test("known vectors", () => {
    expect(fnv1a32hex("")).toBe("811c9dc5");
    expect(fnv1a32hex("a")).toBe("e40c292c");
    expect(fnv1a32hex("foobar")).toBe("bf9cf968");
  });
  test("is deterministic and order-sensitive", () => {
    expect(fnv1a32hex("ab")).toBe(fnv1a32hex("ab"));
    expect(fnv1a32hex("ab")).not.toBe(fnv1a32hex("ba"));
  });
});

describe("text normalization & similarity (§17.2)", () => {
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

describe("text-quote localization cascade (§17.1)", () => {
  test("locates a short exact quote", () => {
    const text = "line one\nconst MAX = 5;\nline three";
    const r = localizeTextQuote(
      text,
      { kind: "text-quote", exact: "const MAX = 5;", prefix: "", suffix: "" },
      9,
    );
    expect(r).not.toBeNull();
    expect(text.slice(r?.start, r?.end)).toBe("const MAX = 5;");
  });
  test("locates a quote that moved", () => {
    const text = "// a new header line added at the top\nconst MAX = 5;";
    const r = localizeTextQuote(
      text,
      { kind: "text-quote", exact: "const MAX = 5;", prefix: "", suffix: "" },
      0,
    );
    expect(r).not.toBeNull();
    expect(text.slice(r?.start, r?.end)).toBe("const MAX = 5;");
  });
  test("handles long quotes (>32 chars) via head+suffix", () => {
    const exact =
      "the retry policy caps attempts at five and then gives up entirely";
    const text = `prologue\n${exact}\nepilogue`;
    const r = localizeTextQuote(
      text,
      { kind: "text-quote", exact, prefix: "prologue\n", suffix: "\nepilogue" },
      9,
    );
    expect(r).not.toBeNull();
    expect(text.slice(r?.start, r?.end)).toContain(
      "the retry policy caps attempts",
    );
  });
  test("returns null when the quote is gone", () => {
    const r = localizeTextQuote(
      "completely unrelated content here",
      { kind: "text-quote", exact: "const MAX = 5;", prefix: "", suffix: "" },
      0,
    );
    expect(r).toBeNull();
  });
});

// Convenience: a fully-specified GradeInput with one drift dimension overridden.
function gradeInput(over: Partial<Parameters<typeof grade>[0]> = {}) {
  return grade({
    selectors: [],
    coarseOnly: false,
    ambiguous: false,
    startDelta: 0,
    textQuoteFound: true,
    textQuoteSimilarity: 1,
    valueFound: false,
    valueScore: 0,
    ...over,
  });
}

describe("confidence fusion & grading (§17.3, two-axis AnchorState)", () => {
  test("fuses over found selectors only", () => {
    const c = fuseConfidence([
      {
        kind: "text-quote",
        found: true,
        score: 1,
        weight: WEIGHTS["text-quote"],
      },
      { kind: "ast-node", found: true, score: 1, weight: WEIGHTS["ast-node"] },
      { kind: "value", found: false, score: 0, weight: WEIGHTS.value },
    ]);
    expect(c).toBeCloseTo(1, 5);
  });

  // bandConfidence maps confidence → AnchorState (one vocabulary, both sides).
  // Bands (§17.3): C ≥ 0.8 unchanged, ≥ 0.5 moved, ≥ 0.2 changed, else orphaned.
  test("confidence bands → AnchorState", () => {
    expect(bandConfidence(0.95)).toBe("unchanged");
    expect(bandConfidence(0.8)).toBe("unchanged");
    expect(bandConfidence(0.65)).toBe("moved");
    expect(bandConfidence(0.5)).toBe("moved");
    expect(bandConfidence(0.35)).toBe("changed");
    expect(bandConfidence(0.2)).toBe("changed");
    expect(bandConfidence(0.1)).toBe("orphaned");
    expect(bandConfidence(0)).toBe("orphaned");
  });

  test("fewer than two found selectors → orphaned (confidence forced to 0)", () => {
    const g = grade({
      selectors: [{ kind: "text-quote", found: true, score: 1, weight: 0.3 }],
      coarseOnly: false,
      ambiguous: false,
      startDelta: 0,
      textQuoteFound: true,
      textQuoteSimilarity: 1,
      valueFound: false,
      valueScore: 0,
    });
    expect(g.state).toBe("orphaned");
    expect(g.confidence).toBe(0);
  });

  test("value veto forces changed at confidence 0.3", () => {
    const g = grade({
      selectors: [
        { kind: "text-quote", found: true, score: 0.95, weight: 0.3 },
        { kind: "value", found: true, score: 0, weight: 0.2 },
      ],
      coarseOnly: false,
      ambiguous: false,
      startDelta: 0,
      textQuoteFound: true,
      textQuoteSimilarity: 0.95,
      valueFound: true,
      valueScore: 0,
    });
    expect(g.state).toBe("changed");
    expect(g.confidence).toBe(0.3);
  });

  test("coarse-only anchors are never drift → unchanged", () => {
    const g = grade({
      selectors: [],
      coarseOnly: true,
      ambiguous: false,
      startDelta: null,
      textQuoteFound: false,
      textQuoteSimilarity: 0,
      valueFound: false,
      valueScore: 0,
    });
    expect(g.state).toBe("unchanged");
  });

  test("move-awareness: unchanged → moved when start drifts > 4 chars", () => {
    const g = grade({
      selectors: [
        { kind: "text-quote", found: true, score: 1, weight: 0.3 },
        { kind: "ast-node", found: true, score: 1, weight: 0.35 },
      ],
      coarseOnly: false,
      ambiguous: false,
      startDelta: 12,
      textQuoteFound: true,
      textQuoteSimilarity: 1,
      valueFound: false,
      valueScore: 0,
    });
    expect(g.state).toBe("moved");
  });

  test("ambiguous: a multiply-matched quote over a clean fuse → ambiguous", () => {
    const g = grade({
      selectors: [
        { kind: "text-quote", found: true, score: 1, weight: 0.3 },
        { kind: "ast-node", found: true, score: 1, weight: 0.35 },
      ],
      coarseOnly: false,
      ambiguous: true,
      startDelta: 0,
      textQuoteFound: true,
      textQuoteSimilarity: 1,
      valueFound: false,
      valueScore: 0,
    });
    expect(g.state).toBe("ambiguous");
  });

  test("ambiguous does not override a genuine content changed/orphaned", () => {
    // Low-confidence fuse lands in `changed`; ambiguity must not mask it.
    const g = grade({
      selectors: [
        { kind: "text-quote", found: true, score: 0.3, weight: 0.3 },
        { kind: "ast-node", found: true, score: 0.3, weight: 0.35 },
      ],
      coarseOnly: false,
      ambiguous: true,
      startDelta: 0,
      textQuoteFound: true,
      textQuoteSimilarity: 0.3,
      valueFound: false,
      valueScore: 0,
    });
    expect(g.state).toBe("changed");
  });

  test("grade has no `expired` axis — expired is resolve-level only", () => {
    // Sanity: the GradeResult type/keys never carry an expired flag; grade only
    // emits an AnchorState (expired lives on the Verdict, asserted below).
    const g = gradeInput({
      selectors: [
        { kind: "text-quote", found: true, score: 1, weight: 0.3 },
        { kind: "ast-node", found: true, score: 1, weight: 0.35 },
      ],
    });
    expect(g.state).toBe("unchanged");
    expect(Object.keys(g)).toEqual(["state", "confidence", "notes"]);
  });
});

describe("expired is a resolve-level orthogonal flag (§17.3, moved OUT of grade)", () => {
  test("resolveAssertion sets verdict.expired without disturbing doc/code", async () => {
    const repo = await makeRepo();
    try {
      await repo.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
      // A past TTL → the time flag must fire while both sides still resolve.
      const { assertion } = await record(repo, {
        doc: "README.md",
        text: "Retries are capped at five attempts.",
        file: "src/retry.ts",
        quote: "export const MAX_ATTEMPTS = 5;",
        trust: "verified",
        ttl: "2000-01-01T00:00:00.000Z",
      });

      const docContent = await repo.read("README.md");
      const codeContent = await repo.read("src/retry.ts");
      const verdict = resolveAssertion(assertion, {
        doc: docContent,
        code: new Map([["src/retry.ts", codeContent]]),
      });

      // expired is orthogonal: set independently, doc/code resolve on their own.
      expect(verdict.expired).toBe(true);
      expect(verdict.doc).toBe("unchanged");
      expect(verdict.code).toBe("unchanged");
      // expired is not an AnchorState value — it never leaks into either side.
      expect(verdict.doc).not.toBe("expired");
      expect(verdict.code).not.toBe("expired");
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
        trust: "verified",
        // no ttl → never expires
      });
      const verdict = resolveAssertion(assertion, {
        doc: await repo.read("README.md"),
        code: new Map([["src/retry.ts", await repo.read("src/retry.ts")]]),
      });
      expect(verdict.expired).toBe(false);
    } finally {
      await repo.cleanup();
    }
  });
});
