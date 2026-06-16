/**
 * The resolver wire protocol (§7.1): JSONL-RPC over stdio. Defined once in Zod so
 * the protocol JSON Schema and the per-language SDKs are generated from it. The
 * line-framing and dispatch are vendored and owned (§16) — trivial and on the
 * isolation boundary.
 *
 * Framing: one JSON object per line (`\n`-delimited) in each direction.
 */
import * as z from "zod";
import { Assertion, Verdict, Advisory, Proposition } from "../core/model.ts";

export const PROTOCOL_VERSION = "1" as const;

/** describe → the resolver announces the anchor kinds it handles. */
export const DescribeResult = z.object({
  name: z.string(),
  version: z.string(),
  /** Anchor `kind`s this resolver declares (§7.2). */
  kinds: z.array(z.string()),
  /** Precision tier; 3 = quarantined advisory (advises, never gates — §7.4). */
  tier: z.number().int().default(1),
  /** Advisory resolvers return advisories only; they never gate a verdict. */
  advisory: z.boolean().default(false),
});
export type DescribeResult = z.infer<typeof DescribeResult>;

/** resolve params — the engine reads the file; the resolver stays pure/isolated. */
export const ResolveParams = z.object({
  assertion: Assertion,
  /** Current text of the anchored file, or null if it is absent. */
  text: z.string().nullable(),
  /** The proposition this assertion verifies (for semantic/behavioral advisors). */
  proposition: Proposition.optional(),
});
export type ResolveParams = z.infer<typeof ResolveParams>;

/** resolve result — a gating verdict (deterministic) and/or non-gating advisories. */
export const ResolveResult = z.object({
  verdict: Verdict.optional(),
  advisories: z.array(Advisory).default([]),
});
export type ResolveResult = z.infer<typeof ResolveResult>;

export const RpcRequest = z.object({
  id: z.number().int(),
  method: z.enum(["describe", "resolve"]),
  params: z.unknown().optional(),
});
export type RpcRequest = z.infer<typeof RpcRequest>;

export const RpcError = z.object({ message: z.string(), code: z.number().int().default(-1) });
export const RpcResponse = z.object({
  id: z.number().int(),
  result: z.unknown().optional(),
  error: RpcError.optional(),
});
export type RpcResponse = z.infer<typeof RpcResponse>;

export const PROTOCOL_SCHEMAS = {
  DescribeResult,
  ResolveParams,
  ResolveResult,
  RpcRequest,
  RpcResponse,
} as const;

// ── Vendored line-framing ────────────────────────────────────────────────────

/** Encode a message as a single JSONL line. */
export function encodeLine(msg: unknown): string {
  return JSON.stringify(msg) + "\n";
}

/**
 * A streaming line splitter: feed it chunks, get back complete lines. Holds a
 * partial trailing line until the next chunk completes it.
 */
export class LineFramer {
  private buf = "";
  push(chunk: string): string[] {
    this.buf += chunk;
    const lines: string[] = [];
    let nl: number;
    while ((nl = this.buf.indexOf("\n")) !== -1) {
      const line = this.buf.slice(0, nl);
      this.buf = this.buf.slice(nl + 1);
      if (line.trim().length > 0) lines.push(line);
    }
    return lines;
  }
}
