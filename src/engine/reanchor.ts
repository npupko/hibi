/**
 * `reanchor` (§9, §18-B) — re-point an existing claim at its current location.
 *
 * The documented sentence and the code it describes both move; `reanchor`
 * re-resolves **both sides** of the bidirectional anchor against the current
 * working tree and rewrites the stored baseline so a `moved`/`changed` claim
 * settles back to `unchanged`. The caller may hand in explicit replacement spans
 * (`docSpec` / `code[]`) — those override; otherwise each side is re-localized
 * through its existing selectors.
 *
 * A side that cannot be located and has no replacement span is an un-relocatable
 * orphan: re-anchoring cannot invent a target, so it throws and the author must
 * supply a new location or retire the claim (§9). On success the proposition's
 * `textCache` + `fingerprint` are refreshed from the **new confirmed doc span**
 * — the live sentence, never the stale cache (§4/§18-B).
 */

import { regionText } from "../algo/localize.ts";
import {
  type ResolveFiles,
  resolveAssertion,
  resolveSide,
} from "../algo/resolve.ts";
import { propositionFingerprint } from "../core/ids.ts";
import type {
  AnchorState,
  Assertion,
  Region,
  SelectorBundle,
} from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";
import {
  type AnchorAnalyzer,
  buildGlobBundle,
  buildPathBundle,
  buildSelectorBundle,
  composeAnchor,
} from "./anchor.ts";
import { languageForFile } from "./lang.ts";
import {
  type CodeTarget,
  documentIdForPath,
  type RecordContents,
  type RegionSpec,
  resolveRegion,
} from "./record.ts";

export interface ReanchorInput {
  claimId: string;
  /** Replacement doc-side span; omit to re-localize via the existing selectors. */
  docSpec?: RegionSpec;
  /**
   * Re-home the doc anchor to a **different file** — symmetric with the code
   * side's per-target `file`. Omit to keep the claim on its current document.
   * When set, the doc span re-resolves against this file and the assertion's
   * `documentId` moves with it: the same claim, relocated (split/merge/rename/
   * extract), never an orphan-plus-fresh-record. The caller supplies the file's
   * content as `contents.docContent`.
   */
  docPath?: string;
  /** Replacement code-side targets; omit to re-localize the existing bundles. */
  code?: CodeTarget[];
  /** New `@ref` to stamp; omit to keep the assertion's current ref. */
  ref?: string;
  analyzer?: AnchorAnalyzer;
  /**
   * Preview only: compute the would-be result (the post-reanchor per-side states)
   * without persisting any `put*` write (§9 `--dry-run`). The in-memory `next`
   * assertion is still resolved, so the preview is accurate.
   */
  dryRun?: boolean;
}

export interface ReanchorResult {
  assertion: Assertion;
  doc: AnchorState;
  code: AnchorState;
}

/** Build the ResolveFiles view (doc + code map) from the supplied contents. */
function toResolveFiles(contents: RecordContents): ResolveFiles {
  return {
    doc: contents.docContent,
    code: new Map(Object.entries(contents.codeContents)),
  };
}

/** Re-locate the doc side, preferring an explicit replacement span. */
function relocateDoc(
  assertion: Assertion,
  contents: RecordContents,
  input: ReanchorInput,
  located: Region | undefined,
): { bundle: SelectorBundle; region: Region; confirmed: string } {
  const docContent = contents.docContent;
  if (docContent === null) {
    throw new Error(
      `claim ${assertion.id} orphaned — provide a new location or retire`,
    );
  }
  // Relocating onto a different file must carry an explicit span: the existing
  // selectors describe the old file, so re-matching them against the new file's
  // content could latch onto a coincidentally similar sentence and mis-anchor.
  if (
    input.docPath !== undefined &&
    input.docPath !== assertion.anchor.doc.file &&
    !input.docSpec
  ) {
    throw new Error(
      `relocating claim ${assertion.id} to ${input.docPath} requires an explicit doc span (--doc-quote/--doc-range/--doc-line); the existing selectors describe the old file and must not be re-matched against a different one`,
    );
  }
  const region = input.docSpec
    ? resolveRegion(docContent, input.docSpec)
    : located;
  if (!region) {
    throw new Error(
      `claim ${assertion.id} orphaned — provide a new location or retire`,
    );
  }
  // Prose side: no analyzer/language; carry the existing inline-id forward.
  const inlineId = assertion.anchor.doc.selectors.find(
    (s): s is Extract<typeof s, { kind: "inline-id" }> =>
      s.kind === "inline-id",
  )?.id;
  const bundle = buildSelectorBundle(
    // Relocation (`--doc`) re-homes the bundle onto the new file; otherwise the
    // doc bundle keeps its current file.
    input.docPath ?? assertion.anchor.doc.file,
    docContent,
    region,
    {
      inlineId,
    },
  );
  return { bundle, region, confirmed: regionText(docContent, region) };
}

