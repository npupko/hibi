import { afterEach, describe, expect, test } from "bun:test";
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { parseArgs } from "node:util";
import { COMMANDS, optionsFor, parseArgsOptions } from "../src/cli/options.ts";
import { exists } from "../src/fs.ts";

const CLI = join(import.meta.dir, "..", "src", "cli", "index.ts");

interface RemediationAction {
  id?: string;
  command?: string;
}
interface Remediation {
  recommended?: string | null;
  actions?: RemediationAction[];
}
interface CliVerdict {
  claimId?: string;
  kind?: string;
  doc?: string;
  evidence?: string;
  judge?: string;
  expired?: boolean;
  conflicts?: unknown[];
  gates?: boolean;
  changed?: string;
  remediation?: Remediation | null;
  reasons?: string[];
  detail?: unknown;
}
interface ListRow {
  claimId?: string;
  kind?: string;
  state?: string;
  side?: string;
  sentence?: string;
  duplicates?: { claimId?: string; why?: string }[];
}
interface CliJson {
  ok?: boolean;
  error?: string;
  errors?: string[];
  action?: string;
  schemaVersion?: string;
  next?: string;
  store?: string;
  version?: string;
  id?: string;
  kind?: string;
  doc?: string;
  evidence?: string;
  enforcement?: string;
  warnings?: string[];
  alreadyRetired?: boolean;
  batch?: boolean;
  results?: { id?: string }[];
  before?: { doc?: { quote?: string }; evidence?: { quote?: string }[] };
  after?: { doc?: { quote?: string }; evidence?: { quote?: string }[] };
  summary?: {
    clean?: number;
    total?: number;
    retired?: number;
    warning?: number;
    gating?: number;
    uncovered?: number;
  };
  verdicts?: CliVerdict[];
  changedFiles?: string[];
  count?: number;
  state?: string;
  claims?: ListRow[];
  candidates?: { side?: string; file?: string }[];
  relocated?: { claimId?: string }[];
  kept?: { side?: string; file?: string; state?: string; reason?: string }[];
  misses?: { claimId?: string }[];
  strandedClaims?: string[];
  dryRun?: boolean;
  claim?: unknown;
  type?: string;
  properties?: Record<string, unknown>;
  exitCode?: number;
}

interface RunResult {
  code: number;
  json: CliJson;
  stdout: string;
  stderr: string;
}

/** `record` needs `--by`; tests that are not about it get the repo's git user. */
function withBy(args: string[]): string[] {
  if (args[0] !== "record" || args.includes("--by") || args.includes("--help"))
    return args;
  return [...args, "--by", "human:t@t.co"];
}

async function run(
  cwd: string,
  args: string[],
  stdin?: string,
  { shellPipe = false }: { shellPipe?: boolean } = {},
): Promise<RunResult> {
  const env: Record<string, string | undefined> = { ...process.env };
  delete env.TYPESAFE_API_KEY;
  const cli = ["bun", "run", CLI, ...withBy(args)];
  // `shellPipe` puts an OS pipe between the CLI and its reader, as in `hibi check | jq`.
  const argv = shellPipe
    ? ["bash", "-c", 'set -o pipefail; "$@" | cat', "bash", ...cli]
    : cli;
  const proc = Bun.spawn(argv, {
    cwd,
    env,
    stdout: "pipe",
    stderr: "pipe",
    stdin: stdin === undefined ? "ignore" : new TextEncoder().encode(stdin),
  });
  const stdout = await new Response(proc.stdout).text();
  const stderr = await new Response(proc.stderr).text();
  const code = await proc.exited;
  let json: CliJson = {};
  try {
    json = JSON.parse(stdout.trim().split("\n").at(-1) ?? "");
  } catch {
    /* non-JSON */
  }
  return { code, json, stdout, stderr };
}

let dirs: string[] = [];
async function repo(): Promise<string> {
  const d = await mkdtemp(join(tmpdir(), "ce-cli-"));
  dirs.push(d);
  await Bun.spawn(["git", "init", "-q"], { cwd: d }).exited;
  await Bun.spawn(["git", "config", "user.email", "t@t.co"], { cwd: d }).exited;
  await Bun.spawn(["git", "config", "user.name", "t"], { cwd: d }).exited;
  return d;
}
async function commit(d: string) {
  await Bun.spawn(["git", "add", "-A"], { cwd: d }).exited;
  await Bun.spawn(["git", "commit", "-qm", "c"], { cwd: d }).exited;
}
async function write(root: string, rel: string, content: string) {
  const abs = join(root, rel);
  await mkdir(dirname(abs), { recursive: true });
  await writeFile(abs, content);
}
async function snapshotStore(root: string): Promise<Map<string, string>> {
  const out = new Map<string, string>();
  const walk = async (d: string) => {
    for (const e of await readdir(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) await walk(p);
      else out.set(p, await readFile(p, "utf8"));
    }
  };
  await walk(join(root, ".claims"));
  return out;
}
function sameStore(a: Map<string, string>, b: Map<string, string>): boolean {
  if (a.size !== b.size) return false;
  for (const [k, v] of a) if (b.get(k) !== v) return false;
  return true;
}
afterEach(async () => {
  await Promise.all(dirs.map((d) => rm(d, { recursive: true, force: true })));
  dirs = [];
});

/** Run a remediation `command` through bash, with `hibi` pointing at this CLI. */
async function runMenu(d: string, command: string): Promise<number> {
  const env: Record<string, string | undefined> = { ...process.env };
  delete env.TYPESAFE_API_KEY;
  const line = command.replace(/^hibi /, `bun run ${CLI} `);
  const proc = Bun.spawn(["bash", "-c", `${line} --json`], {
    cwd: d,
    env,
    stdout: "pipe",
    stderr: "pipe",
  });
  await new Response(proc.stdout).text();
  return proc.exited;
}

/** A repo with one enforced claim on MAX_ATTEMPTS. */
async function seeded(): Promise<{ d: string; id: string }> {
  const d = await repo();
  await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
  await write(d, "README.md", "# Doc\n\nRetries are capped at 5 attempts.\n");
  await run(d, ["init"]);
  const rec = await run(d, [
    "record",
    "--kind",
    "code",
    "--doc",
    "README.md",
    "--doc-quote",
    "Retries are capped at 5 attempts",
    "--code-file",
    "src/retry.ts",
    "--code-quote",
    "MAX_ATTEMPTS = 5",
    "--owner",
    "human:alice@example.com",
  ]);
  return { d, id: rec.json.id ?? "" };
}

