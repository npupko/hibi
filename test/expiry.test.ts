import { describe, expect, test } from "bun:test";
import { resolveClaim } from "../src/algo/resolve.ts";
import type { Attestation, Claim } from "../src/core/model.ts";
import { buildSelectorBundle } from "../src/engine/anchor.ts";
import {
  addDuration,
  countingAttestation,
  expiryLabel,
  isExpired,
} from "../src/engine/expiry.ts";

const T0 = "2026-01-01T00:00:00.000Z";
const at = (iso: string) => new Date(iso);

function att(by: string, basis: Attestation["basis"], when = T0): Attestation {
  if (basis === "ack") {
    return { by, at: when, basis, ack: { judgeKey: "k" } } as Attestation;
  }
  return { by, at: when, basis };
}

function claim(attestations: Attestation[], ttl?: string): Claim {
  const doc = "Pro costs $29/month.\n";
  return {
    id: "clm_00000000000000aa",
    documentId: "doc_00000000000000aa",
    kind: "attested",
    sentence: "Pro costs $29/month.",
    doc: buildSelectorBundle("docs/pricing.md", doc, { start: 0, end: 20 }),
    evidence: [],
    sources: [],
    procedure: null,
    ...(ttl !== undefined ? { ttl } : {}),
    enforcement: "enforced",
    attestations,
    createdAt: T0,
    ref: "r",
  };
}

describe("counting rule (SPEC 5)", () => {
  const cases: [string, Attestation["basis"], boolean][] = [
    ["human:alice@example.com", "vouch", true],
    ["agent:opus", "vouch", false],
    ["process:ci", "vouch", false],
    ["human:alice@example.com", "evidence", true],
    ["agent:opus", "evidence", true],
    ["process:ci", "evidence", true],
    ["human:alice@example.com", "ack", false],
    ["agent:opus", "ack", false],
    ["process:ci", "ack", false],
  ];
  for (const [by, basis, counts] of cases) {
    test(`${basis} by ${by.split(":")[0]}: ${counts ? "counts" : "does not count"}`, () => {
      const c = claim([att(by, basis)], "P30D");
      expect(countingAttestation(c) !== undefined).toBe(counts);
      expect(isExpired(c, at("2026-01-02T00:00:00Z"))).toBe(!counts);
    });
  }

  test("the newest counting attestation wins; later non-counting ones are ignored", () => {
    const c = claim(
      [
        att("human:a", "vouch", "2026-01-01T00:00:00Z"),
        att("agent:x", "evidence", "2026-03-01T00:00:00Z"),
        att("agent:x", "vouch", "2026-05-01T00:00:00Z"),
        att("human:a", "ack", "2026-05-02T00:00:00Z"),
      ],
      "P30D",
    );
    expect(countingAttestation(c)?.at).toBe("2026-03-01T00:00:00Z");
    expect(isExpired(c, at("2026-03-30T23:59:59Z"))).toBe(false);
    expect(isExpired(c, at("2026-03-31T00:00:00Z"))).toBe(true);
  });

  test("no ttl never expires; a ttl with no counting attestation is expired", () => {
    expect(isExpired(claim([]), at("2099-01-01T00:00:00Z"))).toBe(false);
    const c = claim([att("agent:opus", "vouch")], "P90D");
    expect(isExpired(c, at(T0))).toBe(true);
    expect(expiryLabel(c, at(T0))).toBe(
      "expired: ttl P90D is set and no attestation counts (only evidence or a human vouch resets ttl)",
    );
  });

  test("the boundary is inclusive: now >= at + ttl", () => {
    const c = claim([att("human:a", "vouch")], "P1D");
    expect(isExpired(c, at("2026-01-01T23:59:59.999Z"))).toBe(false);
    expect(isExpired(c, at("2026-01-02T00:00:00.000Z"))).toBe(true);
    expect(expiryLabel(c, at("2026-01-02T00:00:00.000Z"))).toBe(
      `expired: ttl P1D ran out at 2026-01-02T00:00:00.000Z, counted from the vouch by human:a at ${T0}`,
    );
  });
});

describe("addDuration (ISO 8601)", () => {
  const add = (start: string, d: string) =>
    addDuration(at(start), d).toISOString();

  test("days, weeks, and time parts add fixed seconds", () => {
    expect(add(T0, "P90D")).toBe("2026-04-01T00:00:00.000Z");
    expect(add(T0, "P2W")).toBe("2026-01-15T00:00:00.000Z");
    expect(add(T0, "PT12H")).toBe("2026-01-01T12:00:00.000Z");
    expect(add(T0, "PT1H30M15S")).toBe("2026-01-01T01:30:15.000Z");
    expect(add(T0, "P1DT1H")).toBe("2026-01-02T01:00:00.000Z");
  });

  test("years and months add calendar units, clamping to the month's last day", () => {
    expect(add(T0, "P1Y")).toBe("2027-01-01T00:00:00.000Z");
    expect(add(T0, "P3M")).toBe("2026-04-01T00:00:00.000Z");
    expect(add("2026-01-31T10:00:00Z", "P1M")).toBe("2026-02-28T10:00:00.000Z");
    expect(add("2028-01-31T10:00:00Z", "P1M")).toBe("2028-02-29T10:00:00.000Z");
    expect(add("2028-02-29T00:00:00Z", "P1Y")).toBe("2029-02-28T00:00:00.000Z");
    expect(add("2026-11-15T00:00:00Z", "P1Y2M3DT4H")).toBe(
      "2028-01-18T04:00:00.000Z",
    );
  });

  test("rejects strings that are not durations", () => {
    for (const bad of ["", "P", "90D", "PT", "P1.5D", "P1H", "PT1D"]) {
      expect(() => addDuration(at(T0), bad)).toThrow(
        "not an ISO 8601 duration",
      );
    }
  });
});

describe("resolveClaim applies expiry", () => {
  test("expired is computed from the counting rule at `now`", () => {
    const c = claim([att("human:a", "vouch")], "P30D");
    const files = { doc: "Pro costs $29/month.\n", evidence: new Map() };
    const fresh = resolveClaim(c, files, { now: at("2026-01-15T00:00:00Z") });
    expect(fresh.expired).toBe(false);
    const stale = resolveClaim(c, files, { now: at("2026-02-01T00:00:00Z") });
    expect(stale.expired).toBe(true);
    expect(stale.warns).toBe(true);
    expect(stale.gates).toBe(false);
    const docMode = resolveClaim(c, files, {
      now: at("2026-02-01T00:00:00Z"),
      mode: "doc",
    });
    expect(docMode.gates).toBe(true);
    expect(docMode.reasons).toContain("expired gates under check --doc");
  });
});
