/**
 * Schema v2 fitness functions (ADR-003 D28/D29). These are guards, not fixtures:
 *   - Version invariant — a store whose config.json version ≠ MODEL_VERSION fails
 *     to load with the verbatim D28 message.
 *   - Strictness invariant — any unknown key in a stored object fails the parse.
 *   - Provenance invariant — a `modelBacked` resolver's provenance-less advisory
 *     is dropped with the D29 warning; a provenance-carrying one passes through.
 */

import { describe, expect, test } from "bun:test";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { ResolveFiles } from "../src/algo/resolve.ts";
import { type Advisory, Assertion, MODEL_VERSION } from "../src/core/model.ts";
import { composeAnchor } from "../src/engine/anchor.ts";
import type { Resolver } from "../src/resolver/registry.ts";
import { DriftResolver, ResolverRegistry } from "../src/resolver/registry.ts";
import { ClaimStore } from "../src/store/store.ts";
import { makeRepo } from "./helpers.ts";

describe("schema v2 — version gate (D28)", () => {
  test("a store written by model v1 fails to load with the verbatim D28 message", async () => {
    const r = await makeRepo();
    try {
      // Downgrade the freshly-init'd v2 store's config to v1 on disk.
      await writeFile(
        join(r.store.dir, "config.json"),
        `${JSON.stringify({ version: "v1", nonce: "deadbeef" }, null, 2)}\n`,
      );
      await expect(ClaimStore.open(r.root)).rejects.toThrow(
        `this store was written by hibi model v1 and this binary requires ${MODEL_VERSION}. hibi ships no migration (beta): re-run 'hibi init' and re-record, or use a matching hibi version.`,
      );
    } finally {
      await r.cleanup();
    }
  });
});

describe("schema v2 — strict objects (D28)", () => {
  test("an unknown key (claimKind) on a stored object fails Zod parsing", () => {
    const base = {
      id: "a",
      propositionId: "p",
      documentId: "d",
      owner: "o",
      ref: "r",
      anchor: composeAnchor(
        {
          file: "doc.md",
          selectors: [
            {
              kind: "text-quote",
              exact: "foo bar baz",
              prefix: "",
              suffix: "",
            },
          ],
        },
        [],
      ),
      // The removed pre-D12 taxonomy field — must be rejected, not silently stripped.
      claimKind: "structural",
    };
    expect(() => Assertion.parse(base)).toThrow();
  });
});

/** A fake in-process advisory resolver that emits a caller-supplied advisory list. */
class FakeAdvisor implements Resolver {
  readonly name = "fake-advisor";
  readonly kinds = ["text-quote"];
  readonly tier = 3;
  readonly advisory = true;
  readonly modelBacked: boolean;
  constructor(
    private advisories: Advisory[],
    modelBacked: boolean,
  ) {
    this.modelBacked = modelBacked;
  }
  async resolve() {
    return { advisories: this.advisories };
  }
}

function oneAssertion(): Assertion {
  return Assertion.parse({
    id: "a",
    propositionId: "p",
    documentId: "d",
    owner: "o",
    ref: "r",
    anchor: composeAnchor(
      {
        file: "doc.md",
        selectors: [
          { kind: "text-quote", exact: "foo bar baz", prefix: "", suffix: "" },
        ],
      },
      [],
    ),
    enforcement: "suggested",
  });
}

function files(doc: string | null): ResolveFiles {
  return { doc, code: new Map() };
}

async function withStderr<T>(
  fn: () => Promise<T>,
): Promise<{ result: T; stderr: string }> {
  const chunks: string[] = [];
  const orig = process.stderr.write.bind(process.stderr);
  const spy = (chunk: string | Uint8Array): boolean => {
    chunks.push(
      typeof chunk === "string" ? chunk : new TextDecoder().decode(chunk),
    );
    return true;
  };
  process.stderr.write = spy as typeof process.stderr.write;
  try {
    const result = await fn();
    return { result, stderr: chunks.join("") };
  } finally {
    process.stderr.write = orig;
  }
}

describe("schema v2 — advisory provenance (D29)", () => {
  test("a modelBacked resolver's provenance-less advisory is dropped with the verbatim warning", async () => {
    const registry = new ResolverRegistry();
    registry.register(new DriftResolver());
    registry.register(
      new FakeAdvisor(
        [{ resolver: "fake-advisor", message: "no provenance here" }],
        true,
      ),
    );
    const { result: verdict, stderr } = await withStderr(() =>
      registry.resolve(oneAssertion(), files("foo bar baz")),
    );
    expect(verdict.advisories).toHaveLength(0);
    expect(stderr).toContain(
      "dropped 1 advisories from fake-advisor: modelBacked resolvers must attach provenance (model, promptHash, contextHash).",
    );
  });

  test("a modelBacked resolver's provenance-carrying advisory passes through", async () => {
    const registry = new ResolverRegistry();
    registry.register(new DriftResolver());
    registry.register(
      new FakeAdvisor(
        [
          {
            resolver: "fake-advisor",
            message: "grounded",
            provenance: {
              model: "m",
              promptHash: "ph",
              contextHash: "ch",
            },
          },
        ],
        true,
      ),
    );
    const { result: verdict, stderr } = await withStderr(() =>
      registry.resolve(oneAssertion(), files("foo bar baz")),
    );
    expect(verdict.advisories).toHaveLength(1);
    expect(verdict.advisories[0]?.message).toBe("grounded");
    expect(stderr).not.toContain("dropped");
  });

  test("a non-modelBacked resolver's provenance-less advisory passes through unchanged", async () => {
    const registry = new ResolverRegistry();
    registry.register(new DriftResolver());
    registry.register(
      new FakeAdvisor(
        [{ resolver: "fake-advisor", message: "plain advice" }],
        false,
      ),
    );
    const { result: verdict, stderr } = await withStderr(() =>
      registry.resolve(oneAssertion(), files("foo bar baz")),
    );
    expect(verdict.advisories).toHaveLength(1);
    expect(stderr).not.toContain("dropped");
  });
});
