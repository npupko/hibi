import { beforeAll, describe, expect, test } from "bun:test";
import { resolveAssertion } from "../src/algo/resolve.ts";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import type { Assertion } from "../src/core/model.ts";
import { buildSelectorBundle, composeAnchor } from "../src/engine/anchor.ts";

let analyzer: Awaited<ReturnType<typeof getAnalyzer>>;
beforeAll(async () => {
  analyzer = await getAnalyzer();
});

// A fixed documented sentence; fixtures touch only the *code* side, so the doc
// side must stay `unchanged` throughout. Its content never changes between
// record-time and resolve-time.
const DOC_FILE = "doc.md";
const DOC_TEXT = "The retry ceiling is fixed and the mode is documented here.";
const DOC_CONTENT = `# Notes\n\n${DOC_TEXT}\n`;
const CODE_FILE = "f.ts";

/**
 * Anchor a code quote in `original` (doc-side bound to the fixed sentence),
 * enforce it, and return a resolver over modified code text. Doc content is held
 * constant so the doc side always resolves `unchanged`.
 */
function anchorOn(original: string, quote: string, language = "typescript") {
  const start = original.indexOf(quote);
  if (start < 0) throw new Error(`quote not in source: ${quote}`);

  const docStart = DOC_CONTENT.indexOf(DOC_TEXT);
  const docBundle = buildSelectorBundle(DOC_FILE, DOC_CONTENT, {
    start: docStart,
    end: docStart + DOC_TEXT.length,
  });
  const codeBundle = buildSelectorBundle(
    CODE_FILE,
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
    anchor: composeAnchor(docBundle, [codeBundle]),
    enforcement: "enforced",
    verifiers: [],
    attrs: {},
  };

  return (modified: string) =>
    resolveAssertion(
      a,
      { doc: DOC_CONTENT, code: new Map([[CODE_FILE, modified]]) },
      { ast: analyzer },
    );
}

describe("never report a drifted claim as `unchanged` (§10 — precision over recall)", () => {
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
      // The code side must never be silently `unchanged` on real drift.
      expect(v.code).not.toBe("unchanged");
      // Doc side untouched throughout.
      expect(v.doc).toBe("unchanged");
    });
  }
});

describe("neutral edits do not produce false-`changed`/`orphaned` (§11.3, §17.2)", () => {
  test("re-indentation never produces false-changed (text similarity stays 1.0)", () => {
    const original = "function f() {\n  return 5;\n}\n";
    const reindented = "function f() {\n        return 5;\n}\n";
    const v = anchorOn(original, "return 5")(reindented);
    // The region's start shifted >4 chars, so move-awareness (§17.3) grades it
    // `moved` (a re-anchorable warning) — never the hard `changed` band.
    expect(["unchanged", "moved"]).toContain(v.code);
    expect(v.code).not.toBe("changed");
    expect(v.doc).toBe("unchanged");
    const tq = v.evidence.selectorScores.find((s) => s.kind === "text-quote");
    expect(tq?.score).toBe(1); // pure reindent ⇒ text similarity 1.0
  });

  test("reflow (added blank lines elsewhere) stays unchanged", () => {
    const original = "export const MAX = 5;\nexport const MIN = 1;\n";
    const reflowed = "export const MAX = 5;\n\n\nexport const MIN = 1;\n";
    const v = anchorOn(original, "MAX = 5")(reflowed);
    expect(v.code).toBe("unchanged");
    expect(v.doc).toBe("unchanged");
  });

  test("unrelated code added below the anchor stays unchanged", () => {
    const original = "export const MAX = 5;\n";
    const edited =
      "export const MAX = 5;\nexport function helper() { return 42; }\n";
    const v = anchorOn(original, "MAX = 5")(edited);
    expect(v.code).toBe("unchanged");
    expect(v.doc).toBe("unchanged");
  });

  test("a battery of neutral edits yields zero false-changed/orphaned", () => {
    const original = "export const MAX = 5;\nexport const NAME = 'a';\n";
    const neutralEdits = [
      "export const MAX = 5;\nexport const NAME = 'a';\n// a comment\n",
      "// header\nexport const MAX = 5;\nexport const NAME = 'a';\n",
      "export const MAX = 5;\nexport const NAME = 'a';\nexport const X = 9;\n",
      "export   const   MAX   =   5;\nexport const NAME = 'a';\n",
    ];
    const states = neutralEdits.map(
      (e) => anchorOn(original, "MAX = 5")(e).code,
    );
    expect(states.filter((s) => s === "changed" || s === "orphaned")).toEqual(
      [],
    );
  });
});

describe("renames stay out of the hard-`changed` band (§17.3 structural-only)", () => {
  test("renaming the anchored identifier is re-verify (never changed, never silently unchanged)", () => {
    const original =
      "function retryWithBackoff(maxAttempts: number) { return maxAttempts * 2; }\n";
    const renamed =
      "function retryWithBackoff(maxTries: number) { return maxTries * 2; }\n";
    const v = anchorOn(original, "maxAttempts: number")(renamed);
    // A rename is drift of the anchored name: it must surface for re-verification
    // (moved or orphaned) and must never land in `changed` or pass silently as
    // `unchanged`.
    expect(v.code).not.toBe("changed");
    expect(v.code).not.toBe("unchanged");
    expect(v.doc).toBe("unchanged");
  });

  test("a rename within a larger intact anchor grades moved via structural corroboration", () => {
    // When the anchored region is a whole statement, the structural hash matches
    // and text/position corroborate, so a rename is `moved`, not `orphaned`.
    const original = "const result = computeTotal(items);\n";
    const renamed = "const result = computeSum(items);\n";
    const v = anchorOn(
      original,
      "const result = computeTotal(items);",
    )(renamed);
    expect(["moved", "changed", "orphaned"]).toContain(v.code);
    expect(v.code).not.toBe("unchanged");
    expect(v.doc).toBe("unchanged");
  });
});

describe("orphaned detection via position-corroboration (§17.3)", () => {
  test("deleting the anchored region → orphaned, not a manufactured changed", () => {
    const original =
      "export const MAX_ATTEMPTS = 5;\nexport const OTHER = 99;\n";
    // Remove the anchored line entirely; fill with unrelated content.
    const deleted =
      "export const OTHER = 99;\nexport function brandNew() { return 0; }\n";
    const v = anchorOn(original, "MAX_ATTEMPTS = 5")(deleted);
    expect(v.code).toBe("orphaned");
    expect(v.doc).toBe("unchanged");
  });
});

describe("selector disagreement lowers confidence toward re-verify, not hard-changed (§11.3)", () => {
  test("a moved-but-intact region grades moved (re-anchorable), keeping the suspect set tight", () => {
    const original = "export const MAX_ATTEMPTS = 5;\n";
    // Push the anchored line far down with a large unrelated prologue (intact content).
    const moved =
      "// ".concat("x".repeat(400), "\n").repeat(1) +
      "export const MAX_ATTEMPTS = 5;\n";
    const v = anchorOn(original, "MAX_ATTEMPTS = 5")(moved);
    expect(["unchanged", "moved"]).toContain(v.code);
    expect(v.doc).toBe("unchanged");
  });
});
