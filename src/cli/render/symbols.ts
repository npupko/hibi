/**
 * Status → symbol + color, always paired with the machine status code so the
 * rendering is never color-only (§8 accessibility). Severity collapses the
 * side-tagged status vocabulary onto the four buckets a human scans for:
 * gating (✖), warning (⚠), clean (✓), and neutral/lifecycle (—).
 */

import type { Style } from "./style.ts";

export type Severity = "gating" | "warn" | "clean" | "neutral";

/** Side-tagged statuses that gate the build when the claim is enforced (§9). */
const GATING_STATUSES = new Set([
  "code:changed",
  "doc:changed",
  "code:orphaned",
  "doc:orphaned",
  "code:ambiguous",
  "doc:ambiguous",
  "behavior:refuted",
  "expired",
]);

/** Re-anchorable / advisory statuses — exit 3, never gate (§9/ADR-001). */
const WARN_STATUSES = new Set(["code:moved", "doc:moved", "behavior:at-risk"]);

/** Lifecycle / neutral tags — informational, no drift (§6). */
const NEUTRAL_STATUSES = new Set([
  "retracted",
  "superseded",
  "amended",
  "archived",
]);

/**
 * Classify a side-tagged status string. Note a `changed`/`orphaned` status only
 * truly gates when its claim is *enforced*; callers that hold the verdict prefer
 * `severityOfVerdict`, which consults `gates` directly. This string-only path is
 * for rollup views (overview, single-doc status) that carry the status word.
 */
export function severityOfStatus(status: string): Severity {
  if (GATING_STATUSES.has(status)) return "gating";
  if (WARN_STATUSES.has(status)) return "warn";
  if (NEUTRAL_STATUSES.has(status)) return "neutral";
  return "clean";
}

const UNICODE: Record<Severity, string> = {
  gating: "✖",
  warn: "⚠",
  clean: "✓",
  neutral: "—",
};

const ASCII: Record<Severity, string> = {
  gating: "x",
  warn: "!",
  clean: "v",
  neutral: "-",
};

/** The bare symbol for a severity, ASCII when unicode is disabled. */
export function severitySymbol(sev: Severity, unicode: boolean): string {
  return (unicode ? UNICODE : ASCII)[sev];
}

/** The color wrapper for a severity (a no-op when the Style has color off). */
export function severityColor(
  sev: Severity,
  style: Style,
): (s: string) => string {
  switch (sev) {
    case "gating":
      return style.red;
    case "warn":
      return style.yellow;
    case "clean":
      return style.green;
    default:
      return style.dim;
  }
}

/** A colored symbol for a severity, ready to drop into a line. */
export function badge(sev: Severity, unicode: boolean, style: Style): string {
  return severityColor(sev, style)(severitySymbol(sev, unicode));
}
