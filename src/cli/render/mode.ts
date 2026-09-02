/**
 * Resolve the output mode once, from flags + TTY + environment. Machines pipe
 * (non-TTY) and get compact JSON; an interactive human on a TTY sees the rich
 * rendering. `--format` overrides; `--json` is an alias for `--format json`.
 *
 * Precedence: explicit flag > env (`NO_COLOR`/`FORCE_COLOR`/`HIBI_ASCII`) > TTY.
 */

export type OutputKind = "json" | "json-pretty" | "rich" | "compact";

export interface OutputMode {
  kind: OutputKind;
  color: boolean;
  unicode: boolean;
  /** Include the evidence tail (`--explain`). */
  explain: boolean;
  /** Emit the `remediation` menu; off via `--no-hints` / `HIBI_ADVICE=0`. */
  hints: boolean;
}

export interface ModeFlags {
  /** `human` | `compact` | `json` | `json-pretty`. */
  format?: string;
  json?: boolean;
  /** `auto` | `always` | `never` (anything else is treated as `auto`). */
  color?: string;
  explain?: boolean;
  noHints?: boolean;
}

export interface ModeEnv {
  isTTY?: boolean;
  env?: Record<string, string | undefined>;
}

function resolveKind(flags: ModeFlags, isTTY: boolean): OutputKind {
  switch (flags.format) {
    case "json":
      return "json";
    case "json-pretty":
      return "json-pretty";
    case "human":
      return "rich";
    case "compact":
      return "compact";
  }
  if (flags.json) return "json";
  return isTTY ? "rich" : "json";
}

function resolveColor(
  flags: ModeFlags,
  kind: OutputKind,
  isTTY: boolean,
  env: Record<string, string | undefined>,
): boolean {
  if (kind === "json" || kind === "json-pretty") return false;
  if (flags.color === "always") return true;
  if (flags.color === "never") return false;
  if (env.NO_COLOR != null) return false;
  if (env.FORCE_COLOR != null) return true;
  return isTTY;
}

/** Unicode symbols unless `HIBI_ASCII=1` or a non-UTF locale is set. */
function resolveUnicode(env: Record<string, string | undefined>): boolean {
  if (env.HIBI_ASCII === "1") return false;
  const locale = env.LC_ALL || env.LC_CTYPE || env.LANG || "";
  if (locale && !/UTF-?8/i.test(locale)) return false;
  return true;
}

function resolveHints(
  flags: ModeFlags,
  env: Record<string, string | undefined>,
): boolean {
  if (flags.noHints) return false;
  if (env.HIBI_ADVICE === "0") return false;
  return true;
}

export function resolveMode(flags: ModeFlags, ctx: ModeEnv = {}): OutputMode {
  const isTTY = Boolean(ctx.isTTY);
  const env = ctx.env ?? {};
  const kind = resolveKind(flags, isTTY);
  return {
    kind,
    color: resolveColor(flags, kind, isTTY, env),
    unicode: resolveUnicode(env),
    explain: Boolean(flags.explain),
    hints: resolveHints(flags, env),
  };
}
