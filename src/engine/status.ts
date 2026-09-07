/**
 * The file scope of one document for `check --doc`: the document itself plus
 * every code file its claims pin, so the check costs work proportional to one
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
  for (const a of await store.allAssertions()) {
    if (a.documentId !== documentId) continue;
    files.add(a.anchor.doc.file);
    for (const bundle of a.anchor.code) files.add(bundle.file);
  }
  return { documentId, files };
}
