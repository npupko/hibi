#!/usr/bin/env bun
import { readFile } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
/**
 * The Hibi CLI (§9) — JSON-first, quiet by default; the consumer is a machine.
 * Verbs: init · record · check · query · diff · supersede · retract · status ·
 * schema. Exit codes follow the §9 contract.
 *
 * This is a thin *imperative shell*: it parses argv, resolves the environment
 * (git ref / blame / changed-files) into plain values, delegates to the `Engine`
 * facade (the single in-process orchestration layer), and serializes the result.
 * The one exception is `record`, which composes the git-free core (`planRecord` +
 * `recordClaim`) directly so it can blame the resolved line between the two steps.
 */
import { parseArgs } from "node:util";
import pkg from "../../package.json" with { type: "json" };
import type { AuthoredTrust, Region } from "../core/model.ts";
import { planRecord, recordClaim } from "../engine/record.ts";
import { exists } from "../fs.ts";
import { blameAuthor, changedFiles, currentRef } from "../git/git.ts";
import {
  Engine,
  type FailOn,
  loadAnalyzer,
  type StoreLocation,
} from "../index.ts";

const EXIT_OPERATIONAL_ERROR = 1;

function out(value: unknown, pretty: boolean): void {
  process.stdout.write(
    `${JSON.stringify(value, jsonReplacer, pretty ? 2 : 0)}\n`,
  );
}
function jsonReplacer(_k: string, v: unknown) {
  return typeof v === "bigint" ? v.toString() : v;
}
function fail(message: string, pretty: boolean): never {
  out({ ok: false, error: message }, pretty);
  process.exit(EXIT_OPERATIONAL_ERROR);
}

function absPath(root: string, p: string): string {
  return isAbsolute(p) ? p : join(root, p);
}
function num(v: unknown): number | undefined {
  return v !== undefined ? Number(v) : undefined;
}

