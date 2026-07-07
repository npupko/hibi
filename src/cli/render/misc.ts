/**
 * Concise human confirmations for the write/utility verbs (§9). Each is a single
 * line (or a short grouped list for `query`) that restates what changed — the
 * machine still gets the full JSON via `--json`. `schema` is intentionally absent:
 * it *is* machine output and stays JSON in every mode.
 */

import type {
  ArchiveResult,
  Assertion,
  CoverageResult,
  DoctorReport,
  ListResult,
  ListSeverity,
  QueryHit,
  ReanchorResult,
  RecordResult,
  RelocateResult,
  RetireResult,
  RetractResult,
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
  let out = `${ok(style, mode)} recorded  ${style.cyan(a.id)}   ${sides}  ${style.dim(`(${trust}, ${a.enforcement})`)}${deduped}\n`;
  // A suggested claim is advisory; a duplicate proposition likely wants reanchor.
  if (a.enforcement === "suggested") {
    out += `  ${style.yellow(mode.unicode ? "⚠" : "!")} ${style.dim("suggested — won't gate; pass --enforce to make it gating")}\n`;
  }
  if (result.existingClaims.length > 0) {
    out += `  ${style.dim(`already claimed by ${result.existingClaims.join(", ")} — did you mean reanchor?`)}\n`;
  }
  return out;
}

/** A "preview only" tag for the dry-run renderings. */
function dryTag(style: Style, dryRun: boolean): string {
  return dryRun ? `${style.dim(" (dry-run — nothing written)")}` : "";
}

export function renderReanchor(
  result: ReanchorResult,
  style: Style,
  mode: OutputMode,
  dryRun = false,
): string {
  const verb = dryRun ? "would reanchor" : "reanchored";
  return `${ok(style, mode)} ${verb}  ${style.cyan(result.assertion.id)}   ${style.dim(`doc:${result.doc}  code:${result.code}`)}${dryTag(style, dryRun)}\n`;
}

export function renderCoverage(
  doc: string,
  result: CoverageResult,
  style: Style,
  mode: OutputMode,
): string {
  const { blocks, coveredBlocks, uncoveredBlocks, coverageRatio } =
    result.summary;
  const pct = Math.round(coverageRatio * 100);
  const head = `${ok(style, mode)} ${style.bold(doc)}  ${style.dim(`${coveredBlocks}/${blocks} blocks grounded (${pct}%)`)}`;
  if (uncoveredBlocks === 0) return `${head}\n`;
  // List the uncovered blocks — the audit worklist (ground or prune each).
  // `preview` is already collapsed to one line and capped by the engine; render it
  // verbatim so the terminal and the JSON payload show the identical text. An
  // executable block (```sh/bash/…) is flagged: it can carry a `command:` verifier.
  const lines = result.regions
    .filter((r) => !r.covered)
    .map((r) => {
      const mark = r.executable
        ? style.yellow(mode.unicode ? "⚡" : "$")
        : style.yellow(mode.unicode ? "○" : "o");
      const tag = r.executable ? ` ${style.dim("executable")}` : "";
      return `  ${mark} ${style.dim(`[${r.range.start}-${r.range.end}]`)} ${style.dim(`"${r.preview}"`)}${tag}`;
    });
  return `${head}\n${lines.join("\n")}\n`;
}

// ── Stranded-claim relocate hints (single-sourced — §6 silent-orphan hardening) ──
// Built once here so the JSON `next` envelope (cli/index.ts) and the human
// stranded line (below) emit the byte-identical command, never a drifted copy.

/** The relocate hint for a stranded supersede/amend: old → new document. */
export function supersedeRelocateHint(
  oldPath: string,
  newPath: string,
): string {
  return `hibi relocate --from ${oldPath} --to ${newPath}`;
}

/** The relocate hint for a stranded archive: doc → successor (placeholder if none). */
export function archiveRelocateHint(
  docPath: string,
  successor?: string,
): string {
  return `hibi relocate --from ${docPath} --to ${successor ?? "<newDoc>"}`;
}

/** The relocate hint for a stranded retract: no successor, so offer retire too. */
export function retractRelocateHint(docPath: string): string {
  return `hibi relocate --from ${docPath} --to <newDoc>  # or: hibi retire <id>`;
}

/**
 * The stranded-claims warning line shared by the lifecycle ops: when live claims
 * remain on a document that just left the read path, point at `hibi relocate`
 * rather than letting them silently rot (Tier-1 silent-orphan hardening).
 */
function strandedLine(
  strandedClaims: string[],
  relocateHint: string,
  style: Style,
  mode: OutputMode,
): string {
  if (strandedClaims.length === 0) return "";
  const n = strandedClaims.length;
  const mark = style.yellow(mode.unicode ? "⚠" : "!");
  return (
    `  ${mark} ${n} claim${n === 1 ? "" : "s"} stranded on the old document — ` +
    `${style.dim(relocateHint)}\n`
  );
}

export function renderSupersede(
  result: SupersedeResult,
  type: string,
  style: Style,
  mode: OutputMode,
  dryRun = false,
): string {
  const verb = dryRun ? "would supersede" : type;
  const head = `${ok(style, mode)} ${style.bold(result.newDoc.path)} ${verb} ${style.bold(result.oldDoc.path)}  ${style.dim(`(${result.oldDoc.path} → ${result.oldDoc.lifecycle})`)}${dryTag(style, dryRun)}\n`;
  return (
    head +
    strandedLine(
      result.strandedClaims,
      supersedeRelocateHint(result.oldDoc.path, result.newDoc.path),
      style,
      mode,
    )
  );
}

