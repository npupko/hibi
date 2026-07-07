/**
 * Deterministic behavioral classification (§17.6, D12). A claim is a
 * *behavioral-candidate* when the author declares it (`behavioral: true`), when
 * it links a verifier, or when this keyword heuristic matches ordering / retry /
 * complexity / concurrency / caching / validation / error language. The result
 * is a single boolean — a **label, never a verdict**: it only decides whether the
 * change-gate is consulted; it never sets a behavioral state by itself.
 *
 * There is no enum of behavioral *kinds* (D12): the 7 keyword families below
 * collapse into one boolean OR. An author who wants to label the flavor of a
 * claim puts it in the open `attrs` bag; the engine does not interpret it.
 */

/** The keyword families that mark a claim as a behavioral-candidate (§17.6). */
const PATTERNS: RegExp[] = [
  // ordering / comparison
  /\b(sort(s|ed|ing)?|order(s|ed|ing)?|ascending|descending|alphabetical|sequence|sequential)\b/i,
  // retry / temporal-sequencing
  /\b(retr(y|ies|ied|ying)|back-?off|re-?attempt|attempts?|times?\s*out|timed?\s*out|timeout)\b/i,
  // complexity
  /(\bO\([^)]*\)|\b(time|space)\s+complexity\b|\b(linear|constant|logarithmic|quadratic|amortized)\b)/i,
  // concurrency
  /\b(concurren\w*|thread[-\s]?safe|atomic\w*|race\s+condition|deadlock|mutex|lock(s|ed|ing)?|parallel\w*|synchroniz\w*)\b/i,
  // caching
  /\b(cach\w*|memoiz\w*|invalidat\w*|evict\w*)\b/i,
  // validation
  /\b(validat\w*|sanitiz\w*|reject(s|ed|ing)?|required|constraint\w*|enforc\w*)\b/i,
  // exception / error language
  /\b(error\w*|exception\w*|throw(s|n|ing)?|fail(s|ed|ure|ing)?|fallback|recover\w*|raise(s|d)?)\b/i,
];

/** Whether the documented text reads as a behavioral claim (keyword heuristic). */
export function classifyBehavioral(text: string): boolean {
  return PATTERNS.some((re) => re.test(text));
}

/**
 * The classification rule (§17.6, D12), stated once and used everywhere:
 *   - `behavioral === true`  → behavioral (wording irrelevant);
 *   - `behavioral === false` → not behavioral (heuristic skipped; the schema
 *     already guarantees `verifiers[]` is empty in this case);
 *   - `behavioral` absent    → behavioral iff the heuristic matches the live
 *     documented text OR the claim links at least one verifier.
 */
export function isBehavioral(
  behavioral: boolean | undefined,
  liveText: string | null,
  hasVerifiers: boolean,
): boolean {
  if (behavioral === true) return true;
  if (behavioral === false) return false;
  if (hasVerifiers) return true;
  return liveText ? classifyBehavioral(liveText) : false;
}
