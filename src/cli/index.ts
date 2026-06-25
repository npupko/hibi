#!/usr/bin/env bun
import { isAbsolute, join } from "node:path";
/**
 * The Hibi CLI (§9) — JSON-first, quiet by default; the consumer is a machine.
 * Verbs: init · record · check · diff · status · query · list · supersede ·
 * retract · archive · relocate · doctor · suggest · reanchor · retire · schema ·
 * version · help. Exit codes follow the §9 two-axis contract (0 clean · 2 gating
 * · 3 moved/at-risk warning · 1 error); `doctor` is purely informational (always 0).
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
  Verdict,
  Verifier,
} from "../core/model.ts";
import { changedFiles, currentRef } from "../git/git.ts";
import {
  Engine,
  type FailOn,
  type RecordCall,
  type StoreLocation,
} from "../index.ts";
import { completionScript, isShell } from "./completions.ts";
import { renderCheck } from "./render/check.ts";
import { fileReader } from "./render/helpers.ts";
import {
  type ProjectionOptions,
  projectCheckReport,
  projectVerdict,
  SCHEMA_VERSION,
} from "./render/json.ts";
import * as misc from "./render/misc.ts";
import { type OutputMode, resolveMode } from "./render/mode.ts";
import { renderOverview, renderStatusDetail } from "./render/status.ts";
import { makeStyle } from "./render/style.ts";

const EXIT_OPERATIONAL_ERROR = 1;

function out(value: unknown, pretty: boolean): void {
  process.stdout.write(
    `${JSON.stringify(value, jsonReplacer, pretty ? 2 : 0)}\n`,
  );
}
function jsonReplacer(_k: string, v: unknown) {
  return typeof v === "bigint" ? v.toString() : v;
}

/**
 * `--ids-only` (§9 query/list): a bare, de-duplicated, newline-delimited list of
 * claim ids on stdout — mode-independent, for `for id in $(hibi … --ids-only)`
 * shell loops. Bypasses `emit` entirely (no JSON envelope, no human rendering).
 */
function emitIds(ids: string[]): void {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const id of ids) {
    if (seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  if (out.length > 0) process.stdout.write(`${out.join("\n")}\n`);
}

/** True when the resolved mode wants a human (non-JSON) rendering. */
function isHuman(mode: OutputMode): boolean {
  return mode.kind === "rich" || mode.kind === "compact";
}

/**
 * Emit a command result honoring the resolved mode: a human renderer when the
 * mode is rich/compact (the `human` closure runs only then, so its extra store
 * reads never cost the machine path), else the byte-identical JSON the CLI has
 * always emitted (`--json` ≡ the historical default).
 */
async function emit(
  mode: OutputMode,
  value: unknown,
  human: () => string | Promise<string>,
): Promise<void> {
  if (isHuman(mode)) process.stdout.write(await human());
  else out(value, mode.kind === "json-pretty");
}

/**
 * Build the rich/compact `check`/`diff` render context: join the report's
 * verdicts to their assertions (for `owner`/`ref`/`ttl`) and propositions (for
 * the quoted sentence), and hand the renderer a file reader rooted at the anchor
 * root so it can resolve `path:line` anchors. Only called in human mode.
 */
async function checkContext(
  engine: Engine,
  report: import("../index.ts").CheckReport,
  mode: OutputMode,
  verb: string,
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
    verb,
    lead,
  };
}

/** The verbosity/advice projection axes derived from the resolved mode (§9). */
function projection(mode: OutputMode): ProjectionOptions {
  return { explain: mode.explain, hints: mode.hints };
}

/**
 * `propositionId → fingerprint` for the `--explain` projection (the SARIF-style
 * stable fingerprint). Empty (undefined) on the concise path so the hot path
 * never pays the extra store read.
 */
