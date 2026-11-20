/**
 * FakeJudge: the judge used in tests. It answers from a script, keyed by
 * sentence text or computed by a caller-supplied function, and throws
 * `JudgeUnavailable` when told to. It never touches the network.
 *
 * Answers are encoded in the Jev wire shape (the System One `answers`
 * object), and the returned `result` is decoded from them by the same
 * mappers the Jev adapter uses. A cache entry written by this judge is
 * therefore read back exactly as one written by Jev: there is one decode
 * path for judge answers.
 */

import { DEFAULT_JUDGE_MODEL } from "../core/model.ts";
import type { Answer } from "./http.ts";
import {
  buildConflictRequest,
  buildSupportsRequest,
  buildTriageRequests,
  isCodeByConstruction,
  mapConflictAnswers,
  mapSupportsAnswers,
  mapTriageAnswers,
} from "./jev.ts";
import supportsFile from "./questions/supports.json";
import {
  type ConflictRelation,
  type ConflictResult,
  type ConflictSide,
  choiceConfidence,
  type Judge,
  JudgeError,
  type JudgeResponse,
  JudgeUnavailable,
  type Region,
  type SupportsInput,
  type SupportsRelation,
  type SupportsResult,
  type TriageResult,
} from "./types.ts";

/** A scripted answer: a lookup by text, or a function of the input. */
export type Scripted<In extends unknown[], Out> =
  | Record<string, Out>
  | ((...input: In) => Out | undefined);

export interface FakeScript {
  /** Keyed by region text; `regionId` is filled in from the region. */
  triage?: Scripted<[Region], Omit<TriageResult, "regionId">>;
  /** Keyed by `input.sentence`. */
  supports?: Scripted<[SupportsInput], SupportsResult>;
  /** Keyed by `pairKey(a.text, b.text)`. */
  conflict?: Scripted<[ConflictSide, ConflictSide], ConflictResult>;
}

export interface FakeCall {
  judgment: "triage" | "supports" | "conflict";
  request: unknown;
}

/** The lookup key for a scripted conflict answer: both texts, sorted, newline-joined. */
export function pairKey(a: string, b: string): string {
  return [a, b].sort().join("\n");
}

function lookup<In extends unknown[], Out>(
  script: Scripted<In, Out> | undefined,
  key: string,
  input: In,
): Out | undefined {
  if (script === undefined) return undefined;
  if (typeof script === "function") return script(...input);
  return Object.hasOwn(script, key) ? script[key] : undefined;
}

export class FakeJudge implements Judge {
  readonly model: string;
  /** Every request this judge received, in order. */
  readonly calls: FakeCall[] = [];
  private unavailable: string | null = null;
  private error: string | null = null;

  /** `model` defaults to the model `init` writes, so cache keys match the config as in production. */
  constructor(
    private readonly script: FakeScript = {},
    model = DEFAULT_JUDGE_MODEL,
  ) {
    this.model = model;
  }

  /** Make every following call throw `JudgeUnavailable` with `message`; `null` restores answers. */
  setUnavailable(message: string | null = "fake judge unavailable"): void {
    this.unavailable = message;
  }

  /** Make every following call throw `JudgeError` with `message`; `null` restores answers. */
  setError(message: string | null = "fake judge error"): void {
    this.error = message;
  }

  private begin(judgment: FakeCall["judgment"], request: unknown): void {
    if (this.unavailable !== null) {
      throw new JudgeUnavailable(this.unavailable);
    }
    if (this.error !== null) {
      throw new JudgeError(this.error);
    }
    this.calls.push({ judgment, request });
  }

  async triage(
    doc: { path: string },
    regions: Region[],
  ): Promise<JudgeResponse<TriageResult[]>> {
    const requests = buildTriageRequests(this.model, doc, regions);
    this.begin("triage", { doc, regions });
    const answers = requests.map((request) => {
      const chunk = (request.state as { regions: Region[] }).regions;
      const out: Record<string, Answer> = {};
      chunk.forEach((region, i) => {
        const answer = lookup(this.script.triage, region.text, [region]);
        if (answer === undefined) {
          throw new Error(
            `FakeJudge has no scripted triage answer for region ${JSON.stringify(region.text)}`,
          );
        }
        Object.assign(
          out,
          encodeTriage(i, answer, isCodeByConstruction(region)),
        );
      });
      return out;
    });
    return {
      model: this.model,
      request: requests,
      answers,
      result: mapTriageAnswers(requests, answers),
    };
  }

