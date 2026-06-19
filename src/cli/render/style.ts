/**
 * A hand-rolled ANSI styling helper — no runtime dependency, matching hibi's
 * vendor-tiny philosophy. Every method is a no-op when `color` is false, so the
 * caller styles unconditionally and lets the resolved mode decide.
 *
 * Color is paired with symbols/text everywhere it is used (never color-alone),
 * so a `color: false` build loses nothing but the SGR codes (accessibility, §8).
 */

/** SGR wrap: open code … reset, or the bare string when color is disabled. */
function sgr(open: number, close: number, on: boolean) {
  return (s: string): string => (on ? `\x1b[${open}m${s}\x1b[${close}m` : s);
}

export interface Style {
  bold(s: string): string;
  dim(s: string): string;
  red(s: string): string;
  yellow(s: string): string;
  green(s: string): string;
  cyan(s: string): string;
}

/** Build a Style whose methods emit ANSI only when `color` is true. */
export function makeStyle(color: boolean): Style {
  return {
    bold: sgr(1, 22, color),
    dim: sgr(2, 22, color),
    red: sgr(31, 39, color),
    yellow: sgr(33, 39, color),
    green: sgr(32, 39, color),
    cyan: sgr(36, 39, color),
  };
}

/** The display width of a string, ignoring any SGR escape sequences it carries. */
export function visibleWidth(s: string): number {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: stripping ANSI SGR codes is the point.
  return s.replace(/\x1b\[[0-9;]*m/g, "").length;
}
