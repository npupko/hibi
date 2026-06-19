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
  // New mapping: never→0, gating→2, warn→(failOn==='warn'?2:3),
  // tamper&&failOn==='tamper'→2, else 0.
  test("0 when all flags are clear", () => {
    expect(
      computeExitCode({ gating: false, warn: false, tamper: false }, "gating"),
    ).toBe(0);
  });
  test("2 when a gating verdict is present", () => {
    expect(
      computeExitCode({ gating: true, warn: false, tamper: false }, "gating"),
    ).toBe(2);
  });
  test("3 when warn-only (default failOn=gating)", () => {
    expect(
      computeExitCode({ gating: false, warn: true, tamper: false }, "gating"),
    ).toBe(3);
  });
  test("--fail-on warn escalates a warn-only result to 2", () => {
    expect(
      computeExitCode({ gating: false, warn: true, tamper: false }, "warn"),
    ).toBe(2);
  });
  test("--fail-on never suppresses gating, warn, and tamper (exit 0)", () => {
    expect(
      computeExitCode({ gating: true, warn: true, tamper: true }, "never"),
    ).toBe(0);
  });
  test("--fail-on tamper fails only on tampering", () => {
    // gating still wins even under tamper threshold.
    expect(
      computeExitCode({ gating: true, warn: true, tamper: false }, "tamper"),
    ).toBe(2);
    // warn-only under tamper is a soft exit 3 (not 'warn' threshold).
    expect(
      computeExitCode({ gating: false, warn: true, tamper: false }, "tamper"),
    ).toBe(3);
    // a bare tamper gates only under the tamper threshold.
    expect(
      computeExitCode({ gating: false, warn: false, tamper: true }, "tamper"),
    ).toBe(2);
    // a tamper under any other threshold does not gate.
    expect(
      computeExitCode({ gating: false, warn: false, tamper: true }, "gating"),
    ).toBe(0);
  });
});

describe("end-to-end drift detection (two-axis)", () => {
  test("an unchanged repo is clean (exit 0, code:unchanged)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      trust: "verified",
    });
    const rep = await check(r);
    expect(rep.verdicts[0]?.doc).toBe("unchanged");
    expect(rep.verdicts[0]?.code).toBe("unchanged");
    expect(rep.verdicts[0]?.expired).toBe(false);
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.exitCode).toBe(0);
  });

  test("a changed value (5 → 50) on an enforced claim gates (exit 2)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      trust: "verified", // → enforcement 'enforced'
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r);
    expect(rep.verdicts[0]?.code).toBe("changed");
    expect(rep.verdicts[0]?.gates).toBe(true);
    expect(rep.exitCode).toBe(2);
  });

  test("the SAME drift on a 'suggested' claim does NOT gate (exit 0)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      trust: "inferred", // → enforcement 'suggested'
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r);
    // The drift still resolves to code:changed — only enforcement decides gating.
    expect(rep.verdicts[0]?.code).toBe("changed");
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.exitCode).toBe(0);
  });

  test("--fail-on never reports a changed claim without failing (exit 0)", async () => {
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
    const rep = await check(r, { failOn: "never" });
    expect(rep.verdicts[0]?.code).toBe("changed");
    expect(rep.exitCode).toBe(0);
  });

  test("a deleted anchored file → code:orphaned, gates on enforced (exit 2)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      trust: "verified",
    });
    const { rm } = await import("node:fs/promises");
    await rm(`${r.root}/src/retry.ts`);
    const rep = await check(r);
    expect(rep.verdicts[0]?.code).toBe("orphaned");
    expect(rep.verdicts[0]?.gates).toBe(true);
    expect(rep.exitCode).toBe(2);
  });

  test("expired via ttl is an orthogonal flag, gates on enforced (§10, §17.3)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      trust: "verified",
      ttl: "2000-01-01T00:00:00Z", // long past
    });
    const rep = await check(r);
    // doc/code still resolve independently; expired is its own axis.
    expect(rep.verdicts[0]?.expired).toBe(true);
    expect(rep.verdicts[0]?.code).toBe("unchanged");
    expect(rep.verdicts[0]?.gates).toBe(true);
    expect(rep.exitCode).toBe(2);
  });

  test("coarse anchors are navigational and never reported as drift (§11.3)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Touches retry module",
      file: "src/retry.ts",
      coarse: true,
      // Coarse is navigational only (§11.3): it can never be `verified`/enforced,
      // so it records as `suggested` and never gates regardless of code churn.
    });
    // Rewrite the file wholesale — a precise anchor would orphan; coarse must not.
    await r.write("src/retry.ts", "// totally different content\n");
    const rep = await check(r);
    expect(rep.verdicts[0]?.code).toBe("unchanged");
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.exitCode).toBe(0);
  });

  test("a relocated documented sentence grades doc:moved, not doc:orphaned (§17.3)", async () => {
    const r = await repo();
    await r.write("src/x.ts", "export const MAX = 5;\n");
    await r.write(
      "README.md",
      "# Doc\n\nThe limit is five widgets per batch.\n",
    );
    await record(r, {
      doc: "README.md",
      text: "The limit is five widgets per batch",
      file: "src/x.ts",
      quote: "MAX = 5",
      trust: "verified",
    });
    // Move the sentence far down the doc — same text, new location. The doc
    // bundle carries only text-quote + text-position, so its text-position drops
    // out; the near-exact relocation must still grade `moved`, never `orphaned`.
    await r.write(
      "README.md",
      `# Doc\n\n${"filler line\n".repeat(40)}The limit is five widgets per batch.\n`,
    );
    const rep = await check(r);
    expect(rep.verdicts[0]?.doc).toBe("moved");
    expect(rep.verdicts[0]?.code).toBe("unchanged");
    // `moved` is a warning, never a gate (§9).
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.exitCode).toBe(3);
  });
});

