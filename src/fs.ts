/** Tiny filesystem helpers shared across the engine, store, and CLI. */
import { realpathSync } from "node:fs";
import { access } from "node:fs/promises";
import {
  basename,
  dirname,
  isAbsolute,
  join,
  posix,
  relative,
  resolve,
  sep,
} from "node:path";
import { splitAlias } from "./engine/repos.ts";

/**
 * A user-supplied document or file path as the store keys it: a POSIX path
 * relative to the anchor root `root`. A relative path is taken relative to
 * `root`, so `./docs/a.md`, `docs/a.md`, and `<root>/docs/a.md` all give
 * `docs/a.md`. An aliased file (`api:./src/x.ts`) keeps its `alias:` prefix
 * and has its relative part normalized (`api:src/x.ts`). A path outside the
 * root keeps its leading `../` parts. An absolute path is compared after
 * resolving symlinks in both it and the root (`/var` and `/private/var` on
 * macOS name the same directory).
 */
export function normalizePath(root: string, p: string): string {
  const split = splitAlias(p);
  if (split !== undefined) {
    const rel = posix.normalize(split.relative.replaceAll("\\", "/"));
    return `${split.alias}:${rel}`;
  }
  const rel = isAbsolute(p)
    ? relative(realish(resolve(root)), realish(resolve(p)))
    : relative(resolve(root), resolve(root, p));
  return sep === "/" ? rel : rel.split(sep).join("/");
}

/** `abs` with symlinks resolved in its longest existing prefix. */
function realish(abs: string): string {
  const rest: string[] = [];
  let dir = abs;
  for (;;) {
    try {
      return join(realpathSync(dir), ...rest.reverse());
    } catch {
      const parent = dirname(dir);
      if (parent === dir) return abs;
      rest.push(basename(dir));
      dir = parent;
    }
  }
}

/** Does a path exist and is it accessible? Never throws. */
export async function exists(p: string): Promise<boolean> {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}
