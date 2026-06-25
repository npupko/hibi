/**
 * Archival remediation (§6): for an obsolete-in-full document, move it out of the
 * read path and leave a tombstone/redirect to the successor at the original path,
 * and set the document lifecycle to `archived`. The engine owns archival (§6
 * division of labor).
 */

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import type { Document } from "../core/model.ts";
import { exists } from "../fs.ts";
import type { ClaimStore } from "../store/store.ts";
import { documentIdForPath } from "./record.ts";
import { liveClaimsOnDocument } from "./supersede.ts";

function tombstone(docPath: string, successorPath?: string): string {
  const redirect = successorPath
    ? `\nSuperseded by [\`${successorPath}\`](${successorPath}).\n`
    : "\n";
  return [
    "---",
    "hibi-status: archived",
    "---",
    "",
    "# Archived",
    "",
    `This document (\`${docPath}\`) has been **archived** and moved out of the read path.`,
    redirect,
  ].join("\n");
}

export interface ArchiveResult {
  document: Document;
  archivedTo: string | null;
  successor?: string;
  /** Live claim ids still anchored to the archived document (see SupersedeResult). */
  strandedClaims: string[];
}

export async function archiveDocument(
  store: ClaimStore,
  docPath: string,
  successorPath?: string,
  opts: { dryRun?: boolean } = {},
): Promise<ArchiveResult> {
  const root = store.anchorRoot;
  const id = documentIdForPath(docPath);
  const doc: Document = (await store.getDocument(id)) ?? {
    id,
    path: docPath,
    lifecycle: "active",
    edges: [],
  };

  const abs = join(root, docPath);
  let archivedTo: string | null = null;
  if (await exists(abs)) {
    const relDest = join("archive", docPath);
    // --dry-run: report where the file *would* move (and that the doc would flip
    // to archived) without moving it, writing the tombstone, or touching the store.
    if (!opts.dryRun) {
      const dest = join(root, relDest);
      await mkdir(dirname(dest), { recursive: true });
      const content = await readFile(abs, "utf8");
      await writeFile(dest, content);
      await writeFile(abs, tombstone(docPath, successorPath));
    }
    archivedTo = relDest;
  }

  doc.lifecycle = "archived";
  if (!opts.dryRun) await store.putDocument(doc);
  const strandedClaims = await liveClaimsOnDocument(store, doc.id);
  return {
    document: doc,
    archivedTo,
    successor: successorPath,
    strandedClaims,
  };
}
