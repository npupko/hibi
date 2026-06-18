#!/usr/bin/env bun
import { access, readFile } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
/**
 * The Hibi CLI (§9) — JSON-first, quiet by default; the consumer is a
 * machine. Verbs: init · record · check · query · diff · supersede · retract ·
 * status · schema. Exit codes follow the §9 contract.
 */
import { parseArgs } from "node:util";
import type { AstAnalyzer } from "../algo/resolve.ts";
import type { AuthoredTrust, Region } from "../core/model.ts";
import type { AnchorAnalyzer } from "../engine/anchor.ts";
import { archiveDocument } from "../engine/archive.ts";
import { type FailOn, runCheck } from "../engine/check.ts";
import { queryByPath } from "../engine/query.ts";
import {
  documentIdForPath,
  recordClaim,
  resolveRegion,
} from "../engine/record.ts";
import { retract, supersede } from "../engine/supersede.ts";
import { blameAuthor, changedFiles, currentRef } from "../git/git.ts";
import { DriftResolver, ResolverRegistry } from "../resolver/registry.ts";
import { ClaimStore } from "../store/store.ts";

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

async function exists(p: string): Promise<boolean> {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}

/** Lazily load the tree-sitter analyzer (Tier-2). Tier-1 works without it. */
let analyzerPromise:
  | Promise<(AstAnalyzer & AnchorAnalyzer) | undefined>
  | undefined;
async function loadAnalyzer(): Promise<
  (AstAnalyzer & AnchorAnalyzer) | undefined
> {
  if (!analyzerPromise) {
    analyzerPromise = import("../ast/analyzer.ts")
      .then((m) => m.getAnalyzer())
      .catch(() => undefined);
  }
  return analyzerPromise;
}

