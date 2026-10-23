/** `hibi version`: print the version. */
import pkg from "../../../package.json" with { type: "json" };
import { envelope } from "../render/json.ts";
import * as misc from "../render/misc.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
} from "../shared.ts";

export async function runVersion(
  ctx: CommandContext,
  _args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style } = ctx;
  await emit(
    mode,
    envelope("version", { name: "hibi", version: pkg.version }),
    () => misc.renderVersion(pkg.version, style),
  );
  return 0;
}
