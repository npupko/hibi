/** Human output for `hibi evidence`: one line per item, then its reason labels. */

import type { EvidenceReport, EvidenceResult } from "../../engine/evidence.ts";
import type { OutputMode } from "./mode.ts";
import type { Style } from "./style.ts";
import { badge, type Severity } from "./symbols.ts";

const SEVERITY: Record<EvidenceResult["judge"], Severity> = {
  supported: "clean",
  refuted: "gating",
  review: "warn",
  pending: "warn",
};

export function renderEvidence(
  report: EvidenceReport,
  style: Style,
  mode: OutputMode,
): string {
  const lines: string[] = [];
  for (const r of report.results) {
    const tags = [
      r.attested ? "attested" : "not attested",
      ...(r.cached ? ["cached"] : []),
    ];
    lines.push(
      `${badge(SEVERITY[r.judge], mode.unicode, style)} ${r.judge.padEnd(9)} ${style.cyan(r.claimId)}  ${style.dim(`(${tags.join(", ")})`)}`,
    );
    for (const reason of r.reasons) lines.push(`    ${style.dim(reason)}`);
  }
  const s = report.summary;
  lines.push(
    style.dim(
      `${s.supported} supported · ${s.refuted} refuted · ${s.review} review · ${s.pending} pending · ${s.requests} judge request${s.requests === 1 ? "" : "s"}`,
    ),
  );
  return `${lines.join("\n")}\n`;
}
