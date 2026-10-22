/**
 * `hibi evidence` (design/SPEC.md sections 11, 8.3 supports, 8.4, 6.4): an
 * agent submits evidence it collected for a claim; the judge answers
 * `supports(sentence, context, evidence, source)`; the verdict is cached and,
 * when it is `supports` at or above `judge.threshold`, an `evidence`
 * attestation is appended to the claim. Raw evidence text is never stored:
 * the cache keeps its sha256, the source, the agent's summary, and the
 * judge's probabilities.
 *
 * Input (`--from-file`, a JSON array or one object):
 *
 *   [{ "claimId": "clm_…", "evidence": "<text the agent observed>",
 *      "source": "<where it came from>", "summary": "<one line>",
 *      "by": "agent:<name>" }]
 *
 * JSON output (inside the CLI envelope `{ ok, action: "evidence", schemaVersion, …, next }`):
 *
 *   { "results": [{
 *       "claimId": "clm_…",
 *       "judge": "supported | refuted | review | pending",
 *       "relation": "supports | refutes | does_not_address" | null,   // null when pending
 *       "confidence": 0.93 | null,
 *       "probabilities": { "supports": 0.95, "refutes": 0.03, "does_not_address": 0.02 } | null,
 *       "reasons": ["evidence supports the sentence (confidence 0.93 >= threshold 0.8)"],
 *       "attested": true,        // an `evidence` attestation was appended
 *       "cached": false          // the verdict came from .claims/judge, the judge was not asked
 *     }],
 *     "summary": { "supported": 1, "refuted": 0, "review": 0, "pending": 0,
 *                  "requests": 1 } }  // requests: judge calls answered in this run
 *
 * The outcome comes from `supportsOutcome` in `reference.ts`, shared with
 * `check`, including the value rule (SPEC 8.5): when the sentence and the
 * evidence hold different values of the same kind and unit, the result is
 * `refuted` with `value differs: …`. When `source` equals one of the claim's
 * `sources[].id` or `sources[].resource`, the reasons add `source matches
 * declared source "<id or resource>"` (SPEC 4.3).
 *
 * The cache entry's `request` is the request as sent with `state.evidence`
 * replaced by `sha256:<hash>`; the key is computed over the request as sent.
 * Its `evidence` block keeps the hash, source, summary, actor, time, and the
 * values extracted from the evidence text, never the text itself.
 *
 * Validation runs over the whole batch before any judge call; one invalid
 * item rejects the batch and nothing is asked or written.
 */

import { createHash } from "node:crypto";
import {
  ACTOR_PATTERN,
  type Attestation,
  type Claim,
  type Config,
  type JudgeCacheEntry,
  type JudgeState,
} from "../core/model.ts";
import type { Answer, SystemOneRequest } from "../judge/http.ts";
import { buildSupportsRequest, mapSupportsAnswers } from "../judge/jev.ts";
import {
  type Judge,
  JudgeUnavailable,
  type SupportsRelation,
  type SupportsResult,
} from "../judge/types.ts";
import type { ClaimStore } from "../store/store.ts";
import {
  declaredSourceLabel,
  storedValues,
  supportsOutcome,
} from "./reference.ts";

/**
 * The longest evidence text accepted, in characters. About 8k tokens at 4
 * characters per token, which keeps one `supports` request well inside the
 * per-question budget of the Jev adapter (SPEC 8.2: requests stay under 64k
 * tokens; `QUESTION_TOKEN_BUDGET` in src/judge/jev.ts).
 */
export const MAX_EVIDENCE_CHARS = 32_000;

export interface EvidenceItem {
  claimId: string;
  /** The text the agent observed. Hashed, sent to the judge, never stored. */
  evidence: string;
  /** Where the evidence came from: a URL, a command, a file. */
  source: string;
  /** One line kept in the cache and on the attestation note. */
  summary: string;
  /** OKF actor: `human:<id>`, `agent:<name>`, or `process:<name>`. */
  by: string;
}

export interface EvidenceResult {
  claimId: string;
  judge: JudgeState;
  /** Null when the judge could not answer (`pending`). */
  relation: SupportsRelation | null;
  confidence: number | null;
  probabilities: Record<SupportsRelation, number> | null;
  /** One label per rule that decided the outcome. */
  reasons: string[];
  /** True when an `evidence` attestation was appended to the claim. */
  attested: boolean;
  /** True when the verdict came from the judge cache. */
  cached: boolean;
}

