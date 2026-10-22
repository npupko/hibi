/**
 * The judge axis (SPEC 7.3, 8.4).
 *
 * `judgeReference` asks `supports(sentence, referencedText)` for a `reference`
 * claim, once per `(model, request)`: the cache is checked first, and a miss
 * calls the judge and stores the verdict. `JudgeUnavailable` (no API key, a
 * network failure, or no judge supplied) is `pending`.
 *
 * `supportsOutcome` maps a `SupportsResult` to a judge state with the one
 * threshold, `config.judge.threshold`:
 *
 *   - `supports` at or above threshold: `supported`
 *   - `refutes` at or above threshold: `refuted` (gates)
 *   - anything else (`refutes` or `supports` below threshold,
 *     `does_not_address`): `review` (warns)
 */

import type { Claim, JudgeCacheEntry, JudgeState } from "../core/model.ts";
import { cacheKey, type JudgeCache } from "../judge/cache.ts";
import { buildSupportsRequest, mapSupportsAnswers } from "../judge/jev.ts";
import {
  type Judge,
  JudgeUnavailable,
  type SupportsResult,
} from "../judge/types.ts";
import { claimContext } from "./context.ts";

export interface JudgeOutcome {
  state: JudgeState;
  reasons: string[];
  /** The cache key of the verdict, when there is one (an `ack` names it). */
  key?: string;
}

function fmt(p: number): string {
  return p.toFixed(2);
}

/** The judge state and reason labels for one `supports` result (SPEC 8.4). */
export function supportsOutcome(
  result: SupportsResult,
  threshold: number,
): JudgeOutcome {
  const c = fmt(result.confidence);
  if (result.relation === "does_not_address") {
    return {
      state: "review",
      reasons: [
        result.forced
          ? `evidence did not address the sentence (addresses=${fmt(result.addresses)})`
          : `judge: evidence does not address the sentence (confidence ${c})`,
      ],
    };
  }
  const verb = result.relation === "supports" ? "supports" : "refutes";
  if (result.confidence >= threshold) {
    return {
      state: result.relation === "supports" ? "supported" : "refuted",
      reasons: [
        `judge: evidence ${verb} the sentence (confidence ${c} at or above threshold ${threshold})`,
      ],
    };
  }
  return {
    state: "review",
    reasons: [
      `judge: evidence leans ${verb} below threshold (confidence ${c} < ${threshold})`,
    ],
  };
}

/** The outcome stored in a cached `supports` entry. */
export function cachedSupportsOutcome(
  entry: JudgeCacheEntry,
  threshold: number,
): JudgeOutcome {
  const outcome = supportsOutcome(
    mapSupportsAnswers(
      entry.answers as Parameters<typeof mapSupportsAnswers>[0],
    ),
    threshold,
  );
  return { ...outcome, key: entry.key };
}

export interface JudgeReferenceInput {
  claim: Pick<Claim, "id" | "sentence" | "doc" | "evidence">;
  /** The current text of the claim's document. */
  docText: string;
  /** The current text of the referenced span. */
  referencedText: string;
  /** The current sentence; defaults to `claim.sentence`. */
  sentence?: string;
  /** Absent means no judge can answer: a cache miss is `pending`. */
  judge?: Judge;
  cache: JudgeCache;
  /** `config.judge.model`. */
  model: string;
  /** `config.judge.threshold`. */
  threshold: number;
  /** Time stamped on a new cache entry. Default: now. */
  now?: Date;
}

/** Judge one `reference` claim whose doc and evidence sides both resolved. */
export async function judgeReference(
  input: JudgeReferenceInput,
): Promise<JudgeOutcome> {
  const sentence = input.sentence ?? input.claim.sentence;
  const request = buildSupportsRequest(input.model, {
    sentence,
    context: claimContext(input.docText, { ...input.claim, sentence }),
    evidence: input.referencedText,
    source: input.claim.evidence[0]?.file ?? "",
  });
  const key = cacheKey(input.model, request);
  const hit = await input.cache.get(key);
  if (hit !== undefined) return cachedSupportsOutcome(hit, input.threshold);

  if (input.judge === undefined) {
    return {
      state: "pending",
      reasons: [
        "judge: pending: no judge is configured (set TYPESAFE_API_KEY)",
      ],
    };
  }
  let response: Awaited<ReturnType<Judge["supports"]>>;
  try {
    response = await input.judge.supports({
      sentence,
      context: (request.state as { claim_context: string }).claim_context,
      evidence: input.referencedText,
      source: (request.state as { evidence_source: string }).evidence_source,
    });
  } catch (e) {
    if (e instanceof JudgeUnavailable) {
      return { state: "pending", reasons: [`judge: pending: ${e.message}`] };
    }
    throw e;
  }
  await input.cache.put({
    key,
    model: response.model,
    judgment: "supports",
    subject: { claimId: input.claim.id },
    request,
    answers: response.answers,
    at: (input.now ?? new Date()).toISOString(),
  });
  return { ...supportsOutcome(response.result, input.threshold), key };
}
