/**
 * Gating policy (SPEC 7.4): the single place that decides whether a verdict
 * gates or warns. Every rule that is not plain from the axis values returns a
 * reason label.
 *
 * | condition                                           | `check` | `check --doc` |
 * |-----------------------------------------------------|---------|---------------|
 * | changed / orphaned / ambiguous on either side        | gate    | gate          |
 * | judge: refuted (only produced at or above threshold) | gate    | gate          |
 * | a conflict at or above threshold                     | gate    | gate          |
 * | expired                                              | warn    | gate          |
 * | moved, review, pending, unavailable                  | warn    | warn          |
 * | a conflict below threshold                           | warn    | warn          |
 * | suggested or retired claim                           | never   | never         |
 *
 * `warns` is false whenever `gates` is true.
 */
import {
  type AnchorState,
  type ConflictFinding,
  DEFAULT_JUDGE_THRESHOLD,
  type Enforcement,
  type EvidenceState,
  type JudgeState,
} from "./model.ts";

/** `store` is a whole-store `check`; `doc` is `check --doc`, where `expired` gates. */
export type GateMode = "store" | "doc";

const GATING_ANCHOR: ReadonlySet<EvidenceState> = new Set<EvidenceState>([
  "changed",
  "orphaned",
  "ambiguous",
]);

const WARN_ANCHOR: ReadonlySet<EvidenceState> = new Set<EvidenceState>([
  "moved",
  "unavailable",
]);

const WARN_JUDGE: ReadonlySet<JudgeState> = new Set<JudgeState>([
  "review",
  "pending",
]);

export function isGatingAnchor(state: EvidenceState | undefined): boolean {
  return state !== undefined && GATING_ANCHOR.has(state);
}

export function isWarnAnchor(state: EvidenceState | undefined): boolean {
  return state !== undefined && WARN_ANCHOR.has(state);
}

export interface VerdictDimensions {
  doc: AnchorState;
  evidence?: EvidenceState;
  judge?: JudgeState;
  expired: boolean;
  conflicts?: readonly ConflictFinding[];
}

export interface GateOptions {
  mode?: GateMode;
  /** `config.judge.threshold`; conflicts at or above it gate. */
  threshold?: number;
}

export interface GateResult {
  gates: boolean;
  warns: boolean;
  /** Labels for the rules applied that the axis values alone do not show. */
  reasons: string[];
}

function fmt(p: number): string {
  return p.toFixed(2);
}

/** Apply the SPEC 7.4 table to one verdict. */
export function gateVerdict(
  v: VerdictDimensions,
  enforcement: Enforcement,
  opts: GateOptions = {},
): GateResult {
  const mode = opts.mode ?? "store";
  const threshold = opts.threshold ?? DEFAULT_JUDGE_THRESHOLD;
  const reasons: string[] = [];
  let gates = false;
  let warns = false;

  if (isGatingAnchor(v.doc) || isGatingAnchor(v.evidence)) gates = true;
  if (isWarnAnchor(v.doc) || isWarnAnchor(v.evidence)) warns = true;
  if (v.judge === "refuted") gates = true;
  if (v.judge !== undefined && WARN_JUDGE.has(v.judge)) warns = true;

  for (const c of v.conflicts ?? []) {
    if (c.probability >= threshold) {
      gates = true;
      reasons.push(
        `conflicts with ${c.claimId} (p=${fmt(c.probability)} at or above threshold ${threshold}): ${c.reason}`,
      );
    } else {
      warns = true;
      reasons.push(
        `possible conflict with ${c.claimId} (p=${fmt(c.probability)} below threshold ${threshold}): ${c.reason}`,
      );
    }
  }

  if (v.expired) {
    if (mode === "doc") {
      gates = true;
      reasons.push("expired gates under check --doc");
    } else {
      warns = true;
      reasons.push(
        "expired warns in a whole-store check and gates under check --doc",
      );
    }
  }

  if (enforcement !== "enforced") {
    if (gates || warns) {
      reasons.push(`${enforcement} claim: never gates or warns`);
    }
    return { gates: false, warns: false, reasons };
  }
  return { gates, warns: warns && !gates, reasons };
}

/** Whether the verdict gates (SPEC 7.4). */
export function computeGates(
  v: VerdictDimensions,
  enforcement: Enforcement,
  opts: GateOptions = {},
): boolean {
  return gateVerdict(v, enforcement, opts).gates;
}

/**
 * Whether the verdict warns. A `check` verdict carries `warns` and that value
 * is returned; otherwise the table is applied in `store` mode.
 */
export function isWarnVerdict(
  v: VerdictDimensions & { gates: boolean; warns?: boolean },
  enforcement: Enforcement,
): boolean {
  if (enforcement !== "enforced" || v.gates) return false;
  if (v.warns !== undefined) return v.warns;
  return gateVerdict(v, enforcement).warns;
}
