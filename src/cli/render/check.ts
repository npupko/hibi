/**
 * Render a `CheckReport` for a human (rich and compact views). Diagnostics are
 * grouped by document; each suspect claim gets a status, the quoted sentence,
 * a `path:line` code anchor, owner, freshness, and a `help` line. Clean
 * documents collapse to a count.
 */

import { isWarnVerdict } from "../../core/gating.ts";
import type { Remediation } from "../../core/model.ts";
import { topAction } from "../../core/remediation.ts";
import type {
  Assertion,
  CheckReport,
  DocumentReport,
  Proposition,
  Verdict,
} from "../../index.ts";
import {
  codeAnchor,
  docSentence,
  type FileRead,
  freshness,
  oneLine,
} from "./helpers.ts";
import type { OutputMode } from "./mode.ts";
import type { Style } from "./style.ts";
import {
  badge,
  rankSeverity,
  type Severity,
  severityColor,
  severitySymbol,
} from "./symbols.ts";

export interface CheckRenderContext {
  report: CheckReport;
  assertionsById: Map<string, Assertion>;
  propsById: Map<string, Proposition>;
  read: FileRead;
  style: Style;
  mode: OutputMode;
  /** Optional lead lines (the `--since` prefix). */
  lead?: string[];
}

interface SuspectClaim {
  status: string;
  severity: Severity;
  assertion: Assertion | undefined;
  verdict: Verdict | undefined;
  propositionId: string;
}

function suspectsFor(
  doc: DocumentReport,
  docVerdicts: Verdict[],
  ctx: CheckRenderContext,
): SuspectClaim[] {
  // Propositions are deduplicated across documents by fingerprint, so the
  // lookup must stay inside this document's own verdicts.
  const verdictsByProp = new Map<string, Verdict>();
  for (const v of docVerdicts) {
    if (!verdictsByProp.has(v.propositionId))
      verdictsByProp.set(v.propositionId, v);
  }
  return doc.suspect.map((s) => {
    const verdict = verdictsByProp.get(s.propositionId);
    const assertion = verdict
      ? ctx.assertionsById.get(verdict.assertionId)
      : undefined;
    const enforcement = assertion?.enforcement ?? "suggested";
    const severity: Severity = verdict?.gates
      ? "gating"
      : verdict && isWarnVerdict(verdict, enforcement)
        ? "warn"
        : "neutral";
    return {
      status: s.status,
      severity,
      assertion,
      verdict,
      propositionId: s.propositionId,
    };
  });
}

function claimLabel(claim: SuspectClaim): string {
  return claim.assertion?.id ?? claim.propositionId;
}

function leadLine(ctx: CheckRenderContext): string {
  const { style, report } = ctx;
  const ref =
    report.ref && report.ref !== "WORKTREE"
      ? style.dim(report.ref.slice(0, 7))
      : style.dim("worktree");
  return `${style.bold("hibi check")} ${ref}`;
}

function footer(ctx: CheckRenderContext): string {
  const { style, report } = ctx;
  const { gating, warning } = report.summary;
  const docCount = report.documents.length;
  const parts: string[] = [];
  parts.push(gating > 0 ? style.red(`${gating} gating`) : `${gating} gating`);
  parts.push(
    warning > 0 ? style.yellow(`${warning} warning`) : `${warning} warning`,
  );
  const exit =
    report.exitCode === 0
      ? style.green("exit 0")
      : style.bold(`exit ${report.exitCode}`);
  const docs = `${docCount} document${docCount === 1 ? "" : "s"}`;
  return `${style.dim("Found")} ${parts.join(", ")} ${style.dim(`across ${docs}.`)}  ${exit}`;
}

function worstOf(suspects: SuspectClaim[]): Severity {
  let worst: Severity = "clean";
  for (const s of suspects) {
    if (rankSeverity(s.severity) < rankSeverity(worst)) worst = s.severity;
  }
  return worst;
}

function docHeader(
  doc: DocumentReport,
  suspects: SuspectClaim[],
  total: number,
  ctx: CheckRenderContext,
): string {
  const { style, mode } = ctx;
  const worst = worstOf(suspects);
  const sym = badge(worst, mode.unicode, style);
  const clean = Math.max(total - suspects.length, 0);
  const count =
    suspects.length > 0
      ? style.dim(`${suspects.length} suspect · ${clean} clean`)
      : style.dim(`${total} claim${total === 1 ? "" : "s"} clean`);
  return `${sym} ${style.bold(doc.path)}   ${count}`;
}

