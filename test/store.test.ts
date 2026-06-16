import { describe, test, expect, afterEach } from "bun:test";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { ClaimStore } from "../src/store/store.ts";
import { makeRepo, record, type TempRepo } from "./helpers.ts";

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

describe("claim store (§6, §8)", () => {
  test("init writes a config with a per-repo nonce and a cache .gitignore", async () => {
    const r = await repo();
    const config = await r.store.config();
    expect(config.nonce).toBe("deadbeef");
    expect(config.version).toBe("v1");
    const gi = await readFile(join(r.store.dir, ".gitignore"), "utf8");
    expect(gi).toContain("cache/");
  });

  test("nonces are random hex by default", () => {
    const n = ClaimStore.newNonce();
    expect(n).toMatch(/^[0-9a-f]{8}$/);
    expect(ClaimStore.newNonce()).not.toBe(n); // overwhelmingly likely
  });

  test("one file per claim — never a monolithic lockfile (§6)", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await r.write("src/b.ts", "export const B = 2;\n");
    await record(r, { doc: "d.md", text: "A is 1", file: "src/a.ts", quote: "A = 1" });
    await record(r, { doc: "d.md", text: "B is 2", file: "src/b.ts", quote: "B = 2" });
    const claimFiles = await readdir(join(r.store.dir, "claims"));
    expect(claimFiles.filter((f) => f.endsWith(".json")).length).toBe(2);
  });

  test("propositions dedup by content fingerprint (§5)", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\nexport const B = 2;\n");
    const r1 = await record(r, { doc: "d1.md", text: "Same claim text", file: "src/a.ts", quote: "A = 1" });
    const r2 = await record(r, { doc: "d2.md", text: "Same claim text", file: "src/a.ts", quote: "B = 2" });
    expect(r2.dedupedProposition).toBe(true);
    expect(r2.proposition.id).toBe(r1.proposition.id);
    const props = await r.store.allPropositions();
    expect(props.length).toBe(1); // one proposition, two assertions
    expect((await r.store.allAssertions()).length).toBe(2);
  });

  test("records validate against the schema on load", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    const { assertion } = await record(r, { doc: "d.md", text: "A is 1", file: "src/a.ts", quote: "A = 1" });
    const reopened = await ClaimStore.open(r.root);
    const loaded = await reopened.getAssertion(assertion.id);
    expect(loaded?.anchor.selectors.some((s) => s.kind === "ast-node")).toBe(true);
  });

  test("open throws when no store exists", async () => {
    await expect(ClaimStore.open("/no/such/store/xyz")).rejects.toThrow();
  });

  test("verified trust requires a precise anchor + ref (§10)", async () => {
    const r = await repo();
    await r.write("src/a.ts", "export const A = 1;\n");
    await expect(record(r, { doc: "d.md", text: "coarse verified", file: "src/a.ts", trust: "verified", coarse: true })).rejects.toThrow();
  });
});
