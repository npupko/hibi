/**
 * ADR-003 Phase 2 engine fitness functions:
 *   D23 — record-time doc-quote guard (length floor + context uniqueness), at
 *         `record`, `reanchor`, and `record --from-file` (batch writes nothing).
 *   D24 — `reanchor --suggest` is read-only (store byte-identical) and refuses
 *         mutation flags.
 *   D26 — the reverse-import test suggestion appends to the declare-a-verifier
 *         remediation rationale, and never for a claim with declared verifiers.
 *   D30 — `coverage --fail-uncovered` exits 2 iff a block is uncovered.
 *   Check-purity — no `check` flag combination mutates `.claims/`.
 */

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
import { Engine } from "../src/index.ts";

const CLI = join(import.meta.dir, "..", "src", "cli", "index.ts");

let dirs: string[] = [];
async function repo(): Promise<string> {
  const d = await mkdtemp(join(tmpdir(), "ce-adr003-"));
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
async function run(
  cwd: string,
  args: string[],
): Promise<{ code: number; json: Record<string, unknown>; stdout: string }> {
  const proc = Bun.spawn(["bun", "run", CLI, ...args], {
    cwd,
    stdout: "pipe",
    stderr: "pipe",
  });
  const stdout = await new Response(proc.stdout).text();
  const code = await proc.exited;
  let json: Record<string, unknown> = {};
  try {
    json = JSON.parse(stdout.trim().split("\n").at(-1) ?? "");
  } catch {
    /* non-JSON */
  }
  return { code, json, stdout };
}
/** Read every file under `.claims/` into a path→bytes map, for byte-identical checks. */
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

describe("D23 — record-time doc-quote guard", () => {
  test("a doc quote shorter than 8 characters is rejected verbatim (record)", async () => {
    const d = await repo();
    await write(d, "doc.md", "# D\n\nabcdefg\n");
    const engine = await Engine.init(d);
    await expect(
      engine.record({ docPath: "doc.md", docQuote: "abcdefg" }),
    ).rejects.toThrow(
      "doc quote is shorter than 8 characters — too short to anchor reliably. Record a wider span (--doc-range) that covers the full sentence.",
    );
  });

  test("a repeated quote the surrounding context cannot disambiguate is rejected verbatim (record)", async () => {
    const d = await repo();
    // The exact sentence appears twice, each wrapped in an identical filler line
    // longer than the 48-char context window — so the stored context scores both
    // occurrences equally and cannot select one.
    const filler = "filler line long enough to exceed forty eight chars here";
    await write(
      d,
      "doc.md",
      `${filler}\nThe value is fixed here\n${filler}\nThe value is fixed here\n${filler}\n`,
    );
    const engine = await Engine.init(d);
    await expect(
      engine.record({ docPath: "doc.md", docQuote: "The value is fixed here" }),
    ).rejects.toThrow(
      "doc quote occurs 2 times in doc.md and the surrounding context does not select a single occurrence. Record a wider span (--doc-range), or add an inline ID and re-record.",
    );
  });

  test("a --from-file batch containing one bad spec writes nothing", async () => {
    const d = await repo();
    await write(d, "a.md", "# A\n\nA well formed sentence here.\n");
    await write(d, "b.md", "# B\n\nshort\n");
    await run(d, ["init"]);
    const specs = [
      { doc: "a.md", docQuote: "A well formed sentence here." },
      { doc: "b.md", docQuote: "short" }, // 5 chars → the whole batch must fail
    ];
    const proc = Bun.spawn(["bun", "run", CLI, "record", "--from-file", "-"], {
      cwd: d,
      stdin: new TextEncoder().encode(JSON.stringify(specs)),
      stdout: "pipe",
      stderr: "pipe",
    });
    const code = await proc.exited;
    expect(code).toBe(1);
    // The store recorded nothing — the batch rolled back.
    const claims = await readdir(join(d, ".claims", "claims"));
    expect(claims.filter((f) => f.endsWith(".json")).length).toBe(0);
  });

  test("reanchor also enforces the length floor", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# D\n\nThe A constant is one.\n");
    const engine = await Engine.init(d);
    const rec = await engine.record({
      docPath: "doc.md",
      docQuote: "The A constant is one.",
      code: [{ file: "src/a.ts", quote: "A = 1" }],
    });
    // Reword the doc to a too-short span and try to reanchor onto it.
    await write(d, "doc.md", "# D\n\ntiny\n");
    await expect(
      engine.reanchor(rec.assertion.id, { docQuote: "tiny" }),
    ).rejects.toThrow(
      "doc quote is shorter than 8 characters — too short to anchor reliably. Record a wider span (--doc-range) that covers the full sentence.",
    );
  });
});

