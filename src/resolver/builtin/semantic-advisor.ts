/**
 * Tier-3 semantic advisor (§6, §7.4) — the quarantined, opt-in advisory resolver.
 *
 * The deterministic tiers (1–2) detect *structural* change; they do NOT judge
 * whether a natural-language *behavioral* claim is still true ("sorts ascending",
 * "retries on timeout", an "O(n)" complexity claim). This advisor surfaces such
 * claims for re-verification. It **advises and never gates**: it returns
 * advisories only — never a verdict — so the deterministic verdict always stands.
 *
 * This implementation is itself deterministic (keyword surfacing). An LLM-backed
 * "is it still true?" advisor would plug in identically behind this same contract,
 * out-of-process — but the core has no model in the loop (§11.1).
 */
import type { DescribeResult, ResolveParams, ResolveResult } from "../protocol.ts";
import type { ResolverHandler } from "../server.ts";

/** Phrases that signal a behavioral claim the structural tiers cannot verify. */
const BEHAVIORAL_PATTERNS: { re: RegExp; why: string }[] = [
  { re: /\bsort(s|ed|ing)?\b|ascending|descending/i, why: "ordering claim" },
  { re: /\bretr(y|ies|ied|ying)\b|backoff|exponential/i, why: "retry/backoff behavior" },
  { re: /\bO\([^)]+\)|complexity|linear time|constant time/i, why: "complexity claim" },
  { re: /\btimeout|timed? out|deadline\b/i, why: "timeout behavior" },
  { re: /\bidempotent|thread[- ]?safe|concurren|atomic\b/i, why: "concurrency/safety claim" },
  { re: /\bcaches?|memoiz|invalidat/i, why: "caching behavior" },
  { re: /\bvalidates?|sanitiz|escapes?\b/i, why: "validation behavior" },
];

export function semanticAdvisorHandler(): ResolverHandler {
  return {
    describe(): DescribeResult {
      return {
        name: "semantic-advisor",
        version: "1",
        kinds: ["text-quote", "ast-node"],
        tier: 3,
        advisory: true,
      };
    },
    resolve(params: ResolveParams): ResolveResult {
      const text = params.proposition?.text ?? "";
      const matches = BEHAVIORAL_PATTERNS.filter((p) => p.re.test(text));
      if (matches.length === 0) return { advisories: [] };
      const reasons = [...new Set(matches.map((m) => m.why))].join(", ");
      return {
        advisories: [
          {
            resolver: "semantic-advisor",
            message: `behavioral claim (${reasons}) — structural tiers cannot judge this; re-verify semantically`,
          },
        ],
      };
    },
  };
}
