/**
 * `archive --doc <p> [--successor <p>]`: move an obsolete document out of the
 * read path, leave a tombstone at the original path, and set the lifecycle to
 * `archived`. Live claims left on the document are reported, never moved.
 */

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import type { Document } from "../core/model.ts";
import { exists } from "../fs.ts";
import type { ClaimStore } from "../store/store.ts";
import { documentIdForPath, newDocument } from "./record.ts";
import { isLiveClaimOn } from "./supersede.ts";

function tombstone(docPath: string, successorPath?: string): string {
  const redirect = successorPath
    ? `\nSuperseded by [\`${successorPath}\`](${successorPath}).\n`
    : "\n";
  return [
    "# Archived",
    "",
    `This document (\`${docPath}\`) has been archived and moved out of the read path.`,
    redirect,
  ].join("\n");
}

export interface ArchiveResult {
  document: Document;
  archivedTo: string | null;
  successor?: string;
  strandedClaims: string[];
  dryRun: boolean;
}

export async function archiveDocument(
  store: ClaimStore,
  docPath: string,
  successorPath?: string,
  opts: { dryRun?: boolean } = {},
): Promise<ArchiveResult> {
  const root = store.anchorRoot;
  const id = documentIdForPath(docPath);
  const doc: Document =
    (await store.getDocument(id)) ?? newDocument(id, docPath);
  const dryRun = opts.dryRun ?? false;

  const abs = join(root, docPath);
  let archivedTo: string | null = null;
  if (await exists(abs)) {
    const relDest = join("archive", docPath);
    const dest = join(root, relDest);
    const alreadyArchived =
      doc.lifecycle === "archived" && (await exists(dest));
    if (!dryRun && !alreadyArchived) {
      await mkdir(dirname(dest), { recursive: true });
      const content = await readFile(abs, "utf8");
      await writeFile(dest, content);
      await writeFile(abs, tombstone(docPath, successorPath));
    }
    archivedTo = relDest;
  }

  doc.lifecycle = "archived";
  if (!dryRun) await store.putDocument(doc);
  const strandedClaims = (await store.allAssertions())
    .filter((a) => isLiveClaimOn(a, doc.id))
    .map((a) => a.id);
  return {
    document: doc,
    archivedTo,
    successor: successorPath,
    strandedClaims,
    dryRun,
  };
}