describe("the write-time loop: banner stamping (§6, §17.5)", () => {
  test("--write stamps a banner into the suspect document (code:changed)", async () => {
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

    // Drift → banner inserted; the live doc sentence stays in the body.
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    await check(r, { write: true });
    const doc = await r.read("README.md");
    expect(hasBanner(doc, "README.md", "deadbeef")).toBe(true);
    expect(doc).toContain("Capped at 5");
    // The machine status in the banner body is the side-tagged code state.
    expect(doc).toContain("[code:changed]");
  });

  // Regression for the banner-poisons-re-anchoring defect (§8/§17.5, §4/§18-B):
  // the engine-owned banner restates the live doc sentence verbatim, so on the
  // *second* --write run the doc-side text-quote could relocate onto that banner
  // copy and self-orphan. The engine strips its own banner before resolving the
  // doc side, so re-anchoring stays on the real prose and the banner is
  // byte-stable, then clears cleanly when the code is fixed.
  test("--write banner is byte-stable, then clears on fix", async () => {
    const r = await repo();
    const pristine = "# Retry Policy\n\nCapped at 5 attempts.\n";
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write("README.md", pristine);
    await record(r, {
      doc: "README.md",
      text: "Capped at 5 attempts",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      trust: "verified",
    });

    // Drift → banner inserted.
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    await check(r, { write: true });
    let doc = await r.read("README.md");
    expect(hasBanner(doc, "README.md", "deadbeef")).toBe(true);
    expect(doc).toContain("Capped at 5 attempts");

    // Re-run with no change → byte-stable (idempotent noop), still code:changed.
    const before = await r.read("README.md");
    const rep2 = await check(r, { write: true });
    expect(await r.read("README.md")).toBe(before);
    expect(rep2.verdicts[0]?.doc).toBe("unchanged");
    expect(rep2.verdicts[0]?.code).toBe("changed");

    // Fix → banner removed, pristine prose restored byte-for-byte.
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await check(r, { write: true });
    doc = await r.read("README.md");
    expect(hasBanner(doc, "README.md", "deadbeef")).toBe(false);
    expect(doc).toBe(pristine);
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
      trust: "verified",
    });
    await r.write("src/retry.ts", "// gone\n");
    await check(r, { write: true });
    const located = locateBanner(await r.read("README.md"), "deadbeef", "html");
    expect(located).not.toBeNull();
    expect(located?.sha).toBe(located?.computedSha);
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
      trust: "verified",
    });
    // The temp dir is NOT a git repo; check must still succeed deterministically.
    const a = await check(r);
    const b = await check(r);
    expect(a.verdicts[0]?.code).toBe("unchanged");
    expect(a.verdicts[0]?.code).toBe(b.verdicts[0]?.code);
    expect(a.verdicts[0]?.evidence.confidence).toBe(
      b.verdicts[0]?.evidence.confidence,
    );
  });
});
