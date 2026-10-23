/** `hibi completions`: print a shell completion script. */
import { completionScript, isShell } from "../completions.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  fail,
  print,
} from "../shared.ts";

export async function runCompletions(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode } = ctx;
  const { positionals } = args;
  const shell = positionals[0];
  if (!isShell(shell)) {
    return fail(`completions requires zsh | bash | fish`, mode);
  }
  await print(completionScript(shell));
  return 0;
}
