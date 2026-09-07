/**
 * Concise human confirmations for the write and utility verbs. Each is a
 * single line or a short list that restates what changed; the machine gets
 * the full JSON via `--format json`.
 */

import type {
  ArchiveResult,
  CoverageResult,
  ListResult,
  ListSeverity,
  ReanchorResult,
  ReanchorSuggestResult,
  RecordResult,
  RetireResult,
  SupersedeResult,
} from "../../index.ts";
import { oneLine } from "./helpers.ts";
import type { OutputMode } from "./mode.ts";
import type { Style } from "./style.ts";
import { badge, type Severity } from "./symbols.ts";
import { renderTable } from "./table.ts";

function ok(style: Style, mode: OutputMode): string {
  return style.green(mode.unicode ? "✓" : "+");
}

function warn(style: Style, mode: OutputMode): string {
  return style.yellow(mode.unicode ? "⚠" : "!");
}

function arrow(mode: OutputMode): string {
  return mode.unicode ? "↔" : "<->";
}

function dryTag(style: Style, dryRun: boolean): string {
  return dryRun ? `${style.dim(" (dry-run: nothing written)")}` : "";
}

function warningLines(
  warnings: string[],
  style: Style,
  mode: OutputMode,
): string {
  return warnings
    .map((w) => `  ${warn(style, mode)} ${style.dim(w)}\n`)
    .join("");
}

export function renderInit(
  result: { store: string; nonce: string; version: string },
  style: Style,
  mode: OutputMode,
): string {
  return `${ok(style, mode)} initialized claim store  ${style.bold(result.store)}  ${style.dim(`(nonce ${result.nonce}, ${result.version})`)}\n`;
}

export function renderRecord(
  result: RecordResult,
  style: Style,
  mode: OutputMode,
): string {
  const a = result.assertion;
  const doc = result.document.path;
  const code = a.anchor.code[0]?.file;
  const sides = code ? `${doc} ${arrow(mode)} ${code}` : doc;
  const deduped = result.dedupedProposition ? style.dim(" (deduped)") : "";
  const facets = [a.enforcement, a.verified ? "verified" : ""]
    .filter(Boolean)
    .join(", ");
  let out = `${ok(style, mode)} recorded  ${style.cyan(a.id)}   ${sides}  ${style.dim(`(${facets})`)}${deduped}\n`;
  if (a.enforcement === "suggested") {
    out += `  ${warn(style, mode)} ${style.dim("suggested: this claim never gates")}\n`;
  }
  if (result.existingClaims.length > 0) {
    out += `  ${style.dim(`already claimed by ${result.existingClaims.join(", ")}; did you mean reanchor?`)}\n`;
  }
  return out + warningLines(result.warnings, style, mode);
}

export function renderReanchor(
  result: ReanchorResult,
  style: Style,
  mode: OutputMode,
  dryRun = false,
): string {
  const verb = dryRun ? "would reanchor" : "reanchored";
  const lines = [
    `${ok(style, mode)} ${verb}  ${style.cyan(result.assertion.id)}   ${style.dim(`doc:${result.doc}  code:${result.code}`)}${dryTag(style, dryRun)}`,
  ];
  const side = (
    label: string,
    b: { file: string; quote: string },
    a: { file: string; quote: string },
  ) => {
    const same = b.file === a.file && b.quote === a.quote;
    const where =
      b.file === a.file
        ? a.file
        : `${b.file} ${mode.unicode ? "→" : "->"} ${a.file}`;
    lines.push(
      `  ${style.dim(label)} ${where}${same ? style.dim("  (unchanged)") : ""}`,
    );
    if (!same) {
      lines.push(
        `    ${style.dim("before:")} ${style.dim(`"${oneLine(b.quote)}"`)}`,
      );
      lines.push(`    ${style.dim("after: ")} "${oneLine(a.quote)}"`);
    }
  };
  side("doc ", result.before.doc, result.after.doc);
  result.after.code.forEach((a, i) => {
    const b = result.before.code[i] ?? a;
    side("code", b, a);
  });
  return `${lines.join("\n")}\n${warningLines(result.warnings, style, mode)}`;
}

export function renderReanchorSuggest(
  result: ReanchorSuggestResult,
  style: Style,
  mode: OutputMode,
): string {
  const n = result.candidates.length;
  const head = `${ok(style, mode)} ${style.bold("reanchor --suggest")}  ${style.cyan(result.claimId)}  ${style.dim(`${n} candidate${n === 1 ? "" : "s"}`)}`;
  if (n === 0) {
    return `${head}\n  ${style.dim("no candidate locations found; retire the claim, or reanchor with an explicit span")}\n`;
  }
  const lines = result.candidates.map((c) => {
    const sim = `${Math.round(c.similarity * 100)}%`;
    return `  ${style.green(sim.padStart(4))}  ${style.dim(c.side.padEnd(4))} ${style.bold(c.file)} ${style.dim(`[${c.start}-${c.end}]`)}  ${style.dim(`"${oneLine(c.snippet)}"`)}`;
  });
  return `${head}\n${lines.join("\n")}\n`;
}

