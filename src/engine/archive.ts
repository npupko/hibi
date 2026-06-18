/**
 * Archival remediation (§6): for an obsolete-in-full document, move it out of the
 * read path and leave a tombstone/redirect to the successor at the original path,
 * and set the document lifecycle to `archived`. The engine owns archival (§6
 * division of labor).
 */
import { join, dirname } from "node:path";
import { mkdir, readFile, writeFile, access } from "node:fs/promises";
import type { ClaimStore } from "../store/store.ts";
import type { Document } from "../core/model.ts";
import { documentIdForPath } from "./record.ts";

async function exists(p: string): Promise<boolean> {
  try {
    await access(p);
    return true;
  } catch {
    return false;
  }
}

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
}

export async function archiveDocument(
  store: ClaimStore,
  root: string,
  docPath: string,
  successorPath?: string,
): Promise<ArchiveResult> {
  const id = documentIdForPath(docPath);
  const doc: Document = (await store.getDocument(id)) ?? { id, path: docPath, lifecycle: "active", edges: [] };

  const abs = join(root, docPath);
  let archivedTo: string | null = null;
  if (await exists(abs)) {
    const relDest = join("archive", docPath);
    const dest = join(root, relDest);
    await mkdir(dirname(dest), { recursive: true });
    const content = await readFile(abs, "utf8");
    await writeFile(dest, content);
    await writeFile(abs, tombstone(docPath, successorPath));
    archivedTo = relDest;
  }

  doc.lifecycle = "archived";
  await store.putDocument(doc);
  return { document: doc, archivedTo, successor: successorPath };
}
