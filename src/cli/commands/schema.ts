/** `hibi schema`: list the schemas or print one as JSON Schema. */
import { envelope } from "../render/json.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  fail,
  out,
  str,
} from "../shared.ts";

export async function runSchema(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode } = ctx;
  const { values } = args;
  const { SCHEMAS } = await import("../../core/model.ts");
  const z = await import("zod");
  const all: Record<string, import("zod").ZodType> = { ...SCHEMAS };
  const name = str(values.name);
  // Machine output in every mode: indented unless the format is plain json.
  const pretty = mode.kind !== "json";
  if (name) {
    const schema = all[name];
    if (!schema) {
      return fail(
        `unknown schema: ${name}. Known: ${Object.keys(all).join(", ")}`,
        mode,
      );
    }
    await out(
      z.toJSONSchema(schema, { target: "draft-2020-12", reused: "inline" }),
      pretty,
    );
  } else {
    await out(envelope("schema", { schemas: Object.keys(all) }), pretty);
  }
  return 0;
}
