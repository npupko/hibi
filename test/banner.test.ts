import { describe, expect, test } from "bun:test";
import {
  type BannerPayload,
  buildBanner,
  commentStyleFor,
  hasBanner,
  locateBanner,
  removeBanner,
  stampBanner,
} from "../src/banner/banner.ts";

const NONCE = "a1b2c3d4";
const payload: BannerPayload = {
  entries: [
    {
      status: "code:changed",
      id: "prop_002",
      text: "Retries are capped at 5 attempts",
    },
    { status: "code:orphaned", id: "prop_001", text: "Backoff is exponential" },
  ],
};

describe("comment style selection", () => {
  test("maps extensions to styles", () => {
    expect(commentStyleFor("README.md")).toBe("html");
    expect(commentStyleFor("conf.yaml")).toBe("hash");
    expect(commentStyleFor("retry.ts")).toBe("slash");
    expect(commentStyleFor("notes.txt")).toBe("none");
  });
});

describe("banner body", () => {
  test("entries are sorted by id", () => {
    const block = buildBanner(payload, NONCE, "html");
    const i1 = block.indexOf("prop_001");
    const i2 = block.indexOf("prop_002");
    expect(i1).toBeGreaterThan(-1);
    expect(i1).toBeLessThan(i2);
  });
  test("the END line carries only the version and the nonce", () => {
    const block = buildBanner(payload, NONCE, "none");
    expect(block).toMatch(/HIBI:END v1 a1b2c3d4$/m);
    expect(block).not.toContain("sha=");
  });
});

describe("idempotent stamping", () => {
  test("insert then re-stamp identical content is a noop (byte-stable)", () => {
    const original = "# My Doc\n\nSome prose here.\n";
    const first = stampBanner(original, "doc.md", payload, NONCE);
    expect(first.action).toBe("insert");
    const second = stampBanner(first.content, "doc.md", payload, NONCE);
    expect(second.action).toBe("noop");
    expect(second.content).toBe(first.content);
  });

  test("re-stamp with changed status replaces only the banner region", () => {
    const original = "# Doc\n\nbody\n";
    const a = stampBanner(original, "doc.md", payload, NONCE);
    const changed: BannerPayload = {
      entries: [
        {
          status: "code:moved",
          id: "prop_001",
          text: "Backoff is exponential",
        },
      ],
    };
    const b = stampBanner(a.content, "doc.md", changed, NONCE);
    expect(b.action).toBe("replace");
    expect(b.content).toContain("# Doc");
    expect(b.content).toContain("body");
    expect(b.content).not.toContain("prop_002");
  });

  test("round-trip: stamp then remove restores exact pre-banner bytes", () => {
    for (const [file, original] of [
      ["doc.md", "# Title\n\nProse paragraph.\n"],
      ["conf.yaml", "key: value\nother: 2\n"],
      ["retry.ts", "export const MAX = 5;\n"],
      ["notes.txt", "plain text line\n"],
    ] as const) {
      const stamped = stampBanner(original, file, payload, NONCE);
      expect(stamped.action).toBe("insert");
      const removed = removeBanner(stamped.content, file, NONCE);
      expect(removed.action).toBe("remove");
      expect(removed.content).toBe(original);
    }
  });

  test("stamp/unstamp/stamp is byte-stable", () => {
    const original = "# Doc\n\nbody\n";
    const s1 = stampBanner(original, "doc.md", payload, NONCE).content;
    const r1 = removeBanner(s1, "doc.md", NONCE).content;
    const s2 = stampBanner(r1, "doc.md", payload, NONCE).content;
    expect(s2).toBe(s1);
  });

  test("a hand-edit inside the banner is overwritten by the next stamp", () => {
    const stamped = stampBanner(
      "# Doc\n\nbody\n",
      "doc.md",
      payload,
      NONCE,
    ).content;
    const edited = stamped.replace("capped at 5", "capped at 99");
    const res = stampBanner(edited, "doc.md", payload, NONCE);
    expect(res.action).toBe("replace");
    expect(res.content).toContain("capped at 5");
    expect(res.content).not.toContain("capped at 99");
  });
});

describe("frontmatter placement", () => {
  test("html banner goes after a leading YAML frontmatter fence", () => {
    const original = "---\ntitle: Hi\nstatus: active\n---\n\n# Heading\n";
    const stamped = stampBanner(original, "doc.md", payload, NONCE);
    const fmEnd = stamped.content.indexOf("---\n", 3);
    const bannerStart = stamped.content.indexOf("<!--");
    expect(bannerStart).toBeGreaterThan(fmEnd);
    expect(
      stamped.content.startsWith("---\ntitle: Hi\nstatus: active\n---"),
    ).toBe(true);
  });
});

describe("nonce safety and legacy END lines", () => {
  test("a document quoting the banner format with a different nonce is never matched", () => {
    const quoting =
      "Here is the format: HIBI:BEGIN v1 deadbeef\nand the rest.\n";
    expect(locateBanner(quoting, NONCE, "none")).toBeNull();
    const stamped = stampBanner(quoting, "notes.txt", payload, NONCE);
    expect(stamped.content).toContain("HIBI:BEGIN v1 deadbeef");
    expect(hasBanner(stamped.content, "notes.txt", NONCE)).toBe(true);
  });

  test("an END line with a legacy sha= suffix is still located and replaced", () => {
    const legacy = [
      "# Doc",
      "<!--",
      `HIBI:BEGIN v1 ${NONCE}`,
      "STALE DOCUMENT — 1 suspect claim(s) — re-verify before trusting.",
      "[code:changed] (prop_009) Old entry",
      `HIBI:END v1 ${NONCE} sha=0123abcd`,
      "-->",
      "",
      "body",
      "",
    ].join("\n");
    const located = locateBanner(legacy, NONCE, "html");
    expect(located).not.toBeNull();
    expect(legacy.slice(located?.blockStart, located?.blockEnd)).toContain(
      "sha=0123abcd",
    );
    const res = stampBanner(legacy, "doc.md", payload, NONCE);
    expect(res.action).toBe("replace");
    expect(res.content).not.toContain("sha=");
    expect(res.content).not.toContain("prop_009");
    expect(res.content).toContain("prop_001");
    expect(removeBanner(res.content, "doc.md", NONCE).content).toBe(
      "# Doc\nbody\n",
    );
  });
});
