import { beforeAll, describe, expect, test } from "bun:test";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { importCandidates } from "../src/engine/evidence.ts";

let analyzer: Awaited<ReturnType<typeof getAnalyzer>>;
beforeAll(async () => {
  analyzer = await getAnalyzer();
});

/**
 * Per-grammar import extraction (§17.6, D14). Verifies the node-kind map against
 * the pinned grammars with a fixture per language (the §17.4 discipline), and
 * that bare/package specifiers are skipped by the walk or the resolver.
 */
describe("import extraction — the change-gate's file-level reachability", () => {
  test("typescript: import / export-from / require; bare specifiers pass through", () => {
    const src = [
      'import { a } from "./a";',
      'import b from "../b.ts";',
      'export { c } from "./c";',
      'const d = require("./d");',
      'import x from "react";', // bare — kept as a specifier, skipped at resolve
    ].join("\n");
    const specs = analyzer.extractImports(src, "typescript");
    expect(specs).toContain("./a");
    expect(specs).toContain("../b.ts");
    expect(specs).toContain("./c");
    expect(specs).toContain("./d");
    expect(specs).toContain("react");
  });

  test("python: relative from-imports only; absolute imports are skipped", () => {
    const src = [
      "from .sibling import x",
      "from ..pkg.mod import y",
      "import os",
      "from collections import OrderedDict",
    ].join("\n");
    const specs = analyzer.extractImports(src, "python");
    expect(specs).toContain(".sibling");
    expect(specs).toContain("..pkg.mod");
    // Absolute imports never contribute a relative specifier.
    expect(specs.some((s) => s.startsWith("os"))).toBe(false);
  });

  test("rust: `mod foo;` declarations, not inline modules or `use` paths", () => {
    const src = [
      "mod foo;",
      "mod bar;",
      "mod inline { fn f() {} }",
      "use crate::baz::Thing;",
    ].join("\n");
    const specs = analyzer.extractImports(src, "rust");
    expect(specs).toContain("foo");
    expect(specs).toContain("bar");
    expect(specs).not.toContain("inline");
    expect(specs).not.toContain("baz");
  });

  test("go / java package imports are not file-resolvable (skipped)", () => {
    expect(analyzer.extractImports('import "fmt"\n', "go")).toEqual([]);
    expect(analyzer.extractImports("import java.util.List;\n", "java")).toEqual(
      [],
    );
  });

  test("resolveImportPaths: relative candidates per language; bare → none", () => {
    // TS: relative resolves to the sibling with common extensions + index.
    const ts = importCandidates("src/main.ts", "./dep", "typescript");
    expect(ts).toContain("src/dep.ts");
    expect(ts).toContain("src/dep/index.ts");
    // A bare/package specifier yields no candidates.
    expect(importCandidates("src/main.ts", "react", "typescript")).toEqual([]);
    // Rust mod → foo.rs or foo/mod.rs.
    expect(importCandidates("src/lib.rs", "foo", "rust")).toEqual([
      "src/foo.rs",
      "src/foo/mod.rs",
    ]);
    // Python relative → .py or package __init__.py.
    expect(importCandidates("pkg/mod.py", ".sibling", "python")).toEqual([
      "pkg/sibling.py",
      "pkg/sibling/__init__.py",
    ]);
  });
});
