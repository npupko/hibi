/**
 * `relocate` (§9, Tier-1 silent-orphan hardening) — the batch consolidation
 * primitive. When a document is consolidated into another (split/merge/rename),
 * its live claims are stranded on the old document; `relocate` re-homes each one
 * to the new document in a single pass instead of hand-`reanchor`ing claim by
 * claim, and reports the claims it could not place so none is silently dropped.
 *
 * This module is the *pure planner*: given each stranded claim's current
 * documented text and the destination document's content, it decides which
 * claims can be re-homed verbatim and which need manual attention. A match is a
 * literal `toDocContent.includes(text)` — the same documented sentence must
 * appear verbatim in the destination, so the re-anchored quote latches onto real
 * prose, never a fuzzy near-match. The imperative shell (`Engine.relocate`) does
 * the file I/O and drives `reanchor` over the planned matches.
 */

/** One stranded claim handed to the planner: its id + its current documented text. */
export interface RelocationClaim {
  claimId: string;
  /** The claim's current documented sentence (resolved span, else textCache). */
  text: string;
}

/** A claim that can be re-homed verbatim: its id + the quote to anchor on. */
export interface RelocationMatch {
  claimId: string;
  quote: string;
}

/** A claim the planner could not place — surfaced, never silently dropped. */
export interface RelocationMiss {
  claimId: string;
  text: string;
  reason: string;
}

export interface RelocationPlan {
  matches: RelocationMatch[];
  misses: RelocationMiss[];
}

/** One re-homed claim in the shell-level relocate result: its id + post-reanchor states. */
export interface RelocatedClaim {
  claimId: string;
  doc: string;
  code: string;
}

/** The shell-level `Engine.relocate` result (the CLI envelope payload). */
export interface RelocateResult {
  from: string;
  to: string;
  relocated: RelocatedClaim[];
  misses: { claimId: string; reason: string }[];
  /** True when this was a `--dry-run` preview (no claim was re-homed). */
  dryRun: boolean;
}

/**
 * Plan a relocation: a claim matches when its documented sentence appears
 * verbatim in the destination document; everything else is a miss the caller
 * must resolve by hand (reanchor with an explicit span, or retire). An empty
 * documented sentence can never be located, so it is a miss too.
 */
export function planRelocation(
  claims: RelocationClaim[],
  toDocContent: string,
  toDoc: string,
): RelocationPlan {
  const matches: RelocationMatch[] = [];
  const misses: RelocationMiss[] = [];
  for (const claim of claims) {
    if (claim.text.length > 0 && toDocContent.includes(claim.text)) {
      matches.push({ claimId: claim.claimId, quote: claim.text });
    } else {
      misses.push({
        claimId: claim.claimId,
        text: claim.text,
        reason: `documented sentence not found in ${toDoc} — reanchor or retire by hand`,
      });
    }
  }
  return { matches, misses };
}
