/**
 * Value extraction for the numeric rule (SPEC 8.5). The judge is not asked to
 * compare quantities; code extracts numbers, money, versions, and dates from
 * both sentences and compares them.
 *
 * Extraction is a fixed table of regular expressions, applied in table order.
 * A match that overlaps text an earlier row already took is skipped, so `$29`
 * is read once as money and not again as the number `29`, and `$5m` is five
 * million dollars, never five minutes. There is no language processing: a
 * value's unit is what sits next to it (a currency sign or code, `%`, or a
 * duration word), never the noun it counts.
 *
 * | rule | kind | matches | normalized | unit |
 * |---|---|---|---|---|
 * | `iso-date` | date | `2026-10-22`, also the date part of a timestamp | the date | none |
 * | `month-day-year` | date | `October 22, 2026`, `Oct 22 2026` | `2026-10-22` | none |
 * | `v-version` | version | `v1.2.3`, `v4`, `v2.0.0-beta.1` | without `v` | none |
 * | `dotted-version` | version | three or more dotted parts, optional pre-release | as written | none |
 * | `keyword-version` | version | `version 1.2`, `release 3.10`, `ver 2.1` | the number | none |
 * | `currency-sign` | money | `$29`, `€ 5`, `£1,299.00`, `$5m`, `$2.5k`, `$1bn` | the number, times 1e3 / 1e6 / 1e9 for `k` / `m` / `b` or `bn` | `USD`, `EUR`, `GBP` |
 * | `currency-code-after` | money | `29 USD` | the number | the code |
 * | `currency-code-before` | money | `USD 29` | the number | the code |
 * | `percent` | number | `5%`, `12.5 %` | the number | `%` |
 * | `duration-word` | number | `5 days`, `30 seconds`, `1 hour` | the number | singular unit word |
 * | `duration-short` | number | `90d`, `500ms`, `12h` (unit touches the number) | the number | unit word |
 * | `number` | number | `29`, `1,000`, `0.75` | the number | none |
 *
 * Comparison (`compareValues`): versions are compared after dropping a
 * leading `v` and trailing `.0` parts, so `v1.2` equals `1.2.0`. On
 * `supports` the unitless `number` group is left out; only money,
 * unit-bearing numbers, versions, and dates can refute. On `conflict` every
 * group counts.
 */

export type ValueKind = "number" | "version" | "date" | "money";

export interface Value {
  kind: ValueKind;
  /** The text as it appears in the sentence. */
  raw: string;
  /** Canonical form: `1,299.50` is `1299.5`, `v1.2.3` is `1.2.3`, `Oct 5, 2026` is `2026-10-05`. */
  normalized: string;
  /** Currency code for money; `%` or a duration unit (`day`, `hour`, ...) for numbers. */
  unit?: string;
}

export interface ValueRule {
  /** Stable row name, used by tests and in comments. */
  name: string;
  kind: ValueKind;
  /** Global, unicode. Must not match the empty string. */
  pattern: RegExp;
  /** Builds the value from a match, or rejects it with `null` (for example month 13). */
  read(m: RegExpExecArray): Omit<Value, "kind"> | null;
}

/** A number with optional thousands separators and decimals: `29`, `1,299`, `0.5`. */
const NUM = String.raw`\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?`;
/** No letter, digit, underscore, or dot directly before a value. */
const START = String.raw`(?<![\p{L}\p{N}_.])`;
/** No letter, digit, or underscore directly after, and no `.<digit>` continuation. */
const END = String.raw`(?![\p{L}\p{N}_]|\.\p{N})`;

const CURRENCY_SIGNS: Record<string, string> = {
  $: "USD",
  "€": "EUR",
  "£": "GBP",
};
const CURRENCY_CODES = ["USD", "EUR", "GBP", "JPY", "CAD", "AUD", "CHF"];

/** Duration words, singular and plural, with a space allowed before them. */
const DURATION_WORDS: Record<string, string> = {
  millisecond: "millisecond",
  milliseconds: "millisecond",
  second: "second",
  seconds: "second",
  sec: "second",
  secs: "second",
  minute: "minute",
  minutes: "minute",
  min: "minute",
  mins: "minute",
  hour: "hour",
  hours: "hour",
  hr: "hour",
  hrs: "hour",
  day: "day",
  days: "day",
  week: "week",
  weeks: "week",
  month: "month",
  months: "month",
  year: "year",
  years: "year",
};

