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
      status: "stale",
      id: "prop_002",
      text: "Retries are capped at 5 attempts",
    },
    { status: "ghost", id: "prop_001", text: "Backoff is exponential" },
  ],
};

describe("comment style selection (§17.5)", () => {
  test("maps extensions to styles", () => {
    expect(commentStyleFor("README.md")).toBe("html");
    expect(commentStyleFor("conf.yaml")).toBe("hash");
    expect(commentStyleFor("retry.ts")).toBe("slash");
    expect(commentStyleFor("notes.txt")).toBe("none");
  });
});

describe("banner body ordering & checksum (§17.5)", () => {
  test("entries are sorted by id, not similarity", () => {
    const block = buildBanner(payload, NONCE, "html");
    const i1 = block.indexOf("prop_001");
    const i2 = block.indexOf("prop_002");
    expect(i1).toBeGreaterThan(-1);
    expect(i1).toBeLessThan(i2);
  });
  test("END line carries an 8-hex FNV-1a checksum", () => {
    const block = buildBanner(payload, NONCE, "none");
    expect(block).toMatch(/HIBI:END v1 a1b2c3d4 sha=[0-9a-f]{8}/);
  });
});

describe("idempotent stamping (§17.5)", () => {
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
        { status: "fresh", id: "prop_001", text: "Backoff is exponential" },
      ],
    };
    const b = stampBanner(a.content, "doc.md", changed, NONCE);
    expect(b.action).toBe("replace");
    expect(b.content).toContain("# Doc");
    expect(b.content).toContain("body");
    expect(b.content).not.toContain("prop_002");
  });

  test("round-trip: stamp then remove restores exact pre-banner bytes", () => {
    // Pre-normalized inputs (engine owns surrounding whitespace).
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
});

describe("frontmatter placement (§17.5)", () => {
  test("html banner goes after a leading YAML frontmatter fence", () => {
    const original = "---\ntitle: Hi\nstatus: active\n---\n\n# Heading\n";
    const stamped = stampBanner(original, "doc.md", payload, NONCE);
    const fmEnd = stamped.content.indexOf("---\n", 3);
    const bannerStart = stamped.content.indexOf("<!--");
    expect(bannerStart).toBeGreaterThan(fmEnd);
    // Frontmatter preserved intact.
    expect(
      stamped.content.startsWith("---\ntitle: Hi\nstatus: active\n---"),
    ).toBe(true);
  });
});

describe("nonce safety & tamper detection (§17.5)", () => {
  test("a document quoting the banner format with a different nonce is never matched", () => {
    const quoting =
      "Here is the format: HIBI:BEGIN v1 deadbeef\nand the rest.\n";
    expect(locateBanner(quoting, NONCE, "none")).toBeNull();
    // Stamping inserts a new banner and leaves the quoted text untouched.
    const stamped = stampBanner(quoting, "notes.txt", payload, NONCE);
    expect(stamped.content).toContain("HIBI:BEGIN v1 deadbeef");
    expect(hasBanner(stamped.content, "notes.txt", NONCE)).toBe(true);
  });

  test("a hand-edit inside the banner is detected via checksum mismatch", () => {
    const stamped = stampBanner(
      "# Doc\n\nbody\n",
      "doc.md",
      payload,
      NONCE,
    ).content;
    const tampered = stamped.replace(
      "Retries are capped at 5 attempts",
      "Retries are capped at 99 attempts",
    );
    const located = locateBanner(tampered, NONCE, "html");
    expect(located).not.toBeNull();
    expect(located?.sha).not.toBe(located?.computedSha);
  });

  test("--fail-on tamper refuses to overwrite a hand-edited banner", () => {
    const stamped = stampBanner(
      "# Doc\n\nbody\n",
      "doc.md",
      payload,
      NONCE,
    ).content;
    const tampered = stamped.replace("capped at 5", "capped at 99");
    const res = stampBanner(tampered, "doc.md", payload, NONCE, {
      failOnTamper: true,
    });
    expect(res.action).toBe("noop");
    expect(res.tampered).toBe(true);
    expect(res.content).toBe(tampered);
  });

  test("without fail-on, the fresh stamp wins over a tampered banner", () => {
    const stamped = stampBanner(
      "# Doc\n\nbody\n",
      "doc.md",
      payload,
      NONCE,
    ).content;
    const tampered = stamped.replace("capped at 5", "capped at 99");
    const res = stampBanner(tampered, "doc.md", payload, NONCE);
    expect(res.action).toBe("replace");
    expect(res.content).toContain("capped at 5");
    expect(res.content).not.toContain("capped at 99");
  });
});
