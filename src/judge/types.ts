/** Judge: the model-backed interface behind hibi's three judgments. See design/SPEC.md section 8. */

export type RegionKind =
  | "sentence"
  | "heading"
  | "list-item"
  | "table-row"
  | "fenced";

export interface Region {
  id: string;
  kind: RegionKind;
  text: string;
  headingPath: string[];
}

export type TriageKind =
  | "factual_claim"
  | "instruction"
  | "opinion"
  | "narrative";
export type EvidenceKind = "code" | "other_doc" | "external" | "nothing";

export interface TriageResult {
  regionId: string;
  kind: TriageKind;
  kindProbabilities: Record<TriageKind, number>;
  kindConfidence: number;
  /**
   * Present only when `kind` is `factual_claim`. A region that is code by
   * construction (fenced block, path, or command) gets `code` with
   * probability 1 and was not asked the evidence question.
   */
  evidenceKind?: EvidenceKind;
  evidenceProbabilities?: Record<EvidenceKind, number>;
  /** Noul probability that the region could be shown true or false by evidence. */
  checkable: number;
}

export interface SupportsInput {
  sentence: string;
  context: string;
  evidence: string;
  source: string;
}

export type SupportsRelation = "supports" | "refutes" | "does_not_address";

export interface SupportsResult {
  relation: SupportsRelation;
  probabilities: Record<SupportsRelation, number>;
  confidence: number;
  /** Noul probability that the evidence addresses the sentence at all. */
  addresses: number;
  /**
   * Set when the `addresses` Noul forced `relation` to `does_not_address`.
   * `confidence` is then the Noul confidence of `addresses`; `probabilities`
   * stay the raw Choice probabilities.
   */
  forced?: "addresses";
}

export interface ConflictSide {
  text: string;
  doc: string;
  context: string;
}

export type ConflictRelation = "consistent" | "contradictory" | "unrelated";

export interface ConflictResult {
  relation: ConflictRelation;
  probabilities: Record<ConflictRelation, number>;
  confidence: number;
  sameSubject: number;
  contradicts: number;
  /**
   * Set when the `same_subject` Noul forced `relation` to `unrelated`.
   * `confidence` is then the Noul confidence of `same_subject`;
   * `probabilities` stay the raw Choice probabilities.
   */
  forced?: "same_subject";
}

/** Every judge call returns the exact request and raw answers so they can be cached and audited. */
export interface JudgeResponse<T> {
  model: string;
  request: unknown;
  answers: unknown;
  result: T;
}

export interface Judge {
  /** The pinned versioned model id this judge answers with. */
  readonly model: string;
  triage(
    doc: { path: string },
    regions: Region[],
  ): Promise<JudgeResponse<TriageResult[]>>;
  supports(input: SupportsInput): Promise<JudgeResponse<SupportsResult>>;
  conflict(
    a: ConflictSide,
    b: ConflictSide,
  ): Promise<JudgeResponse<ConflictResult>>;
}

/** Thrown when no judge can answer (no API key, network down). Callers map it to `pending`. */
export class JudgeUnavailable extends Error {
  constructor(message: string) {
    super(message);
    this.name = "JudgeUnavailable";
  }
}

/** Choice confidence per the TypeSafe docs: (p_max - 1/n) / (1 - 1/n). */
export function choiceConfidence(
  probabilities: Record<string, number>,
): number {
  const values = Object.values(probabilities);
  const n = values.length;
  if (n < 2) return 1;
  const pMax = Math.max(...values);
  return (pMax - 1 / n) / (1 - 1 / n);
}

/** Noul confidence per the TypeSafe docs: |2p - 1|. */
export function noulConfidence(p: number): number {
  return Math.abs(2 * p - 1);
}
