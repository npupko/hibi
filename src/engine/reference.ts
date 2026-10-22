/**
 * The judge axis (SPEC 7.3, 8.4, 8.5).
 *
 * `judgeReference` asks `supports(sentence, referencedText)` for a `reference`
 * claim, once per `(model, request)`: the cache is checked first, and a miss
 * calls the judge and stores the verdict. `JudgeUnavailable` (no API key, a
 * network failure, or no judge supplied) is `pending`.
 *
 * `supportsOutcome` maps a `SupportsResult` to a judge state with the one
 * threshold, `config.judge.threshold`. It is the only such mapping; `check`
 * and `evidence` both use it. One label per rule:
 *
 * | rule | state | label |
 * |---|---|---|
 * | relation is not `does_not_address` and the sentence and the evidence hold different values of the same kind and unit (money, unit-bearing numbers, versions, dates; not the unitless `number` group) | `refuted` | `value differs: numbers differ: 29 vs 39` |
 * | `supports` at or above threshold | `supported` | `evidence supports the sentence (confidence 0.94 >= threshold 0.8)` |
 * | `refutes` at or above threshold | `refuted` | `evidence refutes the sentence (confidence 0.92 >= threshold 0.8)` |
 * | `supports` or `refutes` below threshold | `review` | `evidence leans supports, confidence 0.55 below threshold 0.8` |
 * | `does_not_address` | `review` | `evidence did not address the sentence` (with `addresses=…` when the Noul floor forced it) |
 *
 * `declaredSourceLabel` adds `source matches declared source "<id or
 * resource>"` when an evidence source equals one of the claim's `sources[]`
 * (SPEC 4.3). It changes no state.
 */

import type { ClaimSides } from "../algo/resolve.ts";
import type {
  Claim,
  EvidenceState,
  ExtractedValue,
  JudgeCacheEntry,
  JudgeState,
} from "../core/model.ts";
import { cacheKey, type JudgeCache } from "../judge/cache.ts";
import type { Answer, SystemOneRequest } from "../judge/http.ts";
import { buildSupportsRequest, mapSupportsAnswers } from "../judge/jev.ts";
import {
  type Judge,
  JudgeError,
  JudgeUnavailable,
  MAX_EVIDENCE_CHARS,
  type SupportsResult,
} from "../judge/types.ts";
import { claimContext } from "./context.ts";
import { compareValues, extractValues } from "./values.ts";

export interface JudgeOutcome {
  state: JudgeState;
  reasons: string[];
  /** The cache key of the verdict, when there is one (an `ack` names it). */
  key?: string;
}

/**
 * Value groups the value rule ignores on `supports`: a bare number in the
 * evidence (a count, an id, a line number) says too little to refute.
 */
export const SUPPORTS_EXCLUDED_GROUPS: readonly string[] = ["number"];

function fmt(p: number): string {
  return p.toFixed(2);
}

/** The values the numeric rule compares, as stored with cached evidence. */
export function storedValues(text: string): ExtractedValue[] {
  return extractValues(text).map((v) => ({
    kind: v.kind,
    normalized: v.normalized,
    ...(v.unit !== undefined ? { unit: v.unit } : {}),
  }));
}

/**
 * The judge state and reason labels for one `supports` result (SPEC 8.4),
 * with the value rule (SPEC 8.5) when the sentence and evidence values are
 * given.
 */
export function supportsOutcome(
  result: SupportsResult,
  threshold: number,
  values?: { sentence: ExtractedValue[]; evidence: ExtractedValue[] },
): JudgeOutcome {
  const c = fmt(result.confidence);
  const t = `${threshold}`;
  if (result.relation === "does_not_address") {
    return {
      state: "review",
      reasons: [
        result.forced === "addresses"
          ? `evidence did not address the sentence (addresses=${fmt(result.addresses)})`
          : "evidence did not address the sentence",
      ],
    };
  }
  if (values !== undefined) {
    // Only money, unit-bearing numbers, versions, and dates can refute.
    const cmp = compareValues(
      values.sentence.map((v) => ({ ...v, raw: v.normalized })),
      values.evidence.map((v) => ({ ...v, raw: v.normalized })),
      { exclude: SUPPORTS_EXCLUDED_GROUPS },
    );
    if (cmp.differs && cmp.label !== undefined) {
      return { state: "refuted", reasons: [`value differs: ${cmp.label}`] };
    }
  }
  const verb = result.relation;
  if (result.confidence >= threshold) {
    return {
      state: verb === "supports" ? "supported" : "refuted",
      reasons: [
        `evidence ${verb} the sentence (confidence ${c} >= threshold ${t})`,
      ],
    };
  }
  return {
    state: "review",
    reasons: [`evidence leans ${verb}, confidence ${c} below threshold ${t}`],
  };
}

/** `source matches declared source "<id or resource>"` when `source` names one of the claim's sources. */
export function declaredSourceLabel(
  claim: Pick<Claim, "sources">,
  source: string,
): string | undefined {
  for (const s of claim.sources) {
    if (s.id !== undefined && s.id === source) {
      return `source matches declared source "${s.id}"`;
    }
    if (s.resource === source) {
      return `source matches declared source "${s.resource}"`;
    }
  }
  return undefined;
}

/** When a cache entry was last written for its claim: the later of its `at` and its evidence time. */
function touchedAt(e: JudgeCacheEntry): string {
  return e.evidence !== undefined && e.evidence.at > e.at
    ? e.evidence.at
    : e.at;
}

