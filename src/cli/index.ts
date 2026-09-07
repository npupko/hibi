#!/usr/bin/env bun
/**
 * The hibi CLI: JSON-first, the consumer is usually an agent. Verbs: init,
 * record, check, list, coverage, reanchor, retire, supersede, archive, schema,
 * completions, version. Exit codes: 0 clean, 2 gating, 1 operational error.
 *
 * A thin imperative shell: it parses argv against the per-command option
 * tables (`options.ts`), resolves git into plain values, delegates to the
 * `Engine` facade, and serializes the result.
 */
import { isAbsolute, join } from "node:path";
import { parseArgs } from "node:util";
import pkg from "../../package.json" with { type: "json" };
import type { Verifier } from "../core/model.ts";
import { changedFiles, currentRef } from "../git/git.ts";
import {
  ClaimStore,
  type CodeTarget,
  Engine,
  type FailOn,
  type ListState,
  type RecordCall,
  type RecordResult,
  type RegionSpec,
  type StoreLocation,
} from "../index.ts";
import { completionScript, isShell } from "./completions.ts";
import {
  COMMANDS,
  type CommandSpec,
  commandHelp,
  commandSpec,
  parseArgsOptions,
  usage,
  validateValues,
} from "./options.ts";
import { renderCheck } from "./render/check.ts";
import { fileReader } from "./render/helpers.ts";
import {
  envelope,
  type ProjectionOptions,
  projectCheckReport,
  SCHEMA_VERSION,
} from "./render/json.ts";
import * as misc from "./render/misc.ts";
import { type OutputMode, resolveMode } from "./render/mode.ts";
import { renderOverview } from "./render/status.ts";
import { makeStyle } from "./render/style.ts";

const EXIT_OPERATIONAL_ERROR = 1;

type Values = Record<string, string | boolean | string[] | undefined>;

function out(value: unknown, pretty: boolean): void {
  process.stdout.write(
    `${JSON.stringify(value, jsonReplacer, pretty ? 2 : 0)}\n`,
  );
}
function jsonReplacer(_k: string, v: unknown) {
  return typeof v === "bigint" ? v.toString() : v;
}

function emitIds(ids: string[]): void {
  const seen = new Set<string>();
  const list: string[] = [];
  for (const id of ids) {
    if (seen.has(id)) continue;
    seen.add(id);
    list.push(id);
  }
  if (list.length > 0) process.stdout.write(`${list.join("\n")}\n`);
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
  if (isHuman(mode)) process.stdout.write(await human());
  else out(value, mode.kind === "json-pretty");
}

