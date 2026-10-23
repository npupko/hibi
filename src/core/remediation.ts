/**
 * The deterministic verdict→remediation mapping: the single source of the
 * "what do I do about this flag?" menu, consumed by the JSON and the renderer.
 *
 * A menu, not a prescription: hibi cannot know intent, so `recommended` is set
 * only when the next step is clear, and `actions` are ordered by preference.
 *
 * Each condition contributes a section, most severe first:
 *
 *   1. anchor gating (orphaned, changed, ambiguous): reanchor / update / retire
 *   2. judge `refuted`: `fix-evidence`, `fix-claim`; never reanchor, so a
 *      refuted claim gets no changed or ambiguous anchor section (an orphan
 *      still does: the span must be found before anything else)
 *   3. conflicts: `reconcile` naming the other claim, then one `ack` per
 *      other claim (`--ack conflict:<otherId>`)
 *   4. expired: `attest` (a human vouch), `evidence` (attested claims only)
 *   5. judge `review`: `ack` (`--ack review`), `evidence` (attested claims
 *      only; a reference claim is judged from the referenced document)
 *   6. moved: `reanchor`
 *
 * `recommended` is the first section's recommendation (it may be null);
 * `actions` are the sections' actions in order, each (id, title) once.
 * `pending` and `unavailable` add nothing: the reason label says what to do.
 */

import type {
  AnchorState,
  ClaimKind,
  EvidenceState,
  JudgeState,
  Remediation,
  RemediationAction,
  Verdict,
} from "./model.ts";

export interface RemediationInput {
  claimId: string;
  /** The claim kind; `evidence` is offered only for `attested` claims. */
  kind?: ClaimKind;
  doc: AnchorState;
  evidence?: EvidenceState;
  judge?: JudgeState;
  expired: boolean;
  /** Other claims this one conflicts with. */
  conflicts?: ReadonlyArray<{ claimId: string }>;
  /** The located text of the doc side; used when the doc side reads `changed`. */
  docText?: string | null;
  /**
   * Every evidence span in stored order, when a side reads `changed` or
   * `ambiguous`: its file and the quote to pass (the located whole-word text
   * of a `changed` side, the stored quote of the others, null for an
   * `ambiguous` side). An explicit span list replaces all of a claim's spans,
   * so the command names every one.
   */
  evidenceSpans?: ReadonlyArray<{ file: string; quote: string | null }>;
}

/** Double-quote a shell word: `"`, `$`, `` ` `` and `\` are escaped. */
function shellQuote(text: string): string {
  return `"${text.replace(/["$`\\]/g, "\\$&")}"`;
}

/** A path as a shell word: bare when it has no special characters. */
function shellPath(path: string): string {
  return /^[\w./:@+-]+$/.test(path) ? path : shellQuote(path);
}

/**
 * The span flags plain `reanchor` needs: a side that reads `changed` or
 * `ambiguous` is left as stored without an explicit span. A `changed` side
 * carries its located text when known, else a placeholder; an `ambiguous`
 * side carries the placeholder (the quote must be widened).
 */
function spanFlags(v: RemediationInput): string {
  const parts: string[] = [];
  if (v.doc === "changed" || v.doc === "ambiguous") {
    const text =
      v.doc === "changed" && v.docText ? v.docText : "<new sentence>";
    parts.push(valueFlag("doc-quote", text));
  }
  if (v.evidence === "changed" || v.evidence === "ambiguous") {
    const flag = v.kind === "reference" ? "ref" : "code";
    const spans = v.evidenceSpans?.length
      ? v.evidenceSpans
      : [{ file: "<file>", quote: null }];
    for (const span of spans) {
      const file = span.file === "<file>" ? span.file : shellPath(span.file);
      parts.push(
        `--${flag}-file ${file} ${valueFlag(`${flag}-quote`, span.quote || "<quote>")}`,
      );
    }
  }
  return parts.map((p) => ` ${p}`).join("");
}

