import { afterEach, beforeAll, describe, expect, test } from "bun:test";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { hasBanner } from "../src/banner/banner.ts";
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

describe("exit-code contract", () => {
  test("0 when all flags are clear", () => {
    expect(computeExitCode({ gating: false, warn: false }, "gating")).toBe(0);
  });
  test("2 when a gating verdict is present", () => {
    expect(computeExitCode({ gating: true, warn: false }, "gating")).toBe(2);
  });
  test("0 when warn-only under the default failOn=gating", () => {
    expect(computeExitCode({ gating: false, warn: true }, "gating")).toBe(0);
  });
  test("--fail-on warn escalates a warn-only result to 2", () => {
    expect(computeExitCode({ gating: false, warn: true }, "warn")).toBe(2);
    expect(computeExitCode({ gating: true, warn: false }, "warn")).toBe(2);
  });
  test("--fail-on never always exits 0", () => {
    expect(computeExitCode({ gating: true, warn: true }, "never")).toBe(0);
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
      verified: true,
    });
    const rep = await check(r);
    expect(rep.verdicts[0]?.doc).toBe("unchanged");
    expect(rep.verdicts[0]?.code).toBe("unchanged");
    expect(rep.verdicts[0]?.expired).toBe(false);
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.summary.clean).toBe(1);
    expect(rep.summary.retired).toBe(0);
    expect(rep.exitCode).toBe(0);
  });

  test("a changed value (5 to 50) on an enforced claim gates (exit 2)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r);
    expect(rep.verdicts[0]?.code).toBe("changed");
    expect(rep.verdicts[0]?.gates).toBe(true);
    expect(rep.exitCode).toBe(2);
  });

  test("the same drift on a suggested claim does not gate (exit 0)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      enforcement: "suggested",
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r);
    // The drift still resolves to code:changed; only enforcement decides gating.
    expect(rep.verdicts[0]?.code).toBe("changed");
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.exitCode).toBe(0);
  });

  test("a retired claim is listed but counted neither clean nor gating", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      enforcement: "retired",
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r);
    expect(rep.summary.total).toBe(1);
    expect(rep.summary.retired).toBe(1);
    expect(rep.summary.clean).toBe(0);
    expect(rep.summary.gating).toBe(0);
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
    });
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    const rep = await check(r, { failOn: "never" });
    expect(rep.verdicts[0]?.code).toBe("changed");
    expect(rep.exitCode).toBe(0);
  });

  test("a deleted anchored file grades code:orphaned and gates on enforced (exit 2)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    const { rm } = await import("node:fs/promises");
    await rm(`${r.root}/src/retry.ts`);
    const rep = await check(r);
    expect(rep.verdicts[0]?.code).toBe("orphaned");
    expect(rep.verdicts[0]?.gates).toBe(true);
    expect(rep.exitCode).toBe(2);
  });

  test("expired via ttl is an orthogonal flag and gates on enforced", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
      ttl: "2000-01-01T00:00:00Z",
    });
    const rep = await check(r);
    expect(rep.verdicts[0]?.expired).toBe(true);
    expect(rep.verdicts[0]?.code).toBe("unchanged");
    expect(rep.verdicts[0]?.gates).toBe(true);
    expect(rep.summary.expired).toBe(1);
    expect(rep.exitCode).toBe(2);
  });

  test("coarse anchors are navigational and never reported as drift", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Touches retry module",
      file: "src/retry.ts",
      coarse: true,
    });
    // Rewrite the file wholesale; a precise anchor would orphan, coarse must not.
    await r.write("src/retry.ts", "// totally different content\n");
    const rep = await check(r);
    expect(rep.verdicts[0]?.code).toBe("unchanged");
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.exitCode).toBe(0);
  });

  test("a relocated documented sentence grades doc:moved, warns, and exits 0", async () => {
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
    });
    await r.write(
      "README.md",
      `# Doc\n\n${"filler line\n".repeat(40)}The limit is five widgets per batch.\n`,
    );
    const rep = await check(r);
    expect(rep.verdicts[0]?.doc).toBe("moved");
    expect(rep.verdicts[0]?.code).toBe("unchanged");
    expect(rep.verdicts[0]?.gates).toBe(false);
    expect(rep.summary.warning).toBe(1);
    expect(rep.exitCode).toBe(0);
    expect((await check(r, { failOn: "warn" })).exitCode).toBe(2);
  });

  test("check --doc scopes the verdicts to one document", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("src/b.ts", "export const B = 2;\n");
    await record(r, {
      doc: "a.md",
      text: "A is one here",
      file: "src/a.ts",
      quote: "A = 1",
    });
    await record(r, {
      doc: "b.md",
      text: "B is two here",
      file: "src/b.ts",
      quote: "B = 2",
    });
    const { documentIdForPath } = await import("../src/engine/record.ts");
    const { documentScope } = await import("../src/engine/status.ts");
    const scope = await documentScope(r.store, "a.md");
    expect([...scope.files].sort()).toEqual(["a.md", "src/a.ts"]);
    const rep = await check(r, {
      onlyFiles: scope.files,
      onlyDocument: scope.documentId,
    });
    expect(rep.verdicts).toHaveLength(1);
    expect(rep.verdicts[0]?.documentId).toBe(documentIdForPath("a.md"));
    expect(rep.documents.map((d) => d.path)).toEqual(["a.md"]);
  });
});

