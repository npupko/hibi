/**
 * `query --path` (§9): "what claims are anchored to / cover this file or region?"
 * — the before-edit lookup, including coarse edges for blast-radius.
 */
import type { ClaimStore } from "../store/store.ts";
import type { Assertion, Proposition } from "../core/model.ts";

export interface QueryHit {
  assertion: Assertion;
  proposition: Proposition | undefined;
  documentPath: string | undefined;
  /** True when the match is via a coarse path/glob edge (navigational). */
  coarse: boolean;
}

/** Find every claim whose anchor targets or covers `path`. */
export async function queryByPath(store: ClaimStore, path: string): Promise<QueryHit[]> {
  const assertions = await store.allAssertions();
  const props = new Map((await store.allPropositions()).map((p) => [p.id, p]));
  const docs = new Map((await store.allDocuments()).map((d) => [d.id, d.path]));
  const hits: QueryHit[] = [];

  for (const a of assertions) {
    let matched = false;
    let coarse = false;
    if (a.anchor.file === path) matched = true;
    for (const s of a.anchor.selectors) {
      if (s.kind === "path" && (s.path === path || path.startsWith(s.path))) {
        matched = true;
        coarse = true;
      }
      if (s.kind === "glob" && new Bun.Glob(s.glob).match(path)) {
        matched = true;
        coarse = true;
      }
    }
    // A precise anchor on this file is not coarse even if it also carries a path selector.
    if (a.anchor.file === path) coarse = false;
    if (matched) {
      hits.push({
        assertion: a,
        proposition: props.get(a.propositionId),
        documentPath: docs.get(a.documentId),
        coarse,
      });
    }
  }
  return hits;
}