/** `--flag "<text>"`, or `--flag="<text>"` when the text starts with `-`. */
function valueFlag(flag: string, text: string): string {
  return `--${flag}${text.startsWith("-") ? "=" : " "}${shellQuote(text)}`;
}

const reanchorCmd = (id: string, flags = ""): string =>
  `hibi reanchor ${id}${flags}`;
const retireCmd = (id: string): string => `hibi retire ${id}`;

function updateThenReanchor(id: string, flags: string): RemediationAction {
  return {
    id: "update-claim",
    title: "Update the sentence, then reanchor",
    rationale:
      "the evidence changed; if the sentence is now wrong, rewrite it, then run the command",
    command: reanchorCmd(id, flags),
  };
}

function reverifyDoc(id: string, flags: string): RemediationAction {
  return {
    id: "reverify-doc",
    title: "Re-read the edited sentence against the evidence, then reanchor",
    rationale:
      "the prose changed; confirm the evidence still backs it, then run the command",
    command: reanchorCmd(id, flags),
  };
}

function reconcile(id: string, flags: string): RemediationAction {
  return {
    id: "reconcile",
    title: "Reconcile the doc and the evidence, then reanchor",
    rationale:
      "both sides changed; re-verify the current sentence against the current evidence, then run the command",
    command: reanchorCmd(id, flags),
  };
}

