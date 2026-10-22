/**
 * Gating policy: the single place that decides whether a verdict gates.
 *
 * Only an `enforced` claim can gate, and only `changed` / `orphaned` /
 * `ambiguous` (either side), `expired`, or a judge `refuted` do so.
 * `moved` is a warning: exit 0 by default, exit 2 under `--fail-on warn`.
 * `unavailable` never gates.
 */
import type {
  AnchorState,
  Enforcement,
  EvidenceState,
  JudgeState,
} from "./model.ts";

const GATING_ANCHOR: ReadonlySet<EvidenceState> = new Set<EvidenceState>([
  "changed",
  "orphaned",
  "ambiguous",
]);

const WARN_ANCHOR: ReadonlySet<EvidenceState> = new Set<EvidenceState>([
  "moved",
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
}

export function computeGates(
  v: VerdictDimensions,
  enforcement: Enforcement,
): boolean {
  if (enforcement !== "enforced") return false;
  return (
    isGatingAnchor(v.doc) ||
    isGatingAnchor(v.evidence) ||
    v.expired ||
    v.judge === "refuted"
  );
}

/** A `moved` span on either side of an enforced claim that does not gate. */
export function isWarnVerdict(
  v: VerdictDimensions & { gates: boolean },
  enforcement: Enforcement,
): boolean {
  if (enforcement !== "enforced") return false;
  if (v.gates) return false;
  return isWarnAnchor(v.doc) || isWarnAnchor(v.evidence);
}
