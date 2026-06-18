import { beforeAll, describe, expect, test } from "bun:test";
import { resolveAssertion } from "../src/algo/resolve.ts";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import type { Assertion } from "../src/core/model.ts";
import { buildAnchor } from "../src/engine/anchor.ts";

let analyzer: Awaited<ReturnType<typeof getAnalyzer>>;
beforeAll(async () => {
  analyzer = await getAnalyzer();
});

/** Small deterministic PRNG (mulberry32) so the fuzz run is reproducible. */
function rng(seed: number) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function anchorOn(original: string, quote: string, language = "typescript") {
  const start = original.indexOf(quote);
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

const BASES = [
  {
    src: "export const MAX_ATTEMPTS = 5;\nexport function go() { return 1; }\n",
    quote: "MAX_ATTEMPTS = 5",
  },
  {
    src: "function calc(x: number) {\n  return x * 2;\n}\nconst y = 10;\n",
    quote: "return x * 2",
  },
  {
    src: 'const cfg = { mode: "strict", limit: 7 };\nexport default cfg;\n',
    quote: "limit: 7",
  },
];

/** A neutral edit: changes the file but NOT the anchored construct's meaning. */
function neutralEdit(
  src: string,
  anchorQuote: string,
  r: () => number,
): string {
  const idx = src.indexOf(anchorQuote);
  const after = idx + anchorQuote.length;
  const choice = Math.floor(r() * 5);
  switch (choice) {
    case 0: // blank lines after the anchor line
      return `${src.slice(0, after)}\n\n${src.slice(after)}`;
    case 1: // a comment appended at EOF
      return `${src}// note ${Math.floor(r() * 1000)}\n`;
    case 2: // an unrelated function appended at EOF
      return `${src}export function extra${Math.floor(r() * 1000)}() { return 0; }\n`;
    case 3: // trailing whitespace on the last line
      return src.replace(/\n$/, "   \n");
    default: // a comment line at EOF (keeps anchor start stable)
      return `${src}// trailing comment\n`;
  }
}

/** A real drift: mutate the anchored literal/identifier. */
function driftEdit(src: string, anchorQuote: string, r: () => number): string {
  // Replace the first digit run inside the anchored quote with a different number.
  const idx = src.indexOf(anchorQuote);
  const region = src.slice(idx, idx + anchorQuote.length);
  if (/\d/.test(region)) {
    const mutated = region.replace(/\d+/, (m) =>
      String(Number(m) + 1 + Math.floor(r() * 90)),
    );
    return src.slice(0, idx) + mutated + src.slice(idx + anchorQuote.length);
  }
  // No digit → mutate an identifier/operator within the region.
  const mutated = region
    .replace(/[a-zA-Z]+/, (m) => `${m}X`)
    .replace(/\*/, "+");
  return src.slice(0, idx) + mutated + src.slice(idx + anchorQuote.length);
}

describe("precision rate (§10 — false-stale ≤ ~2%, never fresh-on-drift)", () => {
  test("neutral edits keep the false-positive (stale/ghost) rate at or below 2%", () => {
    const r = rng(12345);
    let total = 0;
    let falsePositive = 0;
    for (const base of BASES) {
      const resolve = anchorOn(base.src, base.quote);
      for (let i = 0; i < 40; i++) {
        const edited = neutralEdit(base.src, base.quote, r);
        const state = resolve(edited).state;
        total++;
        if (state === "stale" || state === "ghost") falsePositive++;
      }
    }
    const rate = falsePositive / total;
    expect(total).toBe(120);
    expect(rate).toBeLessThanOrEqual(0.02);
  });

  test("real drifts are NEVER graded fresh (every missed drift is at least re-verify)", () => {
    const r = rng(98765);
    let total = 0;
    let falseFresh = 0;
    for (const base of BASES) {
      const resolve = anchorOn(base.src, base.quote);
      for (let i = 0; i < 40; i++) {
        const drifted = driftEdit(base.src, base.quote, r);
        if (drifted === base.src) continue; // skip non-mutations
        const state = resolve(drifted).state;
        total++;
        if (state === "fresh") falseFresh++;
      }
    }
    expect(total).toBeGreaterThan(0);
    expect(falseFresh).toBe(0);
  });
});
