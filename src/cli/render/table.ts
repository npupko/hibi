/**
 * A minimal width-aware column layout. Widths fit the content, capped per
 * column with an ellipsis, and ANSI codes are ignored when measuring.
 */

import { visibleWidth } from "./style.ts";

export interface Column {
  header: string;
  /** Cap the rendered width; longer plain cells are truncated with an ellipsis. */
  max?: number;
  align?: "left" | "right";
}

function truncate(s: string, width: number, unicode: boolean): string {
  if (s.length <= width) return s;
  const ell = unicode ? "…" : "...";
  if (width <= ell.length) return s.slice(0, width);
  return s.slice(0, width - ell.length) + ell;
}

function pad(cell: string, width: number, align: "left" | "right"): string {
  const gap = width - visibleWidth(cell);
  if (gap <= 0) return cell;
  const spaces = " ".repeat(gap);
  return align === "right" ? spaces + cell : cell + spaces;
}

/**
 * Render rows under a header into aligned lines. Truncation applies only to
 * plain (unstyled) cells; style after the table when a hard cap matters.
 */
export function renderTable(
  columns: Column[],
  rows: string[][],
  opts: { unicode?: boolean; indent?: string } = {},
): string[] {
  const unicode = opts.unicode ?? true;
  const indent = opts.indent ?? "";

  const widths = columns.map((col, i) => {
    const cap = col.max ?? Number.POSITIVE_INFINITY;
    let w = Math.min(visibleWidth(col.header), cap);
    for (const row of rows) {
      const cell = row[i] ?? "";
      w = Math.max(w, Math.min(visibleWidth(cell), cap));
    }
    return w;
  });

  const renderRow = (cells: string[]): string =>
    indent +
    columns
      .map((col, i) => {
        const cap = col.max ?? Number.POSITIVE_INFINITY;
        let cell = cells[i] ?? "";
        const plain = visibleWidth(cell) === cell.length;
        if (plain && cell.length > cap) cell = truncate(cell, cap, unicode);
        return pad(cell, widths[i] ?? 0, col.align ?? "left");
      })
      .join("  ")
      .replace(/\s+$/, "");

  const header = renderRow(columns.map((c) => c.header));
  const sep =
    indent +
    columns
      .map((_, i) => "─".repeat(widths[i] ?? 0))
      .join("  ")
      .replace(/\s+$/, "");
  return [
    header,
    unicode ? sep : sep.replace(/─/g, "-"),
    ...rows.map(renderRow),
  ];
}