describe("CLI end-to-end", () => {
  test("init → record → check clean → drift → exit 2 (enforced by default)", async () => {
    const { d } = await seeded();
    const init = await run(d, ["init"]);
    expect(init.code).toBe(0);
    expect(init.json.version).toBe("v4");
    expect(init.json.schemaVersion).toBe("v4");

    const clean = await run(d, ["check"]);
    expect(clean.code).toBe(0);
    expect(clean.json.summary?.clean).toBe(1);

    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const drifted = await run(d, ["check"]);
    expect(drifted.code).toBe(2);
    expect(drifted.json.verdicts?.[0]?.evidence).toBe("changed");
    expect(drifted.json.verdicts?.[0]?.gates).toBe(true);
    expect(drifted.json.verdicts?.[0]?.changed).toContain("value changed");
  });

  test("piped record returns the lean summary; --explain returns the full result", async () => {
    const { d, id } = await seeded();
    expect(id).toMatch(/^clm_[0-9a-f]{16}$/);
    const rec = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "README.md",
      "--doc-quote",
      "Retries are capped at 5 attempts",
      "--code-file",
      "src/retry.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
    ]);
    expect(Object.keys(rec.json).sort()).toEqual(
      [
        "action",
        "doc",
        "enforcement",
        "evidence",
        "existingClaims",
        "id",
        "kind",
        "next",
        "ok",
        "schemaVersion",
        "updated",
      ].sort(),
    );
    expect(rec.json.enforcement).toBe("enforced");
    const full = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "README.md",
      "--doc-quote",
      "Retries are capped at 5 attempts",
      "--code-file",
      "src/retry.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
      "--explain",
    ]);
    expect(full.json.claim).toBeDefined();
  });

  test("record --suggest never gates; the vouch defaults to the git user", async () => {
    const d = await repo();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await write(d, "README.md", "# Doc\n\nRetries are capped at 5 attempts.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "README.md",
      "--doc-quote",
      "Retries are capped at 5 attempts",
      "--code-file",
      "src/retry.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
      "--suggest",
      "--explain",
    ]);
    const claim = rec.json.claim as {
      enforcement?: string;
      attestations?: { by?: string; basis?: string }[];
    };
    expect(claim.enforcement).toBe("suggested");
    expect(claim.attestations).toEqual([
      expect.objectContaining({ by: "human:t@t.co", basis: "vouch" }),
    ]);
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const drifted = await run(d, ["check"]);
    expect(drifted.code).toBe(0);
    expect(drifted.json.verdicts?.[0]?.evidence).toBe("changed");
    expect(drifted.json.verdicts?.[0]?.gates).toBe(false);
  });

  test("record flags: every problem is one line, all reported", async () => {
    const d = await repo();
    await write(d, "doc.md", "# Doc\n\nThe price is $29 per month.\n");
    await run(d, ["init"]);
    const res = await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "The price is $29 per month.",
      "--kind",
      "reference",
      "--source",
      "no-equals-sign",
      "--code-quote",
      "x",
    ]);
    expect(res.code).toBe(1);
    expect(res.json.errors).toEqual([
      "--code-quote needs a --code-file before it",
      '--source expects <id>=<resource>, e.g. stripe=https://dashboard.stripe.com/products/prod_123 (got "no-equals-sign")',
    ]);
    const res2 = await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "The price is $29 per month.",
      "--kind",
      "reference",
    ]);
    expect(res2.json.errors).toEqual([
      "kind reference needs ref: { file, quote | range }",
    ]);
    const mixed = await run(
      d,
      ["record", "--from-file", "-", "--doc", "doc.md"],
      "[]",
    );
    expect(mixed.code).toBe(1);
    expect(mixed.json.error).toContain("drop --doc");
  });

  test("record refuses a sibling-repo alias that is not checked out", async () => {
    const d = await repo();
    await write(d, "doc.md", "# Doc\n\nLimits are 100 requests.\n");
    await run(d, ["init"]);
    const configPath = join(d, ".claims", "config.json");
    const config = JSON.parse(await readFile(configPath, "utf8"));
    await writeFile(
      configPath,
      JSON.stringify({ ...config, repos: { api: "../no-such-sibling" } }),
    );
    const res = await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "Limits are 100 requests.",
      "--kind",
      "code",
      "--code-file",
      "api:src/limits.ts",
      "--code-quote",
      "LIMIT = 100",
    ]);
    expect(res.code).toBe(1);
    expect(res.json.errors?.[0]).toContain(
      'sibling repo "api" is not checked out at ../no-such-sibling',
    );
  });

  test("record reads an aliased evidence file from the sibling repo", async () => {
    const d = await repo();
    const sibling = await mkdtemp(join(tmpdir(), "ce-sibling-"));
    dirs.push(sibling);
    await write(sibling, "src/limits.ts", "export const LIMIT = 100;\n");
    await write(d, "doc.md", "# Doc\n\nLimits are 100 requests.\n");
    await run(d, ["init"]);
    const configPath = join(d, ".claims", "config.json");
    const config = JSON.parse(await readFile(configPath, "utf8"));
    await writeFile(
      configPath,
      JSON.stringify({ ...config, repos: { api: sibling } }),
    );
    const res = await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "Limits are 100 requests.",
      "--kind",
      "code",
      "--code-file",
      "api:src/limits.ts",
      "--code-quote",
      "LIMIT = 100",
    ]);
    expect(res.code).toBe(0);
    expect(res.json.evidence).toBe("api:src/limits.ts");
    const check = await run(d, ["check"]);
    expect(check.code).toBe(0);
    expect(check.json.verdicts?.[0]?.evidence).toBe("unchanged");
  });

  test("record rejects a nonexistent --code-file with exit 1", async () => {
    const d = await repo();
    await write(d, "README.md", "# Doc\n\nRetries are capped at 5 attempts.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "README.md",
      "--doc-quote",
      "Retries are capped at 5 attempts",
      "--code-file",
      "src/nope.ts",
      "--code-quote",
      "x",
    ]);
    expect(rec.code).toBe(1);
    expect(rec.json.error).toContain("not found");
  });

  test("a doc quote that occurs twice returns a warning in the result", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(
      d,
      "README.md",
      "# Doc\n\nThe A constant is one.\n\nSecond section.\n\nThe A constant is one.\n",
    );
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "README.md",
      "--doc-quote",
      "The A constant is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    expect(rec.code).toBe(0);
    expect(rec.json.warnings?.[0]).toContain("occurs 2 times");
  });

  test("record --from-file - reads JSON from stdin, all or nothing", async () => {
    const d = await repo();
    await write(
      d,
      "src/conf.ts",
      "export const TTL_MS = 60000;\nexport const RETRIES = 3;\n",
    );
    await write(
      d,
      "docs/conf.md",
      "# Config\n\nThe cache TTL is 60000ms.\nRetries default to 3.\n",
    );
    await run(d, ["init"]);
    const ok = await run(
      d,
      ["record", "--from-file", "-"],
      JSON.stringify([
        {
          doc: "docs/conf.md",
          docQuote: "The cache TTL is 60000ms",
          kind: "code",
          code: [{ file: "src/conf.ts", quote: "TTL_MS = 60000" }],
          by: "agent:batch",
        },
        {
          doc: "docs/conf.md",
          docQuote: "Retries default to 3",
          kind: "code",
          code: [{ file: "src/conf.ts", range: "L2:L2" }],
        },
      ]),
    );
    expect(ok.code).toBe(0);
    expect(ok.json.batch).toBe(true);
    expect(ok.json.results?.length).toBe(2);
    expect((await run(d, ["check"])).json.summary?.clean).toBe(2);

    const before = await snapshotStore(d);
    const bad = await run(
      d,
      ["record", "--from-file", "-"],
      JSON.stringify([
        {
          doc: "docs/conf.md",
          docQuote: "The cache TTL is 60000ms",
          kind: "code",
          code: [{ file: "src/conf.ts", quote: "TTL_MS = 60000" }],
        },
        {
          doc: "docs/conf.md",
          docQuote: "NOT PRESENT IN THE FILE",
          kind: "code",
          code: [{ file: "src/conf.ts", quote: "RETRIES = 3" }],
        },
        {
          doc: "docs/conf.md",
          docQuote: "Retries default to 3",
          kind: "code",
          code: [{ file: "src/missing.ts", quote: "RETRIES = 3" }],
        },
      ]),
    );
    expect(bad.code).toBe(1);
    expect(bad.json.errors).toHaveLength(2);
    expect(bad.json.errors?.[0]).toStartWith("item 1: ");
    expect(bad.json.errors?.[1]).toStartWith("item 2: ");
    expect(sameStore(before, await snapshotStore(d))).toBe(true);

    // Shape problems are all reported, one line each, before anything is read.
    const invalid = await run(
      d,
      ["record", "--from-file", "-"],
      JSON.stringify([
        { doc: "docs/conf.md", docQuote: "Retries default to 3", ttl: "90d" },
        {
          doc: "docs/conf.md",
          docRange: "L4:L4",
          kind: "attested",
          code: [{ file: "src/conf.ts", quote: "x" }],
          extra: 1,
        },
      ]),
    );
    expect(invalid.code).toBe(1);
    expect(invalid.json.errors).toEqual([
      "item 0: kind must be one of code, reference, attested (got undefined)",
      'item 0: ttl must be an ISO 8601 duration such as P90D (got "90d")',
      'item 1: unknown key "extra"; allowed: doc, docQuote, docRange, kind, code, ref, sources, procedure, ttl, owner, by, suggest',
      "item 1: code is only for kind code (kind is attested)",
      "item 1: kind attested needs a ttl, e.g. P90D",
    ]);
    expect(sameStore(before, await snapshotStore(d))).toBe(true);
  });

  test("moved exits 0 as a warning and 2 under --fail-on warn", async () => {
    const { d } = await seeded();
    await write(
      d,
      "src/retry.ts",
      `${"// prologue\n".repeat(4)}export const MAX_ATTEMPTS = 5;\n`,
    );
    const soft = await run(d, ["check"]);
    expect(soft.code).toBe(0);
    expect(soft.json.verdicts?.[0]?.evidence).toBe("moved");
    expect(soft.json.summary?.warning).toBe(1);
    expect((await run(d, ["check", "--fail-on", "warn"])).code).toBe(2);
    expect((await run(d, ["check", "--fail-on", "never"])).code).toBe(0);
  });

  test("check --since scopes to changed files and rejects an unknown ref", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "src/b.ts", "export const B = 2;\n");
    await write(d, "doc.md", "# Doc\n\nA is 1 and B is 2 here.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "doc.md",
      "--doc-quote",
      "A is 1 and",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "doc.md",
      "--doc-quote",
      "B is 2 here",
      "--code-file",
      "src/b.ts",
      "--code-quote",
      "B = 2",
    ]);
    await commit(d);
    await write(d, "src/a.ts", "export const A = 100;\n");
    const res = await run(d, ["check", "--since", "HEAD"]);
    expect(res.json.changedFiles).toContain("src/a.ts");
    expect(res.json.changedFiles).not.toContain("src/b.ts");
    expect(res.json.verdicts?.length).toBe(1);
    const bad = await run(d, ["check", "--since", "no-such-ref"]);
    expect(bad.code).toBe(1);
    expect(bad.json.error).toContain("unknown git ref");
  });

  test("check --doc scopes to one document and shares the exit-code path", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "src/b.ts", "export const B = 2;\n");
    await write(d, "a.md", "# A\n\nA is 1 here.\n");
    await write(d, "b.md", "# B\n\nB is 2 here.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "a.md",
      "--doc-quote",
      "A is 1 here",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "b.md",
      "--doc-quote",
      "B is 2 here",
      "--code-file",
      "src/b.ts",
      "--code-quote",
      "B = 2",
    ]);
    await write(d, "src/a.ts", "// gone\n");
    const a = await run(d, ["check", "--doc", "a.md"]);
    expect(a.code).toBe(2);
    expect(a.json.doc).toBe("a.md");
    expect(a.json.verdicts?.length).toBe(1);
    // The same document by ./a.md and by its absolute path.
    const dotted = await run(d, ["check", "--doc", "./a.md"]);
    expect(dotted.code).toBe(2);
    expect(dotted.json.doc).toBe("a.md");
    expect(dotted.json.verdicts?.length).toBe(1);
    const absolute = await run(d, ["check", "--doc", join(d, "a.md")]);
    expect(absolute.code).toBe(2);
    expect(absolute.json.verdicts?.length).toBe(1);
    const b = await run(d, ["check", "--doc", "b.md"]);
    expect(b.code).toBe(0);
    expect(b.json.verdicts?.length).toBe(1);
    const never = await run(d, [
      "check",
      "--doc",
      "a.md",
      "--fail-on",
      "never",
    ]);
    expect(never.code).toBe(0);
    // An unregistered (or missing) document is an operational error.
    const untracked = await run(d, ["check", "--doc", "nope.md"]);
    expect(untracked.code).toBe(1);
    expect(untracked.json.ok).toBe(false);
    expect(untracked.json.error).toBe(
      "no claims are registered for nope.md; run hibi audit --doc nope.md",
    );
  });

  test("diff and status are gone", async () => {
    const { d } = await seeded();
    for (const verb of ["diff", "status"]) {
      const res = await run(d, [verb]);
      expect(res.code).toBe(1);
      expect(res.json.error).toContain(`unknown command: ${verb}`);
    }
  });

  test("retired claims are excluded from clean and never gate", async () => {
    const { d, id } = await seeded();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    expect((await run(d, ["check"])).code).toBe(2);
    const first = await run(d, ["retire", id]);
    expect(first.code).toBe(0);
    expect(first.json.alreadyRetired).toBe(false);
    const after = await run(d, ["check"]);
    expect(after.code).toBe(0);
    expect(after.json.summary?.clean).toBe(0);
    expect(after.json.summary?.retired).toBe(1);
    expect((await run(d, ["retire", id])).json.alreadyRetired).toBe(true);
    const row = (await run(d, ["list"])).json.claims?.find(
      (r) => r.claimId === id,
    );
    expect(row?.state).toBe("retired");
  });

  test("list filters by --state and --path, and --ids-only prints bare ids", async () => {
    const { d, id } = await seeded();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const all = await run(d, ["list"]);
    const row = all.json.claims?.[0];
    expect(row?.claimId).toBe(id);
    expect(row?.state).toBe("gating");
    expect(row?.kind).toBe("code");
    expect(row?.sentence).toContain("Retries are capped");
    expect(Object.keys(row ?? {}).sort()).toEqual([
      "claimId",
      "documentId",
      "enforcement",
      "expiresAt",
      "kind",
      "owner",
      "sentence",
      "state",
      "ttl",
    ]);
    expect((await run(d, ["list", "--state", "gating"])).json.count).toBe(1);
    expect((await run(d, ["list", "--state", "clean"])).json.count).toBe(0);
    const byPath = await run(d, ["list", "--path", "src/retry.ts"]);
    expect(byPath.json.count).toBe(1);
    expect(byPath.json.claims?.[0]?.side).toBe("evidence");
    expect(
      (await run(d, ["list", "--path", "README.md"])).json.claims?.[0]?.side,
    ).toBe("doc");
    expect((await run(d, ["list", "--path", "src/other.ts"])).json.count).toBe(
      0,
    );
    const ids = await run(d, ["list", "--ids-only"]);
    expect(ids.stdout.trim()).toBe(id);
    expect((await run(d, ["list", "--state", "bogus"])).code).toBe(1);
    expect((await run(d, ["list", "--state", "orphaned"])).code).toBe(1);
  });

  test("list --state gating drains after retire; duplicate and stranded filters", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "src/gone.ts", "export const X = 1;\n");
    await write(d, "ok.md", "# OK\n\nA is one.\n");
    await write(d, "o.md", "# O\n\nOrphan here.\n");
    await write(d, "dup.md", "# Dup\n\nA is one.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "ok.md",
      "--doc-quote",
      "A is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "dup.md",
      "--doc-quote",
      "A is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const orphan = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "o.md",
      "--doc-quote",
      "Orphan here.",
      "--code-file",
      "src/gone.ts",
      "--code-quote",
      "X = 1",
    ]);
    await rm(join(d, "src/gone.ts"));
    expect(
      (await run(d, ["list", "--state", "gating"])).json.claims?.map(
        (c) => c.claimId,
      ),
    ).toEqual([orphan.json.id ?? ""]);
    // ok.md and dup.md hold the same sentence: a duplicate by text, both ways.
    const dup = await run(d, ["list", "--state", "duplicate"]);
    expect(dup.json.count).toBe(2);
    expect(dup.json.claims?.[0]?.duplicates?.[0]?.why).toBe("same sentence");
    await run(d, ["retire", orphan.json.id ?? ""]);
    expect((await run(d, ["list", "--state", "gating"])).json.count).toBe(0);
    await write(d, "new.md", "# New\n\nSomething else.\n");
    await run(d, ["supersede", "--from", "ok.md", "--to", "new.md"]);
    // supersede moved the ok.md claim; one recorded on ok.md afterwards is stranded there.
    await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "ok.md",
      "--doc-quote",
      "A is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const stranded = await run(d, ["list", "--state", "stranded"]);
    expect(stranded.json.count).toBe(1);
    expect(stranded.json.claims?.[0]?.state).toBe("clean");
  });

  test("a gating verdict carries the remediation menu with commands, --no-hints strips it", async () => {
    const { d, id } = await seeded();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rem = (await run(d, ["check"])).json.verdicts?.[0]?.remediation;
    expect(rem?.recommended).toBe("update-claim");
    expect(rem?.actions?.map((a) => a.id)).toEqual([
      "update-claim",
      "reanchor",
      "retire",
    ]);
    expect(rem?.actions?.[2]?.command).toBe(`hibi retire ${id}`);
    const stripped = await run(d, ["check", "--no-hints"]);
    expect(stripped.json.verdicts?.[0]?.remediation).toBeUndefined();
    expect(stripped.json.verdicts?.[0]?.gates).toBe(true);
    const explained = await run(d, ["check", "--explain"]);
    expect(explained.json.verdicts?.[0]?.detail).toBeDefined();
    expect(
      (await run(d, ["check"])).json.verdicts?.[0]?.detail,
    ).toBeUndefined();
  });

  test("an orphan recommends the read-only --suggest pass", async () => {
    const { d, id } = await seeded();
    await rm(join(d, "src/retry.ts"));
    const v = (await run(d, ["check"])).json.verdicts?.[0];
    expect(v?.evidence).toBe("orphaned");
    expect(v?.remediation?.recommended).toBe("reanchor");
    expect(v?.remediation?.actions?.[0]?.command).toBe(
      `hibi reanchor ${id} --suggest`,
    );
  });

  test("reanchor refuses an orphaned side, accepts an explicit new span, and reports before/after quotes", async () => {
    const { d, id } = await seeded();
    await rm(join(d, "src/retry.ts"));
    const before = await snapshotStore(d);
    const refused = await run(d, ["reanchor", id]);
    expect(refused.code).toBe(1);
    expect(refused.json.errors).toEqual([
      "evidence: orphaned; pass --code-quote to re-anchor",
    ]);
    expect(sameStore(before, await snapshotStore(d))).toBe(true);
    await write(d, "src/limits.ts", "export const MAX_ATTEMPTS = 5;\n");
    const moved = await run(d, [
      "reanchor",
      id,
      "--code-file",
      "src/limits.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
    ]);
    expect(moved.code).toBe(0);
    expect(moved.json.evidence).toBe("unchanged");
    expect(moved.json.before?.evidence?.[0]?.quote).toBe("MAX_ATTEMPTS = 5");
    expect(moved.json.after?.evidence?.[0]?.quote).toBe("MAX_ATTEMPTS = 5");
    expect((await run(d, ["check"])).code).toBe(0);
  });

  test("plain reanchor leaves a changed doc side as stored (exit 1) and relocates a moved side next to it (exit 0)", async () => {
    const { d, id } = await seeded();
    await write(
      d,
      "README.md",
      "# Doc\n\nRetries are capped at five attempts.\n",
    );
    expect((await run(d, ["check"])).json.verdicts?.[0]?.doc).toBe("changed");
    const before = await snapshotStore(d);
    const changed = await run(d, ["reanchor", id]);
    expect(changed.code).toBe(1);
    expect(changed.json.errors).toEqual([
      "doc: changed; pass --doc-quote to re-anchor",
    ]);
    expect(sameStore(before, await snapshotStore(d))).toBe(true);
    // The evidence side moves too: it is relocated, the doc side is kept.
    await write(
      d,
      "src/retry.ts",
      `${"// p\n".repeat(4)}export const MAX_ATTEMPTS = 5;\n`,
    );
    const mixed = await run(d, ["reanchor", id]);
    expect(mixed.code).toBe(0);
    expect(mixed.json.kept).toEqual([
      {
        side: "doc",
        file: "README.md",
        state: "changed",
        reason: "doc: changed; pass --doc-quote to re-anchor",
      },
    ]);
    expect(mixed.json.after?.doc?.quote).toBe(
      "Retries are capped at 5 attempts",
    );
    const v = (await run(d, ["check"])).json.verdicts?.[0];
    expect(v?.doc).toBe("changed");
    expect(v?.evidence).toBe("unchanged");
  });

  test("the remediation reanchor command for a two-span claim lists every span and keeps both", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "src/b.ts", "export const B = 2;\n");
    await write(d, "doc.md", "# D\n\nA is one and B is two.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "doc.md",
      "--doc-quote",
      "A is one and B is two.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
      "--code-file",
      "src/b.ts",
      "--code-quote",
      "B = 2",
    ]);
    const id = rec.json.id ?? "";
    await write(d, "src/b.ts", "export const B = 20;\n");
    const v = (await run(d, ["check"])).json.verdicts?.[0];
    expect(v?.evidence).toBe("changed");
    const cmd = v?.remediation?.actions?.[0]?.command ?? "";
    expect(cmd).toBe(
      `hibi reanchor ${id} --code-file src/a.ts --code-quote "A = 1" --code-file src/b.ts --code-quote "B = 20"`,
    );
    expect(await runMenu(d, cmd)).toBe(0);
    const after = (await run(d, ["check"])).json.verdicts?.[0];
    expect(after?.evidence).toBe("unchanged");
    const listed = await run(d, ["reanchor", id, "--dry-run"]);
    expect(listed.json.after?.evidence?.map((e) => e.quote)).toEqual([
      "A = 1",
      "B = 20",
    ]);
  });

  test("the remediation reanchor command for a changed doc side and a changed evidence side runs as written", async () => {
    const { d, id } = await seeded();
    await write(
      d,
      "README.md",
      '# Doc\n\nRetries are "capped" at $7 attempts.\n',
    );
    const docV = (await run(d, ["check"])).json.verdicts?.[0];
    expect(docV?.doc).toBe("changed");
    const docCmd = docV?.remediation?.actions?.[0]?.command ?? "";
    expect(docCmd).toBe(
      `hibi reanchor ${id} --doc-quote "Retries are \\"capped\\" at \\$7 attempts"`,
    );
    expect(await runMenu(d, docCmd)).toBe(0);
    expect((await run(d, ["check"])).json.verdicts?.[0]?.doc).toBe("unchanged");
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const evV = (await run(d, ["check"])).json.verdicts?.[0];
    expect(evV?.evidence).toBe("changed");
    const evCmd = evV?.remediation?.actions?.[0]?.command ?? "";
    expect(evCmd).toBe(
      `hibi reanchor ${id} --code-file src/retry.ts --code-quote "MAX_ATTEMPTS = 50"`,
    );
    expect(await runMenu(d, evCmd)).toBe(0);
    expect((await run(d, ["check"])).json.verdicts?.[0]?.evidence).toBe(
      "unchanged",
    );
  });

  test("reanchor re-localizes a moved claim and --dry-run writes nothing", async () => {
    const { d, id } = await seeded();
    await write(
      d,
      "src/retry.ts",
      `${"// p\n".repeat(4)}export const MAX_ATTEMPTS = 5;\n`,
    );
    expect((await run(d, ["check"])).json.verdicts?.[0]?.evidence).toBe(
      "moved",
    );
    const before = await snapshotStore(d);
    const dry = await run(d, ["reanchor", id, "--dry-run"]);
    expect(dry.json.dryRun).toBe(true);
    expect(sameStore(before, await snapshotStore(d))).toBe(true);
    const re = await run(d, ["reanchor", id]);
    expect(re.code).toBe(0);
    expect(re.json.kept).toBeUndefined();
    expect(re.json.evidence).toBe("unchanged");
    expect((await run(d, ["check"])).json.verdicts?.[0]?.evidence).toBe(
      "unchanged",
    );
  });

  test("a single-value flag given twice is an error, not a dropped span", async () => {
    const { d, id } = await seeded();
    await write(d, "src/b.ts", "export const B = 2;\n");
    const before = await snapshotStore(d);
    const r = await run(d, [
      "reanchor",
      id,
      "--doc-quote",
      "Retries are capped",
      "--doc-quote",
      "Retries are capped at 5 attempts",
    ]);
    expect(r.code).toBe(1);
    expect(r.json.error).toBe(
      "--doc-quote given twice; reanchor takes one span per side",
    );
    expect(sameStore(before, await snapshotStore(d))).toBe(true);
    // --code-file is repeatable: each one opens a span, and every span is stored.
    const two = await run(d, [
      "reanchor",
      id,
      "--code-file",
      "src/retry.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
      "--code-file",
      "src/b.ts",
      "--code-quote",
      "B = 2",
    ]);
    expect(two.code).toBe(0);
    expect(two.json.after?.evidence?.map((e) => e.quote)).toEqual([
      "MAX_ATTEMPTS = 5",
      "B = 2",
    ]);
    const orphanQuote = await run(d, ["reanchor", id, "--code-quote", "B = 2"]);
    expect(orphanQuote.json.error).toBe(
      "--code-quote needs a --code-file before it",
    );
    const fmt = await run(d, ["list", "--format", "json", "--format", "json"]);
    expect(fmt.json.error).toBe(
      "--format given twice; list takes one --format",
    );
  });

  test("a value starting with - needs the --flag=<value> form, which every value flag accepts", async () => {
    const { d, id } = await seeded();
    const r = await run(d, ["reanchor", id, "--json", "--doc-quote", "-x"]);
    expect(r.code).toBe(1);
    expect(r.json.error).toBe(
      '--doc-quote needs a value; for a value starting with "-" use --doc-quote=<text>',
    );
    const end = await run(d, ["reanchor", id, "--json", "--code-file"]);
    expect(end.json.error).toBe(
      '--code-file needs a value; for a value starting with "-" use --code-file=<path>',
    );
    await write(
      d,
      "README.md",
      "# Doc\n\n- Retries are capped at 5 attempts.\n",
    );
    const eq = await run(d, [
      "reanchor",
      id,
      "--doc-quote=- Retries are capped at 5 attempts",
    ]);
    expect(eq.code).toBe(0);
    expect(eq.json.after?.doc?.quote).toBe(
      "- Retries are capped at 5 attempts",
    );
    // The = form parses for every value flag of every command.
    for (const cmd of COMMANDS) {
      for (const o of optionsFor(cmd)) {
        if (o.type !== "string") continue;
        const parsed = parseArgs({
          args: [`--${o.name}=-v`],
          strict: true,
          allowPositionals: true,
          options: parseArgsOptions(cmd),
        });
        const v = parsed.values[o.name];
        expect(o.multiple ? v : [v]).toEqual(["-v"]);
      }
    }
  });

  test("reanchor --suggest searches both sides, is read-only, and refuses span flags", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# D\n\nThe A constant is one.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "doc.md",
      "--doc-quote",
      "The A constant is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const id = rec.json.id ?? "";
    await rm(join(d, "src/a.ts"));
    await write(d, "src/b.ts", "export const A = 1;\n");
    const before = await snapshotStore(d);
    const sug = await run(d, ["reanchor", id, "--suggest"]);
    expect(sug.code).toBe(0);
    expect(sug.json.action).toBe("reanchor-suggest");
    const sides = sug.json.candidates?.map((c) => `${c.side}:${c.file}`);
    expect(sides).toContain("doc:doc.md");
    expect(sides).toContain("evidence:src/b.ts");
    expect(sameStore(before, await snapshotStore(d))).toBe(true);
    const bad = await run(d, ["reanchor", id, "--suggest", "--doc-quote", "x"]);
    expect(bad.code).toBe(1);
  });

  test("reanchor --doc moves the claim to another document (explicit span required)", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "wip.md", "# WIP\n\nThe A constant is one.\n");
    await write(d, "docs/a.md", "# A\n\nThe A constant is one.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "wip.md",
      "--doc-quote",
      "The A constant is one",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const id = rec.json.id ?? "";
    expect((await run(d, ["reanchor", id, "--doc", "docs/a.md"])).code).toBe(1);
    const re = await run(d, [
      "reanchor",
      id,
      "--doc",
      "docs/a.md",
      "--doc-quote",
      "The A constant is one",
    ]);
    expect(re.code).toBe(0);
    expect(re.json.doc).toBe("unchanged");
    await rm(join(d, "wip.md"));
    expect((await run(d, ["check"])).code).toBe(0);
    const byRange = await run(d, ["reanchor", id, "--doc-range", "L3:L3"]);
    expect(byRange.json.doc).toBe("unchanged");
  });

  test("supersede moves every live claim; archive tombstones", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\nexport const B = 2;\n");
    await write(d, "v1.md", "# V1\n\nA is one.\nB is two.\n");
    await run(d, ["init"]);
    const a = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "v1.md",
      "--doc-quote",
      "A is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const b = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "v1.md",
      "--doc-quote",
      "B is two.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "B = 2",
    ]);
    await write(d, "v2.md", "# V2\n\nA is one.\n");
    const dry = await run(d, [
      "supersede",
      "--from",
      "v1.md",
      "--to",
      "v2.md",
      "--dry-run",
    ]);
    expect(dry.json.dryRun).toBe(true);
    expect((await run(d, ["list", "--state", "stranded"])).json.count).toBe(0);
    const sup = await run(d, ["supersede", "--from", "v1.md", "--to", "v2.md"]);
    expect(sup.code).toBe(0);
    const byId = (x: { claimId?: string }, y: { claimId?: string }) =>
      (x.claimId ?? "").localeCompare(y.claimId ?? "");
    expect([...(sup.json.relocated ?? [])].sort(byId)).toEqual(
      [
        { claimId: a.json.id ?? "", doc: "unchanged", evidence: "unchanged" },
        { claimId: b.json.id ?? "", doc: "orphaned", evidence: "unchanged" },
      ].sort(byId),
    );
    expect(sup.json.misses).toEqual([]);
    expect(sup.json.strandedClaims).toEqual([]);
    expect(sup.json.next).toBe(
      "reanchor or retire each orphaned claim, then hibi check",
    );
    expect(
      (await run(d, ["supersede", "--from", "v1.md", "--to", "v1.md"])).code,
    ).toBe(1);
    const arch = await run(d, [
      "archive",
      "--doc",
      "v1.md",
      "--successor",
      "v2.md",
    ]);
    expect(arch.code).toBe(0);
    expect(await exists(join(d, "archive", "v1.md"))).toBe(true);
    expect(await readFile(join(d, "v1.md"), "utf8")).toContain("Archived");
    expect(arch.json.strandedClaims).toEqual([]);
  });

  test("--help prints the command's table and never runs it; unknown flags exit 1", async () => {
    const d = await repo();
    const help = await run(d, ["init", "--help"]);
    expect(help.code).toBe(0);
    expect(help.stdout).toContain("hibi init");
    expect(await exists(join(d, ".claims"))).toBe(false);
    const rec = await run(d, ["record", "--help"]);
    expect(rec.stdout).toContain("--doc-quote");
    expect(rec.stdout).not.toContain("--trust");
    const typo = await run(d, ["check", "--wirte"]);
    expect(typo.code).toBe(1);
    expect(typo.json.error).toContain("--wirte");
    const trust = await run(d, ["record", "--trust", "yes"]);
    expect(trust.code).toBe(1);
    await run(d, ["init"]);
    const badRange = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "x.md",
      "--doc-range",
      "3",
    ]);
    expect(badRange.code).toBe(1);
    expect(badRange.stderr).not.toContain("at ");
    expect(badRange.json.error).toContain("start:end");
    const badEnum = await run(d, ["check", "--fail-on", "tamper"]);
    expect(badEnum.code).toBe(1);
    expect((await run(d, ["frobnicate"])).code).toBe(1);
    expect((await run(d, [])).code).toBe(1);
    expect((await run(d, ["--help"])).code).toBe(0);
  });

  test("--json is byte-identical to the piped default; --format json-pretty is the same data", async () => {
    const { d } = await seeded();
    for (const args of [
      ["check"],
      ["check", "--doc", "README.md"],
      ["list"],
      ["schema", "--name", "Claim"],
      ["version"],
    ]) {
      const def = await run(d, args);
      const forced = await run(d, [...args, "--json"]);
      expect(forced.stdout).toBe(def.stdout);
      const pretty = await run(d, [...args, "--format", "json-pretty"]);
      expect(JSON.parse(pretty.stdout)).toEqual(JSON.parse(def.stdout));
    }
    const human = await run(d, [
      "check",
      "--format",
      "human",
      "--color",
      "never",
    ]);
    expect(human.stdout).toContain("hibi check");
    const overview = await run(d, [
      "check",
      "--overview",
      "--format",
      "human",
      "--color",
      "never",
    ]);
    expect(overview.stdout).toContain("Store v4");
  });

  test("piped JSON past the 64 KiB pipe buffer arrives whole on exit 0, 1, and 2", async () => {
    const d = await repo();
    // One counted pending reason per claim (B.8) keeps check output small; 80 claims still pass 64 KiB.
    const n = 80;
    const pad = "This limit applies to every request the client sends. ".repeat(
      40,
    );
    const code = (shift: number) =>
      Array.from(
        { length: n },
        (_, i) => `export const LIMIT_${i} = ${i + shift};\n`,
      ).join("");
    await write(d, "src/limits.ts", code(0));
    await write(
      d,
      "doc.md",
      `# Limits\n\n${Array.from({ length: n }, (_, i) => `Limit number ${i} is set to ${i}. ${pad}\n`).join("")}`,
    );
    await run(d, ["init"]);
    const items = Array.from({ length: n }, (_, i) => ({
      doc: "doc.md",
      docRange: `L${i + 3}:L${i + 3}`,
      kind: "code",
      code: [{ file: "src/limits.ts", quote: `LIMIT_${i} = ${i}` }],
    }));
    expect(
      (await run(d, ["record", "--from-file", "-"], JSON.stringify(items)))
        .code,
    ).toBe(0);
    const whole = (r: RunResult) => {
      expect(r.stdout.length).toBeGreaterThan(65536);
      return JSON.parse(r.stdout);
    };

    const piped = { shellPipe: true };

    const list = await run(d, ["list"], undefined, piped);
    expect(list.code).toBe(0);
    expect(whole(list).count).toBe(n);

    await write(d, "src/limits.ts", code(1000));
    const check = await run(d, ["check"], undefined, piped);
    expect(check.code).toBe(2);
    expect(whole(check).summary.gating).toBe(n);

    const bad = await run(
      d,
      ["record", "--from-file", "-"],
      JSON.stringify(Array.from({ length: 3000 }, () => ({ doc: "doc.md" }))),
      piped,
    );
    expect(bad.code).toBe(1);
    expect(whole(bad).errors.length).toBeGreaterThan(3000);
  });

  test("--store-dir decouples the store from the anchor root", async () => {
    const d = await repo();
    const storeHome = await mkdtemp(join(tmpdir(), "ce-cli-store-"));
    dirs.push(storeHome);
    const storeDir = join(storeHome, "claims");
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# Doc\n\nA is 1 here.\n");
    expect((await run(d, ["init", "--store-dir", storeDir])).json.store).toBe(
      storeDir,
    );
    const rec = await run(d, [
      "record",
      "--store-dir",
      storeDir,
      "--kind",
      "code",
      "--doc",
      "doc.md",
      "--doc-quote",
      "A is 1 here",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    expect(rec.code).toBe(0);
    expect(await exists(join(d, ".claims"))).toBe(false);
    expect(
      (await run(d, ["check", "--store-dir", storeDir])).json.summary?.clean,
    ).toBe(1);
  });

  test("schema emits generated JSON Schema by name and lists the model schemas", async () => {
    const d = await repo();
    const res = await run(d, ["schema", "--name", "Claim"]);
    expect(res.code).toBe(0);
    // zod 4.5 emits a registered root schema as a `$ref` into `$defs`.
    const raw = res.json as CliJson & {
      $ref?: string;
      $defs?: Record<string, CliJson>;
    };
    const root =
      raw.$ref !== undefined
        ? raw.$defs?.[raw.$ref.split("/").at(-1) ?? ""]
        : raw;
    expect(root?.type).toBe("object");
    expect(root?.properties?.attestations).toBeDefined();
    const list = await run(d, ["schema"]);
    expect(list.stdout).toContain("JudgeCacheEntry");
    expect(list.stdout).not.toContain("ResolveParams");
    expect((await run(d, ["schema", "--name", "Nope"])).code).toBe(1);
  });

  test("completions are generated from the option tables", async () => {
    const d = await repo();
    const zsh = await run(d, ["completions", "zsh"]);
    expect(zsh.code).toBe(0);
    expect(zsh.stdout).toContain("--from-file");
    expect(zsh.stdout).toContain("'supersede'");
    expect(zsh.stdout).not.toContain("'doctor'");
    expect(zsh.stdout).not.toContain("--trust");
    expect((await run(d, ["completions", "tcsh"])).code).toBe(1);
  });

  test("no check flag combination mutates .claims/", async () => {
    const { d } = await seeded();
    const baseline = await snapshotStore(d);
    for (const args of [
      ["check"],
      ["check", "--fail-on", "never"],
      ["check", "--overview"],
      ["list"],
    ]) {
      await run(d, args);
      expect(sameStore(baseline, await snapshotStore(d))).toBe(true);
    }
  });

  test("removed flags are unknown options", async () => {
    const { d } = await seeded();
    for (const args of [
      ["check", "--write"],
      ["check", "--run-verifiers"],
      ["check", "--verifier-timeout", "5"],
      ["record", "--verified"],
      ["record", "--verifier", "command:x"],
      ["record", "--glob", "src/**"],
    ]) {
      const res = await run(d, args);
      expect(res.code).toBe(1);
      expect(res.json.error).toContain("unknown option");
    }
  });

  test("a v3 store is refused with a one-line no-upgrade error", async () => {
    const d = await repo();
    await write(
      d,
      ".claims/config.json",
      JSON.stringify({ version: "v3", nonce: "deadbeef" }),
    );
    const res = await run(d, ["check"]);
    expect(res.code).toBe(1);
    expect(res.json.error).toContain("version v3");
    expect(res.json.error).toContain("no upgrade path");
  });

  test("init on a v3 store prints the no-upgrade-path message", async () => {
    const d = await repo();
    await write(
      d,
      ".claims/config.json",
      JSON.stringify({ version: "v3", nonce: "deadbeef" }),
    );
    const res = await run(d, ["init"]);
    expect(res.code).toBe(1);
    expect(res.json.ok).toBe(false);
    expect(res.json.error).toContain("this claim store is version v3");
    expect(res.json.error).toContain("no upgrade path");
  });

  test("check and list failures print the JSON error envelope", async () => {
    const { d } = await seeded();
    await write(d, ".claims/claims/clm_0000000000000bad.json", "{ not json");
    for (const args of [["check"], ["list"]]) {
      const res = await run(d, args);
      expect(res.code).toBe(1);
      expect(res.json.ok).toBe(false);
      expect(typeof res.json.error).toBe("string");
    }
  });

  test("record --from-file on a missing file prints the JSON error envelope", async () => {
    const { d } = await seeded();
    const res = await run(d, ["record", "--from-file", "nope.json"]);
    expect(res.code).toBe(1);
    expect(res.json).toEqual({
      ok: false,
      error: "record --from-file: --from-file nope.json: no such file",
    });
  });

  test("record --from-file takes --by as the default for items without by", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# Doc\n\nA is 1 here.\n\nA is one, really.\n");
    await run(d, ["init"]);
    const items = [
      {
        doc: "./doc.md",
        docQuote: "A is 1 here",
        kind: "code",
        code: [{ file: "src/a.ts", quote: "A = 1" }],
      },
      {
        doc: "doc.md",
        docQuote: "A is one, really.",
        kind: "code",
        code: [{ file: "./src/a.ts", quote: "A = 1" }],
        by: "agent:opus",
      },
    ];
    const res = await run(
      d,
      [
        "record",
        "--from-file",
        "-",
        "--by",
        "human:dev@example.com",
        "--explain",
      ],
      JSON.stringify(items),
    );
    expect(res.code).toBe(0);
    const claims = await Promise.all(
      (res.json.results as unknown as { id: string }[]).map(
        async (r) =>
          JSON.parse(
            await readFile(
              join(d, ".claims", "claims", `${r.id}.json`),
              "utf8",
            ),
          ) as { attestations: { by: string }[]; doc: { file: string } },
      ),
    );
    expect(claims.map((c) => c.attestations[0]?.by)).toEqual([
      "human:dev@example.com",
      "agent:opus",
    ]);
    expect(claims.map((c) => c.doc.file)).toEqual(["doc.md", "doc.md"]);
  });

  test("reanchor takes --ref-* for a reference claim and refuses --code-* on it", async () => {
    const d = await repo();
    await write(d, "doc.md", "# Doc\n\nRefunds take two weeks.\n");
    await write(
      d,
      "billing.md",
      "# Billing\n\nRefunds are issued within 14 days.\n",
    );
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--kind",
      "reference",
      "--doc",
      "doc.md",
      "--doc-quote",
      "Refunds take two weeks.",
      "--ref-file",
      "billing.md",
      "--ref-quote",
      "Refunds are issued within 14 days.",
    ]);
    const id = rec.json.id ?? "";
    await write(d, "billing.md", "# Billing\n\nWe refund within 14 days.\n");
    const bad = await run(d, [
      "reanchor",
      id,
      "--code-file",
      "billing.md",
      "--code-quote",
      "We refund",
    ]);
    expect(bad.code).toBe(1);
    expect(bad.json.error).toContain("is a reference claim; --code-file");
    const ok = await run(d, [
      "reanchor",
      id,
      "--ref-file",
      "billing.md",
      "--ref-quote",
      "We refund within 14 days.",
    ]);
    expect(ok.code).toBe(0);
    expect(ok.json.evidence).toBe("unchanged");
  });

  test("--fail-on help names what warn does", async () => {
    const d = await repo();
    const res = await run(d, ["check", "--help"]);
    expect(res.stdout).toContain("warn: any warning also fails");
  });

  test("record without --by fails naming --by and the actor forms, even with a git user.email", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# Doc\n\nA is 1 here.\n");
    await run(d, ["init"]);
    const proc = Bun.spawn(
      [
        "bun",
        "run",
        CLI,
        "record",
        "--kind",
        "code",
        "--doc",
        "doc.md",
        "--doc-quote",
        "A is 1 here",
        "--code-file",
        "src/a.ts",
        "--code-quote",
        "A = 1",
      ],
      { cwd: d, stdout: "pipe", stderr: "pipe" },
    );
    const out = JSON.parse((await new Response(proc.stdout).text()).trim());
    expect(await proc.exited).toBe(1);
    expect(out.error).toContain("--by");
    expect(out.errors).toEqual([
      "by is required: pass --by <actor> or a by key, as human:<id>, agent:<name>, or process:<name>",
    ]);
    const withBy = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      "doc.md",
      "--doc-quote",
      "A is 1 here",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
      "--by",
      "agent:opus",
      "--explain",
    ]);
    expect(withBy.code).toBe(0);
    expect(
      (withBy.json.claim as { attestations?: { by?: string }[] })
        .attestations?.[0]?.by,
    ).toBe("agent:opus");
  });
});

