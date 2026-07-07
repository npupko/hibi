/**
 * The built-in `command` verifier runner (§17.6, D13) — the in-tree runner that
 * makes `supported`/`refuted` reachable in a stock install.
 *
 * It is a runner resolver, not an anchor resolver: `kinds: []` (it never grades a
 * span), `advisory: false` (a passing/failing verifier may gate an enforced
 * claim), `verifierKinds: ["command"]`. `verify()` runs the verifier's `ref` as a
 * child-process shell command — exit `0` → `supported`, non-zero → `refuted`, a
 * timeout or spawn failure → `null` (no result; the deterministic baseline holds).
 *
 * Security (normative, §17.6/§7): verifiers execute repo-committed commands, so
 * they run ONLY under `check --run-verifiers` — the registry never dispatches a
 * verifier otherwise. Nothing here is ever eval'd in-process; every command runs
 * out-of-process, exactly like an external resolver.
 */

import type { ResolveFiles } from "../../algo/resolve.ts";
import type { Assertion, Proposition, Verifier } from "../../core/model.ts";
import type { VerifyResult } from "../protocol.ts";
import type { Resolver } from "../registry.ts";

/** Default per-verifier timeout (§17.6): a real test suite needs far more than the 5s RPC timeout. */
export const DEFAULT_VERIFIER_TIMEOUT_MS = 120_000;

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

  /** Not an anchor resolver — it grades no span, so `resolve` yields no verdict. */
  async resolve(
    _assertion: Assertion,
    _files: ResolveFiles,
    _proposition?: Proposition,
  ) {
    return {};
  }

  /**
   * Run one `command` verifier out-of-process. `ref` is a shell string, so it is
   * dispatched via `sh -c` (POSIX; Windows support is out of scope for now), with
   * the anchor root as the working directory so relative test paths resolve.
   */
  async verify(
    _assertion: Assertion,
    verifier: Verifier,
    _files: ResolveFiles,
  ): Promise<VerifyResult | null> {
    if (verifier.kind !== "command") return null;
    let timedOut = false;
    try {
      const proc = Bun.spawn(["sh", "-c", verifier.ref], {
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
      // A timeout is "no result" — the deterministic baseline is kept, never a
      // false `refuted` from a killed process.
      if (timedOut) return null;
      return {
        behavior: code === 0 ? "supported" : "refuted",
        advisories: [],
        notes: [
          code === 0
            ? `command exited 0: ${verifier.ref}`
            : `command exited ${code}: ${verifier.ref}`,
        ],
      };
    } catch {
      // Spawn failure → no result.
      return null;
    }
  }
}
