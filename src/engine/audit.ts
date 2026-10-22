/**
 * `audit --doc <path>` (SPEC 10): split a document into regions, match each
 * region to the live claims on that document by their doc anchor, triage the
 * uncovered regions with the judge, and report one row per region.
 *
 * JSON shape (the CLI wraps it in the usual `{ ok, action, schemaVersion }`
 * envelope):
 *
 * ```json
 * {
 *   "doc": "docs/pricing.md",
 *   "summary": {
 *     "regions": 12, "covered": 3, "uncovered": 9,
 *     "byKind": { "code": 2, "reference": 0, "attested": 1, "unverifiable": 1,
 *                 "not-a-claim": 3, "unsure": 1, "pending": 1 },
 *     "requests": 1
 *   },
 *   "rows": [
 *     { "region": { "id": "r4", "line": 42, "kind": "sentence", "text": "Pro costs $29/month." },
 *       "claimId": null,
 *       "kind": "attested",
 *       "probability": 0.93,
 *       "duplicateOf": null },
 *     { "region": { "id": "r5", "line": 43, "kind": "sentence", "text": "It is fast." },
 *       "claimId": null,
 *       "kind": "unsure",
 *       "unsure": "kind",
 *       "probabilities": { "kind": { "factual_claim": 0.4, "instruction": 0.05, "opinion": 0.5, "narrative": 0.05 } },
 *       "duplicateOf": null }
 *   ]
 * }
 * ```
 *
 * Row fields:
 * - `claimId`: the live claim whose located doc span overlaps the region, or
 *   null. A covered row's `kind` is that claim's kind and it carries no
 *   probability; the judge is not asked about covered regions.
 * - `kind` for an uncovered row comes from triage (SPEC 8.3):
 *   `factual_claim` + `code` -> `code`, + `other_doc` -> `reference`,
 *   + `external` -> `attested`, + `nothing` -> `unverifiable`; any other
 *   triage kind -> `not-a-claim`. The decision is taken only when the triage
 *   kind's Choice confidence is at or above `judge.threshold` and, for a
 *   `factual_claim`, the evidence-kind Choice confidence is too. Otherwise the
 *   row is `unsure`, names the Choice that fell short in `unsure` (`kind` or
 *   `evidenceKind`; `kind` is checked first), and carries `probabilities` (the
 *   triage kind distribution, plus the evidence-kind distribution when the top
 *   kind is `factual_claim`) instead of `probability`. `pending` means the
 *   judge was unavailable; such a row has neither field.
 * - `probability`: the judge's probability for the triage kind it picked.
 * - `duplicateOf`: for a covered row, the claim this row's claim duplicates
 *   (from the contradiction and duplicate pass); null otherwise.
 * - `summary.requests`: triage requests the judge answered in this run. Cache
 *   hits are not counted, so a second run over the same text reports 0.
 */

import { resolveSide } from "../algo/resolve.ts";
import { documentIdForPath } from "../core/ids.ts";
import type { Claim, ClaimKind, Config } from "../core/model.ts";
import { cacheKey, type JudgeCache } from "../judge/cache.ts";
import type { Answer } from "../judge/http.ts";
import { buildTriageRequests, mapTriageAnswers } from "../judge/jev.ts";
import {
  choiceConfidence,
  type EvidenceKind,
  type Judge,
  JudgeError,
  judgeFailureLabel,
  type Region,
  type RegionKind,
  type TriageKind,
  type TriageResult,
} from "../judge/types.ts";
import { type DocRegion, splitRegions } from "./regions.ts";

export type AuditKind =
  | ClaimKind
  | "unverifiable"
  | "not-a-claim"
  | "unsure"
  | "pending";

export const AUDIT_KINDS: readonly AuditKind[] = [
  "code",
  "reference",
  "attested",
  "unverifiable",
  "not-a-claim",
  "unsure",
  "pending",
];

export interface AuditProbabilities {
  kind: Record<TriageKind, number>;
  evidenceKind?: Record<EvidenceKind, number>;
}

