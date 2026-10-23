#!/usr/bin/env bun
/**
 * The hibi CLI: JSON-first, the consumer is usually an agent. Verbs: init,
 * record, check, list, audit, evidence, attest, reanchor, retire, supersede,
 * archive, schema, completions, version. Exit codes: 0 clean or warnings,
 * 2 gating, 1 operational error.
 *
 * A thin imperative shell: it parses argv against the per-command option
 * tables (`options.ts`), resolves git into plain values, delegates to the
 * `Engine` facade, and serializes the result.
 */
import { appendFile, readFile, writeFile } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { parseArgs } from "node:util";
import pkg from "../../package.json" with { type: "json" };
import { parseAck } from "../engine/attest.ts";
import { auditExitCode } from "../engine/audit.ts";
import { changedFiles, currentRef } from "../git/git.ts";
import {
  ClaimStore,
  Engine,
  type EvidenceTarget,
  EvidenceValidationError,
  type FailOn,
  type ListState,
  normalizePath,
  type RecordCall,
  type RecordResult,
  type RegionSpec,
  type StoreLocation,
} from "../index.ts";
import { completionScript, isShell } from "./completions.ts";
import {
  COMMANDS,
  commandHelp,
  commandSpec,
  INSTRUCTIONS_LINE,
  missingValue,
  parseArgsOptions,
  repeatedFlag,
  usage,
  validateValues,
} from "./options.ts";
import {
  parseRange,
  parseRecordItems,
  parseSourceFlag,
  RecordValidationError,
} from "./record-input.ts";
import { renderAttest } from "./render/attest.ts";
import { renderAudit } from "./render/audit.ts";
import { renderCheck } from "./render/check.ts";
import { evidenceNext, renderEvidence } from "./render/evidence.ts";
import { fileReader } from "./render/helpers.ts";
import {
  envelope,
  type ProjectionOptions,
  projectCheckReport,
} from "./render/json.ts";
import * as misc from "./render/misc.ts";
import { type OutputMode, resolveMode } from "./render/mode.ts";
import { renderOverview } from "./render/status.ts";
import { makeStyle } from "./render/style.ts";

const EXIT_OPERATIONAL_ERROR = 1;

type Values = Record<string, string | boolean | string[] | undefined>;

/**
 * Write to stdout or stderr and resolve once the write's callback reports it
 * written. Every CLI write goes through here and is awaited before
 * `process.exit`: a write to a pipe is asynchronous, and exiting earlier
 * drops whatever did not fit in the pipe buffer. A reader that closed early
 * (EPIPE, e.g. `| head`) wants nothing more, so that error is ignored.
 */
function writeOut(stream: NodeJS.WriteStream, text: string): Promise<void> {
  return new Promise((resolve, reject) => {
    if (text.length === 0) return resolve();
    stream.write(text, (e) => {
      if (e && (e as { code?: string }).code !== "EPIPE") reject(e);
      else resolve();
    });
  });
}
const print = (text: string) => writeOut(process.stdout, text);
const printErr = (text: string) => writeOut(process.stderr, text);

async function out(value: unknown, pretty: boolean): Promise<void> {
  await print(`${JSON.stringify(value, jsonReplacer, pretty ? 2 : 0)}\n`);
}
function jsonReplacer(_k: string, v: unknown) {
  return typeof v === "bigint" ? v.toString() : v;
}

async function emitIds(ids: string[]): Promise<void> {
  const seen = new Set<string>();
  const list: string[] = [];
  for (const id of ids) {
    if (seen.has(id)) continue;
    seen.add(id);
    list.push(id);
  }
  if (list.length > 0) await print(`${list.join("\n")}\n`);
}

function isHuman(mode: OutputMode): boolean {
  return mode.kind === "rich" || mode.kind === "compact";
}

/** Emit a result: the human renderer in rich/compact mode, else JSON. */
async function emit(
  mode: OutputMode,
  value: unknown,
  human: () => string | Promise<string>,
): Promise<void> {
  if (isHuman(mode)) await print(await human());
  else await out(value, mode.kind === "json-pretty");
}

