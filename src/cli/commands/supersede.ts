/** `hibi supersede`: move a document's claims to its successor. */
import { envelope } from "../render/json.ts";
import * as misc from "../render/misc.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  fail,
} from "../shared.ts";

export async function runSupersede(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, open } = ctx;
  const { values } = args;
  const engine = await open();
  try {
    const result = await engine.supersede({
      from: String(values.from),
      to: String(values.to),
      dryRun: Boolean(values["dry-run"]),
    });
    const next = result.dryRun
      ? "re-run without --dry-run to apply"
      : result.relocated.some((r) => r.doc === "orphaned")
        ? "reanchor or retire each orphaned claim, then hibi check"
        : result.misses.length > 0
          ? "fix each failed move, then hibi check"
          : "hibi check";
    await emit(
      mode,
      envelope(
        "supersede",
        {
          from: result.oldDoc.path,
          to: result.newDoc.path,
          relocated: result.relocated,
          misses: result.misses,
          strandedClaims: result.strandedClaims,
          dryRun: result.dryRun,
        },
        next,
      ),
      () => misc.renderSupersede(result, style, mode),
    );
    return 0;
  } catch (e) {
    return fail((e as Error).message, mode);
  }
}