/** Short duration units, read only when written directly after the number: `90d`, `500ms`. */
const DURATION_SHORT: Record<string, string> = {
  ms: "millisecond",
  s: "second",
  m: "minute",
  h: "hour",
  d: "day",
  w: "week",
  y: "year",
};

const MONTHS: Record<string, number> = {
  jan: 1,
  feb: 2,
  mar: 3,
  apr: 4,
  may: 5,
  jun: 6,
  jul: 7,
  aug: 8,
  sep: 9,
  oct: 10,
  nov: 11,
  dec: 12,
};
const MONTH_NAME = `Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|June?|July?|Aug(?:ust)?|Sept?(?:ember)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?`;

/** `1,299.50` to `1299.5`; `007` to `7`; `29.00` to `29`. */
export function normalizeNumber(raw: string): string {
  const [int = "", frac] = raw.replaceAll(",", "").split(".");
  const i = int.replace(/^0+(?=\d)/, "");
  const f = (frac ?? "").replace(/0+$/, "");
  return f === "" ? i : `${i}.${f}`;
}

/** Multiply a normalized decimal string by 10^`zeros` without float error: `2.5`, 3 is `2500`. */
function scaleDecimal(normalized: string, zeros: number): string {
  const [int = "", frac = ""] = normalized.split(".");
  const digits = int + frac.padEnd(zeros, "0");
  const point = int.length + zeros;
  return normalizeNumber(`${digits.slice(0, point)}.${digits.slice(point)}`);
}

/** Magnitude suffixes after a currency amount: `$5m`, `$2.5k`, `$1bn`. */
const MAGNITUDE: Record<string, number> = { k: 3, m: 6, b: 9, bn: 9 };

function isoDate(y: number, m: number, d: number): string | null {
  if (m < 1 || m > 12 || d < 1 || d > 31) return null;
  return `${String(y).padStart(4, "0")}-${String(m).padStart(2, "0")}-${String(d).padStart(2, "0")}`;
}

function group(m: RegExpExecArray, i: number): string {
  return m[i] ?? "";
}

const re = (source: string, flags = "gu") => new RegExp(source, flags);

/**
 * The extraction table. Order matters: earlier rows claim their text first.
 * Each row has a test in test/values.test.ts.
 */