function richBlock(claim: SuspectClaim, ctx: CheckRenderContext): string[] {
  const { style, mode } = ctx;
  const sym = severityColor(
    claim.severity,
    style,
  )(severitySymbol(claim.severity, mode.unicode));
  const status = severityColor(claim.severity, style)(claim.status);
  const lines: string[] = [];
  lines.push(`  ${sym} ${status}   ${style.cyan(claimLabel(claim))}`);
  if (claim.verdict) {
    const sentence = oneLine(docSentence(claim.verdict, ctx.propsById));
    lines.push(`     ${style.dim(`"${sentence}"`)}`);
    const anchor = codeAnchor(claim.verdict, claim.assertion, ctx.read);
    const facets: string[] = [];
    if (anchor) facets.push(anchor);
    if (claim.assertion?.owner && claim.assertion.owner !== "unknown")
      facets.push(`owner ${claim.assertion.owner}`);
    facets.push(freshness(claim.verdict, claim.assertion));
    if (facets.length) lines.push(`     ${style.dim(facets.join("   "))}`);
    const reasons = [
      ...new Set(
        claim.verdict.evidence.changedEvidence
          .map((c) => c.detail)
          .filter((r): r is string => Boolean(r)),
      ),
    ];
    if (reasons.length)
      lines.push(`     ${style.dim(`reason: ${reasons.join("; ")}`)}`);
  }
  if (mode.hints) {
    const help =
      remediationLine(claim.verdict?.remediation ?? null) ??
      lifecycleHint(claim.status);
    if (help) lines.push(`     ${style.dim(`help: ${help}`)}`);
  }
  return lines;
}

function remediationLine(rem: Remediation | null): string | null {
  const top = topAction(rem);
  if (!top) return null;
  return top.command
    ? `${top.command}  (${top.title}: ${top.rationale})`
    : `${top.title}: ${top.rationale}`;
}

function lifecycleHint(status: string): string | null {
  switch (status) {
    case "superseded":
      return "this document was superseded; read its successor instead";
    case "archived":
      return "this document was archived; read its successor instead";
    default:
      return null;
  }
}

function compactLine(claim: SuspectClaim, ctx: CheckRenderContext): string {
  const { style, mode } = ctx;
  const sym = severityColor(
    claim.severity,
    style,
  )(severitySymbol(claim.severity, mode.unicode));
  const status = severityColor(claim.severity, style)(claim.status.padEnd(16));
  const anchor = claim.verdict
    ? (codeAnchor(claim.verdict, claim.assertion, ctx.read) ?? "")
    : "";
  const sentence = claim.verdict
    ? oneLine(docSentence(claim.verdict, ctx.propsById), 40)
    : "";
  return `  ${sym} ${status} ${style.cyan(claimLabel(claim).padEnd(18))} ${style.dim(anchor)}   ${style.dim(`"${sentence}"`)}`;
}

export function renderCheck(ctx: CheckRenderContext): string {
  const { report, mode } = ctx;
  const compact = mode.kind === "compact";

  const verdictsByDoc = new Map<string, Verdict[]>();
  for (const v of report.verdicts) {
    const list = verdictsByDoc.get(v.documentId) ?? [];
    list.push(v);
    verdictsByDoc.set(v.documentId, list);
  }

  const out: string[] = [];
  for (const line of ctx.lead ?? []) out.push(line);
  out.push(leadLine(ctx));
  out.push("");

  if (report.documents.length === 0) {
    out.push(
      ctx.style.dim("No claims recorded. Run `hibi record` to add one."),
    );
    out.push("");
    out.push(footer(ctx));
    return `${out.join("\n")}\n`;
  }

  const enriched = report.documents.map((doc) => {
    const docVerdicts = verdictsByDoc.get(doc.id) ?? [];
    const total = docVerdicts.filter(
      (v) => ctx.assertionsById.get(v.assertionId)?.enforcement !== "retired",
    ).length;
    const suspects = suspectsFor(doc, docVerdicts, ctx);
    return { doc, total, suspects };
  });
  enriched.sort(
    (a, b) =>
      rankSeverity(worstOf(a.suspects)) - rankSeverity(worstOf(b.suspects)),
  );

  for (const { doc, total, suspects } of enriched) {
    out.push(docHeader(doc, suspects, total, ctx));
    if (suspects.length === 0) continue;
    for (const claim of suspects) {
      if (compact) out.push(compactLine(claim, ctx));
      else {
        out.push(...richBlock(claim, ctx));
        out.push("");
      }
    }
    if (compact) out.push("");
  }

  if (out[out.length - 1] !== "") out.push("");
  out.push(footer(ctx));
  return `${out.join("\n")}\n`;
}
