/**
 * Text normalization and similarity for the Tier-1 text comparison (§17.2).
 */

/**
 * Normalize text for comparison (§17.2): per line strip leading whitespace, then
 * collapse interior whitespace runs (including newlines) to a single space. The
 * effect: a pure reindent/reflow normalizes to identical text and scores 1.0.
 */
export function normalizeText(s: string): string {
  return s
    .split(/\r?\n/)
    .map((line) => line.replace(/^[ \t\f\v ]+/, ""))
    .join("\n")
    .replace(/\s+/g, " ")
    .trim();
}

/** Whitespace-collapsed form for `value`-tier equality (§17.2). */
export function collapseWhitespace(s: string): string {
  return s.replace(/\s+/g, " ").trim();
}

/** Levenshtein edit distance (two-row DP). */
export function levenshtein(a: string, b: string): number {
  if (a === b) return 0;
  if (a.length === 0) return b.length;
  if (b.length === 0) return a.length;

  let prev = new Array<number>(b.length + 1);
  let curr = new Array<number>(b.length + 1);
  for (let j = 0; j <= b.length; j++) prev[j] = j;

  for (let i = 1; i <= a.length; i++) {
    curr[0] = i;
    const ca = a.charCodeAt(i - 1);
    for (let j = 1; j <= b.length; j++) {
      const cost = ca === b.charCodeAt(j - 1) ? 0 : 1;
      const del = (prev[j] ?? 0) + 1;
      const ins = (curr[j - 1] ?? 0) + 1;
      const sub = (prev[j - 1] ?? 0) + cost;
      curr[j] = Math.min(del, ins, sub);
    }
    [prev, curr] = [curr, prev];
  }
  return prev[b.length] ?? 0;
}

/**
 * Normalized text similarity (§17.2): normalize both sides, then
 * `max(0, 1 − editDistance / maxLen)`, returning 1 on post-normalization
 * equality. A pure reindent/reflow scores 1.0.
 */
export function textSimilarity(a: string, b: string): number {
  const na = normalizeText(a);
  const nb = normalizeText(b);
  if (na === nb) return 1;
  const maxLen = Math.max(na.length, nb.length);
  if (maxLen === 0) return 1;
  const dist = levenshtein(na, nb);
  return Math.max(0, 1 - dist / maxLen);
}
