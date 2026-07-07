/**
 * The deterministic verdict→remediation mapping (§9) — the single source of the
 * "what do I do about this flag?" menu, consumed by both the machine JSON and
 * the human renderer.
 *
 * This is a *menu*, not a prescription. hibi routes attention deterministically
 * but cannot know developer intent (was the code change deliberate? is the doc
 * the spec, or stale prose?), so `recommended` is set only when the next step is
 * unambiguous, and each row's `actions` are ordered safest/most-severe-first so
 * truncation + primacy favor the safe action. The mapping is a fixed lookup over
 * the verdict's computed states — never a model decision (§7/§11), which is why
 * surfacing it does not violate the "no model on the verdict path" invariant.
 *
 * The lookup key is the tuple `(doc, code, behavior?, expired)`. `expired` is an
 * orthogonal time flag (never a state), composed onto whatever anchor/behavior
 * remediation already applies. Document *lifecycle* (superseded/amended/
 * retracted) is a separate, document-scoped concern surfaced via the document
 * report's banner copy, not here — a Verdict carries no lifecycle.
 */

import type {
  AnchorState,
  BehaviorState,
  ChangedEvidence,
  Remediation,
  RemediationAction,
  Verdict,
} from "./model.ts";

/** The verdict fields the mapping reads (a Verdict satisfies this). */
export interface RemediationInput {
  assertionId: string;
  doc: AnchorState;
  code: AnchorState;
  behavior?: BehaviorState;
  expired: boolean;
  changedEvidence?: ChangedEvidence[];
}

// ── Action builders ──────────────────────────────────────────────────────────
// `command` is populated only for runnable deterministic actions with the claim
// id pre-filled; prose actions (and the orphan re-anchor, which needs a target)
// carry none so an agent never runs a command that cannot succeed.

const reanchorCmd = (id: string): string => `hibi reanchor ${id}`;
const retireCmd = (id: string): string => `hibi retire ${id}`;

function reanchorMoved(id: string): RemediationAction {
  return {
    id: "reanchor",
    title: "Re-anchor to current content",
    applicability: "auto",
    effect: "deterministic",
    rationale: "the span moved (content intact) — update its stored position",
    command: reanchorCmd(id),
  };
}

function reanchorTighten(id: string): RemediationAction {
  return {
    id: "reanchor",
    title: "Tighten the anchor",
    applicability: "needs-review",
    effect: "deterministic",
    rationale: "the anchor matches several places — re-anchor to a unique span",
    command: reanchorCmd(id),
  };
}

function reanchorIfTrue(id: string): RemediationAction {
  return {
    id: "reanchor",
    title: "Re-anchor if the sentence still holds",
    applicability: "needs-review",
    effect: "deterministic",
    rationale:
      "the content changed — re-anchor only after confirming it is true",
    command: reanchorCmd(id),
  };
}

/** Orphan re-anchor: a bare `reanchor` cannot resolve it, so NO command. */
function reanchorToTarget(): RemediationAction {
  return {
    id: "reanchor",
    title: "Re-anchor to a new location",
    applicability: "manual",
    effect: "deterministic",
    rationale:
      "the span was deleted — re-anchor with an explicit --doc-range / --code-file target",
  };
}

function retire(id: string): RemediationAction {
  return {
    id: "retire",
    title: "Retire the claim",
    applicability: "manual",
    effect: "deterministic",
    rationale: "the claim is obsolete — withdraw it so it no longer gates",
    command: retireCmd(id),
  };
}

function supersede(): RemediationAction {
  return {
    id: "supersede",
    title: "Supersede the document",
    applicability: "manual",
    effect: "prose",
    rationale:
      "a newer document replaces this one — author the supersedes edge",
  };
}

function fixCode(): RemediationAction {
  return {
    id: "fix-code",
    title: "Fix the code to match the doc",
    applicability: "manual",
    effect: "prose",
    rationale: "if the doc is the spec, the code drifted from it",
  };
}

function fixClaim(): RemediationAction {
  return {
    id: "fix-claim",
    title: "Fix the documented claim",
    applicability: "manual",
    effect: "prose",
    rationale: "if the code is correct, the sentence is now wrong — rewrite it",
  };
}

function reverifyDoc(): RemediationAction {
  return {
    id: "reverify-doc",
    title: "Re-read the current doc span and re-verify",
    applicability: "manual",
    effect: "prose",
    rationale:
      "the prose was edited — its meaning may have inverted; re-verify against the code",
  };
}

function reconcile(): RemediationAction {
  return {
    id: "reconcile",
    title: "Reconcile the doc and the code",
    applicability: "manual",
    effect: "prose",
    rationale:
      "both sides changed — re-verify the current doc against the current code; do not auto-decide",
  };
}

function reverifyBehavior(detail: string | undefined): RemediationAction {
  return {
    id: "reverify-behavior",
    title: "Re-verify the documented behavior",
    applicability: "manual",
    effect: "prose",
    rationale: detail
      ? `reachable code changed (${detail}) — re-examine the behavior`
      : "reachable code changed — re-examine the behavior",
  };
}

