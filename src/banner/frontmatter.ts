/**
 * Optional markdown frontmatter status (§8). A vendored `---` splitter (§16) —
 * frontmatter is an *optional* machine-readable enhancement where it exists,
 * never a dependency. The engine writes a dedicated, clearly-owned key
 * (`hibi-status`) so it never clobbers an author's own `status:` field,
 * and can be removed cleanly. No YAML library is needed for a scalar key.
 */

const FM_KEY = "hibi-status";

export interface Frontmatter {
  hasFrontmatter: boolean;
  /** Inner lines between the fences (no fences). */
  inner: string[];
  /** The body after the closing fence (including its content). */
  body: string;
  /** The exact newline that followed the closing fence (for faithful rejoin). */
  closingNewline: string;
}

export function splitFrontmatter(text: string): Frontmatter {
  const lines = text.split("\n");
  if (lines[0]?.trim() !== "---") {
    return { hasFrontmatter: false, inner: [], body: text, closingNewline: "" };
  }
  for (let i = 1; i < lines.length; i++) {
    if (lines[i]!.trim() === "---") {
      const inner = lines.slice(1, i);
      const rest = lines.slice(i + 1);
      return {
        hasFrontmatter: true,
        inner,
        body: rest.join("\n"),
        closingNewline: "\n",
      };
    }
  }
  return { hasFrontmatter: false, inner: [], body: text, closingNewline: "" };
}

function rejoin(inner: string[], body: string): string {
  return ["---", ...inner, "---", body].join("\n");
}

/**
 * Set (or, with `null`, remove) the engine-owned frontmatter status key. Only
 * acts when the document already has a frontmatter fence; returns text unchanged
 * otherwise (frontmatter is never created — the banner is the universal carrier).
 */
export function setFrontmatterStatus(
  text: string,
  status: string | null,
): string {
  const fm = splitFrontmatter(text);
  if (!fm.hasFrontmatter) return text;

  const without = fm.inner.filter(
    (l) => !new RegExp(`^[ \\t]*${FM_KEY}[ \\t]*:`).test(l),
  );
  const inner =
    status === null ? without : [...without, `${FM_KEY}: ${status}`];
  return rejoin(inner, fm.body);
}

/** Read the engine-owned frontmatter status, if present. */
export function getFrontmatterStatus(text: string): string | undefined {
  const fm = splitFrontmatter(text);
  if (!fm.hasFrontmatter) return undefined;
  for (const l of fm.inner) {
    const m = new RegExp(`^[ \\t]*${FM_KEY}[ \\t]*:[ \\t]*(.+?)[ \\t]*$`).exec(
      l,
    );
    if (m) return m[1];
  }
  return undefined;
}
