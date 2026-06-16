import { describe, test, expect, beforeAll } from "bun:test";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { buildAnchor } from "../src/engine/anchor.ts";
import { resolveAssertion } from "../src/algo/resolve.ts";
import type { Assertion } from "../src/core/model.ts";

let analyzer: Awaited<ReturnType<typeof getAnalyzer>>;
beforeAll(async () => {
  analyzer = await getAnalyzer();
});

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
    language: "typescript", file: "a.ts",
    original: "export const MAX_ATTEMPTS = 5;\n", quote: "MAX_ATTEMPTS = 5", expectedValue: "5",
    drifted: "export const MAX_ATTEMPTS = 50;\n",
  },
  {
    language: "python", file: "a.py",
    original: "MAX_ATTEMPTS = 5\n", quote: "MAX_ATTEMPTS = 5", expectedValue: "5",
    drifted: "MAX_ATTEMPTS = 50\n",
  },
  {
    language: "rust", file: "a.rs",
    original: "const MAX_ATTEMPTS: u32 = 5;\n", quote: "MAX_ATTEMPTS: u32 = 5", expectedValue: "5",
    drifted: "const MAX_ATTEMPTS: u32 = 50;\n",
  },
  {
    language: "go", file: "a.go",
    original: "const MaxAttempts = 5\n", quote: "MaxAttempts = 5", expectedValue: "5",
    drifted: "const MaxAttempts = 50\n",
  },
  {
    language: "java", file: "a.java",
    original: "class C { static final int MAX_ATTEMPTS = 5; }\n", quote: "MAX_ATTEMPTS = 5", expectedValue: "5",
    drifted: "class C { static final int MAX_ATTEMPTS = 50; }\n",
  },
];

describe("all five first-party grammars (§16, §17.4)", () => {
  for (const c of CASES) {
    test(`${c.language}: parses, fingerprints, extracts value, and detects drift`, () => {
      const start = c.original.indexOf(c.quote);
      const region = { start, end: start + c.quote.length };

      // Parse + fingerprint succeed.
      const fp = analyzer.analyze(c.original, c.language, region);
      expect(fp).not.toBeNull();
      expect(fp!.structuralHash).toMatch(/^[0-9a-f]{16}$/);

      // Value extraction per the language map.
      expect(analyzer.extractValue(c.original, c.language, region)).toBe(c.expectedValue);

      // The anchor carries an ast-node and a value selector.
      const anchor = buildAnchor(c.file, c.original, region, { language: c.language, analyzer });
      expect(anchor.selectors.some((s) => s.kind === "ast-node")).toBe(true);
      expect(anchor.selectors.some((s) => s.kind === "value")).toBe(true);

      // A changed literal is detected (never graded fresh).
      const a: Assertion = { id: "a", propositionId: "p", documentId: "d", owner: "o", ref: "r", anchor, attrs: {} };
      const fresh = resolveAssertion(a, c.original, { ast: analyzer });
      expect(fresh.state).toBe("fresh");
      const drift = resolveAssertion(a, c.drifted, { ast: analyzer });
      expect(drift.state).not.toBe("fresh");
    });
  }
});
