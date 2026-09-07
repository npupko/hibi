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
  assertionId?: string;
  doc?: string;
  code?: string;
  behavior?: string;
  expired?: boolean;
  gates?: boolean;
  changed?: string;
  remediation?: Remediation | null;
  notes?: string[];
  evidence?: unknown;
  advisories?: unknown;
  fingerprint?: string;
}
interface ListRow {
  claimId?: string;
  status?: string;
  severity?: string;
  gates?: boolean;
  recommended?: string | null;
  documentPath?: string | null;
  codePath?: string | null;
  side?: string;
  text?: string;
}
interface CliJson {
  ok?: boolean;
  error?: string;
  action?: string;
  schemaVersion?: string;
  next?: string;
  nonce?: string;
  store?: string;
  id?: string;
  doc?: string;
  code?: string;
  enforcement?: string;
  verified?: boolean;
  warnings?: string[];
  alreadyRetired?: boolean;
  batch?: boolean;
  results?: { id?: string }[];
  before?: { doc?: { quote?: string }; code?: { quote?: string }[] };
  after?: { doc?: { quote?: string }; code?: { quote?: string }[] };
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
  found?: boolean;
  count?: number;
  state?: string;
  claims?: ListRow[];
  candidates?: { side?: string; file?: string }[];
  relocated?: { claimId?: string }[];
  misses?: { claimId?: string }[];
  strandedClaims?: string[];
  dryRun?: boolean;
  assertion?: unknown;
  type?: string;
  properties?: { anchor?: unknown };
  exitCode?: number;
}

interface RunResult {
  code: number;
  json: CliJson;
  stdout: string;
  stderr: string;
}

