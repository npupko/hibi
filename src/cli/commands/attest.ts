/** `hibi attest`: attest a claim. */
import { parseAck } from "../../engine/attest.ts";
import { renderAttest } from "../render/attest.ts";
import { envelope } from "../render/json.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  fail,
  str,
} from "../shared.ts";

export async function runAttest(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, open } = ctx;
  const { values, positionals } = args;
  const engine = await open();
  try {
    const ackValue = str(values.ack);
    const result = await engine.attest(positionals[0] as string, {
      by: String(values.by),
      note: str(values.note),
      ack: ackValue === undefined ? undefined : parseAck(ackValue),
    });
    await emit(mode, envelope("attest", { ...result }, "hibi check"), () =>
      renderAttest(result, style, mode),
    );
    return 0;
  } catch (e) {
    return fail((e as Error).message, mode);
  }
}
