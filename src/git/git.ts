/**
 * Advisory git access. git is used only to scope `check --since` and to fill
 * the recorded ref, never to compute a verdict. `check` is fully offline.
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
    return null;
  }
}

/** The repo root, or the given dir when not inside a git work tree. */
export async function repoRoot(cwd: string): Promise<string> {
  const out = await git(["rev-parse", "--show-toplevel"], cwd);
  return out ? out.trim() : cwd;
}

/** The current HEAD ref, or "WORKTREE" outside a repo. */
export async function currentRef(cwd: string): Promise<string> {
  const out = await git(["rev-parse", "HEAD"], cwd);
  return out ? out.trim() : "WORKTREE";
}

/** Whether `ref` names a commit git can resolve. */
export async function refExists(ref: string, cwd: string): Promise<boolean> {
  const out = await git(
    ["rev-parse", "--verify", "--quiet", `${ref}^{commit}`],
    cwd,
  );
  return out !== null && out.trim().length > 0;
}

/**
 * Files changed between `ref` and the working tree (HEAD diff + unstaged +
 * untracked), relative to `cwd`. Throws when `ref` does not resolve.
 */
export async function changedFiles(
  ref: string,
  cwd: string,
): Promise<string[]> {
  if (!(await refExists(ref, cwd))) {
    throw new Error(`unknown git ref: ${ref}`);
  }
  const set = new Set<string>();
  for (const args of [
    ["diff", "--name-only", "--relative", ref],
    ["diff", "--name-only", "--relative", "--cached"],
    ["ls-files", "--others", "--exclude-standard"],
  ]) {
    const out = await git(args, cwd);
    if (out)
      for (const line of out.split("\n")) if (line.trim()) set.add(line.trim());
  }
  return [...set];
}
