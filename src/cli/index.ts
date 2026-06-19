#!/usr/bin/env bun
import { isAbsolute, join } from "node:path";
/**
 * The Hibi CLI (§9) — JSON-first, quiet by default; the consumer is a machine.
 * Verbs: init · record · check · diff · status · query · supersede · retract ·
 * archive · suggest · reanchor · schema · version · help. Exit codes follow the
 * §9 two-axis contract (0 clean · 2 gating · 3 moved/at-risk warning · 1 error).
 *
 * This is a thin *imperative shell*: it parses argv, resolves the environment
 * (git ref / blame / changed-files) into plain values, delegates to the `Engine`
 * facade (the single in-process orchestration layer), and serializes the result.
 * Span-first by design — `record` reads the claim text from the documented span
 * (doc side) and anchors zero or more code-side spans; git stays a host concern.
 */
import { parseArgs } from "node:util";
import pkg from "../../package.json" with { type: "json" };
import type {
  AuthoredTrust,
  ClaimKind,
  Enforcement,
  Verifier,
} from "../core/model.ts";
import { changedFiles, currentRef } from "../git/git.ts";
import {
  Engine,
  type FailOn,
  type RecordCall,
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

/**
 * Parse a span flag pair into a RegionSpec fragment. The doc/code sides share
 * the same three locators: `--*-quote <s>` (literal), `--*-range L42:L44` or
 * `42:44` (1-based line range or char offsets), `--*-line N` (1-based line). The
 * caller passes the already-read flag values; this returns `{quote?, start?,
 * end?, line?}` or undefined when no locator was given (a coarse target).
 */
function spanSpec(
  quote: unknown,
  range: unknown,
  line: unknown,
):
  | {
      quote?: string;
      start?: number;
      end?: number;
      line?: number;
      startLine?: number;
      endLine?: number;
    }
  | undefined {
  if (quote !== undefined) return { quote: String(quote) };
  if (range !== undefined) {
    const raw = String(range).trim();
    const colon = raw.indexOf(":");
    if (colon < 0) {
      throw new Error(
        `--*-range expects start:end (e.g. L42:L44 for lines, or 100:130 for char offsets), got: ${raw}`,
      );
    }
    const aRaw = raw.slice(0, colon);
    const bRaw = raw.slice(colon + 1);
    // The `L` prefix on either bound selects the 1-based line-range form.
    const isLine = /^L/i.test(aRaw) || /^L/i.test(bRaw);
    const a = Number(aRaw.replace(/^L/i, ""));
    const b = Number(bRaw.replace(/^L/i, ""));
    if (!Number.isFinite(a) || !Number.isFinite(b)) {
      throw new Error(`--*-range expects numeric bounds, got: ${raw}`);
    }
    return isLine ? { startLine: a, endLine: b } : { start: a, end: b };
  }
  if (line !== undefined) return { line: num(line) };
  return undefined;
}

/**
 * Parse a repeatable `--verifier kind:ref` flag into Verifiers (§5/§17.6). The
 * value may be a lone string (one verifier) or an array (parseArgs collects
 * repeats). The first `:` splits kind from ref so a ref may itself contain `:`.
 */
function parseVerifiers(raw: unknown): Verifier[] {
  if (raw === undefined) return [];
  const items = Array.isArray(raw) ? raw : [raw];
  return items.map((item) => {
    const s = String(item);
    const idx = s.indexOf(":");
    const kind = (idx >= 0 ? s.slice(0, idx) : s) as Verifier["kind"];
    const ref = idx >= 0 ? s.slice(idx + 1) : "";
    return { kind, ref };
  });
}

async function main(argv: string[]): Promise<number> {
  const cmd = argv[0];
  const rest = argv.slice(1);

  const { values, positionals } = parseArgs({
    args: rest,
    allowPositionals: true,
    strict: false,
    options: {
      cwd: { type: "string" },
      "store-dir": { type: "string" },
      pretty: { type: "boolean", default: false },
      // record / reanchor — doc side (span-first)
      doc: { type: "string" },
      "doc-quote": { type: "string" },
      "doc-range": { type: "string" },
      "doc-line": { type: "string" },
      "inline-id": { type: "string" },
      // record / reanchor — code side
      "code-file": { type: "string" },
      "code-quote": { type: "string" },
      "code-range": { type: "string" },
      "code-line": { type: "string" },
      coarse: { type: "boolean", default: false },
      glob: { type: "string" },
      // record — authored facets
      text: { type: "string" }, // legacy override only
      trust: { type: "string", default: "inferred" },
      enforce: { type: "boolean", default: false },
      enforcement: { type: "string" },
      "claim-kind": { type: "string" },
      verifier: { type: "string", multiple: true },
      owner: { type: "string" },
      ref: { type: "string" },
      ttl: { type: "string" },
      // check / diff / status
      write: { type: "boolean", default: false },
      "fail-on": { type: "string", default: "gating" },
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
      if (!values.doc) return fail("record requires --doc", pretty);

      // Doc side: the documented sentence's span supplies the claim text.
      const docSpec = spanSpec(
        values["doc-quote"],
        values["doc-range"],
        values["doc-line"],
      );
      const legacyText = values.text as string | undefined;
      if (!docSpec && !legacyText)
        return fail(
          "record requires a doc span (--doc-quote/--doc-range/--doc-line) or legacy --text",
          pretty,
        );

      // Code side: zero or more targets. One is enough; coarse/glob are coarse.
      const code: NonNullable<RecordCall["code"]> = [];
      const coarse = Boolean(values.coarse);
      const glob = values.glob as string | undefined;
      const codeFile = values["code-file"] as string | undefined;
      if (glob) {
        code.push({ file: glob, glob });
      } else if (codeFile) {
        const codeSpec = spanSpec(
          values["code-quote"],
          values["code-range"],
          values["code-line"],
        );
        code.push({ file: codeFile, ...codeSpec, coarse });
      } else if (coarse) {
        return fail("--coarse requires --code-file or use --glob", pretty);
      }

      // Enforcement: explicit --enforcement wins; --enforce is shorthand for
      // "enforced"; otherwise the engine derives it from trust + resolution.
      const enforcement = (values.enforcement as string | undefined)
        ? (String(values.enforcement) as Enforcement)
        : values.enforce
          ? ("enforced" as Enforcement)
          : undefined;

      const ref = (values.ref as string) ?? (await currentRef(anchorRoot));
      const call: RecordCall = {
        docPath: values.doc as string,
        docQuote: docSpec?.quote,
        docRange:
          docSpec && docSpec.quote === undefined
            ? {
                start: docSpec.start,
                end: docSpec.end,
                line: docSpec.line,
                startLine: docSpec.startLine,
                endLine: docSpec.endLine,
              }
            : undefined,
        inlineId: values["inline-id"] as string | undefined,
        text: legacyText,
        code,
        authoredTrust: String(values.trust) as AuthoredTrust,
        owner: values.owner as string | undefined,
        ref,
        ttl: values.ttl as string | undefined,
        enforcement,
        claimKind: values["claim-kind"] as ClaimKind | undefined,
        verifiers: parseVerifiers(values.verifier),
      };

      try {
        const result = await engine.record(call);
        out({ ok: true, action: "record", ...result }, pretty);
        return 0;
      } catch (e) {
        // recordClaim throws when an `enforced` outcome can't resolve both
        // sides (§9/§18-B) — surface it as an operational error (exit 1).
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

    case "status": {
      const engine = await open();
      if (!values.doc) return fail("status requires --doc", pretty);
      const result = await engine.status(values.doc as string, {
        ref: await currentRef(anchorRoot),
      });
      out({ ok: true, action: "status", ...result }, pretty);
      // Read-time gate via the report's own verdicts: 2 if any gates, else 3
      // when a moved/at-risk warning is present, else 0. `moved`/`at-risk`
      // never gate (§9/ADR-001) — the suspect status strings carry the warning.
      if (result.verdicts.some((v) => v.gates)) return 2;
      const warn = result.suspect.some(
        (s) =>
          s.status === "code:moved" ||
          s.status === "doc:moved" ||
          s.status === "behavior:at-risk",
      );
      return warn ? 3 : 0;
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

    case "suggest": {
      const engine = await open();
      if (!values.doc) return fail("suggest requires --doc", pretty);
      try {
        const result = await engine.suggest(values.doc as string);
        out(
          {
            ok: true,
            action: "suggest",
            doc: values.doc,
            since: values.since,
            count: result.created.length,
            ...result,
          },
          pretty,
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, pretty);
      }
    }

    case "reanchor": {
      const engine = await open();
      const claimId = positionals[0];
      if (!claimId)
        return fail("reanchor requires a <claim-id> positional", pretty);
      const docSpec = spanSpec(
        values["doc-quote"],
        values["doc-range"],
        values["doc-line"],
      );
      const code: NonNullable<RecordCall["code"]> = [];
      const glob = values.glob as string | undefined;
      const codeFile = values["code-file"] as string | undefined;
      if (glob) {
        code.push({ file: glob, glob });
      } else if (codeFile) {
        const codeSpec = spanSpec(
          values["code-quote"],
          values["code-range"],
          values["code-line"],
        );
        code.push({
          file: codeFile,
          ...codeSpec,
          coarse: Boolean(values.coarse),
        });
      }
      try {
        const result = await engine.reanchor(claimId, {
          docQuote: docSpec?.quote,
          docRange:
            docSpec && docSpec.quote === undefined
              ? { start: docSpec.start, end: docSpec.end, line: docSpec.line }
              : undefined,
          code: code.length > 0 ? code : undefined,
          ref: values.ref as string | undefined,
        });
        out({ ok: true, action: "reanchor", ...result }, pretty);
        return 0;
      } catch (e) {
        return fail((e as Error).message, pretty);
      }
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

const USAGE = `hibi — deterministic doc/code claim tracking (JSON-first)

Usage: hibi <command> [options]

Commands:
  init                              Initialize a claim store (.claims/) with a banner nonce
  record   --doc <p> (--doc-quote <s>|--doc-range L42:L44|--doc-line <n>)
           [--code-file <f> (--code-quote <s>|--code-range L1:L9|--code-line <n>)]
           [--coarse|--glob <g>] [--trust verified|inferred|assumed]
           [--enforce|--enforcement <e>] [--claim-kind <k>] [--verifier kind:ref ...] [--ttl <iso>]
                                    Record a span-first claim (doc span + zero or more code spans)
  check    [--write] [--fail-on gating|warn|tamper|never]
                                    Verify all claims; emit verdicts; exit per contract
  diff     --since <ref> [--write]  What did this change invalidate? (write-time loop)
  status   --doc <p>                Read-time "is this current?" gate for one document
  query    --path <p>               What claims are anchored to / cover this path?
  suggest  --doc <p> [--since <ref>]  Propose anchorable claims from a document (suggested records)
  reanchor <claim-id> [--doc-quote …] [--code-file …]  Re-resolve a claim against current content
  supersede --new <p> --old <p> --type supersedes|amends [--propositions id,id]
  retract  --doc <p>                Mark a document retracted (author withdrew)
  archive  --doc <p> [--successor <p>]  Move an obsolete doc out of the read path (tombstone)
  schema   [--name <Name>]          Emit generated JSON Schema(s)

Exit codes: 0 clean · 2 gating (changed/orphaned/ambiguous/expired/refuted on enforced) · 3 moved/at-risk · 1 error
Options: --pretty (human output) · --cwd <dir> (anchor root) · --store-dir <dir> (store
  location, default <anchor>/.claims) · --no-ast (skip tree-sitter)
`;

main(process.argv.slice(2))
  .then((code) => process.exit(code))
  .catch((e) => {
    process.stderr.write(`${String(e?.stack ?? e)}\n`);
    process.exit(EXIT_OPERATIONAL_ERROR);
  });
