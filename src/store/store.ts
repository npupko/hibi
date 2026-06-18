/**
 * The committed claim store (§6, §8). Holds authored records (Documents,
 * Propositions, Assertions+Anchors) — *not* computed verdicts. Written as one
 * file per record so merges stay scoped and meaningful, never a monolithic
 * lockfile. A git-ignored cache may live alongside as a pure optimization.
 *
 * Layout (`.claims/` beside the docs):
 *   config.json                 — { version, nonce }
 *   documents/<id>.json
 *   propositions/<id>.json
 *   claims/<assertionId>.json   — the Assertion + its Anchor baseline
 */

import { randomUUID } from "node:crypto";
import { mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import {
  Assertion,
  Document,
  MODEL_VERSION,
  Proposition,
  StoreConfig,
} from "../core/model.ts";
import { exists } from "../fs.ts";

export const STORE_DIR = ".claims";

const SUBDIRS = {
  documents: "documents",
  propositions: "propositions",
  claims: "claims",
} as const;

/**
 * Where a store lives and what its anchors resolve against — decoupled (§8).
 * A bare string is the common case (`anchorRoot`, store at `<anchorRoot>/.claims`);
 * the object form lets a consumer (e.g. atlas) keep the store outside the tree it
 * anchors into, so one repo can carry many investigation-scoped stores.
 */
export interface StoreLocation {
  /** Absolute path the anchors resolve against (the repo / content root). */
  anchorRoot: string;
  /** Absolute path to the store directory. Default: `<anchorRoot>/.claims`. */
  storeDir?: string;
}

function resolveLocation(location: string | StoreLocation): {
  anchorRoot: string;
  dir: string;
} {
  // Normalize to absolute so anchors and the store resolve deterministically,
  // independent of the process cwd when the store is later used (§8).
  if (typeof location === "string") {
    const anchorRoot = resolve(location);
    return { anchorRoot, dir: join(anchorRoot, STORE_DIR) };
  }
  const anchorRoot = resolve(location.anchorRoot);
  return {
    anchorRoot,
    dir: location.storeDir
      ? resolve(location.storeDir)
      : join(anchorRoot, STORE_DIR),
  };
}

export class ClaimStore {
  /** Absolute path the anchors resolve against (the repo / content root). */
  readonly anchorRoot: string;
  /** Absolute path to the store directory (holds config + records). */
  readonly dir: string;

  private constructor(loc: { anchorRoot: string; dir: string }) {
    this.anchorRoot = loc.anchorRoot;
    this.dir = loc.dir;
  }

  /** Generate a fresh per-repository banner nonce: 8 regex-safe hex chars (§17.5). */
  static newNonce(): string {
    return randomUUID().replace(/-/g, "").slice(0, 8);
  }

  /** Initialize a store; idempotent — refuses to clobber an existing config. */
  static async init(
    location: string | StoreLocation,
    nonce = ClaimStore.newNonce(),
  ): Promise<ClaimStore> {
    const s = new ClaimStore(resolveLocation(location));
    await mkdir(s.dir, { recursive: true });
    for (const sub of Object.values(SUBDIRS))
      await mkdir(join(s.dir, sub), { recursive: true });
    const configPath = join(s.dir, "config.json");
    if (!(await exists(configPath))) {
      const config: StoreConfig = { version: MODEL_VERSION, nonce };
      await writeFile(configPath, `${JSON.stringify(config, null, 2)}\n`);
    }
    // git-ignore the regenerable cache (§6).
    await writeFile(join(s.dir, ".gitignore"), "cache/\n");
    return s;
  }

  /** Open an existing store; throws if not initialized. */
  static async open(location: string | StoreLocation): Promise<ClaimStore> {
    const s = new ClaimStore(resolveLocation(location));
    if (!(await exists(join(s.dir, "config.json")))) {
      throw new Error(`No claim store at ${s.dir}. Run \`hibi init\` first.`);
    }
    return s;
  }

  static async isInitialized(
    location: string | StoreLocation,
  ): Promise<boolean> {
    return exists(join(resolveLocation(location).dir, "config.json"));
  }

  async config(): Promise<StoreConfig> {
    const raw = JSON.parse(
      await readFile(join(this.dir, "config.json"), "utf8"),
    );
    return StoreConfig.parse(raw);
  }

  // ── Documents ──
  async putDocument(doc: Document): Promise<void> {
    await this.write(SUBDIRS.documents, doc.id, Document.parse(doc));
  }
  async getDocument(id: string): Promise<Document | undefined> {
    return this.read(SUBDIRS.documents, id, Document);
  }
  async allDocuments(): Promise<Document[]> {
    return this.readAll(SUBDIRS.documents, Document);
  }

  // ── Propositions ──
  async putProposition(p: Proposition): Promise<void> {
    await this.write(SUBDIRS.propositions, p.id, Proposition.parse(p));
  }
  async getProposition(id: string): Promise<Proposition | undefined> {
    return this.read(SUBDIRS.propositions, id, Proposition);
  }
  async allPropositions(): Promise<Proposition[]> {
    return this.readAll(SUBDIRS.propositions, Proposition);
  }
  /** Find an existing proposition by content fingerprint (the dedup unit, §5). */
  async findPropositionByFingerprint(
    fingerprint: string,
  ): Promise<Proposition | undefined> {
    const all = await this.allPropositions();
    return all.find((p) => p.fingerprint === fingerprint);
  }

  // ── Assertions (claims) ──
  async putAssertion(a: Assertion): Promise<void> {
    await this.write(SUBDIRS.claims, a.id, Assertion.parse(a));
  }
  async getAssertion(id: string): Promise<Assertion | undefined> {
    return this.read(SUBDIRS.claims, id, Assertion);
  }
  async allAssertions(): Promise<Assertion[]> {
    return this.readAll(SUBDIRS.claims, Assertion);
  }

  /** Remove a record (used by `retract`). */
  async deleteAssertion(id: string): Promise<void> {
    await rm(join(this.dir, SUBDIRS.claims, `${id}.json`), { force: true });
  }

  // ── private helpers ──
  private async write(sub: string, id: string, value: unknown): Promise<void> {
    await mkdir(join(this.dir, sub), { recursive: true });
    await writeFile(
      join(this.dir, sub, `${id}.json`),
      `${JSON.stringify(value, null, 2)}\n`,
    );
  }

  private async read<T>(
    sub: string,
    id: string,
    schema: { parse(v: unknown): T },
  ): Promise<T | undefined> {
    const path = join(this.dir, sub, `${id}.json`);
    if (!(await exists(path))) return undefined;
    return schema.parse(JSON.parse(await readFile(path, "utf8")));
  }

  private async readAll<T>(
    sub: string,
    schema: { parse(v: unknown): T },
  ): Promise<T[]> {
    const dir = join(this.dir, sub);
    if (!(await exists(dir))) return [];
    const files = (await readdir(dir)).filter((f) => f.endsWith(".json"));
    const out: T[] = [];
    for (const f of files.sort()) {
      out.push(schema.parse(JSON.parse(await readFile(join(dir, f), "utf8"))));
    }
    return out;
  }
}