/** Report an operational error and exit 1. `errors` (one line each) also goes into the JSON. */
async function fail(
  message: string,
  mode: OutputMode,
  errors?: string[],
): Promise<never> {
  if (isHuman(mode)) {
    await printErr(`${makeStyle(mode.color).red("error:")} ${message}\n`);
  } else {
    await out(
      { ok: false, error: message, ...(errors ? { errors } : {}) },
      mode.kind === "json-pretty",
    );
  }
  process.exit(EXIT_OPERATIONAL_ERROR);
}

function absPath(root: string, p: string): string {
  return isAbsolute(p) ? p : join(root, p);
}

function str(v: unknown): string | undefined {
  return v === undefined ? undefined : String(v);
}

function projection(mode: OutputMode): ProjectionOptions {
  return { explain: mode.explain, hints: mode.hints };
}

/** Build the rich/compact `check` render context. */
async function checkContext(
  engine: Engine,
  report: import("../index.ts").CheckReport,
  mode: OutputMode,
  lead?: string[],
) {
  const claims = await engine.store.allClaims();
  return {
    report,
    claimsById: new Map(claims.map((c) => [c.id, c])),
    read: fileReader(engine.store.anchorRoot),
    style: makeStyle(mode.color),
    mode,
    lead,
  };
}

/** A span from `--*-quote <text>` or `--*-range L42:L44 | 100:130`; undefined when neither was given. */
function spanSpec(quote: unknown, range: unknown): RegionSpec | undefined {
  if (quote !== undefined) return { quote: String(quote) };
  if (range !== undefined) return parseRange(String(range));
  return undefined;
}

/** Read `--from-file <path|->`: stdin for `-`, else a file under the anchor root. */
async function readFromFile(root: string, fromFile: string): Promise<string> {
  if (fromFile === "-") return Bun.stdin.text();
  try {
    return await readFile(absPath(root, fromFile), "utf8");
  } catch (e) {
    const code = (e as { code?: string }).code;
    if (code === "ENOENT")
      throw new Error(`--from-file ${fromFile}: no such file`);
    throw e;
  }
}

/** The replacement reference span of `reanchor` (`--ref-file` with `--ref-quote` or `--ref-range`). */
function reanchorRefSide(values: Values): EvidenceTarget | undefined {
  const refFile = str(values["ref-file"]);
  const region = spanSpec(values["ref-quote"], values["ref-range"]);
  if (refFile === undefined) {
    if (region !== undefined) {
      throw new Error("--ref-quote/--ref-range need --ref-file");
    }
    return undefined;
  }
  if (region === undefined) {
    throw new Error(`--ref-file ${refFile} needs --ref-quote or --ref-range`);
  }
  return { file: refFile, region };
}

/**
 * The replacement code spans of `reanchor`, in flag order: each `--code-file`
 * opens a span that takes the `--code-quote` or `--code-range` after it.
 */
function reanchorCodeSide(tokens: Token[]): EvidenceTarget[] {
  const spans: { file: string; region?: RegionSpec }[] = [];
  for (const t of tokens) {
    if (t.kind !== "option" || t.value === undefined) continue;
    if (t.name === "code-file") {
      spans.push({ file: t.value });
    } else if (t.name === "code-quote" || t.name === "code-range") {
      const last = spans.at(-1);
      if (last === undefined) {
        throw new Error(`--${t.name} needs a --code-file before it`);
      }
      if (last.region !== undefined) {
        throw new Error(
          `--code-file ${last.file} already has a span; pass one --code-quote or --code-range per --code-file`,
        );
      }
      last.region =
        t.name === "code-quote" ? { quote: t.value } : parseRange(t.value);
    }
  }
  return spans.map((s) => {
    if (s.region === undefined) {
      throw new Error(
        `--code-file ${s.file} needs --code-quote or --code-range`,
      );
    }
    return { file: s.file, region: s.region };
  });
}

type Token = {
  kind: string;
  name?: string;
  value?: string;
};

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

