/**
 * Jev adapter: implements the Judge interface on TypeSafe's System One API.
 * See design/SPEC.md section 8. The questions are data in ./questions/*.json;
 * this file builds requests from them and maps answers to results.
 */
import type {
  Answer,
  ChoiceAnswer,
  SystemOneClient,
  SystemOneRequest,
  SystemOneResponse,
} from "./http.ts";
import conflictFile from "./questions/conflict.json";
import supportsFile from "./questions/supports.json";
import triageFile from "./questions/triage.json";
import {
  type ConflictRelation,
  type ConflictResult,
  type ConflictSide,
  choiceConfidence,
  type EvidenceKind,
  type Judge,
  JudgeError,
  type JudgeResponse,
  noulConfidence,
  type Region,
  type SupportsInput,
  type SupportsRelation,
  type SupportsResult,
  type TriageKind,
  type TriageResult,
} from "./types.ts";

// ---------------------------------------------------------------------------
// Code rules and limits. Every automatic rule the adapter applies is here.
// ---------------------------------------------------------------------------

/**
 * supports: when the `addresses` Noul is below this, the relation is
 * `does_not_address` whatever the Choice said. The companion-Noul gate from
 * the research note, section 2b (design/evidence/typesafe-research-2026-10-22.md).
 * The result then carries `forced: "addresses"` and the Noul's confidence.
 */
export const ADDRESSES_FLOOR = 0.3;

/**
 * conflict: when the `same_subject` Noul is below this, the relation is
 * `unrelated` whatever the Choice said. Research note, section 2c.
 * The result then carries `forced: "same_subject"` and the Noul's confidence.
 */
export const SAME_SUBJECT_FLOOR = 0.3;

/** Token estimate: 4 characters per token, conservative for English prose. */
export const CHARS_PER_TOKEN = 4;

/** Per-request budget for state plus questions; the model limit is 64k (research note, section 5). */
export const REQUEST_TOKEN_BUDGET = 48_000;

/** Budget for state plus any one question; the model limit is 32k (research note, section 5). */
export const QUESTION_TOKEN_BUDGET = 24_000;

// ---------------------------------------------------------------------------

const TRIAGE_KINDS: TriageKind[] = [
  "factual_claim",
  "instruction",
  "opinion",
  "narrative",
];
const EVIDENCE_KINDS: EvidenceKind[] = [
  "code",
  "other_doc",
  "external",
  "nothing",
];
const CONFLICT_RELATIONS: ConflictRelation[] = [
  "consistent",
  "contradictory",
  "unrelated",
];
const SUPPORTS_RELATIONS: SupportsRelation[] = [
  "supports",
  "refutes",
  "does_not_address",
];

const CODE_EVIDENCE: Record<EvidenceKind, number> = {
  code: 1,
  other_doc: 0,
  external: 0,
  nothing: 0,
};

export function estimateTokens(value: unknown): number {
  return Math.ceil(JSON.stringify(value).length / CHARS_PER_TOKEN);
}

/** Estimated token cost of one region inside a triage request. */
export interface RegionCost {
  /** The region's entry in `state.regions`. */
  state: number;
  /** All of the region's questions together. */
  questions: number;
  /** The region's largest single question. */
  largestQuestion: number;
}

/**
 * Packs consecutive regions into chunks. Each chunk becomes one request whose
 * state holds `doc` plus only that chunk's regions. A chunk is closed before
 * either budget is exceeded: the state plus all its questions within
 * `request`, and the state plus any one question within `question`.
 * `fixed` covers what every request carries (`model`, `doc`, JSON framing).
 * Returns region indices per chunk.
 */
export function chunkRegions(
  costs: RegionCost[],
  fixed: number,
  budgets = { request: REQUEST_TOKEN_BUDGET, question: QUESTION_TOKEN_BUDGET },
): number[][] {
  const fits = (state: number, questions: number, largest: number) =>
    fixed + state + questions <= budgets.request &&
    fixed + state + largest <= budgets.question;
  const chunks: number[][] = [];
  let current: number[] = [];
  let state = 0;
  let questions = 0;
  let largest = 0;
  costs.forEach((cost, i) => {
    if (!fits(cost.state, cost.questions, cost.largestQuestion)) {
      throw new Error(
        `Triage region ${i} is too large for one request (${cost.state} state tokens, ${cost.questions} question tokens).`,
      );
    }
    const next = Math.max(largest, cost.largestQuestion);
    if (!fits(state + cost.state, questions + cost.questions, next)) {
      chunks.push(current);
      current = [];
      state = 0;
      questions = 0;
      largest = 0;
    }
    current.push(i);
    state += cost.state;
    questions += cost.questions;
    largest = Math.max(largest, cost.largestQuestion);
  });
  if (current.length > 0) chunks.push(current);
  return chunks;
}

