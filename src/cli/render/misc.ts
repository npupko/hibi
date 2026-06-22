/**
 * Concise human confirmations for the write/utility verbs (§9). Each is a single
 * line (or a short grouped list for `query`) that restates what changed — the
 * machine still gets the full JSON via `--json`. `schema` is intentionally absent:
 * it *is* machine output and stays JSON in every mode.
 */

import type {
  ArchiveResult,
  Assertion,
  ListResult,
  ListSeverity,
  QueryHit,
  ReanchorResult,
  RecordResult,
  RetireResult,
  SupersedeResult,
} from "../../index.ts";
import type { OutputMode } from "./mode.ts";
import type { Style } from "./style.ts";
import { badge, type Severity } from "./symbols.ts";
import { renderTable } from "./table.ts";

/** The green check lead, ASCII `+` when unicode is off. */
function ok(style: Style, mode: OutputMode): string {
  return style.green(mode.unicode ? "✓" : "+");
}

/** The doc ↔ code arrow, ASCII `<->` fallback. */
function arrow(mode: OutputMode): string {
  return mode.unicode ? "↔" : "<->";
}

/** The first code-side file an assertion pins, for the confirmation line. */
function codeFile(a: Assertion): string | undefined {
  return a.anchor.code[0]?.file;
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
  trust: string,
  style: Style,
  mode: OutputMode,
): string {
  const a = result.assertion;
  const doc = result.document.path;
  const code = codeFile(a);
  const sides = code ? `${doc} ${arrow(mode)} ${code}` : doc;
  const deduped = result.dedupedProposition ? style.dim(" (deduped)") : "";
  return `${ok(style, mode)} recorded  ${style.cyan(a.id)}   ${sides}  ${style.dim(`(${trust}, ${a.enforcement})`)}${deduped}\n`;
}

export function renderReanchor(
  result: ReanchorResult,
  style: Style,
  mode: OutputMode,
): string {
  return `${ok(style, mode)} reanchored  ${style.cyan(result.assertion.id)}   ${style.dim(`doc:${result.doc}  code:${result.code}`)}\n`;
}

export function renderSuggest(
  doc: string,
  created: RecordResult[],
  style: Style,
  mode: OutputMode,
): string {
  const n = created.length;
  const head = `${ok(style, mode)} suggested ${n} claim${n === 1 ? "" : "s"} from ${style.bold(doc)}`;
  if (n === 0) return `${head}\n`;
  const lines = created.map(
    (r) =>
      `  ${style.cyan(r.assertion.id)}  ${style.dim(`"${oneLine(r.proposition.textCache)}"`)}`,
  );
  return `${head}\n${lines.join("\n")}\n`;
}

export function renderSupersede(
  result: SupersedeResult,
  type: string,
  style: Style,
  mode: OutputMode,
): string {
  return `${ok(style, mode)} ${style.bold(result.newDoc.path)} ${type} ${style.bold(result.oldDoc.path)}  ${style.dim(`(${result.oldDoc.path} → ${result.oldDoc.lifecycle})`)}\n`;
}

export function renderRetract(
  doc: { path: string; lifecycle: string },
  style: Style,
  mode: OutputMode,
): string {
  return `${ok(style, mode)} retracted  ${style.bold(doc.path)}  ${style.dim(`(${doc.lifecycle})`)}\n`;
}

export function renderArchive(
  result: ArchiveResult,
  style: Style,
  mode: OutputMode,
): string {
  const succ = result.successor
    ? style.dim(`  → successor ${result.successor}`)
    : "";
  return `${ok(style, mode)} archived  ${style.bold(result.document.path)}${succ}\n`;
}

export function renderQuery(
  path: string,
  hits: QueryHit[],
  style: Style,
  _mode: OutputMode,
): string {
  const head = `${style.bold(`${hits.length} claim${hits.length === 1 ? "" : "s"}`)} ${style.dim(`covering ${path}`)}`;
  if (hits.length === 0) return `${head}\n`;
  const lines = hits.map((h) => {
    const coarse = h.coarse ? style.dim(" (coarse)") : "";
    const text = h.proposition
      ? `  ${style.dim(`"${oneLine(h.proposition.textCache)}"`)}`
      : "";
    return `  ${style.cyan(h.assertion.id)}  ${style.dim(`[${h.side}]`)} ${h.documentPath ?? "?"}${coarse}${text}`;
  });
  return `${head}\n${lines.join("\n")}\n`;
}

export function renderRetire(
  result: RetireResult,
  style: Style,
  mode: OutputMode,
): string {
  const note = result.alreadyRetired ? style.dim(" (already retired)") : "";
  return `${ok(style, mode)} retired  ${style.cyan(result.assertion.id)}${note}\n`;
}

/** Map a list row's severity onto the four-bucket badge vocabulary. */
function listSeverity(s: ListSeverity): Severity {
  return s === "warning" ? "warn" : s;
}

export function renderList(
  result: ListResult,
  style: Style,
  mode: OutputMode,
): string {
  const out: string[] = [];
  out.push(`${style.bold("hibi list")} ${style.dim(`(${result.state})`)}`);
  out.push("");
  if (result.claims.length === 0) {
    out.push(style.dim("No claims match."));
    return `${out.join("\n")}\n`;
  }
  const rows = result.claims.map((r) => [
    badge(listSeverity(r.severity), mode.unicode, style),
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
  const n = result.count;
  out.push(style.dim(`${n} claim${n === 1 ? "" : "s"}.`));
  return `${out.join("\n")}\n`;
}

export function renderVersion(version: string, style: Style): string {
  return `${style.bold("hibi")} ${version}\n`;
}

/** Local copy of the helpers' one-liner (kept tiny to avoid a cross-import cycle). */
function oneLine(s: string, max = 64): string {
  const flat = s.replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
}
