/**
 * `@npupko/hibi/resolver`: what an out-of-process resolver imports. Implement a
 * `ResolverHandler` and pass it to `serveResolver`; hibi owns the JSONL-RPC
 * framing and dispatch over stdio.
 *
 *   import { serveResolver } from "@npupko/hibi/resolver";
 *   serveResolver({
 *     describe: () => ({ name: "my-resolver", version: "1", kinds: ["my-kind"], tier: 2, advisory: false }),
 *     resolve: ({ assertion, files }) => ({ verdict: myVerdict(assertion, files) }),
 *   });
 */

export type {
  Advisory,
  Anchor,
  AnchorState,
  Assertion,
  BehaviorState,
  ChangedEvidence,
  Enforcement,
  Proposition,
  Selector,
  SelectorBundle,
  Verdict,
  VerdictEvidence,
  Verifier,
} from "../core/model.ts";
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
} from "./protocol.ts";
export { type ResolverHandler, serveResolver } from "./server.ts";
