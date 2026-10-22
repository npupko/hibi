/**
 * Cross-repository aliases (SPEC 4.1). `config.repos` maps an alias to a
 * directory relative to the repository root: `{ "api": "../api" }`. An
 * evidence side's `file` may then be `api:src/limits.ts`.
 *
 * - An alias missing from `config.repos` is a record-time error
 *   (`assertAliasKnown`).
 * - An alias whose directory does not exist resolves the side to
 *   `unavailable`: a warning, never a gate, with a reason label.
 */

import { existsSync, statSync } from "node:fs";
import { isAbsolute, join, posix } from "node:path";
import type { Config } from "../core/model.ts";

/**
 * `<alias>:<relative>`. The relative part may not start with `/` or `\`, so
 * `https://…` and `C:\…` are not aliased files.
 */
const ALIASED = /^([A-Za-z][\w.-]*):([^\\/].*)$/;

export type ResolvedFile =
  | { path: string }
  | {
      unavailable: true;
      alias: string;
      /** The reason label reported on the verdict. */
      reason: string;
    };

/** The alias and relative part of `file`, or undefined when it has no alias. */
export function splitAlias(
  file: string,
): { alias: string; relative: string } | undefined {
  const m = ALIASED.exec(file);
  if (m === null) return undefined;
  return { alias: m[1] as string, relative: m[2] as string };
}

/** True when `file` has the `<alias>:<relative>` form. */
export function isAliasedFile(file: string): boolean {
  return splitAlias(file) !== undefined;
}

/** Throws when `file` names an alias that `config.repos` does not define. `record` calls this. */
export function assertAliasKnown(
  config: Pick<Config, "repos">,
  file: string,
): void {
  const split = splitAlias(file);
  if (split === undefined) return;
  if (!Object.hasOwn(config.repos, split.alias)) {
    const known = Object.keys(config.repos);
    throw new Error(
      `unknown repo alias "${split.alias}" in ${file}; add it to "repos" in .claims/config.json${known.length ? ` (known: ${known.join(", ")})` : ""}`,
    );
  }
}

function isDirectory(abs: string): boolean {
  try {
    return existsSync(abs) && statSync(abs).isDirectory();
  } catch {
    return false;
  }
}

/**
 * Map a side's `file` to a path relative to `root` (the repository root), or
 * report the side `unavailable`. A file with no alias is returned unchanged.
 */
export function resolveFile(
  config: Pick<Config, "repos">,
  file: string,
  root: string,
): ResolvedFile {
  const split = splitAlias(file);
  if (split === undefined) return { path: file };
  const dir = config.repos[split.alias];
  if (dir === undefined) {
    return {
      unavailable: true,
      alias: split.alias,
      reason: `repo alias "${split.alias}" is not defined in .claims/config.json repos`,
    };
  }
  const abs = isAbsolute(dir) ? dir : join(root, dir);
  if (!isDirectory(abs)) {
    return {
      unavailable: true,
      alias: split.alias,
      reason: `sibling repo "${split.alias}" is not checked out at ${dir}`,
    };
  }
  return { path: posix.join(dir, split.relative) };
}
