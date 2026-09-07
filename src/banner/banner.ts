/**
 * The sentinel-delimited, idempotent status banner that `check --write`
 * stamps into a suspect document so readers without hibi see the flags.
 *
 *   - sentinels carry a per-repository nonce, so a doc that merely quotes the
 *     banner format is never matched or overwritten;
 *   - markers are line-anchored and version-tagged;
 *   - all whitespace inside the banner is engine-owned.
 *
 * Re-stamping identical content is byte-for-byte stable; clearing restores the
 * exact pre-banner bytes. An END line written by an older hibi with a
 * `sha=` checksum is still recognized so the banner gets replaced once.
 */
import { extname } from "node:path";

export const BANNER_VERSION = 1;
export const DEFAULT_HEADLINE = (n: number) =>
  `STALE DOCUMENT — ${n} suspect claim(s) — re-verify before trusting.`;

/** Instruction files that get the compact single-line banner. */
export const DEFAULT_INSTRUCTION_FILES = [
  "CLAUDE.md",
  "AGENTS.md",
  ".cursorrules",
  ".github/copilot-instructions.md",
] as const;

export function isInstructionFile(
  path: string,
  globs: readonly string[] = DEFAULT_INSTRUCTION_FILES,
): boolean {
  const base = path.split("/").pop() ?? path;
  return globs.some((g) => {
    const glob = new Bun.Glob(g);
    return glob.match(path) || glob.match(base);
  });
}

export type CommentStyle = "html" | "hash" | "slash" | "none";

export interface BannerEntry {
  status: string;
  id: string;
  text: string;
}
export interface BannerPayload {
  headline?: string;
  entries: BannerEntry[];
}

export type BannerAction = "insert" | "replace" | "remove" | "noop";
export interface StampResult {
  content: string;
  action: BannerAction;
}

// ── Comment style by extension ───────────────────────────────────────────────

const HASH_EXT = new Set([
  ".py",
  ".sh",
  ".bash",
  ".zsh",
  ".yaml",
  ".yml",
  ".toml",
  ".cfg",
  ".ini",
  ".rb",
]);
const SLASH_EXT = new Set([
  ".ts",
  ".tsx",
  ".js",
  ".jsx",
  ".mjs",
  ".cjs",
  ".rs",
  ".c",
  ".h",
  ".cc",
  ".cpp",
  ".hpp",
  ".go",
  ".java",
  ".kt",
  ".swift",
  ".scala",
]);
const HTML_EXT = new Set([
  ".md",
  ".markdown",
  ".mdx",
  ".html",
  ".htm",
  ".xml",
  ".svg",
]);

export function commentStyleFor(filePath: string): CommentStyle {
  const ext = extname(filePath).toLowerCase();
  if (HTML_EXT.has(ext)) return "html";
  if (HASH_EXT.has(ext)) return "hash";
  if (SLASH_EXT.has(ext)) return "slash";
  return "none";
}

// ── Sentinels ────────────────────────────────────────────────────────────────

function beginSentinel(nonce: string): string {
  return `HIBI:BEGIN v${BANNER_VERSION} ${nonce}`;
}
function endSentinel(nonce: string): string {
  return `HIBI:END v${BANNER_VERSION} ${nonce}`;
}

function beginRe(nonce: string): RegExp {
  return new RegExp(
    `^[ \\t]*(?:#|//)?[ \\t]*HIBI:BEGIN[ \\t]+v\\d+[ \\t]+${nonce}[ \\t]*$`,
  );
}
/** The optional `sha=` group accepts END lines written before v0.6. */
function endRe(nonce: string): RegExp {
  return new RegExp(
    `^[ \\t]*(?:#|//)?[ \\t]*HIBI:END[ \\t]+v\\d+[ \\t]+${nonce}(?:[ \\t]+sha=[0-9a-f]{8})?[ \\t]*$`,
  );
}

// ── Body & block building ────────────────────────────────────────────────────

/** The body: headline plus one line per claim, sorted by id. */
export function bannerBody(payload: BannerPayload): string[] {
  const entries = [...payload.entries].sort((a, b) =>
    a.id < b.id ? -1 : a.id > b.id ? 1 : 0,
  );
  const headline = payload.headline ?? DEFAULT_HEADLINE(entries.length);
  return [headline, ...entries.map((e) => `[${e.status}] (${e.id}) ${e.text}`)];
}

function wrapCore(core: string[], style: CommentStyle): string {
  switch (style) {
    case "html":
      return ["<!--", ...core, "-->"].join("\n");
    case "hash":
      return core.map((l) => (l.length === 0 ? "#" : `# ${l}`)).join("\n");
    case "slash":
      return core.map((l) => (l.length === 0 ? "//" : `// ${l}`)).join("\n");
    case "none":
      return core.join("\n");
  }
}

function sealBody(body: string[], nonce: string, style: CommentStyle): string {
  return wrapCore([beginSentinel(nonce), ...body, endSentinel(nonce)], style);
}

export function buildBanner(
  payload: BannerPayload,
  nonce: string,
  style: CommentStyle,
): string {
  return sealBody(bannerBody(payload), nonce, style);
}

