import { describe, expect, test } from "bun:test";
import {
  compareValues,
  extractValues,
  normalizeNumber,
  VALUE_RULES,
  type Value,
} from "../src/engine/values.ts";

/** One example per row of VALUE_RULES. */
const ROWS: Record<string, { text: string; values: Value[] }> = {
  "iso-date": {
    text: "Released 2026-10-22T09:12:00Z, frozen 2026-11-01.",
    values: [
      { kind: "date", raw: "2026-10-22T09:12:00Z", normalized: "2026-10-22" },
      { kind: "date", raw: "2026-11-01", normalized: "2026-11-01" },
    ],
  },
  "month-day-year": {
    text: "Since October 22, 2026 and Oct 5 2026.",
    values: [
      { kind: "date", raw: "October 22, 2026", normalized: "2026-10-22" },
      { kind: "date", raw: "Oct 5 2026", normalized: "2026-10-05" },
    ],
  },
  "v-version": {
    text: "Store v4 needs v1.2.3 or v2.0.0-beta.1.",
    values: [
      { kind: "version", raw: "v4", normalized: "4" },
      { kind: "version", raw: "v1.2.3", normalized: "1.2.3" },
      { kind: "version", raw: "v2.0.0-beta.1", normalized: "2.0.0-beta.1" },
    ],
  },
  "dotted-version": {
    text: "Requires Node 20.1.0.",
    values: [{ kind: "version", raw: "20.1.0", normalized: "20.1.0" }],
  },
  "keyword-version": {
    text: "Pinned to version 1.2.",
    values: [{ kind: "version", raw: "1.2", normalized: "1.2" }],
  },
  "currency-sign": {
    text: "Pro costs $29/month, € 5 per seat, £1,299.00 a year.",
    values: [
      { kind: "money", raw: "$29", normalized: "29", unit: "USD" },
      { kind: "money", raw: "€ 5", normalized: "5", unit: "EUR" },
      { kind: "money", raw: "£1,299.00", normalized: "1299", unit: "GBP" },
    ],
  },
  "currency-code-after": {
    text: "Pro costs 29 USD.",
    values: [{ kind: "money", raw: "29 USD", normalized: "29", unit: "USD" }],
  },
  "currency-code-before": {
    text: "Pro costs EUR 39.50.",
    values: [
      { kind: "money", raw: "EUR 39.50", normalized: "39.5", unit: "EUR" },
    ],
  },
  percent: {
    text: "Uses 12.5% of the budget, never 100 %.",
    values: [
      { kind: "number", raw: "12.5%", normalized: "12.5", unit: "%" },
      { kind: "number", raw: "100 %", normalized: "100", unit: "%" },
    ],
  },
  "duration-word": {
    text: "Tokens expire after 5 days or 1 Hour.",
    values: [
      { kind: "number", raw: "5 days", normalized: "5", unit: "day" },
      { kind: "number", raw: "1 Hour", normalized: "1", unit: "hour" },
    ],
  },
  "duration-short": {
    text: "Cache lives 90d; requests time out at 500ms.",
    values: [
      { kind: "number", raw: "90d", normalized: "90", unit: "day" },
      { kind: "number", raw: "500ms", normalized: "500", unit: "millisecond" },
    ],
  },
  number: {
    text: "Retries 3 times over 1,000 rows at 0.75 load.",
    values: [
      { kind: "number", raw: "3", normalized: "3" },
      { kind: "number", raw: "1,000", normalized: "1000" },
      { kind: "number", raw: "0.75", normalized: "0.75" },
    ],
  },
};

describe("extractValues: one test per table row", () => {
  test("every row of VALUE_RULES has an example here", () => {
    expect(Object.keys(ROWS).sort()).toEqual(
      VALUE_RULES.map((r) => r.name).sort(),
    );
  });

  for (const [name, row] of Object.entries(ROWS)) {
    test(name, () => {
      expect(extractValues(row.text)).toEqual(row.values);
    });
  }
});

describe("extractValues: what is not a value", () => {
  test("digits inside identifiers, hashes, and hex are skipped", () => {
    expect(extractValues("See clm_ab12, sha256 and 0x1F, 64k, 3rd.")).toEqual(
      [],
    );
  });

  test("a short duration unit counts only when it touches the number", () => {
    expect(extractValues("Wait 90 d.")).toEqual([
      { kind: "number", raw: "90", normalized: "90" },
    ]);
  });

  test("a two-part number without a version word is a number", () => {
    expect(extractValues("Bun 1.2 is required.")).toEqual([
      { kind: "number", raw: "1.2", normalized: "1.2" },
    ]);
  });

  test("an impossible ISO date is not a date", () => {
    expect(extractValues("On 2026-13-01.").map((v) => v.kind)).toEqual([
      "number",
      "number",
      "number",
    ]);
  });

  test("money is read once, not again as a bare number", () => {
    expect(extractValues("$29")).toHaveLength(1);
  });

  test("sentences without values", () => {
    expect(extractValues("The cache lives under .hibi.")).toEqual([]);
  });
});

test("normalizeNumber", () => {
  expect(normalizeNumber("1,299.50")).toBe("1299.5");
  expect(normalizeNumber("29.00")).toBe("29");
  expect(normalizeNumber("007")).toBe("7");
  expect(normalizeNumber("0.5")).toBe("0.5");
});

describe("compareValues", () => {
  const cmp = (a: string, b: string) =>
    compareValues(extractValues(a), extractValues(b));

  test("different prices differ, with the SPEC 8.5 label", () => {
    expect(cmp("Pro costs $29/month.", "Pro costs $39/month.")).toEqual({
      differs: true,
      label: "numbers differ: 29 vs 39",
    });
  });

  test("the same amount written two ways does not differ", () => {
    expect(cmp("Pro costs $29.", "Pro costs 29.00 USD.").differs).toBe(false);
  });

  test("overlapping sets do not differ", () => {
    expect(cmp("Plans cost $29 or $49.", "Pro costs $29.").differs).toBe(false);
  });

  test("different units are not compared", () => {
    expect(cmp("Expires after 5 days.", "Expires after 7 hours.")).toEqual({
      differs: false,
    });
    expect(cmp("Pro costs $29.", "Pro costs 29 EUR.").differs).toBe(false);
  });

  test("money and a bare number are different kinds", () => {
    expect(cmp("Pro costs $29.", "Pro has 30 seats.").differs).toBe(false);
  });

  test("one side without values does not differ", () => {
    expect(cmp("Pro costs $29.", "Pro is the paid plan.")).toEqual({
      differs: false,
    });
  });

  test("versions and dates have their own labels", () => {
    expect(cmp("Needs v1.2.0.", "Needs v1.3.0.")).toEqual({
      differs: true,
      label: "versions differ: 1.2.0 vs 1.3.0",
    });
    expect(cmp("Ships 2026-10-01.", "Ships November 1, 2026.")).toEqual({
      differs: true,
      label: "dates differ: 2026-10-01 vs 2026-11-01",
    });
  });

  test("several values on a side are listed", () => {
    expect(cmp("Retries 3 or 4 times.", "Retries 5 times.")).toEqual({
      differs: true,
      label: "numbers differ: 3, 4 vs 5",
    });
  });
});
