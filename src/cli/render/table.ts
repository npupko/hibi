/**
 * A minimal width-aware column layout — hand-rolled, no table dependency. Widths
 * fit the content (header + cells), capped per-column with `…`/`...` truncation,
 * and ANSI codes in a cell are ignored when measuring so styled text aligns.
 */

import { visibleWidth } from "./style.ts";

export interface Column {
  header: string;
  /** Cap the rendered width; longer cells are truncated with an ellipsis. */
  max?: number;
  align?: "left" | "right";
}

/** Truncate to a visible width, appending `…`/`...`. Assumes plain (unstyled) text. */
function truncate(s: string, width: number, unicode: boolean): string {
  if (s.length <= width) return s;
  const ell = unicode ? "…" : "...";
  if (width <= ell.length) return s.slice(0, width);
  return s.slice(0, width - ell.length) + ell;
}

/** Pad a (possibly styled) cell to `width`, accounting for invisible SGR codes. */
function pad(cell: string, width: number, align: "left" | "right"): string {
  const gap = width - visibleWidth(cell);
  if (gap <= 0) return cell;
  const spaces = " ".repeat(gap);
  return align === "right" ? spaces + cell : cell + spaces;
}

/**
 * Render rows under a header into aligned lines (header, separator, rows). Cells
 * may contain ANSI styling; truncation is applied only to the unstyled `max` cap
 * so callers should style *after* the table when a hard cap matters. In practice
 * the columns we truncate (paths) are passed unstyled.
 */
export function renderTable(
  columns: Column[],
  rows: string[][],
  opts: { unicode?: boolean; indent?: string } = {},
): string[] {
  const unicode = opts.unicode ?? true;
  const indent = opts.indent ?? "";

  // Column width = widest visible cell (after per-column truncation), header included.
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
        // Only truncate plain cells; a styled cell's cap is the caller's concern.
        if (visibleWidth(cell) > cap && cell === stripPlain(cell)) {
          cell = truncate(cell, cap, unicode);
        }
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

/** True when `s` carries no SGR codes (so truncation is safe to apply). */
function stripPlain(s: string): string {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: detecting ANSI SGR codes.
  return s.replace(/\x1b\[[0-9;]*m/g, "");
}
