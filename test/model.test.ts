import { describe, expect, test } from "bun:test";
import * as z from "zod";
import {
  Assertion,
  AuthoredTrust,
  ComputedState,
  Document,
  DocumentLifecycle,
  Proposition,
  SCHEMAS,
  Selector,
  Verdict,
} from "../src/core/model.ts";
import { PROTOCOL_SCHEMAS } from "../src/resolver/protocol.ts";

describe("canonical model is the single source of truth (§5)", () => {
  test("every schema exports to JSON Schema without throwing", () => {
    for (const [name, schema] of Object.entries({
      ...SCHEMAS,
      ...PROTOCOL_SCHEMAS,
    })) {
      expect(
        () =>
          z.toJSONSchema(schema, { target: "draft-2020-12", reused: "ref" }),
        name,
      ).not.toThrow();
    }
  });

  test("enums match §10 exactly", () => {
    expect(AuthoredTrust.options).toEqual(["verified", "inferred", "assumed"]);
    expect(ComputedState.options).toEqual([
      "fresh",
      "moved",
      "stale",
      "ghost",
      "expired",
    ]);
    expect(DocumentLifecycle.options).toEqual([
      "active",
      "amended",
      "superseded",
      "archived",
      "retracted",
    ]);
  });

  test("Anchor selectors are a discriminated union on kind (§4, §7.2)", () => {
    const js = z.toJSONSchema(Selector) as {
      anyOf?: unknown[];
      oneOf?: unknown[];
    };
    const variants = js.oneOf ?? js.anyOf;
    expect(Array.isArray(variants)).toBe(true);
    expect(variants?.length).toBe(6); // text-quote, text-position, ast-node, value, path, glob
  });

  test("a valid record round-trips through Zod", () => {
    const a: Assertion = {
      id: "asrt_1",
      propositionId: "prop_1",
      documentId: "doc_1",
      owner: "x",
      ref: "r",
      anchor: {
        file: "a.ts",
        selectors: [{ kind: "text-quote", exact: "x", prefix: "", suffix: "" }],
      },
      attrs: {},
    };
    expect(() => Assertion.parse(a)).not.toThrow();
  });

  test("an invalid selector kind is rejected", () => {
    expect(() =>
      Assertion.parse({
        id: "a",
        propositionId: "p",
        documentId: "d",
        owner: "x",
        ref: "r",
        anchor: { file: "a.ts", selectors: [{ kind: "bogus" }] },
        attrs: {},
      }),
    ).toThrow();
  });

  test("an empty selector bundle is rejected (min 1)", () => {
    expect(() => z.object({ a: Assertion }).parse({})).toThrow();
    const bad = { file: "a.ts", selectors: [] };
    expect(() => z.toJSONSchema(Selector)).not.toThrow();
    expect(
      Assertion.safeParse({
        id: "a",
        propositionId: "p",
        documentId: "d",
        owner: "x",
        ref: "r",
        anchor: bad,
        attrs: {},
      }).success,
    ).toBe(false);
  });

  test("Document applies defaults (lifecycle active, edges [])", () => {
    const d = Document.parse({ id: "d", path: "x.md" });
    expect(d.lifecycle).toBe("active");
    expect(d.edges).toEqual([]);
  });

  test("Proposition requires authoredTrust from the enum", () => {
    expect(
      Proposition.safeParse({
        id: "p",
        text: "t",
        authoredTrust: "nope",
        fingerprint: "f",
      }).success,
    ).toBe(false);
  });

  test("Verdict (ephemeral) validates the computed states", () => {
    const v: z.infer<typeof Verdict> = {
      assertionId: "a",
      propositionId: "p",
      documentId: "d",
      state: "stale",
      confidence: 0.3,
      selectorScores: [],
      notes: [],
      advisories: [],
    };
    expect(() => Verdict.parse(v)).not.toThrow();
  });
});