export interface AuditRow {
  region: { id: string; line: number; kind: RegionKind; text: string };
  claimId: string | null;
  kind: AuditKind;
  probability?: number;
  probabilities?: AuditProbabilities;
  /** On an `unsure` row: the Choice whose confidence fell below `judge.threshold`. */
  unsure?: "kind" | "evidenceKind";
  duplicateOf: string | null;
}

export interface AuditSummary {
  regions: number;
  covered: number;
  uncovered: number;
  byKind: Record<AuditKind, number>;
  requests: number;
}

export interface AuditReport {
  doc: string;
  summary: AuditSummary;
  rows: AuditRow[];
}

export interface AuditInput {
  store: { judgeCache(): JudgeCache };
  judge: Judge;
  /** Repository-relative path of the document. */
  docPath: string;
  /** The document's current text. */
  text: string;
  /** Claims in the store; only live claims on `docPath` are used. */
  claims: Claim[];
  config: Config;
  /** The claim a covered row's claim duplicates, or null. */
  duplicateOf?: (claimId: string) => string | null;
  /** Clock for cache entry timestamps. */
  now?: () => Date;
}

/** The first `config.ignore` glob that matches `docPath`, or null. */
export function ignoredBy(config: Config, docPath: string): string | null {
  return config.ignore.find((g) => new Bun.Glob(g).match(docPath)) ?? null;
}

function overlaps(
  a: { start: number; end: number },
  b: { start: number; end: number },
): boolean {
  return a.start < b.end && a.end > b.start;
}

/** SPEC 8.3 mapping from a confident triage answer to an audit kind. */
export function auditKindOf(result: TriageResult): AuditKind {
  if (result.kind !== "factual_claim") return "not-a-claim";
  switch (result.evidenceKind) {
    case "code":
      return "code";
    case "other_doc":
      return "reference";
    case "external":
      return "attested";
    default:
      return "unverifiable";
  }
}

/** A row with its keys in the documented order: region, claimId, kind, the kind's fields, duplicateOf. */
function row(
  base: Pick<AuditRow, "region" | "claimId" | "duplicateOf">,
  fields: Omit<AuditRow, "region" | "claimId" | "duplicateOf">,
): AuditRow {
  return {
    region: base.region,
    claimId: base.claimId,
    ...fields,
    duplicateOf: base.duplicateOf,
  };
}

function triageRow(
  base: Pick<AuditRow, "region" | "claimId" | "duplicateOf">,
  result: TriageResult,
  threshold: number,
): AuditRow {
  const unsure =
    result.kindConfidence < threshold
      ? "kind"
      : result.evidenceProbabilities !== undefined &&
          choiceConfidence(result.evidenceProbabilities) < threshold
        ? "evidenceKind"
        : null;
  if (unsure === null) {
    return row(base, {
      kind: auditKindOf(result),
      probability: result.kindProbabilities[result.kind],
    });
  }
  const probabilities: AuditProbabilities = { kind: result.kindProbabilities };
  if (result.evidenceProbabilities !== undefined) {
    probabilities.evidenceKind = result.evidenceProbabilities;
  }
  return row(base, { kind: "unsure", unsure, probabilities });
}

interface Triaged {
  results: Map<string, TriageResult>;
  answered: number;
}

/**
 * Triage `regions`: one request per chunk, each looked up in the judge cache
 * first. Only misses reach the judge. A chunk the judge cannot answer leaves
 * its regions without a result (pending).
 */
