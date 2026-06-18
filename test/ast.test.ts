import { beforeAll, describe, expect, test } from "bun:test";
import { resolveAssertion } from "../src/algo/resolve.ts";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import type { Assertion } from "../src/core/model.ts";
import { buildAnchor } from "../src/engine/anchor.ts";

let analyzer: Awaited<ReturnType<typeof getAnalyzer>>;
beforeAll(async () => {
  analyzer = await getAnalyzer();
});

const region = (text: string, quote: string) => {
  const start = text.indexOf(quote);
  return { start, end: start + quote.length };
};

describe("tree-sitter snapping & two-tier hash (§17.2)", () => {
  test("structural hash is invariant under re-indentation", () => {
    const a = "export const MAX_ATTEMPTS = 5;";
    const b = "export   const    MAX_ATTEMPTS   =   5;";
    const fa = analyzer.analyze(
      a,
      "typescript",
      region(a, "MAX_ATTEMPTS = 5"),
    )!;
    const fb = analyzer.analyze(
      b,
      "typescript",
      region(b, "MAX_ATTEMPTS   =   5"),
    )!;
    expect(fa.structuralHash).toBe(fb.structuralHash);
    expect(fa.semanticHash).toBe(fb.semanticHash); // whitespace is collapsed
  });

  test("a rename keeps the structural hash but changes the semantic hash", () => {
    const a = "function retry(maxAttempts: number) { return maxAttempts; }";
    const b = "function retry(maxTries: number) { return maxTries; }";
    const fa = analyzer.analyze(
      a,
      "typescript",
      region(a, "retry(maxAttempts"),
    )!;
    const fb = analyzer.analyze(b, "typescript", region(b, "retry(maxTries"))!;
    expect(fa.structuralHash).toBe(fb.structuralHash);
    expect(fa.semanticHash).not.toBe(fb.semanticHash);
  });

  test("a changed numeric literal changes the semantic hash (5 → 50)", () => {
    const a = "const MAX = 5;";
    const b = "const MAX = 50;";
    const fa = analyzer.analyze(a, "typescript", region(a, "MAX = 5"))!;
    const fb = analyzer.analyze(b, "typescript", region(b, "MAX = 50"))!;
    expect(fa.semanticHash).not.toBe(fb.semanticHash);
  });
});

describe("value extraction (§17.4)", () => {
  test("extracts a scalar number", () => {
    const t = "const MAX = 5;";
    expect(analyzer.extractValue(t, "typescript", region(t, "MAX = 5"))).toBe(
      "5",
    );
  });
  test("extracts a string", () => {
    const t = 'const name = "alice";';
    expect(
      analyzer.extractValue(t, "typescript", region(t, 'name = "alice"')),
    ).toBe('"alice"');
  });
  test("extracts an array with whitespace stripped", () => {
    const t = "const xs = [1, 2, 3];";
    expect(
      analyzer.extractValue(t, "typescript", region(t, "xs = [1, 2, 3]")),
    ).toBe("[1,2,3]");
  });
  test("Python integer", () => {
    const t = "MAX = 5";
    expect(analyzer.extractValue(t, "python", region(t, "MAX = 5"))).toBe("5");
  });
  test("Rust integer literal", () => {
    const t = "const MAX: u32 = 5;";
    expect(analyzer.extractValue(t, "rust", region(t, "MAX: u32 = 5"))).toBe(
      "5",
    );
  });
  test("Go int literal", () => {
    const t = "const Max = 5";
    expect(analyzer.extractValue(t, "go", region(t, "Max = 5"))).toBe("5");
  });
});

describe("end-to-end value veto: a 5 → 50 change trips even at the boundary (§4, §17.3)", () => {
  test("value selector catches a boundary insertion the text tier misses", () => {
    const original = "export const MAX_ATTEMPTS = 5;\n";
    const anchor = buildAnchor(
      "src/retry.ts",
      original,
      region(original, "MAX_ATTEMPTS = 5"),
      {
        language: "typescript",
        analyzer,
      },
    );
    // The anchor must carry both an ast-node and a value selector.
    expect(anchor.selectors.some((s) => s.kind === "ast-node")).toBe(true);
    const valueSel = anchor.selectors.find((s) => s.kind === "value");
    expect(valueSel).toBeDefined();

    const assertion: Assertion = {
      id: "asrt_1",
      propositionId: "p",
      documentId: "d",
      owner: "x",
      ref: "r",
      anchor,
      attrs: {},
    };
    const changed = "export const MAX_ATTEMPTS = 50;\n";
    const verdict = resolveAssertion(assertion, changed, { ast: analyzer });
    // Despite text similarity ~1.0, the value veto forces stale.
    expect(verdict.state).toBe("stale");
    expect(verdict.notes.join(" ")).toContain("value veto");
  });

  test("an unchanged file grades fresh with full corroboration", () => {
    const original = "export const MAX_ATTEMPTS = 5;\n";
    const anchor = buildAnchor(
      "src/retry.ts",
      original,
      region(original, "MAX_ATTEMPTS = 5"),
      {
        language: "typescript",
        analyzer,
      },
    );
    const assertion: Assertion = {
      id: "a",
      propositionId: "p",
      documentId: "d",
      owner: "x",
      ref: "r",
      anchor,
      attrs: {},
    };
    const verdict = resolveAssertion(assertion, original, { ast: analyzer });
    expect(verdict.state).toBe("fresh");
    expect(verdict.confidence).toBeCloseTo(1, 5);
  });
});