describe("fix pass B (CLI)", () => {
  test("B.2: version --json is wrapped in the envelope", async () => {
    const d = await repo();
    const res = await run(d, ["version", "--json"]);
    expect(res.code).toBe(0);
    const pkg = JSON.parse(
      await readFile(join(import.meta.dir, "..", "package.json"), "utf8"),
    );
    expect(res.json as Record<string, unknown>).toEqual({
      ok: true,
      action: "version",
      schemaVersion: "v4",
      name: "hibi",
      version: pkg.version,
    });
  });

  test("B.3: batch record results carry id, updated and existingClaims; the batch counts updates", async () => {
    const { d, id } = await seeded();
    await write(
      d,
      "docs/other.md",
      "# Other\n\nRetries are capped at 5 attempts.\n",
    );
    const res = await run(
      d,
      ["record", "--from-file", "-"],
      JSON.stringify([
        {
          doc: "README.md",
          docQuote: "Retries are capped at 5 attempts",
          kind: "code",
          code: [{ file: "src/retry.ts", quote: "MAX_ATTEMPTS = 5" }],
        },
        {
          doc: "docs/other.md",
          docQuote: "Retries are capped at 5 attempts",
          kind: "code",
          code: [{ file: "src/retry.ts", quote: "MAX_ATTEMPTS = 5" }],
        },
      ]),
    );
    expect(res.code).toBe(0);
    const json = res.json as CliJson & {
      updated?: number;
      results?: {
        id?: string;
        updated?: boolean;
        existingClaims?: string[];
      }[];
    };
    expect(json.count).toBe(2);
    expect(json.updated).toBe(1);
    expect(json.results?.[0]).toMatchObject({
      id,
      updated: true,
      existingClaims: [],
    });
    expect(Object.keys(json.results?.[0] ?? {})).not.toContain("claimId");
    const second = json.results?.[1];
    expect(second?.id).not.toBe(id);
    expect(second?.updated).toBe(false);
    expect(second?.existingClaims).toEqual([id]);
  });
});

