/**
 * No-taxonomy invariant (ADR-002 described it, ADR-003 D32 makes it a real test):
 * the generated v2 JSON schemas must not reintroduce a closed behavioral taxonomy.
 * Parses the committed `schemas/*.v2.json` artifacts and asserts:
 *   (a) the Assertion schema has no `claimKind` property (the removed pre-D12 enum);
 *   (b) `Verifier.kind` is an open `{"type":"string","minLength":1}` — no enum;
 *   (c) the Enforcement enum is exactly `["suggested","enforced","retired"]`.
 */

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { MODEL_VERSION } from "../src/core/model.ts";

const SCHEMA_DIR = join(import.meta.dir, "..", "schemas");
function schema(name: string): Record<string, unknown> {
  return JSON.parse(
    readFileSync(join(SCHEMA_DIR, `${name}.${MODEL_VERSION}.json`), "utf8"),
  );
}

describe("no-taxonomy invariant (D32)", () => {
  test("the Assertion schema has no `claimKind` property", () => {
    const props = schema("Assertion").properties as Record<string, unknown>;
    expect(Object.hasOwn(props, "claimKind")).toBe(false);
  });

  test("Verifier.kind is an open string, not a closed enum", () => {
    const props = schema("Verifier").properties as Record<string, unknown>;
    const kind = props.kind as Record<string, unknown>;
    expect(kind.type).toBe("string");
    expect(kind.minLength).toBe(1);
    expect(kind.enum).toBeUndefined();
  });

  test("the Enforcement enum is exactly suggested/enforced/retired", () => {
    const props = schema("Assertion").properties as Record<string, unknown>;
    const enforcement = props.enforcement as { enum?: string[] };
    expect(enforcement.enum).toEqual(["suggested", "enforced", "retired"]);
  });
});
