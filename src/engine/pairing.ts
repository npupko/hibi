/**
 * Pairing for the contradiction and duplicate pass (SPEC 9.1). Code picks the
 * candidate pairs; the judge only sees pairs that survive. All-pairs is the
 * wrong first step (design/evidence/typesafe-research-2026-10-22.md, section 8).
 *
 * Two live claims (enforcement other than `retired`) pair when either holds:
 *
 * 1. They share a rare content token: a token that appears in fewer than
 *    `max(RARE_MIN_CLAIMS, RARE_SHARE * number of live claims)` claims.
 * 2. Both sentences contain a value of the same kind and unit (the same
 *    `valueGroup`, so `compareValues` can compare them) and they share any
 *    content token.
 *
 * A content token is a case-folded run of letters, at least
 * `MIN_TOKEN_LETTERS` long, not in `STOP_WORDS`. Claims in the same document
 * pair by the same rule. `why` on each pair names the rule and the tokens.
 */
import type { Claim } from "../core/model.ts";
import { extractValues, valueGroup } from "./values.ts";

/** A token is rare when it appears in fewer than this share of live claims (SPEC 9.1: 5 percent). */
export const RARE_SHARE = 0.05;

/**
 * Floor for the rarity cutoff. With the 5 percent rule alone, a store of fewer
 * than 40 claims has a cutoff of 2 or less, and a token found in two claims
 * would never count as rare. With a floor of 3, a token shared by exactly two
 * claims is always rare.
 */
export const RARE_MIN_CLAIMS = 3;

/** Shortest token, in letters, that counts as content. */
export const MIN_TOKEN_LETTERS = 3;

/** Fixed English stop words removed before pairing. Case-folded. */
export const STOP_WORDS: ReadonlySet<string> = new Set([
  "about",
  "above",
  "after",
  "again",
  "against",
  "all",
  "also",
  "always",
  "and",
  "any",
  "are",
  "because",
  "been",
  "before",
  "being",
  "below",
  "between",
  "both",
  "but",
  "can",
  "cannot",
  "could",
  "did",
  "does",
  "doing",
  "down",
  "during",
  "each",
  "either",
  "every",
  "few",
  "for",
  "from",
  "further",
  "had",
  "has",
  "have",
  "having",
  "her",
  "here",
  "hers",
  "him",
  "his",
  "how",
  "into",
  "its",
  "itself",
  "just",
  "may",
  "might",
  "more",
  "most",
  "must",
  "never",
  "nor",
  "not",
  "now",
  "off",
  "once",
  "one",
  "only",
  "other",
  "our",
  "ours",
  "out",
  "over",
  "own",
  "same",
  "shall",
  "she",
  "should",
  "some",
  "such",
  "than",
  "that",
  "the",
  "their",
  "theirs",
  "them",
  "then",
  "there",
  "these",
  "they",
  "this",
  "those",
  "through",
  "too",
  "under",
  "until",
  "upon",
  "use",
  "used",
  "uses",
  "using",
  "very",
  "via",
  "was",
  "were",
  "what",
  "when",
  "where",
  "whether",
  "which",
  "while",
  "who",
  "whom",
  "why",
  "will",
  "with",
  "within",
  "without",
  "would",
  "yet",
  "you",
  "your",
  "yours",
]);

export interface Pair {
  /** The claim with the smaller id. */
  a: Claim;
  b: Claim;
  /** The two claim ids, sorted, joined with `:`. */
  pairId: string;
  /** Why the pair was formed, e.g. `shared rare tokens: stripe` or `both state money USD values and share: price, plan`. */
  why: string;
}

export interface PairingOptions {
  /** Overrides `RARE_SHARE`; for tests. */
  rareShare?: number;
  /** Overrides `RARE_MIN_CLAIMS`; for tests. */
  rareMinClaims?: number;
}

const TOKEN = /\p{L}+/gu;

/** The content tokens of a sentence, deduplicated, in order of first appearance. */
export function contentTokens(text: string): string[] {
  const out: string[] = [];
  for (const m of text.toLowerCase().matchAll(TOKEN)) {
    const t = m[0];
    if ([...t].length < MIN_TOKEN_LETTERS || STOP_WORDS.has(t)) continue;
    if (!out.includes(t)) out.push(t);
  }
  return out;
}

/** `clm_a:clm_b` with the ids sorted. */
export function pairIdOf(x: string, y: string): string {
  return x < y ? `${x}:${y}` : `${y}:${x}`;
}

/** The rarity cutoff for `n` live claims: a token is rare when its claim count is below this. */
export function rareCutoff(
  n: number,
  share = RARE_SHARE,
  min = RARE_MIN_CLAIMS,
): number {
  return Math.max(min, share * n);
}

export function pairClaims(claims: Claim[], opts: PairingOptions = {}): Pair[] {
  const live = claims
    .filter((c) => c.enforcement !== "retired")
    .sort((x, y) => (x.id < y.id ? -1 : x.id > y.id ? 1 : 0));
  const tokens = live.map((c) => new Set(contentTokens(c.sentence)));
  const valueGroups = live.map(
    (c) => new Set(extractValues(c.sentence).map(valueGroup)),
  );
  const hasValue = valueGroups.map((g) => g.size > 0);

  const postings = new Map<string, number[]>();
  tokens.forEach((set, i) => {
    for (const t of set) {
      const list = postings.get(t);
      if (list) list.push(i);
      else postings.set(t, [i]);
    }
  });
  const cutoff = rareCutoff(live.length, opts.rareShare, opts.rareMinClaims);
  const isRare = (t: string) => (postings.get(t)?.length ?? 0) < cutoff;

  // Candidate pairs: claims that share any token. Rule 1 needs a rare token,
  // rule 2 needs values in a common group; both are checked per candidate below.
  const seen = new Set<string>();
  const candidates: [number, number][] = [];
  for (const [t, list] of postings) {
    const viaRare = isRare(t);
    for (let x = 0; x < list.length; x++) {
      const i = list[x] as number;
      if (!viaRare && !hasValue[i]) continue;
      for (let y = x + 1; y < list.length; y++) {
        const j = list[y] as number;
        if (!viaRare && !hasValue[j]) continue;
        const key = `${i},${j}`;
        if (seen.has(key)) continue;
        seen.add(key);
        candidates.push([i, j]);
      }
    }
  }

  const pairs: Pair[] = [];
  for (const [i, j] of candidates) {
    const a = live[i] as Claim;
    const b = live[j] as Claim;
    const tb = tokens[j] as Set<string>;
    const shared = [...(tokens[i] as Set<string>)].filter((t) => tb.has(t));
    const rare = shared.filter(isRare);
    const parts: string[] = [];
    if (rare.length > 0) parts.push(`shared rare tokens: ${rare.join(", ")}`);
    const gj = valueGroups[j] as Set<string>;
    const sameGroups = [...(valueGroups[i] as Set<string>)].filter((g) =>
      gj.has(g),
    );
    if (sameGroups.length > 0 && shared.length > 0) {
      parts.push(
        `both state ${sameGroups.join(", ")} values and share: ${shared.join(", ")}`,
      );
    }
    if (parts.length === 0) continue;
    pairs.push({ a, b, pairId: pairIdOf(a.id, b.id), why: parts.join("; ") });
  }
  return pairs.sort((x, y) =>
    x.pairId < y.pairId ? -1 : x.pairId > y.pairId ? 1 : 0,
  );
}