export const VALUE_RULES: readonly ValueRule[] = [
  {
    // 2026-10-22, also the date part of 2026-10-22T09:12:00Z
    name: "iso-date",
    kind: "date",
    pattern: re(
      String.raw`(?<![\p{N}-])(\d{4})-(\d{2})-(\d{2})(?:T[\d:.]+(?:Z|[+-]\d{2}:?\d{2})?)?(?![\p{N}])`,
    ),
    read: (m) => {
      const normalized = isoDate(
        Number(group(m, 1)),
        Number(group(m, 2)),
        Number(group(m, 3)),
      );
      return normalized === null ? null : { raw: m[0], normalized };
    },
  },
  {
    // October 22, 2026 / Oct 22 2026
    name: "month-day-year",
    kind: "date",
    pattern: re(
      String.raw`\b(${MONTH_NAME})\.?\s+(\d{1,2}),?\s+(\d{4})(?!\p{N})`,
      "giu",
    ),
    read: (m) => {
      const month = MONTHS[group(m, 1).slice(0, 3).toLowerCase()] ?? 0;
      const normalized = isoDate(
        Number(group(m, 3)),
        month,
        Number(group(m, 2)),
      );
      return normalized === null ? null : { raw: m[0], normalized };
    },
  },
  {
    // v1.2.3, v4, v2.0.0-beta.1
    name: "v-version",
    kind: "version",
    pattern: re(
      String.raw`${START}v(\d+(?:\.\d+)*(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?)${END}`,
    ),
    read: (m) => ({ raw: m[0], normalized: group(m, 1) }),
  },
  {
    // 1.2.3, 10.0.1.4, 1.2.3-rc.1
    name: "dotted-version",
    kind: "version",
    pattern: re(
      String.raw`${START}(\d+\.\d+\.\d+(?:\.\d+)*(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?)${END}`,
    ),
    read: (m) => ({ raw: group(m, 1), normalized: group(m, 1) }),
  },
  {
    // version 1.2, release 3.10: a two-part number is a version only after one of these words
    name: "keyword-version",
    kind: "version",
    pattern: re(
      String.raw`\b(?:version|release|ver)\s+(\d+\.\d+)${END}`,
      "giu",
    ),
    read: (m) => ({ raw: group(m, 1), normalized: group(m, 1) }),
  },
  {
    // $29, € 5, £1,299.00, and with a magnitude: $5m, $2.5k, $1bn (never 5 minutes)
    name: "currency-sign",
    kind: "money",
    pattern: re(String.raw`([$€£])\s?(${NUM})(bn|[kmb])?${END}`, "giu"),
    read: (m) => {
      const n = normalizeNumber(group(m, 2));
      const zeros = MAGNITUDE[group(m, 3).toLowerCase()] ?? 0;
      return {
        raw: m[0],
        normalized: zeros === 0 ? n : scaleDecimal(n, zeros),
        unit: CURRENCY_SIGNS[group(m, 1)] ?? group(m, 1),
      };
    },
  },
  {
    // 29 USD, 5 EUR
    name: "currency-code-after",
    kind: "money",
    pattern: re(String.raw`${START}(${NUM})\s?(${CURRENCY_CODES.join("|")})\b`),
    read: (m) => ({
      raw: m[0],
      normalized: normalizeNumber(group(m, 1)),
      unit: group(m, 2),
    }),
  },
  {
    // USD 29
    name: "currency-code-before",
    kind: "money",
    pattern: re(String.raw`\b(${CURRENCY_CODES.join("|")})\s?(${NUM})${END}`),
    read: (m) => ({
      raw: m[0],
      normalized: normalizeNumber(group(m, 2)),
      unit: group(m, 1),
    }),
  },
  {
    // 5%, 12.5 %
    name: "percent",
    kind: "number",
    pattern: re(String.raw`${START}(${NUM})\s?%`),
    read: (m) => ({
      raw: m[0],
      normalized: normalizeNumber(group(m, 1)),
      unit: "%",
    }),
  },
  {
    // 5 days, 1 hour, 30 seconds
    name: "duration-word",
    kind: "number",
    pattern: re(
      String.raw`${START}(${NUM})\s?(${Object.keys(DURATION_WORDS).join("|")})\b`,
      "giu",
    ),
    read: (m) => ({
      raw: m[0],
      normalized: normalizeNumber(group(m, 1)),
      unit: DURATION_WORDS[group(m, 2).toLowerCase()],
    }),
  },
  {
    // 90d, 500ms, 12h: the unit letter must touch the number
    name: "duration-short",
    kind: "number",
    pattern: re(String.raw`${START}(${NUM})(ms|s|m|h|d|w|y)(?![\p{L}\p{N}_])`),
    read: (m) => ({
      raw: m[0],
      normalized: normalizeNumber(group(m, 1)),
      unit: DURATION_SHORT[group(m, 2)],
    }),
  },
  {
    // 29, 1,000, 0.75
    name: "number",
    kind: "number",
    pattern: re(`${START}(${NUM})${END}`),
    read: (m) => ({ raw: m[0], normalized: normalizeNumber(group(m, 1)) }),
  },
];

/** Every value in `text`, in order of position. */
export function extractValues(text: string): Value[] {
  const taken: [number, number][] = [];
  const found: { at: number; value: Value }[] = [];
  const overlaps = (s: number, e: number) =>
    taken.some(([ts, te]) => s < te && ts < e);
  for (const rule of VALUE_RULES) {
    rule.pattern.lastIndex = 0;
    for (const m of text.matchAll(rule.pattern)) {
      const start = m.index;
      const end = start + m[0].length;
      if (overlaps(start, end)) continue;
      const read = rule.read(m);
      if (read === null) continue;
      taken.push([start, end]);
      const value: Value = { kind: rule.kind, ...read };
      if (value.unit === undefined) delete value.unit;
      found.push({ at: start, value });
    }
  }
  return found.sort((a, b) => a.at - b.at).map((f) => f.value);
}

