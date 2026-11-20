import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  documentIdForPath,
  recordClaim,
  resolveRegion,
} from "../src/engine/record.ts";
import { exists } from "../src/fs.ts";
import { ClaimStore } from "../src/store/store.ts";
import { makeRepo, record, type TempRepo } from "./helpers.ts";

describe("resolveRegion span parsing", () => {
  const content = "line one\nline two\nline three\n";
  test("a 1-based line range maps to char offsets", () => {
    // Lines 1-2 cover "line one\nline two" (chars 0..17).
    expect(resolveRegion(content, { startLine: 1, endLine: 2 })).toEqual({
      start: 0,
      end: 17,
    });
  });
  test("a quote maps to its first occurrence", () => {
    expect(resolveRegion(content, { quote: "line two" })).toEqual({
      start: 9,
      end: 17,
    });
    expect(() => resolveRegion(content, { quote: "absent" })).toThrow(
      "Quote not found",
    );
  });
  test("an explicit char range stays char offsets", () => {
    expect(resolveRegion(content, { start: 0, end: 8 })).toEqual({
      start: 0,
      end: 8,
    });
  });
  test("a malformed (NaN) range is rejected, never silently anchored", () => {
    expect(() =>
      resolveRegion(content, { start: Number.NaN, end: 5 }),
    ).toThrow();
    expect(() =>
      resolveRegion(content, { startLine: Number.NaN, endLine: 2 }),
    ).toThrow();
  });
  test("a line past the end of the file is rejected", () => {
    expect(() => resolveRegion(content, { startLine: 9, endLine: 9 })).toThrow(
      "past the end of the file",
    );
  });
  test("an empty spec is rejected", () => {
    expect(() => resolveRegion(content, {})).toThrow();
  });
});

let repos: TempRepo[] = [];
async function repo() {
  const r = await makeRepo();
  repos.push(r);
  return r;
}
afterEach(async () => {
  await Promise.all(repos.map((r) => r.cleanup()));
  repos = [];
});

