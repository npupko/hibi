/**
 * Tier-3 semantic advisor (§6, §7.4) — the quarantined, opt-in advisory resolver.
 *
 * The deterministic tiers (1–2) decide the structural AnchorState of each side
 * (doc/code); they do NOT judge whether a natural-language *behavioral* claim is
 * still true ("sorts ascending", "retries on timeout", an "O(n)" complexity
 * claim). That judgement is the BehaviorState axis, and it is not something a
 * structural resolver can settle. This advisor surfaces such claims for
 * re-verification. It **advises and never gates**: it declares `verifierKinds:
 * []`, returns advisories only — never a verdict and never a BehaviorState — so
 * the deterministic AnchorState verdict always stands.
 *
 * This implementation is itself deterministic (keyword surfacing via
 * `classifyClaimKind`). An LLM-backed "is it still true?" advisor would plug in
 * identically behind this same contract, out-of-process — but the core has no
 * model in the loop (§11.1).
 */
import { classifyClaimKind } from "../../algo/behavioral.ts";
import type {
  DescribeResult,
  ResolveParams,
  ResolveResult,
} from "../protocol.ts";
import type { ResolverHandler } from "../server.ts";

export function semanticAdvisorHandler(): ResolverHandler {
  return {
    describe(): DescribeResult {
      return {
        name: "semantic-advisor",
        version: "1",
        kinds: ["text-quote", "ast-node"],
        tier: 3,
        advisory: true,
        // Settles no BehaviorState — it only surfaces claims, never verifies.
        verifierKinds: [],
      };
    },
    resolve(params: ResolveParams): ResolveResult {
      const text = params.proposition?.textCache ?? "";
      const kind = classifyClaimKind(text);
      if (!kind) return { advisories: [] };
      return {
        advisories: [
          {
            resolver: "semantic-advisor",
            message: `behavioral claim (${kind}) — the structural AnchorState tiers cannot judge this; its BehaviorState needs semantic re-verification`,
          },
        ],
      };
    },
  };
}
