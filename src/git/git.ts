/**
 * Advisory git access (§6, D8). git is used ONLY for advisory work — scoping the
 * write-time loop (`diff --name-only`) and attribution (blame) — never to compute
 * a verdict. `check` is fully offline and correct under shallow CI clones.
 */
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const exec = promisify(execFile);

async function git(args: string[], cwd: string): Promise<string | null> {
  try {
    const { stdout } = await exec("git", args, {
      cwd,
      maxBuffer: 64 * 1024 * 1024,
    });
    return stdout;
  } catch {
    return null; // advisory — never throw onto the verdict path
  }
}

/** The repo root, or the given dir when not inside a git work tree. */
export async function repoRoot(cwd: string): Promise<string> {
  const out = await git(["rev-parse", "--show-toplevel"], cwd);
  return out ? out.trim() : cwd;
}

/** The current HEAD ref, or "WORKTREE" outside a repo (advisory attribution). */
export async function currentRef(cwd: string): Promise<string> {
  const out = await git(["rev-parse", "HEAD"], cwd);
  return out ? out.trim() : "WORKTREE";
}

/**
 * Files changed between `ref` and the working tree (HEAD diff + unstaged +
 * untracked). Scopes the write-time loop (§6); purely advisory.
 */
export async function changedFiles(
  ref: string,
  cwd: string,
): Promise<string[]> {
  const set = new Set<string>();
  for (const args of [
    ["diff", "--name-only", ref],
    ["diff", "--name-only", "--cached"],
    ["ls-files", "--others", "--exclude-standard"],
  ]) {
    const out = await git(args, cwd);
    if (out)
      for (const line of out.split("\n")) if (line.trim()) set.add(line.trim());
  }
  return [...set];
}

/** Blame attribution for a line (advisory only). */
export async function blameAuthor(
  file: string,
  line: number,
  cwd: string,
): Promise<string | null> {
  const out = await git(
    ["blame", "-L", `${line},${line}`, "--porcelain", file],
    cwd,
  );
  if (!out) return null;
  const m = out.match(/^author (.+)$/m);
  const author = m?.[1];
  return author !== undefined ? author.trim() : null;
}
