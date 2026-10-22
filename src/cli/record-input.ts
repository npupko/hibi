/**
 * The `record` input contract (SPEC 11): one JSON item per claim, from
 * `--from-file` or built from the flags. Every item is validated before
 * anything is written; every problem is one line, and all of them are
 * reported together.
 *
 *   {
 *     "doc": "docs/pricing.md",                       // required
 *     "docQuote": "Pro costs $29/month.",             // exactly one of docQuote, docRange
 *     "docRange": "L12:L12",                          //   lines L<a>:L<b>, or char offsets <a>:<b>
 *     "kind": "code" | "reference" | "attested",      // required
 *     "code": [{ "file": "src/x.ts", "quote": "…", "range": "L1:L3" }],  // kind code: one or more; quote or range
 *     "ref": { "file": "docs/other.md", "quote": "…", "range": "…" },    // kind reference: exactly one; quote or range
 *     "sources": [{ "id": "stripe", "resource": "https://…", "title": "…" }],
 *     "procedure": { "instructions": "…", "steps": [{ "id": "…", "command": "…", "stdin": "…" }] },
 *     "ttl": "P90D",                                  // required on attested
 *     "owner": "human:alice@example.com",
 *     "by": "agent:opus",                             // default: human:<git user.email>
 *     "suggest": false                                // true: never gates
 *   }
 *
 * A `code` or `ref` file may name a sibling repo (`api:src/limits.ts`,
 * SPEC 4.1); the engine refuses it at record time when the alias is unknown
 * or not checked out.
 */

import {
  ACTOR_PATTERN,
  ClaimKind,
  DURATION_PATTERN,
  type Procedure,
  type Source,
} from "../core/model.ts";
import type { EvidenceTarget, RecordCall, RegionSpec } from "../index.ts";

export const RECORD_ITEM_KEYS = [
  "doc",
  "docQuote",
  "docRange",
  "kind",
  "code",
  "ref",
  "sources",
  "procedure",
  "ttl",
  "owner",
  "by",
  "suggest",
] as const;

const SPAN_KEYS = ["file", "quote", "range"];
const SOURCE_KEYS = ["id", "resource", "title"];
const PROCEDURE_KEYS = ["instructions", "steps"];
const STEP_KEYS = ["id", "command", "stdin"];

/** A batch with problems; `errors` holds one line per problem. */
export class RecordValidationError extends Error {
  constructor(readonly errors: string[]) {
    super(
      `record: ${errors.length} problem${errors.length === 1 ? "" : "s"}; nothing was recorded.\n${errors.map((e) => `  ${e}`).join("\n")}`,
    );
    this.name = "RecordValidationError";
  }
}

/**
 * Parse a range: `L42:L44` (inclusive 1-based lines) or `100:130` (char
 * offsets). Throws a one-line message.
 */
export function parseRange(raw: string): RegionSpec {
  const text = raw.trim();
  const colon = text.indexOf(":");
  if (colon < 0) {
    throw new Error(
      `a range is start:end (L42:L44 for lines, or 100:130 for char offsets), got ${JSON.stringify(raw)}`,
    );
  }
  const aRaw = text.slice(0, colon);
  const bRaw = text.slice(colon + 1);
  const isLine = /^L/i.test(aRaw) || /^L/i.test(bRaw);
  const a = Number(aRaw.replace(/^L/i, ""));
  const b = Number(bRaw.replace(/^L/i, ""));
  if (
    aRaw.trim() === "" ||
    bRaw.trim() === "" ||
    !Number.isInteger(a) ||
    !Number.isInteger(b)
  ) {
    throw new Error(`a range needs integer bounds, got ${JSON.stringify(raw)}`);
  }
  return isLine ? { startLine: a, endLine: b } : { start: a, end: b };
}

function isObject(x: unknown): x is Record<string, unknown> {
  return typeof x === "object" && x !== null && !Array.isArray(x);
}

function unknownKeys(
  obj: Record<string, unknown>,
  allowed: readonly string[],
  where: string,
  errors: string[],
): void {
  for (const key of Object.keys(obj)) {
    if (!allowed.includes(key)) {
      errors.push(
        `${where}unknown key "${key}"; allowed: ${allowed.join(", ")}`,
      );
    }
  }
}

function nonEmptyString(x: unknown): x is string {
  return typeof x === "string" && x.trim() !== "";
}

