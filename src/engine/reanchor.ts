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

import {
  localizeTextQuote,
  positionBias,
  regionText,
} from "../algo/localize.ts";
import { textSimilarity } from "../algo/normalize.ts";
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
  newDocument,
  type RecordContents,
  type RegionSpec,
  resolveRegion,
  validateDocQuote,
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
  /**
   * Refreshed change-gate baseline (§17.6, D14/D15), computed by the shell.
   * Undefined → the existing baseline carries forward unchanged.
   */
  evidenceBaseline?: Record<string, string>;
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
  /** D15: present iff `verified` trust was downgraded (reanchored without `--ref`). */
  reanchorDowngrade?: { from: string; to: string; reason: string };
}

/** One candidate re-anchor target (§9, D24 `reanchor --suggest`). */
export interface ReanchorCandidate {
  doc: string;
  start: number;
  end: number;
  similarity: number;
  /** The region text, trimmed to 120 chars. */
  snippet: string;
}

/** The read-only `reanchor --suggest` result (D24) — never writes anything. */
export interface ReanchorSuggestResult {
  action: "reanchor-suggest";
  claimId: string;
  candidates: ReanchorCandidate[];
}

/** Minimum similarity for a candidate to be worth listing (D24). */
const SUGGEST_MIN_SIMILARITY = 0.5;
/** Never surface more than this many candidates (D24). */
const SUGGEST_MAX_CANDIDATES = 5;
/** Snippet cap for a candidate region (D24). */
const SUGGEST_SNIPPET_MAX = 120;

/**
 * Orphan recovery suggestions (§9, D24). Takes the claim's stored doc-side
 * `text-quote` (its exact string + context — NOT the proposition `textCache`) and
 * runs the existing `localizeTextQuote` cascade against the current content of
 * every registered Document. Read-only: it proposes targets; only an explicit
 * `reanchor --doc-range` (with D15's attestation rules) actually moves an anchor.
 * Files missing on disk are skipped; candidates below `SUGGEST_MIN_SIMILARITY`
 * are dropped; the rest sort by similarity desc, then document path asc, then
 * region start asc, capped at `SUGGEST_MAX_CANDIDATES`.
 */
export function suggestReanchorCandidates(
  assertion: Assertion,
  docs: { path: string; content: string }[],
): ReanchorCandidate[] {
  const sel = assertion.anchor.doc.selectors;
  const tq = sel.find(
    (s): s is Extract<typeof s, { kind: "text-quote" }> =>
      s.kind === "text-quote",
  );
  if (!tq) return [];
  const tp = sel.find(
    (s): s is Extract<typeof s, { kind: "text-position" }> =>
      s.kind === "text-position",
  );
  const bias = positionBias(tp);

  const candidates: ReanchorCandidate[] = [];
  for (const { path, content } of docs) {
    const region = localizeTextQuote(content, tq, bias);
    if (!region) continue;
    const text = regionText(content, region);
    const similarity = textSimilarity(text, tq.exact);
    if (similarity < SUGGEST_MIN_SIMILARITY) continue;
    const snippet =
      text.length > SUGGEST_SNIPPET_MAX
        ? text.slice(0, SUGGEST_SNIPPET_MAX)
        : text;
    candidates.push({
      doc: path,
      start: region.start,
      end: region.end,
      similarity,
      snippet,
    });
  }

  candidates.sort(
    (a, b) =>
      b.similarity - a.similarity ||
      (a.doc < b.doc ? -1 : a.doc > b.doc ? 1 : 0) ||
      a.start - b.start,
  );
  return candidates.slice(0, SUGGEST_MAX_CANDIDATES);
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

  // D23 — the re-anchored doc quote must still anchor reliably (relocateDoc has
  // already guaranteed docContent is non-null).
  if (contents.docContent !== null) {
    validateDocQuote(
      contents.docContent,
      doc.region,
      input.docPath ?? assertion.anchor.doc.file,
    );
  }

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

  // D15 — reanchor attestation (anti-gaming). `--ref` asserts re-verification:
  // selectors + evidence baseline refresh, authored trust is retained, the new
  // ref is recorded. WITHOUT `--ref` the claim is still re-anchored (the
  // doc:moved repair loop is legitimately evidence-free), but `verified` authored
  // trust is downgraded to `inferred` and the downgrade is recorded on the
  // assertion — the claim is findable again, but nobody re-attested it is *true*.
  // The shared-proposition caveat: downgrade only when currently `verified`.
  //
  // D25 — attestation-free exact re-anchor (pure-move repair). A byte-shift is
  // evidence-neutral: there is nothing to re-attest, so it retains `verified` and
  // records no downgrade iff BOTH (1) the re-resolved doc span's text-quote exact
  // is byte-identical to the stored exact AND resolved uniquely at similarity 1.0
  // (same sentence, new offset — not `ambiguous`), and (2) the code side
  // re-resolves against its stored baseline as exactly `unchanged` (not `moved`,
  // not `changed`). Anything fuzzier downgrades exactly as D15 shipped. The
  // exception is gameable only by *not changing anything* — not a gaming vector.
  const docTqBefore = assertion.anchor.doc.selectors.find(
    (s): s is Extract<typeof s, { kind: "text-quote" }> =>
      s.kind === "text-quote",
  );
  const preDoc = resolveSide(assertion.anchor.doc, contents.docContent);
  const beforeStates = resolveAssertion(assertion, files);
  const pureMove =
    docTqBefore !== undefined &&
    doc.confirmed === docTqBefore.exact && // byte-identical → similarity 1.0
    preDoc.region !== null &&
    preDoc.state !== "ambiguous" && // resolved to a single occurrence
    beforeStates.code === "unchanged"; // code side re-resolves unchanged

  const downgrade =
    input.ref === undefined &&
    proposition.authoredTrust === "verified" &&
    !pureMove;
  const downgradeRecord = {
    from: "verified",
    to: "inferred",
    reason: "reanchored without --ref — no re-attestation of truth",
  };

  const attrs = downgrade
    ? { ...assertion.attrs, reanchorDowngrade: downgradeRecord }
    : assertion.attrs;

  const next: Assertion = {
    ...assertion,
    documentId,
    anchor: composeAnchor(doc.bundle, codeBundles),
    ref: input.ref ?? assertion.ref,
    evidenceBaseline: input.evidenceBaseline ?? assertion.evidenceBaseline,
    attrs,
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
      await store.putDocument(newDocument(documentId, input.docPath));
    } else if (existing.lifecycle !== "active") {
      await store.putDocument({ ...existing, lifecycle: "active" });
    }
  }

  proposition.textCache = doc.confirmed;
  proposition.fingerprint = propositionFingerprint(doc.confirmed);
  // D15: withdraw the `verified` attestation when re-anchored without a `--ref`.
  if (downgrade) proposition.authoredTrust = "inferred";
  if (!input.dryRun) await store.putProposition(proposition);

  // Confirm the post-reanchor states (should settle to `unchanged`).
  const after = resolveAssertion(next, files);
  return {
    assertion: next,
    doc: after.doc,
    code: after.code,
    reanchorDowngrade: downgrade ? downgradeRecord : undefined,
  };
}
