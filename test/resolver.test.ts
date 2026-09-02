import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import type { ResolveFiles } from "../src/algo/resolve.ts";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import type { Assertion, Proposition } from "../src/core/model.ts";
import { buildSelectorBundle, composeAnchor } from "../src/engine/anchor.ts";
import { OutOfProcessResolver } from "../src/resolver/client.ts";
import { loadManifest } from "../src/resolver/manifest.ts";
import { LineFramer } from "../src/resolver/protocol.ts";
import {
  BUILTIN_KINDS,
  DriftResolver,
  ResolverRegistry,
} from "../src/resolver/registry.ts";

const ROOT = join(import.meta.dir, "..");
const ECHO = join(ROOT, "test", "fixtures", "echo-resolver.ts");

function echoProc(timeoutMs = 8000) {
  return new OutOfProcessResolver({
    name: "echo",
    command: "bun",
    args: ["run", ECHO],
    timeoutMs,
    cwd: ROOT,
  });
}

const prop = (textCache: string): Proposition => ({
  id: "prop_x",
  textCache,
  fingerprint: "f",
});

const assertion = (): Assertion => ({
  id: "a",
  propositionId: "prop_x",
  documentId: "d",
  owner: "o",
  ref: "r",
  anchor: composeAnchor(
    {
      file: "doc.md",
      selectors: [{ kind: "text-quote", exact: "foo", prefix: "", suffix: "" }],
    },
    [
      {
        file: "x.ts",
        selectors: [
          { kind: "text-quote", exact: "foo", prefix: "", suffix: "" },
        ],
      },
    ],
  ),
  enforcement: "suggested",
  verified: false,
  verifiers: [],
  attrs: {},
});

/** ResolveFiles helper: a doc string + a code Map. */
function files(
  doc: string | null,
  code: Record<string, string | null> = {},
): ResolveFiles {
  return { doc, code: new Map(Object.entries(code)) };
}

describe("vendored line framing", () => {
  test("reassembles messages split across chunks", () => {
    const f = new LineFramer();
    expect(f.push('{"a":1}\n{"b":')).toEqual(['{"a":1}']);
    expect(f.push("2}\n")).toEqual(['{"b":2}']);
  });
  test("drops blank lines", () => {
    const f = new LineFramer();
    expect(f.push("\n\n{}\n")).toEqual(["{}"]);
  });
});

describe("out-of-process resolver over JSONL-RPC", () => {
  test("describe announces kinds, tier, advisory", async () => {
    const proc = echoProc();
    const desc = await proc.describe();
    proc.dispose();
    expect(desc).not.toBeNull();
    expect(desc?.name).toBe("echo");
    expect(desc?.tier).toBe(3);
    expect(desc?.advisory).toBe(true);
    expect(desc?.kinds).toEqual(["text-quote"]);
    expect(desc?.verifierKinds).toEqual([]);
  });

  test("resolve returns the advisory the resolver produces", async () => {
    const proc = echoProc();
    const res = await proc.resolve({
      assertion: assertion(),
      files: { doc: null, code: { "x.ts": "code" } },
      proposition: prop("Retries on timeout with exponential backoff"),
    });
    proc.dispose();
    expect(res?.advisories).toEqual([{ resolver: "echo", message: "echo" }]);
    expect(res?.verdict).toBeUndefined();
  });

  test("a resolver that never responds is timed out and degrades to null", async () => {
    const proc = new OutOfProcessResolver({
      name: "hang",
      command: "sleep",
      args: ["30"],
      timeoutMs: 250,
    });
    const start = Date.now();
    const res = await proc.resolve({
      assertion: assertion(),
      files: { doc: null, code: {} },
    });
    proc.dispose();
    expect(res).toBeNull();
    expect(Date.now() - start).toBeLessThan(3000);
  });

  test("verify round-trips: a resolver with no verify handler answers unknown-method, which degrades to null", async () => {
    const proc = echoProc();
    const res = await proc.verify({
      assertion: assertion(),
      verifier: { kind: "command", ref: "bun test" },
      changedEvidence: [],
    });
    proc.dispose();
    expect(res).toBeNull();
  });

  test("a resolver that crashes degrades to null without throwing", async () => {
    const proc = new OutOfProcessResolver({
      name: "boom",
      command: "false",
      args: [],
      timeoutMs: 1000,
    });
    const res = await proc.describe();
    proc.dispose();
    expect(res).toBeNull();
  });
});