/** A span `{file, quote | range}`; pushes errors and returns undefined when invalid. */
function span(
  raw: unknown,
  where: string,
  errors: string[],
): EvidenceTarget | undefined {
  if (!isObject(raw)) {
    errors.push(`${where} must be an object { file, quote | range }`);
    return undefined;
  }
  const before = errors.length;
  unknownKeys(raw, SPAN_KEYS, `${where}: `, errors);
  if (!nonEmptyString(raw.file)) {
    errors.push(`${where}.file must be a non-empty string`);
  }
  const hasQuote = raw.quote !== undefined;
  const hasRange = raw.range !== undefined;
  let region: RegionSpec | undefined;
  if (hasQuote === hasRange) {
    errors.push(`${where} needs exactly one of quote or range`);
  } else if (hasQuote) {
    if (!nonEmptyString(raw.quote)) {
      errors.push(`${where}.quote must be a non-empty string`);
    } else {
      region = { quote: raw.quote };
    }
  } else if (typeof raw.range !== "string") {
    errors.push(`${where}.range must be a string such as L1:L3`);
  } else {
    try {
      region = parseRange(raw.range);
    } catch (e) {
      errors.push(`${where}.range: ${(e as Error).message}`);
    }
  }
  if (errors.length > before || region === undefined) return undefined;
  return { file: raw.file as string, region };
}

function sources(raw: unknown, errors: string[]): Source[] | undefined {
  if (!Array.isArray(raw)) {
    errors.push("sources must be an array of { id?, resource, title? }");
    return undefined;
  }
  const out: Source[] = [];
  raw.forEach((s, i) => {
    const where = `sources[${i}]`;
    if (!isObject(s)) {
      errors.push(`${where} must be an object { id?, resource, title? }`);
      return;
    }
    unknownKeys(s, SOURCE_KEYS, `${where}: `, errors);
    if (!nonEmptyString(s.resource)) {
      errors.push(`${where}.resource must be a non-empty string`);
    }
    for (const k of ["id", "title"] as const) {
      if (s[k] !== undefined && !nonEmptyString(s[k])) {
        errors.push(`${where}.${k} must be a non-empty string when present`);
      }
    }
    out.push({
      ...(s.id !== undefined ? { id: String(s.id) } : {}),
      resource: String(s.resource),
      ...(s.title !== undefined ? { title: String(s.title) } : {}),
    });
  });
  return out;
}

function procedure(raw: unknown, errors: string[]): Procedure | undefined {
  if (!isObject(raw)) {
    errors.push("procedure must be an object { instructions, steps? }");
    return undefined;
  }
  unknownKeys(raw, PROCEDURE_KEYS, "procedure: ", errors);
  if (!nonEmptyString(raw.instructions)) {
    errors.push("procedure.instructions must be a non-empty string");
  }
  const out: Procedure = { instructions: String(raw.instructions) };
  if (raw.steps !== undefined) {
    if (!Array.isArray(raw.steps)) {
      errors.push(
        "procedure.steps must be an array of { id, command, stdin? }",
      );
      return out;
    }
    out.steps = [];
    raw.steps.forEach((st, i) => {
      const where = `procedure.steps[${i}]`;
      if (!isObject(st)) {
        errors.push(`${where} must be an object { id, command, stdin? }`);
        return;
      }
      unknownKeys(st, STEP_KEYS, `${where}: `, errors);
      for (const k of ["id", "command"] as const) {
        if (!nonEmptyString(st[k])) {
          errors.push(`${where}.${k} must be a non-empty string`);
        }
      }
      if (st.stdin !== undefined && typeof st.stdin !== "string") {
        errors.push(`${where}.stdin must be a string when present`);
      }
      out.steps?.push({
        id: String(st.id),
        command: String(st.command),
        ...(st.stdin !== undefined ? { stdin: String(st.stdin) } : {}),
      });
    });
  }
  return out;
}

/**
 * Validate one item. Returns the call, or the problems (one line each,
 * without an item prefix).
 */
