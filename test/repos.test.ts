import { afterEach, describe, expect, test } from "bun:test";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { runCheck } from "../src/engine/check.ts";
import {
  assertAliasKnown,
  isAliasedFile,
  resolveFile,
} from "../src/engine/repos.ts";
import { makeRepo, record, type TempRepo } from "./helpers.ts";

let repos: TempRepo[] = [];
afterEach(async () => {
  await Promise.all(repos.map((r) => r.cleanup()));
  repos = [];
});

describe("alias syntax", () => {
  test("`<alias>:<relative>` is aliased; plain paths, URLs, and drive letters are not", () => {
    expect(isAliasedFile("api:src/limits.ts")).toBe(true);
    expect(isAliasedFile("my-api:a.ts")).toBe(true);
    expect(isAliasedFile("src/limits.ts")).toBe(false);
    expect(isAliasedFile("https://example.com/x")).toBe(false);
    expect(isAliasedFile("C:\\repo\\x.ts")).toBe(false);
    expect(isAliasedFile("C:/repo/x.ts")).toBe(false);
  });

  test("an alias missing from config is a record-time error", () => {
    const config = { repos: { api: "../api" } };
    expect(() => assertAliasKnown(config, "api:src/x.ts")).not.toThrow();
    expect(() => assertAliasKnown(config, "src/x.ts")).not.toThrow();
    expect(() => assertAliasKnown(config, "web:src/x.ts")).toThrow(
      'unknown repo alias "web" in web:src/x.ts; add it to "repos" in .claims/config.json (known: api)',
    );
  });
});

describe("resolveFile", () => {
  test("a plain path is unchanged; an alias maps through config.repos", async () => {
    const r = await makeRepo();
    repos.push(r);
    await mkdir(join(r.root, "sib"), { recursive: true });
    const config = { repos: { api: "sib" } };
    expect(resolveFile(config, "src/x.ts", r.root)).toEqual({
      path: "src/x.ts",
    });
    expect(resolveFile(config, "api:src/x.ts", r.root)).toEqual({
      path: "sib/src/x.ts",
    });
  });

  test("an alias whose directory is missing is unavailable, with the reason label", async () => {
    const r = await makeRepo();
    repos.push(r);
    expect(
      resolveFile({ repos: { api: "../api" } }, "api:src/x.ts", r.root),
    ).toEqual({
      unavailable: true,
      alias: "api",
      reason: 'sibling repo "api" is not checked out at ../api',
    });
    expect(resolveFile({ repos: {} }, "api:src/x.ts", r.root)).toEqual({
      unavailable: true,
      alias: "api",
      reason: 'repo alias "api" is not defined in .claims/config.json repos',
    });
  });
});

describe("check with a sibling repo", () => {
  async function setup() {
    const r = await makeRepo();
    repos.push(r);
    await r.write("sib/src/limits.ts", "export const LIMIT = 100;\n");
    const { claim } = await record(r, {
      doc: "README.md",
      text: "The limit is 100 requests.",
      file: "sib/src/limits.ts",
      quote: "LIMIT = 100",
    });
    // Point the evidence side at the alias and declare it.
    claim.evidence = claim.evidence.map((b) => ({
      ...b,
      file: "api:src/limits.ts",
    }));
    await r.store.putClaim(claim);
    const cfgPath = join(r.store.dir, "config.json");
    const cfg = JSON.parse(await Bun.file(cfgPath).text());
    cfg.repos = { api: "sib" };
    await writeFile(cfgPath, JSON.stringify(cfg));
    return r;
  }

  test("a checked-out sibling resolves like a local file", async () => {
    const r = await setup();
    const rep = await runCheck(r.store);
    expect(rep.verdicts[0]?.evidence).toBe("unchanged");
    expect(rep.exitCode).toBe(0);
  });

  test("a missing sibling is unavailable: warns, never gates, labelled", async () => {
    const r = await setup();
    const { rm } = await import("node:fs/promises");
    await rm(join(r.root, "sib"), { recursive: true, force: true });
    const rep = await runCheck(r.store, { failOn: "gating" });
    const v = rep.verdicts[0];
    expect(v?.evidence).toBe("unavailable");
    expect(v?.gates).toBe(false);
    expect(v?.warns).toBe(true);
    expect(v?.reasons).toContain(
      'evidence: sibling repo "api" is not checked out at sib',
    );
    expect(rep.summary.evidence.unavailable).toBe(1);
    expect(rep.exitCode).toBe(0);
    expect((await runCheck(r.store, { failOn: "warn" })).exitCode).toBe(2);
  });
});
