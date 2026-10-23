/** `hibi init`: create the claim store, optionally append the instruction line. */
import { appendFile, readFile, writeFile } from "node:fs/promises";
import { Engine } from "../../index.ts";
import { INSTRUCTIONS_LINE } from "../options.ts";
import { envelope } from "../render/json.ts";
import * as misc from "../render/misc.ts";
import {
  absPath,
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  fail,
  str,
} from "../shared.ts";

/** Append the instruction line to `file` unless it is already there. Creates the file. */
async function appendInstructionLine(
  file: string,
): Promise<{ file: string; appended: boolean }> {
  let text: string | null = null;
  try {
    text = await readFile(file, "utf8");
  } catch (e) {
    if ((e as { code?: string }).code !== "ENOENT") throw e;
  }
  if (text === null) {
    await writeFile(file, `${INSTRUCTIONS_LINE}\n`);
    return { file, appended: true };
  }
  if (text.includes(INSTRUCTIONS_LINE)) return { file, appended: false };
  const sep = text === "" || text.endsWith("\n") ? "" : "\n";
  await appendFile(file, `${sep}${INSTRUCTIONS_LINE}\n`);
  return { file, appended: true };
}

export async function runInit(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, anchorRoot, loc } = ctx;
  const { values } = args;
  let engine: Engine;
  try {
    engine = await Engine.init(loc);
  } catch (e) {
    return fail((e as Error).message, mode);
  }
  const config = await engine.store.config();
  const instructionsFile = str(values.instructions);
  let instructions: { file: string; appended: boolean } | undefined;
  if (instructionsFile !== undefined) {
    try {
      const r = await appendInstructionLine(
        absPath(anchorRoot, instructionsFile),
      );
      instructions = { file: instructionsFile, appended: r.appended };
    } catch (e) {
      return fail((e as Error).message, mode);
    }
  }
  const payload = {
    store: engine.store.dir,
    version: config.version,
    judge: config.judge,
    ...(instructions !== undefined ? { instructions } : {}),
  };
  await emit(mode, envelope("init", payload, "hibi audit --doc <file>"), () =>
    misc.renderInit(payload, style, mode),
  );
  return 0;
}