describe("claim store", () => {
  test("init writes a complete v4 config and the three directories", async () => {
    const r = await repo();
    const config = await r.store.config();
    expect(config).toEqual({
      version: "v4",
      repos: {},
      judge: { model: "jev-1.14.0", threshold: 0.8 },
      ignore: [],
    });
    const entries = (await readdir(r.store.dir)).sort();
    expect(entries).toEqual([
      ".gitignore",
      "claims",
      "config.json",
      "documents",
      "judge",
    ]);
  });

  test("one file per claim, never a monolithic lockfile", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("src/b.ts", "export const B = 2;\n");
    await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
    });
    await record(r, {
      doc: "d.md",
      text: "B is 2 here",
      file: "src/b.ts",
      quote: "B = 2",
    });
    const claimFiles = await readdir(join(r.store.dir, "claims"));
    expect(claimFiles.filter((f) => f.endsWith(".json")).length).toBe(2);
  });

  test("a recorded claim holds its sentence, v4 ids, and one initial vouch", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    const { claim, document } = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
      by: "human:alice@example.com",
    });
    expect(claim.id).toMatch(/^clm_[0-9a-f]{16}$/);
    expect(document.id).toMatch(/^doc_[0-9a-f]{16}$/);
    expect(claim.documentId).toBe(document.id);
    expect(claim.kind).toBe("code");
    expect(claim.sentence).toBe("A is 1 here");
    expect(claim.attestations).toHaveLength(1);
    expect(claim.attestations[0]?.by).toBe("human:alice@example.com");
    expect(claim.attestations[0]?.basis).toBe("vouch");
    expect(claim.createdAt).toBe(claim.attestations[0]?.at as string);
    expect(document.createdAt).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });

  test("the same sentence in two documents is two claims, each pointing at the other", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\nexport const B = 2;\n");
    const r1 = await record(r, {
      doc: "d1.md",
      text: "Same claim text",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const r2 = await record(r, {
      doc: "d2.md",
      text: "Same claim text",
      file: "src/a.ts",
      quote: "B = 2",
    });
    expect(r2.claim.id).not.toBe(r1.claim.id);
    expect(r2.existingClaims).toEqual([r1.claim.id]);
    expect((await r.store.allClaims()).length).toBe(2);
  });

  test("re-recording the same sentence on the same document updates the claim and appends a vouch", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    const first = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const second = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
      by: "agent:opus",
    });
    expect(second.claim.id).toBe(first.claim.id);
    expect(second.updated).toBe(true);
    expect(second.claim.attestations.map((a) => a.by)).toEqual([
      "human:tester@example.com",
      "agent:opus",
    ]);
    expect((await r.store.allClaims()).length).toBe(1);
  });

  test("records carry doc and evidence bundles that validate on load", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    const { claim } = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const reopened = await ClaimStore.open(r.root);
    const loaded = await reopened.getClaim(claim.id);
    expect(loaded?.doc.selectors.length).toBeGreaterThan(0);
    expect(loaded?.evidence.length).toBeGreaterThan(0);
    expect(
      loaded?.evidence.some((b) =>
        b.selectors.some((s) => s.kind === "ast-node"),
      ),
    ).toBe(true);
    // A precise code span records `enforced` by default.
    expect(loaded?.enforcement).toBe("enforced");
  });

  test("a line-range code target anchors the whole line", async () => {
    const r = await repo();
    await r.write("src/a.ts", "// header\nexport const A = 1;\n");
    const { claim } = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      line: 2,
    });
    const tq = claim.evidence[0]?.selectors.find(
      (s) => s.kind === "text-quote",
    );
    expect(tq?.kind === "text-quote" ? tq.exact : "").toBe(
      "export const A = 1;",
    );
  });

  test("open throws when no store exists", async () => {
    await expect(ClaimStore.open("/no/such/store/xyz")).rejects.toThrow();
  });

  test("open refuses a v3 store with a one-line no-upgrade error", async () => {
    const r = await repo();
    await writeFile(
      join(r.store.dir, "config.json"),
      `${JSON.stringify({ version: "v3", nonce: "deadbeef" })}\n`,
    );
    const err = await ClaimStore.open(r.root).then(
      () => null,
      (e: Error) => e,
    );
    expect(err?.message).toContain("version v3");
    expect(err?.message).toContain("no upgrade path");
    expect(err?.message).not.toContain("\n");
  });

  test("open refuses a judge model alias with a one-line error", async () => {
    const r = await repo();
    const config = { ...ClaimStore.defaultConfig() };
    config.judge = { model: "jev-latest", threshold: 0.8 };
    await writeFile(join(r.store.dir, "config.json"), JSON.stringify(config));
    const err = await ClaimStore.open(r.root).then(
      () => null,
      (e: Error) => e,
    );
    expect(err?.message).toContain("pinned versioned id");
    expect(err?.message).not.toContain("\n");
  });

  test("open refuses a threshold outside [0, 1]", async () => {
    const r = await repo();
    const config = { ...ClaimStore.defaultConfig() };
    config.judge = { model: "jev-1.14.0", threshold: 1.5 };
    await writeFile(join(r.store.dir, "config.json"), JSON.stringify(config));
    await expect(ClaimStore.open(r.root)).rejects.toThrow(
      "judge.threshold must be a number in [0, 1]",
    );
  });

  test("a doc quote shorter than 8 characters is refused", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await expect(
      record(r, {
        doc: "d.md",
        text: "A is 1",
        file: "src/a.ts",
        quote: "A = 1",
      }),
    ).rejects.toThrow("too short");
  });

  test("a recording actor outside the OKF forms is refused", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await expect(
      record(r, {
        doc: "d.md",
        text: "A is 1 here",
        file: "src/a.ts",
        quote: "A = 1",
        by: "alice",
      }),
    ).rejects.toThrow("human:<id>, agent:<name>, or process:<name>");
  });
});

describe("withRollback", () => {
  test("a failure undoes this store's writes only", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    const a = await record(r, {
      doc: "d.md",
      text: "A is 1 here",
      file: "src/a.ts",
      quote: "A = 1",
    });
    const file = (id: string) => `.claims/claims/${id}.json`;
    const before = await r.read(file(a.claim.id));
    const other = await ClaimStore.open(r.root);
    const ours = "clm_1111111111111111";
    const oursThenTheirs = "clm_2222222222222222";
    const theirs = "clm_3333333333333333";
    await expect(
      r.store.withRollback(async () => {
        await r.store.putClaim({ ...a.claim, sentence: "overwritten" });
        await r.store.putClaim({ ...a.claim, id: ours });
        await r.store.putClaim({ ...a.claim, id: oursThenTheirs });
        // Another writer, meanwhile: a new record, and a later write over one of ours.
        await other.putClaim({ ...a.claim, id: theirs });
        await other.putClaim({
          ...a.claim,
          id: oursThenTheirs,
          sentence: "their write",
        });
        throw new Error("boom");
      }),
    ).rejects.toThrow("boom");
    expect(await r.read(file(a.claim.id))).toBe(before);
    expect(await r.store.getClaim(ours)).toBeUndefined();
    expect((await r.store.getClaim(oursThenTheirs))?.sentence).toBe(
      "their write",
    );
    expect((await r.store.getClaim(theirs))?.id).toBe(theirs);
  });
});

