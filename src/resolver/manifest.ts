/**
 * Default-deny resolver manifest (§7.1). Third-party out-of-process resolvers run
 * ONLY if they are explicitly listed in `.claims/resolvers.json`. Absent file →
 * no external resolvers (default-deny).
 */

import { access, readFile } from "node:fs/promises";
import { join } from "node:path";
import * as z from "zod";

export const ResolverSpec = z.strictObject({
  name: z.string(),
  command: z.string(),
  args: z.array(z.string()).default([]),
  /** Per-request timeout; a slow resolver is killed (§7.1). */
  timeoutMs: z.number().int().positive().default(5000),
  /** Optional explicit allow-list of kinds; otherwise taken from `describe`. */
  kinds: z.array(z.string()).optional(),
  /**
   * The resolver is LLM-backed (§19, D29): its advisories must carry structured
   * `provenance` (model, promptHash, contextHash). The registry drops any
   * provenance-less advisory from a `modelBacked` resolver and warns once per run.
   */
  modelBacked: z.boolean().default(false),
});
export type ResolverSpec = z.infer<typeof ResolverSpec>;

export const Manifest = z.strictObject({
  resolvers: z.array(ResolverSpec).default([]),
});
export type Manifest = z.infer<typeof Manifest>;

/** The manifest lives inside the store dir (decoupled from the anchor root, §8). */
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
