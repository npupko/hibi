/**
 * Supersession (§4, §6): a typed document edge, authored *forward* on the new
 * document, with the *reverse* edge derived by the engine. Granular:
 *   - `supersedes` (full)  → old Document lifecycle → `superseded`.
 *   - `amends` (partial)   → named Propositions flip; old lifecycle → `amended`.
 * An old document can legitimately receive both supersession and code-drift.
 */

import type { Assertion, Document, Edge } from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";
import { documentIdForPath, newDocument } from "./record.ts";

async function upsertDocument(
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

export interface SupersedeInput {
  /** The new (superseding/amending) document path. */
  newDocPath: string;
  /** The old (superseded/amended) document path. */
  oldDocPath: string;
  type: "supersedes" | "amends";
  /** Required for `amends`: the proposition ids being amended. */
  propositions?: string[];
  /** Preview only: compute edges + stranded claims without persisting (§9 `--dry-run`). */
  dryRun?: boolean;
}

export interface SupersedeResult {
  newDoc: Document;
  oldDoc: Document;
  /**
   * Live claim ids still anchored to the *old* document after this op — they are
   * stranded on a document that has left the read path and will quietly rot
   * unless relocated. Reported, never auto-fixed: the remedy is `hibi relocate`
   * (Tier-1 silent-orphan hardening).
   */
  strandedClaims: string[];
}

/**
 * The single stranding predicate (§6 silent-orphan hardening): a claim is
 * stranded on a document that has left the read path when it still names that
 * `documentId` and has not been retired. `retired` claims are withdrawn and
 * excluded — they are inert by design and need no relocation. Shared by every
 * lifecycle op and by `relocate`, so "what counts as live/stranded" is defined
 * exactly once.
 */
export function isLiveClaimOn(a: Assertion, docId: string): boolean {
  return a.documentId === docId && a.enforcement !== "retired";
}

/** Live claim ids still anchored to a document — the id-only stranding probe. */
export async function liveClaimsOnDocument(
  store: ClaimStore,
  docId: string,
): Promise<string[]> {
  return (await store.allAssertions())
    .filter((a) => isLiveClaimOn(a, docId))
    .map((a) => a.id);
}

export async function supersede(
  store: ClaimStore,
  input: SupersedeInput,
): Promise<SupersedeResult> {
  const newDoc = await upsertDocument(store, input.newDocPath, input.dryRun);
  const oldDoc = await upsertDocument(store, input.oldDocPath, input.dryRun);

  if (input.type === "supersedes") {
    const forward: Edge = {
      type: "supersedes",
      target: oldDoc.id,
      derived: false,
    };
    const reverse: Edge = {
      type: "superseded-by",
      source: newDoc.id,
      derived: true,
    };
    if (!hasEdge(newDoc, forward)) newDoc.edges.push(forward);
    if (!hasEdge(oldDoc, reverse)) oldDoc.edges.push(reverse);
    oldDoc.lifecycle = "superseded";
  } else {
    const props = input.propositions ?? [];
    if (props.length === 0)
      throw new Error("`amends` requires one or more proposition ids.");
    const forward: Edge = {
      type: "amends",
      target: oldDoc.id,
      propositions: props,
      derived: false,
    };
    const reverse: Edge = {
      type: "amended-by",
      source: newDoc.id,
      propositions: props,
      derived: true,
    };
    if (!hasEdge(newDoc, forward)) newDoc.edges.push(forward);
    if (!hasEdge(oldDoc, reverse)) oldDoc.edges.push(reverse);
    // The doc stays in the read path; only the named propositions flip.
    if (oldDoc.lifecycle === "active") oldDoc.lifecycle = "amended";
  }

  if (!input.dryRun) {
    await store.putDocument(newDoc);
    await store.putDocument(oldDoc);
  }
  const strandedClaims = await liveClaimsOnDocument(store, oldDoc.id);
  return { newDoc, oldDoc, strandedClaims };
}

export interface RetractResult {
  document: Document;
  /** Live claim ids still anchored to the retracted document (see SupersedeResult). */
  strandedClaims: string[];
}

/** Mark a document retracted — the author withdrew it (§10). */
export async function retract(
  store: ClaimStore,
  docPath: string,
  opts: { dryRun?: boolean } = {},
): Promise<RetractResult> {
  const doc = await upsertDocument(store, docPath, opts.dryRun);
  doc.lifecycle = "retracted";
  // --dry-run: report the would-retract result + stranded claims without writing.
  if (!opts.dryRun) await store.putDocument(doc);
  const strandedClaims = await liveClaimsOnDocument(store, doc.id);
  return { document: doc, strandedClaims };
}

/** Set of proposition ids amended within a document (from derived edges). */
export function amendedPropositions(doc: Document): Set<string> {
  const out = new Set<string>();
  for (const e of doc.edges)
    if (e.type === "amended-by") for (const p of e.propositions) out.add(p);
  return out;
}
