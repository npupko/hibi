/**
 * The judge cache (SPEC 6.4): one file per verdict at
 * `.claims/judge/<key>.json`, where `key` is the sha256 of the model id and
 * the canonical request JSON. A verdict is never re-asked for the same key, so
 * a second run over the same tree cannot flip it.
 */

import { createHash } from "node:crypto";
import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { JudgeCacheEntry, type JudgeSubject } from "../core/model.ts";
import { exists } from "../fs.ts";

/**
 * Canonical JSON: object keys sorted by UTF-16 code unit, arrays kept in
 * order, no whitespace. Keys whose value is `undefined` are dropped and
 * `undefined` array items become `null`, as `JSON.stringify` does. Values JSON
 * cannot represent (non-finite numbers, bigints, functions, symbols) throw.
 */
export function canonicalJson(value: unknown): string {
  if (value === null) return "null";
  switch (typeof value) {
    case "string":
    case "boolean":
      return JSON.stringify(value);
    case "number":
      if (!Number.isFinite(value)) {
        throw new Error(`canonicalJson: non-finite number ${value}`);
      }
      return JSON.stringify(value);
    case "object": {
      if (Array.isArray(value)) {
        return `[${value
          .map((v) => (v === undefined ? "null" : canonicalJson(v)))
          .join(",")}]`;
      }
      const obj = value as Record<string, unknown>;
      const keys = Object.keys(obj)
        .filter((k) => obj[k] !== undefined)
        .sort();
      return `{${keys
        .map((k) => `${JSON.stringify(k)}:${canonicalJson(obj[k])}`)
        .join(",")}}`;
    }
    default:
      throw new Error(`canonicalJson: cannot serialize a ${typeof value}`);
  }
}

/** The cache key: sha256 hex of `<model>\n<canonical request JSON>`. */
export function cacheKey(model: string, request: unknown): string {
  return createHash("sha256")
    .update(`${model}\n${canonicalJson(request)}`)
    .digest("hex");
}

function matches(subject: JudgeSubject, filter: JudgeSubject): boolean {
  for (const k of ["claimId", "pairId", "docId"] as const) {
    if (filter[k] !== undefined && subject[k] !== filter[k]) return false;
  }
  return true;
}

export class JudgeCache {
  constructor(readonly dir: string) {}

  key(model: string, request: unknown): string {
    return cacheKey(model, request);
  }

  async get(key: string): Promise<JudgeCacheEntry | undefined> {
    const path = join(this.dir, `${key}.json`);
    if (!(await exists(path))) return undefined;
    return parseEntry(`${key}.json`, await readFile(path, "utf8"));
  }

  async put(entry: JudgeCacheEntry): Promise<void> {
    const parsed = JudgeCacheEntry.parse(entry);
    await mkdir(this.dir, { recursive: true });
    await writeFile(
      join(this.dir, `${parsed.key}.json`),
      `${JSON.stringify(parsed, null, 2)}\n`,
    );
  }

  /** Entries whose subject matches every field set in `subject`, oldest first. */
  async list(subject: JudgeSubject = {}): Promise<JudgeCacheEntry[]> {
    if (!(await exists(this.dir))) return [];
    const files = (await readdir(this.dir)).filter((f) => f.endsWith(".json"));
    const out: JudgeCacheEntry[] = [];
    for (const f of files.sort()) {
      const entry = parseEntry(f, await readFile(join(this.dir, f), "utf8"));
      if (matches(entry.subject, subject)) out.push(entry);
    }
    return out.sort((a, b) =>
      a.at < b.at ? -1 : a.at > b.at ? 1 : a.key < b.key ? -1 : 1,
    );
  }
}

function parseEntry(where: string, raw: string): JudgeCacheEntry {
  try {
    return JudgeCacheEntry.parse(JSON.parse(raw));
  } catch (e) {
    throw new Error(
      `judge cache entry judge/${where} failed schema validation. Cause: ${(e as Error).message}`,
    );
  }
}
