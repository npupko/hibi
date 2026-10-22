/**
 * `supersede --from <old> --to <new>`: set `supersededBy` on the old document,
 * flip it to `superseded`, and relocate every live
 * claim whose documented sentence appears verbatim in the new document. The
 * claims that do not carry over are reported as misses, never dropped.
 */

import { regionText } from "../algo/localize.ts";
import { resolveSide } from "../algo/resolve.ts";
import type { Claim, Document } from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";
import type { ReanchorResult } from "./reanchor.ts";
import { documentIdForPath, newDocument } from "./record.ts";

export async function upsertDocument(
  store: ClaimStore,
  path: string,
  dryRun = false,
  now: Date = new Date(),
): Promise<Document> {
  const id = documentIdForPath(path);
  let doc = await store.getDocument(id);
  if (!doc) {
    doc = newDocument(id, path, now);
    if (!dryRun) await store.putDocument(doc);
  }
  return doc;
}

/** A claim is live on a document when it names that document and is not retired. */
export function isLiveClaimOn(a: Claim, docId: string): boolean {
  return a.documentId === docId && a.enforcement !== "retired";
}

async function liveClaimsOn(
  store: ClaimStore,
  docId: string,
): Promise<Claim[]> {
  return (await store.allClaims()).filter((a) => isLiveClaimOn(a, docId));
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
  evidence?: string;
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

/** Each live claim's current sentence: the live span, else the stored sentence. */
function currentTexts(
  live: Claim[],
  fromContent: string | null,
): { claimId: string; text: string }[] {
  return live.map((a) => {
    let text: string | undefined;
    if (fromContent !== null) {
      const located = resolveSide(a.doc, fromContent).region;
      if (located) text = regionText(fromContent, located);
    }
    return { claimId: a.id, text: text ?? a.sentence };
  });
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
  const texts = currentTexts(live, fromContent);
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
      relocated.push({
        claimId,
        doc: result.doc,
        ...(result.evidence !== undefined ? { evidence: result.evidence } : {}),
      });
    } catch (e) {
      misses.push({ claimId, reason: (e as Error).message });
    }
  }

  oldDoc.lifecycle = "superseded";
  oldDoc.supersededBy = newDoc.id;
  if (!dryRun) {
    await store.putDocument(newDoc);
    await store.putDocument(oldDoc);
  }

  const strandedClaims = dryRun
    ? misses.map((m) => m.claimId)
    : (await liveClaimsOn(store, oldDoc.id)).map((a) => a.id);
  return { newDoc, oldDoc, relocated, misses, strandedClaims, dryRun };
}
