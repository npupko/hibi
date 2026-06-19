import { afterEach, describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { exists } from "../src/fs.ts";

const CLI = join(import.meta.dir, "..", "src", "cli", "index.ts");

interface Selector {
  kind?: string;
}

/** One side of an anchor's selector bundle (doc side or one code bundle). */
interface SelectorBundle {
  file?: string;
  selectors?: Selector[];
}

/**
 * Minimal shape of the CLI's JSON output that these tests actually read — the
 * two-axis, verdict-first model (§9). `assertion.anchor` is bidirectional:
 * `code[]` is an array of per-target bundles, each with its own `selectors[]`.
 */
interface CliJson {
  ok?: boolean;
  nonce?: string;
  store?: string;
  assertion?: { anchor?: { doc?: SelectorBundle; code?: SelectorBundle[] } };
  summary?: { clean?: number };
  verdicts?: {
    doc?: string;
    code?: string;
    expired?: boolean;
    gates?: boolean;
  }[];
  changedFiles?: string[];
  count?: number;
  current?: boolean;
  oldDoc?: { lifecycle?: string };
  type?: string;
  properties?: { anchor?: unknown };
}

interface RunResult {
  code: number;
  json: CliJson;
  stdout: string;
}

async function run(cwd: string, args: string[]): Promise<RunResult> {
  const proc = Bun.spawn(["bun", "run", CLI, ...args], {
    cwd,
    stdout: "pipe",
    stderr: "pipe",
  });
  const stdout = await new Response(proc.stdout).text();
  const code = await proc.exited;
  let json: CliJson = {};
  try {
    json = JSON.parse(stdout.trim().split("\n").at(-1) ?? "");
  } catch {
    /* non-JSON (e.g. usage) */
  }
  return { code, json, stdout };
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
async function write(root: string, rel: string, content: string) {
  const abs = join(root, rel);
  await mkdir(dirname(abs), { recursive: true });
  await writeFile(abs, content);
}
afterEach(async () => {
  await Promise.all(dirs.map((d) => rm(d, { recursive: true, force: true })));
  dirs = [];
});

/**
 * Pull the kinds present in the code side's first bundle. Span-first record
 * builds the doc-side bundle from the documented sentence and one code-side
 * bundle from the pinned code span; the code bundle carries the `value`
 * selector that trips a literal change (§4/§17.4).
 */
function codeSelectorKinds(json: CliJson): (string | undefined)[] {
  return (json.assertion?.anchor?.code?.[0]?.selectors ?? []).map(
    (s: Selector) => s.kind,
  );
}

describe("CLI end-to-end (§9)", () => {
  test("init → record → check clean → drift → exit 2 (enforced gates)", async () => {
    const d = await repo();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await write(d, "README.md", "# Doc\n\nCapped at 5 attempts.\n");

    const init = await run(d, ["init"]);
    expect(init.code).toBe(0);
    expect(init.json.nonce).toMatch(/^[0-9a-f]{8}$/);

    // Span-first record: --doc-quote locates the documented sentence (doc side),
    // --code-file/--code-quote pins the code (code side). `verified` trust makes
    // the claim ENFORCED, so its drift gates.
    const rec = await run(d, [
      "record",
      "--doc",
      "README.md",
      "--doc-quote",
      "Capped at 5",
      "--code-file",
      "src/retry.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
      "--trust",
      "verified",
      "--owner",
      "alice",
    ]);
    expect(rec.code).toBe(0);
    // The code-side bundle carries a `value` selector (5 → 50 trips it).
    expect(codeSelectorKinds(rec.json)).toContain("value");

    const clean = await run(d, ["check"]);
    expect(clean.code).toBe(0);
    expect(clean.json.summary?.clean).toBe(1);

    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const drifted = await run(d, ["check"]);
    expect(drifted.code).toBe(2);
    // Verdict-first: the code side resolved but its content changed.
    expect(drifted.json.verdicts?.[0]?.code).toBe("changed");
    expect(drifted.json.verdicts?.[0]?.gates).toBe(true);
  });

  test("a NON-enforced (suggested) claim never gates — drift is exit 0", async () => {
    // Precision-over-recall (§9/ADR-001): only ENFORCED claims gate. The default
    // trust (`inferred`) derives `suggested`, which is advisory — the same code
    // drift that gated above must NOT gate here.
    const d = await repo();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await write(d, "README.md", "# Doc\n\nCapped at 5 attempts.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--doc",
      "README.md",
      "--doc-quote",
      "Capped at 5",
      "--code-file",
      "src/retry.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
    ]);

    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const drifted = await run(d, ["check"]);
    expect(drifted.code).toBe(0);
    // The drift is still computed and reported — it just does not gate.
    expect(drifted.json.verdicts?.[0]?.code).toBe("changed");
    expect(drifted.json.verdicts?.[0]?.gates).toBe(false);
  });

  test("check --write stamps a banner that the consumer can see in the raw file", async () => {
    const d = await repo();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await write(d, "README.md", "# Doc\n\nCapped at 5 attempts.\n");
    await run(d, ["init"]);
    // --enforce makes the claim gating so the banner stamps.
    await run(d, [
      "record",
      "--doc",
      "README.md",
      "--doc-quote",
      "Capped at 5",
      "--code-file",
      "src/retry.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
      "--enforce",
    ]);
    await write(d, "src/retry.ts", "// removed\n");
    const res = await run(d, ["check", "--write"]);
    expect(res.code).toBe(2);
    const doc = await Bun.file(join(d, "README.md")).text();
    expect(doc).toContain("HIBI:BEGIN");
    expect(doc).toContain("STALE DOCUMENT");
  });

  test("diff --since scopes to changed files (the write-time loop)", async () => {
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
      "A is 1",
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
      "B is 2",
      "--code-file",
      "src/b.ts",
      "--code-quote",
      "B = 2",
    ]);
    await Bun.spawn(["git", "add", "-A"], { cwd: d }).exited;
    await Bun.spawn(["git", "commit", "-qm", "init"], { cwd: d }).exited;

    // Change only a.ts.
    await write(d, "src/a.ts", "export const A = 100;\n");
    const res = await run(d, ["diff", "--since", "HEAD"]);
    expect(res.json.changedFiles).toContain("src/a.ts");
    expect(res.json.changedFiles).not.toContain("src/b.ts");
    // Only the changed file's claim is evaluated.
    expect(res.json.verdicts?.length).toBe(1);
  });

  test("query --path reports claims covering a file", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# Doc\n\nA is 1 here.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "A is 1",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const res = await run(d, ["query", "--path", "src/a.ts"]);
    expect(res.code).toBe(0);
    expect(res.json.count).toBe(1);
  });

  test("status --doc is a read-time gate returning non-zero when suspect", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# Doc\n\nA is 1 here.\n");
    await run(d, ["init"]);
    // Enforced so its drift gates the read-time check.
    await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "A is 1",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
      "--enforce",
    ]);
    expect((await run(d, ["status", "--doc", "doc.md"])).code).toBe(0);
    await write(d, "src/a.ts", "// gone\n");
    const after = await run(d, ["status", "--doc", "doc.md"]);
    expect(after.code).toBe(2);
    // Two-axis: `current` is false iff a verdict gates (no single rollup state).
    expect(after.json.current).toBe(false);
  });

  test("supersede authors the edge and flips lifecycle", async () => {
    const d = await repo();
    await run(d, ["init"]);
    const res = await run(d, [
      "supersede",
      "--new",
      "v2.md",
      "--old",
      "v1.md",
      "--type",
      "supersedes",
    ]);
    expect(res.code).toBe(0);
    expect(res.json.oldDoc?.lifecycle).toBe("superseded");
  });

  test("schema emits generated JSON Schema by name", async () => {
    const d = await repo();
    const res = await run(d, ["schema", "--name", "Assertion"]);
    expect(res.code).toBe(0);
    expect(res.json.type).toBe("object");
    expect(res.json.properties?.anchor).toBeDefined();
  });

  test("unknown command is an operational error (exit 1)", async () => {
    const d = await repo();
    const res = await run(d, ["frobnicate"]);
    expect(res.code).toBe(1);
    expect(res.json.ok).toBe(false);
  });

  /**
   * Lock the machine contract (§9): the CLI is run with stdout piped (non-TTY),
   * so the *default* already resolves to compact JSON. Forcing `--json` must
   * produce byte-identical output — and the historical default *was* that exact
   * compact JSON, so this pins "`--json` ≡ pre-change default" for every command
   * a machine reads. `--json --pretty` is the same bytes, just indented.
   */
  test("--json output is byte-identical to the piped default (the machine contract)", async () => {
    const d = await repo();
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await write(d, "README.md", "# Doc\n\nCapped at 5 attempts.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--doc",
      "README.md",
      "--doc-quote",
      "Capped at 5",
      "--code-file",
      "src/retry.ts",
      "--code-quote",
      "MAX_ATTEMPTS = 5",
      "--trust",
      "verified",
    ]);

    // Read-only verbs: same store state, default-piped vs forced --json.
    const readOnly: string[][] = [
      ["check"],
      ["status", "--doc", "README.md"],
      ["status"], // the repo-wide overview path
      ["query", "--path", "src/retry.ts"],
      ["schema", "--name", "Assertion"],
      ["version"],
    ];
    for (const args of readOnly) {
      const def = await run(d, args);
      const forced = await run(d, [...args, "--json"]);
      expect(forced.stdout).toBe(def.stdout);
      expect(forced.code).toBe(def.code);
      // --json --pretty parses to the same object (indented, not different data).
      const pretty = await run(d, [...args, "--json", "--pretty"]);
      expect(JSON.parse(pretty.stdout)).toEqual(JSON.parse(def.stdout));
    }
  });

  test("--store-dir decouples the store from the anchor root", async () => {
    const d = await repo();
    const storeHome = await mkdtemp(join(tmpdir(), "ce-cli-store-"));
    dirs.push(storeHome);
    const storeDir = join(storeHome, "claims");
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# Doc\n\nA is 1 here.\n");

    const init = await run(d, ["init", "--store-dir", storeDir]);
    expect(init.code).toBe(0);
    expect(init.json.store).toBe(storeDir);

    const rec = await run(d, [
      "record",
      "--store-dir",
      storeDir,
      "--doc",
      "doc.md",
      "--doc-quote",
      "A is 1",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    expect(rec.code).toBe(0);

    // The store lives at the custom dir — nothing under <anchor>/.claims.
    expect(await exists(join(d, ".claims"))).toBe(false);
    expect(await exists(join(storeDir, "config.json"))).toBe(true);

    // Anchors still resolve against the anchor root through the far store.
    const clean = await run(d, ["check", "--store-dir", storeDir]);
    expect(clean.code).toBe(0);
    expect(clean.json.summary?.clean).toBe(1);
  });
});
