import { describe, expect, test } from "bun:test";
import { makeStyle, visibleWidth } from "../src/cli/render/style.ts";
import { badge, severitySymbol } from "../src/cli/render/symbols.ts";
import { renderTable } from "../src/cli/render/table.ts";

describe("style", () => {
  test("color off is a no-op (no SGR codes)", () => {
    const s = makeStyle(false);
    expect(s.red("x")).toBe("x");
    expect(s.bold("x")).toBe("x");
  });
  test("color on wraps in SGR codes", () => {
    const s = makeStyle(true);
    expect(s.red("x")).toBe("\x1b[31mx\x1b[39m");
  });
  test("visibleWidth ignores SGR codes", () => {
    expect(visibleWidth(makeStyle(true).red("hello"))).toBe(5);
    expect(visibleWidth("hello")).toBe(5);
  });
});

describe("symbols — ASCII fallback", () => {
  test("unicode symbols", () => {
    expect(severitySymbol("gating", true)).toBe("✖");
    expect(severitySymbol("warn", true)).toBe("⚠");
    expect(severitySymbol("clean", true)).toBe("✓");
    expect(severitySymbol("neutral", true)).toBe("—");
  });
  test("ASCII symbols when unicode is off", () => {
    expect(severitySymbol("gating", false)).toBe("x");
    expect(severitySymbol("warn", false)).toBe("!");
    expect(severitySymbol("clean", false)).toBe("v");
    expect(severitySymbol("neutral", false)).toBe("-");
  });
  test("badge pairs symbol with color (never color-alone)", () => {
    const s = makeStyle(true);
    expect(badge("gating", true, s)).toBe("\x1b[31m✖\x1b[39m");
  });
});

describe("table", () => {
  test("aligns columns to the widest cell and renders a separator", () => {
    const lines = renderTable(
      [{ header: "Doc" }, { header: "N" }],
      [
        ["README.md", "3"],
        ["a.md", "10"],
      ],
      { unicode: true },
    );
    expect(lines).toHaveLength(4); // header + sep + 2 rows
    // The short doc is padded to the width of "README.md" (4 → 9 = +5 spaces).
    expect(lines[3]).toMatch(/^a\.md {5}/);
    // The separator spans the first column's full width.
    expect(lines[1]).toMatch(/^─{9}/);
  });

  test("truncates a plain cell past its max with an ellipsis", () => {
    const lines = renderTable(
      [{ header: "Document", max: 8 }],
      [["a-very-long-document-name.md"]],
      { unicode: true },
    );
    expect(lines[2]).toBe("a-very-…");
  });

  test("ASCII separator + ellipsis when unicode is off", () => {
    const lines = renderTable(
      [{ header: "Document", max: 8 }],
      [["a-very-long-document-name.md"]],
      { unicode: false },
    );
    expect(lines[1]).not.toContain("─");
    expect(lines[1]).toContain("-");
    expect(lines[2]).toBe("a-ver..."); // 5 chars + "..." = the 8-wide cap
  });

  test("a styled cell is padded by its visible width, not its byte length", () => {
    const s = makeStyle(true);
    const lines = renderTable(
      [{ header: "S" }, { header: "T" }],
      [
        [s.red("x"), "A"],
        ["xx", "B"],
      ],
      { unicode: true },
    );
    // Col 1 is padded to width 2 ("xx") even though the styled cell carries SGR
    // codes, so both rows share the same visible width and col 2 aligns.
    expect(visibleWidth(lines[2] ?? "")).toBe(visibleWidth(lines[3] ?? ""));
    expect(visibleWidth(lines[2] ?? "")).toBe(5);
  });
});