export interface EvidenceSummary {
  supported: number;
  refuted: number;
  review: number;
  pending: number;
  /** Judge calls answered in this run (cache misses that got an answer). */
  requests: number;
}

export interface EvidenceReport {
  results: EvidenceResult[];
  summary: EvidenceSummary;
}

export interface SubmitEvidenceInput {
  store: ClaimStore;
  judge: Judge;
  /** Raw items; validated as a batch before any judge call. */
  items: unknown[];
  /** The text around the claim's sentence in its document. */
  context: (claim: Claim) => string;
  config: Config;
  now?: Date;
}

/** One batch validation failure; `errors` names every bad item. */
export class EvidenceValidationError extends Error {
  constructor(readonly errors: string[]) {
    super(
      `evidence: ${errors.length} invalid item${errors.length === 1 ? "" : "s"}; nothing was submitted.\n${errors.map((e) => `  ${e}`).join("\n")}`,
    );
    this.name = "EvidenceValidationError";
  }
}

const ITEM_KEYS = ["claimId", "evidence", "source", "summary", "by"] as const;

/** sha256 hex of the evidence text as submitted. */
export function evidenceHash(evidence: string): string {
  return createHash("sha256").update(evidence).digest("hex");
}

/** Problems with one item, given the claims it may name. */
function itemErrors(
  item: unknown,
  claims: Map<string, Claim>,
): { claimId?: string; errors: string[] } {
  if (typeof item !== "object" || item === null || Array.isArray(item)) {
    return { errors: ["is not an object"] };
  }
  const raw = item as Record<string, unknown>;
  const errors: string[] = [];
  for (const key of Object.keys(raw)) {
    if (!(ITEM_KEYS as readonly string[]).includes(key)) {
      errors.push(`unknown key "${key}"; an item has ${ITEM_KEYS.join(", ")}`);
    }
  }
  for (const key of ITEM_KEYS) {
    if (typeof raw[key] !== "string") {
      errors.push(`"${key}" must be a string`);
    }
  }
  const str = (k: (typeof ITEM_KEYS)[number]) =>
    typeof raw[k] === "string" ? (raw[k] as string) : undefined;
  const claimId = str("claimId");
  if (claimId !== undefined) {
    const claim = claims.get(claimId);
    if (claim === undefined) {
      errors.push(`no claim ${claimId} in the store`);
    } else if (claim.enforcement === "retired") {
      errors.push(`claim ${claimId} is retired; evidence is not accepted`);
    }
  }
  const evidence = str("evidence");
  if (evidence !== undefined) {
    if (evidence.trim() === "") {
      errors.push(`"evidence" is empty`);
    } else if (evidence.length > MAX_EVIDENCE_CHARS) {
      errors.push(
        `"evidence" is ${evidence.length} characters; the limit is ${MAX_EVIDENCE_CHARS}. Trim it to the part that bears on the sentence, or summarize it.`,
      );
    }
  }
  for (const key of ["source", "summary"] as const) {
    const v = str(key);
    if (v !== undefined && v.trim() === "") errors.push(`"${key}" is empty`);
  }
  const by = str("by");
  if (by !== undefined && !ACTOR_PATTERN.test(by)) {
    errors.push(
      `"by" must be human:<id>, agent:<name>, or process:<name> (got ${JSON.stringify(by)})`,
    );
  }
  return { claimId, errors };
}

/** Validate a whole batch; throws `EvidenceValidationError` naming every bad item. */
export function validateEvidenceItems(
  items: unknown[],
  claims: Map<string, Claim>,
): EvidenceItem[] {
  if (items.length === 0) {
    throw new EvidenceValidationError(["the batch has no items"]);
  }
  const errors: string[] = [];
  items.forEach((item, i) => {
    const r = itemErrors(item, claims);
    const where = r.claimId ? `item ${i} (${r.claimId})` : `item ${i}`;
    for (const e of r.errors) errors.push(`${where}: ${e}`);
  });
  if (errors.length > 0) throw new EvidenceValidationError(errors);
  return items as EvidenceItem[];
}

/**
 * The request as stored in the cache: the exact request with `state.evidence`
 * replaced by `sha256:<hash>`, because raw evidence text is never stored
 * (SPEC 6.4). The cache key is computed over the request as sent.
 */
export function storedSupportsRequest(
  request: SystemOneRequest,
  hash: string,
): SystemOneRequest {
  return {
    ...request,
    state: {
      ...(request.state as Record<string, unknown>),
      evidence: `sha256:${hash}`,
    },
  };
}

interface Planned {
  item: EvidenceItem;
  claim: Claim;
  hash: string;
  request: SystemOneRequest;
  key: string;
  hit?: JudgeCacheEntry;
}

