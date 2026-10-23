/**
 * Render a `CheckReport` for a human (rich and compact views). Diagnostics are
 * grouped by document; each suspect claim gets its status tags (anchor
 * states, `judge:<state>`, `conflict:<other id>`, `expired`), the quoted
 * sentence, a `path:line` evidence anchor, owner, freshness, and a `help`
 * line. `--explain` adds every reason label. Clean documents collapse to a
 * count. The worklist follows the documents.
 */

import type { Remediation } from "../../core/model.ts";
import { topAction } from "../../core/remediation.ts";
import {
  type CheckVerdict,
  orderStatuses,
  verdictStatuses,
  type WorklistItem,
} from "../../engine/check.ts";
import type { CheckReport, Claim, DocumentReport } from "../../index.ts";
import {
  evidenceAnchor,
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
  claimsById: Map<string, Claim>;
  read: FileRead;
  style: Style;
  mode: OutputMode;
  /** Optional lead lines (the `--since` prefix). */
  lead?: string[];
}

interface SuspectClaim {
  status: string;
  severity: Severity;
  claim: Claim | undefined;
  verdict: CheckVerdict | undefined;
  claimId: string;
}

/** A contradiction-pass warning label: the pending-pairs count or a review finding. */
const PENDING_PAIR = /^conflict checks pending: \d+$/;
const REVIEW_PAIR = /^possible conflict with clm_[0-9a-f]+: /;

/**
 * The status column: every tag of the verdict, most severe first, with the
 * other claim named on each conflict (`conflict:<id>`). The contradiction
 * pass's warnings (`conflict:pending`, `conflict:review`) are read from the
 * verdict's reasons, and `suspectStatus` (the engine's most severe tag for
 * the claim) is added when the verdict alone does not show it, so a warning
 * claim never prints as `unchanged`.
 */
export function statusTags(
  v: CheckVerdict,
  lcTags: string[] = [],
  suspectStatus?: string,
): string {
  const own = [...verdictStatuses(v), ...lcTags];
  if (v.reasons.some((r) => REVIEW_PAIR.test(r))) own.push("conflict:review");
  if (v.reasons.some((r) => PENDING_PAIR.test(r))) own.push("conflict:pending");
  if (suspectStatus !== undefined && !own.includes(suspectStatus)) {
    own.push(suspectStatus);
  }
  const tags = orderStatuses(own).flatMap((t) =>
    t === "conflict" ? v.conflicts.map((c) => `conflict:${c.claimId}`) : [t],
  );
  return tags.join(" ") || "unchanged";
}

function suspectsFor(
  doc: DocumentReport,
  docVerdicts: CheckVerdict[],
  ctx: CheckRenderContext,
): SuspectClaim[] {
  const verdictsById = new Map(docVerdicts.map((v) => [v.claimId, v]));
  const lcTags = doc.lifecycle === "active" ? [] : [doc.lifecycle];
  return doc.suspect.map((s) => {
    const verdict = verdictsById.get(s.claimId);
    const claim = ctx.claimsById.get(s.claimId);
    const severity: Severity = verdict?.gates
      ? "gating"
      : verdict?.warns
        ? "warn"
        : "neutral";
    return {
      status: verdict ? statusTags(verdict, lcTags, s.status) : s.status,
      severity,
      claim,
      verdict,
      claimId: s.claimId,
    };
  });
}

function claimLabel(claim: SuspectClaim): string {
  return claim.claimId;
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
  const { gating, warning, worklist } = report.summary;
  const docCount = report.documents.length;
  const parts: string[] = [];
  parts.push(gating > 0 ? style.red(`${gating} gating`) : `${gating} gating`);
  parts.push(
    warning > 0 ? style.yellow(`${warning} warning`) : `${warning} warning`,
  );
  if (worklist > 0) parts.push(`${worklist} in worklist`);
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
    const sentence = oneLine(claim.claim?.sentence ?? "");
    lines.push(`     ${style.dim(`"${sentence}"`)}`);
    const anchor = evidenceAnchor(claim.verdict, claim.claim, ctx.read);
    const facets: string[] = [];
    if (anchor) facets.push(anchor);
    if (claim.claim?.owner) facets.push(`owner ${claim.claim.owner}`);
    facets.push(freshness(claim.verdict, claim.claim));
    if (facets.length) lines.push(`     ${style.dim(facets.join("   "))}`);
    if (mode.explain) {
      for (const r of claim.verdict.reasons)
        lines.push(`     ${style.dim(`reason: ${r}`)}`);
    } else {
      const reasons = [
        ...new Set(
          claim.verdict.detail.changes
            .map((c) => c.detail)
            .filter((r): r is string => Boolean(r)),
        ),
      ];
      if (reasons.length)
        lines.push(`     ${style.dim(`reason: ${reasons.join("; ")}`)}`);
    }
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
    ? (evidenceAnchor(claim.verdict, claim.claim, ctx.read) ?? "")
    : "";
  const sentence = claim.verdict
    ? oneLine(claim.claim?.sentence ?? "", 40)
    : "";
  return `  ${sym} ${status} ${style.cyan(claimLabel(claim).padEnd(18))} ${style.dim(anchor)}   ${style.dim(`"${sentence}"`)}`;
}

function worklistLines(item: WorklistItem, ctx: CheckRenderContext): string[] {
  const { style, mode } = ctx;
  const head = `  ${style.yellow(item.reason.padEnd(13))} ${style.cyan(item.claimId)}   ${style.dim(item.document)}`;
  if (mode.kind === "compact") {
    return [`${head}   ${style.dim(`"${oneLine(item.sentence, 40)}"`)}`];
  }
  const lines = [head, `     ${style.dim(`"${oneLine(item.sentence)}"`)}`];
  const facets: string[] = [`kind ${item.kind}`];
  if (item.owner) facets.push(`owner ${item.owner}`);
  if (item.sources.length > 0)
    facets.push(
      `sources ${item.sources.map((s) => s.id ?? s.resource).join(", ")}`,
    );
  lines.push(`     ${style.dim(facets.join("   "))}`);
  if (item.procedure)
    lines.push(
      `     ${style.dim(`procedure: ${oneLine(item.procedure.instructions)}`)}`,
    );
  if (item.lastEvidence) {
    const e = item.lastEvidence;
    lines.push(
      `     ${style.dim(`last evidence: ${oneLine(e.summary, 48)} (${e.source}, ${e.by}, ${e.at})`)}`,
    );
  }
  return lines;
}

function worklistSection(ctx: CheckRenderContext): string[] {
  const items = ctx.report.worklist;
  if (items.length === 0) return [];
  const out = [ctx.style.bold(`Worklist (${items.length})`)];
  for (const item of items) {
    out.push(...worklistLines(item, ctx));
    if (ctx.mode.kind !== "compact") out.push("");
  }
  if (ctx.mode.kind === "compact") out.push("");
  return out;
}

export function renderCheck(ctx: CheckRenderContext): string {
  const { report, mode } = ctx;
  const compact = mode.kind === "compact";

  const verdictsByDoc = new Map<string, CheckVerdict[]>();
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
      (v) => ctx.claimsById.get(v.claimId)?.enforcement !== "retired",
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
  out.push(...worklistSection(ctx));
  out.push(footer(ctx));
  return `${out.join("\n")}\n`;
}
