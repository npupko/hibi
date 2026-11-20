/** Human output for `hibi attest`: what was recorded, and whether it resets the ttl. */

import type { AttestResult } from "../../engine/attest.ts";
import type { OutputMode } from "./mode.ts";
import type { Style } from "./style.ts";
import { badge } from "./symbols.ts";

export function renderAttest(
  result: AttestResult,
  style: Style,
  mode: OutputMode,
): string {
  const a = result.attestation;
  const ids = result.claimIds.map((id) => style.cyan(id)).join(" and ");
  const lines = [
    `${badge("clean", mode.unicode, style)} ${a.basis} recorded on ${ids}  ${style.dim(`by ${a.by}`)}`,
  ];
  if (a.note !== undefined) lines.push(`    ${style.dim(`note: ${a.note}`)}`);
  if (a.ack !== undefined) {
    const what =
      "judgeKey" in a.ack
        ? `review verdict ${a.ack.judgeKey.slice(0, 12)}`
        : `conflict ${a.ack.pairId}`;
    lines.push(`    ${style.dim(`acknowledges ${what}`)}`);
  }
  if (!result.countsTowardTtl) {
    const why =
      a.basis === "ack"
        ? "recorded; does not reset the ttl (an ack never does)"
        : "recorded; does not reset the ttl (agent vouches never do)";
    lines.push(`  ${badge("warn", mode.unicode, style)} ${style.dim(why)}`);
  }
  return `${lines.join("\n")}\n`;
}
