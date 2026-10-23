/** `hibi check`: verify claims and report verdicts. */
import { changedFiles, currentRef } from "../../git/git.ts";
import { type Engine, type FailOn, normalizePath } from "../../index.ts";
import { renderCheck } from "../render/check.ts";
import { fileReader } from "../render/helpers.ts";
import { type ProjectionOptions, projectCheckReport } from "../render/json.ts";
import type { OutputMode } from "../render/mode.ts";
import { renderOverview } from "../render/status.ts";
import { makeStyle } from "../render/style.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  fail,
  str,
} from "../shared.ts";

function projection(mode: OutputMode): ProjectionOptions {
  return { explain: mode.explain, hints: mode.hints };
}

/** Build the rich/compact `check` render context. */
async function checkContext(
  engine: Engine,
  report: import("../../index.ts").CheckReport,
  mode: OutputMode,
  lead?: string[],
) {
  const claims = await engine.store.allClaims();
  return {
    report,
    claimsById: new Map(claims.map((c) => [c.id, c])),
    read: fileReader(engine.store.anchorRoot),
    style: makeStyle(mode.color),
    mode,
    lead,
  };
}

export async function runCheck(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, anchorRoot, open } = ctx;
  const { values } = args;
  const engine = await open();
  const since = str(values.since);
  const doc = str(values.doc);
  let files: string[] | undefined;
  if (since !== undefined) {
    try {
      files = await changedFiles(since, anchorRoot);
    } catch (e) {
      return fail((e as Error).message, mode);
    }
  }
  let report: import("../../index.ts").CheckReport;
  try {
    report = await engine.check({
      onlyFiles: files,
      doc,
      failOn: (str(values["fail-on"]) ?? "gating") as FailOn,
      ref: await currentRef(anchorRoot),
    });
  } catch (e) {
    return fail((e as Error).message, mode);
  }
  const extra: Record<string, unknown> = {};
  if (since !== undefined) {
    extra.since = since;
    extra.changedFiles = files;
  }
  if (doc !== undefined) {
    extra.doc = normalizePath(engine.store.anchorRoot, doc);
  }
  const value = projectCheckReport(report, projection(mode), extra);
  await emit(mode, value, async () => {
    if (values.overview) {
      const [claims, config] = await Promise.all([
        engine.store.allClaims(),
        engine.store.config(),
      ]);
      return renderOverview({
        report,
        claims,
        storeVersion: config.version,
        style,
        mode,
      });
    }
    const lead: string[] = [];
    if (since !== undefined) {
      lead.push(
        `${style.dim("since")} ${style.bold(since)}  ${style.dim(`${files?.length ?? 0} changed file${files?.length === 1 ? "" : "s"}`)}`,
      );
    }
    return renderCheck(await checkContext(engine, report, mode, lead));
  });
  return report.exitCode;
}
