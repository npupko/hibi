/**
 * The repo-wide overview table for `check --overview`: every tracked document
 * with its worst status, claim counts, owner, verification ref, and
 * lifecycle, plus a footer with the store version and duplicate count.
 */

import { isWarnVerdict } from "../../core/gating.ts";
import { liveFingerprintCounts } from "../../engine/list.ts";
import type {
  Assertion,
  CheckReport,
  Proposition,
  Verdict,
} from "../../index.ts";
import type { OutputMode } from "./mode.ts";
import type { Style } from "./style.ts";
import {
  badge,
  rankSeverity,
  type Severity,
  severityColor,
  severitySymbol,
} from "./symbols.ts";
import { renderTable } from "./table.ts";

export interface OverviewContext {
  report: CheckReport;
  assertions: Assertion[];
  propositions: Proposition[];
  storeVersion: string;
  style: Style;
  mode: OutputMode;
}

/** Fingerprints asserted by more than one live claim. */
export function duplicateCount(
  assertions: Assertion[],
  propositions: Proposition[],
): number {
  return [...liveFingerprintCounts(assertions, propositions).values()].filter(
    (n) => n > 1,
  ).length;
}

export function renderOverview(ctx: OverviewContext): string {
  const { report, assertions, style, mode } = ctx;

  const assertById = new Map(assertions.map((a) => [a.id, a]));
  const verdictsByDoc = new Map<string, Verdict[]>();
  for (const v of report.verdicts) {
    const list = verdictsByDoc.get(v.documentId) ?? [];
    list.push(v);
    verdictsByDoc.set(v.documentId, list);
  }
  const ownersByDoc = new Map<string, Set<string>>();
  const refsByDoc = new Map<string, Set<string>>();
  for (const a of assertions) {
    if (!ownersByDoc.has(a.documentId))
      ownersByDoc.set(a.documentId, new Set());
    if (!refsByDoc.has(a.documentId)) refsByDoc.set(a.documentId, new Set());
    if (a.owner && a.owner !== "unknown")
      ownersByDoc.get(a.documentId)?.add(a.owner);
    if (a.ref) refsByDoc.get(a.documentId)?.add(a.ref);
  }

  const rows: { sortKey: number; cells: string[] }[] = [];
  for (const doc of report.documents) {
    const verdicts = verdictsByDoc.get(doc.id) ?? [];
    let gating = 0;
    let warn = 0;
    let retired = 0;
    for (const v of verdicts) {
      const enf = assertById.get(v.assertionId)?.enforcement ?? "suggested";
      if (enf === "retired") retired += 1;
      else if (v.gates) gating += 1;
      else if (isWarnVerdict(v, enf)) warn += 1;
    }
    const clean = verdicts.length - gating - warn - retired;

    const worst: Severity =
      gating > 0
        ? "gating"
        : warn > 0
          ? "warn"
          : doc.lifecycle !== "active"
            ? "neutral"
            : "clean";

    const owners = [...(ownersByDoc.get(doc.id) ?? [])];
    const owner =
      owners.length === 0
        ? "—"
        : owners.length === 1
          ? owners[0]
          : `${owners[0]} +${owners.length - 1}`;
    const refs = [...(refsByDoc.get(doc.id) ?? [])].filter(
      (r) => r !== "WORKTREE",
    );
    const verified =
      refs.length === 0
        ? "—"
        : refs.length === 1
          ? (refs[0]?.slice(0, 7) ?? "—")
          : "mixed";

    const counts = `${severityColor("gating", style)(`${gating}${severitySymbol("gating", mode.unicode)}`)} ${severityColor("warn", style)(`${warn}${severitySymbol("warn", mode.unicode)}`)} ${severityColor("clean", style)(`${clean}${severitySymbol("clean", mode.unicode)}`)}`;

    rows.push({
      sortKey: rankSeverity(worst),
      cells: [
        badge(worst, mode.unicode, style),
        doc.path,
        counts,
        owner ?? "—",
        verified,
        doc.lifecycle,
      ],
    });
  }
  rows.sort((a, b) => a.sortKey - b.sortKey);

  const out: string[] = [];
  out.push(style.bold("hibi check --overview"));
  out.push("");

  if (rows.length === 0) {
    out.push(
      style.dim("No documents tracked. Run `hibi record` to add a claim."),
    );
    return `${out.join("\n")}\n`;
  }

  out.push(
    ...renderTable(
      [
        { header: "" },
        { header: "Document", max: 48 },
        { header: "Claims" },
        { header: "Owner", max: 18 },
        { header: "Ref" },
        { header: "Lifecycle" },
      ],
      rows.map((r) => r.cells),
      { unicode: mode.unicode, indent: "  " },
    ),
  );
  out.push("");

  const { gating, warning, retired } = report.summary;
  const exit =
    report.exitCode === 0
      ? style.green("exit 0")
      : style.bold(`exit ${report.exitCode}`);
  const dupes = duplicateCount(ctx.assertions, ctx.propositions);
  out.push(
    `${style.dim("Tracking")} ${rows.length} document${rows.length === 1 ? "" : "s"}${style.dim(",")} ${gating > 0 ? style.red(`${gating} gating`) : `${gating} gating`}${style.dim(",")} ${warning > 0 ? style.yellow(`${warning} warning`) : `${warning} warning`}${style.dim(`, ${retired} retired.`)}  ${exit}`,
  );
  out.push(
    style.dim(
      `Store ${ctx.storeVersion}, ${dupes} duplicate proposition${dupes === 1 ? "" : "s"}.`,
    ),
  );
  return `${out.join("\n")}\n`;
}
