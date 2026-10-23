/** `hibi audit`: report a document's claim coverage. */
import { auditExitCode } from "../../engine/audit.ts";
import { renderAudit } from "../render/audit.ts";
import { envelope } from "../render/json.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  fail,
} from "../shared.ts";

export async function runAudit(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, open } = ctx;
  const { values } = args;
  const engine = await open();
  const doc = String(values.doc);
  try {
    const report = await engine.audit(doc);
    await emit(mode, envelope("audit", { ...report }), () =>
      renderAudit(report, style, mode),
    );
    return auditExitCode(report, {
      failUncovered: Boolean(values["fail-uncovered"]),
    });
  } catch (e) {
    return fail((e as Error).message, mode);
  }
}
