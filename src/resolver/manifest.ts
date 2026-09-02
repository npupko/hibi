/**
 * Default-deny resolver manifest. Third-party out-of-process resolvers run
 * only if listed in `.claims/resolvers.json`. Absent file: no external resolvers.
 */

import { access, readFile } from "node:fs/promises";
import { join } from "node:path";
import * as z from "zod";

export const ResolverSpec = z.strictObject({
  name: z.string(),
  command: z.string(),
  args: z.array(z.string()).default([]),
  /** Per-request timeout; a slow resolver is killed. */
  timeoutMs: z.number().int().positive().default(5000),
  /** Optional explicit allow-list of kinds; otherwise taken from `describe`. */
  kinds: z.array(z.string()).optional(),
  /**
   * Allow a non-advisory resolver to claim a built-in kind (`text-quote`,
   * `text-position`, `ast-node`, `value`, `coarse`) and so replace the
   * deterministic core verdict. Off by default.
   */
  override: z.boolean().default(false),
  /** LLM-backed: its advisories must carry structured `provenance`. */
  modelBacked: z.boolean().default(false),
});
export type ResolverSpec = z.infer<typeof ResolverSpec>;

export const Manifest = z.strictObject({
  resolvers: z.array(ResolverSpec).default([]),
});
export type Manifest = z.infer<typeof Manifest>;

export function manifestPath(storeDir: string): string {
  return join(storeDir, "resolvers.json");
}

/** Load the manifest; default-deny (empty) when absent or unreadable. */
export async function loadManifest(storeDir: string): Promise<Manifest> {
  const path = manifestPath(storeDir);
  try {
    await access(path);
  } catch {
    return { resolvers: [] };
  }
  try {
    return Manifest.parse(JSON.parse(await readFile(path, "utf8")));
  } catch {
    return { resolvers: [] };
  }
}
