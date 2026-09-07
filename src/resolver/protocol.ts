/**
 * The resolver wire protocol: JSONL-RPC over stdio. Defined once in Zod so the
 * protocol JSON Schema is generated from it. One JSON object per line in each
 * direction.
 */
import * as z from "zod";
import {
  Advisory,
  Assertion,
  BehaviorState,
  ChangedEvidence,
  Proposition,
  Verdict,
  Verifier,
} from "../core/model.ts";

export const PROTOCOL_VERSION = "1" as const;

/** describe → the resolver announces the anchor kinds it handles. */
export const DescribeResult = z.object({
  name: z.string(),
  version: z.string(),
  kinds: z.array(z.string()),
  /** Verifier kinds this resolver runs. */
  verifierKinds: z.array(z.string()).default([]),
  /** Precision tier; 3 is the advisory tier. */
  tier: z.number().int().default(1),
  /** Advisory resolvers return advisories only; they never gate. */
  advisory: z.boolean().default(false),
});
export type DescribeResult = z.infer<typeof DescribeResult>;

/** resolve params: the engine reads the files; the resolver stays pure. */
export const ResolveParams = z.object({
  assertion: Assertion,
  files: z.object({
    doc: z.string().nullable(),
    code: z.record(z.string(), z.string().nullable()),
  }),
  proposition: Proposition.optional(),
});
export type ResolveParams = z.infer<typeof ResolveParams>;

export const ResolveResult = z.object({
  verdict: Verdict.optional(),
  advisories: z.array(Advisory).default([]),
});
export type ResolveResult = z.infer<typeof ResolveResult>;

/**
 * verify params: run one verifier. A runner reads its own files; the engine
 * sends only the assertion, the verifier, and the evidence that changed.
 */
export const VerifyParams = z.object({
  assertion: Assertion,
  verifier: Verifier,
  changedEvidence: z.array(ChangedEvidence).default([]),
});
export type VerifyParams = z.infer<typeof VerifyParams>;

export const VerifyResult = z.object({
  behavior: BehaviorState,
  advisories: z.array(Advisory).default([]),
  notes: z.array(z.string()).default([]),
});
export type VerifyResult = z.infer<typeof VerifyResult>;

export const RpcRequest = z.object({
  id: z.number().int(),
  method: z.enum(["describe", "resolve", "verify"]),
  params: z.unknown().optional(),
});
export type RpcRequest = z.infer<typeof RpcRequest>;

export const RpcError = z.object({
  message: z.string(),
  code: z.number().int().default(-1),
});
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
  VerifyParams,
  VerifyResult,
  RpcRequest,
  RpcResponse,
} as const;

// ── Line framing ─────────────────────────────────────────────────────────────

export function encodeLine(msg: unknown): string {
  return `${JSON.stringify(msg)}\n`;
}

/** A streaming line splitter: feed it chunks, get back complete lines. */
export class LineFramer {
  private buf = "";
  push(chunk: string): string[] {
    this.buf += chunk;
    const lines: string[] = [];
    let nl = this.buf.indexOf("\n");
    while (nl !== -1) {
      const line = this.buf.slice(0, nl);
      this.buf = this.buf.slice(nl + 1);
      if (line.trim().length > 0) lines.push(line);
      nl = this.buf.indexOf("\n");
    }
    return lines;
  }
}
