/**
 * `supersede --from <old> --to <new>`: set `supersededBy` on the old document,
 * flip it to `superseded`, and move every live claim on the old document to
 * the new one. A claim whose move fails is reported as a miss, never dropped.
 * Relocation moves the claim to the new file (`documentId` and the doc
 * side's file). When the stored quote occurs verbatim exactly once in the new
 * file, the text-position selector is rebuilt there; otherwise the selectors
 * stay as stored, so an edited sentence reads `changed`. The sentence, the
 * quote, the hashes, evidence spans and their baselines never change, so a
 * stale claim keeps gating. The `--from` document must already be registered;
 * supersede never creates it.
 */

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

/** The shell capabilities the lifecycle ops need: file reads and moving a claim's doc side. */
export interface LifecycleDeps {
  readDoc(rel: string): Promise<string | null>;
  /** Move the claim to `doc` (reanchor with `docOnly`). */
  moveDoc(
    claimId: string,
    opts: { doc: string; dryRun?: boolean },
  ): Promise<ReanchorResult>;
}

export interface SupersedeInput {
  /** The old (superseded) document path. */
  from: string;
  /** The new (superseding) document path. */
  to: string;
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
  const oldDoc = await store.getDocument(documentIdForPath(input.from));
  if (oldDoc === undefined) {
    throw new Error(`document ${input.from} is not registered`);
  }
  const newDoc = await upsertDocument(store, input.to, dryRun);

  // Every live claim moves to the new document. Nothing is read from the
  // old file: the stored quote alone decides whether the doc position is
  // rebuilt (`repositionDoc` in reanchor.ts), so an edited sentence is never
  // re-baselined. A move that fails is a miss and stays stranded.
  const live = await liveClaimsOn(store, oldDoc.id);
  const relocated: RelocatedClaim[] = [];
  const misses: SupersedeResult["misses"] = [];
  for (const claim of live) {
    try {
      const result = await deps.moveDoc(claim.id, {
        doc: input.to,
        dryRun,
      });
      relocated.push({
        claimId: claim.id,
        doc: result.doc,
        ...(result.evidence !== undefined ? { evidence: result.evidence } : {}),
      });
    } catch (e) {
      misses.push({ claimId: claim.id, reason: (e as Error).message });
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
