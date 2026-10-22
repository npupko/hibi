/**
 * Human rendering of `hibi audit`: one line per region with its line number,
 * kind, probability, and to-do, then the summary. The to-do mapping lives only
 * in `adviceFor`; the JSON report carries `kind` and never advice (SPEC 10).
 */

import type { AuditReport, AuditRow } from "../../engine/audit.ts";
import { oneLine } from "./helpers.ts";
import type { OutputMode } from "./mode.ts";
import type { Style } from "./style.ts";

/**
 * The fixed to-do for a row, or null when there is nothing to do:
 * - covered (a claim backs the region): nothing
 * - uncovered `code`, `reference`, `attested`: "record"
 * - `unverifiable`: "rewrite as an attributed opinion, or remove"
 * - `not-a-claim`: nothing
 * - `unsure` on the triage kind: "unsure: read it yourself"
 * - `unsure` on the evidence kind: "unsure: evidence kind"
 * - `pending`: "pending: judge unavailable"
 */
export function adviceFor(row: AuditRow): string | null {
  if (row.claimId !== null) return null;
  switch (row.kind) {
    case "code":
    case "reference":
    case "attested":
      return "record";
    case "unverifiable":
      return "rewrite as an attributed opinion, or remove";
    case "not-a-claim":
      return null;
    case "unsure":
      return row.unsure === "evidenceKind"
        ? "unsure: evidence kind"
        : "unsure: read it yourself";
    case "pending":
      return "pending: judge unavailable";
  }
}

const KIND_WIDTH = "not-a-claim".length;

function rowLine(row: AuditRow, style: Style, lineWidth: number): string {
  const line = style.dim(`L${row.region.line}`.padEnd(lineWidth + 1));
  const kind = row.kind.padEnd(KIND_WIDTH);
  const p = row.probability === undefined ? "  — " : row.probability.toFixed(2);
  const advice = adviceFor(row);
  const parts = [line, kind, style.dim(p)];
  if (row.claimId !== null) parts.push(style.cyan(row.claimId));
  if (row.duplicateOf !== null) {
    parts.push(style.dim(`duplicate of ${row.duplicateOf}`));
  }
  if (advice !== null) {
    const colored =
      advice === "record" ? style.yellow(advice) : style.bold(advice);
    parts.push(colored);
  }
  parts.push(style.dim(`"${oneLine(row.region.text, 60)}"`));
  return `  ${parts.join("  ")}`;
}

export function renderAudit(
  report: AuditReport,
  style: Style,
  _mode: OutputMode,
): string {
  const { summary } = report;
  const lineWidth = Math.max(
    0,
    ...report.rows.map((r) => `${r.region.line}`.length),
  );
  const lines = report.rows.map((r) => rowLine(r, style, lineWidth));
  const counts = Object.entries(summary.byKind)
    .filter(([, n]) => n > 0)
    .map(([k, n]) => `${k} ${n}`)
    .join(", ");
  const head = style.bold(report.doc);
  const tail = style.dim(
    `${summary.regions} regions, ${summary.covered} covered, ${summary.uncovered} uncovered; ${counts || "no regions"}; ${summary.requests} judge request${summary.requests === 1 ? "" : "s"}`,
  );
  return `${head}\n${lines.length > 0 ? `${lines.join("\n")}\n` : ""}${tail}\n`;
}
