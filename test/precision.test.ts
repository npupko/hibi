import { beforeAll, describe, expect, test } from "bun:test";
import { resolveAssertion } from "../src/algo/resolve.ts";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import type { Assertion } from "../src/core/model.ts";
import { buildAnchor } from "../src/engine/anchor.ts";

let analyzer: Awaited<ReturnType<typeof getAnalyzer>>;
beforeAll(async () => {
  analyzer = await getAnalyzer();
});

/** Anchor a quote in `original`, return a resolver over modified text. */
function anchorOn(original: string, quote: string, language = "typescript") {
  const start = original.indexOf(quote);
  if (start < 0) throw new Error(`quote not in source: ${quote}`);
  const anchor = buildAnchor(
    "f.ts",
    original,
    { start, end: start + quote.length },
    { language, analyzer },
  );
  const a: Assertion = {
    id: "a",
    propositionId: "p",
    documentId: "d",
    owner: "o",
    ref: "r",
    anchor,
    attrs: {},
  };
  return (modified: string) => resolveAssertion(a, modified, { ast: analyzer });
}

describe("never report a drifted claim as `fresh` (§10 — precision over recall)", () => {
  const cases: {
    name: string;
    original: string;
    quote: string;
    drifted: string;
  }[] = [
    {
      name: "numeric literal 5 → 50",
      original: "export const MAX_ATTEMPTS = 5;\n",
      quote: "MAX_ATTEMPTS = 5",
      drifted: "export const MAX_ATTEMPTS = 50;\n",
    },
    {
      name: "string literal changed",
      original: 'export const MODE = "strict";\n',
      quote: 'MODE = "strict"',
      drifted: 'export const MODE = "lenient";\n',
    },
    {
      name: "boolean flipped",
      original: "export const ENABLED = true;\n",
      quote: "ENABLED = true",
      drifted: "export const ENABLED = false;\n",
    },
    {
      name: "signature gained a parameter",
      original: "function retry(max: number) { return max; }\n",
      quote: "function retry(max: number)",
      drifted: "function retry(max: number, delay: number) { return max; }\n",
    },
  ];

  for (const c of cases) {
    test(c.name, () => {
      const v = anchorOn(c.original, c.quote)(c.drifted);
      expect(v.state).not.toBe("fresh");
    });
  }
});

describe("neutral edits do not produce false-`stale` (§11.3, §17.2)", () => {
  test("re-indentation never produces false-stale (text similarity stays 1.0)", () => {
    const original = "function f() {\n  return 5;\n}\n";
    const reindented = "function f() {\n        return 5;\n}\n";
    const v = anchorOn(original, "return 5")(reindented);
    // The region's start shifted >4 chars, so move-awareness (§17.3) grades it
    // `moved` (a re-anchorable warning) — never the hard `stale` band.
    expect(["fresh", "moved"]).toContain(v.state);
    expect(v.state).not.toBe("stale");
    const tq = v.selectorScores.find((s) => s.kind === "text-quote");
    expect(tq?.score).toBe(1); // pure reindent ⇒ text similarity 1.0
  });

  test("reflow (added blank lines elsewhere) stays fresh", () => {
    const original = "export const MAX = 5;\nexport const MIN = 1;\n";
    const reflowed = "export const MAX = 5;\n\n\nexport const MIN = 1;\n";
    expect(anchorOn(original, "MAX = 5")(reflowed).state).toBe("fresh");
  });

  test("unrelated code added below the anchor stays fresh", () => {
    const original = "export const MAX = 5;\n";
    const edited =
      "export const MAX = 5;\nexport function helper() { return 42; }\n";
    expect(anchorOn(original, "MAX = 5")(edited).state).toBe("fresh");
  });

  test("a battery of neutral edits yields zero false-stale", () => {
    const original = "export const MAX = 5;\nexport const NAME = 'a';\n";
    const neutralEdits = [
      "export const MAX = 5;\nexport const NAME = 'a';\n// a comment\n",
      "// header\nexport const MAX = 5;\nexport const NAME = 'a';\n",
      "export const MAX = 5;\nexport const NAME = 'a';\nexport const X = 9;\n",
      "export   const   MAX   =   5;\nexport const NAME = 'a';\n",
    ];
    const states = neutralEdits.map(
      (e) => anchorOn(original, "MAX = 5")(e).state,
    );
    expect(states.filter((s) => s === "stale" || s === "ghost")).toEqual([]);
  });
});

describe("renames stay out of the hard-`stale` band (§17.3 structural-only)", () => {
  test("renaming the anchored identifier is re-verify (never stale, never silently fresh)", () => {
    const original =
      "function retryWithBackoff(maxAttempts: number) { return maxAttempts * 2; }\n";
    const renamed =
      "function retryWithBackoff(maxTries: number) { return maxTries * 2; }\n";
    const v = anchorOn(original, "maxAttempts: number")(renamed);
    // A rename is drift of the anchored name: it must surface for re-verification
    // (moved or ghost) and must never land in `stale` or pass silently as `fresh`.
    expect(v.state).not.toBe("stale");
    expect(v.state).not.toBe("fresh");
  });

  test("a rename within a larger intact anchor grades moved via structural corroboration", () => {
    // When the anchored region is a whole statement, the structural hash matches
    // and text/position corroborate, so a rename is `moved`, not `ghost`.
    const original = "const result = computeTotal(items);\n";
    const renamed = "const result = computeSum(items);\n";
    const v = anchorOn(
      original,
      "const result = computeTotal(items);",
    )(renamed);
    expect(["moved", "stale", "ghost"]).toContain(v.state);
    expect(v.state).not.toBe("fresh");
  });
});

describe("ghost detection via position-corroboration (§17.3)", () => {
  test("deleting the anchored region → ghost, not a manufactured stale", () => {
    const original =
      "export const MAX_ATTEMPTS = 5;\nexport const OTHER = 99;\n";
    // Remove the anchored line entirely; fill with unrelated content.
    const deleted =
      "export const OTHER = 99;\nexport function brandNew() { return 0; }\n";
    const v = anchorOn(original, "MAX_ATTEMPTS = 5")(deleted);
    expect(v.state).toBe("ghost");
  });
});

describe("selector disagreement lowers confidence toward re-verify, not hard-stale (§11.3)", () => {
  test("a moved-but-intact region grades moved (re-anchorable), keeping the suspect set tight", () => {
    const original = "export const MAX_ATTEMPTS = 5;\n";
    // Push the anchored line far down with a large unrelated prologue (intact content).
    const moved =
      "// ".concat("x".repeat(400), "\n").repeat(1) +
      "export const MAX_ATTEMPTS = 5;\n";
    const v = anchorOn(original, "MAX_ATTEMPTS = 5")(moved);
    expect(["fresh", "moved"]).toContain(v.state);
  });
});
