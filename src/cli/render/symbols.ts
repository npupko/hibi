/**
 * Status → symbol + color, always paired with the status text so the
 * rendering is never color-only. Severity collapses the side-tagged status
 * vocabulary onto four buckets: gating, warn, clean, neutral (lifecycle).
 */

import type { Style } from "./style.ts";

export type Severity = "gating" | "warn" | "clean" | "neutral";

/** Sort rank: gating first, then warn, neutral, clean. */
export function rankSeverity(s: Severity): number {
  return s === "gating" ? 0 : s === "warn" ? 1 : s === "neutral" ? 2 : 3;
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

export function severitySymbol(sev: Severity, unicode: boolean): string {
  return (unicode ? UNICODE : ASCII)[sev];
}

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

export function badge(sev: Severity, unicode: boolean, style: Style): string {
  return severityColor(sev, style)(severitySymbol(sev, unicode));
}
