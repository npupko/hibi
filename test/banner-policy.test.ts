import { describe, expect, test } from "bun:test";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  buildCompactBanner,
  hasBanner,
  isInstructionFile,
} from "../src/banner/banner.ts";
import { Engine } from "../src/index.ts";
import { makeRepo } from "./helpers.ts";

const NONCE = "deadbeef";

describe("compact instruction-file banner", () => {
  test("isInstructionFile matches the default instruction files by name/path", () => {
    expect(isInstructionFile("CLAUDE.md")).toBe(true);
    expect(isInstructionFile("AGENTS.md")).toBe(true);
    expect(isInstructionFile("nested/CLAUDE.md")).toBe(true);
    expect(isInstructionFile(".github/copilot-instructions.md")).toBe(true);
    expect(isInstructionFile("docs/guide.md")).toBe(false);
  });

  test("buildCompactBanner is a single pointer line inside the sentinel block", () => {
    const block = buildCompactBanner(3, "CLAUDE.md", NONCE, "html");
    expect(block).toContain(
      "STALE — 3 claim(s); run `hibi check --doc CLAUDE.md`",
    );
    expect(block).toContain(`HIBI:BEGIN v1 ${NONCE}`);
    // One body line only, no per-claim entries.
    const body = block
      .split("\n")
      .filter((l) => !l.includes("HIBI:") && l !== "<!--" && l !== "-->");
    expect(body).toHaveLength(1);
  });

  test("check --write stamps a one-line banner into an instruction file", async () => {
    const repo = await makeRepo();
    await repo.write("src/retry.ts", "export const MAX = 5;\n");
    await repo.write("AGENTS.md", "# Agents\n\nRetries are capped at 5.\n");
    const engine = await Engine.open(repo.root);
    await engine.record({
      docPath: "AGENTS.md",
      docQuote: "Retries are capped at 5.",
      code: [{ file: "src/retry.ts", region: { quote: "MAX = 5" } }],
      verified: true,
      ref: "PR-1",
    });
    // Drift the code so the claim goes suspect and a banner is stamped.
    await repo.write("src/retry.ts", "export const MAX = 9;\n");
    await engine.check({ write: true });

    const stamped = await repo.read("AGENTS.md");
    expect(hasBanner(stamped, "AGENTS.md", NONCE)).toBe(true);
    expect(stamped).toContain("STALE — ");
    expect(stamped).toContain("run `hibi check --doc AGENTS.md`");
    // The full per-claim block headline never appears in the compact banner.
    expect(stamped).not.toContain("STALE DOCUMENT —");
    await repo.cleanup();
  });
});

describe("pristine documents are never stamped", () => {
  test("check --write over a doc matching a config pristine glob is byte-identical on disk", async () => {
    const repo = await makeRepo();
    await writeFile(
      join(repo.store.dir, "config.json"),
      `${JSON.stringify({ version: "v3", nonce: NONCE, pristine: ["VENDOR.md"] }, null, 2)}\n`,
    );
    await repo.write("src/retry.ts", "export const MAX = 5;\n");
    await repo.write("VENDOR.md", "# Vendored\n\nRetries are capped at 5.\n");
    const engine = await Engine.open(repo.root);
    await engine.record({
      docPath: "VENDOR.md",
      docQuote: "Retries are capped at 5.",
      code: [{ file: "src/retry.ts", region: { quote: "MAX = 5" } }],
      verified: true,
      ref: "PR-1",
    });
    // Delete the anchored span so the code side orphans and the claim gates.
    await repo.write("src/retry.ts", "export const OTHER = 1;\n");

    const before = await repo.read("VENDOR.md");
    const report = await engine.check({ write: true });
    const after = await repo.read("VENDOR.md");

    expect(after).toBe(before);
    expect(hasBanner(after, "VENDOR.md", NONCE)).toBe(false);
    // The verdict is still computed and gates.
    const v = report.verdicts[0];
    expect(v?.gates).toBe(true);
    expect(report.exitCode).toBe(2);
    await repo.cleanup();
  });
});
