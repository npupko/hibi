/**
 * The resolver registry (§7). Variety is pushed down here: the engine dispatches
 * each anchor to the resolver(s) that declare its kind. The built-in two-axis
 * anchor-resolution logic is itself a resolver behind the same contract; third
 * parties add more out-of-process, in any language, gated by the default-deny
 * manifest.
 *
 * Strict rule (§7.4, §11.1; ADR-001 gating invariant): only a non-advisory
 * (deterministic) resolver may produce a gating verdict. Advisory (Tier-3)
 * resolvers only attach advisories. Behavioral evidence is upgraded by a
 * separate verifier dispatch (§17.6), guarded doc-first so a verifier never
 * certifies a claim whose documented sentence is in flux (§18-B).
 */

import {
  type AstAnalyzer,
  type ResolveFiles,
  resolveAssertion,
} from "../algo/resolve.ts";
import { computeGates } from "../core/gating.ts";
import type {
  Advisory,
  Assertion,
  BehaviorState,
  Proposition,
  Verdict,
  Verifier,
} from "../core/model.ts";
import { remediationForVerdict } from "../core/remediation.ts";
import type { ClaimStore } from "../store/store.ts";
import { OutOfProcessResolver } from "./client.ts";
import { loadManifest } from "./manifest.ts";
import type { VerifyResult } from "./protocol.ts";

/** Per-call extras the engine hands a resolver alongside the anchored files. */
export interface ResolveExtra {
  /**
   * The change-gate evidence (§17.6, D14): current contents of every
   * evidence-set path. Only the built-in drift resolver consumes it; external
   * out-of-process resolvers compute their own verdict and ignore it.
   */
  evidence?: ReadonlyMap<string, string | null>;
}

export interface Resolver {
  name: string;
  kinds: string[];
  tier: number;
  advisory: boolean;
  /** Verifier kinds this resolver can run to upgrade behavioral belief (§17.6). */
  verifierKinds?: string[];
  resolve(
    assertion: Assertion,
    files: ResolveFiles,
    proposition?: Proposition,
    extra?: ResolveExtra,
  ): Promise<{ verdict?: Verdict; advisories?: Advisory[] }>;
  /** Run one verifier; null = unable to run (caller keeps the baseline). */
  verify?(
    assertion: Assertion,
    verifier: Verifier,
    files: ResolveFiles,
  ): Promise<VerifyResult | null>;
}

/**
 * The built-in anchor-resolution resolver — the deterministic two-axis fusion of
 * §17. It owns no hand-built fallback verdict: `resolveAssertion` already returns
 * `orphaned` for a missing file (§17.1), so a missing anchor side is just an
 * ordinary verdict, never a special case here.
 */
export class DriftResolver implements Resolver {
  readonly name = "builtin:drift";
  readonly kinds = [
    "text-quote",
    "text-position",
    "ast-node",
    "value",
    "inline-id",
    "path",
    "glob",
  ];
  readonly tier = 2;
  readonly advisory = false;

  constructor(
    private ast?: AstAnalyzer,
    private now?: number,
  ) {}

  async resolve(
    assertion: Assertion,
    files: ResolveFiles,
    _proposition?: Proposition,
    extra?: ResolveExtra,
  ) {
    return {
      verdict: resolveAssertion(assertion, files, {
        ast: this.ast,
        now: this.now,
        evidence: extra?.evidence,
      }),
    };
  }
}

/**
 * Wraps an out-of-process resolver process as a Resolver. Translates the engine's
 * in-memory `ResolveFiles` (a Map) to/from the JSONL wire shape (a Record) and
 * back, and forwards behavioral `verify` calls.
 */
class ProcessResolver implements Resolver {
  constructor(
    public name: string,
    public kinds: string[],
    public tier: number,
    public advisory: boolean,
    public verifierKinds: string[],
    private proc: OutOfProcessResolver,
  ) {}

  async resolve(
    assertion: Assertion,
    files: ResolveFiles,
    proposition?: Proposition,
  ) {
    const res = await this.proc.resolve({
      assertion,
      files: toWireFiles(files),
      proposition,
    });
    if (!res) return {}; // timed out / crashed → degrade silently
    // A declared-advisory resolver can never gate: drop any verdict it returns.
    if (this.advisory) return { advisories: res.advisories ?? [] };
    return { verdict: res.verdict, advisories: res.advisories ?? [] };
  }

  async verify(
    assertion: Assertion,
    verifier: Verifier,
    files: ResolveFiles,
  ): Promise<VerifyResult | null> {
    return this.proc.verify({
      assertion,
      verifier,
      files: toWireFiles(files),
      changedEvidence: [],
    });
  }

  dispose() {
    this.proc.dispose();
  }
}

/** Convert the in-memory `ResolveFiles` Map to the JSONL wire Record shape. */
function toWireFiles(files: ResolveFiles): {
  doc: string | null;
  code: Record<string, string | null>;
} {
  const code: Record<string, string | null> = {};
  for (const [path, content] of files.code) code[path] = content;
  return { doc: files.doc, code };
}

export class ResolverRegistry {
  private resolvers: Resolver[] = [];
  private disposers: Array<() => void> = [];
  private driftResolver: DriftResolver;
  /**
   * Whether to dispatch verifiers (§17.6, D13). Default **false** — verifiers
   * execute repo-committed commands, so they run only under the explicit
   * `check --run-verifiers` opt-in. `status`/`query`/`list`/`doctor`/plain
   * `check` leave this false, so no verifier process ever spawns.
   */
  runVerifiers = false;

