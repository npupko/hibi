import { mkdtemp, mkdir, writeFile, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { ClaimStore } from "../src/store/store.ts";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import { recordClaim, resolveRegion } from "../src/engine/record.ts";
import type { AuthoredTrust } from "../src/core/model.ts";

export interface TempRepo {
  root: string;
  store: ClaimStore;
  write(rel: string, content: string): Promise<void>;
  read(rel: string): Promise<string>;
  cleanup(): Promise<void>;
}

export async function makeRepo(): Promise<TempRepo> {
  const root = await mkdtemp(join(tmpdir(), "ce-test-"));
  const store = await ClaimStore.init(root, "deadbeef");
  return {
    root,
    store,
    async write(rel, content) {
      const abs = join(root, rel);
      await mkdir(dirname(abs), { recursive: true });
      await writeFile(abs, content);
    },
    async read(rel) {
      return readFile(join(root, rel), "utf8");
    },
    async cleanup() {
      await rm(root, { recursive: true, force: true });
    },
  };
}

/** Record a precise claim against a code file already written into the repo. */
export async function record(
  repo: TempRepo,
  opts: {
    doc: string;
    text: string;
    file: string;
    quote?: string;
    line?: number;
    trust?: AuthoredTrust;
    ttl?: string;
    coarse?: boolean;
  },
) {
  const analyzer = await getAnalyzer();
  const content = opts.coarse ? "" : await repo.read(opts.file);
  const region =
    opts.coarse || !content
      ? undefined
      : resolveRegion(content, { quote: opts.quote, line: opts.line });
  return recordClaim(repo.store, content || null, {
    docPath: opts.doc,
    text: opts.text,
    authoredTrust: opts.trust ?? "inferred",
    owner: "tester",
    ref: "testref",
    ttl: opts.ttl,
    codeFile: opts.file,
    region,
    coarse: opts.coarse,
    analyzer,
  });
}