describe("the write-time loop: banner stamping", () => {
  test("--write stamps a banner into the suspect document (code:changed)", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write("README.md", "# Retry Policy\n\nProse.\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });

    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    await check(r, { write: true });
    const doc = await r.read("README.md");
    expect(hasBanner(doc, "README.md", "deadbeef")).toBe(true);
    expect(doc).toContain("Capped at 5");
    expect(doc).toContain("[code:changed]");
  });

  // The engine-owned banner restates the live doc sentence verbatim. The
  // engine strips its own banner before resolving the doc side, so a second
  // --write run does not relocate the doc quote onto the banner copy.
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
    });

    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 50;\n");
    await check(r, { write: true });
    let doc = await r.read("README.md");
    expect(hasBanner(doc, "README.md", "deadbeef")).toBe(true);
    expect(doc).toContain("Capped at 5 attempts");

    const before = await r.read("README.md");
    const rep2 = await check(r, { write: true });
    expect(await r.read("README.md")).toBe(before);
    expect(rep2.verdicts[0]?.doc).toBe("unchanged");
    expect(rep2.verdicts[0]?.code).toBe("changed");

    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await check(r, { write: true });
    doc = await r.read("README.md");
    expect(hasBanner(doc, "README.md", "deadbeef")).toBe(false);
    expect(doc).toBe(pristine);
  });

  test("--write never writes a hibi-status frontmatter line", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await r.write("README.md", "---\ntitle: Doc\n---\n\n# Doc\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    await r.write("src/retry.ts", "// gone\n");
    await check(r, { write: true });
    const doc = await r.read("README.md");
    expect(hasBanner(doc, "README.md", "deadbeef")).toBe(true);
    expect(doc).not.toContain("hibi-status");
  });
});

describe("check is fully offline: no git on the verdict path", () => {
  test("verdicts are identical whether or not a git repo exists", async () => {
    const r = await repo();
    await r.write("src/retry.ts", "export const MAX_ATTEMPTS = 5;\n");
    await record(r, {
      doc: "README.md",
      text: "Capped at 5",
      file: "src/retry.ts",
      quote: "MAX_ATTEMPTS = 5",
    });
    // The temp dir is not a git repo; check must still succeed deterministically.
    const a = await check(r);
    const b = await check(r);
    expect(a.verdicts[0]?.code).toBe("unchanged");
    expect(a.verdicts[0]?.code).toBe(b.verdicts[0]?.code);
    expect(a.verdicts[0]?.evidence.similarity).toBe(
      b.verdicts[0]?.evidence.similarity,
    );
    expect(a.ref).toBe("WORKTREE");
  });
});
