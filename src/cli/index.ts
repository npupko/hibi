#!/usr/bin/env bun
/**
 * The hibi CLI: JSON-first, the consumer is usually an agent. Verbs: init,
 * record, check, list, audit, evidence, attest, reanchor, retire, supersede,
 * archive, schema, completions, version. Exit codes: 0 clean or warnings,
 * 2 gating, 1 operational error.
 *
 * A thin imperative shell: it parses argv against the per-command option
 * tables (`options.ts`), resolves git into plain values, dispatches to the
 * command module (`commands/<command>.ts`), which delegates to the
 * `Engine` facade and serializes the result.
 */
import { parseArgs } from "node:util";
import pkg from "../../package.json" with { type: "json" };
import { ClaimStore, Engine, type StoreLocation } from "../index.ts";
import { runArchive } from "./commands/archive.ts";
import { runAttest } from "./commands/attest.ts";
import { runAudit } from "./commands/audit.ts";
import { runCheck } from "./commands/check.ts";
import { runCompletions } from "./commands/completions.ts";
import { runEvidence } from "./commands/evidence.ts";
import { runInit } from "./commands/init.ts";
import { runList } from "./commands/list.ts";
import { runReanchor } from "./commands/reanchor.ts";
import { runRecord } from "./commands/record.ts";
import { runRetire } from "./commands/retire.ts";
import { runSchema } from "./commands/schema.ts";
import { runSupersede } from "./commands/supersede.ts";
import { runVersion } from "./commands/version.ts";
import {
  COMMANDS,
  commandHelp,
  commandSpec,
  missingValue,
  parseArgsOptions,
  repeatedFlag,
  usage,
  validateValues,
} from "./options.ts";
import { type OutputMode, resolveMode } from "./render/mode.ts";
import { makeStyle } from "./render/style.ts";
import {
  absPath,
  type Command,
  EXIT_OPERATIONAL_ERROR,
  fail,
  print,
  printErr,
  str,
  type Token,
  type Values,
} from "./shared.ts";

/** One module per command; the key is the command name in `options.ts`. */
const DISPATCH: Record<string, Command> = {
  init: runInit,
  record: runRecord,
  check: runCheck,
  list: runList,
  audit: runAudit,
  reanchor: runReanchor,
  retire: runRetire,
  supersede: runSupersede,
  archive: runArchive,
  evidence: runEvidence,
  attest: runAttest,
  schema: runSchema,
  completions: runCompletions,
  version: runVersion,
};

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

  const run = DISPATCH[spec.name];
  if (run === undefined) return fail(`unknown command: ${cmd}`, mode);
  return run(
    { mode, style, anchorRoot, loc, open },
    { values, positionals, tokens },
  );
}

// Every write is awaited inside `main`, so exiting here drops nothing.
main(process.argv.slice(2))
  .then((code) => process.exit(code))
  .catch(async (e) => {
    await printErr(`error: ${String(e?.message ?? e)}\n`);
    process.exit(EXIT_OPERATIONAL_ERROR);
  });
