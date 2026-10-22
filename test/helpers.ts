import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { getAnalyzer } from "../src/ast/analyzer.ts";
import type { Claim, Enforcement, SelectorBundle } from "../src/core/model.ts";
import { type EvidenceTarget, recordClaim } from "../src/engine/record.ts";
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
  const store = await ClaimStore.init(root);
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
    ttl?: string;
    coarse?: boolean;
    glob?: string;
    enforcement?: Enforcement;
    by?: string;
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
  let code: EvidenceTarget[];
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
    { docContent, evidenceContents: codeContents },
    {
      docPath: opts.doc,
      docSpec: { quote: opts.text },
      by: opts.by ?? "human:tester@example.com",
      owner: "human:tester@example.com",
      ref: "testref",
      ttl: opts.ttl,
      evidence: code,
      enforcement,
      analyzer,
    },
  );
}

/** A hand-built `code` claim over the given bundles, for resolver-level tests. */
export function codeClaim(
  doc: SelectorBundle,
  evidence: SelectorBundle[],
  enforcement: Enforcement = "enforced",
): Claim {
  return {
    id: "clm_0000000000000001",
    documentId: "doc_0000000000000001",
    kind: "code",
    sentence: "a sentence",
    doc,
    evidence,
    sources: [],
    procedure: null,
    enforcement,
    attestations: [],
    createdAt: "2026-10-22T00:00:00.000Z",
    ref: "r",
  };
}