/** Build the resolver registry: built-in drift + manifest-gated externals (§7). */
async function buildRegistry(
  root: string,
  analyzer: AstAnalyzer | undefined,
): Promise<ResolverRegistry> {
  const registry = new ResolverRegistry();
  registry.register(new DriftResolver(analyzer));
  await registry.loadFromManifest(root);
  return registry;
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
  const root = (values.cwd as string) ?? process.cwd();
  const analyzer = values["no-ast"] ? undefined : await loadAnalyzer();

  switch (cmd) {
    case "init": {
      const store = await ClaimStore.init(root);
      const config = await store.config();
      out(
        {
          ok: true,
          action: "init",
          store: store.dir,
          nonce: config.nonce,
          version: config.version,
        },
        pretty,
      );
      return 0;
    }

    case "record": {
      const store = await ClaimStore.open(root).catch(() =>
        fail("No claim store. Run `hibi init`.", pretty),
      );
      if (!values.doc || !values.text)
        return fail("record requires --doc and --text", pretty);
      const trust = String(values.trust) as AuthoredTrust;
      const coarse = Boolean(values.coarse);
      const codeFile = (values.file as string) ?? "";
      let codeContent: string | null = null;
      let region: Region | undefined;
      if (!coarse) {
        if (!codeFile)
          return fail("record requires --file (or --coarse)", pretty);
        const abs = absPath(root, codeFile);
        if (!(await exists(abs)))
          return fail(`Code file not found: ${codeFile}`, pretty);
        codeContent = await readFile(abs, "utf8");
        try {
          region = resolveRegion(codeContent, {
            quote: values.quote as string | undefined,
            start:
              values.start !== undefined ? Number(values.start) : undefined,
            end: values.end !== undefined ? Number(values.end) : undefined,
            line: values.line !== undefined ? Number(values.line) : undefined,
          });
        } catch (e) {
          return fail((e as Error).message, pretty);
        }
      }
      const ref = (values.ref as string) ?? (await currentRef(root));
      // Owner: explicit, else advisory git-blame of the anchored line, else unknown.
      let owner = values.owner as string | undefined;
      if (!owner && region && codeContent) {
        const line = codeContent.slice(0, region.start).split("\n").length;
        owner = (await blameAuthor(codeFile, line, root)) ?? undefined;
      }
      try {
        const result = await recordClaim(store, codeContent, {
          docPath: values.doc as string,
          text: values.text as string,
          authoredTrust: trust,
          owner: owner ?? "unknown",
          ref,
          ttl: values.ttl as string | undefined,
          codeFile: coarse ? codeFile || (values.doc as string) : codeFile,
          region,
          coarse,
          analyzer: analyzer ?? undefined,
        });
        out({ ok: true, action: "record", ...result }, pretty);
        return 0;
      } catch (e) {
        return fail((e as Error).message, pretty);
      }
    }

    case "check": {
      const store = await ClaimStore.open(root).catch(() =>
        fail("No claim store. Run `hibi init`.", pretty),
      );
      const registry = await buildRegistry(root, analyzer);
      const report = await runCheck(store, {
        registry,
        write: Boolean(values.write),
        failOn: String(values["fail-on"]) as FailOn,
        ref: await currentRef(root),
      });
      registry.dispose();
      out({ ok: true, action: "check", ...report }, pretty);
      return report.exitCode;
    }

    case "status": {
      const store = await ClaimStore.open(root).catch(() =>
        fail("No claim store. Run `hibi init`.", pretty),
      );
      if (!values.doc) return fail("status requires --doc", pretty);
      const docId = documentIdForPath(values.doc as string);
      const doc = await store.getDocument(docId);
      const statusRegistry = await buildRegistry(root, analyzer);
      const report = await runCheck(store, {
        registry: statusRegistry,
        write: false,
        ref: await currentRef(root),
      });
      statusRegistry.dispose();
      const docReport = report.documents.find((d) => d.id === docId);
      const verdicts = report.verdicts.filter((v) => v.documentId === docId);
      out(
        {
          ok: true,
          action: "status",
          doc: values.doc,
          found: Boolean(doc),
          lifecycle: doc?.lifecycle ?? null,
          current: !docReport || docReport.suspect.length === 0,
          suspect: docReport?.suspect ?? [],
          verdicts,
        },
        pretty,
      );
      // Read-time gate: non-zero when this doc is suspect.
      return docReport && docReport.suspect.length > 0 ? 2 : 0;
    }

    case "query": {
      const store = await ClaimStore.open(root).catch(() =>
        fail("No claim store. Run `hibi init`.", pretty),
      );
      if (!values.path) return fail("query requires --path", pretty);
      const hits = await queryByPath(store, values.path as string);
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
      const store = await ClaimStore.open(root).catch(() =>
        fail("No claim store. Run `hibi init`.", pretty),
      );
      if (!values.since) return fail("diff requires --since <ref>", pretty);
      const files = await changedFiles(values.since as string, root);
      const diffRegistry = await buildRegistry(root, analyzer);
      const report = await runCheck(store, {
        registry: diffRegistry,
        write: Boolean(values.write),
        onlyFiles: new Set(files),
        failOn: String(values["fail-on"]) as FailOn,
        ref: await currentRef(root),
      });
      diffRegistry.dispose();
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
      const store = await ClaimStore.open(root).catch(() =>
        fail("No claim store. Run `hibi init`.", pretty),
      );
      if (!values.new || !values.old || !values.type) {
        return fail(
          "supersede requires --new, --old, and --type (supersedes|amends)",
          pretty,
        );
      }
      try {
        const result = await supersede(store, {
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
      const store = await ClaimStore.open(root).catch(() =>
        fail("No claim store. Run `hibi init`.", pretty),
      );
      if (!values.doc) return fail("retract requires --doc", pretty);
      const doc = await retract(store, values.doc as string);
      out({ ok: true, action: "retract", document: doc }, pretty);
      return 0;
    }

    case "archive": {
      const store = await ClaimStore.open(root).catch(() =>
        fail("No claim store. Run `hibi init`.", pretty),
      );
      if (!values.doc) return fail("archive requires --doc", pretty);
      const result = await archiveDocument(
        store,
        root,
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
      out({ name: "hibi", version: "0.1.0" }, pretty);
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

function absPath(root: string, p: string): string {
  return isAbsolute(p) ? p : join(root, p);
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
Options: --pretty (human output) · --cwd <dir> · --no-ast (skip tree-sitter)
`;

main(process.argv.slice(2))
  .then((code) => process.exit(code))
  .catch((e) => {
    process.stderr.write(`${String(e?.stack ?? e)}\n`);
    process.exit(EXIT_OPERATIONAL_ERROR);
  });
