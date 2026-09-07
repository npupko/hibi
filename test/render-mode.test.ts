import { describe, expect, test } from "bun:test";
import { resolveMode } from "../src/cli/render/mode.ts";

/**
 * The mode resolver keeps the machine flow byte-identical: a piped (non-TTY)
 * run with no flags resolves to compact `json`. The matrix below pins
 * flags x isTTY x NO_COLOR/FORCE_COLOR/--color to kind + color.
 */
describe("resolveMode: view kind", () => {
  test("default + non-TTY resolves to compact json (the machine contract)", () => {
    expect(resolveMode({}, { isTTY: false }).kind).toBe("json");
  });
  test("default + TTY resolves to rich", () => {
    expect(resolveMode({}, { isTTY: true }).kind).toBe("rich");
  });
  test("--json forces compact json even on a TTY", () => {
    expect(resolveMode({ json: true }, { isTTY: true }).kind).toBe("json");
  });
  test("--format json-pretty gives indented json", () => {
    expect(resolveMode({ format: "json-pretty" }, { isTTY: true }).kind).toBe(
      "json-pretty",
    );
  });
  test("--format human forces rich even when piped", () => {
    expect(resolveMode({ format: "human" }, { isTTY: false }).kind).toBe(
      "rich",
    );
  });
  test("--format compact gives the compact human view even when piped", () => {
    expect(resolveMode({ format: "compact" }, { isTTY: false }).kind).toBe(
      "compact",
    );
  });
  test("--format beats --json", () => {
    expect(resolveMode({ json: true, format: "compact" }, {}).kind).toBe(
      "compact",
    );
  });
  test("an unknown --format falls back to the TTY default", () => {
    expect(resolveMode({ format: "bogus" }, { isTTY: false }).kind).toBe(
      "json",
    );
  });
});

describe("resolveMode: color", () => {
  test("json never carries color", () => {
    expect(resolveMode({ json: true }, { isTTY: true }).color).toBe(false);
    expect(
      resolveMode({ format: "json-pretty", color: "always" }, { isTTY: true })
        .color,
    ).toBe(false);
  });
  test("rich on a TTY is colored by default", () => {
    expect(resolveMode({}, { isTTY: true }).color).toBe(true);
  });
  test("rich piped is uncolored by default", () => {
    expect(resolveMode({ format: "human" }, { isTTY: false }).color).toBe(
      false,
    );
  });
  test("--color always wins over a non-TTY", () => {
    expect(
      resolveMode({ format: "human", color: "always" }, { isTTY: false }).color,
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
      resolveMode(
        { format: "human" },
        { isTTY: false, env: { FORCE_COLOR: "1" } },
      ).color,
    ).toBe(true);
  });
  test("NO_COLOR beats FORCE_COLOR when both set", () => {
    expect(
      resolveMode(
        { format: "human" },
        { isTTY: true, env: { NO_COLOR: "1", FORCE_COLOR: "1" } },
      ).color,
    ).toBe(false);
  });
});

describe("resolveMode: unicode", () => {
  test("default is unicode", () => {
    expect(resolveMode({}, { isTTY: true }).unicode).toBe(true);
  });
  test("HIBI_ASCII=1 forces ASCII", () => {
    expect(
      resolveMode({}, { isTTY: true, env: { HIBI_ASCII: "1" } }).unicode,
    ).toBe(false);
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

describe("resolveMode: explain and hints", () => {
  test("explain is off unless the flag is set", () => {
    expect(resolveMode({}, {}).explain).toBe(false);
    expect(resolveMode({ explain: true }, {}).explain).toBe(true);
  });
  test("hints are on by default, off via --no-hints or HIBI_ADVICE=0", () => {
    expect(resolveMode({}, {}).hints).toBe(true);
    expect(resolveMode({ noHints: true }, {}).hints).toBe(false);
    expect(resolveMode({}, { env: { HIBI_ADVICE: "0" } }).hints).toBe(false);
  });
});