export function renderRetract(
  result: RetractResult,
  style: Style,
  mode: OutputMode,
  dryRun = false,
): string {
  const doc = result.document;
  const verb = dryRun ? "would retract" : "retracted";
  const head = `${ok(style, mode)} ${verb}  ${style.bold(doc.path)}  ${style.dim(`(${doc.lifecycle})`)}${dryTag(style, dryRun)}\n`;
  return (
    head +
    strandedLine(
      result.strandedClaims,
      retractRelocateHint(doc.path),
      style,
      mode,
    )
  );
}

export function renderArchive(
  result: ArchiveResult,
  style: Style,
  mode: OutputMode,
  dryRun = false,
): string {
  const succ = result.successor
    ? style.dim(`  → successor ${result.successor}`)
    : "";
  const verb = dryRun ? "would archive" : "archived";
  const head = `${ok(style, mode)} ${verb}  ${style.bold(result.document.path)}${succ}${dryTag(style, dryRun)}\n`;
  return (
    head +
    strandedLine(
      result.strandedClaims,
      archiveRelocateHint(result.document.path, result.successor),
      style,
      mode,
    )
  );
}

export function renderRelocate(
  result: RelocateResult,
  style: Style,
  mode: OutputMode,
): string {
  const verb = result.dryRun ? "would relocate" : "relocated";
  const n = result.relocated.length;
  const m = result.misses.length;
  const head = `${ok(style, mode)} ${verb} ${n}, ${m} need${m === 1 ? "s" : ""} manual attention  ${style.dim(`${result.from} → ${result.to}`)}${dryTag(style, result.dryRun)}\n`;
  if (m === 0) return head;
  const lines = result.misses.map(
    (miss) =>
      `  ${style.yellow(mode.unicode ? "⚠" : "!")} ${style.cyan(miss.claimId)}  ${style.dim(miss.reason)}`,
  );
  return `${head}${lines.join("\n")}\n`;
}

export function renderDoctor(
  report: DoctorReport,
  style: Style,
  mode: OutputMode,
): string {
  const out: string[] = [];
  const mark = report.healthy
    ? style.green(mode.unicode ? "✓" : "+")
    : style.yellow(mode.unicode ? "⚠" : "!");
  out.push(
    `${mark} ${style.bold("hibi doctor")} ${style.dim(report.healthy ? "(healthy)" : "(needs attention)")}`,
  );
  out.push("");
  const rows = [
    ["orphaned anchors", report.counts.orphanedAnchors],
    ["suggested, no code", report.counts.suggestedNoCode],
    ["stranded on stale doc", report.counts.staleDocClaims],
    ["duplicate propositions", report.counts.duplicatePropositions],
  ] as const;
  for (const [label, count] of rows) {
    const padded = String(count).padStart(3);
    const c = count === 0 ? style.dim(padded) : style.yellow(padded);
    out.push(`  ${c}  ${label}`);
  }
  // Observability rates (never gate): the tighten-the-gate / kill-switch signals.
  const pct = (r: number) => `${Math.round(r * 100)}%`;
  const rate = (r: number) =>
    r > 0.3 ? style.yellow(pct(r)) : style.dim(pct(r));
  out.push("");
  out.push(
    `  ${style.dim("behavioral flag-rate")} ${rate(report.rates.behavioralFlagRate)} ${style.dim("(>30% → tighten the gate)")}`,
  );
  out.push(
    `  ${style.dim("doc orphaned/moved/changed")} ${rate(report.rates.docOrphanedRate)} / ${style.dim(pct(report.rates.docMovedRate))} / ${style.dim(pct(report.rates.docChangedRate))} ${style.dim("(>30% orphaned → require inline IDs)")}`,
  );
  // The claim ids per non-empty category, so the next command is one copy away.
  const detail: string[] = [];
  for (const o of report.orphanedAnchors)
    detail.push(
      `  ${style.dim(`orphaned ${o.side}:`)} ${style.cyan(o.claimId)} ${style.dim(o.path)}`,
    );
  for (const s of report.suggestedNoCode)
    detail.push(
      `  ${style.dim("suggested-no-code:")} ${style.cyan(s.claimId)} ${style.dim(s.docPath ?? "—")}`,
    );
  for (const s of report.staleDocClaims)
    detail.push(
      `  ${style.dim(`stranded (${s.lifecycle}):`)} ${style.cyan(s.claimId)} ${style.dim(s.docPath ?? "—")}`,
    );
  for (const d of report.duplicatePropositions)
    detail.push(
      `  ${style.dim("duplicate prop:")} ${style.cyan(d.claimIds.join(", "))}`,
    );
  if (detail.length > 0) {
    out.push("");
    out.push(...detail);
  }
  return `${out.join("\n")}\n`;
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
  dryRun = false,
): string {
  const note = result.alreadyRetired ? style.dim(" (already retired)") : "";
  const verb = dryRun && !result.alreadyRetired ? "would retire" : "retired";
  return `${ok(style, mode)} ${verb}  ${style.cyan(result.assertion.id)}${note}${dryTag(style, dryRun)}\n`;
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