export function parseRecordItem(
  raw: unknown,
  context: { ref: string; defaultBy?: string },
): { call?: RecordCall; errors: string[] } {
  const errors: string[] = [];
  if (!isObject(raw)) return { errors: ["is not an object"] };
  unknownKeys(raw, RECORD_ITEM_KEYS, "", errors);

  if (!nonEmptyString(raw.doc)) errors.push("doc must be a non-empty path");

  let docQuote: string | undefined;
  let docRange: RegionSpec | undefined;
  const hasQuote = raw.docQuote !== undefined;
  const hasRange = raw.docRange !== undefined;
  if (hasQuote === hasRange) {
    errors.push("needs exactly one of docQuote or docRange");
  } else if (hasQuote) {
    if (nonEmptyString(raw.docQuote)) docQuote = raw.docQuote;
    else errors.push("docQuote must be a non-empty string");
  } else if (typeof raw.docRange !== "string") {
    errors.push("docRange must be a string such as L1:L3");
  } else {
    try {
      docRange = parseRange(raw.docRange);
    } catch (e) {
      errors.push(`docRange: ${(e as Error).message}`);
    }
  }

  const kind = ClaimKind.safeParse(raw.kind);
  if (!kind.success) {
    errors.push(
      `kind must be one of ${ClaimKind.options.join(", ")} (got ${JSON.stringify(raw.kind)})`,
    );
  }
  const k = kind.success ? kind.data : undefined;

  const evidence: EvidenceTarget[] = [];
  if (raw.code !== undefined && k !== undefined && k !== "code") {
    errors.push(`code is only for kind code (kind is ${k})`);
  }
  if (raw.ref !== undefined && k !== undefined && k !== "reference") {
    errors.push(`ref is only for kind reference (kind is ${k})`);
  }
  if (k === "code") {
    if (!Array.isArray(raw.code) || raw.code.length === 0) {
      errors.push(
        "kind code needs code: [{ file, quote | range }] with at least one span",
      );
    } else {
      raw.code.forEach((c, i) => {
        const t = span(c, `code[${i}]`, errors);
        if (t) evidence.push(t);
      });
    }
  }
  if (k === "reference") {
    if (raw.ref === undefined) {
      errors.push("kind reference needs ref: { file, quote | range }");
    } else {
      const t = span(raw.ref, "ref", errors);
      if (t) evidence.push(t);
    }
  }

  const srcs =
    raw.sources === undefined ? undefined : sources(raw.sources, errors);
  const proc =
    raw.procedure === undefined ? undefined : procedure(raw.procedure, errors);

  if (raw.ttl !== undefined) {
    if (typeof raw.ttl !== "string" || !DURATION_PATTERN.test(raw.ttl)) {
      errors.push(
        `ttl must be an ISO 8601 duration such as P90D (got ${JSON.stringify(raw.ttl)})`,
      );
    }
  } else if (k === "attested") {
    errors.push("kind attested needs a ttl, e.g. P90D");
  }
  for (const key of ["owner", "by"] as const) {
    const v = raw[key];
    if (v !== undefined && (typeof v !== "string" || !ACTOR_PATTERN.test(v))) {
      errors.push(
        `${key} must be human:<id>, agent:<name>, or process:<name> (got ${JSON.stringify(v)})`,
      );
    }
  }
  const by = typeof raw.by === "string" ? raw.by : context.defaultBy;
  if (by === undefined) {
    errors.push(
      "no actor to record under: pass by (--by) or set git user.email",
    );
  }
  if (raw.suggest !== undefined && typeof raw.suggest !== "boolean") {
    errors.push("suggest must be true or false");
  }

  if (errors.length > 0 || k === undefined || by === undefined) {
    return { errors };
  }
  return {
    errors,
    call: {
      docPath: raw.doc as string,
      docQuote,
      docRange,
      kind: k,
      evidence,
      ...(srcs !== undefined ? { sources: srcs } : {}),
      ...(proc !== undefined ? { procedure: proc } : {}),
      by,
      ...(typeof raw.owner === "string" ? { owner: raw.owner } : {}),
      ref: context.ref,
      ...(typeof raw.ttl === "string" ? { ttl: raw.ttl } : {}),
      enforcement: raw.suggest === true ? "suggested" : "enforced",
    },
  };
}

/**
 * Validate a batch. Throws `RecordValidationError` naming every problem as
 * `item <i>: <problem>` (or just `<problem>` when `prefix` is false).
 */
export function parseRecordItems(
  items: unknown[],
  context: { ref: string; defaultBy?: string },
  prefix = true,
): RecordCall[] {
  if (items.length === 0) {
    throw new RecordValidationError(["the batch has no items"]);
  }
  const errors: string[] = [];
  const calls: RecordCall[] = [];
  items.forEach((item, i) => {
    const r = parseRecordItem(item, context);
    const where = prefix ? `item ${i}: ` : "";
    for (const e of r.errors) errors.push(`${where}${e}`);
    if (r.call) calls.push(r.call);
  });
  if (errors.length > 0) throw new RecordValidationError(errors);
  return calls;
}

/** One `--source <id>=<resource>` flag value: split at the first `=`. */
export function parseSourceFlag(value: string): Source {
  const eq = value.indexOf("=");
  if (eq <= 0 || eq === value.length - 1) {
    throw new Error(
      `--source expects <id>=<resource>, e.g. stripe=https://dashboard.stripe.com/products/prod_123 (got ${JSON.stringify(value)})`,
    );
  }
  return { id: value.slice(0, eq), resource: value.slice(eq + 1) };
}