export function renderCoverage(
  doc: string,
  result: CoverageResult,
  style: Style,
  mode: OutputMode,
): string {
  const { regions, covered, uncovered, coverageRatio } = result.summary;
  const pct = Math.round(coverageRatio * 100);
  const head = `${ok(style, mode)} ${style.bold(doc)}  ${style.dim(`${covered}/${regions} regions backed by a claim (${pct}%)`)}`;
  if (uncovered === 0) return `${head}\n`;
  const lines = result.regions
    .filter((r) => !r.covered)
    .map(
      (r) =>
        `  ${style.yellow(mode.unicode ? "○" : "o")} ${style.dim(`[${r.range.start}-${r.range.end}]`)} ${style.dim(`"${r.preview}"`)}`,
    );
  return `${head}\n${lines.join("\n")}\n`;
}

function strandedLine(
  strandedClaims: string[],
  style: Style,
  mode: OutputMode,
): string {
  if (strandedClaims.length === 0) return "";
  const n = strandedClaims.length;
  return `  ${warn(style, mode)} ${n} live claim${n === 1 ? "" : "s"} still on the old document: ${style.dim(strandedClaims.join(", "))}\n`;
}

export function renderSupersede(
  result: SupersedeResult,
  style: Style,
  mode: OutputMode,
): string {
  const verb = result.dryRun ? "would supersede" : "superseded";
  const n = result.relocated.length;
  const m = result.misses.length;
  let out = `${ok(style, mode)} ${verb}  ${style.bold(result.oldDoc.path)} ${mode.unicode ? "→" : "->"} ${style.bold(result.newDoc.path)}  ${style.dim(`${n} claim${n === 1 ? "" : "s"} relocated, ${m} need${m === 1 ? "s" : ""} attention`)}${dryTag(style, result.dryRun)}\n`;
  for (const miss of result.misses) {
    out += `  ${warn(style, mode)} ${style.cyan(miss.claimId)}  ${style.dim(miss.reason)}\n`;
  }
  return out + strandedLine(result.strandedClaims, style, mode);
}

export function renderArchive(
  result: ArchiveResult,
  style: Style,
  mode: OutputMode,
): string {
  const succ = result.successor
    ? style.dim(`  ${mode.unicode ? "→" : "->"} successor ${result.successor}`)
    : "";
  const verb = result.dryRun ? "would archive" : "archived";
  const head = `${ok(style, mode)} ${verb}  ${style.bold(result.document.path)}${succ}${dryTag(style, result.dryRun)}\n`;
  return head + strandedLine(result.strandedClaims, style, mode);
}

export function renderRetire(
  result: RetireResult,
  style: Style,
  mode: OutputMode,
  dryRun = false,
): string {
  const note = result.alreadyRetired ? style.dim(" (already retired)") : "";
  const verb = dryRun && !result.alreadyRetired ? "would retire" : "retired";
  return `${ok(style, mode)} ${verb}  ${style.cyan(result.assertion.id)}${note}${dryTag(style, dryRun)}\n`;
}

function listSeverity(s: ListSeverity): Severity {
  return s === "warning" ? "warn" : s;
}

export function renderList(
  result: ListResult,
  style: Style,
  mode: OutputMode,
): string {
  const out: string[] = [];
  const scope = result.path ? `${result.state}, ${result.path}` : result.state;
  out.push(`${style.bold("hibi list")} ${style.dim(`(${scope})`)}`);
  out.push("");
  if (result.claims.length === 0) {
    out.push(style.dim("No claims match."));
    return `${out.join("\n")}\n`;
  }
  const rows = result.claims.map((r) => [
    badge(
      r.status === "retired" ? "neutral" : listSeverity(r.severity),
      mode.unicode,
      style,
    ),
    r.status,
    r.claimId,
    r.documentPath ?? "—",
    r.codePath ?? "—",
    r.recommended ?? "—",
  ]);
  out.push(
    ...renderTable(
      [
        { header: "" },
        { header: "Status" },
        { header: "Claim" },
        { header: "Document", max: 32 },
        { header: "Code", max: 32 },
        { header: "Action" },
      ],
      rows,
      { unicode: mode.unicode, indent: "  " },
    ),
  );
  out.push("");
  for (const r of result.claims) {
    out.push(
      `  ${style.cyan(r.claimId)}  ${style.dim(`"${oneLine(r.text)}"`)}`,
    );
  }
  out.push("");
  const n = result.count;
  out.push(style.dim(`${n} claim${n === 1 ? "" : "s"}.`));
  return `${out.join("\n")}\n`;
}

export function renderVersion(version: string, style: Style): string {
  return `${style.bold("hibi")} ${version}\n`;
}
