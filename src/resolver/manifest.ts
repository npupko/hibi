/**
 * Default-deny resolver manifest (§7.1). Third-party out-of-process resolvers run
 * ONLY if they are explicitly listed in `.claims/resolvers.json`. Absent file →
 * no external resolvers (default-deny).
 */
import { join } from "node:path";
import { readFile, access } from "node:fs/promises";
import * as z from "zod";
import { STORE_DIR } from "../store/store.ts";

export const ResolverSpec = z.object({
  name: z.string(),
  command: z.string(),
  args: z.array(z.string()).default([]),
  /** Per-request timeout; a slow resolver is killed (§7.1). */
  timeoutMs: z.number().int().positive().default(5000),
  /** Optional explicit allow-list of kinds; otherwise taken from `describe`. */
  kinds: z.array(z.string()).optional(),
});
export type ResolverSpec = z.infer<typeof ResolverSpec>;

export const Manifest = z.object({
  resolvers: z.array(ResolverSpec).default([]),
});
export type Manifest = z.infer<typeof Manifest>;

export function manifestPath(root: string): string {
  return join(root, STORE_DIR, "resolvers.json");
}

/** Load the manifest; default-deny (empty) when absent or unreadable. */
export async function loadManifest(root: string): Promise<Manifest> {
  const path = manifestPath(root);
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
