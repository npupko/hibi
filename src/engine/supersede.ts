/**
 * `supersede --from <old> --to <new>`: author the `supersedes` edge on the new
 * document, flip the old document to `superseded`, and relocate every live
 * claim whose documented sentence appears verbatim in the new document. The
 * claims that do not carry over are reported as misses, never dropped.
 */

import { regionText } from "../algo/localize.ts";
import { resolveSide } from "../algo/resolve.ts";
import type { Assertion, Document, Edge } from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";
import type { ReanchorResult } from "./reanchor.ts";
import { documentIdForPath, newDocument } from "./record.ts";

export async function upsertDocument(
  store: ClaimStore,
  path: string,
  dryRun = false,
): Promise<Document> {
  const id = documentIdForPath(path);
  let doc = await store.getDocument(id);
  if (!doc) {
    doc = newDocument(id, path);
    if (!dryRun) await store.putDocument(doc);
  }
  return doc;
}

function hasEdge(doc: Document, edge: Edge): boolean {
  return doc.edges.some((e) => JSON.stringify(e) === JSON.stringify(edge));
}

/** A claim is live on a document when it names that document and is not retired. */
export function isLiveClaimOn(a: Assertion, docId: string): boolean {
  return a.documentId === docId && a.enforcement !== "retired";
}

async function liveClaimsOn(
  store: ClaimStore,
  docId: string,
): Promise<Assertion[]> {
  return (await store.allAssertions()).filter((a) => isLiveClaimOn(a, docId));
}

/** The shell capabilities the lifecycle ops need: file reads and reanchoring. */
export interface LifecycleDeps {
  readDoc(rel: string): Promise<string | null>;
  reanchor(
    claimId: string,
    opts: { doc: string; docQuote: string; ref?: string; dryRun?: boolean },
  ): Promise<ReanchorResult>;
}

export interface SupersedeInput {
  /** The old (superseded) document path. */
  from: string;
  /** The new (superseding) document path. */
  to: string;
  ref?: string;
  dryRun?: boolean;
}

export interface RelocatedClaim {
  claimId: string;
  doc: string;
  code: string;
}

export interface SupersedeResult {
  newDoc: Document;
  oldDoc: Document;
  relocated: RelocatedClaim[];
  misses: { claimId: string; reason: string }[];
  /** Live claim ids still on the old document after this op. */
  strandedClaims: string[];
  dryRun: boolean;
}

/** Each live claim's current sentence: the live span, else the cached text. */
async function currentTexts(
  store: ClaimStore,
  live: Assertion[],
  fromContent: string | null,
): Promise<{ claimId: string; text: string }[]> {
  return Promise.all(
    live.map(async (a) => {
      let text: string | undefined;
      if (fromContent !== null) {
        const located = resolveSide(a.anchor.doc, fromContent).region;
        if (located) text = regionText(fromContent, located);
      }
      if (text === undefined) {
        const prop = await store.getProposition(a.propositionId);
        text = prop?.textCache ?? "";
      }
      return { claimId: a.id, text };
    }),
  );
}

export async function supersede(
  store: ClaimStore,
  deps: LifecycleDeps,
  input: SupersedeInput,
): Promise<SupersedeResult> {
  if (input.from === input.to) {
    throw new Error("supersede --from and --to must differ.");
  }
  const toContent = await deps.readDoc(input.to);
  if (toContent === null) {
    throw new Error(`Document not found on disk: ${input.to}`);
  }
  const dryRun = input.dryRun ?? false;
  const newDoc = await upsertDocument(store, input.to, dryRun);
  const oldDoc = await upsertDocument(store, input.from, dryRun);

  // Relocate first, so the stranded report reflects what is left.
  const fromContent = await deps.readDoc(input.from);
  const live = await liveClaimsOn(store, oldDoc.id);
  const texts = await currentTexts(store, live, fromContent);
  const relocated: RelocatedClaim[] = [];
  const misses: SupersedeResult["misses"] = [];
  for (const { claimId, text } of texts) {
    if (text.length === 0 || !toContent.includes(text)) {
      misses.push({
        claimId,
        reason: `documented sentence not found in ${input.to}; reanchor with an explicit span or retire`,
      });
      continue;
    }
    try {
      const result = await deps.reanchor(claimId, {
        doc: input.to,
        docQuote: text,
        ref: input.ref,
        dryRun,
      });
      relocated.push({ claimId, doc: result.doc, code: result.code });
    } catch (e) {
      misses.push({ claimId, reason: (e as Error).message });
    }
  }

  const forward: Edge = { type: "supersedes", target: oldDoc.id };
  if (!hasEdge(newDoc, forward)) newDoc.edges.push(forward);
  oldDoc.lifecycle = "superseded";
  if (!dryRun) {
    await store.putDocument(newDoc);
    await store.putDocument(oldDoc);
  }

  const strandedClaims = dryRun
    ? misses.map((m) => m.claimId)
    : (await liveClaimsOn(store, oldDoc.id)).map((a) => a.id);
  return { newDoc, oldDoc, relocated, misses, strandedClaims, dryRun };
}