/** The newest `supports` entry among `entries`, by `touchedAt`; the later one wins a tie. */
export function newestSupports(
  entries: JudgeCacheEntry[],
): JudgeCacheEntry | undefined {
  let best: JudgeCacheEntry | undefined;
  for (const e of entries) {
    if (e.judgment !== "supports") continue;
    if (best === undefined || touchedAt(e) >= touchedAt(best)) best = e;
  }
  return best;
}

/**
 * The outcome stored in a cached `supports` entry for `claim`. The value rule
 * uses the values stored with the evidence, and the source label its source.
 */
export function cachedSupportsOutcome(
  entry: JudgeCacheEntry,
  threshold: number,
  claim: Pick<Claim, "sentence" | "sources">,
): JudgeOutcome {
  const outcome = supportsOutcome(
    mapSupportsAnswers(entry.answers as Record<string, Answer>),
    threshold,
    entry.evidence !== undefined
      ? {
          sentence: storedValues(claim.sentence),
          evidence: entry.evidence.values ?? [],
        }
      : undefined,
  );
  const source =
    entry.evidence !== undefined
      ? declaredSourceLabel(claim, entry.evidence.source)
      : undefined;
  return {
    ...outcome,
    reasons: source ? [...outcome.reasons, source] : outcome.reasons,
    key: entry.key,
  };
}

/** The anchor states in which the judge is consulted for a `reference` claim. */
const JUDGEABLE: ReadonlySet<EvidenceState> = new Set<EvidenceState>([
  "unchanged",
  "moved",
]);

/** The texts a `reference` claim is judged on, as they read now. */
export interface ReferenceTexts {
  /** The current text of the claim's document. */
  docText: string;
  /** The current sentence: the located doc span. */
  sentence: string;
  /** The current text of the referenced span. */
  referencedText: string;
}

/**
 * Whether a `reference` claim can be judged from its resolved sides (SPEC
 * 7.3): both sides `unchanged` or `moved` and both texts located. Otherwise
 * the reason label says why the judge is not consulted.
 */
export function referenceTexts(
  claim: Pick<Claim, "sentence">,
  sides: ClaimSides,
  docText: string | null,
): { texts: ReferenceTexts } | { reason: string } {
  const ev = sides.evidence[0];
  if (!JUDGEABLE.has(sides.doc.state)) {
    return {
      reason: `not consulted while the doc side is ${sides.doc.state}`,
    };
  }
  if (ev === undefined || !JUDGEABLE.has(ev.state)) {
    return {
      reason: `not consulted while the evidence side is ${ev?.state ?? "missing"}`,
    };
  }
  if (docText === null || ev.liveText === null) {
    return { reason: "not consulted; the referenced span has no located text" };
  }
  return {
    texts: {
      docText,
      sentence: sides.doc.liveText ?? claim.sentence,
      referencedText: ev.liveText,
    },
  };
}

/** The label for a referenced span over `MAX_EVIDENCE_CHARS`. */
export const REFERENCE_TOO_LONG = `referenced span is longer than ${MAX_EVIDENCE_CHARS.toLocaleString("en-US")} characters`;

/**
 * The `supports` request and cache key for a `reference` claim as its texts
 * read now. `check` judges with it and `attest --ack review` names its key,
 * so both always agree on which verdict is current.
 */
export function referenceSupportsRequest(
  model: string,
  claim: Pick<Claim, "id" | "sentence" | "doc" | "evidence">,
  texts: ReferenceTexts,
): { request: SystemOneRequest; key: string } {
  const request = buildSupportsRequest(model, {
    sentence: texts.sentence,
    context: claimContext(texts.docText, {
      ...claim,
      sentence: texts.sentence,
    }),
    evidence: texts.referencedText,
    source: claim.evidence[0]?.file ?? "",
  });
  return { request, key: cacheKey(model, request) };
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
  /** The model in the cache key when no judge is given; otherwise `judge.model` is used. */
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
  const texts: ReferenceTexts = {
    docText: input.docText,
    sentence: input.sentence ?? input.claim.sentence,
    referencedText: input.referencedText,
  };
  if (texts.referencedText.length > MAX_EVIDENCE_CHARS) {
    return { state: "pending", reasons: [REFERENCE_TOO_LONG] };
  }
  const { request, key } = referenceSupportsRequest(
    input.judge?.model ?? input.model,
    input.claim,
    texts,
  );
  const hit = await input.cache.get(key);
  const values = {
    sentence: storedValues(texts.sentence),
    evidence: storedValues(texts.referencedText),
  };
  if (hit !== undefined) {
    const outcome = supportsOutcome(
      mapSupportsAnswers(hit.answers as Record<string, Answer>),
      input.threshold,
      values,
    );
    return { ...outcome, key };
  }

  if (input.judge === undefined) {
    return {
      state: "pending",
      reasons: ["pending: no judge is configured (set TYPESAFE_API_KEY)"],
    };
  }
  let response: Awaited<ReturnType<Judge["supports"]>>;
  try {
    response = await input.judge.supports({
      sentence: texts.sentence,
      context: (request.state as { claim_context: string }).claim_context,
      evidence: texts.referencedText,
      source: (request.state as { evidence_source: string }).evidence_source,
    });
  } catch (e) {
    if (e instanceof JudgeUnavailable) {
      return { state: "pending", reasons: [`pending: ${e.message}`] };
    }
    if (e instanceof JudgeError) {
      return { state: "pending", reasons: [`judge error: ${e.message}`] };
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
  return {
    ...supportsOutcome(
      mapSupportsAnswers(response.answers as Record<string, Answer>),
      input.threshold,
      values,
    ),
    key,
  };
}
