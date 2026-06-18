/**
 * Engine-side resolver client (§7.1): spawns an out-of-process resolver and talks
 * JSONL-RPC over its stdio. A slow or crashing resolver is timed-out and cannot
 * corrupt the determinism-critical engine — every failure degrades to `null`,
 * never throws onto the verdict path.
 */
import { type ChildProcessByStdio, spawn } from "node:child_process";
import type { Readable, Writable } from "node:stream";
import {
  DescribeResult,
  encodeLine,
  LineFramer,
  type ResolveParams,
  ResolveResult,
} from "./protocol.ts";

export interface ResolverProcessSpec {
  name: string;
  command: string;
  args?: string[];
  timeoutMs?: number;
  cwd?: string;
}

export class OutOfProcessResolver {
  private child?: ChildProcessByStdio<Writable, Readable, null>;
  private framer = new LineFramer();
  private pending = new Map<
    number,
    { resolve: (v: unknown) => void; timer: ReturnType<typeof setTimeout> }
  >();
  private nextId = 1;
  private dead = false;
  readonly timeoutMs: number;

  constructor(private spec: ResolverProcessSpec) {
    this.timeoutMs = spec.timeoutMs ?? 5000;
  }

  private ensure(): boolean {
    if (this.dead) return false;
    if (this.child) return true;
    try {
      this.child = spawn(this.spec.command, this.spec.args ?? [], {
        cwd: this.spec.cwd,
        stdio: ["pipe", "pipe", "inherit"],
      }) as ChildProcessByStdio<Writable, Readable, null>;
    } catch {
      this.dead = true;
      return false;
    }
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (chunk: string) => {
      for (const line of this.framer.push(chunk)) {
        let msg: { id: number; result?: unknown; error?: { message: string } };
        try {
          msg = JSON.parse(line);
        } catch {
          continue;
        }
        const p = this.pending.get(msg.id);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(msg.id);
          p.resolve(msg.error ? null : msg.result);
        }
      }
    });
    const die = () => {
      this.dead = true;
      for (const [, p] of this.pending) {
        clearTimeout(p.timer);
        p.resolve(null);
      }
      this.pending.clear();
    };
    this.child.on("error", die);
    this.child.on("exit", die);
    return true;
  }

  private rpc(method: string, params?: unknown): Promise<unknown> {
    if (!this.ensure() || !this.child) return Promise.resolve(null);
    const id = this.nextId++;
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        resolve(null); // timed out — degrade, never corrupt
      }, this.timeoutMs);
      this.pending.set(id, { resolve, timer });
      try {
        this.child!.stdin.write(encodeLine({ id, method, params }));
      } catch {
        clearTimeout(timer);
        this.pending.delete(id);
        resolve(null);
      }
    });
  }

  async describe(): Promise<DescribeResult | null> {
    const raw = await this.rpc("describe");
    if (raw === null) return null;
    const parsed = DescribeResult.safeParse(raw);
    return parsed.success ? parsed.data : null;
  }

  async resolve(params: ResolveParams): Promise<ResolveResult | null> {
    const raw = await this.rpc("resolve", params);
    if (raw === null) return null;
    const parsed = ResolveResult.safeParse(raw);
    return parsed.success ? parsed.data : null;
  }

  dispose(): void {
    this.dead = true;
    try {
      this.child?.kill();
    } catch {
      /* ignore */
    }
  }
}
