import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import type {
  AuthoredTrust,
  Enforcement,
  Verifier,
} from "../src/core/model.ts";
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
 * Record a precise (or coarse) claim against a code file already in the repo.
 *
 * Span-first per the new model (§9/§18-B): the documented sentence (`text`) is
 * anchored in the doc file as the doc-side bundle — the helper ensures the doc
 * actually contains the sentence so the doc side resolves `unchanged`. Default
 * enforcement follows authored trust: `verified` → `enforced` (gating), else
 * `suggested` (advisory) — overridable via `enforcement`.
 */
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
    glob?: string;
    enforcement?: Enforcement;
    behavioral?: boolean;
    verifiers?: Verifier[];
  },
) {
  const analyzer = await getAnalyzer();

  // Ensure the doc carries the documented sentence so the doc side resolves.
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

  // Build the code side.
  const codeContents: Record<string, string | null> = {};
  let code: CodeTarget[];
  if (opts.glob) {
    code = [{ file: opts.glob, glob: opts.glob }];
  } else if (opts.coarse) {
    code = [{ file: opts.file, coarse: true }];
    codeContents[opts.file] = null;
  } else {
    const content = await repo.read(opts.file);
    codeContents[opts.file] = content;
    code = [
      { file: opts.file, region: { quote: opts.quote, line: opts.line } },
    ];
  }

  const trust = opts.trust ?? "inferred";
  const enforcement =
    opts.enforcement ?? (trust === "verified" ? "enforced" : "suggested");

  return recordClaim(
    repo.store,
    { docContent, codeContents },
    {
      docPath: opts.doc,
      docSpec: { quote: opts.text },
      authoredTrust: trust,
      owner: "tester",
      ref: "testref",
      ttl: opts.ttl,
      code,
      enforcement,
      behavioral: opts.behavioral,
      verifiers: opts.verifiers,
      analyzer,
    },
  );
}
