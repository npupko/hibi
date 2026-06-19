/**
 * Hibi — TypeScript resolver SDK (§7.1, §12).
 *
 * A thin SDK for authoring an out-of-process resolver in TypeScript. Implement a
 * `ResolverHandler` and pass it to `serveResolver` — the SDK owns the JSONL-RPC
 * framing and dispatch over stdio.
 *
 * Example:
 *   import { serveResolver } from "@npupko/hibi/sdk/ts";
 *   serveResolver({
 *     describe: () => ({ name: "my-resolver", version: "1", kinds: ["scip-symbol"], tier: 2, advisory: false }),
 *     resolve: ({ assertion, text }) => ({ verdict: myVerdict(assertion, text) }),
 *   });
 */

export type {
  Advisory,
  Anchor,
  AnchorState,
  Assertion,
  BehaviorScope,
  BehaviorState,
  ChangedEvidence,
  Enforcement,
  Proposition,
  Selector,
  SelectorBundle,
  Verdict,
  VerdictEvidence,
  Verifier,
} from "../../src/core/model.ts";
export {
  type DescribeResult,
  encodeLine,
  LineFramer,
  PROTOCOL_VERSION,
  type ResolveParams,
  type ResolveResult,
  type RpcRequest,
  type RpcResponse,
  type VerifyParams,
  type VerifyResult,
} from "../../src/resolver/protocol.ts";
export {
  type ResolverHandler,
  serveResolver,
} from "../../src/resolver/server.ts";
