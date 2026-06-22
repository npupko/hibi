/**
 * Resolve the output mode once, from flags + TTY + environment (§9 human/machine
 * split). The contract: machines always pipe (non-TTY) and get compact JSON
 * byte-for-byte identical to the historical default; only an interactive human
 * on a TTY (or one who asks with `--pretty`) sees the rich rendering.
 *
 * Precedence is explicit flag > env (`NO_COLOR`/`FORCE_COLOR`) > TTY, applied
 * independently to the view kind, color, and unicode axes.
 */

export type OutputKind = "json" | "json-pretty" | "rich" | "compact";

export interface OutputMode {
  kind: OutputKind;
  color: boolean;
  unicode: boolean;
  /** Include the bulky `evidence`/`advisories`/`fingerprint` tail (§9 `--explain`). */
  explain: boolean;
  /** Emit the `remediation` menu; off via `--no-hints` / `HIBI_ADVICE=0` (§9). */
  hints: boolean;
}

export interface ModeFlags {
  json?: boolean;
  pretty?: boolean;
  compact?: boolean;
  /** `auto` | `always` | `never` (anything else is treated as `auto`). */
  color?: string;
  simple?: boolean;
  /** `--explain` / `--detailed`: add the full evidence tail to the JSON. */
  explain?: boolean;
  /** `--no-hints`: drop the remediation menu (also via `HIBI_ADVICE=0`). */
  noHints?: boolean;
}

export interface ModeEnv {
  isTTY?: boolean;
  env?: Record<string, string | undefined>;
}

/**
 * View kind from the flag vocabulary:
 *   `--json --pretty` → indented JSON (the *old* `--pretty`)
 *   `--json`          → compact JSON (machines)
 *   `--pretty`        → rich human view, even when piped
 *   `--compact`       → one-line-per-claim human view
 *   default           → rich on a TTY, else compact JSON
 */
function resolveKind(flags: ModeFlags, isTTY: boolean): OutputKind {
  if (flags.json) return flags.pretty ? "json-pretty" : "json";
  if (flags.pretty) return "rich";
  if (flags.compact) return "compact";
  return isTTY ? "rich" : "json";
}

/** Color is meaningless for JSON; for human views: flag > NO_COLOR/FORCE_COLOR > TTY. */
function resolveColor(
  flags: ModeFlags,
  kind: OutputKind,
  isTTY: boolean,
  env: Record<string, string | undefined>,
): boolean {
  if (kind === "json" || kind === "json-pretty") return false;
  if (flags.color === "always") return true;
  if (flags.color === "never") return false;
  // `auto` / unset → environment, then TTY. NO_COLOR wins over FORCE_COLOR.
  if (env.NO_COLOR != null) return false;
  if (env.FORCE_COLOR != null) return true;
  return isTTY;
}

/** Unicode symbols unless `--simple` or a non-UTF locale is explicitly set. */
function resolveUnicode(
  flags: ModeFlags,
  env: Record<string, string | undefined>,
): boolean {
  if (flags.simple) return false;
  const locale = env.LC_ALL || env.LC_CTYPE || env.LANG || "";
  if (locale && !/UTF-?8/i.test(locale)) return false;
  return true;
}

/** Remediation hints on unless `--no-hints` or `HIBI_ADVICE=0` (git advice.* precedent). */
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
    unicode: resolveUnicode(flags, env),
    explain: Boolean(flags.explain),
    hints: resolveHints(flags, env),
  };
}