async function fingerprints(
  engine: Engine,
  mode: OutputMode,
): Promise<Map<string, string> | undefined> {
  if (!mode.explain) return undefined;
  const props = await engine.store.allPropositions();
  return new Map(props.map((p) => [p.id, p.fingerprint]));
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

/**
 * The `docRange` payload shared by the record and reanchor assemblies, so a
 * locator (a line, a line range, or char offsets) can never reach one authoring
 * path but be dropped on another. A quote or coarse span has no range → undefined.
 */
function docRangeOf(
  docSpec: ReturnType<typeof spanSpec>,
): RecordCall["docRange"] {
  return docSpec && docSpec.quote === undefined
    ? {
        start: docSpec.start,
        end: docSpec.end,
        line: docSpec.line,
        startLine: docSpec.startLine,
        endLine: docSpec.endLine,
      }
    : undefined;
}

/**
 * Resolve an enforcement override consistently across authoring paths; undefined
 * lets the engine derive enforcement from trust and resolution (§9).
 */
function enforcementOf(
  enforcement: unknown,
  enforce: unknown,
): Enforcement | undefined {
  return enforcement
    ? (String(enforcement) as Enforcement)
    : enforce
      ? ("enforced" as Enforcement)
      : undefined;
}

/**
 * Build a `RecordCall` from a plain spec object — one item of a
 * `record --from-file` batch (§9). Keys mirror the CLI long-flags in camelCase
 * (`doc`, `docQuote`, `codeFile`, `codeQuote`, `trust`, `owner`, …). Reuses
 * `spanSpec`/`parseVerifiers` so a batched item resolves identically to the same
 * claim authored flag-by-flag. Throws on a malformed item so the batch fails
 * loudly rather than recording a half-formed claim.
 */
function recordCallFromSpec(
  spec: Record<string, unknown>,
  fallbackRef: string,
): RecordCall {
  const doc = spec.doc;
  if (typeof doc !== "string" || doc.length === 0)
    throw new Error("each record item needs a `doc` path");

  const docSpec = spanSpec(spec.docQuote, spec.docRange, spec.docLine);
  const legacyText = spec.text as string | undefined;
  // An empty `text` is rejected like a missing one (`!legacyText`): otherwise the
  // item would record a proposition with an empty fingerprint.
  if (!docSpec && !legacyText)
    throw new Error(
      `record item for ${doc} needs a doc span (docQuote/docRange/docLine) or text`,
    );

  const code: NonNullable<RecordCall["code"]> = [];
  const coarse = Boolean(spec.coarse);
  const glob = spec.glob as string | undefined;
  const codeFile = spec.codeFile as string | undefined;
  if (glob) {
    code.push({ file: glob, glob });
  } else if (codeFile) {
    const codeSpec = spanSpec(spec.codeQuote, spec.codeRange, spec.codeLine);
    code.push({ file: codeFile, ...codeSpec, coarse });
  } else if (coarse) {
    throw new Error(
      `record item for ${doc}: \`coarse\` requires \`codeFile\` or \`glob\``,
    );
  }

  const enforcement = enforcementOf(spec.enforcement, spec.enforce);

  return {
    docPath: doc,
    docQuote: docSpec?.quote,
    docRange: docRangeOf(docSpec),
    inlineId: spec.inlineId as string | undefined,
    text: legacyText,
    code,
    // Mirror the single-flag default (`inferred`); a batch never silently mints
    // `verified` (which requires deliberate evidence) on the author's behalf.
    authoredTrust: (spec.trust
      ? String(spec.trust)
      : "inferred") as AuthoredTrust,
    owner: spec.owner as string | undefined,
    ref: (spec.ref as string | undefined) ?? fallbackRef,
    ttl: spec.ttl as string | undefined,
    enforcement,
    claimKind: spec.claimKind as ClaimKind | undefined,
    verifiers: parseVerifiers(spec.verifier),
  };
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
      // Output-mode vocabulary (§9): default auto (human on a TTY, else JSON);
      // --json forces compact JSON; --json --pretty indents it; --pretty forces
      // the rich human view even when piped; --compact is one line per claim.
      pretty: { type: "boolean", default: false },
      json: { type: "boolean", default: false },
      compact: { type: "boolean", default: false },
      color: { type: "string" },
      simple: { type: "boolean", default: false },
      // Verbosity + advice (§9): --explain (alias --detailed) adds the evidence
      // tail; --no-hints (or HIBI_ADVICE=0) drops the remediation menu.
      explain: { type: "boolean", default: false },
      detailed: { type: "boolean", default: false },
      "no-hints": { type: "boolean", default: false },
      // list / query
      state: { type: "string" },
      "ids-only": { type: "boolean", default: false },
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
      // record — batch authoring (a JSON array of claim specs; `-` = stdin)
      "from-file": { type: "string" },
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
      // relocate (batch re-home of stranded claims; neither collides with from-file)
      from: { type: "string" },
      to: { type: "string" },
      // lifecycle / reanchor / relocate — preview without writing
      "dry-run": { type: "boolean", default: false },
      // schema
      name: { type: "string" },
    },
  });

  // Resolve the output mode once, from flags + TTY + env (§9). Machines pipe
  // (non-TTY) → compact JSON, byte-identical to the historical default; only an
  // interactive human (or `--pretty`) gets the rich rendering.
  const mode = resolveMode(
    {
      json: Boolean(values.json),
      pretty: Boolean(values.pretty),
      compact: Boolean(values.compact),
      color: values.color as string | undefined,
      simple: Boolean(values.simple),
      explain: Boolean(values.explain) || Boolean(values.detailed),
      noHints: Boolean(values["no-hints"]),
    },
    { isTTY: process.stdout.isTTY, env: process.env },
  );
  const style = makeStyle(mode.color);
  const anchorRoot = (values.cwd as string) ?? process.cwd();
  const noAst = Boolean(values["no-ast"]);
  // The store defaults to <anchorRoot>/.claims; --store-dir decouples it (§8).
  const storeDir = values["store-dir"] as string | undefined;
  const loc: string | StoreLocation = storeDir
    ? { anchorRoot, storeDir: absPath(anchorRoot, storeDir) }
    : anchorRoot;

  const open = () =>
    Engine.open(loc, { noAst }).catch(() =>
      fail("No claim store. Run `hibi init`.", mode),
    );

  switch (cmd) {
    case "init": {
      const engine = await Engine.init(loc);
      const config = await engine.store.config();
      const value = {
        ok: true,
        action: "init",
        schemaVersion: SCHEMA_VERSION,
        store: engine.store.dir,
        nonce: config.nonce,
        version: config.version,
        next: "hibi suggest --doc <file>",
      };
      await emit(mode, value, () =>
        misc.renderInit(
          { store: value.store, nonce: value.nonce, version: value.version },
          style,
          mode,
        ),
      );
      return 0;
    }

    case "record": {
      const engine = await open();

      // Batch authoring (§9): `--from-file <p|->` reads a JSON array of claim
      // specs and records them in one pass — no shell-quoting of verbatim spans,
      // no per-claim process spawn. The lowest-friction path for an agent
      // grounding many docs at once. `-` reads the array from stdin.
      const fromFile = values["from-file"] as string | undefined;
      if (fromFile !== undefined) {
        const raw =
          fromFile === "-"
            ? await Bun.stdin.text()
            : await Bun.file(
                isAbsolute(fromFile) ? fromFile : join(anchorRoot, fromFile),
              ).text();
        let items: unknown;
        try {
          items = JSON.parse(raw);
        } catch (e) {
          return fail(
            `record --from-file: invalid JSON (${(e as Error).message})`,
            mode,
          );
        }
        if (!Array.isArray(items))
          return fail(
            "record --from-file expects a JSON array of claim specs",
            mode,
          );
        const fallbackRef =
          (values.ref as string) ?? (await currentRef(anchorRoot));
        // Phase 1 — validate every spec before any write, so a malformed item
        // fails the whole batch loudly rather than leaving a partial store.
        const calls: RecordCall[] = [];
        for (let i = 0; i < items.length; i++) {
          const item = items[i];
          if (typeof item !== "object" || item === null)
            return fail(`record --from-file: item ${i} is not an object`, mode);
          try {
            calls.push(
              recordCallFromSpec(item as Record<string, unknown>, fallbackRef),
            );
          } catch (e) {
            return fail(
              `record --from-file item ${i}: ${(e as Error).message}`,
              mode,
            );
          }
        }
        // Phase 2 — record. Serial by design: a shared one-file-per-claim store
        // is not written concurrently (§6). Phase 1 validated structure only; a
        // resolution failure (a quote absent from its file, an `enforced` claim
        // that won't resolve) surfaces here, mid-write. To keep the all-or-nothing
        // guarantee, roll back every record this batch introduced if any item throws.
        const snapshot = async () => ({
          assertions: new Set(
            (await engine.store.allAssertions()).map((x) => x.id),
          ),
          propositions: new Set(
            (await engine.store.allPropositions()).map((x) => x.id),
          ),
          documents: new Set(
            (await engine.store.allDocuments()).map((x) => x.id),
          ),
        });
        const before = await snapshot();
        const rollback = async () => {
          for (const x of await engine.store.allAssertions())
            if (!before.assertions.has(x.id))
              await engine.store.deleteAssertion(x.id);
          for (const x of await engine.store.allPropositions())
            if (!before.propositions.has(x.id))
              await engine.store.deleteProposition(x.id);
          for (const x of await engine.store.allDocuments())
            if (!before.documents.has(x.id))
              await engine.store.deleteDocument(x.id);
        };
        const results: { claimId: string; dedupedProposition: boolean }[] = [];
        for (const [i, call] of calls.entries()) {
          try {
            const result = await engine.record(call);
            results.push({
              claimId: result.assertion.id,
              dedupedProposition: result.dedupedProposition,
            });
          } catch (e) {
            await rollback();
            return fail(
              `record --from-file item ${i} (${call.docPath}): ${(e as Error).message}`,
              mode,
            );
          }
        }
        const value = {
          ok: true,
          action: "record",
          schemaVersion: SCHEMA_VERSION,
          batch: true,
          count: results.length,
          results,
          next: "hibi check",
        };
        await emit(
          mode,
          value,
          () =>
            `recorded ${results.length} claim${results.length === 1 ? "" : "s"} from ${fromFile}\n`,
        );
        return 0;
      }

      if (!values.doc) return fail("record requires --doc", mode);

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
          mode,
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
        return fail("--coarse requires --code-file or use --glob", mode);
      }

      // Enforcement: explicit --enforcement wins; --enforce is shorthand for
      // "enforced"; otherwise the engine derives it from trust + resolution.
      const enforcement = enforcementOf(values.enforcement, values.enforce);

      const ref = (values.ref as string) ?? (await currentRef(anchorRoot));
      const call: RecordCall = {
        docPath: values.doc as string,
        docQuote: docSpec?.quote,
        docRange: docRangeOf(docSpec),
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
        // A `suggested` claim is advisory — it never gates. Surface that, plus a
        // duplicate-proposition hint when the same proposition is already claimed.
        const suggestedWarning =
          result.assertion.enforcement === "suggested"
            ? "recorded as suggested — won't gate the build; pass --enforce to make it gating"
            : undefined;
        const value = {
          ok: true,
          action: "record",
          schemaVersion: SCHEMA_VERSION,
          ...result,
          claimId: result.assertion.id,
          ...(suggestedWarning ? { warning: suggestedWarning } : {}),
          next:
            result.existingClaims.length > 0
              ? "this proposition is already claimed — did you mean `hibi reanchor`?"
              : "hibi check",
        };
        await emit(mode, value, () =>
          misc.renderRecord(result, String(values.trust), style, mode),
        );
        return 0;
      } catch (e) {
        // recordClaim throws when an `enforced` outcome can't resolve both
        // sides (§9/§18-B) — surface it as an operational error (exit 1).
        return fail((e as Error).message, mode);
      }
    }

    case "check": {
      const engine = await open();
      const report = await engine.check({
        write: Boolean(values.write),
        failOn: String(values["fail-on"]) as FailOn,
        ref: await currentRef(anchorRoot),
      });
      const value = projectCheckReport(
        "check",
        report,
        projection(mode),
        undefined,
        await fingerprints(engine, mode),
      );
      await emit(mode, value, async () =>
        renderCheck(await checkContext(engine, report, mode, "check")),
      );
      return report.exitCode;
    }

    case "diff": {
      const engine = await open();
      if (!values.since) return fail("diff requires --since <ref>", mode);
      const files = await changedFiles(values.since as string, anchorRoot);
      const report = await engine.check({
        onlyFiles: files,
        write: Boolean(values.write),
        failOn: String(values["fail-on"]) as FailOn,
        ref: await currentRef(anchorRoot),
      });
      const value = projectCheckReport(
        "diff",
        report,
        projection(mode),
        { since: values.since, changedFiles: files },
        await fingerprints(engine, mode),
      );
      await emit(mode, value, async () => {
        const lead = [
          `${style.dim("since")} ${style.bold(String(values.since))}  ${style.dim(`${files.length} changed file${files.length === 1 ? "" : "s"}`)}`,
        ];
        return renderCheck(
          await checkContext(engine, report, mode, "diff", lead),
        );
      });
      return report.exitCode;
    }

    case "status": {
      const engine = await open();
      // No `--doc` → the repo-wide overview: a full check rendered as a table.
      if (!values.doc) {
        const report = await engine.check({
          ref: await currentRef(anchorRoot),
        });
        const value = projectCheckReport(
          "status",
          report,
          projection(mode),
          undefined,
          await fingerprints(engine, mode),
        );
        await emit(mode, value, async () =>
          renderOverview({
            report,
            assertions: await engine.store.allAssertions(),
            style,
            mode,
          }),
        );
        return report.exitCode;
      }
      const result = await engine.status(values.doc as string, {
        ref: await currentRef(anchorRoot),
      });
      const fps = await fingerprints(engine, mode);
      const value = {
        ok: true,
        action: "status",
        schemaVersion: SCHEMA_VERSION,
        doc: result.doc,
        found: result.found,
        lifecycle: result.lifecycle,
        current: result.current,
        suspect: result.suspect,
        verdicts: result.verdicts.map((v: Verdict) =>
          projectVerdict(v, projection(mode), fps),
        ),
      };
      await emit(mode, value, () =>
        renderStatusDetail({ result, style, mode }),
      );
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
      if (!values.path) return fail("query requires --path", mode);
      const hits = await engine.query(values.path as string);
      if (values["ids-only"]) {
        emitIds(hits.map((h) => h.assertion.id));
        return 0;
      }
      const value = {
        ok: true,
        action: "query",
        schemaVersion: SCHEMA_VERSION,
        path: values.path,
        count: hits.length,
        hits,
      };
      await emit(mode, value, () =>
        misc.renderQuery(String(values.path), hits, style, mode),
      );
      return 0;
    }

    case "suggest": {
      const engine = await open();
      if (!values.doc) return fail("suggest requires --doc", mode);
      try {
        const result = await engine.suggest(values.doc as string);
        const value = {
          ok: true,
          action: "suggest",
          schemaVersion: SCHEMA_VERSION,
          doc: values.doc,
          since: values.since,
          count: result.created.length,
          ...result,
          next: "hibi check",
        };
        await emit(mode, value, () =>
          misc.renderSuggest(String(values.doc), result.created, style, mode),
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "reanchor": {
      const engine = await open();
      const claimId = positionals[0];
      if (!claimId)
        return fail("reanchor requires a <claim-id> positional", mode);
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
      const dryRun = Boolean(values["dry-run"]);
      try {
        const result = await engine.reanchor(claimId, {
          doc: values.doc as string | undefined,
          docQuote: docSpec?.quote,
          docRange: docRangeOf(docSpec),
          code: code.length > 0 ? code : undefined,
          ref: values.ref as string | undefined,
          dryRun,
        });
        const value = {
          ok: true,
          action: "reanchor",
          schemaVersion: SCHEMA_VERSION,
          ...result,
          claimId: result.assertion.id,
          ...(dryRun
            ? { dryRun: true, next: "re-run without --dry-run to apply" }
            : { next: "hibi check" }),
        };
        await emit(mode, value, () =>
          misc.renderReanchor(result, style, mode, dryRun),
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "supersede": {
      const engine = await open();
      if (!values.new || !values.old || !values.type) {
        return fail(
          "supersede requires --new, --old, and --type (supersedes|amends)",
          mode,
        );
      }
      const dryRun = Boolean(values["dry-run"]);
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
          dryRun,
        });
        // Stranded live claims still anchor the old doc — point at relocation
        // rather than letting them quietly rot (Tier-1 silent-orphan hardening).
        const stranded = result.strandedClaims.length > 0;
        const value = {
          ok: true,
          action: "supersede",
          schemaVersion: SCHEMA_VERSION,
          ...result,
          ...(dryRun ? { dryRun: true } : {}),
          next: dryRun
            ? "re-run without --dry-run to apply"
            : stranded
              ? misc.supersedeRelocateHint(
                  String(values.old),
                  String(values.new),
                )
              : "hibi check",
        };
        await emit(mode, value, () =>
          misc.renderSupersede(
            result,
            String(values.type),
            style,
            mode,
            dryRun,
          ),
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "retract": {
      const engine = await open();
      if (!values.doc) return fail("retract requires --doc", mode);
      const dryRun = Boolean(values["dry-run"]);
      const result = await engine.retract(values.doc as string, { dryRun });
      const stranded = result.strandedClaims.length > 0;
      const value = {
        ok: true,
        action: "retract",
        schemaVersion: SCHEMA_VERSION,
        ...result,
        ...(dryRun ? { dryRun: true } : {}),
        // A retract has no successor doc, so the relocation target is the
        // author's call — offer the shape and the retire alternative.
        next: dryRun
          ? "re-run without --dry-run to apply"
          : stranded
            ? misc.retractRelocateHint(String(values.doc))
            : "hibi check",
      };
      await emit(mode, value, () =>
        misc.renderRetract(result, style, mode, dryRun),
      );
      return 0;
    }

    case "archive": {
      const engine = await open();
      if (!values.doc) return fail("archive requires --doc", mode);
      const dryRun = Boolean(values["dry-run"]);
      const result = await engine.archive(
        values.doc as string,
        values.successor as string | undefined,
        { dryRun },
      );
      const stranded = result.strandedClaims.length > 0;
      const value = {
        ok: true,
        action: "archive",
        schemaVersion: SCHEMA_VERSION,
        ...result,
        ...(dryRun ? { dryRun: true } : {}),
        next: dryRun
          ? "re-run without --dry-run to apply"
          : stranded
            ? misc.archiveRelocateHint(
                String(values.doc),
                values.successor as string | undefined,
              )
            : "hibi check",
      };
      await emit(mode, value, () =>
        misc.renderArchive(result, style, mode, dryRun),
      );
      return 0;
    }

    case "relocate": {
      const engine = await open();
      if (!values.from || !values.to) {
        return fail(
          "relocate requires --from <oldDoc> and --to <newDoc>",
          mode,
        );
      }
      const dryRun = Boolean(values["dry-run"]);
      try {
        const result = await engine.relocate(
          values.from as string,
          values.to as string,
          { dryRun, ref: values.ref as string | undefined },
        );
        const value = {
          ok: true,
          action: "relocate",
          schemaVersion: SCHEMA_VERSION,
          ...result,
          next: dryRun ? "re-run without --dry-run to apply" : "hibi check",
        };
        await emit(mode, value, () => misc.renderRelocate(result, style, mode));
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "doctor": {
      const engine = await open();
      const report = await engine.doctor({
        ref: await currentRef(anchorRoot),
      });
      // Route the `next` hint to the most pressing non-empty category. The
      // relocate hint reuses the single-sourced builder so the flag names can
      // never drift from the lifecycle ops' hints. Duplicate propositions are
      // collapsed by *retiring* the redundant claim (reanchor recomputes the same
      // fingerprint, so it does not merge them).
      const next =
        report.counts.orphanedAnchors > 0
          ? "hibi list --state orphaned"
          : report.counts.staleDocClaims > 0
            ? misc.supersedeRelocateHint("<oldDoc>", "<newDoc>")
            : report.counts.suggestedNoCode > 0
              ? "hibi list --state suggested"
              : report.counts.duplicatePropositions > 0
                ? "hibi retire <id>  # drop a duplicate-proposition claim"
                : "store is healthy";
      const value = {
        ok: true,
        action: "doctor",
        schemaVersion: SCHEMA_VERSION,
        ...report,
        next,
      };
      await emit(mode, value, () => misc.renderDoctor(report, style, mode));
      // Purely informational — `doctor` never gates (locked user decision).
      return 0;
    }

    case "retire": {
      const engine = await open();
      const claimId = positionals[0];
      if (!claimId)
        return fail("retire requires a <claim-id> positional", mode);
      const dryRun = Boolean(values["dry-run"]);
      try {
        const result = await engine.retire(claimId, { dryRun });
        const value = {
          ok: true,
          action: "retire",
          schemaVersion: SCHEMA_VERSION,
          ...result,
          claimId: result.assertion.id,
          ...(dryRun
            ? { dryRun: true, next: "re-run without --dry-run to apply" }
            : { next: "hibi check" }),
        };
        await emit(mode, value, () =>
          misc.renderRetire(result, style, mode, dryRun),
        );
        return 0;
      } catch (e) {
        return fail((e as Error).message, mode);
      }
    }

    case "list": {
      const engine = await open();
      // Under parseArgs `strict:false`, `--state` with no value parses as the
      // boolean `true`; catch that distinctly from a wrong value.
      const rawState = values.state;
      const VALID_STATES = [
        "all",
        "gating",
        "warning",
        "clean",
        "orphaned",
        "suggested",
      ] as const;
      if (rawState === true) {
        return fail(
          `list --state expects a value: ${VALID_STATES.join("|")}`,
          mode,
        );
      }
      const state = (rawState as string | undefined) ?? "all";
      if (!(VALID_STATES as readonly string[]).includes(state)) {
        return fail(
          `list --state expects ${VALID_STATES.join("|")} (got: ${state})`,
          mode,
        );
      }
      const result = await engine.list({
        state: state as (typeof VALID_STATES)[number],
        ref: await currentRef(anchorRoot),
        hints: mode.hints,
      });
      if (values["ids-only"]) {
        emitIds(result.claims.map((c) => c.claimId));
        return 0;
      }
      const value = {
        ok: true,
        action: "list",
        schemaVersion: SCHEMA_VERSION,
        ...result,
      };
      await emit(mode, value, () => misc.renderList(result, style, mode));
      return 0;
    }

    case "schema": {
      const { SCHEMAS } = await import("../core/model.ts");
      const z = await import("zod");
      const name = values.name as string | undefined;
      // `schema` is machine output by definition — it stays JSON in every mode,
      // so the human paths emit the same indented (rich) / compact JSON a human
      // reading a schema actually wants, never a prose rendering.
      const schemaPretty = mode.kind !== "json";
      if (name) {
        const schema = (
          SCHEMAS as Record<string, (typeof SCHEMAS)[keyof typeof SCHEMAS]>
        )[name];
        if (!schema)
          return fail(
            `Unknown schema: ${name}. Known: ${Object.keys(SCHEMAS).join(", ")}`,
            mode,
          );
        out(
          // `reused: "inline"` matches `scripts/gen-schemas.ts` so this output
          // is byte-identical to the committed `schemas/*.json` artifact.
          z.toJSONSchema(schema, { target: "draft-2020-12", reused: "inline" }),
          schemaPretty,
        );
      } else {
        out(
          {
            ok: true,
            schemaVersion: SCHEMA_VERSION,
            schemas: Object.keys(SCHEMAS),
          },
          schemaPretty,
        );
      }
      return 0;
    }

    case "completions": {
      const shell = positionals[0];
      if (!isShell(shell)) {
        return fail(
          `completions requires a shell: zsh | bash | fish (got: ${shell ?? "none"})`,
          mode,
        );
      }
      process.stdout.write(completionScript(shell));
      return 0;
    }

    case "version":
    case "--version": {
      await emit(
        mode,
        { name: "hibi", version: pkg.version, schemaVersion: SCHEMA_VERSION },
        () => misc.renderVersion(pkg.version, style),
      );
      return 0;
    }

    case undefined:
    case "help":
    case "--help": {
      process.stdout.write(USAGE);
      return cmd === undefined ? EXIT_OPERATIONAL_ERROR : 0;
    }

    default:
      return fail(`Unknown command: ${cmd}. Run \`hibi help\`.`, mode);
  }
}

const USAGE = `hibi — deterministic doc/code claim tracking

Usage: hibi <command> [options]

Commands:
  init                              Initialize a claim store (.claims/) with a banner nonce
  record   --doc <p> (--doc-quote <s>|--doc-range L42:L44|--doc-line <n>)
           [--code-file <f> (--code-quote <s>|--code-range L1:L9|--code-line <n>)]
           [--coarse|--glob <g>] [--trust verified|inferred|assumed]
           [--enforce|--enforcement <e>] [--claim-kind <k>] [--verifier kind:ref ...] [--ttl <iso>]
                                    Record a span-first claim (doc span + zero or more code spans)
  record   --from-file <p|->        Batch-record a JSON array of claim specs (- = stdin)
  check    [--write] [--fail-on gating|warn|tamper|never]
                                    Verify all claims; emit verdicts; exit per contract
  diff     --since <ref> [--write]  What did this change invalidate? (write-time loop)
  status   [--doc <p>]              No --doc: repo-wide health overview. --doc: one-document gate.
  query    --path <p>               What claims are anchored to / cover this doc OR code path?
  list     [--state all|gating|warning|clean|orphaned|suggested]  Triage: one lean row per claim
  suggest  --doc <p> [--since <ref>]  Propose anchorable claims from a document (suggested records)
  reanchor <claim-id> [--doc <p>] [--doc-quote …] [--code-file …]  Re-resolve a claim, or relocate
                                    either side to a different file (--doc moves the doc anchor)
  retire   <claim-id>               Withdraw one claim (enforcement → retired); idempotent
  supersede --new <p> --old <p> --type supersedes|amends [--propositions id,id]
  retract  --doc <p>                Mark a document retracted (author withdrew)
  archive  --doc <p> [--successor <p>]  Move an obsolete doc out of the read path (tombstone)
  relocate --from <p> --to <p>      Re-home every live claim stranded on --from to --to (one pass)
  doctor                            Store-health report (orphans, stranded, duplicates); always exit 0
  schema   [--name <Name>]          Emit generated JSON Schema(s)
  completions <zsh|bash|fish>       Print a shell completion script

Output: rich human view on a terminal, compact JSON when piped/redirected/CI.
  --json            force compact JSON (the machine contract; what agents read)
  --json --pretty   indented JSON
  --pretty          force the rich human view, even when piped
  --compact         one line per claim (human)
  --explain         add the full evidence tail to the JSON (alias --detailed)
  --no-hints        drop the remediation menu (also via HIBI_ADVICE=0)
  --color auto|always|never   (also honors NO_COLOR / FORCE_COLOR)
  --simple          ASCII symbols instead of unicode
  --dry-run         preview without writing (reanchor/retire/supersede/relocate)
  --ids-only        emit a bare, newline-delimited claim-id list (query/list)

Examples:
  hibi check                      # rich, grouped-by-document drift report
  hibi check --json | jq .summary # machine JSON for a script or agent
  hibi status                     # repo-wide document health table
  hibi diff --since origin/main   # what did my change invalidate?

Exit codes: 0 clean · 2 gating (changed/orphaned/ambiguous/expired/refuted on enforced) · 3 moved/at-risk · 1 error
Globals: --cwd <dir> (anchor root) · --store-dir <dir> (store location, default
  <anchor>/.claims) · --no-ast (skip tree-sitter)
`;

main(process.argv.slice(2))
  .then((code) => process.exit(code))
  .catch((e) => {
    process.stderr.write(`${String(e?.stack ?? e)}\n`);
    process.exit(EXIT_OPERATIONAL_ERROR);
  });
