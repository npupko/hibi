/**
 * Human output for `hibi evidence`: one line per item, then its reason labels.
 * `evidenceNext` is the envelope's `next` hint.
 */

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

/**
 * The `next` hint for an evidence report. A pending item's reason starts with
 * `judge unavailable:` (no key, network) or `judge error:` (an answer hibi
 * could not use); the hint names the cause the pending items actually have.
 */
export function evidenceNext(report: EvidenceReport): string {
  const reasons = report.results
    .filter((r) => r.judge === "pending")
    .flatMap((r) => r.reasons);
  if (reasons.length === 0) return "hibi check";
  const unavailable = reasons.some((r) => r.startsWith("judge unavailable"));
  const error = reasons.some((r) => r.startsWith("judge error"));
  const steps: string[] = [];
  if (unavailable) steps.push("set TYPESAFE_API_KEY or restore the network");
  if (error) {
    steps.push(
      "read the judge error in results[].reasons and fix its cause (the request or the judge's answer)",
    );
  }
  return `${steps.join("; ")}, then re-submit the pending items`;
}