function fail(message: string, mode: OutputMode): never {
  if (isHuman(mode)) {
    process.stderr.write(`${makeStyle(mode.color).red("error:")} ${message}\n`);
  } else {
    out({ ok: false, error: message }, mode.kind === "json-pretty");
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

async function fingerprints(
  engine: Engine,
  mode: OutputMode,
): Promise<Map<string, string> | undefined> {
  if (!mode.explain) return undefined;
  const props = await engine.store.allPropositions();
  return new Map(props.map((p) => [p.id, p.fingerprint]));
}

/** Build the rich/compact `check` render context. */
async function checkContext(
  engine: Engine,
  report: import("../index.ts").CheckReport,
  mode: OutputMode,
  lead?: string[],
) {
  const [assertions, propositions] = await Promise.all([
    engine.store.allAssertions(),
    engine.store.allPropositions(),
  ]);
  return {
    report,
    assertionsById: new Map(assertions.map((a) => [a.id, a])),
    propsById: new Map(propositions.map((p) => [p.id, p])),
    read: fileReader(engine.store.anchorRoot),
    style: makeStyle(mode.color),
    mode,
    lead,
  };
}

/**
 * Parse a span: `--*-quote <text>` (literal) or `--*-range L42:L44` (1-based
 * lines) / `100:130` (char offsets). Undefined when neither was given.
 */
function spanSpec(quote: unknown, range: unknown): RegionSpec | undefined {
  if (quote !== undefined) return { quote: String(quote) };
  if (range !== undefined) {
    const raw = String(range).trim();
    const colon = raw.indexOf(":");
    if (colon < 0) {
      throw new Error(
        `a range is start:end (L42:L44 for lines, or 100:130 for char offsets), got: ${raw}`,
      );
    }
    const aRaw = raw.slice(0, colon);
    const bRaw = raw.slice(colon + 1);
    const isLine = /^L/i.test(aRaw) || /^L/i.test(bRaw);
    const a = Number(aRaw.replace(/^L/i, ""));
    const b = Number(bRaw.replace(/^L/i, ""));
    if (!Number.isFinite(a) || !Number.isFinite(b)) {
      throw new Error(`a range needs numeric bounds, got: ${raw}`);
    }
    return isLine ? { startLine: a, endLine: b } : { start: a, end: b };
  }
  return undefined;
}

/** The code side from `--code-file`/`--code-quote`/`--code-range`/`--glob` (or camelCase keys). */
function codeSideOf(v: {
  codeFile?: unknown;
  codeQuote?: unknown;
  codeRange?: unknown;
  glob?: unknown;
}): CodeTarget[] {
  const glob = str(v.glob);
  const codeFile = str(v.codeFile);
  if (glob && codeFile) {
    throw new Error("pass either --glob or --code-file, not both");
  }
  if (glob) return [{ file: glob, coarse: true }];
  if (codeFile) {
    const region = spanSpec(v.codeQuote, v.codeRange);
    if (!region) {
      throw new Error(
        `--code-file ${codeFile} needs --code-quote or --code-range (or use --glob for a coarse edge)`,
      );
    }
    return [{ file: codeFile, region }];
  }
  if (v.codeQuote !== undefined || v.codeRange !== undefined) {
    throw new Error("--code-quote/--code-range need --code-file");
  }
  return [];
}

function verifierTimeoutMsOf(raw: unknown): number | undefined {
  if (raw === undefined) return undefined;
  const secs = Number(raw);
  if (!Number.isFinite(secs) || secs <= 0) {
    throw new Error(
      `--verifier-timeout expects a positive number of seconds, got: ${String(raw)}`,
    );
  }
  return secs * 1000;
}

/** Parse repeatable `kind:ref` verifiers. */
function parseVerifiers(raw: unknown): Verifier[] {
  if (raw === undefined) return [];
  const items = Array.isArray(raw) ? raw : [raw];
  return items.map((item) => {
    const s = String(item);
    const idx = s.indexOf(":");
    if (idx <= 0) {
      throw new Error(`--verifier expects kind:ref, got: ${s}`);
    }
    return { kind: s.slice(0, idx), ref: s.slice(idx + 1) };
  });
}

/** A `RecordCall` from a JSON spec (one `--from-file` item) or the flag values. */
function recordCallFromSpec(
  spec: Record<string, unknown>,
  ref: string,
): RecordCall {
  const doc = spec.doc;
  if (typeof doc !== "string" || doc.length === 0)
    throw new Error("a claim needs a `doc` path");
  const docSpec = spanSpec(spec.docQuote, spec.docRange);
  if (!docSpec) throw new Error(`claim on ${doc} needs docQuote or docRange`);
  return {
    docPath: doc,
    docQuote: docSpec.quote,
    docRange: docSpec.quote === undefined ? docSpec : undefined,
    code: codeSideOf(spec),
    verified: Boolean(spec.verified),
    owner: str(spec.owner),
    ref,
    ttl: str(spec.ttl),
    enforcement: spec.suggest ? "suggested" : "enforced",
    verifiers: parseVerifiers(spec.verifier),
  };
}

function flagsToSpec(values: Values): Record<string, unknown> {
  return {
    doc: values.doc,
    docQuote: values["doc-quote"],
    docRange: values["doc-range"],
    codeFile: values["code-file"],
    codeQuote: values["code-quote"],
    codeRange: values["code-range"],
    glob: values.glob,
    suggest: values.suggest,
    verified: values.verified,
    verifier: values.verifier,
    ttl: values.ttl,
    owner: values.owner,
  };
}

/** The lean `record` result: the handle plus what an agent needs next. */
function recordSummary(r: RecordResult): Record<string, unknown> {
  return {
    id: r.assertion.id,
    doc: r.document.path,
    code: r.assertion.anchor.code[0]?.file ?? null,
    enforcement: r.assertion.enforcement,
    verified: r.assertion.verified,
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
    process.stdout.write(usage());
    return cmd === undefined ? EXIT_OPERATIONAL_ERROR : 0;
  }
  if (cmd === "--version" || cmd === "-v") {
    process.stdout.write(`${pkg.version}\n`);
    return 0;
  }
  const spec = commandSpec(cmd);
  if (!spec) {
    return fail(
      `unknown command: ${cmd}. Commands: ${COMMANDS.filter((c) => !c.aliasOf)
        .map((c) => c.name)
        .join(", ")}. Run \`hibi --help\`.`,
      preMode(rest),
    );
  }
  if (rest.includes("--help") || rest.includes("-h")) {
    process.stdout.write(commandHelp(spec));
    return 0;
  }

  let values: Values;
  let positionals: string[];
  try {
    const parsed = parseArgs({
      args: rest,
      allowPositionals: true,
      strict: true,
      options: parseArgsOptions(spec),
    });
    values = parsed.values as Values;
    positionals = parsed.positionals;
  } catch (e) {
    const err = e as { code?: string; message: string };
    const m = /'(--?[^']+)'/.exec(err.message);
    const flag = m?.[1];
    const msg =
      err.code === "ERR_PARSE_ARGS_UNKNOWN_OPTION"
        ? `unknown option ${flag ?? ""} for hibi ${cmd}; run \`hibi ${cmd} --help\``
        : err.code === "ERR_PARSE_ARGS_INVALID_OPTION_VALUE"
          ? `${flag ?? "an option"} needs a value; run \`hibi ${cmd} --help\``
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

  const invalid = validateValues(spec, values);
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

  const open = async (): Promise<Engine> => {
    // Only a missing store means "run hibi init"; a version skew, a corrupt
    // record, or a failed upgrade each carry their own actionable message.
    const engine = await Engine.open(loc, { noAst }).catch(async (e) =>
      (await ClaimStore.isInitialized(loc))
        ? fail((e as Error).message, mode)
        : fail("No claim store. Run `hibi init`.", mode),
    );
    if (engine.store.upgradedFrom) {
      process.stderr.write(
        `upgraded claim store from ${engine.store.upgradedFrom} to ${SCHEMA_VERSION} (one-time)\n`,
      );
    }
    return engine;
  };

  // Deprecated aliases map onto `check`.
  let effective = spec;
  if (spec.aliasOf === "check") {
    const replacement =
      cmd === "diff"
        ? "hibi check --since <ref>"
        : values.doc
          ? "hibi check --doc <path>"
          : "hibi check --overview";
    process.stderr.write(
      `hibi ${cmd} is deprecated and will be removed; use \`${replacement}\`.\n`,
    );
    if (cmd === "diff" && !values.since) {
      return fail("diff requires --since <ref>", mode);
    }
    if (cmd === "status" && !values.doc) values.overview = true;
    effective = commandSpec("check") as CommandSpec;
  }

  switch (effective.name) {
    case "init": {
      const engine = await Engine.init(loc);
      const config = await engine.store.config();
      const payload = {
        store: engine.store.dir,
        nonce: config.nonce,
        version: config.version,
      };
      await emit(
        mode,
        envelope("init", payload, "hibi coverage --doc <file>"),
        () => misc.renderInit(payload, style, mode),
      );
      return 0;
    }

    case "record": {
      const engine = await open();
      const ref = await currentRef(anchorRoot);
      const fromFile = str(values["from-file"]);

      if (fromFile !== undefined) {
        const raw =
          fromFile === "-"
            ? await Bun.stdin.text()
            : await Bun.file(absPath(anchorRoot, fromFile)).text();
        let items: unknown;
        try {
          items = JSON.parse(raw);
        } catch (e) {
          return fail(
            `record --from-file: invalid JSON (${(e as Error).message})`,
            mode,
          );
        }
        if (!Array.isArray(items)) {
          return fail(
            "record --from-file expects a JSON array of claim specs",
            mode,
          );
        }
        const calls: RecordCall[] = [];
        for (let i = 0; i < items.length; i++) {
          const item = items[i];
          if (typeof item !== "object" || item === null)
            return fail(`record --from-file: item ${i} is not an object`, mode);
          try {
            calls.push(
              recordCallFromSpec(item as Record<string, unknown>, ref),
            );
          } catch (e) {
            return fail(
              `record --from-file item ${i}: ${(e as Error).message}`,
              mode,
            );
          }
        }
        // All or nothing: roll back everything this batch wrote if an item
        // fails. Records are snapshotted whole, not just by id, because
        // re-recording an existing claim updates it in place.
        const before = {
          assertions: new Map(
            (await engine.store.allAssertions()).map((x) => [x.id, x]),
          ),
          propositions: new Map(
            (await engine.store.allPropositions()).map((x) => [x.id, x]),
          ),
          documents: new Map(
            (await engine.store.allDocuments()).map((x) => [x.id, x]),
          ),
        };
        const rollback = async () => {
          for (const x of await engine.store.allAssertions()) {
            const original = before.assertions.get(x.id);
            if (original === undefined)
              await engine.store.deleteAssertion(x.id);
            else await engine.store.putAssertion(original);
          }
          for (const x of await engine.store.allPropositions()) {
            const original = before.propositions.get(x.id);
            if (original === undefined)
              await engine.store.deleteProposition(x.id);
            else await engine.store.putProposition(original);
          }
          for (const x of await engine.store.allDocuments()) {
            const original = before.documents.get(x.id);
            if (original === undefined) await engine.store.deleteDocument(x.id);
            else await engine.store.putDocument(original);
          }
        };
        const results: Record<string, unknown>[] = [];
        for (const [i, call] of calls.entries()) {
          try {
            results.push(recordSummary(await engine.record(call)));
          } catch (e) {
            await rollback();
            return fail(
              `record --from-file item ${i} (${call.docPath}): ${(e as Error).message}`,
              mode,
            );
          }
        }
        await emit(
          mode,
          envelope(
            "record",
            { batch: true, count: results.length, results },
            "hibi check",
          ),
          () =>
            `recorded ${results.length} claim${results.length === 1 ? "" : "s"} from ${fromFile}\n`,
        );
        return 0;
      }

      let call: RecordCall;
      try {
        if (!values.doc) throw new Error("record requires --doc <path>");
        call = recordCallFromSpec(flagsToSpec(values), ref);
      } catch (e) {
        return fail((e as Error).message, mode);
      }
      try {
        const result = await engine.record(call);
        const next =
          result.existingClaims.length > 0
            ? "this sentence is already claimed; did you mean `hibi reanchor`?"
            : "hibi check";
        const payload = mode.explain
          ? { ...result, claimId: result.assertion.id }
          : recordSummary(result);
        await emit(mode, envelope("record", payload, next), () =>
          misc.renderRecord(result, style, mode),
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "check": {
      const engine = await open();
      let verifierTimeoutMs: number | undefined;
      try {
        verifierTimeoutMs = verifierTimeoutMsOf(values["verifier-timeout"]);
      } catch (e) {
        return fail((e as Error).message, mode);
      }
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
      const report = await engine.check({
        onlyFiles: files,
        doc,
        write: Boolean(values.write),
        failOn: (str(values["fail-on"]) ?? "gating") as FailOn,
        ref: await currentRef(anchorRoot),
        runVerifiers: Boolean(values["run-verifiers"]),
        verifierTimeoutMs,
      });
      const extra: Record<string, unknown> = {};
      if (since !== undefined) {
        extra.since = since;
        extra.changedFiles = files;
      }
      if (doc !== undefined) {
        extra.doc = doc;
        extra.found = report.documents.length > 0;
      }
      const value = projectCheckReport(
        report,
        projection(mode),
        extra,
        await fingerprints(engine, mode),
      );
      await emit(mode, value, async () => {
        if (values.overview) {
          const [assertions, propositions, config] = await Promise.all([
            engine.store.allAssertions(),
            engine.store.allPropositions(),
            engine.store.config(),
          ]);
          return renderOverview({
            report,
            assertions,
            propositions,
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
        if (doc !== undefined && report.documents.length === 0) {
          lead.push(style.dim(`${doc}: not tracked (no claims recorded)`));
        }
        return renderCheck(await checkContext(engine, report, mode, lead));
      });
      return report.exitCode;
    }

    case "list": {
      const engine = await open();
      const result = await engine.list({
        state: (str(values.state) ?? "all") as ListState,
        path: str(values.path),
        ref: await currentRef(anchorRoot),
        hints: mode.hints,
      });
      if (values["ids-only"]) {
        emitIds(result.claims.map((c) => c.claimId));
        return 0;
      }
      await emit(mode, envelope("list", { ...result }), () =>
        misc.renderList(result, style, mode),
      );
      return 0;
    }

    case "coverage": {
      const engine = await open();
      const doc = String(values.doc);
      try {
        const result = await engine.coverage(doc);
        const next =
          result.summary.uncovered > 0
            ? "ground or remove the uncovered regions: `hibi record --from-file -`"
            : "hibi check";
        await emit(
          mode,
          envelope(
            "coverage",
            { doc, summary: result.summary, regions: result.regions },
            next,
          ),
          () => misc.renderCoverage(doc, result, style, mode),
        );
        return values["fail-uncovered"] && result.summary.uncovered > 0 ? 2 : 0;
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
        values.glob,
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
        const code = codeSideOf({
          codeFile: values["code-file"],
          codeQuote: values["code-quote"],
          codeRange: values["code-range"],
          glob: values.glob,
        });
        const result = await engine.reanchor(claimId, {
          doc: str(values.doc),
          docQuote: docSpec?.quote,
          docRange: docSpec?.quote === undefined ? docSpec : undefined,
          code: code.length > 0 ? code : undefined,
          ref: await currentRef(anchorRoot),
          dryRun,
        });
        const payload = {
          id: result.assertion.id,
          doc: result.doc,
          code: result.code,
          before: result.before,
          after: result.after,
          ...(result.warnings.length > 0 ? { warnings: result.warnings } : {}),
          ...(dryRun ? { dryRun: true } : {}),
          ...(mode.explain ? { assertion: result.assertion } : {}),
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
              id: result.assertion.id,
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
          ref: await currentRef(anchorRoot),
          dryRun: Boolean(values["dry-run"]),
        });
        const next = result.dryRun
          ? "re-run without --dry-run to apply"
          : result.misses.length > 0
            ? "reanchor or retire each missed claim, then hibi check"
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

    case "schema": {
      const { SCHEMAS } = await import("../core/model.ts");
      const { PROTOCOL_SCHEMAS } = await import("../resolver/protocol.ts");
      const z = await import("zod");
      const all: Record<string, import("zod").ZodType> = {
        ...SCHEMAS,
        ...PROTOCOL_SCHEMAS,
      };
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
        out(
          z.toJSONSchema(schema, { target: "draft-2020-12", reused: "inline" }),
          pretty,
        );
      } else {
        out(envelope("schema", { schemas: Object.keys(all) }), pretty);
      }
      return 0;
    }

    case "completions": {
      const shell = positionals[0];
      if (!isShell(shell)) {
        return fail(`completions requires zsh | bash | fish`, mode);
      }
      process.stdout.write(completionScript(shell));
      return 0;
    }

    case "version": {
      await emit(
        mode,
        { name: "hibi", version: pkg.version, schemaVersion: SCHEMA_VERSION },
        () => misc.renderVersion(pkg.version, style),
      );
      return 0;
    }

    default:
      return fail(`unknown command: ${cmd}`, mode);
  }
}

main(process.argv.slice(2))
  .then((code) => process.exit(code))
  .catch((e) => {
    process.stderr.write(`error: ${String(e?.message ?? e)}\n`);
    process.exit(EXIT_OPERATIONAL_ERROR);
  });