async function main(argv: string[]): Promise<number> {
  const cmd = argv[0];
  const rest = argv.slice(1);

  const { values } = parseArgs({
    args: rest,
    allowPositionals: true,
    strict: false,
    options: {
      cwd: { type: "string" },
      "store-dir": { type: "string" },
      pretty: { type: "boolean", default: false },
      // record
      doc: { type: "string" },
      text: { type: "string" },
      file: { type: "string" },
      quote: { type: "string" },
      start: { type: "string" },
      end: { type: "string" },
      line: { type: "string" },
      trust: { type: "string", default: "inferred" },
      owner: { type: "string" },
      ref: { type: "string" },
      ttl: { type: "string" },
      coarse: { type: "boolean", default: false },
      // check / status
      write: { type: "boolean", default: false },
      "fail-on": { type: "string", default: "suspect" },
      "no-ast": { type: "boolean", default: false },
      // query / diff
      path: { type: "string" },
      since: { type: "string" },
      // supersede
      new: { type: "string" },
      old: { type: "string" },
      type: { type: "string" },
      propositions: { type: "string" },
      successor: { type: "string" },
      // schema
      name: { type: "string" },
    },
  });

  const pretty = Boolean(values.pretty);
  const anchorRoot = (values.cwd as string) ?? process.cwd();
  const noAst = Boolean(values["no-ast"]);
  // The store defaults to <anchorRoot>/.claims; --store-dir decouples it (§8).
  const storeDir = values["store-dir"] as string | undefined;
  const loc: string | StoreLocation = storeDir
    ? { anchorRoot, storeDir: absPath(anchorRoot, storeDir) }
    : anchorRoot;

  const open = () =>
    Engine.open(loc, { noAst }).catch(() =>
      fail("No claim store. Run `hibi init`.", pretty),
    );

  switch (cmd) {
    case "init": {
      const engine = await Engine.init(loc);
      const config = await engine.store.config();
      out(
        {
          ok: true,
          action: "init",
          store: engine.store.dir,
          nonce: config.nonce,
          version: config.version,
        },
        pretty,
      );
      return 0;
    }

    case "record": {
      const engine = await open();
      if (!values.doc || !values.text)
        return fail("record requires --doc and --text", pretty);
      const trust = String(values.trust) as AuthoredTrust;
      const coarse = Boolean(values.coarse);
      const codeFile = (values.file as string) ?? "";
      let codeContent: string | null = null;
      let region: Region | undefined;
      let owner = values.owner as string | undefined;
      if (!coarse) {
        if (!codeFile)
          return fail("record requires --file (or --coarse)", pretty);
        const abs = absPath(anchorRoot, codeFile);
        if (!(await exists(abs)))
          return fail(`Code file not found: ${codeFile}`, pretty);
        codeContent = await readFile(abs, "utf8");
        let plan: { region: Region; line: number };
        try {
          plan = planRecord(codeContent, {
            quote: values.quote as string | undefined,
            start: num(values.start),
            end: num(values.end),
            line: num(values.line),
          });
        } catch (e) {
          return fail((e as Error).message, pretty);
        }
        region = plan.region;
        // Advisory git attribution of the anchored line (a host concern, §6).
        if (!owner)
          owner =
            (await blameAuthor(codeFile, plan.line, anchorRoot)) ?? undefined;
      }
      const ref = (values.ref as string) ?? (await currentRef(anchorRoot));
      try {
        const result = await recordClaim(engine.store, codeContent, {
          docPath: values.doc as string,
          text: values.text as string,
          authoredTrust: trust,
          owner: owner ?? "unknown",
          ref,
          ttl: values.ttl as string | undefined,
          codeFile: coarse ? codeFile || (values.doc as string) : codeFile,
          region,
          coarse,
          // Tier-1 only when coarse (path anchors ignore the analyzer) or --no-ast.
          analyzer: coarse || noAst ? undefined : await loadAnalyzer(),
        });
        out({ ok: true, action: "record", ...result }, pretty);
        return 0;
      } catch (e) {
        return fail((e as Error).message, pretty);
      }
    }

    case "check": {
      const engine = await open();
      const report = await engine.check({
        write: Boolean(values.write),
        failOn: String(values["fail-on"]) as FailOn,
        ref: await currentRef(anchorRoot),
      });
      out({ ok: true, action: "check", ...report }, pretty);
      return report.exitCode;
    }

    case "status": {
      const engine = await open();
      if (!values.doc) return fail("status requires --doc", pretty);
      const result = await engine.status(values.doc as string, {
        ref: await currentRef(anchorRoot),
      });
      out({ ok: true, action: "status", ...result }, pretty);
      // Read-time gate: non-zero when this doc is suspect.
      return result.suspect.length > 0 ? 2 : 0;
    }

    case "query": {
      const engine = await open();
      if (!values.path) return fail("query requires --path", pretty);
      const hits = await engine.query(values.path as string);
      out(
        {
          ok: true,
          action: "query",
          path: values.path,
          count: hits.length,
          hits,
        },
        pretty,
      );
      return 0;
    }

    case "diff": {
      const engine = await open();
      if (!values.since) return fail("diff requires --since <ref>", pretty);
      const files = await changedFiles(values.since as string, anchorRoot);
      const report = await engine.check({
        onlyFiles: files,
        write: Boolean(values.write),
        failOn: String(values["fail-on"]) as FailOn,
        ref: await currentRef(anchorRoot),
      });
      out(
        {
          ok: true,
          action: "diff",
          since: values.since,
          changedFiles: files,
          ...report,
        },
        pretty,
      );
      return report.exitCode;
    }

    case "supersede": {
      const engine = await open();
      if (!values.new || !values.old || !values.type) {
        return fail(
          "supersede requires --new, --old, and --type (supersedes|amends)",
          pretty,
        );
      }
      try {
        const result = await engine.supersede({
          newDocPath: values.new as string,
          oldDocPath: values.old as string,
          type: String(values.type) as "supersedes" | "amends",
          propositions: values.propositions
            ? String(values.propositions)
                .split(",")
                .map((s) => s.trim())
            : undefined,
        });
        out({ ok: true, action: "supersede", ...result }, pretty);
        return 0;
      } catch (e) {
        return fail((e as Error).message, pretty);
      }
    }

    case "retract": {
      const engine = await open();
      if (!values.doc) return fail("retract requires --doc", pretty);
      const doc = await engine.retract(values.doc as string);
      out({ ok: true, action: "retract", document: doc }, pretty);
      return 0;
    }

    case "archive": {
      const engine = await open();
      if (!values.doc) return fail("archive requires --doc", pretty);
      const result = await engine.archive(
        values.doc as string,
        values.successor as string | undefined,
      );
      out({ ok: true, action: "archive", ...result }, pretty);
      return 0;
    }

    case "schema": {
      const { SCHEMAS } = await import("../core/model.ts");
      const z = await import("zod");
      const name = values.name as string | undefined;
      if (name) {
        const schema = (
          SCHEMAS as Record<string, (typeof SCHEMAS)[keyof typeof SCHEMAS]>
        )[name];
        if (!schema)
          return fail(
            `Unknown schema: ${name}. Known: ${Object.keys(SCHEMAS).join(", ")}`,
            pretty,
          );
        out(
          z.toJSONSchema(schema, { target: "draft-2020-12", reused: "ref" }),
          pretty,
        );
      } else {
        out({ ok: true, schemas: Object.keys(SCHEMAS) }, pretty);
      }
      return 0;
    }

    case "version":
    case "--version": {
      out({ name: "hibi", version: pkg.version }, pretty);
      return 0;
    }

    case undefined:
    case "help":
    case "--help": {
      process.stdout.write(USAGE);
      return cmd === undefined ? EXIT_OPERATIONAL_ERROR : 0;
    }

    default:
      return fail(`Unknown command: ${cmd}. Run \`hibi help\`.`, pretty);
  }
}

const USAGE = `hibi — deterministic doc-staleness tracking (JSON-first)

Usage: hibi <command> [options]

Commands:
  init                              Initialize a claim store (.claims/) with a banner nonce
  record   --doc <p> --text <t> --file <f> (--quote <q>|--start <n> --end <n>|--line <n>)
                                    Record a code-anchored claim
  check    [--write] [--fail-on suspect|moved|tamper|never]
                                    Verify all claims; emit verdicts; exit per contract
  status   --doc <p>                Read-time "is this current?" gate for one document
  query    --path <p>               What claims are anchored to / cover this path?
  diff     --since <ref> [--write]  What did this change invalidate? (write-time loop)
  supersede --new <p> --old <p> --type supersedes|amends [--propositions id,id]
  retract  --doc <p>                Mark a document retracted (author withdrew)
  archive  --doc <p> [--successor <p>]  Move an obsolete doc out of the read path (tombstone)
  schema   [--name <Name>]          Emit generated JSON Schema(s)

Exit codes: 0 clean · 2 suspect (stale/ghost/expired) · 3 moved-only · 1 error
Options: --pretty (human output) · --cwd <dir> (anchor root) · --store-dir <dir> (store
  location, default <anchor>/.claims) · --no-ast (skip tree-sitter)
`;

main(process.argv.slice(2))
  .then((code) => process.exit(code))
  .catch((e) => {
    process.stderr.write(`${String(e?.stack ?? e)}\n`);
    process.exit(EXIT_OPERATIONAL_ERROR);
  });
