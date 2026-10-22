/**
 * FakeJudge: the judge used in tests. It answers from a script, keyed by
 * sentence text or computed by a caller-supplied function, and throws
 * `JudgeUnavailable` when told to. It never touches the network.
 */

import {
  type ConflictRelation,
  type ConflictResult,
  type ConflictSide,
  choiceConfidence,
  type Judge,
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

  constructor(
    private readonly script: FakeScript = {},
    model = "fake-1.0.0",
  ) {
    this.model = model;
  }

  /** Make every following call throw `JudgeUnavailable` with `message`; `null` restores answers. */
  setUnavailable(message: string | null = "fake judge unavailable"): void {
    this.unavailable = message;
  }

  private begin(judgment: FakeCall["judgment"], request: unknown): void {
    if (this.unavailable !== null) {
      throw new JudgeUnavailable(this.unavailable);
    }
    this.calls.push({ judgment, request });
  }

  private respond<T>(request: unknown, result: T): JudgeResponse<T> {
    return { model: this.model, request, answers: result, result };
  }

  async triage(
    doc: { path: string },
    regions: Region[],
  ): Promise<JudgeResponse<TriageResult[]>> {
    const request = { doc, regions };
    this.begin("triage", request);
    const result = regions.map((region) => {
      const answer = lookup(this.script.triage, region.text, [region]);
      if (answer === undefined) {
        throw new Error(
          `FakeJudge has no scripted triage answer for region ${JSON.stringify(region.text)}`,
        );
      }
      return { ...answer, regionId: region.id };
    });
    return this.respond(request, result);
  }

  async supports(input: SupportsInput): Promise<JudgeResponse<SupportsResult>> {
    this.begin("supports", input);
    const answer = lookup(this.script.supports, input.sentence, [input]);
    if (answer === undefined) {
      throw new Error(
        `FakeJudge has no scripted supports answer for ${JSON.stringify(input.sentence)}`,
      );
    }
    return this.respond(input, answer);
  }

  async conflict(
    a: ConflictSide,
    b: ConflictSide,
  ): Promise<JudgeResponse<ConflictResult>> {
    const request = { a, b };
    this.begin("conflict", request);
    const answer = lookup(this.script.conflict, pairKey(a.text, b.text), [
      a,
      b,
    ]);
    if (answer === undefined) {
      throw new Error(
        `FakeJudge has no scripted conflict answer for ${JSON.stringify(a.text)} and ${JSON.stringify(b.text)}`,
      );
    }
    return this.respond(request, answer);
  }
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
