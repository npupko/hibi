/**
 * Render a `CheckReport` for a human (§9 rich + compact views). Diagnostics are
 * grouped by document; each suspect claim gets a verdict code, the quoted doc
 * sentence, a `path:line` code anchor, owner, freshness, and a `help` remediation
 * line. Clean documents collapse to a count. A lead line names the ref; a footer
 * restates the counts and the exit code. Shared by `check` and `diff`.
 *
 * Everything here is display-only: it joins authored facets and computes line
 * numbers / relative freshness off wall-clock, never touching the verdict, the
 * machine JSON, or the exit code (which the caller already holds).
 */

import { isWarnVerdict } from "../../core/gating.ts";
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
  remediation,
} from "./helpers.ts";
import type { OutputMode } from "./mode.ts";
import type { Style } from "./style.ts";
import {
  badge,
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
  /** Optional lead lines (the `diff` since/changed-files prefix). */
  lead?: string[];
  /** The verb name shown in the lead line. Default `check`. */
  verb?: string;
}

interface SuspectClaim {
  status: string;
  severity: Severity;
  assertion: Assertion | undefined;
  verdict: Verdict | undefined;
  propositionId: string;
}

/** Join a document's engine-determined suspect entries back to their verdicts. */
function suspectsFor(
  doc: DocumentReport,
  verdictsByProp: Map<string, Verdict>,
  ctx: CheckRenderContext,
): SuspectClaim[] {
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

/** The label a human acts on — the claim (assertion) id, falling back to the prop id. */
function claimLabel(claim: SuspectClaim): string {
  return claim.assertion?.id ?? claim.propositionId;
}

function leadLine(ctx: CheckRenderContext): string {
  const { style, report } = ctx;
  const verb = ctx.verb ?? "check";
  const ref =
    report.ref && report.ref !== "WORKTREE"
      ? style.dim(report.ref.slice(0, 7))
      : style.dim("worktree");
  return `${style.bold(`hibi ${verb}`)} ${ref}`;
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

/** A document header line: its worst-severity badge, path, and a suspect/clean count. */
function docHeader(
  doc: DocumentReport,
  suspects: SuspectClaim[],
  total: number,
  ctx: CheckRenderContext,
): string {
  const { style, mode } = ctx;
  const worst: Severity = suspects.some((s) => s.severity === "gating")
    ? "gating"
    : suspects.some((s) => s.severity === "warn")
      ? "warn"
      : suspects.length > 0
        ? "neutral"
        : "clean";
  const sym = badge(worst, mode.unicode, style);
  const clean = Math.max(total - suspects.length, 0);
  const count =
    suspects.length > 0
      ? style.dim(`${suspects.length} suspect · ${clean} clean`)
      : style.dim(`${total} claim${total === 1 ? "" : "s"} clean`);
  return `${sym} ${style.bold(doc.path)}   ${count}`;
}

/** A rich multi-line diagnostic block for one suspect claim. */
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
  }
  lines.push(`     ${style.dim(`help: ${remediation(claim.status)}`)}`);
  return lines;
}

/** A compact one-line summary for one suspect claim. */
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

  // Index verdicts by document and by proposition for the per-doc join.
  const verdictsByDoc = new Map<string, Verdict[]>();
  const verdictsByProp = new Map<string, Verdict>();
  for (const v of report.verdicts) {
    const list = verdictsByDoc.get(v.documentId) ?? [];
    list.push(v);
    verdictsByDoc.set(v.documentId, list);
    if (!verdictsByProp.has(v.propositionId))
      verdictsByProp.set(v.propositionId, v);
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

  // Suspect documents first (most severe to least), then clean ones collapsed.
  const enriched = report.documents.map((doc) => {
    const total = (verdictsByDoc.get(doc.id) ?? []).length;
    const suspects = suspectsFor(doc, verdictsByProp, ctx);
    return { doc, total, suspects };
  });
  const rank = (s: SuspectClaim[]) =>
    s.some((x) => x.severity === "gating")
      ? 0
      : s.some((x) => x.severity === "warn")
        ? 1
        : s.length > 0
          ? 2
          : 3;
  enriched.sort((a, b) => rank(a.suspects) - rank(b.suspects));

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
