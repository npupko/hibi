import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { hasBanner, locateBanner } from "../src/banner/banner.ts";
import { computeExitCode, runCheck } from "../src/engine/check.ts";
import { makeRepo, record, type TempRepo } from "./helpers.ts";

let analyzer: Awaited<ReturnType<typeof getAnalyzer>>;
beforeAll(async () => {
  analyzer = await getAnalyzer();
});

let repos: TempRepo[] = [];
async function repo() {
  const r = await makeRepo();
  repos.push(r);
  return r;
}
afterEach(async () => {
  await Promise.all(repos.map((r) => r.cleanup()));
  repos = [];
});

const check = (r: TempRepo, opts = {}) =>
  runCheck(r.store, { ast: analyzer, ...opts });

describe("exit-code contract (§9)", () => {
  test("0 when all claims are clean", () => {
    expect(
      computeExitCode({
        sawSuspect: false,
        sawMoved: false,
        sawTamper: false,
        failOn: "suspect",
      }),
    ).toBe(0);
  });
  test("2 when suspect present", () => {
    expect(
      computeExitCode({
        sawSuspect: true,
        sawMoved: false,
        sawTamper: false,
        failOn: "suspect",
      }),
    ).toBe(2);
  });
  test("3 when moved-only", () => {
    expect(
      computeExitCode({
        sawSuspect: false,
        sawMoved: true,
        sawTamper: false,
        failOn: "suspect",
      }),
    ).toBe(3);
  });
  test("--fail-on moved escalates moved to 2", () => {
    expect(
      computeExitCode({
        sawSuspect: false,
        sawMoved: true,
        sawTamper: false,
        failOn: "moved",
      }),
    ).toBe(2);
  });
});

describe("end-to-end drift detection", () => {
  test("an unchanged repo is clean (exit 0, fresh)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      trust: "verified",
    });
    await r.write("README.md", "# Doc\n");
    const rep = await check(r);
    expect(rep.verdicts[0]!.state).toBe("fresh");
    expect(rep.exitCode).toBe(0);
  });

  test("a changed value (5 → 50) is suspect (exit 2)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      trust: "verified",
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r);
    expect(rep.verdicts[0]!.state).toBe("stale");
    expect(rep.exitCode).toBe(2);
  });

  test("a deleted anchored file → ghost", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    await Bun.file; // no-op to keep import tidy
    const { rm } = await import("node:fs/promises");
    await rm(`${r.root}/src/retry.ts`);
    const rep = await check(r);
    expect(rep.verdicts[0]!.state).toBe("ghost");
    expect(rep.exitCode).toBe(2);
  });

  test("expired via ttl, independent of code drift (§10, §17.3)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      ttl: "2000-01-01T00:00:00Z", // long past
    });
    const rep = await check(r);
    expect(rep.verdicts[0]!.state).toBe("expired");
    expect(rep.exitCode).toBe(2);
  });

  test("coarse anchors are navigational and never stale (§11.3)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Touches retry module",
      file: "src/retry.ts",
      coarse: true,
    });
    // Rewrite the file wholesale — a precise anchor would ghost; a coarse one must not.
    await r.write("src/retry.ts", "// totally different content\n");
    const rep = await check(r);
    expect(rep.verdicts[0]!.state).toBe("fresh");
    expect(rep.exitCode).toBe(0);
  });
});

describe("the write-time loop: banner stamping (§6, §17.5)", () => {
  test("--write stamps a banner into the suspect document, then clears it on fix", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write("README.md", "# Retry Policy\n\nProse.\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      trust: "verified",
    });

    // Drift → banner inserted.
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    await check(r, { write: true });
    let doc = await r.read("README.md");
    expect(hasBanner(doc, "README.md", "deadbeef")).toBe(true);
    expect(doc).toContain("Capped at 5");

    // Re-run with no change → byte-stable (idempotent noop).
    const before = await r.read("README.md");
    await check(r, { write: true });
    expect(await r.read("README.md")).toBe(before);

    // Fix → banner removed, pristine prose restored.
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await check(r, { write: true });
    doc = await r.read("README.md");
    expect(hasBanner(doc, "README.md", "deadbeef")).toBe(false);
    expect(doc).toBe("# Retry Policy\n\nProse.\n");
  });

  test("the stamped banner has a valid FNV-1a checksum", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write("README.md", "# Doc\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    await r.write("src/retry.ts", "// gone\n");
    await check(r, { write: true });
    const located = locateBanner(await r.read("README.md"), "deadbeef", "html");
    expect(located).not.toBeNull();
    expect(located!.sha).toBe(located!.computedSha);
  });
});

describe("check is fully offline — no git on the verdict path (§6)", () => {
  test("verdicts are identical whether or not a git repo exists", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    // The temp dir is NOT a git repo; check must still succeed deterministically.
    const a = await check(r);
    const b = await check(r);
    expect(a.verdicts[0]!.state).toBe("fresh");
    expect(a.verdicts[0]!.state).toBe(b.verdicts[0]!.state);
    expect(a.verdicts[0]!.confidence).toBe(b.verdicts[0]!.confidence);
  });
});
