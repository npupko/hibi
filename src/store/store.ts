/**
 * The committed claim store (SPEC 6). Holds authored records (documents and
 * claims) and cached judge verdicts, never computed verdicts. One file per
 * record so merges stay scoped.
 *
 * Layout (`.claims/` at the repository root):
 *   config.json                 { version, repos, judge, ignore }
 *   documents/<doc_id>.json
 *   claims/<clm_id>.json
 *   judge/<sha256>.json         cached judge verdicts (see src/judge/cache.ts)
 */

import { mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import {
  Claim,
  Config,
  DEFAULT_JUDGE_MODEL,
  DEFAULT_JUDGE_THRESHOLD,
  Document,
  MODEL_VERSION,
} from "../core/model.ts";
import { exists } from "../fs.ts";
import { JudgeCache } from "../judge/cache.ts";

export const STORE_DIR = ".claims";

/** A file's text, or null when it does not exist. */
async function readOrNull(path: string): Promise<string | null> {
  try {
    return await readFile(path, "utf8");
  } catch (e) {
    if ((e as { code?: string }).code === "ENOENT") return null;
    throw e;
  }
}

const SUBDIRS = {
  documents: "documents",
  claims: "claims",
  judge: "judge",
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

  private constructor(loc: { anchorRoot: string; dir: string }) {
    this.anchorRoot = loc.anchorRoot;
    this.dir = loc.dir;
  }

  /** The config a new store starts with. Every field is written out. */
  static defaultConfig(): Config {
    return {
      version: MODEL_VERSION,
      repos: {},
      judge: { model: DEFAULT_JUDGE_MODEL, threshold: DEFAULT_JUDGE_THRESHOLD },
      ignore: [],
    };
  }

  /**
   * Initialize a store; idempotent, never clobbers an existing config. An
   * existing store of another version is refused with the same message as
   * `open`: there is no upgrade path.
   */
  static async init(location: string | StoreLocation): Promise<ClaimStore> {
    const s = new ClaimStore(resolveLocation(location));
    const configPath = join(s.dir, "config.json");
    if (await exists(configPath)) await s.assertVersion();
    await mkdir(s.dir, { recursive: true });
    for (const sub of Object.values(SUBDIRS))
      await mkdir(join(s.dir, sub), { recursive: true });
    if (!(await exists(configPath))) {
      await writeFile(
        configPath,
        `${JSON.stringify(ClaimStore.defaultConfig(), null, 2)}\n`,
      );
    }
    // The judge cache is local (SPEC 6): `.claims/.gitignore` keeps it out of
    // git. Written only when missing; an existing file is left as it is.
    const gitignore = join(s.dir, ".gitignore");
    if (!(await exists(gitignore))) {
      await writeFile(gitignore, `${SUBDIRS.judge}/\n`);
    }
    return s;
  }

  /**
   * Open an existing store. A store of any version other than v4 is refused
   * before a record is parsed; there is no upgrade path.
   */
  static async open(location: string | StoreLocation): Promise<ClaimStore> {
    const s = new ClaimStore(resolveLocation(location));
    const configPath = join(s.dir, "config.json");
    if (!(await exists(configPath))) {
      throw new Error(`No claim store at ${s.dir}. Run \`hibi init\` first.`);
    }
    await s.assertVersion();
    await s.config();
    return s;
  }

  /** Throws the "no upgrade path" error when config.json is not a v4 store. */
  private async assertVersion(): Promise<void> {
    let raw: { version?: unknown };
    try {
      raw = JSON.parse(await readFile(join(this.dir, "config.json"), "utf8"));
    } catch (e) {
      throw new Error(
        `.claims/config.json is not valid JSON (${(e as Error).message}).`,
      );
    }
    const found =
      raw !== null && typeof raw.version === "string" ? raw.version : "unknown";
    if (found !== MODEL_VERSION) {
      throw new Error(
        `this claim store is version ${found}; hibi ${MODEL_VERSION} has no upgrade path from it. Remove ${this.dir}, run \`hibi init\`, and re-record.`,
      );
    }
  }

  static async isInitialized(
    location: string | StoreLocation,
  ): Promise<boolean> {
    return exists(join(resolveLocation(location).dir, "config.json"));
  }

  private configCache?: Config;

  async config(): Promise<Config> {
    if (this.configCache) return this.configCache;
    const raw = JSON.parse(
      await readFile(join(this.dir, "config.json"), "utf8"),
    );
    const parsed = Config.safeParse(raw);
    if (!parsed.success) {
      const issue = parsed.error.issues[0];
      const where = issue?.path.length ? issue.path.join(".") : "config";
      const message = issue?.message ?? "invalid";
      throw new Error(
        `.claims/config.json is invalid: ${message.startsWith(where) ? message : `${where}: ${message}`}`,
      );
    }
    this.configCache = parsed.data;
    return this.configCache;
  }

  /** The judge verdict cache at `<store>/judge/`. */
  judgeCache(): JudgeCache {
    return new JudgeCache(join(this.dir, SUBDIRS.judge));
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

  // ── Claims ──
  async putClaim(c: Claim): Promise<void> {
    await this.write(SUBDIRS.claims, c.id, Claim.parse(c));
  }
  async getClaim(id: string): Promise<Claim | undefined> {
    return this.read(SUBDIRS.claims, id, Claim);
  }
  async allClaims(): Promise<Claim[]> {
    return this.readAll(SUBDIRS.claims, Claim);
  }

  // ── Rollback ──

  /** Per record file written inside `withRollback`: its text before the first write (null: it did not exist) and after the last. */
  private journal?: Map<string, { before: string | null; after: string }>;

  /**
   * Run `fn`; when it throws, undo the record writes it made through this
   * store, then rethrow. A claim file it created is removed and a file it
   * overwrote gets its previous text back. A document record it created is
   * kept: documents are keyed by path and another writer may already
   * reference its id. A file it did not write, such as
   * a record another process wrote meanwhile, is not touched, and neither is
   * a file that changed again after this store's last write to it.
   */
  async withRollback<T>(fn: () => Promise<T>): Promise<T> {
    if (this.journal) throw new Error("withRollback does not nest.");
    const journal = new Map<string, { before: string | null; after: string }>();
    this.journal = journal;
    try {
      return await fn();
    } catch (e) {
      const documentsDir = join(this.dir, SUBDIRS.documents);
      for (const [path, { before, after }] of journal) {
        if ((await readOrNull(path)) !== after) continue;
        if (before === null) {
          // A document record is keyed by its path and created once; another
          // writer may already reference its id, so a created one is kept.
          if (dirname(path) === documentsDir) continue;
          await rm(path, { force: true });
        } else await writeFile(path, before);
      }
      throw e;
    } finally {
      this.journal = undefined;
    }
  }

  // ── private helpers ──
  private async write(sub: string, id: string, value: unknown): Promise<void> {
    await mkdir(join(this.dir, sub), { recursive: true });
    const path = join(this.dir, sub, `${id}.json`);
    const text = `${JSON.stringify(value, null, 2)}\n`;
    const noted = this.journal?.get(path);
    const before = noted
      ? noted.before
      : this.journal
        ? await readOrNull(path)
        : null;
    await writeFile(path, text);
    this.journal?.set(path, { before, after: text });
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