// A region is `code` by construction when its text contains a fenced block,
// an inline code span holding a path or a command, or a file path token in
// prose. URLs are not paths. `Name.js` (Node.js, Next.js) is a product name,
// not a file.
const FENCE = /```|~~~/;
const INLINE_CODE = /`([^`\n]+)`/g;
const URL = /\b[a-z][a-z0-9+.-]*:\/\/\S+/gi;
const FILE_EXTENSIONS =
  "ts|tsx|js|jsx|mjs|cjs|json|jsonc|ya?ml|toml|ini|cfg|conf|env|md|mdx|txt|py|rb|rs|go|java|kt|swift|c|h|cc|cpp|hpp|cs|php|sh|bash|zsh|sql|lock|html|css|scss|xml|gradle";
const PATH_TOKEN = new RegExp(
  [
    String.raw`^(?:\.{1,2}|~)?/[\w.-]`, // ./x, ../x, ~/x, /x
    String.raw`^[\w.-]+/(?:[\w.-]+/)*$`, // dir/ or dir/sub/
    String.raw`^[\w.-]*/(?:[\w.-]+/)*[\w-]*\.(?:${FILE_EXTENSIONS})$`, // dir/file.ext
    String.raw`^[\w-]+\.(?:${FILE_EXTENSIONS})$`, // file.ext
    String.raw`^\.[a-z][\w-]*$`, // .dotfile
  ].join("|"),
);
const PRODUCT_NAME = /^[A-Z][a-z]*\.js$/;
const TOKEN_TRIM = /^[("'[{<]+|[)"'\]}>.,;:!?]+$/g;

function isPathToken(raw: string): boolean {
  const token = raw.replace(TOKEN_TRIM, "");
  return PATH_TOKEN.test(token) && !PRODUCT_NAME.test(token);
}

/** Inside a code span, any slash-separated name is a path; so is any path token. */
function isCodePath(span: string): boolean {
  const text = span.replace(URL, " ");
  return /[\w.-]\/[\w.-]/.test(text) || text.split(/\s+/).some(isPathToken);
}

/** A code span that starts with a flag, or a word followed by arguments. */
function isCommand(span: string): boolean {
  const trimmed = span.trim();
  return /^-{1,2}\w/.test(trimmed) || /^[\w.-]+\s+\S/.test(trimmed);
}

export function isCodeByConstruction(region: Region): boolean {
  if (region.kind === "fenced" || FENCE.test(region.text)) return true;
  for (const match of region.text.matchAll(INLINE_CODE)) {
    const span = match[1] ?? "";
    if (isCommand(span) || isCodePath(span)) return true;
  }
  const prose = region.text.replace(INLINE_CODE, " ").replace(URL, " ");
  return prose.split(/\s+/).some((t) => t !== "" && isPathToken(t));
}

// ---------------------------------------------------------------------------

type Questions = Record<string, unknown>;
type Answers = Record<string, Answer>;

function fillIndex(value: unknown, placeholder: string, i: number): unknown {
  if (typeof value === "string") return value.replaceAll(placeholder, `${i}`);
  if (Array.isArray(value))
    return value.map((v) => fillIndex(v, placeholder, i));
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [
        k.replaceAll(placeholder, `${i}`),
        fillIndex(v, placeholder, i),
      ]),
    );
  }
  return value;
}

/** Questions for the region at index `i` of a request's `state.regions`. */
function triageQuestionsFor(i: number, code: boolean): Questions {
  const filled = fillIndex(
    triageFile.questions,
    triageFile.placeholder,
    i,
  ) as Questions;
  if (code) delete filled[`evidenceKind_${i}`];
  return filled;
}

function stateRegion(r: Region): Region {
  return { id: r.id, kind: r.kind, text: r.text, headingPath: r.headingPath };
}

function answer(answers: Answers, id: string): Answer {
  const a = answers[id];
  if (a === undefined) {
    throw new JudgeError(
      `TypeSafe response has no answer for question "${id}".`,
    );
  }
  return a;
}

function choice<K extends string>(
  answers: Answers,
  id: string,
  options: readonly string[],
): { choice: K; probabilities: Record<K, number> } {
  const a = answer(answers, id) as ChoiceAnswer;
  if (typeof a.choice !== "string" || typeof a.probabilities !== "object") {
    throw new JudgeError(`Answer "${id}" is not a Choice answer.`);
  }
  const probabilities: Record<string, number> = {};
  for (const option of options) {
    const p = a.probabilities[option];
    if (typeof p !== "number") {
      throw new JudgeError(
        `Answer "${id}" has no probability for "${option}".`,
      );
    }
    probabilities[option] = p;
  }
  if (!options.includes(a.choice)) {
    throw new JudgeError(`Answer "${id}" chose unknown option "${a.choice}".`);
  }
  return {
    choice: a.choice as K,
    probabilities: probabilities as Record<K, number>,
  };
}

function noul(answers: Answers, id: string): number {
  const a = answer(answers, id);
  if (!("noul" in a) || typeof a.noul !== "number") {
    throw new JudgeError(`Answer "${id}" is not a Noul answer.`);
  }
  return a.noul;
}

// ---------------------------------------------------------------------------
// Request builders: pure, return the exact bodies sent.
// ---------------------------------------------------------------------------

/**
 * One request per chunk. Each request's state holds `doc` plus only that
 * chunk's regions; question IDs and `regions[i]` paths use the index inside
 * that request. Region ids are the caller's, so results merge by id.
 */
export function buildTriageRequests(
  model: string,
  doc: { path: string },
  regions: Region[],
): SystemOneRequest[] {
  const docState = { path: doc.path };
  const fixed = estimateTokens({
    model,
    state: { doc: docState, regions: [] },
    questions: {},
  });
  const costs = regions.map((r, i): RegionCost => {
    // Costed at the region's index in the whole document, which is never
    // shorter than its index inside a chunk. Each part is measured with its
    // key and enclosing brackets so the parts add up to at least the whole.
    const questions = Object.entries(
      triageQuestionsFor(i, isCodeByConstruction(r)),
    ).map(([id, question]) => estimateTokens({ [id]: question }));
    return {
      state: estimateTokens([stateRegion(r)]),
      questions: questions.reduce((sum, t) => sum + t, 0),
      largestQuestion: Math.max(...questions),
    };
  });
  return chunkRegions(costs, fixed).map((indices) => {
    const chunk = indices.map((i) => regions[i] as Region);
    return {
      model,
      state: { doc: docState, regions: chunk.map(stateRegion) },
      questions: Object.assign(
        {},
        ...chunk.map((r, i) => triageQuestionsFor(i, isCodeByConstruction(r))),
      ),
    };
  });
}

export function buildSupportsRequest(
  model: string,
  input: SupportsInput,
): SystemOneRequest {
  return {
    model,
    state: {
      claim: input.sentence,
      claim_context: input.context,
      evidence: input.evidence,
      evidence_source: input.source,
    },
    questions: supportsFile.questions,
  };
}

export function buildConflictRequest(
  model: string,
  a: ConflictSide,
  b: ConflictSide,
): SystemOneRequest {
  return {
    model,
    state: {
      a: { text: a.text, doc: a.doc, context: a.context },
      b: { text: b.text, doc: b.doc, context: b.context },
    },
    questions: conflictFile.questions,
  };
}

// ---------------------------------------------------------------------------
// Answer mappers: pure, so a cached `answers` object yields the same result.
// ---------------------------------------------------------------------------

/** Maps each chunk's answers back to its request's regions, in order. */
export function mapTriageAnswers(
  requests: SystemOneRequest[],
  answers: Answers[],
): TriageResult[] {
  if (requests.length !== answers.length) {
    throw new Error(
      `Triage has ${requests.length} requests but ${answers.length} answer sets.`,
    );
  }
  return requests.flatMap((request, k) => {
    const chunkAnswers = answers[k] as Answers;
    const regions = (request.state as { regions: Region[] }).regions;
    return regions.map((region, i): TriageResult => {
      const kind = choice<TriageKind>(chunkAnswers, `kind_${i}`, TRIAGE_KINDS);
      const result: TriageResult = {
        regionId: region.id,
        kind: kind.choice,
        kindProbabilities: kind.probabilities,
        kindConfidence: choiceConfidence(kind.probabilities),
      };
      if (kind.choice === "factual_claim") {
        if (isCodeByConstruction(region)) {
          result.evidenceKind = "code";
          result.evidenceProbabilities = { ...CODE_EVIDENCE };
        } else {
          const evidence = choice<EvidenceKind>(
            chunkAnswers,
            `evidenceKind_${i}`,
            EVIDENCE_KINDS,
          );
          result.evidenceKind = evidence.choice;
          result.evidenceProbabilities = evidence.probabilities;
        }
      }
      return result;
    });
  });
}

export function mapSupportsAnswers(answers: Answers): SupportsResult {
  const names = supportsFile.relationNames as Record<string, SupportsRelation>;
  const raw = choice<string>(answers, "relation", Object.keys(names));
  const probabilities = Object.fromEntries(
    SUPPORTS_RELATIONS.map((relation) => [relation, 0]),
  ) as Record<SupportsRelation, number>;
  for (const [option, p] of Object.entries(raw.probabilities)) {
    probabilities[names[option] as SupportsRelation] = p;
  }
  const addresses = noul(answers, "addresses");
  if (addresses < ADDRESSES_FLOOR) {
    return {
      relation: "does_not_address",
      probabilities,
      confidence: noulConfidence(addresses),
      addresses,
      forced: "addresses",
    };
  }
  return {
    relation: names[raw.choice] as SupportsRelation,
    probabilities,
    confidence: choiceConfidence(probabilities),
    addresses,
  };
}

export function mapConflictAnswers(answers: Answers): ConflictResult {
  const relation = choice<ConflictRelation>(
    answers,
    "relation",
    CONFLICT_RELATIONS,
  );
  const sameSubject = noul(answers, "same_subject");
  const contradicts = noul(answers, "contradicts");
  if (sameSubject < SAME_SUBJECT_FLOOR) {
    return {
      relation: "unrelated",
      probabilities: relation.probabilities,
      confidence: noulConfidence(sameSubject),
      sameSubject,
      contradicts,
      forced: "same_subject",
    };
  }
  return {
    relation: relation.choice,
    probabilities: relation.probabilities,
    confidence: choiceConfidence(relation.probabilities),
    sameSubject,
    contradicts,
  };
}

// ---------------------------------------------------------------------------

export interface JevJudgeOptions {
  /** Pinned versioned model id, such as `jev-1.13.0`; never an alias. */
  model: string;
  http: SystemOneClient;
}

export class JevJudge implements Judge {
  readonly model: string;
  private readonly http: SystemOneClient;

  constructor(options: JevJudgeOptions) {
    this.model = options.model;
    this.http = options.http;
  }

  /**
   * `request` and `answers` are arrays with one entry per chunk request, in
   * order, because a long document is split across several requests.
   */
  async triage(
    doc: { path: string },
    regions: Region[],
  ): Promise<JudgeResponse<TriageResult[]>> {
    const requests = buildTriageRequests(this.model, doc, regions);
    if (requests.length === 0) {
      return { model: this.model, request: [], answers: [], result: [] };
    }
    const responses = await Promise.all(
      requests.map((r) => this.http.systemOne(r)),
    );
    const answers = responses.map((r) => r.answers);
    return {
      model: sameModel(responses),
      request: requests,
      answers,
      result: mapTriageAnswers(requests, answers),
    };
  }

  async supports(input: SupportsInput): Promise<JudgeResponse<SupportsResult>> {
    const request = buildSupportsRequest(this.model, input);
    const response = await this.http.systemOne(request);
    return {
      model: response.model,
      request,
      answers: response.answers,
      result: mapSupportsAnswers(response.answers),
    };
  }

  async conflict(
    a: ConflictSide,
    b: ConflictSide,
  ): Promise<JudgeResponse<ConflictResult>> {
    const request = buildConflictRequest(this.model, a, b);
    const response = await this.http.systemOne(request);
    return {
      model: response.model,
      request,
      answers: response.answers,
      result: mapConflictAnswers(response.answers),
    };
  }
}

function sameModel(responses: SystemOneResponse[]): string {
  const models = new Set(responses.map((r) => r.model));
  if (models.size !== 1) {
    throw new JudgeError(
      `Triage chunks were answered by different models: ${[...models].join(", ")}.`,
    );
  }
  return responses[0]?.model ?? "";
}
