/**
 * Gating policy: the single place that decides whether a verdict gates.
 *
 * Only an `enforced` claim can gate, and only `changed` / `orphaned` /
 * `ambiguous` (either side), `expired`, or a behavioral `refuted` do so.
 * `moved` is a warning: exit 0 by default, exit 2 under `--fail-on warn`.
 */
import type { AnchorState, BehaviorState, Enforcement } from "./model.ts";

const GATING_ANCHOR: ReadonlySet<AnchorState> = new Set<AnchorState>([
  "changed",
  "orphaned",
  "ambiguous",
]);

const WARN_ANCHOR: ReadonlySet<AnchorState> = new Set<AnchorState>(["moved"]);

export function isGatingAnchor(state: AnchorState): boolean {
  return GATING_ANCHOR.has(state);
}

export function isWarnAnchor(state: AnchorState): boolean {
  return WARN_ANCHOR.has(state);
}

export interface VerdictDimensions {
  doc: AnchorState;
  code: AnchorState;
  behavior?: BehaviorState;
  expired: boolean;
}

export function computeGates(
  v: VerdictDimensions,
  enforcement: Enforcement,
): boolean {
  if (enforcement !== "enforced") return false;
  return (
    isGatingAnchor(v.doc) ||
    isGatingAnchor(v.code) ||
    v.expired ||
    v.behavior === "refuted"
  );
}

/** A `moved` anchor on either side of an enforced claim that does not gate. */
export function isWarnVerdict(
  v: VerdictDimensions & { gates: boolean },
  enforcement: Enforcement,
): boolean {
  if (enforcement !== "enforced") return false;
  if (v.gates) return false;
  return isWarnAnchor(v.doc) || isWarnAnchor(v.code);
}