describe("D24 — reanchor --suggest is read-only", () => {
  test("prints ranked candidates, exits 0, and leaves .claims byte-identical", async () => {
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
    const id = rec.json.claimId as string;
    // Orphan the code side; the doc sentence is still present in doc.md.
    await rm(join(d, "src/a.ts"), { force: true });

    const before = await snapshotStore(d);
    const sug = await run(d, ["reanchor", id, "--suggest", "--json"]);
    const after = await snapshotStore(d);

    expect(sug.code).toBe(0);
    expect(sug.json.action).toBe("reanchor-suggest");
    const candidates = sug.json.candidates as { doc: string }[];
    expect(candidates.length).toBeGreaterThan(0);
    expect(candidates[0]?.doc).toBe("doc.md");
    expect(sameStore(before, after)).toBe(true);
  });

  test("refuses mutation flags with the verbatim message", async () => {
    const d = await repo();
    await write(d, "doc.md", "# D\n\nThe A constant is one.\n");
    await run(d, ["init"]);
    const rec = await run(d, [
      "record",
      "--doc",
      "doc.md",
      "--doc-quote",
      "The A constant is one.",
    ]);
    const id = rec.json.claimId as string;
    const res = await run(d, [
      "reanchor",
      id,
      "--suggest",
      "--ref",
      "PR-9",
      "--json",
    ]);
    expect(res.code).toBe(1);
    expect(res.json.error).toBe(
      "--suggest is read-only and cannot be combined with mutation flags.",
    );
  });
});

describe("D26 — reverse-import test suggestion", () => {
  async function behavioralRepo() {
    const d = await repo();
    await write(
      d,
      "src/helper.ts",
      "export function helper() {\n  return 1;\n}\n",
    );
    await write(
      d,
      "src/thing.ts",
      'import { helper } from "./helper";\nexport function doThing() {\n  return helper();\n}\n',
    );
    await write(
      d,
      "test/thing.test.ts",
      'import { doThing } from "../src/thing";\nimport { test } from "bun:test";\ntest("x", () => { doThing(); });\n',
    );
    await write(d, "doc.md", "# D\n\ndoThing returns the helper result.\n");
    return d;
  }

  test("a behavioral at-risk claim with no verifier gets the covering test in its rationale", async () => {
    const d = await behavioralRepo();
    const engine = await Engine.init(d);
    await engine.record({
      docPath: "doc.md",
      docQuote: "doThing returns the helper result.",
      code: [{ file: "src/thing.ts", quote: "doThing" }],
      behavioral: true,
    });
    // Change the imported evidence file → the behavioral claim goes at-risk.
    await write(
      d,
      "src/helper.ts",
      "export function helper() {\n  return 2;\n}\n",
    );

    const report = await engine.check();
    const v = report.verdicts[0];
    expect(v?.behavior).toBe("at-risk");
    const action = v?.remediation?.actions.find((a) => a.id === "run-verifier");
    expect(action?.rationale).toContain(
      "tests that exercise this code: test/thing.test.ts",
    );
  });

  test("a behavioral at-risk claim WITH a declared verifier gets no suggestion", async () => {
    const d = await behavioralRepo();
    const engine = await Engine.init(d);
    await engine.record({
      docPath: "doc.md",
      docQuote: "doThing returns the helper result.",
      code: [{ file: "src/thing.ts", quote: "doThing" }],
      behavioral: true,
      verifiers: [{ kind: "command", ref: "bun test" }],
    });
    await write(
      d,
      "src/helper.ts",
      "export function helper() {\n  return 2;\n}\n",
    );

    const report = await engine.check();
    const v = report.verdicts[0];
    expect(v?.behavior).toBe("at-risk");
    const action = v?.remediation?.actions.find((a) => a.id === "run-verifier");
    // The action may still exist, but never carries the test-suggestion clause.
    expect(action?.rationale ?? "").not.toContain(
      "tests that exercise this code",
    );
  });
});

describe("D30 — coverage --fail-uncovered", () => {
  test("exits 2 when a block is uncovered and 0 when fully covered", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    // One grounded block → fully covered.
    await write(d, "covered.md", "The A constant is one here.\n");
    await run(d, ["init"]);
    await run(d, [
      "record",
      "--doc",
      "covered.md",
      "--doc-quote",
      "The A constant is one here.",
      "--code-file",
      "src/a.ts",
      "--code-quote",
      "A = 1",
    ]);
    const clean = await run(d, [
      "coverage",
      "--doc",
      "covered.md",
      "--fail-uncovered",
      "--json",
    ]);
    expect(clean.json.summary).toMatchObject({ uncoveredBlocks: 0 });
    expect(clean.code).toBe(0);

    // Add an ungrounded block → uncovered > 0 → exit 2.
    await write(
      d,
      "covered.md",
      "The A constant is one here.\n\nAn ungrounded extra block.\n",
    );
    const gated = await run(d, [
      "coverage",
      "--doc",
      "covered.md",
      "--fail-uncovered",
      "--json",
    ]);
    expect(
      (gated.json.summary as { uncoveredBlocks: number }).uncoveredBlocks,
    ).toBeGreaterThan(0);
    expect(gated.code).toBe(2);
    // Without the flag, the same doc exits 0.
    const soft = await run(d, ["coverage", "--doc", "covered.md", "--json"]);
    expect(soft.code).toBe(0);
  });
});

describe("check-purity invariant", () => {
  test("no check flag combination mutates .claims/", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await write(d, "doc.md", "# D\n\nThe A constant is one.\n");
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
      "--enforce",
    ]);
    const baseline = await snapshotStore(d);
    for (const args of [
      ["check"],
      ["check", "--write"],
      ["check", "--run-verifiers"],
      ["check", "--write", "--run-verifiers"],
      ["check", "--fail-on", "never"],
    ]) {
      await run(d, args);
      const after = await snapshotStore(d);
      expect(sameStore(baseline, after)).toBe(true);
    }
  });
});
