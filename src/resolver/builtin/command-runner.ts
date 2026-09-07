/**
 * The built-in `command` verifier runner: runs a verifier's `ref` as a shell
 * command in the anchor root. Exit 0 → `supported`, non-zero → `refuted`, a
 * timeout or spawn failure → null (no result).
 *
 * Verifiers execute repo-committed commands, so they run only under
 * `check --run-verifiers`; the registry never dispatches one otherwise.
 */

import type { ResolveFiles } from "../../algo/resolve.ts";
import type { Assertion, Verifier } from "../../core/model.ts";
import type { VerifyResult } from "../protocol.ts";
import type { Resolver } from "../registry.ts";

export const DEFAULT_VERIFIER_TIMEOUT_MS = 120_000;

/** `sh -c` on POSIX, `cmd /c` on Windows. */
export function verifierArgv(platform: NodeJS.Platform, ref: string): string[] {
  return platform === "win32" ? ["cmd", "/c", ref] : ["sh", "-c", ref];
}

export class CommandRunnerResolver implements Resolver {
  readonly name = "builtin:command-runner";
  readonly kinds: string[] = [];
  readonly tier = 2;
  readonly advisory = false;
  readonly verifierKinds = ["command"];

  constructor(
    private readonly anchorRoot: string,
    private readonly timeoutMs: number = DEFAULT_VERIFIER_TIMEOUT_MS,
  ) {}

  async resolve(_assertion: Assertion, _files: ResolveFiles) {
    return {};
  }

  async verify(
    _assertion: Assertion,
    verifier: Verifier,
  ): Promise<VerifyResult | null> {
    if (verifier.kind !== "command") return null;
    let timedOut = false;
    try {
      const proc = Bun.spawn(verifierArgv(process.platform, verifier.ref), {
        cwd: this.anchorRoot,
        stdout: "ignore",
        stderr: "ignore",
        stdin: "ignore",
      });
      const timer = setTimeout(() => {
        timedOut = true;
        proc.kill();
      }, this.timeoutMs);
      const code = await proc.exited;
      clearTimeout(timer);
      if (timedOut) return null;
      return {
        behavior: code === 0 ? "supported" : "refuted",
        advisories: [],
        notes: [`command exited ${code}: ${verifier.ref}`],
      };
    } catch {
      return null;
    }
  }
}
