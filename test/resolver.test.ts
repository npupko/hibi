import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import type { ResolveFiles } from "../src/algo/resolve.ts";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import type { Assertion, Proposition } from "../src/core/model.ts";
import { buildSelectorBundle, composeAnchor } from "../src/engine/anchor.ts";
import { OutOfProcessResolver } from "../src/resolver/client.ts";
import { loadManifest } from "../src/resolver/manifest.ts";
import { LineFramer } from "../src/resolver/protocol.ts";
import { DriftResolver, ResolverRegistry } from "../src/resolver/registry.ts";

const ROOT = join(import.meta.dir, "..");
const ADVISOR = join(ROOT, "resolvers", "semantic-advisor.ts");

function advisorProc(timeoutMs = 8000) {
  return new OutOfProcessResolver({
    name: "semantic-advisor",
    command: "bun",
    args: ["run", ADVISOR],
    timeoutMs,
    cwd: ROOT,
  });
}

// Two-axis model: the proposition carries `textCache` (non-authoritative copy of
// the documented sentence); the semantic advisor classifies against it.
const prop = (textCache: string): Proposition => ({
  id: "prop_x",
  textCache,
  authoredTrust: "inferred",
  fingerprint: "f",
});

// Bidirectional anchor (doc-side + code-side bundles) + enforcement + verifiers.
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
  verifiers: [],
  attrs: {},
});

/** ResolveFiles wire helper: a doc string + a code Map (per the new model). */
function files(
  doc: string | null,
  code: Record<string, string | null> = {},
): ResolveFiles {
  return { doc, code: new Map(Object.entries(code)) };
}

describe("vendored line framing (§7.1)", () => {
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

describe("out-of-process resolver over JSONL-RPC (§7.1)", () => {
  test("describe announces kinds, tier, advisory", async () => {
    const proc = advisorProc();
    const desc = await proc.describe();
    proc.dispose();
    expect(desc).not.toBeNull();
    expect(desc?.name).toBe("semantic-advisor");
    expect(desc?.tier).toBe(3);
    expect(desc?.advisory).toBe(true);
    expect(desc?.kinds).toContain("text-quote");
  });

  test("resolve returns advisories for a behavioral claim, none otherwise", async () => {
    const proc = advisorProc();
    const behavioral = await proc.resolve({
      assertion: assertion(),
      files: { doc: null, code: { "x.ts": "code" } },
      proposition: prop("Retries on timeout with exponential backoff"),
    });
    const structural = await proc.resolve({
      assertion: assertion(),
      files: { doc: null, code: { "x.ts": "code" } },
      proposition: prop("MAX_ATTEMPTS equals 5"),
    });
    proc.dispose();
    expect(behavioral?.advisories.length).toBeGreaterThan(0);
    expect(behavioral?.advisories[0]?.message).toContain(
      "semantic re-verification",
    );
    expect(structural?.advisories.length).toBe(0);
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

  test("verify round-trips: a resolver with no verify handler answers unknown-method → null", async () => {
    const proc = advisorProc();
    const res = await proc.verify({
      assertion: assertion(),
      verifier: { kind: "command", ref: "bun test" },
      files: { doc: null, code: { "x.ts": "code" } },
      changedEvidence: [],
    });
    proc.dispose();
    // The semantic advisor declares no verifierKinds and omits verify(); the
    // server replies with an unknown-method error, which degrades to null.
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

describe("default-deny manifest (§7.1)", () => {
  test("absent manifest yields no resolvers", async () => {
    const manifest = await loadManifest("/nonexistent-path-xyz");
    expect(manifest.resolvers).toEqual([]);
  });
});

describe("registry: advisory resolvers advise but never gate (§7.4)", () => {
  test("an unchanged deterministic verdict keeps its state but gains advisories", async () => {
    const analyzer = await getAnalyzer();
    const registry = new ResolverRegistry();
    registry.register(new DriftResolver(analyzer));
    // Manually register the advisor as an out-of-process resolver.
    const proc = advisorProc();
    const desc = await proc.describe();
    if (desc === null) throw new Error("advisor describe() returned null");
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

    // Code side: anchor the literal; doc side: anchor the documented sentence.
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

    // The advisory did NOT change the deterministic verdict (code stays unchanged).
    expect(verdict.code).toBe("unchanged");
    expect(verdict.doc).toBe("unchanged");
    expect(verdict.gates).toBe(false);
    expect(verdict.advisories.length).toBeGreaterThan(0);
    expect(verdict.advisories[0]?.resolver).toBe("semantic-advisor");
  });
});
