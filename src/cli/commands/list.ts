/** `hibi list`: list claims by state and path. */
import { currentRef } from "../../git/git.ts";
import type { ListState } from "../../index.ts";
import { envelope } from "../render/json.ts";
import * as misc from "../render/misc.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  emitIds,
  fail,
  str,
} from "../shared.ts";

export async function runList(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, anchorRoot, open } = ctx;
  const { values } = args;
  const engine = await open();
  let result: import("../../index.ts").ListResult;
  try {
    result = await engine.list({
      state: (str(values.state) ?? "all") as ListState,
      path: str(values.path),
      ref: await currentRef(anchorRoot),
    });
  } catch (e) {
    return fail((e as Error).message, mode);
  }
  if (values["ids-only"]) {
    await emitIds(result.claims.map((c) => c.claimId));
    return 0;
  }
  await emit(mode, envelope("list", { ...result }), () =>
    misc.renderList(result, style, mode),
  );
  return 0;
}
