/**
 * One-time store upgrade from model v2 to v3. Rewrites every record file in
 * place and bumps `config.json`. Runs once, at `ClaimStore.open`.
 *
 * v2 → v3 mapping:
 *   Document:    drop `frontmatterStatus`; `pristine` moves to the store
 *                config's `pristine` globs; keep only `supersedes` edges (drop
 *                `derived`); `amended` → `active`, `retracted` → `archived`.
 *   Proposition: drop `authoredTrust` (moved to `Assertion.verified`).
 *   Assertion:   drop `behavioral`, `behaviorScope`, `evidenceBaseline`,
 *                `suppressed`; `verifiers[].proves` dropped; selectors
 *                `path`/`glob` → `coarse`, `inline-id` dropped;
 *                `attrs.reanchorDowngrade` dropped; `verified` set from the
 *                proposition's `authoredTrust === "verified"`.
 */

import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { MODEL_VERSION } from "../core/model.ts";
import { exists } from "../fs.ts";

type Rec = Record<string, unknown>;

async function readJsonDir(
  dir: string,
): Promise<{ file: string; value: Rec }[]> {
  if (!(await exists(dir))) return [];
  const out: { file: string; value: Rec }[] = [];
  for (const f of (await readdir(dir)).filter((x) => x.endsWith(".json"))) {
    const file = join(dir, f);
    out.push({ file, value: JSON.parse(await readFile(file, "utf8")) as Rec });
  }
  return out;
}

async function writeJson(file: string, value: unknown): Promise<void> {
  await writeFile(file, `${JSON.stringify(value, null, 2)}\n`);
}

function upgradeSelector(s: Rec): Rec | null {
  switch (s.kind) {
    case "path":
      return { kind: "coarse", pattern: s.path };
    case "glob":
      return { kind: "coarse", pattern: s.glob };
    case "inline-id":
      return null;
    default:
      return s;
  }
}

function upgradeBundle(b: Rec): Rec {
  const selectors = ((b.selectors as Rec[]) ?? [])
    .map(upgradeSelector)
    .filter((s): s is Rec => s !== null);
  // A bundle that held only dropped selectors would fail `selectors.min(1)` and
  // make the whole store unreadable; degrade it to a coarse file edge instead.
  if (selectors.length === 0) {
    return { file: b.file, selectors: [{ kind: "coarse", pattern: b.file }] };
  }
  return { file: b.file, selectors };
}

/** True when a v2 document carried the dropped `pristine` flag. */
export function wasPristine(d: Rec): boolean {
  return d.pristine === true;
}

export function upgradeDocument(d: Rec): Rec {
  const lifecycle =
    d.lifecycle === "amended"
      ? "active"
      : d.lifecycle === "retracted"
        ? "archived"
        : (d.lifecycle ?? "active");
  const edges = ((d.edges as Rec[]) ?? [])
    .filter((e) => e.type === "supersedes")
    .map((e) => ({ type: "supersedes", target: e.target }));
  return { id: d.id, path: d.path, lifecycle, edges };
}

export function upgradeProposition(p: Rec): Rec {
  return { id: p.id, textCache: p.textCache, fingerprint: p.fingerprint };
}

export function upgradeAssertion(a: Rec, verified: boolean): Rec {
  const anchor = (a.anchor as Rec) ?? {};
  const attrs = { ...((a.attrs as Rec) ?? {}) };
  delete attrs.reanchorDowngrade;
  const out: Rec = {
    id: a.id,
    propositionId: a.propositionId,
    documentId: a.documentId,
    owner: a.owner,
    ref: a.ref,
    anchor: {
      doc: upgradeBundle((anchor.doc as Rec) ?? {}),
      code: ((anchor.code as Rec[]) ?? []).map(upgradeBundle),
    },
    enforcement: a.enforcement ?? "suggested",
    verified,
    verifiers: ((a.verifiers as Rec[]) ?? []).map((v) => ({
      kind: v.kind,
      ref: v.ref,
    })),
  };
  if (a.ttl !== undefined) out.ttl = a.ttl;
  out.attrs = attrs;
  return out;
}

export async function upgradeV2Store(dir: string): Promise<void> {
  const props = await readJsonDir(join(dir, "propositions"));
  const verifiedByProp = new Map<string, boolean>();
  for (const { file, value } of props) {
    verifiedByProp.set(String(value.id), value.authoredTrust === "verified");
    await writeJson(file, upgradeProposition(value));
  }
  // `Document.pristine` is gone; carry it over as a `config.pristine` glob so
  // `check --write` still never stamps a banner into a doc hibi does not own.
  const pristinePaths: string[] = [];
  for (const { file, value } of await readJsonDir(join(dir, "documents"))) {
    if (wasPristine(value)) pristinePaths.push(String(value.path));
    await writeJson(file, upgradeDocument(value));
  }
  for (const { file, value } of await readJsonDir(join(dir, "claims"))) {
    const verified = verifiedByProp.get(String(value.propositionId)) ?? false;
    await writeJson(file, upgradeAssertion(value, verified));
  }
  const configPath = join(dir, "config.json");
  const config = JSON.parse(await readFile(configPath, "utf8")) as Rec;
  const pristine = [
    ...new Set([...((config.pristine as string[]) ?? []), ...pristinePaths]),
  ];
  await writeJson(configPath, {
    ...config,
    version: MODEL_VERSION,
    ...(pristine.length > 0 ? { pristine } : {}),
  });
}