function reanchorAsIs(id: string, flags: string): RemediationAction {
  return {
    id: "reanchor",
    title: "Reanchor as is",
    rationale: "the sentence is still true; accept the new span",
    command: reanchorCmd(id, flags),
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

function reanchorTighten(id: string, flags: string): RemediationAction {
  return {
    id: "reanchor",
    title: "Reanchor to a unique span",
    rationale: "the quote matches several places; pass a wider span",
    command: reanchorCmd(id, flags),
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

function fixClaim(id: string): RemediationAction {
  return {
    id: "fix-claim",
    title: "Fix the documented claim",
    rationale: `the judge found the evidence refutes ${id}; if the evidence is right, rewrite the sentence`,
  };
}

function fixEvidence(): RemediationAction {
  return {
    id: "fix-evidence",
    title: "Fix the evidence to match the claim",
    rationale:
      "if the sentence is the source of truth, the evidence drifted from it; correct the evidence, or submit newer evidence",
  };
}

const evidenceCmd = (id: string): string =>
  `echo '[{"claimId":"${id}","evidence":"<text>","source":"<where>","summary":"<one line>"}]' | hibi evidence --from-file - --by agent:<name>`;

function submitEvidence(id: string, why: string): RemediationAction {
  return {
    id: "evidence",
    title: "Collect evidence and submit it",
    rationale: why,
    command: evidenceCmd(id),
  };
}

function attest(id: string, evidenceApplies: boolean): RemediationAction {
  return {
    id: "attest",
    title: "Have a human re-check and vouch",
    rationale: evidenceApplies
      ? "only a human vouch or judged evidence resets the ttl; an agent vouch does not count"
      : "only a human vouch resets the ttl; an agent vouch does not count",
    command: `hibi attest ${id} --by human:<you> --note "<what you checked>"`,
  };
}

function ackReview(id: string): RemediationAction {
  return {
    id: "ack",
    title: "Acknowledge the review finding",
    rationale:
      "if the review finding is not a problem, record why; an ack never resets the ttl",
    command: `hibi attest ${id} --by <actor> --ack review --note "<why>"`,
  };
}

function ackConflict(id: string, other: string): RemediationAction {
  return {
    id: "ack",
    title: `Acknowledge the conflict with ${other}`,
    rationale:
      "if the conflict is not a problem, record why; the ack is written to both claims and never resets the ttl",
    command: `hibi attest ${id} --by <actor> --ack conflict:${other} --note "<why>"`,
  };
}

function reconcileConflict(others: string[]): RemediationAction {
  const list = others.join(", ");
  return {
    id: "reconcile",
    title: `Reconcile with ${list}`,
    rationale: `this sentence contradicts ${list}; rewrite whichever is wrong`,
  };
}

interface Section {
  recommended: string | null;
  actions: RemediationAction[];
}

function anchorSection(v: RemediationInput): Section | null {
  const id = v.claimId;
  const flags = spanFlags(v);
  const hasOrphan = v.doc === "orphaned" || v.evidence === "orphaned";
  const docChanged = v.doc === "changed";
  const evidenceChanged = v.evidence === "changed";
  const hasAmbiguous = v.doc === "ambiguous" || v.evidence === "ambiguous";
  if (hasOrphan) {
    return {
      recommended: "reanchor",
      actions: [reanchorSuggest(id), retire(id), supersede()],
    };
  }
  if (docChanged && evidenceChanged) {
    return {
      recommended: "reconcile",
      actions: [reconcile(id, flags), reanchorAsIs(id, flags), retire(id)],
    };
  }
  if (evidenceChanged) {
    return {
      recommended: "update-claim",
      actions: [
        updateThenReanchor(id, flags),
        reanchorAsIs(id, flags),
        retire(id),
      ],
    };
  }
  if (docChanged) {
    return {
      recommended: "reverify-doc",
      actions: [reverifyDoc(id, flags), reanchorAsIs(id, flags), retire(id)],
    };
  }
  if (hasAmbiguous) {
    return { recommended: "reanchor", actions: [reanchorTighten(id, flags)] };
  }
  return null;
}

/** The remediation menu for a verdict, or `null` when there is nothing to do. */
export function remediationFor(v: RemediationInput): Remediation | null {
  const id = v.claimId;
  const sections: Section[] = [];

  const refuted = v.judge === "refuted";
  const orphaned = v.doc === "orphaned" || v.evidence === "orphaned";
  const anchor = refuted && !orphaned ? null : anchorSection(v);
  if (anchor) sections.push(anchor);
  if (refuted) {
    sections.push({
      recommended: null,
      actions: [fixEvidence(), fixClaim(id)],
    });
  }
  const others = (v.conflicts ?? []).map((c) => c.claimId);
  if (others.length > 0) {
    sections.push({
      recommended: "reconcile",
      actions: [
        reconcileConflict(others),
        ...others.map((other) => ackConflict(id, other)),
      ],
    });
  }
  // `hibi evidence` accepts attested claims only.
  const evidenceApplies = v.kind === undefined || v.kind === "attested";
  if (v.expired) {
    sections.push({
      recommended: null,
      actions: [
        attest(id, evidenceApplies),
        ...(evidenceApplies
          ? [
              submitEvidence(
                id,
                "the ttl ran out; judged evidence that supports the sentence resets it",
              ),
            ]
          : []),
      ],
    });
  }
  if (v.judge === "review") {
    sections.push({
      recommended: null,
      actions: [
        ackReview(id),
        ...(evidenceApplies
          ? [
              submitEvidence(
                id,
                "the judge was not sure; evidence that addresses the sentence directly settles it",
              ),
            ]
          : []),
      ],
    });
  }
  if (v.doc === "moved" || v.evidence === "moved") {
    sections.push({ recommended: "reanchor", actions: [reanchorMoved(id)] });
  }

  const first = sections[0];
  if (first === undefined) return null;
  const actions: RemediationAction[] = [];
  const seen = new Set<string>();
  for (const section of sections) {
    for (const a of section.actions) {
      const key = `${a.id}\0${a.title}`;
      if (seen.has(key)) continue;
      seen.add(key);
      actions.push(a);
    }
  }
  return { recommended: first.recommended, actions };
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

/** A retired claim's verdict (`doc: retired`) has no remediation. */
export function remediationForVerdict(verdict: Verdict): Remediation | null {
  if (verdict.doc === "retired" || verdict.evidence === "retired") return null;
  return remediationFor({
    claimId: verdict.claimId,
    kind: verdict.kind,
    doc: verdict.doc,
    evidence: verdict.evidence,
    judge: verdict.judge,
    expired: verdict.expired,
    conflicts: verdict.conflicts,
  });
}
