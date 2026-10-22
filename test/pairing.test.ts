import { describe, expect, test } from "bun:test";
import type { Claim, Enforcement } from "../src/core/model.ts";
import {
  contentTokens,
  pairClaims,
  pairIdOf,
  RARE_MIN_CLAIMS,
  RARE_SHARE,
  rareCutoff,
  STOP_WORDS,
} from "../src/engine/pairing.ts";

function claim(
  n: number,
  sentence: string,
  opts: { file?: string; enforcement?: Enforcement } = {},
): Claim {
  const file = opts.file ?? `docs/doc${n}.md`;
  return {
    id: `clm_${n.toString(16).padStart(16, "0")}`,
    documentId: `doc_${Bun.hash.xxHash64(file).toString(16).padStart(16, "0")}`,
    kind: "attested",
    sentence,
    doc: {
      file,
      selectors: [
        { kind: "text-quote", exact: sentence, prefix: "", suffix: "" },
      ],
    },
    evidence: [],
    sources: [],
    procedure: null,
    ttl: "P90D",
    enforcement: opts.enforcement ?? "enforced",
    attestations: [],
    createdAt: "2026-10-22T00:00:00Z",
    ref: "HEAD",
  };
}

/** Filler claims that share nothing with each other or with the claims under test. */
function filler(from: number, count: number): Claim[] {
  return Array.from({ length: count }, (_, i) =>
    claim(from + i, `Filler${"q".repeat(i + 1)} statement.`),
  );
}

describe("contentTokens", () => {
  test("case-folded words of 3+ letters without stop words", () => {
    expect(
      contentTokens("The Cache is in .hibi/cache.json, not ~/.cache!"),
    ).toEqual(["cache", "hibi", "json"]);
  });

  test("the stop-word list is a fixed, case-folded set", () => {
    expect(STOP_WORDS.has("the")).toBe(true);
    for (const w of STOP_WORDS) expect(w).toBe(w.toLowerCase());
  });
});

test("rarity cutoff: fewer than max(RARE_MIN_CLAIMS, 5% of claims)", () => {
  expect(RARE_SHARE).toBe(0.05);
  expect(rareCutoff(10)).toBe(RARE_MIN_CLAIMS);
  expect(rareCutoff(200)).toBe(10);
  expect(rareCutoff(1000)).toBe(50);
});

