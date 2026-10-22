/**
 * The deterministic verdict→remediation mapping: the single source of the
 * "what do I do about this flag?" menu, consumed by the JSON and the renderer.
 *
 * A menu, not a prescription: hibi cannot know intent, so `recommended` is set
 * only when the next step is clear, and `actions` are ordered by preference.
 * The lookup key is `(doc, evidence?, judge?, expired)`.
 */

import type {
  AnchorState,
  EvidenceState,
  JudgeState,
  Remediation,
  RemediationAction,
  Verdict,
} from "./model.ts";

export interface RemediationInput {
  claimId: string;
  doc: AnchorState;
  evidence?: EvidenceState;
  judge?: JudgeState;
  expired: boolean;
}

const reanchorCmd = (id: string): string => `hibi reanchor ${id}`;
const retireCmd = (id: string): string => `hibi retire ${id}`;

function updateThenReanchor(id: string): RemediationAction {
  return {
    id: "update-claim",
    title: "Update the sentence, then reanchor",
    rationale:
      "the evidence changed; if the sentence is now wrong, rewrite it, then run the command",
    command: reanchorCmd(id),
  };
}

function reverifyDoc(id: string): RemediationAction {
  return {
    id: "reverify-doc",
    title: "Re-read the edited sentence against the evidence, then reanchor",
    rationale:
      "the prose changed; confirm the evidence still backs it, then run the command",
    command: reanchorCmd(id),
  };
}

function reconcile(id: string): RemediationAction {
  return {
    id: "reconcile",
    title: "Reconcile the doc and the evidence, then reanchor",
    rationale:
      "both sides changed; re-verify the current sentence against the current evidence, then run the command",
    command: reanchorCmd(id),
  };
}

function reanchorAsIs(id: string): RemediationAction {
  return {
    id: "reanchor",
    title: "Reanchor as is",
    rationale: "the sentence is still true; accept the new span",
    command: reanchorCmd(id),
  };
}

function reanchorMoved(id: string): RemediationAction {
  return {
    id: "reanchor",
    title: "Reanchor to the new position",
    rationale: "the span moved with its content intact",
    command: reanchorCmd(id),
  };
}

function reanchorTighten(id: string): RemediationAction {
  return {
    id: "reanchor",
    title: "Reanchor to a unique span",
    rationale: "the quote matches several places; pass a wider span",
    command: reanchorCmd(id),
  };
}

function reanchorSuggest(id: string): RemediationAction {
  return {
    id: "reanchor",
    title: "Find where the span went, then reanchor",
    rationale:
      "the span was not found; the command lists candidate locations, then reanchor with an explicit span",
    command: `hibi reanchor ${id} --suggest`,
  };
}

function retire(id: string): RemediationAction {
  return {
    id: "retire",
    title: "Retire the claim",
    rationale: "the claim is obsolete; withdraw it so it no longer gates",
    command: retireCmd(id),
  };
}

function supersede(): RemediationAction {
  return {
    id: "supersede",
    title: "Supersede the document",
    rationale: "a newer document replaces this one",
  };
}

function fixEvidence(): RemediationAction {
  return {
    id: "fix-evidence",
    title: "Fix the evidence to match the doc",
    rationale: "if the doc is the spec, the evidence drifted from it",
  };
}

function fixClaim(): RemediationAction {
  return {
    id: "fix-claim",
    title: "Fix the documented claim",
    rationale:
      "if the evidence is correct, the sentence is now wrong; rewrite it",
  };
}

function reverifyAndReRecord(): RemediationAction {
  return {
    id: "reverify-and-rerecord",
    title: "Re-verify and re-record",
    rationale: "the claim's ttl has passed; re-verify, then re-record it",
  };
}

/** The remediation menu for a verdict, or `null` when there is nothing to do. */
export function remediationFor(v: RemediationInput): Remediation | null {
  const id = v.claimId;
  let rem: Remediation | null = null;

  const hasOrphan = v.doc === "orphaned" || v.evidence === "orphaned";
  const docChanged = v.doc === "changed";
  const evidenceChanged = v.evidence === "changed";
  const hasAmbiguous = v.doc === "ambiguous" || v.evidence === "ambiguous";
  const hasMoved = v.doc === "moved" || v.evidence === "moved";

  if (hasOrphan) {
    rem = {
      recommended: "reanchor",
      actions: [reanchorSuggest(id), retire(id), supersede()],
    };
  } else if (v.judge === "refuted") {
    rem = { recommended: null, actions: [fixEvidence(), fixClaim()] };
  } else if (docChanged && evidenceChanged) {
    rem = {
      recommended: "reconcile",
      actions: [reconcile(id), reanchorAsIs(id), retire(id)],
    };
  } else if (evidenceChanged) {
    rem = {
      recommended: "update-claim",
      actions: [updateThenReanchor(id), reanchorAsIs(id), retire(id)],
    };
  } else if (docChanged) {
    rem = {
      recommended: "reverify-doc",
      actions: [reverifyDoc(id), reanchorAsIs(id), retire(id)],
    };
  } else if (hasAmbiguous) {
    rem = { recommended: "reanchor", actions: [reanchorTighten(id)] };
  } else if (hasMoved) {
    rem = { recommended: "reanchor", actions: [reanchorMoved(id)] };
  }

  if (v.expired) {
    const base = rem ?? { recommended: null, actions: [] };
    const recommended =
      base.recommended === "reanchor" || rem === null
        ? "reverify-and-rerecord"
        : base.recommended;
    rem = { recommended, actions: [...base.actions, reverifyAndReRecord()] };
  }

  return rem;
}

/** The single action a one-line surface shows: `recommended`, else the first. */
export function topAction(rem: Remediation | null): RemediationAction | null {
  if (!rem || rem.actions.length === 0) return null;
  return (
    (rem.recommended && rem.actions.find((a) => a.id === rem.recommended)) ||
    rem.actions[0] ||
    null
  );
}

export function remediationForVerdict(verdict: Verdict): Remediation | null {
  return remediationFor({
    claimId: verdict.claimId,
    doc: verdict.doc,
    evidence: verdict.evidence,
    judge: verdict.judge,
    expired: verdict.expired,
  });
}
