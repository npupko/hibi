/**
 * Two human renderers for `status` (§9): a single-document detail card (from a
 * `StatusResult`) and the repo-wide overview table (from a full `CheckReport` +
 * assertions). The overview is the proactive "survey the health of my docs"
 * view — every tracked document with its worst status, claim counts, owner,
 * verification ref, and lifecycle. Both are display-only.
 */

import { isWarnVerdict } from "../../core/gating.ts";
import type {
  Assertion,
  CheckReport,
  StatusResult,
  Verdict,
} from "../../index.ts";
import type { OutputMode } from "./mode.ts";
import type { Style } from "./style.ts";
import {
  badge,
  type Severity,
  severityColor,
  severityOfStatus,
  severitySymbol,
} from "./symbols.ts";
import { renderTable } from "./table.ts";

// ── Single-document detail card ──────────────────────────────────────────────

export interface StatusDetailContext {
  result: StatusResult;
  style: Style;
  mode: OutputMode;
}

export function renderStatusDetail(ctx: StatusDetailContext): string {
  const { result, style, mode } = ctx;
  const out: string[] = [];

  if (!result.found) {
    out.push(
      `${badge("neutral", mode.unicode, style)} ${style.bold(result.doc)} ${style.dim("— not tracked (no claims recorded)")}`,
    );
    return `${out.join("\n")}\n`;
  }

  const worst: Severity = result.suspect.reduce<Severity>((acc, s) => {
    const sev = severityOfStatus(s.status);
    return rankSeverity(sev) < rankSeverity(acc) ? sev : acc;
  }, "clean");
  const headSym = badge(result.current ? "clean" : worst, mode.unicode, style);
  const state = result.current ? style.green("current") : style.red("suspect");
  const lc =
    result.lifecycle && result.lifecycle !== "active"
      ? style.dim(` · ${result.lifecycle}`)
      : "";
  out.push(`${headSym} ${style.bold(result.doc)}   ${state}${lc}`);
  out.push("");

  if (result.suspect.length === 0) {
    const n = result.verdicts.length;
    out.push(style.dim(`  ${n} claim${n === 1 ? "" : "s"}, all current.`));
  } else {
    for (const s of result.suspect) {
      const sev = severityOfStatus(s.status);
      const sym = severityColor(sev, style)(severitySymbol(sev, mode.unicode));
      const status = severityColor(sev, style)(s.status);
      out.push(`  ${sym} ${status}   ${style.cyan(s.propositionId)}`);
    }
  }
  return `${out.join("\n")}\n`;
}

// ── Repo-wide overview table ─────────────────────────────────────────────────

export interface OverviewContext {
  report: CheckReport;
  assertions: Assertion[];
  style: Style;
  mode: OutputMode;
}

function rankSeverity(s: Severity): number {
  return s === "gating" ? 0 : s === "warn" ? 1 : s === "neutral" ? 2 : 3;
}

export function renderOverview(ctx: OverviewContext): string {
  const { report, assertions, style, mode } = ctx;

  // Per-document rollups: verdict counts, owner(s), and verified ref.
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
    for (const v of verdicts) {
      const enf = assertById.get(v.assertionId)?.enforcement ?? "suggested";
      if (v.gates) gating += 1;
      else if (isWarnVerdict(v, enf)) warn += 1;
    }
    const clean = verdicts.length - gating - warn;

    const lcNeutral = doc.lifecycle !== "active";
    const worst: Severity =
      gating > 0
        ? "gating"
        : warn > 0
          ? "warn"
          : lcNeutral
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
  out.push(style.bold("hibi status"));
  out.push("");

  if (rows.length === 0) {
    out.push(
      style.dim("No documents tracked. Run `hibi record` to add a claim."),
    );
    return `${out.join("\n")}\n`;
  }

  const table = renderTable(
    [
      { header: "" },
      { header: "Document", max: 48 },
      { header: "Claims" },
      { header: "Owner", max: 18 },
      { header: "Verified" },
      { header: "Lifecycle" },
    ],
    rows.map((r) => r.cells),
    { unicode: mode.unicode, indent: "  " },
  );
  out.push(...table);
  out.push("");

  const { gating, warning } = report.summary;
  const exit =
    report.exitCode === 0
      ? style.green("exit 0")
      : style.bold(`exit ${report.exitCode}`);
  out.push(
    `${style.dim("Tracking")} ${rows.length} document${rows.length === 1 ? "" : "s"}${style.dim(",")} ${gating > 0 ? style.red(`${gating} gating`) : `${gating} gating`}${style.dim(",")} ${warning > 0 ? style.yellow(`${warning} warning`) : `${warning} warning`}${style.dim(".")}  ${exit}`,
  );
  return `${out.join("\n")}\n`;
}