describe("B.12 action.yml", () => {
  test("B.12: the action description says a moved span only warns under the default fail-on", async () => {
    const yml = await readFile(
      join(import.meta.dir, "..", "action.yml"),
      "utf8",
    );
    const description = yml.match(/^description: "(.*)"$/m)?.[1] ?? "";
    expect(description).not.toMatch(/\bmoved or changed\b/);
    expect(description).toContain(
      "Under the default fail-on: gating, a moved span only warns.",
    );
    expect(yml).toMatch(/fail-on:[\s\S]*?default: "gating"/);
  });
});

interface BulkJson {
  ok?: boolean;
  error?: string;
  action?: string;
  moved?: boolean;
  relocated?: number;
  dryRun?: boolean;
  next?: string;
  claims?: {
    id?: string;
    relocated?: { side?: string; file?: string }[];
    kept?: { side?: string; file?: string; state?: string; reason?: string }[];
  }[];
}

describe("reanchor --moved", () => {
  async function recordCode(
    d: string,
    doc: string,
    sentence: string,
    file: string,
    quote: string,
  ): Promise<string> {
    const rec = await run(d, [
      "record",
      "--kind",
      "code",
      "--doc",
      doc,
      "--doc-quote",
      sentence,
      "--code-file",
      file,
      "--code-quote",
      quote,
    ]);
    expect(rec.code).toBe(0);
    return rec.json.id ?? "";
  }

  async function twoDocs() {
    const d = await repo();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await write(d, "src/b.ts", "export const BACKOFF_MS = 200;\n");
    await write(d, "src/c.ts", "export const TIMEOUT_S = 30;\n");
    await write(
      d,
      "README.md",
      "# Doc\n\nRetries are capped at 5 attempts.\n\nThe timeout is 30 seconds.\n",
    );
    await write(d, "docs/b.md", "# B\n\nBackoff starts at 200 ms.\n");
    await run(d, ["init"]);
    const a = await recordCode(
      d,
      "README.md",
      "Retries are capped at 5 attempts",
      "src/retry.ts",
      "MAX_ATTEMPTS = 5",
    );
    const b = await recordCode(
      d,
      "docs/b.md",
      "Backoff starts at 200 ms",
      "src/b.ts",
      "BACKOFF_MS = 200",
    );
    const c = await recordCode(
      d,
      "README.md",
      "The timeout is 30 seconds",
      "src/c.ts",
      "TIMEOUT_S = 30",
    );
    // a and b move; c changes and stays as stored.
    await write(
      d,
      "src/retry.ts",
      "// header\n\nexport const MAX_ATTEMPTS = 5;\n",
    );
    await write(d, "src/b.ts", "// header\n\nexport const BACKOFF_MS = 200;\n");
    await write(d, "src/c.ts", "export const TIMEOUT_S = 60;\n");
    return { d, a, b, c };
  }

  async function bulk(d: string, args: string[]) {
    const r = await run(d, ["reanchor", "--moved", ...args]);
    return { code: r.code, json: r.json as BulkJson, stdout: r.stdout };
  }

  test("relocates moved claims across two docs and lists kept sides", async () => {
    const { d, a, b, c } = await twoDocs();
    const r = await bulk(d, []);
    expect(r.code).toBe(0);
    expect(r.json.action).toBe("reanchor");
    expect(r.json.moved).toBe(true);
    expect(r.json.relocated).toBe(2);
    expect(r.json.next).toBe("hibi check");
    const byId = new Map((r.json.claims ?? []).map((x) => [x.id, x]));
    expect(byId.get(a)?.relocated).toEqual([
      { side: "evidence", file: "src/retry.ts" },
    ]);
    expect(byId.get(b)?.relocated).toEqual([
      { side: "evidence", file: "src/b.ts" },
    ]);
    expect(byId.get(c)?.relocated).toEqual([]);
    expect(byId.get(c)?.kept?.[0]?.reason).toBe(
      "evidence: changed; pass --code-quote to re-anchor",
    );
    const check = await run(d, ["check"]);
    const state = new Map(
      (check.json.verdicts ?? []).map((v) => [v.claimId, v.evidence]),
    );
    expect(state.get(a)).toBe("unchanged");
    expect(state.get(b)).toBe("unchanged");
    expect(state.get(c)).toBe("changed");
    // Human output: a header, then one line per listed claim.
    const human = await run(d, ["reanchor", "--moved", "--format", "compact"]);
    expect(human.code).toBe(0);
    expect(human.stdout.trim().split("\n")).toHaveLength(2);
  });

  test("--doc restricts it to one document", async () => {
    const { d, a, b } = await twoDocs();
    const r = await bulk(d, ["--doc", "docs/b.md"]);
    expect(r.code).toBe(0);
    expect(r.json.relocated).toBe(1);
    expect(r.json.claims?.map((x) => x.id)).toEqual([b]);
    const check = await run(d, ["check"]);
    const state = new Map(
      (check.json.verdicts ?? []).map((v) => [v.claimId, v.evidence]),
    );
    expect(state.get(a)).toBe("moved");
    expect(state.get(b)).toBe("unchanged");
    const none = await bulk(d, ["--doc", "docs/none.md"]);
    expect(none.code).toBe(1);
    expect(none.json.error).toContain(
      "no claims are registered for docs/none.md",
    );
  });

  test("--dry-run writes nothing", async () => {
    const { d } = await twoDocs();
    const before = await snapshotStore(d);
    const r = await bulk(d, ["--dry-run"]);
    expect(r.code).toBe(0);
    expect(r.json.dryRun).toBe(true);
    expect(r.json.relocated).toBe(2);
    expect(r.json.next).toBe("re-run without --dry-run to apply");
    expect(sameStore(before, await snapshotStore(d))).toBe(true);
  });

  test("--moved with a claim id, span flags, or without --moved and an id is an error", async () => {
    const { d, a } = await twoDocs();
    const withId = await run(d, ["reanchor", a, "--moved"]);
    expect(withId.code).toBe(1);
    expect(withId.json.error).toContain("takes no claim id");
    const withSpan = await run(d, [
      "reanchor",
      "--moved",
      "--doc-quote",
      "Retries",
    ]);
    expect(withSpan.code).toBe(1);
    const bare = await run(d, ["reanchor"]);
    expect(bare.code).toBe(1);
    expect(bare.json.error).toBe(
      "reanchor requires a <claim-id> argument, or --moved",
    );
  });
});