type Asked =
  | {
      kind: "answered";
      model: string;
      answers: unknown;
      result: SupportsResult;
    }
  | { kind: "cached"; entry: JudgeCacheEntry; result: SupportsResult }
  | { kind: "pending"; message: string };

export async function submitEvidence(
  input: SubmitEvidenceInput,
): Promise<EvidenceReport> {
  const { store, judge, config, context } = input;
  const now = (input.now ?? new Date()).toISOString();
  const decode = (answers: unknown) =>
    mapSupportsAnswers(answers as Record<string, Answer>);
  const cache = store.judgeCache();
  const model = config.judge.model;
  const threshold = config.judge.threshold;

  const claims = new Map((await store.allClaims()).map((c) => [c.id, c]));
  const items = validateEvidenceItems(input.items, claims);

  // Plan: build each request and look it up in the cache.
  const planned: Planned[] = [];
  for (const item of items) {
    const claim = claims.get(item.claimId) as Claim;
    const request = buildSupportsRequest(model, {
      sentence: claim.sentence,
      context: context(claim),
      evidence: item.evidence,
      source: item.source,
    });
    const key = cache.key(model, request);
    planned.push({
      item,
      claim,
      hash: evidenceHash(item.evidence),
      request,
      key,
      hit: await cache.get(key),
    });
  }

  // Ask: every miss goes to the judge. Any error other than
  // `JudgeUnavailable` aborts the batch here, before anything is written.
  const asked: Asked[] = await Promise.all(
    planned.map(async (p): Promise<Asked> => {
      if (p.hit) {
        return { kind: "cached", entry: p.hit, result: decode(p.hit.answers) };
      }
      try {
        const response = await judge.supports({
          sentence: p.claim.sentence,
          context: context(p.claim),
          evidence: p.item.evidence,
          source: p.item.source,
        });
        return {
          kind: "answered",
          model: response.model,
          answers: response.answers,
          result: decode(response.answers),
        };
      } catch (e) {
        if (e instanceof JudgeUnavailable) {
          return { kind: "pending", message: e.message };
        }
        throw e;
      }
    }),
  );

  // Write, in item order.
  const results: EvidenceResult[] = [];
  const summary: EvidenceSummary = {
    supported: 0,
    refuted: 0,
    review: 0,
    pending: 0,
    requests: 0,
  };
  for (const [i, p] of planned.entries()) {
    const a = asked[i] as Asked;
    if (a.kind === "pending") {
      summary.pending += 1;
      results.push({
        claimId: p.claim.id,
        judge: "pending",
        relation: null,
        confidence: null,
        probabilities: null,
        reasons: [`judge unavailable: ${a.message}`],
        attested: false,
        cached: false,
      });
      continue;
    }
    const evidenceValues = storedValues(p.item.evidence);
    const block = {
      hash: p.hash,
      source: p.item.source,
      summary: p.item.summary,
      by: p.item.by,
      at: now,
      values: evidenceValues,
    };
    if (a.kind === "cached") {
      await cache.put({ ...a.entry, evidence: block });
    } else {
      summary.requests += 1;
      await cache.put({
        key: p.key,
        model: a.model,
        judgment: "supports",
        subject: { claimId: p.claim.id },
        request: storedSupportsRequest(p.request, p.hash),
        answers: a.answers,
        evidence: block,
        at: now,
      });
    }

    const outcome = supportsOutcome(a.result, threshold, {
      sentence: storedValues(p.claim.sentence),
      evidence: evidenceValues,
    });
    const source = declaredSourceLabel(p.claim, p.item.source);
    if (source !== undefined) outcome.reasons.push(source);
    summary[outcome.state] += 1;
    let attested = false;
    if (outcome.state === "supported") {
      const attestation: Attestation = {
        by: p.item.by,
        at: now,
        basis: "evidence",
        note: p.item.summary,
        evidenceHash: p.hash,
      };
      // Re-read: an earlier item in this batch may have attested the same claim.
      const current = (await store.getClaim(p.claim.id)) as Claim;
      await store.putClaim({
        ...current,
        attestations: [...current.attestations, attestation],
      });
      attested = true;
    }
    results.push({
      claimId: p.claim.id,
      judge: outcome.state,
      relation: a.result.relation,
      confidence: a.result.confidence,
      probabilities: a.result.probabilities,
      reasons: outcome.reasons,
      attested,
      cached: a.kind === "cached",
    });
  }
  return { results, summary };
}
