/**
 * Deterministic behavioral classification (§17.6). A claim is a
 * *behavioral-candidate* if `claimKind` is declared, or this keyword heuristic
 * matches ordering / retry / complexity / concurrency / caching / validation /
 * error language. Classification is **not a verdict**: it only decides whether
 * the change-gate is consulted; it never sets a state by itself.
 */
import type { ClaimKind } from "../core/model.ts";

const PATTERNS: { kind: ClaimKind; re: RegExp }[] = [
  {
    kind: "ordering",
    re: /\b(sort(s|ed|ing)?|order(s|ed|ing|ing)?|ascending|descending|alphabetical|sequence|sequential)\b/i,
  },
  {
    kind: "retry",
    re: /\b(retr(y|ies|ied|ying)|back-?off|re-?attempt|attempts?|times?\s*out|timed?\s*out|timeout)\b/i,
  },
  {
    kind: "complexity",
    re: /(\bO\([^)]*\)|\b(time|space)\s+complexity\b|\b(linear|constant|logarithmic|quadratic|amortized)\b)/i,
  },
  {
    kind: "concurrency",
    re: /\b(concurren\w*|thread[-\s]?safe|atomic\w*|race\s+condition|deadlock|mutex|lock(s|ed|ing)?|parallel\w*|synchroniz\w*)\b/i,
  },
  {
    kind: "caching",
    re: /\b(cach\w*|memoiz\w*|invalidat\w*|evict\w*)\b/i,
  },
  {
    kind: "validation",
    re: /\b(validat\w*|sanitiz\w*|reject(s|ed|ing)?|required|constraint\w*|enforc\w*)\b/i,
  },
  {
    kind: "error-handling",
    re: /\b(error\w*|exception\w*|throw(s|n|ing)?|fail(s|ed|ure|ing)?|fallback|recover\w*|raise(s|d)?)\b/i,
  },
];

/** Classify a claim's text into a behavioral `ClaimKind`, or `undefined`. */
export function classifyClaimKind(text: string): ClaimKind | undefined {
  for (const p of PATTERNS) if (p.re.test(text)) return p.kind;
  return undefined;
}

/**
 * The effective claim kind for an assertion: the author's declaration wins;
 * otherwise the heuristic classifies the live documented text. `undefined`
 * means non-behavioral (no behavioral state is computed — §10).
 */
export function effectiveClaimKind(
  declared: ClaimKind | undefined,
  liveText: string | null,
): ClaimKind | undefined {
  if (declared) return declared;
  return liveText ? classifyClaimKind(liveText) : undefined;
}
