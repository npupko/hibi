/**
 * Recording a new code-anchored claim (§9 `record`): write a Proposition (deduped
 * by content fingerprint), an Assertion, and the composite baseline Anchor to the
 * store. Agent-authored — the engine never NLP-extracts claims (D2).
 */
import type { ClaimStore } from "../store/store.ts";
import type { Assertion, Proposition, Document, AuthoredTrust, Region } from "../core/model.ts";
import { newId, propositionFingerprint } from "../core/ids.ts";
import { buildAnchor, buildPathAnchor, type AnchorAnalyzer } from "./anchor.ts";
import { languageForFile } from "./lang.ts";

/** Stable document id derived from its repo-relative path. */
export function documentIdForPath(path: string): string {
  return `doc_${Bun.hash.xxHash64(path).toString(16).padStart(16, "0")}`;
}

export interface RecordInput {
  /** Repo-relative path of the document making the claim. */
  docPath: string;
  /** The proposition text — the timeless meaning. */
  text: string;
  authoredTrust: AuthoredTrust;
  owner: string;
  ref: string;
  ttl?: string;
  /** Repo-relative path of the anchored code file (omit for a doc-internal claim). */
  codeFile: string;
  /** The region in the code file (already resolved to char offsets). */
  region?: Region;
  /** Coarse path anchor instead of a precise one (navigational). */
  coarse?: boolean;
  analyzer?: AnchorAnalyzer;
  attrs?: Record<string, unknown>;
}

export interface RecordResult {
  document: Document;
  proposition: Proposition;
  assertion: Assertion;
  dedupedProposition: boolean;
}

export async function recordClaim(
  store: ClaimStore,
  codeContent: string | null,
  input: RecordInput,
): Promise<RecordResult> {
  // Document (upsert by path).
  const docId = documentIdForPath(input.docPath);
  let document = await store.getDocument(docId);
  if (!document) {
    document = { id: docId, path: input.docPath, lifecycle: "active", edges: [] };
    await store.putDocument(document);
  }

  // Proposition (dedup by fingerprint, the dedup unit — §5).
  const fingerprint = propositionFingerprint(input.text);
  let proposition = await store.findPropositionByFingerprint(fingerprint);
  const deduped = proposition !== undefined;
  if (!proposition) {
    proposition = {
      id: newId("prop"),
      text: input.text,
      authoredTrust: input.authoredTrust,
      fingerprint,
    };
    await store.putProposition(proposition);
  }

  // `verified` requires evidence: an anchor + @ref (§10).
  if (input.authoredTrust === "verified" && (input.coarse || !input.region || !input.ref)) {
    throw new Error("`verified` trust requires a precise anchor and a @ref.");
  }

  // Anchor.
  const anchor =
    input.coarse || !input.region
      ? buildPathAnchor(input.codeFile)
      : buildAnchor(input.codeFile, codeContent ?? "", input.region, {
          language: languageForFile(input.codeFile),
          analyzer: input.analyzer,
        });

  const assertion: Assertion = {
    id: newId("asrt"),
    propositionId: proposition.id,
    documentId: docId,
    owner: input.owner,
    ref: input.ref,
    anchor,
    ttl: input.ttl,
    attrs: input.attrs ?? {},
  };
  await store.putAssertion(assertion);

  return { document, proposition, assertion, dedupedProposition: deduped };
}

/** Resolve a region from a literal quote, char offsets, or a 1-based line number. */
export function resolveRegion(
  content: string,
  spec: { quote?: string; start?: number; end?: number; line?: number },
): Region {
  if (spec.quote !== undefined) {
    const idx = content.indexOf(spec.quote);
    if (idx === -1) throw new Error(`Quote not found in file: ${JSON.stringify(spec.quote.slice(0, 40))}…`);
    return { start: idx, end: idx + spec.quote.length };
  }
  if (spec.start !== undefined && spec.end !== undefined) {
    return { start: spec.start, end: spec.end };
  }
  if (spec.line !== undefined) {
    const lines = content.split("\n");
    let off = 0;
    for (let i = 0; i < spec.line - 1 && i < lines.length; i++) off += lines[i]!.length + 1;
    const lineText = lines[spec.line - 1] ?? "";
    return { start: off, end: off + lineText.length };
  }
  throw new Error("A region requires one of: --quote, --start/--end, or --line.");
}
