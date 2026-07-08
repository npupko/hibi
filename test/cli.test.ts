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
interface RemediationAction {
  id?: string;
  applicability?: string;
  effect?: string;
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
}
interface CliJson {
  ok?: boolean;
  action?: string;
  schemaVersion?: string;
  next?: string;
  nonce?: string;
  store?: string;
  claimId?: string;
  alreadyRetired?: boolean;
  // reanchor result: the post-reanchor per-side states sit at the top level
  doc?: string;
  code?: string;
  // record --from-file batch result
  batch?: boolean;
  assertion?: {
    enforcement?: string;
    anchor?: { doc?: SelectorBundle; code?: SelectorBundle[] };
  };
  summary?: { clean?: number; total?: number };
  verdicts?: CliVerdict[];
  changedFiles?: string[];
  count?: number;
  current?: boolean;
  state?: string;
  claims?: ListRow[];
  oldDoc?: { lifecycle?: string };
  type?: string;
  properties?: { anchor?: unknown };
  // lifecycle stranded-claim reporting + relocate / doctor envelopes
  warning?: string;
  existingClaims?: string[];
  strandedClaims?: string[];
  dryRun?: boolean;
  from?: string;
  to?: string;
  relocated?: { claimId?: string; doc?: string; code?: string }[];
  misses?: { claimId?: string; reason?: string }[];
  healthy?: boolean;
  counts?: {
    orphanedAnchors?: number;
    suggestedNoCode?: number;
    staleDocClaims?: number;
    duplicatePropositions?: number;
  };
  orphanedAnchors?: { claimId?: string; side?: string; path?: string }[];
  staleDocClaims?: { claimId?: string }[];
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
      "A is 1 here",
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
      "A is 1 here",
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
      "A is 1 here",
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

  // ── New agent-facing surface (§9) ──────────────────────────────────────────

