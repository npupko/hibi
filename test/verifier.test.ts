import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { Engine } from "../src/index.ts";
import { verifierArgv } from "../src/resolver/builtin/command-runner.ts";
import { makeRepo } from "./helpers.ts";

/**
 * The built-in command verifier runner (ADR-002 D13). Two fitness functions:
 *   - e2e: a `command:` verifier reaches `supported` on exit 0 and `refuted`
 *     (gating) on exit 2, under `check --run-verifiers`;
 *   - verifier-safety invariant: NO verifier process spawns on `status`,
 *     `query`, or plain `check` — only the explicit `--run-verifiers` opt-in.
 */
describe("command verifier runner (D13)", () => {
  async function seed(ref: string) {
    const repo = await makeRepo();
    await repo.write("src/op.ts", "export const N = 1;\n");
    await repo.write("doc.md", "# Doc\n\nThe operation retries on failure.\n");
    const engine = await Engine.open(repo.root);
    const rec = await engine.record({
      docPath: "doc.md",
      docQuote: "The operation retries on failure.",
      code: [{ file: "src/op.ts", quote: "N = 1" }],
      verifiers: [{ kind: "command", ref }],
      authoredTrust: "verified",
      ref: "PR-1",
      enforcement: "enforced",
    });
    return { repo, engine, claimId: rec.assertion.id };
  }

  test("exit 0 → supported under --run-verifiers", async () => {
    const { repo, engine, claimId } = await seed("exit 0");
    const report = await engine.check({ runVerifiers: true });
    const v = report.verdicts.find((x) => x.assertionId === claimId);
    expect(v?.behavior).toBe("supported");
    expect(v?.gates).toBe(false);
    await repo.cleanup();
  });

  test("exit 2 → refuted (gating on an enforced claim) under --run-verifiers", async () => {
    const { repo, engine, claimId } = await seed("exit 2");
    const report = await engine.check({ runVerifiers: true });
    const v = report.verdicts.find((x) => x.assertionId === claimId);
    expect(v?.behavior).toBe("refuted");
    expect(v?.gates).toBe(true);
    expect(report.exitCode).toBe(2);
    await repo.cleanup();
  });

  test("verifier-safety: no process spawns on plain check / status / query", async () => {
    const repo = await makeRepo();
    await repo.write("src/op.ts", "export const N = 1;\n");
    await repo.write("doc.md", "# Doc\n\nThe operation retries on failure.\n");
    const engine = await Engine.open(repo.root);
    // A verifier that leaves an observable side effect if (and only if) it runs.
    await engine.record({
      docPath: "doc.md",
      docQuote: "The operation retries on failure.",
      code: [{ file: "src/op.ts", quote: "N = 1" }],
      verifiers: [{ kind: "command", ref: "touch ran.sentinel" }],
      authoredTrust: "verified",
      ref: "PR-1",
      enforcement: "enforced",
    });
    const sentinel = join(repo.root, "ran.sentinel");

    // None of these opt into verifiers → the process must never spawn.
    await engine.check();
    await engine.status("doc.md");
    await engine.query("src/op.ts");
    await engine.doctor();
    expect(existsSync(sentinel)).toBe(false);

    // The explicit opt-in DOES run it.
    await engine.check({ runVerifiers: true });
    expect(existsSync(sentinel)).toBe(true);
    await repo.cleanup();
  });
});

describe("verifierArgv — cross-platform shell dispatch (PRD §12)", () => {
  test("Windows dispatches via cmd /c", () => {
    expect(verifierArgv("win32", "bun test retry")).toEqual([
      "cmd",
      "/c",
      "bun test retry",
    ]);
  });

  test("POSIX platforms dispatch via sh -c", () => {
    for (const platform of ["linux", "darwin"] as const) {
      expect(verifierArgv(platform, "bun test retry")).toEqual([
        "sh",
        "-c",
        "bun test retry",
      ]);
    }
  });

  test("the ref is passed through verbatim as the single command arg", () => {
    const ref = 'bun test --filter "retry & backoff"';
    expect(verifierArgv("darwin", ref)[2]).toBe(ref);
    expect(verifierArgv("win32", ref)[2]).toBe(ref);
  });
});
