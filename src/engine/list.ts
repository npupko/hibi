/**
 * `list` (SPEC 11): one lean row per claim, filtered by state and by path.
 * Built from a live `CheckReport` and the contradiction pass that `check`
 * ran, so it shares their verdicts and gating.
 *
 * States (`--state`):
 *
 * | state | rows |
 * |---|---|
 * | `all` | every claim |
 * | `gating`, `warning`, `clean` | live claims whose verdict gates, warns, or neither |
 * | `pending` | the `check` worklist (SPEC 7.5) |
 * | `expired` | live claims whose ttl ran out (SPEC 5 counting rule) |
 * | `conflict` | live claims with a finding from the contradiction pass (gating or review) |
 * | `duplicate` | live claims with a duplicate candidate from the pass, or another live claim with the same normalized sentence; each row lists them in `duplicates[]` with a `why` |
 * | `suggested` | claims recorded with `--suggest` |
 * | `stranded` | live claims on a superseded or archived document |
 *
 * Row: `{ claimId, documentId, kind, sentence, enforcement, owner, ttl,
 * expiresAt, side?, state, duplicates? }`. `state` is the verdict's outcome:
 * `gating`, `warning`, `clean`, or `retired`. `side` is set under `--path`
 * (which side names the file). `owner`, `ttl`, and `expiresAt` are null when
 * absent; `expiresAt` is null without a ttl or a counting attestation.
 */

import type {
  Claim,
  ClaimKind,
  DocumentLifecycle,
  Enforcement,
  Verdict,
} from "../core/model.ts";
import type { CheckReport } from "./check.ts";
import type { ConflictPassResult } from "./conflict.ts";
import { expiresAt, isExpired } from "./expiry.ts";
import { normalizeSentence } from "./hash.ts";

export const LIST_STATES = [
  "all",
  "gating",
  "warning",
  "clean",
  "pending",
  "expired",
  "conflict",
  "duplicate",
  "suggested",
  "stranded",
] as const;
export type ListState = (typeof LIST_STATES)[number];

/** A row's outcome: the verdict gates, warns, neither, or the claim is retired. */
export type RowState = "gating" | "warning" | "clean" | "retired";

export interface DuplicateRef {
  claimId: string;
  /** Which rule found it: the pass's reason label, or `same sentence`. */
  why: string;
}

export interface ListRow {
  claimId: string;
  documentId: string;
  kind: ClaimKind;
  sentence: string;
  enforcement: Enforcement;
  owner: string | null;
  ttl: string | null;
  expiresAt: string | null;
  /** Under `--path`: which side names the path. */
  side?: "doc" | "evidence";
  state: RowState;
  /** Under `--state duplicate`: the claims this one duplicates. */
  duplicates?: DuplicateRef[];
}

export interface ListResult {
  state: ListState;
  path?: string;
  count: number;
  claims: ListRow[];
}

export interface ToListRowsInput {
  report: CheckReport;
  claims: Claim[];
  /** The contradiction pass `check` ran. */
  pass: ConflictPassResult;
  lifecycle: Map<string, DocumentLifecycle>;
  state?: ListState;
  path?: string;
  now?: Date;
}

/** Which side of a claim names `path`, if any. */
export function matchedSide(
  c: Claim,
  path: string,
): "doc" | "evidence" | undefined {
  if (c.doc.file === path) return "doc";
  if (c.evidence.some((b) => b.file === path)) return "evidence";
  return undefined;
}

function rowState(v: Verdict, enforcement: Enforcement): RowState {
  if (enforcement === "retired") return "retired";
  if (v.gates) return "gating";
  if (v.warns) return "warning";
  return "clean";
}

/** Duplicates per live claim: the pass's candidates, then same normalized sentence. */
function duplicateIndex(
  claims: Claim[],
  pass: ConflictPassResult,
): Map<string, DuplicateRef[]> {
  const out = new Map<string, DuplicateRef[]>();
  const add = (id: string, ref: DuplicateRef) => {
    const list = out.get(id) ?? [];
    if (!list.some((r) => r.claimId === ref.claimId)) list.push(ref);
    out.set(id, list);
  };
  for (const d of pass.duplicates) {
    add(d.claimId, { claimId: d.duplicateOf, why: d.reason });
  }
  const bySentence = new Map<string, string[]>();
  for (const c of claims) {
    if (c.enforcement === "retired") continue;
    const key = normalizeSentence(c.sentence);
    bySentence.set(key, [...(bySentence.get(key) ?? []), c.id]);
  }
  for (const ids of bySentence.values()) {
    for (const id of ids) {
      for (const other of ids) {
        if (other !== id) add(id, { claimId: other, why: "same sentence" });
      }
    }
  }
  return out;
}

export function toListRows(input: ToListRowsInput): ListResult {
  const state = input.state ?? "all";
  const now = input.now ?? new Date();
  const claimById = new Map(input.claims.map((c) => [c.id, c]));
  const worklist = new Set(input.report.worklist.map((w) => w.claimId));
  const inConflict = new Set(input.pass.findings.map((f) => f.claimId));
  const duplicates =
    state === "duplicate"
      ? duplicateIndex(input.claims, input.pass)
      : new Map<string, DuplicateRef[]>();

  const rows: ListRow[] = [];
  for (const v of input.report.verdicts) {
    const claim = claimById.get(v.claimId);
    if (!claim) continue;
    const live = claim.enforcement !== "retired";
    const lifecycle = input.lifecycle.get(claim.documentId) ?? "active";
    const outcome = rowState(v, claim.enforcement);

    let side: "doc" | "evidence" | undefined;
    if (input.path !== undefined) {
      side = matchedSide(claim, input.path);
      if (!side) continue;
    }

    const matches = (() => {
      switch (state) {
        case "all":
          return true;
        case "gating":
        case "warning":
        case "clean":
          return outcome === state;
        case "pending":
          return worklist.has(claim.id);
        case "expired":
          return live && isExpired(claim, now);
        case "conflict":
          return live && inConflict.has(claim.id);
        case "duplicate":
          return live && duplicates.has(claim.id);
        case "suggested":
          return claim.enforcement === "suggested";
        case "stranded":
          return live && lifecycle !== "active";
      }
    })();
    if (!matches) continue;

    const until = expiresAt(claim);
    rows.push({
      claimId: claim.id,
      documentId: claim.documentId,
      kind: claim.kind,
      sentence: claim.sentence,
      enforcement: claim.enforcement,
      owner: claim.owner ?? null,
      ttl: claim.ttl ?? null,
      expiresAt: until === undefined ? null : until.toISOString(),
      ...(side ? { side } : {}),
      state: outcome,
      ...(state === "duplicate"
        ? { duplicates: duplicates.get(claim.id) ?? [] }
        : {}),
    });
  }

  const rank: Record<RowState, number> = {
    gating: 0,
    warning: 1,
    clean: 2,
    retired: 3,
  };
  rows.sort((a, b) => rank[a.state] - rank[b.state]);

  return {
    state,
    ...(input.path !== undefined ? { path: input.path } : {}),
    count: rows.length,
    claims: rows,
  };
}
