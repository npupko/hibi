/**
 * The committed claim store. Holds authored records (documents, propositions,
 * assertions with anchors), never computed verdicts. One file per record so
 * merges stay scoped.
 *
 * Layout (`.claims/` beside the docs):
 *   config.json                 — { version, nonce, instructionFiles?, pristine? }
 *   documents/<id>.json
 *   propositions/<id>.json
 *   claims/<assertionId>.json
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
import { upgradeV2Store } from "./upgrade.ts";

export const STORE_DIR = ".claims";

const SUBDIRS = {
  documents: "documents",
  propositions: "propositions",
  claims: "claims",
} as const;

/**
 * Where a store lives and what its anchors resolve against. A bare string is
 * the common case (`anchorRoot`, store at `<anchorRoot>/.claims`); the object
 * form keeps the store outside the tree it anchors into.
 */
export interface StoreLocation {
  anchorRoot: string;
  storeDir?: string;
}

function resolveLocation(location: string | StoreLocation): {
  anchorRoot: string;
  dir: string;
} {
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
  readonly anchorRoot: string;
  readonly dir: string;
  /** Set when `open` upgraded a v2 store in place. */
  upgradedFrom?: string;

  private constructor(loc: { anchorRoot: string; dir: string }) {
    this.anchorRoot = loc.anchorRoot;
    this.dir = loc.dir;
  }

  /** A fresh per-repository banner nonce: 8 hex chars. */
  static newNonce(): string {
    return randomUUID().replace(/-/g, "").slice(0, 8);
  }

  /** Initialize a store; idempotent, never clobbers an existing config. */
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
    return s;
  }

  /**
   * Open an existing store. A v2 store is upgraded in place once; any other
   * version skew is refused before a claim file is parsed.
   */
  static async open(location: string | StoreLocation): Promise<ClaimStore> {
    const s = new ClaimStore(resolveLocation(location));
    const configPath = join(s.dir, "config.json");
    if (!(await exists(configPath))) {
      throw new Error(`No claim store at ${s.dir}. Run \`hibi init\` first.`);
    }
    const raw = JSON.parse(await readFile(configPath, "utf8")) as {
      version?: string;
    };
    const found = raw.version ?? "unknown";
    if (found === "v2") {
      await upgradeV2Store(s.dir);
      s.upgradedFrom = "v2";
      return s;
    }
    if (found !== MODEL_VERSION) {
      throw new Error(
        `this store was written by hibi model ${found} and this binary requires ${MODEL_VERSION}. Re-run 'hibi init' and re-record, or use a matching hibi version.`,
      );
    }
    return s;
  }

  static async isInitialized(
    location: string | StoreLocation,
  ): Promise<boolean> {
    return exists(join(resolveLocation(location).dir, "config.json"));
  }

  private configCache?: StoreConfig;

  async config(): Promise<StoreConfig> {
    if (this.configCache) return this.configCache;
    const raw = JSON.parse(
      await readFile(join(this.dir, "config.json"), "utf8"),
    );
    this.configCache = StoreConfig.parse(raw);
    return this.configCache;
  }

  // ── Documents ──
  async putDocument(doc: Document): Promise<void> {
    await this.write(SUBDIRS.documents, doc.id, Document.parse(doc));
  }
  async getDocument(id: string): Promise<Document | undefined> {
    return this.read(SUBDIRS.documents, id, Document);
  }
  async deleteDocument(id: string): Promise<void> {
    await rm(join(this.dir, SUBDIRS.documents, `${id}.json`), { force: true });
  }
  async allDocuments(): Promise<Document[]> {
    return this.readAll(SUBDIRS.documents, Document);
  }

  // ── Propositions ──
  async putProposition(p: Proposition): Promise<void> {
    await this.write(SUBDIRS.propositions, p.id, Proposition.parse(p));
  }
  async deleteProposition(id: string): Promise<void> {
    await rm(join(this.dir, SUBDIRS.propositions, `${id}.json`), {
      force: true,
    });
  }
  async getProposition(id: string): Promise<Proposition | undefined> {
    return this.read(SUBDIRS.propositions, id, Proposition);
  }
  async allPropositions(): Promise<Proposition[]> {
    return this.readAll(SUBDIRS.propositions, Proposition);
  }
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
    return this.parseRecord(
      schema,
      `${sub}/${id}.json`,
      await readFile(path, "utf8"),
    );
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
      out.push(
        this.parseRecord(
          schema,
          `${sub}/${f}`,
          await readFile(join(dir, f), "utf8"),
        ),
      );
    }
    return out;
  }

  private parseRecord<T>(
    schema: { parse(v: unknown): T },
    where: string,
    raw: string,
  ): T {
    try {
      return schema.parse(JSON.parse(raw));
    } catch (e) {
      throw new Error(
        `Claim store record ${where} failed schema validation (an unknown or malformed field). The store version is checked at open, so this is a corrupt record, not version skew. Cause: ${(e as Error).message}`,
      );
    }
  }
}
