/** `hibi record`: record claims from flags or `--from-file`. */
import { currentRef } from "../../git/git.ts";
import type { RecordCall, RecordResult } from "../../index.ts";
import {
  parseRecordItems,
  parseSourceFlag,
  RecordValidationError,
} from "../record-input.ts";
import { envelope } from "../render/json.ts";
import * as misc from "../render/misc.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  fail,
  readFromFile,
  str,
  type Token,
  type Values,
} from "../shared.ts";

/** The `record` flags that `--from-file` replaces. `--by` stays: it is the default for items without `by`. */
const RECORD_ITEM_FLAGS = [
  "doc",
  "doc-quote",
  "doc-range",
  "kind",
  "code-file",
  "code-quote",
  "code-range",
  "ref-file",
  "ref-quote",
  "ref-range",
  "source",
  "instructions",
  "suggest",
  "ttl",
  "owner",
];

/**
 * The JSON item the `record` flags describe, so flags and `--from-file` share
 * one validation. Code spans come from the flag order: each `--code-file`
 * opens a span that takes the `--code-quote` or `--code-range` after it.
 */
function recordItemFromFlags(
  values: Values,
  tokens: Token[],
): { item: Record<string, unknown>; errors: string[] } {
  const errors: string[] = [];
  const code: Record<string, string>[] = [];
  for (const t of tokens) {
    if (t.kind !== "option" || t.value === undefined) continue;
    if (t.name === "code-file") {
      code.push({ file: t.value });
    } else if (t.name === "code-quote" || t.name === "code-range") {
      const last = code.at(-1);
      const key = t.name === "code-quote" ? "quote" : "range";
      if (last === undefined) {
        errors.push(`--${t.name} needs a --code-file before it`);
      } else if (last.quote !== undefined || last.range !== undefined) {
        errors.push(
          `--code-file ${last.file} already has a span; pass one --code-quote or --code-range per --code-file`,
        );
      } else {
        last[key] = t.value;
      }
    }
  }
  const ref: Record<string, unknown> = {};
  if (values["ref-file"] !== undefined) ref.file = values["ref-file"];
  if (values["ref-quote"] !== undefined) ref.quote = values["ref-quote"];
  if (values["ref-range"] !== undefined) ref.range = values["ref-range"];
  const sources = [];
  for (const raw of (values.source as string[] | undefined) ?? []) {
    try {
      sources.push(parseSourceFlag(raw));
    } catch (e) {
      errors.push((e as Error).message);
    }
  }
  const item: Record<string, unknown> = {
    doc: values.doc,
    docQuote: values["doc-quote"],
    docRange: values["doc-range"],
    kind: values.kind,
    ...(code.length > 0 ? { code } : {}),
    ...(Object.keys(ref).length > 0 ? { ref } : {}),
    ...(sources.length > 0 ? { sources } : {}),
    ...(values.instructions !== undefined
      ? { procedure: { instructions: values.instructions } }
      : {}),
    ttl: values.ttl,
    owner: values.owner,
    ...(values.suggest ? { suggest: true } : {}),
  };
  for (const k of Object.keys(item)) if (item[k] === undefined) delete item[k];
  return { item, errors };
}

/** The lean `record` result: the handle plus what an agent needs next. */
function recordSummary(r: RecordResult): Record<string, unknown> {
  return {
    id: r.claim.id,
    kind: r.claim.kind,
    doc: r.document.path,
    evidence: r.claim.evidence[0]?.file ?? null,
    enforcement: r.claim.enforcement,
    updated: r.updated,
    existingClaims: r.existingClaims,
    ...(r.warnings.length > 0 ? { warnings: r.warnings } : {}),
  };
}

export async function runRecord(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, anchorRoot, open } = ctx;
  const { values, tokens } = args;
  const engine = await open();
  const ref = await currentRef(anchorRoot);
  const defaultBy = str(values.by);
  const fromFile = str(values["from-file"]);

  let items: unknown[];
  if (fromFile !== undefined) {
    const extra = RECORD_ITEM_FLAGS.filter((f) => values[f] !== undefined);
    if (extra.length > 0) {
      return fail(
        `--from-file takes every field from the JSON; drop ${extra.map((f) => `--${f}`).join(", ")}`,
        mode,
      );
    }
    let parsed: unknown;
    try {
      const raw = await readFromFile(anchorRoot, fromFile);
      try {
        parsed = JSON.parse(raw);
      } catch (e) {
        throw new Error(`invalid JSON (${(e as Error).message})`);
      }
    } catch (e) {
      return fail(`record --from-file: ${(e as Error).message}`, mode);
    }
    if (!Array.isArray(parsed)) {
      return fail(
        "record --from-file expects a JSON array of claim items",
        mode,
      );
    }
    items = parsed;
  } else {
    const { item, errors } = recordItemFromFlags(values, tokens);
    if (errors.length > 0) {
      const err = new RecordValidationError(errors);
      return fail(err.message, mode, err.errors);
    }
    items = [item];
  }

  let calls: RecordCall[];
  try {
    calls = parseRecordItems(items, { ref, defaultBy }, fromFile !== undefined);
  } catch (e) {
    if (e instanceof RecordValidationError) {
      return fail(e.message, mode, e.errors);
    }
    return fail((e as Error).message, mode);
  }

  // All or nothing: every item is validated before anything is written,
  // and a failure while applying rolls back only what this batch wrote.
  const { results, failures } = await engine.recordBatch(calls);
  if (failures.length > 0) {
    const err = new RecordValidationError(
      failures.map(({ index, message }) => {
        const where = fromFile !== undefined ? `item ${index}: ` : "";
        return `${where}${message.split("\n").join(" ")}`;
      }),
    );
    return fail(err.message, mode, err.errors);
  }

  if (fromFile !== undefined) {
    const summaries = results.map(recordSummary);
    await emit(
      mode,
      envelope(
        "record",
        {
          batch: true,
          count: summaries.length,
          updated: results.filter((r) => r.updated).length,
          results: summaries,
        },
        "hibi check",
      ),
      () => results.map((r) => misc.renderRecord(r, style, mode)).join(""),
    );
    return 0;
  }
  const result = results[0] as RecordResult;
  const next =
    result.existingClaims.length > 0
      ? "this sentence is already claimed; did you mean `hibi reanchor`?"
      : "hibi check";
  const payload = mode.explain
    ? { ...result, claimId: result.claim.id }
    : recordSummary(result);
  await emit(mode, envelope("record", payload, next), () =>
    misc.renderRecord(result, style, mode),
  );
  return 0;
}
