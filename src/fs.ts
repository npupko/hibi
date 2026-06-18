/** Tiny filesystem helpers shared across the engine, store, and CLI. */
import { access } from "node:fs/promises";

/** Does a path exist and is it accessible? Never throws. */
export async function exists(p: string): Promise<boolean> {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}