  constructor(ast?: AstAnalyzer, now?: number) {
    this.driftResolver = new DriftResolver(ast, now);
  }

  register(r: Resolver): void {
    this.resolvers.push(r);
  }

  /**
   * Spawn & register every resolver allowed by the default-deny manifest.
   * The manifest is read from the store dir; resolver processes run with the
   * anchor root as cwd (so their paths resolve against the tracked tree, §8).
   */
  async loadFromManifest(store: ClaimStore): Promise<void> {
    const manifest = await loadManifest(store.dir);
    for (const spec of manifest.resolvers) {
      const proc = new OutOfProcessResolver({
        name: spec.name,
        command: spec.command,
        args: spec.args,
        timeoutMs: spec.timeoutMs,
        cwd: store.anchorRoot,
      });
      const desc = await proc.describe();
      if (!desc) {
        proc.dispose();
        continue; // unreachable/incompatible resolver — skip (default-deny posture)
      }
      const kinds = spec.kinds ?? desc.kinds;
      const pr = new ProcessResolver(
        desc.name,
        kinds,
        desc.tier,
        desc.advisory,
        desc.verifierKinds ?? [],
        proc,
      );
      this.register(pr);
      this.disposers.push(() => pr.dispose());
    }
  }

  /** The first non-advisory resolver covering at least one of the anchor's kinds. */
  primaryFor(assertion: Assertion): Resolver | undefined {
    const anchor = assertion.anchor;
    const anchorKinds = new Set<string>();
    for (const s of anchor.doc.selectors) anchorKinds.add(s.kind);
    for (const bundle of anchor.code) {
      for (const s of bundle.selectors) anchorKinds.add(s.kind);
    }
    return this.resolvers.find(
      (r) => !r.advisory && r.kinds.some((k) => anchorKinds.has(k)),
    );
  }

  advisoryResolvers(): Resolver[] {
    return this.resolvers.filter((r) => r.advisory);
  }

  /**
   * Resolve an assertion (§7.4): deterministic primary verdict → doc-first
   * verifier dispatch (behavioral) → non-gating advisories → recompute gates.
   */
  async resolve(
    assertion: Assertion,
    files: ResolveFiles,
    proposition?: Proposition,
    extra?: ResolveExtra,
  ): Promise<Verdict> {
    // 1 — base verdict from the primary deterministic resolver (defaults to the
    //     built-in drift resolver, which never returns a hand-built fallback).
    const primary = this.primaryFor(assertion) ?? this.driftResolver;
    const base = (await primary.resolve(assertion, files, proposition, extra))
      .verdict;
    const verdict =
      base ??
      (await this.driftResolver.resolve(assertion, files, proposition, extra))
        .verdict;
    if (!verdict) {
      // The drift resolver always returns a verdict; this is unreachable, but
      // keeps the type total without a hand-built state literal.
      throw new Error(`no verdict produced for assertion ${assertion.id}`);
    }

    // 2 — Verifier dispatch (doc-first guard §18-B): only for a behavioral claim
    //     (the deterministic verdict carries a behavior axis), and only when its
    //     documented sentence is locatable (unchanged/moved). A non-behavioral
    //     claim never gains a behavior state, so the two resolve paths agree (§10).
    if (
      this.runVerifiers &&
      assertion.verifiers.length > 0 &&
      verdict.behavior !== undefined &&
      (verdict.doc === "unchanged" || verdict.doc === "moved")
    ) {
      verdict.behavior = await this.dispatchVerifiers(
        assertion,
        files,
        verdict.behavior,
      );
    }

    // 3 — Advisory resolvers (Tier-3 advises, never decides — §7.4).
    for (const adv of this.advisoryResolvers()) {
      const r = await adv.resolve(assertion, files, proposition);
      if (r.advisories?.length) {
        verdict.advisories = [...(verdict.advisories ?? []), ...r.advisories];
      }
    }

    // 4 — Recompute gates: behavior may have moved in step 2 (§9).
    verdict.gates = computeGates(
      {
        doc: verdict.doc,
        code: verdict.code,
        behavior: verdict.behavior,
        expired: verdict.expired,
      },
      assertion.enforcement,
    );

    // 5 — Recompute the remediation menu: a verifier may have upgraded the
    //     behavior axis in step 2, changing which actions apply (§9).
    verdict.remediation = remediationForVerdict(verdict);

    return verdict;
  }

  /**
   * Run each declared verifier through a resolver that handles its kind and merge
   * the results (§17.6): any `refuted` wins; else if ≥1 ran and all `supported`,
   * `supported`; otherwise the deterministic baseline (`at-risk`/`unverified`)
   * is preserved.
   */
  private async dispatchVerifiers(
    assertion: Assertion,
    files: ResolveFiles,
    baseline: BehaviorState | undefined,
  ): Promise<BehaviorState | undefined> {
    const results: BehaviorState[] = [];
    for (const verifier of assertion.verifiers) {
      // A verifier yields refuted/supported, which can gate — so only a
      // NON-advisory resolver may run one (§7.4: an advisor never gates).
      const runner = this.resolvers.find(
        (r) =>
          !r.advisory && r.verify && r.verifierKinds?.includes(verifier.kind),
      );
      if (!runner?.verify) continue;
      const res = await runner.verify(assertion, verifier, files);
      if (res) results.push(res.behavior);
    }
    if (results.length === 0) return baseline;
    if (results.includes("refuted")) return "refuted";
    if (results.every((b) => b === "supported")) return "supported";
    return baseline;
  }

  dispose(): void {
    for (const d of this.disposers) d();
  }
}
