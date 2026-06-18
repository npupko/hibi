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
import {
  access,
  mkdir,
  readdir,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { join } from "node:path";
import {
  Assertion,
  Document,
  MODEL_VERSION,
  Proposition,
  StoreConfig,
} from "../core/model.ts";

export const STORE_DIR = ".claims";

const SUBDIRS = {
  documents: "documents",
  propositions: "propositions",
  claims: "claims",
} as const;

export class ClaimStore {
  readonly root: string; // absolute path to the repo root
  readonly dir: string; // absolute path to .claims

  private constructor(root: string) {
    this.root = root;
    this.dir = join(root, STORE_DIR);
  }

  /** Generate a fresh per-repository banner nonce: 8 regex-safe hex chars (§17.5). */
  static newNonce(): string {
    return randomUUID().replace(/-/g, "").slice(0, 8);
  }

  /** Initialize a store at `root`; idempotent — refuses to clobber an existing config. */
  static async init(
    root: string,
    nonce = ClaimStore.newNonce(),
  ): Promise<ClaimStore> {
    const s = new ClaimStore(root);
    await mkdir(s.dir, { recursive: true });
    for (const sub of Object.values(SUBDIRS))
      await mkdir(join(s.dir, sub), { recursive: true });
    const configPath = join(s.dir, "config.json");
    if (!(await exists(configPath))) {
      const config: StoreConfig = { version: MODEL_VERSION, nonce };
      await writeFile(configPath, JSON.stringify(config, null, 2) + "\n");
    }
    // git-ignore the regenerable cache (§6).
    await writeFile(join(s.dir, ".gitignore"), "cache/\n");
    return s;
  }

  /** Open an existing store; throws if not initialized. */
  static async open(root: string): Promise<ClaimStore> {
    const s = new ClaimStore(root);
    if (!(await exists(join(s.dir, "config.json")))) {
      throw new Error(`No claim store at ${s.dir}. Run \`hibi init\` first.`);
    }
    return s;
  }

  static async isInitialized(root: string): Promise<boolean> {
    return exists(join(root, STORE_DIR, "config.json"));
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
      JSON.stringify(value, null, 2) + "\n",
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

async function exists(p: string): Promise<boolean> {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}
