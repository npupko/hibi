/**
 * Bitap fuzzy substring location — the Google diff-match-patch `match_main`
 * algorithm, vendored and owned (§16, D8). No dependency-healthy package does
 * fuzzy *substring location*, and the spec's §17 parameters presume this exact
 * semantics, so the ~150-line matcher lives in-tree.
 *
 * Reference: google/diff-match-patch `match_main` / `match_bitap_` /
 * `match_alphabet_` / `match_bitapScore_`.
 *
 * Returns the best match index into `text`, or -1 if nothing scores at or below
 * `matchThreshold`.
 */

export interface BitapOptions {
  /** Worst score still accepted; 0.0 perfect … 1.0 very loose. §17.1: 0.4. */
  matchThreshold: number;
  /**
   * How many characters from the expected `loc` a match may stray before
   * distance alone contributes 1.0 to the score. §17.1: 100000 (deliberately
   * large, so a region relocated by hundreds of chars is still found).
   */
  matchDistance: number;
}

/** The Bitap word size caps a pattern at 32 characters (a single 32-bit word). */
export const MATCH_MAX_BITS = 32;

export const DEFAULT_BITAP_OPTIONS: BitapOptions = {
  matchThreshold: 0.4,
  matchDistance: 100000,
};

/** Map each pattern char → bitmask of the positions where it occurs. */
function matchAlphabet(pattern: string): Map<string, number> {
  const s = new Map<string, number>();
  for (let i = 0; i < pattern.length; i++) s.set(pattern.charAt(i), 0);
  for (let i = 0; i < pattern.length; i++) {
    const c = pattern.charAt(i);
    s.set(c, s.get(c)! | (1 << (pattern.length - i - 1)));
  }
  return s;
}

/**
 * Locate `pattern` in `text` near `loc`. Mirrors diff-match-patch `match_main`,
 * including its short-circuits.
 */
export function matchMain(
  text: string,
  pattern: string,
  loc: number,
  opts: BitapOptions = DEFAULT_BITAP_OPTIONS,
): number {
  loc = Math.max(0, Math.min(loc, text.length));
  if (text === pattern) return 0;
  if (text.length === 0) return -1;
  if (text.substring(loc, loc + pattern.length) === pattern) return loc;
  return matchBitap(text, pattern, loc, opts);
}

function matchBitap(text: string, pattern: string, loc: number, opts: BitapOptions): number {
  if (pattern.length > MATCH_MAX_BITS) {
    throw new Error(`Bitap pattern too long (${pattern.length} > ${MATCH_MAX_BITS}).`);
  }
  const { matchThreshold, matchDistance } = opts;
  const s = matchAlphabet(pattern);

  const bitapScore = (e: number, x: number): number => {
    const accuracy = e / pattern.length;
    const proximity = Math.abs(loc - x);
    if (!matchDistance) return proximity ? 1.0 : accuracy;
    return accuracy + proximity / matchDistance;
  };

  let scoreThreshold = matchThreshold;

  // Tighten the threshold using any exact occurrences near loc.
  let bestLoc = text.indexOf(pattern, loc);
  if (bestLoc !== -1) {
    scoreThreshold = Math.min(bitapScore(0, bestLoc), scoreThreshold);
    bestLoc = text.lastIndexOf(pattern, loc + pattern.length);
    if (bestLoc !== -1) scoreThreshold = Math.min(bitapScore(0, bestLoc), scoreThreshold);
  }

  const matchmask = 1 << (pattern.length - 1);
  bestLoc = -1;

  let binMin: number;
  let binMid: number;
  let binMax = pattern.length + text.length;
  let lastRd: number[] = [];

  for (let d = 0; d < pattern.length; d++) {
    // Binary-search the furthest reach worth scanning at this error level d.
    binMin = 0;
    binMid = binMax;
    while (binMin < binMid) {
      if (bitapScore(d, loc + binMid) <= scoreThreshold) binMin = binMid;
      else binMax = binMid;
      binMid = Math.floor((binMax - binMin) / 2 + binMin);
    }
    binMax = binMid;

    let start = Math.max(1, loc - binMid + 1);
    const finish = Math.min(loc + binMid, text.length) + pattern.length;

    const rd: number[] = new Array(finish + 2);
    rd[finish + 1] = (1 << d) - 1;

    for (let j = finish; j >= start; j--) {
      const charMatch = s.get(text.charAt(j - 1)) ?? 0;
      if (d === 0) {
        rd[j] = ((rd[j + 1]! << 1) | 1) & charMatch; // exact (shift-or)
      } else {
        rd[j] =
          (((rd[j + 1]! << 1) | 1) & charMatch) |
          (((lastRd[j + 1]! | lastRd[j]!) << 1) | 1) |
          lastRd[j + 1]!;
      }
      if (rd[j]! & matchmask) {
        const sc = bitapScore(d, j - 1);
        if (sc <= scoreThreshold) {
          scoreThreshold = sc;
          bestLoc = j - 1;
          if (bestLoc > loc) {
            start = Math.max(1, 2 * loc - bestLoc);
          } else {
            break; // best location is before loc; no improvement leftward.
          }
        }
      }
    }
    // No score at d+1 errors can beat the threshold → stop.
    if (bitapScore(d + 1, loc) > scoreThreshold) break;
    lastRd = rd;
  }
  return bestLoc;
}
