/**
 * Engine guards exercised through the Engine facade:
 *   - record-time doc-quote guard (length floor, undisambiguable repeat) and
 *     the warning on a disambiguated repeat;
 *   - reanchor refuses an orphaned side and never stores an empty span;
 *   - reanchor resolves once per side and reports before/after quotes;
 *   - coverage sentence-level regions and the missing-doc error;
 *   - supersede relocation.
 */

import { afterEach, describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { splitRegions } from "../src/engine/coverage.ts";
import { documentIdForPath, Engine } from "../src/index.ts";

let dirs: string[] = [];
async function repo(): Promise<string> {
  const d = await mkdtemp(join(tmpdir(), "ce-guards-"));
  dirs.push(d);
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

describe("record-time doc-quote guard", () => {
  test("a doc quote shorter than 8 characters is rejected", async () => {
    const d = await repo();
    await write(d, "doc.md", "# D\n\nabcdefg\n");
    await write(d, "a.ts", "export const A = 1;\n");
    const engine = await Engine.init(d);
    await expect(
      engine.record({
        docPath: "doc.md",
        docQuote: "abcdefg",
        code: [{ file: "a.ts", region: { quote: "A = 1" } }],
      }),
    ).rejects.toThrow(/shorter than 8 characters/);
  });

  test("a repeated quote the context cannot disambiguate is rejected; a disambiguated one warns", async () => {
    const d = await repo();
    const filler = "filler line long enough to exceed forty eight chars here";
    await write(
      d,
      "same.md",
      `${filler}\nThe value is fixed here\n${filler}\nThe value is fixed here\n${filler}\n`,
    );
    await write(
      d,
      "diff.md",
      `Intro.\nThe value is fixed here\nMiddle section text.\nThe value is fixed here\nOutro.\n`,
    );
    await write(d, "a.ts", "export const A = 1;\n");
    const engine = await Engine.init(d);
    const code = [{ file: "a.ts", region: { quote: "A = 1" } }];
    await expect(
      engine.record({
        docPath: "same.md",
        docQuote: "The value is fixed here",
        code,
      }),
    ).rejects.toThrow(/does not select a single occurrence/);
    const ok = await engine.record({
      docPath: "diff.md",
      docQuote: "The value is fixed here",
      code,
    });
    expect(ok.warnings[0]).toContain("occurs 2 times");
  });

  test("an enforced claim needs a precise code span; --suggest allows a coarse one", async () => {
    const d = await repo();
    await write(d, "doc.md", "# D\n\nThe auth module is documented here.\n");
    const engine = await Engine.init(d);
    await expect(
      engine.record({
        docPath: "doc.md",
        docQuote: "The auth module is documented here.",
        code: [{ file: "src/auth/**", coarse: true }],
      }),
    ).rejects.toThrow(/precise code span/);
    const ok = await engine.record({
      docPath: "doc.md",
      docQuote: "The auth module is documented here.",
      code: [{ file: "src/auth/**", coarse: true }],
      enforcement: "suggested",
    });
    expect(ok.assertion.enforcement).toBe("suggested");
    expect(ok.assertion.anchor.code[0]?.selectors[0]?.kind).toBe("coarse");
    const hits = await engine.list({ path: "src/auth/login.ts" });
    expect(hits.count).toBe(1);
    expect(hits.claims[0]?.side).toBe("code");
  });
});

describe("reanchor safety", () => {
  async function seed() {
    const d = await repo();
    await write(
      d,
      "src/a.ts",
      "export const A = 1;\n\nexport function f() {\n  return 1;\n}\n",
    );
    await write(d, "doc.md", "# D\n\nThe A constant is one.\n");
    const engine = await Engine.init(d);
    const rec = await engine.record({
      docPath: "doc.md",
      docQuote: "The A constant is one.",
      code: [{ file: "src/a.ts", region: { quote: "export function f()" } }],
    });
    return { d, engine, id: rec.assertion.id };
  }

  test("refuses when the code side is orphaned and no replacement span is given", async () => {
    const { d, engine, id } = await seed();
    await write(d, "src/a.ts", "export const A = 1;\n");
    await expect(engine.reanchor(id)).rejects.toThrow(/orphaned/);
    const stored = await engine.store.getAssertion(id);
    const tq = stored?.anchor.code[0]?.selectors.find(
      (s) => s.kind === "text-quote",
    );
    expect(tq?.kind === "text-quote" ? tq.exact : "").toBe(
      "export function f()",
    );
  });

  test("refuses when the doc side is orphaned and no doc span is given", async () => {
    const { d, engine, id } = await seed();
    await write(d, "doc.md", "# D\n\nSomething else entirely.\n");
    await expect(engine.reanchor(id)).rejects.toThrow(/doc span/);
  });

  test("never stores an empty span", async () => {
    const { d, engine, id } = await seed();
    await write(d, "src/a.ts", "\n\n\n\n\n");
    await expect(
      engine.reanchor(id, {
        code: [{ file: "src/a.ts", region: { startLine: 2, endLine: 2 } }],
      }),
    ).rejects.toThrow(/empty span/);
  });

  test("reports before and after quotes per side", async () => {
    const { d, engine, id } = await seed();
    await write(
      d,
      "src/a.ts",
      "export const A = 1;\n\nexport function f(x: number) {\n  return x;\n}\n",
    );
    const res = await engine.reanchor(id, {
      code: [
        { file: "src/a.ts", region: { quote: "export function f(x: number)" } },
      ],
    });
    expect(res.before.code[0]?.quote).toBe("export function f()");
    expect(res.after.code[0]?.quote).toBe("export function f(x: number)");
    expect(res.before.doc.quote).toBe(res.after.doc.quote);
    expect(res.code).toBe("unchanged");
  });

  test("--suggest lists the doc quote across documents and the code quote across same-language files", async () => {
    const { d, engine, id } = await seed();
    await write(d, "src/b.ts", "export function f() {\n  return 2;\n}\n");
    await write(d, "docs/other.md", "# O\n\nThe A constant is one.\n");
    await engine.record({
      docPath: "docs/other.md",
      docQuote: "The A constant is one.",
      code: [{ file: "src/b.ts", region: { quote: "return 2" } }],
    });
    const res = await engine.reanchorSuggest(id);
    const keys = res.candidates.map((c) => `${c.side}:${c.file}`);
    expect(keys).toContain("doc:doc.md");
    expect(keys).toContain("doc:docs/other.md");
    expect(keys).toContain("code:src/a.ts");
    expect(keys).toContain("code:src/b.ts");
  });
});

describe("coverage regions", () => {
  test("prose splits at sentence boundaries; fences, headings, and list items stay whole", () => {
    const text = [
      "# Heading",
      "",
      "First sentence. Second sentence! Third?",
      "",
      "- item one",
      "- item two",
      "",
      "```ts",
      "const a = 1;",
      "",
      "const b = 2;",
      "```",
      "",
    ].join("\n");
    const regions = splitRegions(text).map((r) => text.slice(r.start, r.end));
    expect(regions).toEqual([
      "# Heading",
      "First sentence.",
      "Second sentence!",
      "Third?",
      "- item one",
      "- item two",
      "```ts\nconst a = 1;\n\nconst b = 2;\n```",
    ]);
  });

  test("a missing document is an error, not an empty result", async () => {
    const d = await repo();
    const engine = await Engine.init(d);
    await expect(engine.coverage("nope.md")).rejects.toThrow(/not found/);
  });
});

describe("supersede relocation", () => {
  test("moves verbatim sentences, reports misses, and lists what is stranded", async () => {
    const d = await repo();
    await write(d, "src/a.ts", "export const A = 1;\nexport const B = 2;\n");
    await write(d, "v1.md", "# V1\n\nA is one.\nB is two.\n");
    const engine = await Engine.init(d);
    const a = await engine.record({
      docPath: "v1.md",
      docQuote: "A is one.",
      code: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
    });
    const b = await engine.record({
      docPath: "v1.md",
      docQuote: "B is two.",
      code: [{ file: "src/a.ts", region: { quote: "B = 2" } }],
    });
    await write(d, "v2.md", "# V2\n\nA is one.\n");
    const res = await engine.supersede({ from: "v1.md", to: "v2.md" });
    expect(res.relocated.map((r) => r.claimId)).toEqual([a.assertion.id]);
    expect(res.misses.map((m) => m.claimId)).toEqual([b.assertion.id]);
    expect(res.strandedClaims).toEqual([b.assertion.id]);
    expect(res.oldDoc.lifecycle).toBe("superseded");
    expect(res.newDoc.edges).toEqual([
      { type: "supersedes", target: res.oldDoc.id },
    ]);
    const moved = await engine.store.getAssertion(a.assertion.id);
    expect(moved?.documentId).toBe(documentIdForPath("v2.md"));
    const report = await engine.check();
    expect(
      report.verdicts.find((v) => v.assertionId === a.assertion.id)?.doc,
    ).toBe("unchanged");
  });
});
