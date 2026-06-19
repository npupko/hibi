/**
 * Gating policy (§9 exit-code contract; ADR-001 gating invariant) — the single
 * place that decides whether a verdict gates the build.
 *
 * The rule is deterministic and narrow: **only an `enforced` claim can gate**,
 * and only `changed` / `orphaned` / `ambiguous` (either side), `expired`, or a
 * behavioral `refuted` may do so. `moved` and `at-risk` are *warnings*
 * (exit 3) and **never** gate. `suggested` / `retired` / `unanchored-legacy`
 * claims never gate. Keeping this logic in one pure module is what lets the
 * resolver, the registry, and `check` agree on the same predicate.
 */
import type { AnchorState, BehaviorState, Enforcement } from "./model.ts";

/** Anchor states that gate (exit 2) on an enforced claim. */
const GATING_ANCHOR: ReadonlySet<AnchorState> = new Set<AnchorState>([
  "changed",
  "orphaned",
  "ambiguous",
]);

/** Anchor states that warn but never gate (exit 3). */
const WARN_ANCHOR: ReadonlySet<AnchorState> = new Set<AnchorState>(["moved"]);

/** True if an anchor state is gating-eligible (changed/orphaned/ambiguous). */
export function isGatingAnchor(state: AnchorState): boolean {
  return GATING_ANCHOR.has(state);
}

/** True if an anchor state is a non-gating warning (moved). */
export function isWarnAnchor(state: AnchorState): boolean {
  return WARN_ANCHOR.has(state);
}

/** The computed dimensions a gating decision reads. */
export interface VerdictDimensions {
  doc: AnchorState;
  code: AnchorState;
  behavior?: BehaviorState;
  expired: boolean;
}

/**
 * Whether a verdict gates the build (§9). Only an `enforced` claim can gate; the
 * gating conditions are a gating anchor state on either side, the `expired`
 * flag, or a behavioral `refuted`.
 */
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

/**
 * Whether a verdict is a non-gating warning (exit 3) — a `moved` anchor on
 * either side, or a behavioral `at-risk`, on an enforced claim that does not
 * already gate. `suggested`/`retired`/`unanchored-legacy` claims never warn.
 */
export function isWarnVerdict(
  v: VerdictDimensions & { gates: boolean },
  enforcement: Enforcement,
): boolean {
  if (enforcement !== "enforced") return false;
  if (v.gates) return false;
  return (
    isWarnAnchor(v.doc) || isWarnAnchor(v.code) || v.behavior === "at-risk"
  );
}
