/**
 * The resolver registry. The engine dispatches each anchor to the resolver
 * that declares its kinds; the built-in drift resolver is one such resolver,
 * in-process. Third parties add more out-of-process, in any language, gated
 * by the default-deny manifest.
 *
 * Only a non-advisory resolver may produce a gating verdict. Advisory
 * resolvers only attach advisories. Verifiers run through a separate
 * dispatch, guarded doc-first so a verifier never certifies a claim whose
 * documented sentence is in flux.
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
  ChangedEvidence,
  Proposition,
  Verdict,
  Verifier,
} from "../core/model.ts";
import { remediationForVerdict } from "../core/remediation.ts";
import type { ClaimStore } from "../store/store.ts";
import { OutOfProcessResolver } from "./client.ts";
import { loadManifest } from "./manifest.ts";
import type { VerifyResult } from "./protocol.ts";

/** The selector kinds the built-in drift resolver owns. */
export const BUILTIN_KINDS = [
  "text-quote",
  "text-position",
  "ast-node",
  "value",
  "coarse",
] as const;

export interface Resolver {
  name: string;
  kinds: string[];
  tier: number;
  advisory: boolean;
  /** LLM-backed: its advisories must carry structured `provenance`. */
  modelBacked?: boolean;
  /** Verifier kinds this resolver can run. */
  verifierKinds?: string[];
  resolve(
    assertion: Assertion,
    files: ResolveFiles,
    proposition?: Proposition,
  ): Promise<{ verdict?: Verdict; advisories?: Advisory[] }>;
  /** Run one verifier; null = unable to run. */
  verify?(
    assertion: Assertion,
    verifier: Verifier,
    changedEvidence?: ChangedEvidence[],
  ): Promise<VerifyResult | null>;
}

/** The built-in, in-process anchor resolver. */
export class DriftResolver implements Resolver {
  readonly name = "builtin:drift";
  readonly kinds: string[] = [...BUILTIN_KINDS];
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
  ) {
    return {
      verdict: resolveAssertion(assertion, files, {
        ast: this.ast,
        now: this.now,
      }),
    };
  }
}

/** Wraps an out-of-process resolver as a Resolver. */
export class ProcessResolver implements Resolver {
  constructor(
    public name: string,
    public kinds: string[],
    public tier: number,
    public advisory: boolean,
    public verifierKinds: string[],
    private proc: OutOfProcessResolver,
    public modelBacked = false,
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
    if (!res) return {};
    if (this.advisory) return { advisories: res.advisories ?? [] };
    return { verdict: res.verdict, advisories: res.advisories ?? [] };
  }

  async verify(
    assertion: Assertion,
    verifier: Verifier,
    changedEvidence: ChangedEvidence[] = [],
  ): Promise<VerifyResult | null> {
    return this.proc.verify({ assertion, verifier, changedEvidence });
  }

