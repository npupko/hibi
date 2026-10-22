/**
 * Identity helpers (SPEC 6). Ids are `doc_` or `clm_` followed by 16 lowercase
 * hex characters. A claim id is random; a document id is derived from the
 * document's repository-relative path, so the same path always maps to the
 * same document.
 */
import { randomUUID } from "node:crypto";

function hex16(): string {
  return randomUUID().replace(/-/g, "").slice(0, 16);
}

/** A fresh claim id: `clm_` plus 16 hex characters. */
export function newClaimId(): string {
  return `clm_${hex16()}`;
}

/** The stable document id for a repository-relative path: `doc_` plus the 16-hex xxHash64 of the path. */
export function documentIdForPath(path: string): string {
  return `doc_${Bun.hash.xxHash64(path).toString(16).padStart(16, "0")}`;
}