describe("reanchor with an explicit span in a sibling repo that is not checked out", () => {
  test("keeps that side as stored and exits 0 when another side relocated", async () => {
    const d = await repo();
    const sibling = await mkdtemp(join(tmpdir(), "ce-sibling-"));
    dirs.push(sibling);
    await write(sibling, "src/limits.ts", "export const LIMIT = 100;\n");
    await write(d, "src/local.ts", "export const LOCAL = 1;\n");
    await write(d, "doc.md", "# Doc\n\nLimits are 100 requests.\n");
    await run(d, ["init"]);
    const configPath = join(d, ".claims", "config.json");
    const config = JSON.parse(await readFile(configPath, "utf8"));
    await writeFile(
      configPath,
      JSON.stringify({ ...config, repos: { api: sibling } }),
    );
    const rec = await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "Limits are 100 requests.",
      "--kind",
      "code",
      "--code-file",
      "api:src/limits.ts",
      "--code-quote",
      "LIMIT = 100",
      "--code-file",
      "src/local.ts",
      "--code-quote",
      "LOCAL = 1",
    ]);
    expect(rec.code).toBe(0);
    const id = rec.json.id ?? "";
    // The sibling repo goes away; the local span changes.
    await rm(sibling, { recursive: true, force: true });
    await write(d, "src/local.ts", "export const LOCAL = 2;\n");
    const re = await run(d, [
      "reanchor",
      id,
      "--code-file",
      "api:src/limits.ts",
      "--code-quote",
      "LIMIT = 100",
      "--code-file",
      "src/local.ts",
      "--code-quote",
      "LOCAL = 2",
    ]);
    expect(re.code).toBe(0);
    expect(re.json.kept).toEqual([
      {
        side: "evidence",
        file: "api:src/limits.ts",
        state: "unavailable",
        reason: "evidence: repository api not checked out; kept as stored",
      },
    ]);
    expect(re.json.after?.evidence?.map((e) => e.quote)).toEqual([
      "LIMIT = 100",
      "LOCAL = 2",
    ]);
    // Only the unavailable side given: nothing relocated, exit 1.
    const only = await run(d, [
      "reanchor",
      id,
      "--code-file",
      "api:src/limits.ts",
      "--code-quote",
      "LIMIT = 100",
    ]);
    expect(only.code).toBe(1);
    expect(only.json.errors).toEqual([
      "evidence: repository api not checked out; kept as stored",
    ]);
  });
});