/** The compact instruction-file body: one pointer line. */
export function compactBannerBody(count: number, docPath: string): string[] {
  return [`STALE — ${count} claim(s); run \`hibi check --doc ${docPath}\``];
}

export function buildCompactBanner(
  count: number,
  docPath: string,
  nonce: string,
  style: CommentStyle,
): string {
  return sealBody(compactBannerBody(count, docPath), nonce, style);
}

// ── Locating an existing banner ──────────────────────────────────────────────

interface Located {
  /** char offset of the block start (incl. `<!--` wrapper for html). */
  blockStart: number;
  /** char offset just after the block's last content line (excl. trailing \n). */
  blockEnd: number;
}

function lineOffsets(text: string): number[] {
  const starts = [0];
  for (let i = 0; i < text.length; i++)
    if (text[i] === "\n") starts.push(i + 1);
  return starts;
}

/** Find the first valid BEGIN and the first valid END after it. */
export function locateBanner(
  text: string,
  nonce: string,
  style: CommentStyle,
): Located | null {
  const lines = text.split("\n");
  const starts = lineOffsets(text);
  const bRe = beginRe(nonce);
  const eRe = endRe(nonce);

  let beginIdx = -1;
  for (const [i, line] of lines.entries()) {
    if (bRe.test(line)) {
      beginIdx = i;
      break;
    }
  }
  if (beginIdx === -1) return null;

  let endIdx = -1;
  for (const [i, line] of lines.entries()) {
    if (i <= beginIdx) continue;
    if (eRe.test(line)) {
      endIdx = i;
      break;
    }
  }
  if (endIdx === -1) return null;

  let firstLine = beginIdx;
  let lastLine = endIdx;
  if (style === "html") {
    if (firstLine > 0 && lines[firstLine - 1]?.trim() === "<!--")
      firstLine -= 1;
    if (lastLine < lines.length - 1 && lines[lastLine + 1]?.trim() === "-->")
      lastLine += 1;
  }

  const blockStart = starts[firstLine];
  const lastStart = starts[lastLine];
  const lastText = lines[lastLine];
  if (
    blockStart === undefined ||
    lastStart === undefined ||
    lastText === undefined
  ) {
    return null;
  }
  return { blockStart, blockEnd: lastStart + lastText.length };
}

// ── Placement & splicing ─────────────────────────────────────────────────────

/** For html style, the banner goes after a leading `---` frontmatter fence. */
function placementOffset(text: string, style: CommentStyle): number {
  if (style !== "html") return 0;
  const lines = text.split("\n");
  if (lines[0]?.trim() !== "---") return 0;
  for (const [i, line] of lines.entries()) {
    if (i < 1) continue;
    if (line.trim() === "---") {
      const starts = lineOffsets(text);
      const start = starts[i];
      if (start === undefined) break;
      return start + line.length + (i + 1 < lines.length ? 1 : 0);
    }
  }
  return 0;
}

function splice(head: string, banner: string, remainder: string): string {
  let h = head.replace(/\n+$/, "");
  if (h.length > 0) h += "\n";
  const rem = remainder.replace(/^\n+/, "");
  const sep = rem.length > 0 ? "\n\n" : "\n";
  return h + banner + sep + rem;
}

// ── Public API ───────────────────────────────────────────────────────────────

export interface StampOptions {
  /** Compact instruction-file mode: the single pointer line instead of the block. */
  compact?: { count: number; docPath: string };
}

/** Insert or replace the banner so the document carries `payload`. */
export function stampBanner(
  text: string,
  filePath: string,
  payload: BannerPayload,
  nonce: string,
  opts: StampOptions = {},
): StampResult {
  const style = commentStyleFor(filePath);
  const banner = opts.compact
    ? buildCompactBanner(opts.compact.count, opts.compact.docPath, nonce, style)
    : buildBanner(payload, nonce, style);
  const existing = locateBanner(text, nonce, style);

  if (existing) {
    const current = text.slice(existing.blockStart, existing.blockEnd);
    if (current === banner) return { content: text, action: "noop" };
    const before = text.slice(0, existing.blockStart);
    const after = text.slice(existing.blockEnd);
    return { content: before + banner + after, action: "replace" };
  }

  const at = placementOffset(text, style);
  const head = text.slice(0, at);
  const remainder = text.slice(at);
  return { content: splice(head, banner, remainder), action: "insert" };
}

/** Remove the banner, restoring the pre-banner bytes. */
export function removeBanner(
  text: string,
  filePath: string,
  nonce: string,
): StampResult {
  const style = commentStyleFor(filePath);
  const existing = locateBanner(text, nonce, style);
  if (!existing) return { content: text, action: "noop" };
  const head = text.slice(0, existing.blockStart);
  const tail = text.slice(existing.blockEnd).replace(/^\n+/, "");
  return { content: head + tail, action: "remove" };
}

export function hasBanner(
  text: string,
  filePath: string,
  nonce: string,
): boolean {
  return locateBanner(text, nonce, commentStyleFor(filePath)) !== null;
}
