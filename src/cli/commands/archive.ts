/** `hibi archive`: archive a document. */
import { envelope } from "../render/json.ts";
import * as misc from "../render/misc.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  fail,
  str,
} from "../shared.ts";

export async function runArchive(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, open } = ctx;
  const { values } = args;
  const engine = await open();
  try {
    const result = await engine.archive(
      String(values.doc),
      str(values.successor),
      { dryRun: Boolean(values["dry-run"]) },
    );
    const next = result.dryRun
      ? "re-run without --dry-run to apply"
      : result.strandedClaims.length > 0
        ? "reanchor or retire each stranded claim, then hibi check"
        : "hibi check";
    await emit(
      mode,
      envelope(
        "archive",
        {
          doc: result.document.path,
          archivedTo: result.archivedTo,
          successor: result.successor ?? null,
          lifecycle: result.document.lifecycle,
          strandedClaims: result.strandedClaims,
          dryRun: result.dryRun,
        },
        next,
      ),
      () => misc.renderArchive(result, style, mode),
    );
    return 0;
  } catch (e) {
    return fail((e as Error).message, mode);
  }
}
