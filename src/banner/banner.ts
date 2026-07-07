/**
 * The universal, sentinel-delimited, idempotent status banner (§8, §17.5).
 *
 * Satisfies the four mandatory requirements:
 *   (1) sentinels carry a per-repository nonce (a doc that merely quotes the
 *       banner format is never matched/overwritten);
 *   (2) the END sentinel carries an FNV-1a checksum of the banner body (a
 *       hand-edit inside the banner is detected);
 *   (3) markers are line-anchored and version-tagged;
 *   (4) all whitespace inside the banner is engine-owned.
 *
 * Re-stamping identical content is byte-for-byte stable; clearing restores the
 * exact pre-banner bytes.
 */
import { extname } from "node:path";
import { fnv1a32hex } from "../vendor/fnv1a.ts";

export const BANNER_VERSION = 1;
export const DEFAULT_HEADLINE = (n: number) =>
  `STALE DOCUMENT — ${n} suspect claim(s) — re-verify before trusting.`;

/**
 * Attention-budget instruction files that get the compact single-line banner
 * (§8, D18): always-loaded agent instruction files where every extra byte
 * dilutes instruction-following. Overridable via `StoreConfig.instructionFiles`.
 */
export const DEFAULT_INSTRUCTION_FILES = [
  "CLAUDE.md",
  "AGENTS.md",
  ".cursorrules",
  ".github/copilot-instructions.md",
] as const;

/** Whether a doc path is an instruction file (compact-banner target) — §8/D18. */
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
  /** True when an existing banner's body checksum failed to verify (tamper). */
  tampered?: boolean;
}

// ── Comment style by extension (§17.5) ───────────────────────────────────────

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

// ── Sentinel construction & matching ─────────────────────────────────────────

function beginSentinel(nonce: string): string {
  return `HIBI:BEGIN v${BANNER_VERSION} ${nonce}`;
}
function endSentinel(nonce: string, sha: string): string {
  return `HIBI:END v${BANNER_VERSION} ${nonce} sha=${sha}`;
}

/** Whole-line locate regexes; the comment prefix is optional (§17.5). */
function beginRe(nonce: string): RegExp {
  return new RegExp(
    `^[ \\t]*(?:#|//)?[ \\t]*HIBI:BEGIN[ \\t]+v\\d+[ \\t]+${nonce}[ \\t]*$`,
  );
}
function endRe(nonce: string): RegExp {
  return new RegExp(
    `^[ \\t]*(?:#|//)?[ \\t]*HIBI:END[ \\t]+v\\d+[ \\t]+${nonce}[ \\t]+sha=([0-9a-f]{8})[ \\t]*$`,
  );
}

function stripCommentPrefix(line: string): string {
  return line.replace(/^[ \t]*(?:#|\/\/)[ \t]?/, "");
}

// ── Body & block building ────────────────────────────────────────────────────

/** The checksum-covered body: headline + claim lines sorted by id (§17.5). */
export function bannerBody(payload: BannerPayload): string[] {
  const entries = [...payload.entries].sort((a, b) =>
    a.id < b.id ? -1 : a.id > b.id ? 1 : 0,
  );
  const headline = payload.headline ?? DEFAULT_HEADLINE(entries.length);
  return [headline, ...entries.map((e) => `[${e.status}] (${e.id}) ${e.text}`)];
}

/** Wrap the sentinel-delimited core lines in the file's comment style (§17.5). */
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

/** Seal a body between the nonce sentinels (BEGIN + END-with-checksum) and wrap it. */
function sealBody(body: string[], nonce: string, style: CommentStyle): string {
  const sha = fnv1a32hex(body.join("\n"));
  return wrapCore(
    [beginSentinel(nonce), ...body, endSentinel(nonce, sha)],
    style,
  );
}

/** Build the banner block text (with comment wrapping) for a given style. */
export function buildBanner(
  payload: BannerPayload,
  nonce: string,
  style: CommentStyle,
): string {
  return sealBody(bannerBody(payload), nonce, style);
}

/** The compact instruction-file banner body: one pointer line (§8, D18). */
export function compactBannerBody(count: number, docPath: string): string[] {
  return [`STALE — ${count} claim(s); run \`hibi status --doc ${docPath}\``];
}

/**
 * Build the single-line compact banner for an instruction file (§8, D18): the
 * same sentinel + FNV-1a machinery as the full block, but a one-line pointer
 * body instead of the full per-claim list — always stamped top-of-file.
 */
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
  /** recorded sha on the END line. */
  sha: string;
  /** recomputed sha of the current body. */
  computedSha: string;
}

function lineOffsets(text: string): number[] {
  const starts = [0];
  for (let i = 0; i < text.length; i++)
    if (text[i] === "\n") starts.push(i + 1);
  return starts;
}

/** Find the first valid BEGIN and the first valid END after it (§17.5). */
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
  let sha = "";
  for (const [i, line] of lines.entries()) {
    if (i <= beginIdx) continue;
    const m = eRe.exec(line);
    if (m?.[1] !== undefined) {
      endIdx = i;
      sha = m[1];
      break;
    }
  }
  if (endIdx === -1) return null;

  // Body lines are between BEGIN and END; strip comment prefixes to recompute sha.
  const bodyRaw = lines.slice(beginIdx + 1, endIdx).map(stripCommentPrefix);
  const computedSha = fnv1a32hex(bodyRaw.join("\n"));

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
  const blockEnd = lastStart + lastText.length;
  return { blockStart, blockEnd, sha, computedSha };
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

/**
 * Idempotent splice (§17.5): normalize the head to end in exactly one `\n`, then
 * the banner, then `\n\n` (or a single `\n` at EOF), then the remainder with
 * leading newlines trimmed.
 */
function splice(head: string, banner: string, remainder: string): string {
  let h = head.replace(/\n+$/, "");
  if (h.length > 0) h += "\n";
  const rem = remainder.replace(/^\n+/, "");
  const sep = rem.length > 0 ? "\n\n" : "\n";
  return h + banner + sep + rem;
}

// ── Public API ───────────────────────────────────────────────────────────────

export interface StampOptions {
  /** When true, refuse to overwrite a hand-edited (tampered) banner (§17.5). */
  failOnTamper?: boolean;
  /**
   * Compact instruction-file mode (§8, D18): stamp the single-line pointer
   * banner (`STALE — N claim(s); run …`) instead of the full per-claim block.
   * `count` is the suspect-claim count; `docPath` is the pointer target.
   */
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
    const tampered = existing.sha !== existing.computedSha;
    if (tampered && opts.failOnTamper) {
      return { content: text, action: "noop", tampered: true };
    }
    const current = text.slice(existing.blockStart, existing.blockEnd);
    if (current === banner && !tampered)
      return { content: text, action: "noop" };
    // Replace the existing block in place (engine owns the region).
    const before = text.slice(0, existing.blockStart);
    const after = text.slice(existing.blockEnd);
    return {
      content: before + banner + after,
      action: "replace",
      tampered: tampered || undefined,
    };
  }

  // Insert fresh at the placement offset.
  const at = placementOffset(text, style);
  const head = text.slice(0, at);
  const remainder = text.slice(at);
  return { content: splice(head, banner, remainder), action: "insert" };
}

/** Remove the banner, restoring pristine bytes (§17.5). */
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
  // Reverse the splice: head already ends in its engine-owned newline.
  const content = head + tail;
  return { content, action: "remove" };
}

/** Whether a document currently carries a banner for this nonce. */
export function hasBanner(
  text: string,
  filePath: string,
  nonce: string,
): boolean {
  return locateBanner(text, nonce, commentStyleFor(filePath)) !== null;
}