  dispose() {
    this.proc.dispose();
  }
}

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
  private warnedResolvers = new Set<string>();
  /** Whether to dispatch verifiers. Default false: only `check --run-verifiers` sets it. */
  runVerifiers = false;

  constructor(ast?: AstAnalyzer, now?: number) {
    this.driftResolver = new DriftResolver(ast, now);
  }

  register(r: Resolver): void {
    this.resolvers.push(r);
  }

  /**
   * Spawn and register every resolver allowed by the default-deny manifest.
   * A non-advisory external resolver may not claim a built-in kind unless the
   * manifest entry sets `override: true`; such kinds are dropped with a warning.
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
        continue;
      }
      let kinds = spec.kinds ?? desc.kinds;
      if (!desc.advisory && !spec.override) {
        const claimed = kinds.filter((k) =>
          (BUILTIN_KINDS as readonly string[]).includes(k),
        );
        if (claimed.length > 0) {
          process.stderr.write(
            `resolver ${desc.name} claims built-in kind(s) ${claimed.join(", ")} without "override": true in resolvers.json; ignoring those kinds.\n`,
          );
          kinds = kinds.filter(
            (k) => !(BUILTIN_KINDS as readonly string[]).includes(k),
          );
        }
      }
      const pr = new ProcessResolver(
        desc.name,
        kinds,
        desc.tier,
        desc.advisory,
        desc.verifierKinds ?? [],
        proc,
        spec.modelBacked,
      );
      this.register(pr);
      this.disposers.push(() => pr.dispose());
    }
  }

  /** The non-advisory resolver covering an anchor kind; an external one outranks the builtin. */
  primaryFor(assertion: Assertion): Resolver | undefined {
    const anchor = assertion.anchor;
    const anchorKinds = new Set<string>();
    for (const s of anchor.doc.selectors) anchorKinds.add(s.kind);
    for (const bundle of anchor.code) {
      for (const s of bundle.selectors) anchorKinds.add(s.kind);
    }
    const matching = this.resolvers.filter(
      (r) => !r.advisory && r.kinds.some((k) => anchorKinds.has(k)),
    );
    return matching.find((r) => !(r instanceof DriftResolver)) ?? matching[0];
  }

  advisoryResolvers(): Resolver[] {
    return this.resolvers.filter((r) => r.advisory);
  }

  /** Deterministic primary verdict → verifier dispatch → advisories → recompute gates. */
  async resolve(
    assertion: Assertion,
    files: ResolveFiles,
    proposition?: Proposition,
  ): Promise<Verdict> {
    const primary = this.primaryFor(assertion) ?? this.driftResolver;
    const base = (await primary.resolve(assertion, files, proposition)).verdict;
    const verdict =
      base ??
      (await this.driftResolver.resolve(assertion, files, proposition)).verdict;
    if (!verdict) {
      throw new Error(`no verdict produced for assertion ${assertion.id}`);
    }

    if (
      this.runVerifiers &&
      assertion.verifiers.length > 0 &&
      (verdict.doc === "unchanged" || verdict.doc === "moved")
    ) {
      verdict.behavior = await this.dispatchVerifiers(
        assertion,
        verdict.evidence.changedEvidence,
      );
    }

    for (const adv of this.advisoryResolvers()) {
      const r = await adv.resolve(assertion, files, proposition);
      let advisories = r.advisories ?? [];
      if (adv.modelBacked) {
        const kept = advisories.filter((a) => a.provenance !== undefined);
        const dropped = advisories.length - kept.length;
        if (dropped > 0 && !this.warnedResolvers.has(adv.name)) {
          this.warnedResolvers.add(adv.name);
          process.stderr.write(
            `dropped ${dropped} advisories from ${adv.name}: modelBacked resolvers must attach provenance (model, promptHash, contextHash).\n`,
          );
        }
        advisories = kept;
      }
      if (advisories.length) {
        verdict.advisories = [...(verdict.advisories ?? []), ...advisories];
      }
    }

    verdict.gates = computeGates(
      {
        doc: verdict.doc,
        code: verdict.code,
        behavior: verdict.behavior,
        expired: verdict.expired,
      },
      assertion.enforcement,
    );
    verdict.remediation = remediationForVerdict(verdict);
    return verdict;
  }

  /** Any `refuted` wins; else if at least one ran and all `supported`, `supported`; else absent. */
  private async dispatchVerifiers(
    assertion: Assertion,
    changedEvidence: ChangedEvidence[] = [],
  ): Promise<BehaviorState | undefined> {
    const results: BehaviorState[] = [];
    for (const verifier of assertion.verifiers) {
      const runner = this.resolvers.find(
        (r) =>
          !r.advisory && r.verify && r.verifierKinds?.includes(verifier.kind),
      );
      if (!runner?.verify) continue;
      const res = await runner.verify(assertion, verifier, changedEvidence);
      if (res) results.push(res.behavior);
    }
    if (results.length === 0) return undefined;
    if (results.includes("refuted")) return "refuted";
    if (results.every((b) => b === "supported")) return "supported";
    return undefined;
  }

  dispose(): void {
    for (const d of this.disposers) d();
  }
}