const KIND_ORDER: ValueKind[] = ["money", "number", "version", "date"];
const KIND_LABEL: Record<ValueKind, string> = {
  money: "numbers",
  number: "numbers",
  version: "versions",
  date: "dates",
};

/**
 * The group a value is compared within: its kind, plus its unit when it has
 * one (`number`, `number %`, `number day`, `money USD`). Only values in the
 * same group are compared.
 */
export function valueGroup(v: Value): string {
  return v.unit === undefined ? v.kind : `${v.kind} ${v.unit}`;
}

/**
 * The form two values are compared in. A version drops a leading `v` and
 * trailing `.0` parts of its release part (`v1.2.0` and `1.2` are both
 * `1.2`); every other value is its normalized form.
 */
export function comparable(v: Pick<Value, "kind" | "normalized">): string {
  if (v.kind !== "version") return v.normalized;
  const text = v.normalized.replace(/^v/i, "");
  const dash = text.indexOf("-");
  const release = dash < 0 ? text : text.slice(0, dash);
  const pre = dash < 0 ? "" : text.slice(dash);
  const parts = release.split(".");
  while (parts.length > 1 && /^0+$/.test(parts.at(-1) ?? "")) parts.pop();
  return `${parts.join(".")}${pre}`;
}

/** Per group: each distinct value once, as written (`label`) and as compared (`key`). */
function groups(
  values: Value[],
): Map<string, { label: string; key: string }[]> {
  const out = new Map<string, { label: string; key: string }[]>();
  for (const v of values) {
    const group = valueGroup(v);
    const list = out.get(group) ?? [];
    const key = comparable(v);
    if (!list.some((x) => x.key === key))
      list.push({ label: v.normalized, key });
    out.set(group, list);
  }
  return out;
}

export interface CompareOptions {
  /** Groups left out of the comparison: `supports` passes `["number"]`. */
  exclude?: readonly string[];
}

function kindOf(key: string): ValueKind {
  return key.split(" ")[0] as ValueKind;
}

/**
 * The numeric rule (SPEC 8.5). A deterministic rule that the contradiction
 * pass applies only after the judge has said two sentences are about the same
 * subject (`same_subject` at or above the threshold); on its own it says
 * nothing about whether the sentences are related.
 *
 * Values are grouped by kind and unit. The rule fires when some group appears
 * on both sides and the two sides share no value in it (compared by
 * `comparable`): `$29` vs `$39` differs; `$29 or $49` vs `$29` does not;
 * `5 days` vs `5%` is not compared; `v1.2` vs `1.2.0` does not differ.
 * `opts.exclude` leaves groups out (`supports` excludes the unitless
 * `number` group). The first differing group, in the order money, number, version,
 * date, then unit, names the label: `numbers differ: 29 vs 39`,
 * `versions differ: 1.2.0 vs 1.3.0`, `dates differ: 2026-10-01 vs 2026-11-01`.
 */
export function compareValues(
  a: Value[],
  b: Value[],
  opts: CompareOptions = {},
): { differs: boolean; label?: string } {
  const ga = groups(a);
  const gb = groups(b);
  const excluded = new Set(opts.exclude ?? []);
  const shared = [...ga.keys()]
    .filter((k) => gb.has(k) && !excluded.has(k))
    .sort((x, y) => {
      const kx = KIND_ORDER.indexOf(kindOf(x));
      const ky = KIND_ORDER.indexOf(kindOf(y));
      return kx !== ky ? kx - ky : x < y ? -1 : x > y ? 1 : 0;
    });
  for (const key of shared) {
    const va = ga.get(key) ?? [];
    const vb = gb.get(key) ?? [];
    if (va.some((x) => vb.some((y) => y.key === x.key))) continue;
    const list = (vs: { label: string }[]) => vs.map((v) => v.label).join(", ");
    return {
      differs: true,
      label: `${KIND_LABEL[kindOf(key)]} differ: ${list(va)} vs ${list(vb)}`,
    };
  }
  return { differs: false };
}
