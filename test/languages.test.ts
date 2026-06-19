import { beforeAll, describe, expect, test } from "bun:test";
import { resolveAssertion } from "../src/algo/resolve.ts";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import type { Assertion } from "../src/core/model.ts";
import { buildSelectorBundle, composeAnchor } from "../src/engine/anchor.ts";

let analyzer: Awaited<ReturnType<typeof getAnalyzer>>;
beforeAll(async () => {
  analyzer = await getAnalyzer();
});

const region = (text: string, quote: string) => {
  const start = text.indexOf(quote);
  return { start, end: start + quote.length };
};

interface LangCase {
  language: string;
  file: string;
  original: string;
  quote: string;
  expectedValue: string;
  drifted: string;
}

// One representative value claim per first-party grammar (§16, §17.4).
const CASES: LangCase[] = [
  {
    language: "typescript",
    file: "a.ts",
    original: "export const MAX_ATTEMPTS = 5;\n",
    quote: "MAX_ATTEMPTS = 5",
    expectedValue: "5",
    drifted: "export const MAX_ATTEMPTS = 50;\n",
  },
  {
    language: "python",
    file: "a.py",
    original: "MAX_ATTEMPTS = 5\n",
    quote: "MAX_ATTEMPTS = 5",
    expectedValue: "5",
    drifted: "MAX_ATTEMPTS = 50\n",
  },
  {
    language: "rust",
    file: "a.rs",
    original: "const MAX_ATTEMPTS: u32 = 5;\n",
    quote: "MAX_ATTEMPTS: u32 = 5",
    expectedValue: "5",
    drifted: "const MAX_ATTEMPTS: u32 = 50;\n",
  },
  {
    language: "go",
    file: "a.go",
    original: "const MaxAttempts = 5\n",
    quote: "MaxAttempts = 5",
    expectedValue: "5",
    drifted: "const MaxAttempts = 50\n",
  },
  {
    language: "java",
    file: "a.java",
    original: "class C { static final int MAX_ATTEMPTS = 5; }\n",
    quote: "MAX_ATTEMPTS = 5",
    expectedValue: "5",
    drifted: "class C { static final int MAX_ATTEMPTS = 50; }\n",
  },
];

const DOC_TEXT = "The retry budget is five attempts.\n";
const DOC_SENTENCE = "The retry budget is five attempts.";

describe("all five first-party grammars (§16, §17.4)", () => {
  for (const c of CASES) {
    test(`${c.language}: parses, fingerprints, extracts value, and a changed literal never grades code:unchanged`, () => {
      const start = c.original.indexOf(c.quote);
      const r = { start, end: start + c.quote.length };

      // Parse + fingerprint succeed.
      const fp = analyzer.analyze(c.original, c.language, r);
      expect(fp).not.toBeNull();
      expect(fp?.structuralHash).toMatch(/^[0-9a-f]{16}$/);

      // Value extraction per the language map.
      expect(analyzer.extractValue(c.original, c.language, r)).toBe(
        c.expectedValue,
      );

      // The code-side bundle carries an ast-node and a value selector.
      const codeBundle = buildSelectorBundle(c.file, c.original, r, {
        language: c.language,
        analyzer,
      });
      expect(codeBundle.selectors.some((s) => s.kind === "ast-node")).toBe(
        true,
      );
      expect(codeBundle.selectors.some((s) => s.kind === "value")).toBe(true);

      const docBundle = buildSelectorBundle(
        "README.md",
        DOC_TEXT,
        region(DOC_TEXT, DOC_SENTENCE),
      );

      const a: Assertion = {
        id: "a",
        propositionId: "p",
        documentId: "d",
        owner: "o",
        ref: "r",
        anchor: composeAnchor(docBundle, [codeBundle]),
        enforcement: "suggested",
        verifiers: [],
        attrs: {},
      };

      // The unchanged file grades code:unchanged.
      const fresh = resolveAssertion(
        a,
        { doc: DOC_TEXT, code: new Map([[c.file, c.original]]) },
        { ast: analyzer },
      );
      expect(fresh.code).toBe("unchanged");

      // A changed literal is detected — never graded code:unchanged.
      const drift = resolveAssertion(
        a,
        { doc: DOC_TEXT, code: new Map([[c.file, c.drifted]]) },
        { ast: analyzer },
      );
      expect(drift.code).not.toBe("unchanged");
    });
  }
});
