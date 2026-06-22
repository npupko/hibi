import { describe, expect, test } from "bun:test";
import { resolveMode } from "../src/cli/render/mode.ts";

/**
 * The mode resolver is the contract that keeps the machine flow byte-identical:
 * a piped (non-TTY) run with no flags must resolve to compact `json`. The matrix
 * below pins flags × isTTY × NO_COLOR/FORCE_COLOR/--color → kind + color.
 */
describe("resolveMode — view kind", () => {
  test("default + non-TTY → compact json (the machine contract)", () => {
    expect(resolveMode({}, { isTTY: false }).kind).toBe("json");
  });
  test("default + TTY → rich", () => {
    expect(resolveMode({}, { isTTY: true }).kind).toBe("rich");
  });
  test("--json forces compact json even on a TTY", () => {
    expect(resolveMode({ json: true }, { isTTY: true }).kind).toBe("json");
  });
  test("--json --pretty → indented json", () => {
    expect(
      resolveMode({ json: true, pretty: true }, { isTTY: true }).kind,
    ).toBe("json-pretty");
  });
  test("--pretty forces rich even when piped", () => {
    expect(resolveMode({ pretty: true }, { isTTY: false }).kind).toBe("rich");
  });
  test("--compact → compact human view even when piped", () => {
    expect(resolveMode({ compact: true }, { isTTY: false }).kind).toBe(
      "compact",
    );
  });
  test("--json beats --compact (machine wins)", () => {
    expect(resolveMode({ json: true, compact: true }, {}).kind).toBe("json");
  });
});

describe("resolveMode — color", () => {
  test("json never carries color", () => {
    expect(resolveMode({ json: true }, { isTTY: true }).color).toBe(false);
  });
  test("rich on a TTY is colored by default", () => {
    expect(resolveMode({}, { isTTY: true }).color).toBe(true);
  });
  test("rich piped is uncolored by default", () => {
    expect(resolveMode({ pretty: true }, { isTTY: false }).color).toBe(false);
  });
  test("--color always wins over a non-TTY", () => {
    expect(
      resolveMode({ pretty: true, color: "always" }, { isTTY: false }).color,
    ).toBe(true);
  });
  test("--color never wins over a TTY", () => {
    expect(resolveMode({ color: "never" }, { isTTY: true }).color).toBe(false);
  });
  test("--color never wins over NO_COLOR/FORCE_COLOR and TTY", () => {
    expect(
      resolveMode(
        { color: "never" },
        { isTTY: true, env: { FORCE_COLOR: "1" } },
      ).color,
    ).toBe(false);
  });
  test("NO_COLOR disables color on a TTY", () => {
    expect(resolveMode({}, { isTTY: true, env: { NO_COLOR: "1" } }).color).toBe(
      false,
    );
  });
  test("FORCE_COLOR enables color when piped", () => {
    expect(
      resolveMode({ pretty: true }, { isTTY: false, env: { FORCE_COLOR: "1" } })
        .color,
    ).toBe(true);
  });
  test("NO_COLOR beats FORCE_COLOR when both set", () => {
    expect(
      resolveMode(
        { pretty: true },
        { isTTY: true, env: { NO_COLOR: "1", FORCE_COLOR: "1" } },
      ).color,
    ).toBe(false);
  });
});

describe("resolveMode — unicode", () => {
  test("default is unicode", () => {
    expect(resolveMode({}, { isTTY: true }).unicode).toBe(true);
  });
  test("--simple forces ASCII", () => {
    expect(resolveMode({ simple: true }, { isTTY: true }).unicode).toBe(false);
  });
  test("a UTF-8 locale keeps unicode", () => {
    expect(resolveMode({}, { env: { LANG: "en_US.UTF-8" } }).unicode).toBe(
      true,
    );
  });
  test("a non-UTF locale falls back to ASCII", () => {
    expect(resolveMode({}, { env: { LANG: "C" } }).unicode).toBe(false);
  });
});