describe("default-deny manifest", () => {
  test("absent manifest yields no resolvers", async () => {
    const manifest = await loadManifest("/nonexistent-path-xyz");
    expect(manifest.resolvers).toEqual([]);
  });

  test("the built-in kinds are the five selector kinds", () => {
    expect([...BUILTIN_KINDS]).toEqual([
      "text-quote",
      "text-position",
      "ast-node",
      "value",
      "coarse",
    ]);
  });
});

describe("registry: advisory resolvers advise but never gate", () => {
  test("an unchanged deterministic verdict keeps its state but gains advisories", async () => {
    const analyzer = await getAnalyzer();
    const registry = new ResolverRegistry();
    registry.register(new DriftResolver(analyzer));
    const proc = echoProc();
    const desc = await proc.describe();
    if (desc === null) throw new Error("echo describe() returned null");
    expect(desc.advisory).toBe(true);
    registry.register({
      name: desc.name,
      kinds: desc.kinds,
      tier: desc.tier,
      advisory: true,
      resolve: async (a, f, p) => {
        const r = await proc.resolve({
          assertion: a,
          files: { doc: f.doc, code: Object.fromEntries(f.code) },
          proposition: p,
        });
        return { advisories: r?.advisories ?? [] };
      },
    });

    const code = "export const MAX_ATTEMPTS = 5;\n";
    const cStart = code.indexOf("MAX_ATTEMPTS = 5");
    const codeBundle = buildSelectorBundle(
      "retry.ts",
      code,
      { start: cStart, end: cStart + "MAX_ATTEMPTS = 5".length },
      { language: "typescript", analyzer },
    );
    const docText = "Retries on timeout with backoff.\n";
    const dStart = docText.indexOf("Retries on timeout with backoff");
    const docBundle = buildSelectorBundle("guide.md", docText, {
      start: dStart,
      end: dStart + "Retries on timeout with backoff".length,
    });

    const a: Assertion = {
      id: "a1",
      propositionId: "p1",
      documentId: "d1",
      owner: "o",
      ref: "r",
      anchor: composeAnchor(docBundle, [codeBundle]),
      enforcement: "suggested",
      verified: false,
      verifiers: [],
      attrs: {},
    };
    const p: Proposition = prop("Retries on timeout with backoff");

    const verdict = await registry.resolve(
      a,
      files(docText, { "retry.ts": code }),
      p,
    );
    proc.dispose();
    registry.dispose();

    expect(verdict.code).toBe("unchanged");
    expect(verdict.doc).toBe("unchanged");
    expect(verdict.gates).toBe(false);
    expect(verdict.behavior).toBeUndefined();
    expect(verdict.advisories).toEqual([{ resolver: "echo", message: "echo" }]);
  });

  test("a modelBacked advisory without provenance is dropped", async () => {
    const registry = new ResolverRegistry();
    registry.register(new DriftResolver());
    registry.register({
      name: "llm",
      kinds: ["text-quote"],
      tier: 3,
      advisory: true,
      modelBacked: true,
      resolve: async () => ({
        advisories: [
          { resolver: "llm", message: "no provenance" },
          {
            resolver: "llm",
            message: "with provenance",
            provenance: { model: "m", promptHash: "p", contextHash: "c" },
          },
        ],
      }),
    });
    const verdict = await registry.resolve(
      assertion(),
      files("foo", { "x.ts": "foo" }),
    );
    expect(verdict.advisories.map((x) => x.message)).toEqual([
      "with provenance",
    ]);
  });

  test("verifiers do not run unless runVerifiers is set", async () => {
    const registry = new ResolverRegistry();
    registry.register(new DriftResolver());
    let ran = 0;
    registry.register({
      name: "runner",
      kinds: [],
      tier: 2,
      advisory: false,
      verifierKinds: ["command"],
      resolve: async () => ({}),
      verify: async () => {
        ran += 1;
        return { behavior: "refuted", advisories: [], notes: [] };
      },
    });
    const a: Assertion = {
      ...assertion(),
      enforcement: "enforced",
      verifiers: [{ kind: "command", ref: "exit 2" }],
    };
    const off = await registry.resolve(a, files("foo", { "x.ts": "foo" }));
    expect(ran).toBe(0);
    expect(off.behavior).toBeUndefined();
    expect(off.gates).toBe(false);

    registry.runVerifiers = true;
    const on = await registry.resolve(a, files("foo", { "x.ts": "foo" }));
    expect(ran).toBe(1);
    expect(on.behavior).toBe("refuted");
    expect(on.gates).toBe(true);
    expect(on.remediation?.actions.map((x) => x.id)).toEqual([
      "fix-code",
      "fix-claim",
    ]);
  });
});