  /**
   * Seed an enforced claim against MAX_ATTEMPTS, then change the value so the
   * single verdict gates with `code:changed`. Returns the claim id.
   */
  async function driftedRepo(): Promise<{ d: string; id: string }> {
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
      "5",
      "--trust",
      "verified",
    ]);
    const id = rec.json.claimId ?? "";
    await write(d, "src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    return { d, id };
  }

  test("every envelope is self-describing (schemaVersion) and mutations return the handle + next", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# Doc\n\nA is 1 here.\n");

    const init = await run(d, ["init"]);
    expect(init.json.schemaVersion).toBe("v2");
    expect(init.json.next).toBeDefined();

    const rec = await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "A is 1 here",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    expect(rec.json.schemaVersion).toBe("v2");
    expect(rec.json.claimId).toMatch(/^asrt_/);
    expect(rec.json.next).toBe("hibi check");

    const check = await run(d, ["check"]);
    expect(check.json.schemaVersion).toBe("v2");
  });

  test("concise check is lean (no evidence); --explain adds evidence + fingerprint", async () => {
    const { d } = await driftedRepo();

    const concise = await run(d, ["check"]);
    const cv = concise.json.verdicts?.[0];
    expect(cv?.code).toBe("changed");
    expect(cv?.evidence).toBeUndefined(); // bulky evidence dropped on the hot path
    expect(cv?.fingerprint).toBeUndefined();

    const explained = await run(d, ["check", "--explain"]);
    const ev = explained.json.verdicts?.[0];
    expect(ev?.evidence).toBeDefined();
    expect(ev?.fingerprint).toBeDefined();
    expect(ev?.advisories).toBeDefined();
  });

  test("a gating verdict carries a remediation menu with the id pre-filled", async () => {
    const { d, id } = await driftedRepo();
    const check = await run(d, ["check"]);
    const rem = check.json.verdicts?.[0]?.remediation;
    expect(rem).toBeDefined();
    expect(Array.isArray(rem?.actions)).toBe(true);
    // retire/reanchor commands carry the claim id verbatim.
    const retire = rem?.actions?.find((a) => a.id === "retire");
    expect(retire?.command).toBe(`hibi retire ${id}`);
    expect(retire?.effect).toBe("deterministic");
    const reanchor = rem?.actions?.find((a) => a.id === "reanchor");
    expect(reanchor?.command).toBe(`hibi reanchor ${id}`);
  });

  test("an orphan recommends retire and never pre-fills a bare reanchor command", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# Doc\n\nA is 1 here.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "A is 1 here",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
      "--enforce",
    ]);
    // Delete the code file → the code side orphans.
    await rm(join(d, "src/a.ts"), { force: true });
    const check = await run(d, ["check"]);
    const verdict = check.json.verdicts?.[0];
    const rem = verdict?.remediation;
    expect(rem?.recommended).toBe("retire");
    const reanchor = rem?.actions?.find((a) => a.id === "reanchor");
    // D24 — the orphan reanchor pre-fills the read-only `--suggest` pass (always
    // safe: it only lists candidate targets), never a bare mutating reanchor.
    expect(reanchor?.command).toBe(
      `hibi reanchor ${verdict?.assertionId} --suggest`,
    );
  });

  test("--no-hints / HIBI_ADVICE=0 strips the remediation block", async () => {
    const { d } = await driftedRepo();
    const flagged = await run(d, ["check", "--no-hints"]);
    expect(flagged.json.verdicts?.[0]?.remediation).toBeUndefined();
    expect(flagged.json.verdicts?.[0]?.gates).toBe(true); // the decision still leads
  });

  test("the behavioral carve-out keeps a 1-line `changed` summary on the concise path", async () => {
    const { d } = await driftedRepo();
    const check = await run(d, ["check"]);
    const v = check.json.verdicts?.[0];
    expect(v?.behavior).toBe("at-risk");
    expect(v?.changed).toContain("src/retry.ts"); // path + kind, no --explain needed
  });

  test("retire flips enforcement, is idempotent, and stops the claim gating", async () => {
    const { d, id } = await driftedRepo();
    expect((await run(d, ["check"])).code).toBe(2);

    const first = await run(d, ["retire", id]);
    expect(first.code).toBe(0);
    expect(first.json.action).toBe("retire");
    expect(first.json.alreadyRetired).toBe(false);
    expect(first.json.assertion?.enforcement).toBe("retired");
    expect(first.json.next).toBe("hibi check");

    // A retired claim no longer gates.
    expect((await run(d, ["check"])).code).toBe(0);

    // Idempotent: a second retire is a no-op success.
    const second = await run(d, ["retire", id]);
    expect(second.code).toBe(0);
    expect(second.json.alreadyRetired).toBe(true);
  });

  test("retire requires a claim-id positional (operational error otherwise)", async () => {
    const d = await repo();
    await run(d, ["init"]);
    const res = await run(d, ["retire"]);
    expect(res.code).toBe(1);
    expect(res.json.ok).toBe(false);
  });

  test("list returns lean triage rows and filters by --state", async () => {
    const { d, id } = await driftedRepo();

    const all = await run(d, ["list"]);
    expect(all.code).toBe(0);
    expect(all.json.action).toBe("list");
    expect(all.json.count).toBe(1);
    const row = all.json.claims?.[0];
    expect(row?.claimId).toBe(id);
    expect(row?.status).toBe("code:changed");
    expect(row?.severity).toBe("gating");
    expect(row?.gates).toBe(true);
    expect(row?.documentPath).toBe("README.md");
    expect(row?.codePath).toBe("src/retry.ts");

    const gating = await run(d, ["list", "--state", "gating"]);
    expect(gating.json.count).toBe(1);
    const clean = await run(d, ["list", "--state", "clean"]);
    expect(clean.json.count).toBe(0);
  });

  test("list --state rejects an unknown state (operational error)", async () => {
    const d = await repo();
    await run(d, ["init"]);
    const res = await run(d, ["list", "--state", "bogus"]);
    expect(res.code).toBe(1);
    expect(res.json.ok).toBe(false);
  });

  test("list --no-hints drops the recommended action from rows", async () => {
    // A `moved` verdict recommends `reanchor` (non-null), so --no-hints is observable.
    const d = await repo();
    await write(d, "src/a.ts", "export const MAX = 5;\n");
    await write(d, "doc.md", "# Doc\n\nMax is 5 here.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "Max is 5",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "MAX = 5",
      "--trust",
      "verified",
    ]);
    // Relocate the anchored line far (intact content) → code:moved.
    await write(
      d,
      "src/a.ts",
      `${"// prologue\n".repeat(4)}export const MAX = 5;\n`,
    );
    const withHints = await run(d, ["list", "--state", "warning"]);
    expect(withHints.json.claims?.[0]?.status).toBe("code:moved");
    expect(withHints.json.claims?.[0]?.recommended).toBe("reanchor");
    const noHints = await run(d, ["list", "--state", "warning", "--no-hints"]);
    expect(noHints.json.claims?.[0]?.recommended).toBeNull();
  });

  test("list reports a retired claim as `retired`, not as live drift", async () => {
    const { d, id } = await driftedRepo();
    await run(d, ["retire", id]);
    const all = await run(d, ["list"]);
    const row = all.json.claims?.find((r) => r.claimId === id);
    expect(row?.status).toBe("retired");
    expect(row?.severity).toBe("clean");
    expect(row?.recommended).toBeNull();
  });

  test("list reflects document lifecycle in the status (not a bare `unchanged`)", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "v1.md", "# V1\n\nA is 1 here.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--doc",
      "v1.md",
      "--doc-quote",
      "A is 1 here",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const id = rec.json.claimId ?? "";
    // Supersede v1.md → its lifecycle flips to `superseded`; the anchor is intact.
    await run(d, [
      "supersede",
      "--new",
      "v2.md",
      "--old",
      "v1.md",
      "--type",
      "supersedes",
    ]);
    const all = await run(d, ["list"]);
    const row = all.json.claims?.find((r) => r.claimId === id);
    expect(row?.status).toBe("superseded");
  });

  test("list codePath reflects the code file that drifted", async () => {
    const { d, id } = await driftedRepo();
    const all = await run(d, ["list", "--state", "gating"]);
    const row = all.json.claims?.find((r) => r.claimId === id);
    expect(row?.codePath).toBe("src/retry.ts"); // the file changedEvidence names
  });

  test("record --from-file batch-records a JSON array in one pass (§9)", async () => {
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
    await write(
      d,
      "batch.json",
      JSON.stringify([
        {
          doc: "docs/conf.md",
          docQuote: "The cache TTL is 60000ms",
          codeFile: "src/conf.ts",
          codeQuote: "TTL_MS = 60000",
          trust: "verified",
          owner: "alice",
        },
        {
          doc: "docs/conf.md",
          docQuote: "Retries default to 3",
          codeFile: "src/conf.ts",
          codeQuote: "RETRIES = 3",
          trust: "verified",
          owner: "alice",
        },
      ]),
    );
    const rec = await run(d, ["record", "--from-file", "batch.json"]);
    expect(rec.code).toBe(0);
    expect(rec.json.batch).toBe(true);
    expect(rec.json.count).toBe(2);

    const chk = await run(d, ["check"]);
    expect(chk.code).toBe(0);
    expect(chk.json.summary?.clean).toBe(2);
  });

  test("record --from-file fails the whole batch on a malformed item, writing nothing (§9)", async () => {
    const d = await repo();
    await write(d, "src/conf.ts", "export const TTL_MS = 60000;\n");
    await write(d, "docs/conf.md", "# Config\n\nThe cache TTL is 60000ms.\n");
    await run(d, ["init"]);
    await write(
      d,
      "batch.json",
      JSON.stringify([
        {
          doc: "docs/conf.md",
          docQuote: "The cache TTL is 60000ms",
          codeFile: "src/conf.ts",
          codeQuote: "TTL_MS = 60000",
          trust: "verified",
        },
        { doc: "docs/conf.md" }, // malformed: no doc span
      ]),
    );
    const rec = await run(d, ["record", "--from-file", "batch.json"]);
    expect(rec.code).toBe(1);
    expect(rec.json.ok).toBe(false);

    // Phase-1 validation: the valid first item was NOT written.
    const chk = await run(d, ["check"]);
    expect(chk.json.summary?.total ?? 0).toBe(0);
  });

  test("reanchor --doc relocates the doc anchor to a different file, surviving deletion of the old one (§9)", async () => {
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
      "--trust",
      "verified",
    ]);
    const id = rec.json.claimId ?? "";

    const re = await run(d, [
      "reanchor",
      id,
      "--doc",
      "docs/a.md",
      "--doc-quote",
      "The A constant is one",
    ]);
    expect(re.code).toBe(0);
    expect(re.json.doc).toBe("unchanged"); // re-resolved against the new file
    expect(re.json.code).toBe("unchanged"); // code side untouched

    // The claim moved off wip.md — deleting it no longer orphans the claim.
    await rm(join(d, "wip.md"));
    const chk = await run(d, ["check"]);
    expect(chk.code).toBe(0);
    expect(chk.json.verdicts?.[0]?.doc).toBe("unchanged");
  });

  test("reanchor --doc-range re-resolves via a line range without dropping the bounds (§9)", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# A\n\nThe A constant is one.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "The A constant is one",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
      "--trust",
      "verified",
    ]);
    const id = rec.json.claimId ?? "";

    // Line 3 holds the documented sentence; a line range must resolve to it.
    const re = await run(d, ["reanchor", id, "--doc-range", "L3:L3"]);
    expect(re.code).toBe(0);
    expect(re.json.doc).toBe("unchanged");
  });

  test("reanchor --doc to a different file without an explicit span is rejected, not silently re-matched (§9)", async () => {
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
      "--trust",
      "verified",
    ]);
    const id = rec.json.claimId ?? "";

    // No --doc-quote: re-matching the old selectors against docs/a.md could
    // coincidentally latch onto the wrong sentence — demand a deliberate span.
    const re = await run(d, ["reanchor", id, "--doc", "docs/a.md"]);
    expect(re.code).toBe(1);
    expect(re.json.ok).toBe(false);
  });

  test("record --from-file rolls back fully when a later item fails to resolve (§9)", async () => {
    const d = await repo();
    await write(d, "src/conf.ts", "export const TTL_MS = 60000;\n");
    await write(d, "docs/conf.md", "# Config\n\nThe cache TTL is 60000ms.\n");
    await run(d, ["init"]);
    await write(
      d,
      "batch.json",
      JSON.stringify([
        {
          doc: "docs/conf.md",
          docQuote: "The cache TTL is 60000ms",
          codeFile: "src/conf.ts",
          codeQuote: "TTL_MS = 60000",
          trust: "verified",
        },
        // Passes phase-1 structural validation, but its quote is absent from the
        // file, so it throws during phase-2 record — after item 0 was written.
        { doc: "docs/conf.md", docQuote: "NOT PRESENT IN THE FILE" },
      ]),
    );
    const rec = await run(d, ["record", "--from-file", "batch.json"]);
    expect(rec.code).toBe(1);
    expect(rec.json.ok).toBe(false);

    // The valid first item must NOT survive — a failed batch leaves no partial store.
    const chk = await run(d, ["check"]);
    expect(chk.json.summary?.total ?? 0).toBe(0);
  });

  test("record --from-file rejects an item with an empty text and no doc span (§9)", async () => {
    const d = await repo();
    await write(d, "docs/x.md", "# X\n\nsomething.\n");
    await run(d, ["init"]);
    await write(
      d,
      "batch.json",
      JSON.stringify([{ doc: "docs/x.md", text: "" }]),
    );
    const rec = await run(d, ["record", "--from-file", "batch.json"]);
    expect(rec.code).toBe(1);
    expect(rec.json.ok).toBe(false);

    const chk = await run(d, ["check"]);
    expect(chk.json.summary?.total ?? 0).toBe(0);
  });

  // ── Tier-1/2/3 silent-orphan hardening ──

  test("supersede reports strandedClaims and points next at relocate", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "v1.md", "# V1\n\nA is one.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
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
    const sup = await run(d, [
      "supersede",
      "--new",
      "v2.md",
      "--old",
      "v1.md",
      "--type",
      "supersedes",
    ]);
    expect(sup.code).toBe(0);
    expect(sup.json.strandedClaims).toEqual([rec.json.claimId ?? ""]);
    expect(sup.json.next).toContain("relocate");
  });

  test("relocate re-homes a stranded claim, then check settles clean", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "v1.md", "# V1\n\nA is one.\n");
    await run(d, ["init"]);
    await run(d, [
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
    // Copy the documented sentence into the successor.
    await write(d, "v2.md", "# V2\n\nA is one.\n");
    const rel = await run(d, [
      "relocate",
      "--from",
      "v1.md",
      "--to",
      "v2.md",
      "--json",
    ]);
    expect(rel.code).toBe(0);
    expect(rel.json.relocated?.length).toBe(1);
    expect(rel.json.misses?.length).toBe(0);
    const chk = await run(d, ["check"]);
    expect(chk.code).toBe(0);
  });

  test("doctor exits 0 even with an orphan, and populates categories", async () => {
    const d = await repo();
    await write(d, "src/gone.ts", "export const X = 1;\n");
    await write(d, "o.md", "# O\n\nOrphan here.\n");
    await run(d, ["init"]);
    await run(d, [
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
    const doc = await run(d, ["doctor", "--json"]);
    // Purely informational — never gates.
    expect(doc.code).toBe(0);
    expect(doc.json.healthy).toBe(false);
    expect(doc.json.counts?.orphanedAnchors).toBeGreaterThan(0);
  });

  test("record warns when a claim lands suggested, and flags a duplicate proposition", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "a.md", "# A\n\nA is one.\n");
    await write(d, "b.md", "# B\n\nA is one.\n");
    await run(d, ["init"]);
    // Default trust (inferred) → suggested → warning present.
    const first = await run(d, [
      "record",
      "--doc",
      "a.md",
      "--doc-quote",
      "A is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    expect(first.json.warning).toContain("suggested");
    expect(first.json.existingClaims).toEqual([]);

    // Same sentence on a different doc → duplicate proposition surfaced.
    const second = await run(d, [
      "record",
      "--doc",
      "b.md",
      "--doc-quote",
      "A is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    expect(second.json.existingClaims).toEqual([first.json.claimId ?? ""]);
    expect(second.json.next).toContain("reanchor");
  });

  test("list --state orphaned and --state suggested filter correctly", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "src/gone.ts", "export const X = 1;\n");
    await write(d, "ok.md", "# OK\n\nA is one.\n");
    await write(d, "o.md", "# O\n\nOrphan here.\n");
    await run(d, ["init"]);
    // A healthy suggested claim.
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
    // An orphan-to-be.
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

    const orphaned = await run(d, ["list", "--state", "orphaned"]);
    expect(orphaned.json.claims?.map((c) => c.claimId)).toEqual([
      orphan.json.claimId ?? "",
    ]);

    const suggested = await run(d, ["list", "--state", "suggested"]);
    // Both records are `suggested` (default inferred trust).
    expect(suggested.json.claims?.length).toBe(2);

    // Retiring the orphan must drain it from `--state orphaned` AND from doctor —
    // otherwise the documented cleanup loop never converges (review finding).
    await run(d, ["retire", orphan.json.claimId ?? ""]);
    const afterRetire = await run(d, ["list", "--state", "orphaned"]);
    expect(afterRetire.json.claims?.length).toBe(0);
    const doc = await run(d, ["doctor", "--json"]);
    expect(doc.json.counts?.orphanedAnchors).toBe(0);
  });

  test("retract --dry-run and archive --dry-run leave the store + docs untouched", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "a.md", "# A\n\nA is one.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--doc",
      "a.md",
      "--doc-quote",
      "A is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);

    const snapshot = async (): Promise<Map<string, string>> => {
      const out = new Map<string, string>();
      const walk = async (dir: string) => {
        for (const ent of await readdir(dir, { withFileTypes: true })) {
          const abs = join(dir, ent.name);
          if (ent.isDirectory()) await walk(abs);
          else out.set(abs, await readFile(abs, "utf8"));
        }
      };
      await walk(join(d, ".claims"));
      out.set("a.md", await readFile(join(d, "a.md"), "utf8"));
      return out;
    };

    const before = await snapshot();
    const ret = await run(d, ["retract", "--doc", "a.md", "--dry-run"]);
    expect(ret.json.dryRun).toBe(true);
    const arch = await run(d, ["archive", "--doc", "a.md", "--dry-run"]);
    expect(arch.json.dryRun).toBe(true);
    const after = await snapshot();

    expect([...after.keys()].sort()).toEqual([...before.keys()].sort());
    for (const [k, v] of before) expect(after.get(k)).toBe(v);
    // The archive tombstone/move must not have happened.
    expect(await exists(join(d, "archive", "a.md"))).toBe(false);
  });

  test("--ids-only emits a bare newline-delimited id list", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "a.md", "# A\n\nA is one.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--doc",
      "a.md",
      "--doc-quote",
      "A is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const ids = await run(d, ["list", "--ids-only"]);
    expect(ids.code).toBe(0);
    expect(ids.stdout.trim()).toBe(rec.json.claimId ?? "");
  });

  test("--dry-run leaves the .claims/ store byte-identical", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "a.md", "# A\n\nA is one.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--doc",
      "a.md",
      "--doc-quote",
      "A is one.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const id = rec.json.claimId ?? "";

    const snapshot = async (): Promise<Map<string, string>> => {
      const out = new Map<string, string>();
      const walk = async (dir: string) => {
        for (const ent of await readdir(dir, { withFileTypes: true })) {
          const abs = join(dir, ent.name);
          if (ent.isDirectory()) await walk(abs);
          else out.set(abs, await readFile(abs, "utf8"));
        }
      };
      await walk(join(d, ".claims"));
      return out;
    };

    const before = await snapshot();
    // A dry-run reanchor must not touch the store.
    const dry = await run(d, [
      "reanchor",
      id,
      "--doc-quote",
      "A is one.",
      "--dry-run",
    ]);
    expect(dry.code).toBe(0);
    expect(dry.json.dryRun).toBe(true);
    // And a dry-run retire likewise.
    await run(d, ["retire", id, "--dry-run"]);
    const after = await snapshot();

    expect([...after.keys()].sort()).toEqual([...before.keys()].sort());
    for (const [k, v] of before) expect(after.get(k)).toBe(v);
  });
});
