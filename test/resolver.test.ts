import { describe, test, expect } from "bun:test";
import { join } from "node:path";
import { LineFramer } from "../src/resolver/protocol.ts";
import { OutOfProcessResolver } from "../src/resolver/client.ts";
import { loadManifest } from "../src/resolver/manifest.ts";
import { ResolverRegistry, DriftResolver } from "../src/resolver/registry.ts";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { buildAnchor } from "../src/engine/anchor.ts";
import type { Assertion, Proposition } from "../src/core/model.ts";

const ROOT = join(import.meta.dir, "..");
const ADVISOR = join(ROOT, "resolvers", "semantic-advisor.ts");

function advisorProc(timeoutMs = 8000) {
  return new OutOfProcessResolver({ name: "semantic-advisor", command: "bun", args: ["run", ADVISOR], timeoutMs, cwd: ROOT });
}

const prop = (text: string): Proposition => ({ id: "prop_x", text, authoredTrust: "inferred", fingerprint: "f" });
const assertion = (): Assertion => ({
  id: "a", propositionId: "prop_x", documentId: "d", owner: "o", ref: "r",
  anchor: { file: "x.ts", selectors: [{ kind: "text-quote", exact: "foo", prefix: "", suffix: "" }] },
  attrs: {},
});

describe("vendored line framing (§7.1)", () => {
  test("reassembles messages split across chunks", () => {
    const f = new LineFramer();
    expect(f.push('{"a":1}\n{"b":')).toEqual(['{"a":1}']);
    expect(f.push('2}\n')).toEqual(['{"b":2}']);
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
    expect(desc!.name).toBe("semantic-advisor");
    expect(desc!.tier).toBe(3);
    expect(desc!.advisory).toBe(true);
    expect(desc!.kinds).toContain("text-quote");
  });

  test("resolve returns advisories for a behavioral claim, none otherwise", async () => {
    const proc = advisorProc();
    const behavioral = await proc.resolve({ assertion: assertion(), text: "code", proposition: prop("Retries on timeout with exponential backoff") });
    const structural = await proc.resolve({ assertion: assertion(), text: "code", proposition: prop("MAX_ATTEMPTS equals 5") });
    proc.dispose();
    expect(behavioral!.advisories.length).toBeGreaterThan(0);
    expect(behavioral!.advisories[0]!.message).toContain("re-verify semantically");
    expect(structural!.advisories.length).toBe(0);
  });

  test("a resolver that never responds is timed out and degrades to null", async () => {
    const proc = new OutOfProcessResolver({ name: "hang", command: "sleep", args: ["30"], timeoutMs: 250 });
    const start = Date.now();
    const res = await proc.resolve({ assertion: assertion(), text: null });
    proc.dispose();
    expect(res).toBeNull();
    expect(Date.now() - start).toBeLessThan(3000);
  });

  test("a resolver that crashes degrades to null without throwing", async () => {
    const proc = new OutOfProcessResolver({ name: "boom", command: "false", args: [], timeoutMs: 1000 });
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
  test("a fresh deterministic verdict keeps its state but gains advisories", async () => {
    const analyzer = await getAnalyzer();
    const registry = new ResolverRegistry();
    registry.register(new DriftResolver(analyzer));
    // Manually register the advisor as an out-of-process resolver.
    const proc = advisorProc();
    const desc = await proc.describe();
    expect(desc!.advisory).toBe(true);
    registry.register({
      name: desc!.name, kinds: desc!.kinds, tier: desc!.tier, advisory: true,
      resolve: async (a, t, p) => {
        const r = await proc.resolve({ assertion: a, text: t, proposition: p });
        return { advisories: r?.advisories ?? [] };
      },
    });

    const text = "export const MAX_ATTEMPTS = 5;\n";
    const start = text.indexOf("MAX_ATTEMPTS = 5");
    const anchor = buildAnchor("retry.ts", text, { start, end: start + "MAX_ATTEMPTS = 5".length }, { language: "typescript", analyzer });
    const a: Assertion = { id: "a1", propositionId: "p1", documentId: "d1", owner: "o", ref: "r", anchor, attrs: {} };
    const p: Proposition = prop("Retries on timeout with backoff");

    const verdict = await registry.resolve(a, text, p);
    proc.dispose();
    registry.dispose();

    expect(verdict.state).toBe("fresh"); // advisory did NOT change the deterministic verdict
    expect(verdict.advisories.length).toBeGreaterThan(0);
    expect(verdict.advisories[0]!.resolver).toBe("semantic-advisor");
  });
});
