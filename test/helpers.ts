import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import type { Enforcement, Verifier } from "../src/core/model.ts";
import { type CodeTarget, recordClaim } from "../src/engine/record.ts";
import { ClaimStore } from "../src/store/store.ts";

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

/**
 * Record a claim against a code file already in the repo. The documented
 * sentence (`text`) is anchored in the doc file; the helper appends the
 * sentence to the doc when it is not already there so the doc side resolves
 * `unchanged`. Enforcement defaults to `enforced` for a precise code span and
 * `suggested` for a coarse or glob target; `enforcement` overrides.
 */
export async function record(
  repo: TempRepo,
  opts: {
    doc: string;
    text: string;
    file: string;
    quote?: string;
    line?: number;
    verified?: boolean;
    ttl?: string;
    coarse?: boolean;
    glob?: string;
    enforcement?: Enforcement;
    verifiers?: Verifier[];
  },
) {
  const analyzer = await getAnalyzer();

  let docContent = "";
  try {
    docContent = await repo.read(opts.doc);
  } catch {
    docContent = "";
  }
  if (!docContent.includes(opts.text)) {
    docContent = docContent
      ? `${docContent}\n${opts.text}\n`
      : `${opts.text}\n`;
    await repo.write(opts.doc, docContent);
  }

  const codeContents: Record<string, string | null> = {};
  let code: CodeTarget[];
  let coarse = false;
  if (opts.glob) {
    code = [{ file: opts.glob, coarse: true }];
    coarse = true;
  } else if (opts.coarse) {
    code = [{ file: opts.file, coarse: true }];
    coarse = true;
  } else {
    const content = await repo.read(opts.file);
    codeContents[opts.file] = content;
    const region =
      opts.line !== undefined
        ? { startLine: opts.line, endLine: opts.line }
        : { quote: opts.quote };
    code = [{ file: opts.file, region }];
  }

  const enforcement = opts.enforcement ?? (coarse ? "suggested" : "enforced");

  return recordClaim(
    repo.store,
    { docContent, codeContents },
    {
      docPath: opts.doc,
      docSpec: { quote: opts.text },
      verified: opts.verified ?? false,
      owner: "tester",
      ref: "testref",
      ttl: opts.ttl,
      code,
      enforcement,
      verifiers: opts.verifiers,
      analyzer,
    },
  );
}
