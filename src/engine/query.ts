/**
 * `query --path` (§9): "what claims are anchored to / cover this file or region?"
 * — the before-edit lookup, including coarse edges for blast-radius.
 *
 * The anchor is **bidirectional** (§4), so a file may be hit on either side: the
 * documented sentence lives on `anchor.doc`, the described code on `anchor.code[]`.
 * Each hit reports which `side` matched and whether the match is a coarse,
 * navigational edge (path/glob) rather than a precise span.
 */

import {
  type Assertion,
  COARSE_SELECTOR_KINDS,
  type Proposition,
  type SelectorBundle,
} from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";

export interface QueryHit {
  assertion: Assertion;
  proposition: Proposition | undefined;
  documentPath: string | undefined;
  /** True when the match is via a coarse path/glob edge (navigational). */
  coarse: boolean;
  /** Which side of the bidirectional anchor matched `path`. */
  side: "doc" | "code";
}

/** True when every selector in the bundle is a coarse (path/glob) edge (§11.3). */
function isCoarseBundle(bundle: SelectorBundle): boolean {
  return bundle.selectors.every((s) =>
    (COARSE_SELECTOR_KINDS as readonly string[]).includes(s.kind),
  );
}

/**
 * Does this bundle target `path`? Either its `file` is the path, or a coarse
 * `path`/`glob` selector covers it (a path selector matches by prefix; a glob via
 * `Bun.Glob`).
 */
function bundleMatches(bundle: SelectorBundle, path: string): boolean {
  if (bundle.file === path) return true;
  for (const s of bundle.selectors) {
    if (s.kind === "path" && pathCovers(s.path, path)) return true;
    if (s.kind === "glob" && new Bun.Glob(s.glob).match(path)) return true;
  }
  return false;
}

/**
 * A coarse `path` edge covers `target` when it is the same file or a directory
 * ancestor — matched on a `/` boundary so `src` covers `src/x.ts` but not the
 * unrelated sibling `src2/x.ts`.
 */
function pathCovers(edge: string, target: string): boolean {
  if (edge === target) return true;
  const dir = edge.endsWith("/") ? edge : `${edge}/`;
  return target.startsWith(dir);
}

/** Find every claim whose anchor targets or covers `path`, on either side. */
export async function queryByPath(
  store: ClaimStore,
  path: string,
): Promise<QueryHit[]> {
  const assertions = await store.allAssertions();
  const props = new Map((await store.allPropositions()).map((p) => [p.id, p]));
  const docs = new Map((await store.allDocuments()).map((d) => [d.id, d.path]));
  const hits: QueryHit[] = [];

  for (const a of assertions) {
    const proposition = props.get(a.propositionId);
    const documentPath = docs.get(a.documentId);

    // Doc side — the documented sentence. Always a precise span (never coarse).
    if (a.anchor.doc.file === path) {
      hits.push({
        assertion: a,
        proposition,
        documentPath,
        coarse: false,
        side: "doc",
      });
    }

    // Code side — one hit per matching bundle; coarse iff the bundle is path/glob-only.
    for (const bundle of a.anchor.code) {
      if (!bundleMatches(bundle, path)) continue;
      hits.push({
        assertion: a,
        proposition,
        documentPath,
        coarse: isCoarseBundle(bundle),
        side: "code",
      });
    }
  }
  return hits;
}