/** Append the instruction line to `file` unless it is already there. Creates the file. */
async function appendInstructionLine(
  file: string,
): Promise<{ file: string; appended: boolean }> {
  let text: string | null = null;
  try {
    text = await readFile(file, "utf8");
  } catch (e) {
    if ((e as { code?: string }).code !== "ENOENT") throw e;
  }
  if (text === null) {
    await writeFile(file, `${INSTRUCTIONS_LINE}\n`);
    return { file, appended: true };
  }
  if (text.includes(INSTRUCTIONS_LINE)) return { file, appended: false };
  const sep = text === "" || text.endsWith("\n") ? "" : "\n";
  await appendFile(file, `${sep}${INSTRUCTIONS_LINE}\n`);
  return { file, appended: true };
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

/** Pre-scan the output flags so a parse error can already be reported in the right format. */
function preMode(rest: string[]): OutputMode {
  const fmtAt = rest.indexOf("--format");
  return resolveMode(
    {
      json: rest.includes("--json"),
      format: fmtAt >= 0 ? rest[fmtAt + 1] : undefined,
    },
    { isTTY: process.stdout.isTTY, env: process.env },
  );
}

async function main(argv: string[]): Promise<number> {
  const cmd = argv[0];
  const rest = argv.slice(1);

  if (cmd === undefined || cmd === "help" || cmd === "--help" || cmd === "-h") {
    await print(usage());
    return cmd === undefined ? EXIT_OPERATIONAL_ERROR : 0;
  }
  if (cmd === "--version" || cmd === "-v") {
    await print(`${pkg.version}\n`);
    return 0;
  }
  const spec = commandSpec(cmd);
  if (!spec) {
    return fail(
      `unknown command: ${cmd}. Commands: ${COMMANDS.map((c) => c.name).join(", ")}. Run \`hibi --help\`.`,
      preMode(rest),
    );
  }
  if (rest.includes("--help") || rest.includes("-h")) {
    await print(commandHelp(spec));
    return 0;
  }

  let values: Values;
  let positionals: string[];
  let tokens: Token[];
  try {
    const parsed = parseArgs({
      args: rest,
      allowPositionals: true,
      strict: true,
      tokens: true,
      options: parseArgsOptions(spec),
    });
    values = parsed.values as Values;
    positionals = parsed.positionals;
    tokens = parsed.tokens as Token[];
  } catch (e) {
    const err = e as { code?: string; message: string };
    const m = /'(--?[^' ]+)/.exec(err.message);
    const flag = m?.[1];
    const msg =
      err.code === "ERR_PARSE_ARGS_UNKNOWN_OPTION"
        ? `unknown option ${flag ?? ""} for hibi ${cmd}; run \`hibi ${cmd} --help\``
        : err.code === "ERR_PARSE_ARGS_INVALID_OPTION_VALUE"
          ? flag
            ? missingValue(spec, flag)
            : `an option needs a value; run \`hibi ${cmd} --help\``
          : err.message;
    return fail(msg, preMode(rest));
  }

  const mode = resolveMode(
    {
      format: str(values.format),
      json: Boolean(values.json),
      color: str(values.color),
      explain: Boolean(values.explain),
      noHints: Boolean(values["no-hints"]),
    },
    { isTTY: process.stdout.isTTY, env: process.env },
  );
  const style = makeStyle(mode.color);

  const invalid = repeatedFlag(spec, tokens) ?? validateValues(spec, values);
  if (invalid) return fail(invalid, mode);
  if (spec.positional?.required && positionals.length === 0) {
    return fail(`${cmd} requires a <${spec.positional.name}> argument`, mode);
  }
  if (spec.positional?.values && positionals[0] !== undefined) {
    if (!spec.positional.values.includes(positionals[0])) {
      return fail(
        `${cmd} expects ${spec.positional.values.join("|")} (got: ${positionals[0]})`,
        mode,
      );
    }
  }
  if (!spec.positional && positionals.length > 0) {
    return fail(
      `unexpected argument: ${positionals[0]}; run \`hibi ${cmd} --help\``,
      mode,
    );
  }

  const anchorRoot = str(values.cwd) ?? process.cwd();
  const noAst = Boolean(values["no-ast"]);
  const storeDir = str(values["store-dir"]);
  const loc: string | StoreLocation = storeDir
    ? { anchorRoot, storeDir: absPath(anchorRoot, storeDir) }
    : anchorRoot;

  const open = async (): Promise<Engine> =>
    // Only a missing store means "run hibi init"; a version skew or a corrupt
    // config each carry their own actionable message.
    Engine.open(loc, { noAst }).catch(async (e) =>
      (await ClaimStore.isInitialized(loc))
        ? fail((e as Error).message, mode)
        : fail("No claim store. Run `hibi init`.", mode),
    );

  switch (spec.name) {
    case "init": {
      let engine: Engine;
      try {
        engine = await Engine.init(loc);
      } catch (e) {
        return fail((e as Error).message, mode);
      }
      const config = await engine.store.config();
      const instructionsFile = str(values.instructions);
      let instructions: { file: string; appended: boolean } | undefined;
      if (instructionsFile !== undefined) {
        try {
          const r = await appendInstructionLine(
            absPath(anchorRoot, instructionsFile),
          );
          instructions = { file: instructionsFile, appended: r.appended };
        } catch (e) {
          return fail((e as Error).message, mode);
        }
      }
      const payload = {
        store: engine.store.dir,
        version: config.version,
        judge: config.judge,
        ...(instructions !== undefined ? { instructions } : {}),
      };
      await emit(
        mode,
        envelope("init", payload, "hibi audit --doc <file>"),
        () => misc.renderInit(payload, style, mode),
      );
      return 0;
    }

    case "record": {
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
        calls = parseRecordItems(
          items,
          { ref, defaultBy },
          fromFile !== undefined,
        );
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

    case "check": {
      const engine = await open();
      const since = str(values.since);
      const doc = str(values.doc);
      let files: string[] | undefined;
      if (since !== undefined) {
        try {
          files = await changedFiles(since, anchorRoot);
        } catch (e) {
          return fail((e as Error).message, mode);
        }
      }
      let report: import("../index.ts").CheckReport;
      try {
        report = await engine.check({
          onlyFiles: files,
          doc,
          failOn: (str(values["fail-on"]) ?? "gating") as FailOn,
          ref: await currentRef(anchorRoot),
        });
      } catch (e) {
        return fail((e as Error).message, mode);
      }
      const extra: Record<string, unknown> = {};
      if (since !== undefined) {
        extra.since = since;
        extra.changedFiles = files;
      }
      if (doc !== undefined) {
        extra.doc = normalizePath(engine.store.anchorRoot, doc);
      }
      const value = projectCheckReport(report, projection(mode), extra);
      await emit(mode, value, async () => {
        if (values.overview) {
          const [claims, config] = await Promise.all([
            engine.store.allClaims(),
            engine.store.config(),
          ]);
          return renderOverview({
            report,
            claims,
            storeVersion: config.version,
            style,
            mode,
          });
        }
        const lead: string[] = [];
        if (since !== undefined) {
          lead.push(
            `${style.dim("since")} ${style.bold(since)}  ${style.dim(`${files?.length ?? 0} changed file${files?.length === 1 ? "" : "s"}`)}`,
          );
        }
        return renderCheck(await checkContext(engine, report, mode, lead));
      });
      return report.exitCode;
    }

    case "list": {
      const engine = await open();
      let result: import("../index.ts").ListResult;
      try {
        result = await engine.list({
          state: (str(values.state) ?? "all") as ListState,
          path: str(values.path),
          ref: await currentRef(anchorRoot),
        });
      } catch (e) {
        return fail((e as Error).message, mode);
      }
      if (values["ids-only"]) {
        await emitIds(result.claims.map((c) => c.claimId));
        return 0;
      }
      await emit(mode, envelope("list", { ...result }), () =>
        misc.renderList(result, style, mode),
      );
      return 0;
    }

    case "audit": {
      const engine = await open();
      const doc = String(values.doc);
      try {
        const report = await engine.audit(doc);
        await emit(mode, envelope("audit", { ...report }), () =>
          renderAudit(report, style, mode),
        );
        return auditExitCode(report, {
          failUncovered: Boolean(values["fail-uncovered"]),
        });
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "reanchor": {
      const engine = await open();
      const claimId = positionals[0] as string;
      const mutation = [
        values.doc,
        values["doc-quote"],
        values["doc-range"],
        values["code-file"],
        values["code-quote"],
        values["code-range"],
        values["ref-file"],
        values["ref-quote"],
        values["ref-range"],
        values["dry-run"],
      ].some((f) => f !== undefined && f !== false);
      if (values.suggest) {
        if (mutation) {
          return fail(
            "--suggest is read-only and cannot be combined with span or write flags.",
            mode,
          );
        }
        try {
          const result = await engine.reanchorSuggest(claimId);
          await emit(
            mode,
            envelope("reanchor-suggest", {
              id: result.claimId,
              candidates: result.candidates,
            }),
            () => misc.renderReanchorSuggest(result, style, mode),
          );
          return 0;
        } catch (e) {
          return fail((e as Error).message, mode);
        }
      }
      const dryRun = Boolean(values["dry-run"]);
      try {
        const docSpec = spanSpec(values["doc-quote"], values["doc-range"]);
        const code = reanchorCodeSide(tokens);
        const refSpan = reanchorRefSide(values);
        if (code.length > 0 && refSpan !== undefined) {
          throw new Error(
            "pass either --code-* or --ref-* flags, not both; a claim is code or reference",
          );
        }
        const result = await engine.reanchor(claimId, {
          doc: str(values.doc),
          docQuote: docSpec?.quote,
          docRange: docSpec?.quote === undefined ? docSpec : undefined,
          evidence: code.length > 0 ? code : undefined,
          reference: refSpan,
          ref: await currentRef(anchorRoot),
          dryRun,
        });
        const reasons = result.kept.map((k) => k.reason);
        if (result.relocated === 0 && reasons.length > 0) {
          return fail(
            `claim ${result.claim.id} was left as stored: ${reasons.join(" | ")}`,
            mode,
            reasons,
          );
        }
        const payload = {
          id: result.claim.id,
          doc: result.doc,
          ...(result.evidence !== undefined
            ? { evidence: result.evidence }
            : {}),
          before: result.before,
          after: result.after,
          ...(result.kept.length > 0 ? { kept: result.kept } : {}),
          ...(result.warnings.length > 0 ? { warnings: result.warnings } : {}),
          ...(dryRun ? { dryRun: true } : {}),
          ...(mode.explain ? { claim: result.claim } : {}),
        };
        await emit(
          mode,
          envelope(
            "reanchor",
            payload,
            dryRun ? "re-run without --dry-run to apply" : "hibi check",
          ),
          () => misc.renderReanchor(result, style, mode, dryRun),
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "retire": {
      const engine = await open();
      const claimId = positionals[0] as string;
      const dryRun = Boolean(values["dry-run"]);
      try {
        const result = await engine.retire(claimId, { dryRun });
        await emit(
          mode,
          envelope(
            "retire",
            {
              id: result.claim.id,
              alreadyRetired: result.alreadyRetired,
              ...(dryRun ? { dryRun: true } : {}),
            },
            dryRun ? "re-run without --dry-run to apply" : "hibi check",
          ),
          () => misc.renderRetire(result, style, mode, dryRun),
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "supersede": {
      const engine = await open();
      try {
        const result = await engine.supersede({
          from: String(values.from),
          to: String(values.to),
          dryRun: Boolean(values["dry-run"]),
        });
        const next = result.dryRun
          ? "re-run without --dry-run to apply"
          : result.relocated.some((r) => r.doc === "orphaned")
            ? "reanchor or retire each orphaned claim, then hibi check"
            : result.misses.length > 0
              ? "fix each failed move, then hibi check"
              : "hibi check";
        await emit(
          mode,
          envelope(
            "supersede",
            {
              from: result.oldDoc.path,
              to: result.newDoc.path,
              relocated: result.relocated,
              misses: result.misses,
              strandedClaims: result.strandedClaims,
              dryRun: result.dryRun,
            },
            next,
          ),
          () => misc.renderSupersede(result, style, mode),
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "archive": {
      const engine = await open();
      try {
        const result = await engine.archive(
          String(values.doc),
          str(values.successor),
          { dryRun: Boolean(values["dry-run"]) },
        );
        const next = result.dryRun
          ? "re-run without --dry-run to apply"
          : result.strandedClaims.length > 0
            ? "reanchor or retire each stranded claim, then hibi check"
            : "hibi check";
        await emit(
          mode,
          envelope(
            "archive",
            {
              doc: result.document.path,
              archivedTo: result.archivedTo,
              successor: result.successor ?? null,
              lifecycle: result.document.lifecycle,
              strandedClaims: result.strandedClaims,
              dryRun: result.dryRun,
            },
            next,
          ),
          () => misc.renderArchive(result, style, mode),
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "evidence": {
      const engine = await open();
      const fromFile = String(values["from-file"]);
      const by = str(values.by);
      let parsed: unknown;
      try {
        const raw = await readFromFile(anchorRoot, fromFile);
        try {
          parsed = JSON.parse(raw);
        } catch (e) {
          throw new Error(`invalid JSON (${(e as Error).message})`);
        }
      } catch (e) {
        return fail(`evidence --from-file: ${(e as Error).message}`, mode);
      }
      const isItem = (item: unknown): item is Record<string, unknown> =>
        typeof item === "object" && item !== null && !Array.isArray(item);
      const raws = Array.isArray(parsed) ? parsed : [parsed];
      if (by === undefined) {
        const missing = raws.flatMap((item, i) =>
          isItem(item) && !("by" in item)
            ? [
                `item ${i}${typeof item.claimId === "string" ? ` (${item.claimId})` : ""}: no "by"; pass --by <actor> or a "by" key: human:<id>, agent:<name>, or process:<name>`,
              ]
            : [],
        );
        if (missing.length > 0) {
          const err = new EvidenceValidationError(missing);
          return fail(err.message, mode, err.errors);
        }
      }
      const items = raws.map((item) =>
        isItem(item) && !("by" in item) ? { ...item, by } : item,
      );
      try {
        const report = await engine.evidence(items);
        const next = evidenceNext(report);
        await emit(mode, envelope("evidence", { ...report }, next), () =>
          renderEvidence(report, style, mode),
        );
        return 0;
      } catch (e) {
        if (e instanceof EvidenceValidationError) {
          return fail(e.message, mode, e.errors);
        }
        return fail((e as Error).message, mode);
      }
    }

    case "attest": {
      const engine = await open();
      try {
        const ackValue = str(values.ack);
        const result = await engine.attest(positionals[0] as string, {
          by: String(values.by),
          note: str(values.note),
          ack: ackValue === undefined ? undefined : parseAck(ackValue),
        });
        await emit(mode, envelope("attest", { ...result }, "hibi check"), () =>
          renderAttest(result, style, mode),
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "schema": {
      const { SCHEMAS } = await import("../core/model.ts");
      const z = await import("zod");
      const all: Record<string, import("zod").ZodType> = { ...SCHEMAS };
      const name = str(values.name);
      // Machine output in every mode: indented unless the format is plain json.
      const pretty = mode.kind !== "json";
      if (name) {
        const schema = all[name];
        if (!schema) {
          return fail(
            `unknown schema: ${name}. Known: ${Object.keys(all).join(", ")}`,
            mode,
          );
        }
        await out(
          z.toJSONSchema(schema, { target: "draft-2020-12", reused: "inline" }),
          pretty,
        );
      } else {
        await out(envelope("schema", { schemas: Object.keys(all) }), pretty);
      }
      return 0;
    }

    case "completions": {
      const shell = positionals[0];
      if (!isShell(shell)) {
        return fail(`completions requires zsh | bash | fish`, mode);
      }
      await print(completionScript(shell));
      return 0;
    }

    case "version": {
      await emit(
        mode,
        envelope("version", { name: "hibi", version: pkg.version }),
        () => misc.renderVersion(pkg.version, style),
      );
      return 0;
    }

    default:
      return fail(`unknown command: ${cmd}`, mode);
  }
}

// Every write is awaited inside `main`, so exiting here drops nothing.
main(process.argv.slice(2))
  .then((code) => process.exit(code))
  .catch(async (e) => {
    await printErr(`error: ${String(e?.message ?? e)}\n`);
    process.exit(EXIT_OPERATIONAL_ERROR);
  });
