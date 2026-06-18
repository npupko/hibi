/**
 * Resolver-side runtime (§7.1): the loop a resolver process runs to speak
 * JSONL-RPC over stdio. The TS SDK (sdk/ts) re-exports this; in-tree example
 * resolvers use it directly. Vendored framing/dispatch (§16).
 */
import {
  type DescribeResult,
  encodeLine,
  LineFramer,
  type ResolveParams,
  type ResolveResult,
} from "./protocol.ts";

export interface ResolverHandler {
  describe(): DescribeResult;
  resolve(params: ResolveParams): ResolveResult | Promise<ResolveResult>;
}

/** Run the resolver loop over the given streams (defaults to process stdio). */
export function serveResolver(
  handler: ResolverHandler,
  stdin: NodeJS.ReadableStream = process.stdin,
  stdout: NodeJS.WritableStream = process.stdout,
): void {
  const framer = new LineFramer();
  stdin.setEncoding?.("utf8");
  stdin.on("data", async (chunk: string) => {
    for (const line of framer.push(chunk)) {
      let req: { id: number; method: string; params?: unknown };
      try {
        req = JSON.parse(line);
      } catch {
        continue; // ignore malformed lines
      }
      try {
        if (req.method === "describe") {
          stdout.write(encodeLine({ id: req.id, result: handler.describe() }));
        } else if (req.method === "resolve") {
          const result = await handler.resolve(req.params as ResolveParams);
          stdout.write(encodeLine({ id: req.id, result }));
        } else {
          stdout.write(
            encodeLine({
              id: req.id,
              error: { message: `unknown method: ${req.method}`, code: -1 },
            }),
          );
        }
      } catch (e) {
        stdout.write(
          encodeLine({
            id: req.id,
            error: { message: String((e as Error)?.message ?? e), code: -1 },
          }),
        );
      }
    }
  });
}