describe("withRollback keeps created documents", () => {
  test("a rolled-back batch keeps the document it created when another writer recorded a claim on it", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\nexport const B = 2;\n");
    await r.write("d.md", "# D\n\nA is 1 here.\nB is 2 here.\n");
    const contents = async () => ({
      docContent: await r.read("d.md"),
      evidenceContents: { "src/a.ts": await r.read("src/a.ts") },
    });
    const writerB = await ClaimStore.open(r.root);
    let ours = "";
    let theirs = "";
    await expect(
      r.store.withRollback(async () => {
        // Batch A creates the document record for d.md.
        ours = (
          await recordClaim(r.store, await contents(), {
            docPath: "d.md",
            docSpec: { quote: "A is 1 here." },
            kind: "code",
            evidence: [{ file: "src/a.ts", region: { quote: "A = 1" } }],
            by: "agent:a",
            ref: "testref",
          })
        ).claim.id;
        // Writer B records a claim on the same path meanwhile.
        theirs = (
          await recordClaim(writerB, await contents(), {
            docPath: "d.md",
            docSpec: { quote: "B is 2 here." },
            kind: "code",
            evidence: [{ file: "src/a.ts", region: { quote: "B = 2" } }],
            by: "agent:b",
            ref: "testref",
          })
        ).claim.id;
        throw new Error("apply failed");
      }),
    ).rejects.toThrow("apply failed");
    expect(await r.store.getClaim(ours)).toBeUndefined();
    const b = await r.store.getClaim(theirs);
    expect(b?.id).toBe(theirs);
    expect(b?.documentId).toBe(documentIdForPath("d.md"));
    expect((await r.store.getDocument(documentIdForPath("d.md")))?.path).toBe(
      "d.md",
    );
  });
});

describe("record by kind", () => {
  test("a reference claim stores exactly one document span and no AST selectors", async () => {
    const r = await repo();
    await r.write("d.md", "Pro costs $29/month.\n");
    await r.write(
      "pricing.md",
      "# Pricing\n\nThe Pro plan is $29 per month.\n",
    );
    const { claim } = await recordClaim(
      r.store,
      {
        docContent: await r.read("d.md"),
        evidenceContents: { "pricing.md": await r.read("pricing.md") },
      },
      {
        docPath: "d.md",
        docSpec: { quote: "Pro costs $29/month." },
        kind: "reference",
        evidence: [
          {
            file: "pricing.md",
            region: { quote: "The Pro plan is $29 per month." },
          },
        ],
        by: "human:tester@example.com",
        ref: "r",
      },
    );
    expect(claim.kind).toBe("reference");
    expect(claim.evidence).toHaveLength(1);
    expect(claim.evidence[0]?.selectors.map((s) => s.kind)).toEqual([
      "text-quote",
      "text-position",
    ]);
  });

  test("an attested claim stores sources and a procedure, and needs a ttl", async () => {
    const r = await repo();
    await r.write("d.md", "Pro costs $29/month.\n");
    const base = {
      docPath: "d.md",
      docSpec: { quote: "Pro costs $29/month." },
      kind: "attested" as const,
      evidence: [],
      sources: [{ id: "stripe", resource: "https://dashboard.stripe.com" }],
      procedure: { instructions: "Open the dashboard and read the price." },
      by: "human:tester@example.com",
      ref: "r",
    };
    const contents = {
      docContent: await r.read("d.md"),
      evidenceContents: {},
    };
    await expect(recordClaim(r.store, contents, base)).rejects.toThrow(
      "requires a ttl",
    );
    await expect(
      recordClaim(r.store, contents, { ...base, ttl: "90 days" }),
    ).rejects.toThrow("ISO 8601 duration");
    const { claim } = await recordClaim(r.store, contents, {
      ...base,
      ttl: "P90D",
    });
    expect(claim.evidence).toEqual([]);
    expect(claim.ttl).toBe("P90D");
    expect(claim.sources[0]?.resource).toBe("https://dashboard.stripe.com");
    expect(claim.procedure?.instructions).toContain("dashboard");
  });
});

describe("B.10 .claims/.gitignore", () => {
  let dirs: string[] = [];
  afterEach(async () => {
    await Promise.all(dirs.map((d) => rm(d, { recursive: true, force: true })));
    dirs = [];
  });

  test("B.10: init writes .claims/.gitignore with judge/, adds it to an existing store, and never rewrites it", async () => {
    const root = await mkdtemp(join(tmpdir(), "ce-gitignore-"));
    dirs.push(root);
    const file = join(root, ".claims", ".gitignore");

    await ClaimStore.init(root);
    expect(await readFile(file, "utf8")).toBe("judge/\n");
    expect(await exists(join(root, ".gitignore"))).toBe(false);

    // An existing store without the file gets it on the next init.
    await rm(file);
    const config = await readFile(join(root, ".claims", "config.json"), "utf8");
    await ClaimStore.init(root);
    expect(await readFile(file, "utf8")).toBe("judge/\n");
    expect(await readFile(join(root, ".claims", "config.json"), "utf8")).toBe(
      config,
    );

    // A file the user edited is left as it is.
    await writeFile(file, "judge/\nscratch/\n");
    await ClaimStore.init(root);
    expect(await readFile(file, "utf8")).toBe("judge/\nscratch/\n");
  });
});
