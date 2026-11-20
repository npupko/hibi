/** `hibi evidence`: record judged evidence from `--from-file`. */
import { EvidenceValidationError } from "../../index.ts";
import { evidenceNext, renderEvidence } from "../render/evidence.ts";
import { envelope } from "../render/json.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  fail,
  readFromFile,
  str,
} from "../shared.ts";

export async function runEvidence(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, anchorRoot, open } = ctx;
  const { values } = args;
  const engine = await open();
  const fromFile = String(values["from-file"]);
  const by = str(values.by);
  let parsed: unknown;
  try {
    const raw = await readFromFile(anchorRoot, fromFile);
    try {
      parsed = JSON.parse(raw);
    } catch (e) {
      throw new Error(`invalid JSON (${(e as Error).message})`);
    }
  } catch (e) {
    return fail(`evidence --from-file: ${(e as Error).message}`, mode);
  }
  const isItem = (item: unknown): item is Record<string, unknown> =>
    typeof item === "object" && item !== null && !Array.isArray(item);
  const raws = Array.isArray(parsed) ? parsed : [parsed];
  if (by === undefined) {
    const missing = raws.flatMap((item, i) =>
      isItem(item) && !("by" in item)
        ? [
            `item ${i}${typeof item.claimId === "string" ? ` (${item.claimId})` : ""}: no "by"; pass --by <actor> or a "by" key: human:<id>, agent:<name>, or process:<name>`,
          ]
        : [],
    );
    if (missing.length > 0) {
      const err = new EvidenceValidationError(missing);
      return fail(err.message, mode, err.errors);
    }
  }
  const items = raws.map((item) =>
    isItem(item) && !("by" in item) ? { ...item, by } : item,
  );
  try {
    const report = await engine.evidence(items);
    const next = evidenceNext(report);
    await emit(mode, envelope("evidence", { ...report }, next), () =>
      renderEvidence(report, style, mode),
    );
    return 0;
  } catch (e) {
    if (e instanceof EvidenceValidationError) {
      return fail(e.message, mode, e.errors);
    }
    return fail((e as Error).message, mode);
  }
}