/** Execution-grounding seam (D13): run the linked verifier(s) out-of-process. */
function runVerifier(): RemediationAction {
  return {
    id: "run-verifier",
    title: "Run the linked verifier",
    applicability: "needs-review",
    effect: "deterministic",
    rationale: "executable evidence can confirm or refute the behavior",
    command: "hibi check --run-verifiers",
  };
}

function reverifyAndReRecord(): RemediationAction {
  return {
    id: "reverify-and-rerecord",
    title: "Re-verify and re-record",
    applicability: "manual",
    effect: "prose",
    rationale:
      "the claim's ttl has passed — re-verify, then re-record it fresh",
  };
}

/** A one-line `path (kind)` summary of the first changed-evidence entry. */
function changedSummary(
  evidence: ChangedEvidence[] | undefined,
): string | undefined {
  const first = evidence?.[0];
  return first ? `${first.path} ${first.kind}` : undefined;
}

/**
 * The remediation menu for a verdict, or `null` when there is nothing to do
 * (a clean verdict). The branches are mutually exclusive and ordered by
 * severity; `expired` is appended onto whatever applies.
 */
export function remediationFor(v: RemediationInput): Remediation | null {
  const id = v.assertionId;
  let rem: Remediation | null = null;

  const hasOrphan = v.doc === "orphaned" || v.code === "orphaned";
  const docChanged = v.doc === "changed";
  const codeChanged = v.code === "changed";
  const hasAmbiguous = v.doc === "ambiguous" || v.code === "ambiguous";
  const hasMoved = v.doc === "moved" || v.code === "moved";

  if (hasOrphan) {
    // The span was deleted — retire/supersede/re-anchor-to-target, whatever the
    // behavior axis says. Checked BEFORE `refuted` so an orphaned-and-refuted
    // claim keeps a withdraw path (a refuted claim with intact anchors still
    // takes the refuted branch below).
    rem = {
      recommended: "retire",
      actions: [retire(id), supersede(), reanchorToTarget()],
    };
  } else if (v.behavior === "refuted") {
    // A linked verifier failed: never re-anchor (re-linking clears the gate
    // without fixing the behavior — a documented anti-pattern).
    rem = { recommended: null, actions: [fixCode(), fixClaim()] };
  } else if (docChanged && codeChanged) {
    rem = {
      recommended: null,
      actions: [reconcile(), reanchorIfTrue(id), retire(id)],
    };
  } else if (codeChanged) {
    rem = {
      recommended: null,
      actions: [retire(id), fixCode(), reanchorIfTrue(id)],
    };
  } else if (docChanged) {
    rem = {
      recommended: null,
      actions: [reverifyDoc(), retire(id), reanchorIfTrue(id)],
    };
  } else if (hasAmbiguous) {
    rem = { recommended: "reanchor", actions: [reanchorTighten(id)] };
  } else if (hasMoved) {
    rem = { recommended: "reanchor", actions: [reanchorMoved(id)] };
  } else if (v.behavior === "at-risk") {
    rem = {
      recommended: null,
      actions: [
        reverifyBehavior(changedSummary(v.changedEvidence)),
        runVerifier(),
      ],
    };
  }

  if (v.expired) {
    // `expired` gates on its own and `reanchor` alone never clears it — only
    // re-verifying and re-recording does (and that re-anchors too). So when the
    // base recommendation was a bare re-anchor (moved/ambiguous), or there was
    // nothing else to do, promote `reverify-and-rerecord` to `recommended`;
    // otherwise keep the base recommendation (e.g. `retire` for an orphan, or
    // `null` for an intent-ambiguous change).
    const base = rem ?? { recommended: null, actions: [] };
    const recommended =
      base.recommended === "reanchor" || rem === null
        ? "reverify-and-rerecord"
        : base.recommended;
    rem = {
      recommended,
      actions: [...base.actions, reverifyAndReRecord()],
    };
  }

  return rem;
}

/**
 * The single action a one-line surface (a human `help:` crumb, a row's "next
 * step") should show: the `recommended` action when set, else the safest/first.
 * The single source for "what's the top action?" so every renderer agrees.
 */
export function topAction(rem: Remediation | null): RemediationAction | null {
  if (!rem || rem.actions.length === 0) return null;
  return (
    (rem.recommended && rem.actions.find((a) => a.id === rem.recommended)) ||
    rem.actions[0] ||
    null
  );
}

/** Convenience over a full Verdict (reads its evidence for the at-risk detail). */
export function remediationForVerdict(verdict: Verdict): Remediation | null {
  return remediationFor({
    assertionId: verdict.assertionId,
    doc: verdict.doc,
    code: verdict.code,
    behavior: verdict.behavior,
    expired: verdict.expired,
    changedEvidence: verdict.evidence.changedEvidence,
  });
}
