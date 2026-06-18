/**
 * Hibi — TypeScript resolver SDK (§7.1, §12).
 *
 * A thin SDK for authoring an out-of-process resolver in TypeScript. Implement a
 * `ResolverHandler` and pass it to `serveResolver` — the SDK owns the JSONL-RPC
 * framing and dispatch over stdio.
 *
 * Example:
 *   import { serveResolver } from "hibi/sdk/ts";
 *   serveResolver({
 *     describe: () => ({ name: "my-resolver", version: "1", kinds: ["scip-symbol"], tier: 2, advisory: false }),
 *     resolve: ({ assertion, text }) => ({ verdict: myVerdict(assertion, text) }),
 *   });
 */
export { serveResolver, type ResolverHandler } from "../../src/resolver/server.ts";
export {
  encodeLine,
  LineFramer,
  PROTOCOL_VERSION,
  type DescribeResult,
  type ResolveParams,
  type ResolveResult,
  type RpcRequest,
  type RpcResponse,
} from "../../src/resolver/protocol.ts";
export type { Assertion, Verdict, Advisory, Anchor, Selector, Proposition } from "../../src/core/model.ts";
