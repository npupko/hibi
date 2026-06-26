/**
 * `coverage` (§9) — doc-side coverage: which blocks of a document are backed by a
 * live claim's doc anchor vs uncovered. Reports a structural fact (block has a
 * claim or not); the ground-or-prune judgment is the caller's. Exercised through
 * the Engine facade (the same path the CLI uses).
 */
import { afterEach, describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Engine } from "../src/index.ts";

let dirs: string[] = [];
async function tmp(): Promise<string> {
  const d = await mkdtemp(join(tmpdir(), "ce-cov-"));
  dirs.push(d);
  return d;
}
afterEach(async () => {
  await Promise.all(dirs.map((d) => rm(d, { recursive: true, force: true })));
  dirs = [];
});

const DOC = [
  "# Heading",
  "",
  "The retry limit is five attempts.",
  "",
  "Some background prose with no code behind it.",
  "",
].join("\n");

async function repoWithDoc(): Promise<Engine> {
  const root = await tmp();
  await mkdir(join(root, "src"), { recursive: true });
  await writeFile(join(root, "src/x.ts"), "export const RETRY = 5;\n");
  await writeFile(join(root, "README.md"), DOC);
  return Engine.init(root);
}

describe("coverage", () => {
  test("a doc with no claims is entirely uncovered (ratio 0)", async () => {
    const engine = await repoWithDoc();
    const result = await engine.coverage("README.md");

    expect(result.summary.blocks).toBe(3);
    expect(result.summary.coveredBlocks).toBe(0);
    expect(result.summary.uncoveredBlocks).toBe(3);
    expect(result.summary.coverageRatio).toBe(0);
    expect(result.regions.every((r) => !r.covered)).toBe(true);
    expect(result.regions.every((r) => r.claimIds.length === 0)).toBe(true);
  });

  test("a claim covers exactly the block its doc anchor lands in", async () => {
    const engine = await repoWithDoc();
    const rec = await engine.record({
      docPath: "README.md",
      docQuote: "The retry limit is five attempts",
      code: [{ file: "src/x.ts", quote: "5" }],
      authoredTrust: "verified",
      ref: "r",
    });

    const result = await engine.coverage("README.md");
    expect(result.summary.coveredBlocks).toBe(1);
    expect(result.summary.uncoveredBlocks).toBe(2);
    expect(result.summary.coverageRatio).toBeCloseTo(1 / 3);

    const covered = result.regions.filter((r) => r.covered);
    expect(covered).toHaveLength(1);
    expect(covered[0]?.preview).toContain("retry limit");
    expect(covered[0]?.claimIds).toContain(rec.assertion.id);
    // The heading and the background prose stay uncovered — prune candidates.
    expect(
      result.regions.find((r) => r.preview.includes("background"))?.covered,
    ).toBe(false);
  });

  test("a retired claim no longer covers its block", async () => {
    const engine = await repoWithDoc();
    const rec = await engine.record({
      docPath: "README.md",
      docQuote: "The retry limit is five attempts",
      code: [{ file: "src/x.ts", quote: "5" }],
      authoredTrust: "verified",
      ref: "r",
    });
    expect((await engine.coverage("README.md")).summary.coveredBlocks).toBe(1);

    await engine.retire(rec.assertion.id);
    const after = await engine.coverage("README.md");
    expect(after.summary.coveredBlocks).toBe(0);
    expect(after.regions.every((r) => !r.covered)).toBe(true);
  });

  test("every block grounded → ratio 1", async () => {
    const root = await tmp();
    await mkdir(join(root, "src"), { recursive: true });
    await writeFile(join(root, "src/x.ts"), "export const RETRY = 5;\n");
    await writeFile(
      join(root, "README.md"),
      "First the retry limit is five.\n\nSecond RETRY is the symbol.\n",
    );
    const engine = await Engine.init(root);
    await engine.record({
      docPath: "README.md",
      docQuote: "First the retry limit is five",
      code: [{ file: "src/x.ts", quote: "5" }],
      authoredTrust: "verified",
      ref: "r",
    });
    await engine.record({
      docPath: "README.md",
      docQuote: "Second RETRY is the symbol",
      code: [{ file: "src/x.ts", quote: "RETRY" }],
      authoredTrust: "verified",
      ref: "r",
    });

    const result = await engine.coverage("README.md");
    expect(result.summary.blocks).toBe(2);
    expect(result.summary.coveredBlocks).toBe(2);
    expect(result.summary.coverageRatio).toBe(1);
  });
});