async function run(
  cwd: string,
  args: string[],
  stdin?: string,
): Promise<RunResult> {
  const proc = Bun.spawn(["bun", "run", CLI, ...args], {
    cwd,
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

/** A repo with one enforced claim on MAX_ATTEMPTS. */
async function seeded(): Promise<{ d: string; id: string }> {
  const d = await repo();
  await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
  await write(d, "README.md", "# Doc\n\nRetries are capped at 5 attempts.\n");
  await run(d, ["init"]);
  const rec = await run(d, [
    "record",
    "--doc",
    "README.md",
    "--doc-quote",
    "Retries are capped at 5 attempts",
    "--code-file",
    "src/retry.ts",
    "--code-quote",
    "MAX_ATTEMPTS = 5",
    "--owner",
    "alice",
  ]);
  return { d, id: rec.json.id ?? "" };
}

describe("CLI end-to-end", () => {
  test("init → record → check clean → drift → exit 2 (enforced by default)", async () => {
    const { d } = await seeded();
    const init = await run(d, ["init"]);
    expect(init.code).toBe(0);
    expect(init.json.nonce).toMatch(/^[0-9a-f]{8}$/);
    expect(init.json.schemaVersion).toBe("v3");

    const clean = await run(d, ["check"]);
    expect(clean.code).toBe(0);
    expect(clean.json.summary?.clean).toBe(1);

    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const drifted = await run(d, ["check"]);
    expect(drifted.code).toBe(2);
    expect(drifted.json.verdicts?.[0]?.code).toBe("changed");
    expect(drifted.json.verdicts?.[0]?.gates).toBe(true);
    expect(drifted.json.verdicts?.[0]?.changed).toContain("value changed");
  });

  test("piped record returns the lean summary; --explain returns the full result", async () => {
    const { d, id } = await seeded();
    expect(id).toMatch(/^asrt_/);
    const rec = await run(d, [
      "record",
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
        "code",
        "doc",
        "enforcement",
        "id",
        "next",
        "ok",
        "schemaVersion",
        "verified",
      ].sort(),
    );
    expect(rec.json.enforcement).toBe("enforced");
    const full = await run(d, [
      "record",
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
    expect(full.json.assertion).toBeDefined();
  });

  test("record --suggest never gates; --verified is recorded", async () => {
    const d = await repo();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await write(d, "README.md", "# Doc\n\nRetries are capped at 5 attempts.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--doc",
      "README.md",
      "--doc-quote",
      "Retries are capped at 5 attempts",
      "--code-file",
      "src/retry.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
      "--suggest",
      "--verified",
    ]);
    expect(rec.json.enforcement).toBe("suggested");
    expect(rec.json.verified).toBe(true);
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const drifted = await run(d, ["check"]);
    expect(drifted.code).toBe(0);
    expect(drifted.json.verdicts?.[0]?.code).toBe("changed");
    expect(drifted.json.verdicts?.[0]?.gates).toBe(false);
  });

  test("record rejects a nonexistent --code-file with exit 1", async () => {
    const d = await repo();
    await write(d, "README.md", "# Doc\n\nRetries are capped at 5 attempts.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
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
          codeFile: "src/conf.ts",
          codeQuote: "TTL_MS = 60000",
          verified: true,
        },
        {
          doc: "docs/conf.md",
          docQuote: "Retries default to 3",
          codeFile: "src/conf.ts",
          codeQuote: "RETRIES = 3",
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
          codeFile: "src/conf.ts",
          codeQuote: "TTL_MS = 60000",
        },
        { doc: "docs/conf.md", docQuote: "NOT PRESENT IN THE FILE" },
      ]),
    );
    expect(bad.code).toBe(1);
    expect(sameStore(before, await snapshotStore(d))).toBe(true);
  });

  test("check --write stamps a banner without a checksum, then clears it", async () => {
    const { d } = await seeded();
    await write(d, "src/retry.ts", "// removed\n");
    const res = await run(d, ["check", "--write"]);
    expect(res.code).toBe(2);
    const doc = await readFile(join(d, "README.md"), "utf8");
    expect(doc).toContain("HIBI:BEGIN");
    expect(doc).toContain("STALE DOCUMENT");
    expect(doc).not.toContain("sha=");
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await run(d, ["check", "--write"]);
    expect(await readFile(join(d, "README.md"), "utf8")).not.toContain(
      "HIBI:BEGIN",
    );
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
    expect(soft.json.verdicts?.[0]?.code).toBe("moved");
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
    expect(a.json.found).toBe(true);
    expect(a.json.verdicts?.length).toBe(1);
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
    const untracked = await run(d, ["check", "--doc", "nope.md"]);
    expect(untracked.json.found).toBe(false);
  });

  test("diff and status are deprecated aliases that print a notice", async () => {
    const { d } = await seeded();
    await commit(d);
    const diff = await run(d, ["diff", "--since", "HEAD"]);
    expect(diff.code).toBe(0);
    expect(diff.json.action).toBe("check");
    expect(diff.stderr).toContain("deprecated");
    const status = await run(d, ["status", "--doc", "README.md"]);
    expect(status.json.action).toBe("check");
    expect(status.stderr).toContain("hibi check --doc");
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
    expect(row?.status).toBe("retired");
  });

  test("list filters by --state and --path, and --ids-only prints bare ids", async () => {
    const { d, id } = await seeded();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const all = await run(d, ["list"]);
    const row = all.json.claims?.[0];
    expect(row?.claimId).toBe(id);
    expect(row?.status).toBe("code:changed");
    expect(row?.severity).toBe("gating");
    expect(row?.text).toContain("Retries are capped");
    expect(row?.recommended).toBe("update-claim");
    expect((await run(d, ["list", "--state", "gating"])).json.count).toBe(1);
    expect((await run(d, ["list", "--state", "clean"])).json.count).toBe(0);
    const byPath = await run(d, ["list", "--path", "src/retry.ts"]);
    expect(byPath.json.count).toBe(1);
    expect(byPath.json.claims?.[0]?.side).toBe("code");
    expect(
      (await run(d, ["list", "--path", "README.md"])).json.claims?.[0]?.side,
    ).toBe("doc");
    expect((await run(d, ["list", "--path", "src/other.ts"])).json.count).toBe(
      0,
    );
    const ids = await run(d, ["list", "--ids-only"]);
    expect(ids.stdout.trim()).toBe(id);
    expect((await run(d, ["list", "--state", "bogus"])).code).toBe(1);
    const noHints = await run(d, ["list", "--no-hints"]);
    expect(noHints.json.claims?.[0]?.recommended).toBeNull();
  });

  test("list --state orphaned drains after retire; stranded and duplicate filters", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "src/gone.ts", "export const X = 1;\n");
    await write(d, "ok.md", "# OK\n\nA is one.\n");
    await write(d, "o.md", "# O\n\nOrphan here.\n");
    await write(d, "dup.md", "# Dup\n\nA is one.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
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
      (await run(d, ["list", "--state", "orphaned"])).json.claims?.map(
        (c) => c.claimId,
      ),
    ).toEqual([orphan.json.id ?? ""]);
    expect((await run(d, ["list", "--state", "duplicate"])).json.count).toBe(2);
    await run(d, ["retire", orphan.json.id ?? ""]);
    expect((await run(d, ["list", "--state", "orphaned"])).json.count).toBe(0);
    await write(d, "new.md", "# New\n\nSomething else.\n");
    await run(d, ["supersede", "--from", "ok.md", "--to", "new.md"]);
    const stranded = await run(d, ["list", "--state", "stranded"]);
    expect(stranded.json.count).toBe(1);
    expect(stranded.json.claims?.[0]?.status).toBe("superseded");
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
    expect(explained.json.verdicts?.[0]?.evidence).toBeDefined();
    expect(explained.json.verdicts?.[0]?.fingerprint).toBeDefined();
    expect(
      (await run(d, ["check"])).json.verdicts?.[0]?.evidence,
    ).toBeUndefined();
  });

  test("an orphan recommends the read-only --suggest pass", async () => {
    const { d, id } = await seeded();
    await rm(join(d, "src/retry.ts"));
    const v = (await run(d, ["check"])).json.verdicts?.[0];
    expect(v?.code).toBe("orphaned");
    expect(v?.remediation?.recommended).toBe("reanchor");
    expect(v?.remediation?.actions?.[0]?.command).toBe(
      `hibi reanchor ${id} --suggest`,
    );
  });

  test("reanchor refuses an orphaned side, accepts an explicit new span, and reports before/after quotes", async () => {
    const { d, id } = await seeded();
    await rm(join(d, "src/retry.ts"));
    const refused = await run(d, ["reanchor", id]);
    expect(refused.code).toBe(1);
    expect(refused.json.error).toContain("orphaned");
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
    expect(moved.json.code).toBe("unchanged");
    expect(moved.json.before?.code?.[0]?.quote).toBe("MAX_ATTEMPTS = 5");
    expect(moved.json.after?.code?.[0]?.quote).toBe("MAX_ATTEMPTS = 5");
    expect((await run(d, ["check"])).code).toBe(0);
  });

  test("reanchor re-localizes a moved claim and --dry-run writes nothing", async () => {
    const { d, id } = await seeded();
    await write(
      d,
      "src/retry.ts",
      `${"// p\n".repeat(4)}export const MAX_ATTEMPTS = 5;\n`,
    );
    expect((await run(d, ["check"])).json.verdicts?.[0]?.code).toBe("moved");
    const before = await snapshotStore(d);
    const dry = await run(d, ["reanchor", id, "--dry-run"]);
    expect(dry.json.dryRun).toBe(true);
    expect(sameStore(before, await snapshotStore(d))).toBe(true);
    const re = await run(d, ["reanchor", id]);
    expect(re.json.code).toBe("unchanged");
    expect((await run(d, ["check"])).json.verdicts?.[0]?.code).toBe(
      "unchanged",
    );
  });

  test("reanchor --suggest searches both sides, is read-only, and refuses span flags", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# D\n\nThe A constant is one.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
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
    expect(sides).toContain("code:src/b.ts");
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

  test("supersede relocates verbatim sentences and reports misses; archive tombstones", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\nexport const B = 2;\n");
    await write(d, "v1.md", "# V1\n\nA is one.\nB is two.\n");
    await run(d, ["init"]);
    const a = await run(d, [
      "record",
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
    expect(sup.json.relocated?.map((r) => r.claimId)).toEqual([
      a.json.id ?? "",
    ]);
    expect(sup.json.misses?.map((m) => m.claimId)).toEqual([b.json.id ?? ""]);
    expect(sup.json.strandedClaims).toEqual([b.json.id ?? ""]);
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
    expect(arch.json.strandedClaims).toEqual([b.json.id ?? ""]);
  });

  test("coverage splits prose at sentence level, gates with --fail-uncovered, and errors on a missing doc", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(
      d,
      "doc.md",
      "The A constant is one. Nothing backs this sentence.\n",
    );
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "The A constant is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const cov = await run(d, ["coverage", "--doc", "doc.md"]);
    expect(cov.code).toBe(0);
    expect(cov.json.summary?.uncovered).toBe(1);
    expect(
      (await run(d, ["coverage", "--doc", "doc.md", "--fail-uncovered"])).code,
    ).toBe(2);
    const missing = await run(d, ["coverage", "--doc", "nope.md"]);
    expect(missing.code).toBe(1);
    expect(missing.json.error).toContain("not found");
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
      ["schema", "--name", "Assertion"],
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
    expect(overview.stdout).toContain("Store v3");
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

  test("schema emits generated JSON Schema by name and lists protocol schemas", async () => {
    const d = await repo();
    const res = await run(d, ["schema", "--name", "Assertion"]);
    expect(res.code).toBe(0);
    expect(res.json.type).toBe("object");
    expect(res.json.properties?.anchor).toBeDefined();
    const list = await run(d, ["schema"]);
    expect(list.stdout).toContain("ResolveParams");
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
      ["check", "--write"],
      ["check", "--run-verifiers"],
      ["check", "--fail-on", "never"],
      ["check", "--overview"],
      ["list"],
      ["coverage", "--doc", "README.md"],
    ]) {
      await run(d, args);
      expect(sameStore(baseline, await snapshotStore(d))).toBe(true);
    }
  });

  test("a v2 store is upgraded once at open and then checks clean", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# Doc\n\nA is 1 here.\n");
    const docId = "doc_1";
    await write(
      d,
      ".claims/config.json",
      JSON.stringify({ version: "v2", nonce: "deadbeef" }),
    );
    await write(
      d,
      `.claims/documents/${docId}.json`,
      JSON.stringify({
        id: docId,
        path: "doc.md",
        lifecycle: "active",
        edges: [],
        pristine: false,
      }),
    );
    await write(
      d,
      ".claims/propositions/prop_1.json",
      JSON.stringify({
        id: "prop_1",
        textCache: "A is 1 here",
        authoredTrust: "verified",
        fingerprint: "f",
      }),
    );
    const docText = "# Doc\n\nA is 1 here.\n";
    const s = docText.indexOf("A is 1 here");
    await write(
      d,
      ".claims/claims/asrt_1.json",
      JSON.stringify({
        id: "asrt_1",
        propositionId: "prop_1",
        documentId: docId,
        owner: "o",
        ref: "r",
        anchor: {
          doc: {
            file: "doc.md",
            selectors: [
              {
                kind: "text-quote",
                exact: "A is 1 here",
                prefix: "# Doc\n\n",
                suffix: ".\n",
              },
              { kind: "text-position", start: s, end: s + 11 },
              { kind: "inline-id", id: "x" },
            ],
          },
          code: [
            {
              file: "src/a.ts",
              selectors: [{ kind: "path", path: "src/a.ts" }],
            },
          ],
        },
        enforcement: "enforced",
        behavioral: true,
        evidenceBaseline: {},
        verifiers: [],
        attrs: {},
      }),
    );
    const res = await run(d, ["check"]);
    expect(res.stderr).toContain("upgraded claim store from v2 to v3");
    expect(res.code).toBe(0);
    expect(
      JSON.parse(await readFile(join(d, ".claims/config.json"), "utf8"))
        .version,
    ).toBe("v3");
    const again = await run(d, ["check"]);
    expect(again.stderr).not.toContain("upgraded");
  });
});