  async supports(input: SupportsInput): Promise<JudgeResponse<SupportsResult>> {
    const request = buildSupportsRequest(this.model, input);
    this.begin("supports", input);
    const answer = lookup(this.script.supports, input.sentence, [input]);
    if (answer === undefined) {
      throw new Error(
        `FakeJudge has no scripted supports answer for ${JSON.stringify(input.sentence)}`,
      );
    }
    const answers = encodeSupports(answer);
    return {
      model: this.model,
      request,
      answers,
      result: mapSupportsAnswers(answers),
    };
  }

  async conflict(
    a: ConflictSide,
    b: ConflictSide,
  ): Promise<JudgeResponse<ConflictResult>> {
    const request = buildConflictRequest(this.model, a, b);
    this.begin("conflict", { a, b });
    const answer = lookup(this.script.conflict, pairKey(a.text, b.text), [
      a,
      b,
    ]);
    if (answer === undefined) {
      throw new Error(
        `FakeJudge has no scripted conflict answer for ${JSON.stringify(a.text)} and ${JSON.stringify(b.text)}`,
      );
    }
    const answers = encodeConflict(answer);
    return {
      model: this.model,
      request,
      answers,
      result: mapConflictAnswers(answers),
    };
  }
}

// ── Wire encoding: a result as the System One answers that decode to it ──

function choiceAnswer(
  choice: string,
  probabilities: Record<string, number>,
): Answer {
  return {
    type: "choice",
    choice,
    confidence: choiceConfidence(probabilities),
    probabilities,
  };
}

function noulAnswer(p: number): Answer {
  return { type: "noul", noul: p };
}

/** The Jev option name for each supports relation (inverse of `relationNames`). */
const SUPPORTS_OPTION = Object.fromEntries(
  Object.entries(supportsFile.relationNames).map(([option, relation]) => [
    relation,
    option,
  ]),
) as Record<SupportsRelation, string>;

/**
 * The System One answers for a supports result. A result forced to
 * `does_not_address` by the `addresses` floor keeps the Choice winner of its
 * probabilities as the raw choice.
 */
export function encodeSupports(r: SupportsResult): Record<string, Answer> {
  const choice = r.forced ? winner(r.probabilities) : r.relation;
  const probabilities = Object.fromEntries(
    Object.entries(r.probabilities).map(([relation, p]) => [
      SUPPORTS_OPTION[relation as SupportsRelation],
      p,
    ]),
  );
  return {
    relation: choiceAnswer(SUPPORTS_OPTION[choice], probabilities),
    addresses: noulAnswer(r.addresses),
  };
}

/** The System One answers for a conflict result. */
export function encodeConflict(r: ConflictResult): Record<string, Answer> {
  const choice = r.forced ? winner(r.probabilities) : r.relation;
  return {
    same_subject: noulAnswer(r.sameSubject),
    relation: choiceAnswer(choice, r.probabilities),
    contradicts: noulAnswer(r.contradicts),
  };
}

/** The System One answers for one triage region at index `i` of its request. */
export function encodeTriage(
  i: number,
  r: Omit<TriageResult, "regionId">,
  codeByConstruction: boolean,
): Record<string, Answer> {
  const out: Record<string, Answer> = {
    [`kind_${i}`]: choiceAnswer(r.kind, r.kindProbabilities),
  };
  if (!codeByConstruction) {
    const probabilities = r.evidenceProbabilities ?? {
      code: 0.25,
      other_doc: 0.25,
      external: 0.25,
      nothing: 0.25,
    };
    out[`evidenceKind_${i}`] = choiceAnswer(
      r.evidenceKind ?? winner(probabilities),
      probabilities,
    );
  }
  return out;
}

function winner<K extends string>(probabilities: Record<K, number>): K {
  const entries = Object.entries(probabilities) as [K, number][];
  let best = entries[0] as [K, number];
  for (const e of entries) if (e[1] > best[1]) best = e;
  return best[0];
}

/** A `SupportsResult` from Choice probabilities; `relation` and `confidence` are derived. */
export function supportsAnswer(
  probabilities: Record<SupportsRelation, number>,
  addresses = 1,
): SupportsResult {
  return {
    relation: winner(probabilities),
    probabilities,
    confidence: choiceConfidence(probabilities),
    addresses,
  };
}

/** A `ConflictResult` from Choice probabilities; `relation` and `confidence` are derived. */
export function conflictAnswer(
  probabilities: Record<ConflictRelation, number>,
  sameSubject = 1,
  contradicts = probabilities.contradictory,
): ConflictResult {
  return {
    relation: winner(probabilities),
    probabilities,
    confidence: choiceConfidence(probabilities),
    sameSubject,
    contradicts,
  };
}