async function triage(
  input: AuditInput,
  regions: Region[],
  docId: string,
): Promise<Triaged> {
  const cache = input.store.judgeCache();
  const model = input.judge.model;
  const doc = { path: input.docPath };
  const requests = buildTriageRequests(model, doc, regions);
  const now = input.now ?? (() => new Date());
  let answered = 0;
  const perRequest = await Promise.all(
    requests.map(async (request) => {
      const key = cacheKey(model, request);
      const hit = await cache.get(key);
      let answers: Record<string, Answer>;
      if (hit !== undefined) {
        answers = hit.answers as Record<string, Answer>;
      } else {
        const chunk = (request.state as { regions: Region[] }).regions;
        let response: Awaited<ReturnType<Judge["triage"]>>;
        try {
          response = await input.judge.triage(doc, chunk);
          const raw = response.answers as Record<string, Answer>[];
          if (!Array.isArray(raw) || raw.length !== 1) {
            throw new JudgeError(
              "The judge answered one triage chunk with more or fewer than one answer set.",
            );
          }
          answers = raw[0] as Record<string, Answer>;
        } catch (e) {
          // Unavailable or unusable: the chunk's rows are pending.
          if (judgeFailureLabel(e) !== undefined) return [];
          throw e;
        }
        answered++;
        await cache.put({
          key,
          model: response.model,
          judgment: "triage",
          subject: { docId },
          request,
          answers,
          at: now().toISOString(),
        });
      }
      return mapTriageAnswers([request], [answers]);
    }),
  );
  const results = new Map<string, TriageResult>();
  for (const r of perRequest.flat()) results.set(r.regionId, r);
  return { results, answered };
}

/** Audit one document. Throws when the document matches an ignore glob. */
export async function runAudit(input: AuditInput): Promise<AuditReport> {
  const { docPath, text, config } = input;
  const glob = ignoredBy(config, docPath);
  if (glob !== null) {
    throw new Error(
      `${docPath} matches the ignore glob "${glob}" in .claims/config.json; hibi does not audit ignored documents.`,
    );
  }
  const duplicateOf = input.duplicateOf ?? (() => null);
  const docId = documentIdForPath(docPath);

  const spans: { claim: Claim; start: number; end: number }[] = [];
  const live = input.claims
    .filter((c) => c.documentId === docId && c.enforcement !== "retired")
    .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  for (const claim of live) {
    const side = resolveSide(claim.doc, text);
    if (side.region) spans.push({ claim, ...side.region });
  }

  const regions = splitRegions(text);
  const coveredBy = new Map<string, Claim>();
  for (const region of regions) {
    const hit = spans.find((s) => overlaps(s, region));
    if (hit) coveredBy.set(region.id, hit.claim);
  }
  const uncovered = regions.filter((r) => !coveredBy.has(r.id));
  const { results, answered } = await triage(input, uncovered, docId);

  const rows = regions.map((region: DocRegion): AuditRow => {
    const base = {
      region: {
        id: region.id,
        line: region.line,
        kind: region.kind,
        text: region.text,
      },
      claimId: null,
      duplicateOf: null,
    };
    const claim = coveredBy.get(region.id);
    if (claim) {
      return row(
        { ...base, claimId: claim.id, duplicateOf: duplicateOf(claim.id) },
        { kind: claim.kind },
      );
    }
    const result = results.get(region.id);
    if (!result) return row(base, { kind: "pending" });
    return triageRow(base, result, config.judge.threshold);
  });

  const byKind = Object.fromEntries(AUDIT_KINDS.map((k) => [k, 0])) as Record<
    AuditKind,
    number
  >;
  for (const row of rows) byKind[row.kind]++;
  return {
    doc: docPath,
    summary: {
      regions: rows.length,
      covered: coveredBy.size,
      uncovered: rows.length - coveredBy.size,
      byKind,
      requests: answered,
    },
    rows,
  };
}

/** Kinds that should be recorded as claims. */
const RECORDABLE: readonly AuditKind[] = ["code", "reference", "attested"];

/** An uncovered row whose kind says it should be recorded. */
export function isUncoveredRecordable(row: AuditRow): boolean {
  return row.claimId === null && RECORDABLE.includes(row.kind);
}

/** `--fail-uncovered`: 2 when any uncovered row is `code`, `reference`, or `attested`; else 0. */
export function auditExitCode(
  report: AuditReport,
  opts: { failUncovered?: boolean } = {},
): 0 | 2 {
  return opts.failUncovered && report.rows.some(isUncoveredRecordable) ? 2 : 0;
}
