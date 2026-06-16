/**
 * The resolver registry (§7). Variety is pushed down here: the engine dispatches
 * each anchor to the resolver(s) that declare its kind. The built-in code-anchor
 * drift logic is itself a resolver behind the same contract; third parties add
 * more out-of-process, in any language, gated by the default-deny manifest.
 *
 * Strict rule (§7.4, §11.1): only a non-advisory (deterministic) resolver may
 * produce a gating verdict. Advisory (Tier-3) resolvers only attach advisories.
 */
import type { Assertion, Verdict, Proposition } from "../core/model.ts";
import { resolveAssertion, type AstAnalyzer } from "../algo/resolve.ts";
import { OutOfProcessResolver } from "./client.ts";
import { loadManifest, type ResolverSpec } from "./manifest.ts";

export interface Resolver {
  name: string;
  kinds: string[];
  tier: number;
  advisory: boolean;
  resolve(
    assertion: Assertion,
    text: string | null,
    proposition?: Proposition,
  ): Promise<{ verdict?: Verdict; advisories?: import("../core/model.ts").Advisory[] }>;
}

/** The built-in code-anchor drift resolver — the deterministic fusion of §17. */
export class DriftResolver implements Resolver {
  readonly name = "builtin:drift";
  readonly kinds = ["text-quote", "text-position", "ast-node", "value", "path", "glob"];
  readonly tier = 2;
  readonly advisory = false;

  constructor(
    private ast?: AstAnalyzer,
    private now?: number,
  ) {}

  async resolve(assertion: Assertion, text: string | null, _proposition?: Proposition) {
    if (text === null) {
      return {
        verdict: {
          assertionId: assertion.id,
          propositionId: assertion.propositionId,
          documentId: assertion.documentId,
          state: "ghost" as const,
          confidence: 0,
          selectorScores: [],
          ref: assertion.ref,
          notes: [`anchored file ${assertion.anchor.file} not found`],
          advisories: [],
        },
      };
    }
    return { verdict: resolveAssertion(assertion, text, { ast: this.ast, now: this.now }) };
  }
}

/** Wraps an out-of-process resolver process as a Resolver. */
class ProcessResolver implements Resolver {
  constructor(
    public name: string,
    public kinds: string[],
    public tier: number,
    public advisory: boolean,
    private proc: OutOfProcessResolver,
  ) {}

  async resolve(assertion: Assertion, text: string | null, proposition?: Proposition) {
    const res = await this.proc.resolve({ assertion, text, proposition });
    if (!res) return {}; // timed out / crashed → degrade silently
    // A declared-advisory resolver can never gate: drop any verdict it returns.
    if (this.advisory) return { advisories: res.advisories ?? [] };
    return { verdict: res.verdict, advisories: res.advisories ?? [] };
  }

  dispose() {
    this.proc.dispose();
  }
}

export class ResolverRegistry {
  private resolvers: Resolver[] = [];
  private disposers: Array<() => void> = [];

  register(r: Resolver): void {
    this.resolvers.push(r);
  }

  /** Spawn & register every resolver allowed by the default-deny manifest. */
  async loadFromManifest(root: string): Promise<void> {
    const manifest = await loadManifest(root);
    for (const spec of manifest.resolvers) {
      const proc = new OutOfProcessResolver({ name: spec.name, command: spec.command, args: spec.args, timeoutMs: spec.timeoutMs, cwd: root });
      const desc = await proc.describe();
      if (!desc) {
        proc.dispose();
        continue; // unreachable/incompatible resolver — skip (default-deny posture)
      }
      const kinds = spec.kinds ?? desc.kinds;
      const pr = new ProcessResolver(desc.name, kinds, desc.tier, desc.advisory, proc);
      this.register(pr);
      this.disposers.push(() => pr.dispose());
    }
  }

  /** The first non-advisory resolver covering at least one of the anchor's kinds. */
  primaryFor(assertion: Assertion): Resolver | undefined {
    const anchorKinds = new Set(assertion.anchor.selectors.map((s) => s.kind));
    return this.resolvers.find(
      (r) => !r.advisory && r.kinds.some((k) => anchorKinds.has(k as never)),
    );
  }

  advisoryResolvers(): Resolver[] {
    return this.resolvers.filter((r) => r.advisory);
  }

  /** Resolve an assertion: deterministic primary verdict + non-gating advisories. */
  async resolve(assertion: Assertion, text: string | null, proposition?: Proposition): Promise<Verdict> {
    const primary = this.primaryFor(assertion);
    const base = primary
      ? (await primary.resolve(assertion, text, proposition)).verdict
      : undefined;
    const verdict: Verdict = base ?? {
      assertionId: assertion.id,
      propositionId: assertion.propositionId,
      documentId: assertion.documentId,
      state: "ghost",
      confidence: 0,
      selectorScores: [],
      ref: assertion.ref,
      notes: ["no resolver for anchor kinds"],
      advisories: [],
    };

    // Gather non-gating advisories (Tier-3 advises, never decides — §7.4).
    for (const adv of this.advisoryResolvers()) {
      const r = await adv.resolve(assertion, text, proposition);
      if (r.advisories?.length) verdict.advisories = [...(verdict.advisories ?? []), ...r.advisories];
    }
    return verdict;
  }

  dispose(): void {
    for (const d of this.disposers) d();
  }
}
