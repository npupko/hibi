/** `hibi retire`: retire a claim. */
import { envelope } from "../render/json.ts";
import * as misc from "../render/misc.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  fail,
} from "../shared.ts";

export async function runRetire(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, open } = ctx;
  const { values, positionals } = args;
  const engine = await open();
  const claimId = positionals[0] as string;
  const dryRun = Boolean(values["dry-run"]);
  try {
    const result = await engine.retire(claimId, { dryRun });
    await emit(
      mode,
      envelope(
        "retire",
        {
          id: result.claim.id,
          alreadyRetired: result.alreadyRetired,
          ...(dryRun ? { dryRun: true } : {}),
        },
        dryRun ? "re-run without --dry-run to apply" : "hibi check",
      ),
      () => misc.renderRetire(result, style, mode, dryRun),
    );
    return 0;
  } catch (e) {
    return fail((e as Error).message, mode);
  }
}
