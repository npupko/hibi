/**
 * `hibi attest` (design/SPEC.md sections 11, 5, 9.4): append an attestation
 * to a claim. Without `ack` it is a `vouch`; with `ack` it acknowledges a
 * finding and requires a note:
 *
 *   - `conflict:<otherClaimId>`: the same ack is written to both claims,
 *     pinned to the pair id and both sentence hashes, so it lapses when
 *     either sentence changes;
 *   - `review`: pinned to the cache key of the claim's newest `supports`
 *     verdict.
 *
 * Counting rule (SPEC 5): only a `human:` vouch resets the ttl. An `agent:`
 * or `process:` vouch is recorded and does not count; an ack never counts.
 * The result carries `countsTowardTtl` so the caller sees which applied.
 *
 * JSON output (inside the CLI envelope `{ ok, action: "attest", schemaVersion, …, next }`):
 *
 *   { "claimIds": ["clm_…"] | ["clm_a", "clm_b"],   // two for an ack on a conflict
 *     "attestation": { "by": "human:alice@example.com", "at": "…",
 *                      "basis": "vouch | ack", "note": "…",
 *                      "ack": { "pairId": "clm_a:clm_b", "sentenceHashes": ["…", "…"] }
 *                           | { "judgeKey": "…" } },               // ack only
 *     "countsTowardTtl": true }
 */

import { createHash } from "node:crypto";
import {
  ACTOR_PATTERN,
  Attestation,
  type AttestationAck,
  type Claim,
} from "../core/model.ts";
import type { ClaimStore } from "../store/store.ts";

/** What `--ack` names: a contradiction with another claim, or a `review` verdict. */
export type AckTarget =
  | { kind: "conflict"; otherClaimId: string }
  | { kind: "review" };

export interface AttestInput {
  store: ClaimStore;
  claimId: string;
  by: string;
  note?: string;
  ack?: AckTarget;
  now?: Date;
}

export interface AttestResult {
  /** The claims the attestation was appended to: one, or both sides of a conflict pair. */
  claimIds: string[];
  attestation: Attestation;
  /** True only for a `human:` vouch (SPEC 5). */
  countsTowardTtl: boolean;
}

/** Parse the CLI's `--ack` value: `conflict:<claimId>` or `review`. */
export function parseAck(value: string): AckTarget {
  if (value === "review") return { kind: "review" };
  const m = /^conflict:(.+)$/.exec(value);
  if (m?.[1]) return { kind: "conflict", otherClaimId: m[1] };
  throw new Error(
    `--ack expects conflict:<claimId> or review (got ${JSON.stringify(value)})`,
  );
}

/**
 * sha256 of a sentence after NFC normalization, whitespace collapsed to one
 * space, and trimming. The pair hash for conflict acks (SPEC 9.4).
 */
function sentenceHash(sentence: string): string {
  const normalized = sentence.normalize("NFC").replace(/\s+/g, " ").trim();
  return createHash("sha256").update(normalized).digest("hex");
}

async function liveClaim(store: ClaimStore, id: string): Promise<Claim> {
  const claim = await store.getClaim(id);
  if (!claim) throw new Error(`No claim ${id} in the store.`);
  if (claim.enforcement === "retired") {
    throw new Error(`claim ${id} is retired; nothing to attest.`);
  }
  return claim;
}

/** The key of the claim's newest `supports` verdict: by the later of its `at` and its evidence time. */
async function newestSupportsKey(
  store: ClaimStore,
  claimId: string,
): Promise<string> {
  const entries = (await store.judgeCache().list({ claimId })).filter(
    (e) => e.judgment === "supports",
  );
  let best: { key: string; at: string } | undefined;
  for (const e of entries) {
    const at =
      e.evidence !== undefined && e.evidence.at > e.at ? e.evidence.at : e.at;
    if (best === undefined || at >= best.at) best = { key: e.key, at };
  }
  if (best === undefined) {
    throw new Error(
      `claim ${claimId} has no supports verdict to acknowledge; submit evidence with \`hibi evidence\` first.`,
    );
  }
  return best.key;
}

async function append(
  store: ClaimStore,
  claim: Claim,
  attestation: Attestation,
): Promise<void> {
  await store.putClaim({
    ...claim,
    attestations: [...claim.attestations, attestation],
  });
}

export async function attestClaim(input: AttestInput): Promise<AttestResult> {
  const { store, claimId, by, note, ack } = input;
  const at = (input.now ?? new Date()).toISOString();
  if (!ACTOR_PATTERN.test(by)) {
    throw new Error(
      `--by must be human:<id>, agent:<name>, or process:<name> (got ${JSON.stringify(by)}).`,
    );
  }
  if (note !== undefined && note.trim() === "") {
    throw new Error("--note is empty; pass the text or leave it out.");
  }
  const claim = await liveClaim(store, claimId);

  if (ack === undefined) {
    const attestation = Attestation.parse({ by, at, basis: "vouch", note });
    await append(store, claim, attestation);
    return {
      claimIds: [claim.id],
      attestation,
      countsTowardTtl: by.startsWith("human:"),
    };
  }

  if (note === undefined) {
    throw new Error(
      "an ack needs --note saying why the finding is acceptable.",
    );
  }

  if (ack.kind === "review") {
    const judgeKey = await newestSupportsKey(store, claim.id);
    const attestation = Attestation.parse({
      by,
      at,
      basis: "ack",
      note,
      ack: { judgeKey } satisfies AttestationAck,
    });
    await append(store, claim, attestation);
    return { claimIds: [claim.id], attestation, countsTowardTtl: false };
  }

  if (ack.otherClaimId === claim.id) {
    throw new Error(
      "--ack conflict: names the same claim; name the other one.",
    );
  }
  const other = await liveClaim(store, ack.otherClaimId);
  const [first, second] = [claim, other].sort((a, b) =>
    a.id < b.id ? -1 : 1,
  ) as [Claim, Claim];
  const attestation = Attestation.parse({
    by,
    at,
    basis: "ack",
    note,
    ack: {
      pairId: `${first.id}:${second.id}`,
      sentenceHashes: [
        sentenceHash(first.sentence),
        sentenceHash(second.sentence),
      ],
    } satisfies AttestationAck,
  });
  await append(store, first, attestation);
  await append(store, second, attestation);
  return {
    claimIds: [first.id, second.id],
    attestation,
    countsTowardTtl: false,
  };
}
