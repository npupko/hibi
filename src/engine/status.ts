/**
 * The file scope of one document for `check --doc`: the document itself plus
 * every evidence file its claims pin, so the check costs work proportional to one
 * document. The verdicts are then filtered to that document id.
 */

import type { ClaimStore } from "../store/store.ts";
import { documentIdForPath } from "./record.ts";

export async function documentScope(
  store: ClaimStore,
  docPath: string,
): Promise<{ documentId: string; files: Set<string> }> {
  const documentId = documentIdForPath(docPath);
  const files = new Set<string>([docPath]);
  for (const c of await store.allClaims()) {
    if (c.documentId !== documentId) continue;
    files.add(c.doc.file);
    for (const bundle of c.evidence) files.add(bundle.file);
  }
  return { documentId, files };
}