describe("pairClaims", () => {
  test("a shared rare token pairs two claims; why names it", () => {
    const pairs = pairClaims([
      claim(1, "Stripe bills the Pro plan monthly."),
      claim(2, "Refunds go through Stripe."),
      claim(3, "The cache lives under the hibi folder."),
    ]);
    expect(pairs.map((p) => [p.pairId, p.why])).toEqual([
      [
        "clm_0000000000000001:clm_0000000000000002",
        "shared rare tokens: stripe",
      ],
    ]);
  });

  test("a token in too many claims does not pair on its own", () => {
    // `hibi` is in 4 of 4 claims; cutoff is max(3, 0.2) = 3.
    const pairs = pairClaims([
      claim(1, "hibi reads alpha."),
      claim(2, "hibi reads beta."),
      claim(3, "hibi reads gamma."),
      claim(4, "hibi reads delta."),
    ]);
    expect(pairs).toEqual([]);
  });

  test("both stating values and sharing any token pairs them", () => {
    const claims = [
      claim(1, "hibi retries 3 times."),
      claim(2, "hibi retries 5 times."),
      claim(3, "hibi retries many times."),
      claim(4, "hibi retries twice."),
    ];
    const pairs = pairClaims(claims);
    expect(pairs.map((p) => [p.pairId, p.why])).toEqual([
      [
        "clm_0000000000000001:clm_0000000000000002",
        "both state number values and share: hibi, retries, times",
      ],
    ]);
  });

  test("values of different kinds or units do not pair through a common token", () => {
    const pairs = pairClaims([
      claim(1, "hibi retries 3 times."),
      claim(2, "hibi retries for 5 days."),
      claim(3, "hibi retries at $29."),
      claim(4, "hibi retries 2% of calls."),
    ]);
    expect(pairs).toEqual([]);
  });

  test("a value on one side only does not pair through a common token", () => {
    const pairs = pairClaims([
      claim(1, "hibi retries 3 times."),
      claim(2, "hibi retries often."),
      claim(3, "hibi retries forever."),
    ]);
    expect(pairs).toEqual([]);
  });

  test("both rules on one pair are both named", () => {
    const [pair] = pairClaims([
      claim(1, "Stripe charges $29 for hibi."),
      claim(2, "Stripe charges $39 for hibi."),
      claim(3, "hibi is free."),
      claim(4, "hibi is open."),
    ]);
    expect(pair?.why).toBe(
      "shared rare tokens: stripe, charges; both state money USD values and share: stripe, charges, hibi",
    );
  });

  test("retired claims are not paired", () => {
    const pairs = pairClaims([
      claim(1, "Stripe bills monthly."),
      claim(2, "Stripe bills yearly.", { enforcement: "retired" }),
    ]);
    expect(pairs).toEqual([]);
  });

  test("claims in the same document pair by the same rule", () => {
    const pairs = pairClaims([
      claim(1, "Stripe bills monthly.", { file: "docs/a.md" }),
      claim(2, "Stripe bills yearly.", { file: "docs/a.md" }),
    ]);
    expect(pairs).toHaveLength(1);
  });

  test("pair ids are sorted ids; a has the smaller id; order does not depend on input", () => {
    const claims = [
      claim(9, "Stripe bills monthly."),
      claim(2, "Stripe bills yearly."),
      claim(5, "Postgres stores claims."),
      claim(7, "Postgres stores documents."),
      ...filler(100, 20),
    ];
    const forward = pairClaims(claims);
    const reversed = pairClaims([...claims].reverse());
    expect(forward.map((p) => p.pairId)).toEqual(reversed.map((p) => p.pairId));
    expect(forward.map((p) => p.pairId)).toEqual([
      "clm_0000000000000002:clm_0000000000000009",
      "clm_0000000000000005:clm_0000000000000007",
    ]);
    for (const p of forward) {
      expect(p.a.id < p.b.id).toBe(true);
      expect(p.pairId).toBe(pairIdOf(p.b.id, p.a.id));
    }
  });
});

/** Deterministic PRNG (mulberry32). */
function rng(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Measured on this corpus (seed 42): 841 of 19,900 pairs (4.2%); 748 from
// rare tokens alone, 93 more from the values rule. Pairing on any two values
// instead of values in the same group would give 1,361.
test("a 200-claim corpus pairs far fewer than all pairs", () => {
  const random = rng(42);
  // Zipf-like vocabulary: a few words are common, most are rare.
  const vocab = Array.from(
    { length: 400 },
    (_, i) =>
      `word${"abcdefghij"[i % 10]}${"klmnopqrst"[Math.floor(i / 10) % 10]}${"uvwxyzabcd"[Math.floor(i / 100)]}`,
  );
  const weights = vocab.map((_, i) => 1 / (i + 1));
  const total = weights.reduce((s, w) => s + w, 0);
  const pick = () => {
    let r = random() * total;
    for (let i = 0; i < vocab.length; i++) {
      r -= weights[i] as number;
      if (r <= 0) return vocab[i] as string;
    }
    return vocab[vocab.length - 1] as string;
  };
  // Values of mixed kinds and units, as in real docs.
  const valueText = () => {
    const n = Math.floor(random() * 100);
    const forms = [
      `${n}`,
      `$${n}`,
      `${n}%`,
      `${n} days`,
      `${n}ms`,
      `v1.${n}.0`,
      `2026-01-${String((n % 28) + 1).padStart(2, "0")}`,
    ];
    return forms[Math.floor(random() * forms.length)] as string;
  };
  const claims = Array.from({ length: 200 }, (_, i) => {
    const words = Array.from({ length: 6 }, pick);
    const value = random() < 0.25 ? ` ${valueText()}` : "";
    return claim(i + 1, `${words.join(" ")}${value}.`);
  });
  const allPairs = (200 * 199) / 2;
  const pairs = pairClaims(claims);
  expect(pairs.length).toBe(841);
  expect(pairs.length).toBeLessThan(allPairs * 0.05);
  // Deterministic: same corpus, same pairs.
  expect(pairClaims(claims).map((p) => p.pairId)).toEqual(
    pairs.map((p) => p.pairId),
  );
});
