/**
 * Supersession (§4, §6): a typed document edge, authored *forward* on the new
 * document, with the *reverse* edge derived by the engine. Granular:
 *   - `supersedes` (full)  → old Document lifecycle → `superseded`.
 *   - `amends` (partial)   → named Propositions flip; old lifecycle → `amended`.
 * An old document can legitimately receive both supersession and code-drift.
 */
import type { ClaimStore } from "../store/store.ts";
import type { Document, Edge } from "../core/model.ts";
import { documentIdForPath } from "./record.ts";

async function upsertDocument(store: ClaimStore, path: string): Promise<Document> {
  const id = documentIdForPath(path);
  let doc = await store.getDocument(id);
  if (!doc) {
    doc = { id, path, lifecycle: "active", edges: [] };
    await store.putDocument(doc);
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
}

export interface SupersedeResult {
  newDoc: Document;
  oldDoc: Document;
}

export async function supersede(store: ClaimStore, input: SupersedeInput): Promise<SupersedeResult> {
  const newDoc = await upsertDocument(store, input.newDocPath);
  const oldDoc = await upsertDocument(store, input.oldDocPath);

  if (input.type === "supersedes") {
    const forward: Edge = { type: "supersedes", target: oldDoc.id, derived: false };
    const reverse: Edge = { type: "superseded-by", source: newDoc.id, derived: true };
    if (!hasEdge(newDoc, forward)) newDoc.edges.push(forward);
    if (!hasEdge(oldDoc, reverse)) oldDoc.edges.push(reverse);
    oldDoc.lifecycle = "superseded";
  } else {
    const props = input.propositions ?? [];
    if (props.length === 0) throw new Error("`amends` requires one or more proposition ids.");
    const forward: Edge = { type: "amends", target: oldDoc.id, propositions: props, derived: false };
    const reverse: Edge = { type: "amended-by", source: newDoc.id, propositions: props, derived: true };
    if (!hasEdge(newDoc, forward)) newDoc.edges.push(forward);
    if (!hasEdge(oldDoc, reverse)) oldDoc.edges.push(reverse);
    // The doc stays in the read path; only the named propositions flip.
    if (oldDoc.lifecycle === "active") oldDoc.lifecycle = "amended";
  }

  await store.putDocument(newDoc);
  await store.putDocument(oldDoc);
  return { newDoc, oldDoc };
}

/** Mark a document retracted — the author withdrew it (§10). */
export async function retract(store: ClaimStore, docPath: string): Promise<Document> {
  const doc = await upsertDocument(store, docPath);
  doc.lifecycle = "retracted";
  await store.putDocument(doc);
  return doc;
}

/** Set of proposition ids amended within a document (from derived edges). */
export function amendedPropositions(doc: Document): Set<string> {
  const out = new Set<string>();
  for (const e of doc.edges) if (e.type === "amended-by") for (const p of e.propositions) out.add(p);
  return out;
}