/** Rebuild one code bundle from an explicit replacement target. */
function bundleFromTarget(
  target: CodeTarget,
  contents: RecordContents,
  analyzer: AnchorAnalyzer | undefined,
  claimId: string,
): SelectorBundle {
  if (target.glob) return buildGlobBundle(target.glob);
  if (target.coarse || !target.region) return buildPathBundle(target.file);
  const content = contents.codeContents[target.file];
  if (content === undefined || content === null) {
    throw new Error(
      `claim ${claimId} orphaned — provide a new location or retire`,
    );
  }
  const region = resolveRegion(content, target.region);
  return buildSelectorBundle(target.file, content, region, {
    language: target.language ?? languageForFile(target.file),
    analyzer,
  });
}

/**
 * Re-localize an existing code bundle in place: coarse bundles carry forward
 * unchanged; a precise bundle is rebuilt at its current located region. A precise
 * bundle that could not be located (no region) is an un-relocatable orphan.
 */
function relocateCodeBundle(
  bundle: SelectorBundle,
  contents: RecordContents,
  located: Region | undefined,
  analyzer: AnchorAnalyzer | undefined,
  claimId: string,
): SelectorBundle {
  const isCoarse = bundle.selectors.every(
    (s) => s.kind === "path" || s.kind === "glob",
  );
  if (isCoarse) return bundle;
  const content = contents.codeContents[bundle.file];
  if (content === undefined || content === null || !located) {
    throw new Error(
      `claim ${claimId} orphaned — provide a new location or retire`,
    );
  }
  return buildSelectorBundle(bundle.file, content, located, {
    language: languageForFile(bundle.file),
    analyzer,
  });
}

/**
 * Re-resolve both sides of a claim against current content, rebuild its baseline
 * anchor, and refresh the proposition's confirmed text. Throws on an
 * un-relocatable orphan.
 */
export async function reanchor(
  store: ClaimStore,
  contents: RecordContents,
  input: ReanchorInput,
): Promise<ReanchorResult> {
  const assertion = await store.getAssertion(input.claimId);
  if (!assertion) throw new Error(`No claim ${input.claimId} in the store.`);
  const proposition = await store.getProposition(assertion.propositionId);
  if (!proposition) {
    throw new Error(
      `Proposition ${assertion.propositionId} missing for claim ${input.claimId}.`,
    );
  }

  const files = toResolveFiles(contents);

  // ── Doc side ── re-localize the doc bundle against the current document. Skip
  // this when an explicit span is given: relocateDoc resolves that span directly
  // and ignores the located region, so resolving it would be wasted work.
  const docLocated = input.docSpec
    ? undefined
    : (resolveSide(assertion.anchor.doc, contents.docContent).region ??
      undefined);
  const doc = relocateDoc(assertion, contents, input, docLocated);

  // ── Code side ── resolve each bundle INDEPENDENTLY (not via the aggregated
  // `evidence.codeRegions`, which drops null regions and so is neither
  // index-aligned with `anchor.code` nor safe for two bundles in one file).
  let codeBundles: SelectorBundle[];
  if (input.code !== undefined) {
    codeBundles = input.code.map((t) =>
      bundleFromTarget(t, contents, input.analyzer, assertion.id),
    );
  } else {
    codeBundles = assertion.anchor.code.map((bundle) => {
      const content = contents.codeContents[bundle.file] ?? null;
      const located = resolveSide(bundle, content).region ?? undefined;
      return relocateCodeBundle(
        bundle,
        contents,
        located,
        input.analyzer,
        assertion.id,
      );
    });
  }

  // ── Rewrite the baseline and the confirmed proposition text ──
  // Re-home to a new document when `--doc` relocates the doc anchor (§9). The
  // claim keeps its id, proposition, code side, owner, trust, and history; only
  // its `documentId` moves — no orphan-plus-fresh-record.
  const documentId = input.docPath
    ? documentIdForPath(input.docPath)
    : assertion.documentId;
  const next: Assertion = {
    ...assertion,
    documentId,
    anchor: composeAnchor(doc.bundle, codeBundles),
    ref: input.ref ?? assertion.ref,
  };
  // --dry-run: every `put*` below is guarded so the preview leaves the store
  // byte-identical. The post-state resolve still runs on the in-memory `next`.
  if (!input.dryRun) await store.putAssertion(next);

  // Ensure the destination Document exists and is active. A doc previously
  // retracted/superseded/archived at this path would otherwise lend its stale
  // lifecycle to the freshly-relocated live claim, so reactivate it. The source
  // Document is left intact as audit — never auto-deleted (§6).
  if (!input.dryRun && input.docPath && documentId !== assertion.documentId) {
    const existing = await store.getDocument(documentId);
    if (!existing) {
      await store.putDocument({
        id: documentId,
        path: input.docPath,
        lifecycle: "active",
        edges: [],
        pristine: false,
      });
    } else if (existing.lifecycle !== "active") {
      await store.putDocument({ ...existing, lifecycle: "active" });
    }
  }

  proposition.textCache = doc.confirmed;
  proposition.fingerprint = propositionFingerprint(doc.confirmed);
  if (!input.dryRun) await store.putProposition(proposition);

  // Confirm the post-reanchor states (should settle to `unchanged`).
  const after = resolveAssertion(next, files);
  return { assertion: next, doc: after.doc, code: after.code };
}
