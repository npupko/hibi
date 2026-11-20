/** `hibi reanchor`: move a claim's spans, suggest candidates, or relocate moved claims. */
import { currentRef } from "../../git/git.ts";
import type { EvidenceTarget, RegionSpec } from "../../index.ts";
import { parseRange } from "../record-input.ts";
import { envelope } from "../render/json.ts";
import * as misc from "../render/misc.ts";
import {
  type CommandArgs,
  type CommandContext,
  type CommandResult,
  emit,
  fail,
  str,
  type Token,
  type Values,
} from "../shared.ts";

/** A span from `--*-quote <text>` or `--*-range L42:L44 | 100:130`; undefined when neither was given. */
function spanSpec(quote: unknown, range: unknown): RegionSpec | undefined {
  if (quote !== undefined) return { quote: String(quote) };
  if (range !== undefined) return parseRange(String(range));
  return undefined;
}

/** The replacement reference span of `reanchor` (`--ref-file` with `--ref-quote` or `--ref-range`). */
function reanchorRefSide(values: Values): EvidenceTarget | undefined {
  const refFile = str(values["ref-file"]);
  const region = spanSpec(values["ref-quote"], values["ref-range"]);
  if (refFile === undefined) {
    if (region !== undefined) {
      throw new Error("--ref-quote/--ref-range need --ref-file");
    }
    return undefined;
  }
  if (region === undefined) {
    throw new Error(`--ref-file ${refFile} needs --ref-quote or --ref-range`);
  }
  return { file: refFile, region };
}

/**
 * The replacement code spans of `reanchor`, in flag order: each `--code-file`
 * opens a span that takes the `--code-quote` or `--code-range` after it.
 */
function reanchorCodeSide(tokens: Token[]): EvidenceTarget[] {
  const spans: { file: string; region?: RegionSpec }[] = [];
  for (const t of tokens) {
    if (t.kind !== "option" || t.value === undefined) continue;
    if (t.name === "code-file") {
      spans.push({ file: t.value });
    } else if (t.name === "code-quote" || t.name === "code-range") {
      const last = spans.at(-1);
      if (last === undefined) {
        throw new Error(`--${t.name} needs a --code-file before it`);
      }
      if (last.region !== undefined) {
        throw new Error(
          `--code-file ${last.file} already has a span; pass one --code-quote or --code-range per --code-file`,
        );
      }
      last.region =
        t.name === "code-quote" ? { quote: t.value } : parseRange(t.value);
    }
  }
  return spans.map((s) => {
    if (s.region === undefined) {
      throw new Error(
        `--code-file ${s.file} needs --code-quote or --code-range`,
      );
    }
    return { file: s.file, region: s.region };
  });
}

export async function runReanchor(
  ctx: CommandContext,
  args: CommandArgs,
): Promise<CommandResult> {
  const { mode, style, anchorRoot, open } = ctx;
  const { values, positionals, tokens } = args;
  const engine = await open();
  const dryRun = Boolean(values["dry-run"]);
  if (values.moved) {
    if (positionals[0] !== undefined) {
      return fail(
        "--moved relocates every live claim and takes no claim id; pass --doc <path> to restrict it to one document.",
        mode,
      );
    }
    const spanFlag = [
      values["doc-quote"],
      values["doc-range"],
      values["code-file"],
      values["code-quote"],
      values["code-range"],
      values["ref-file"],
      values["ref-quote"],
      values["ref-range"],
      values.suggest,
    ].some((f) => f !== undefined && f !== false);
    if (spanFlag) {
      return fail(
        "--moved cannot be combined with span flags or --suggest.",
        mode,
      );
    }
    try {
      const result = await engine.reanchorMoved({
        doc: str(values.doc),
        ref: await currentRef(anchorRoot),
        dryRun,
      });
      await emit(
        mode,
        envelope(
          "reanchor",
          {
            moved: true,
            relocated: result.relocated,
            claims: result.claims,
            ...(dryRun ? { dryRun: true } : {}),
          },
          dryRun ? "re-run without --dry-run to apply" : "hibi check",
        ),
        () => misc.renderReanchorMoved(result, style, mode, dryRun),
      );
      return 0;
    } catch (e) {
      return fail((e as Error).message, mode);
    }
  }
  if (positionals[0] === undefined) {
    return fail("reanchor requires a <claim-id> argument, or --moved", mode);
  }
  const claimId = positionals[0];
  const mutation = [
    values.doc,
    values["doc-quote"],
    values["doc-range"],
    values["code-file"],
    values["code-quote"],
    values["code-range"],
    values["ref-file"],
    values["ref-quote"],
    values["ref-range"],
    values["dry-run"],
  ].some((f) => f !== undefined && f !== false);
  if (values.suggest) {
    if (mutation) {
      return fail(
        "--suggest is read-only and cannot be combined with span or write flags.",
        mode,
      );
    }
    try {
      const result = await engine.reanchorSuggest(claimId);
      await emit(
        mode,
        envelope("reanchor-suggest", {
          id: result.claimId,
          candidates: result.candidates,
        }),
        () => misc.renderReanchorSuggest(result, style, mode),
      );
      return 0;
    } catch (e) {
      return fail((e as Error).message, mode);
    }
  }
  try {
    const docSpec = spanSpec(values["doc-quote"], values["doc-range"]);
    const code = reanchorCodeSide(tokens);
    const refSpan = reanchorRefSide(values);
    if (code.length > 0 && refSpan !== undefined) {
      throw new Error(
        "pass either --code-* or --ref-* flags, not both; a claim is code or reference",
      );
    }
    const result = await engine.reanchor(claimId, {
      doc: str(values.doc),
      docQuote: docSpec?.quote,
      docRange: docSpec?.quote === undefined ? docSpec : undefined,
      evidence: code.length > 0 ? code : undefined,
      reference: refSpan,
      ref: await currentRef(anchorRoot),
      dryRun,
    });
    const reasons = result.kept.map((k) => k.reason);
    if (result.relocated === 0 && reasons.length > 0) {
      return fail(
        `claim ${result.claim.id} was left as stored: ${reasons.join(" | ")}`,
        mode,
        reasons,
      );
    }
    const payload = {
      id: result.claim.id,
      doc: result.doc,
      ...(result.evidence !== undefined ? { evidence: result.evidence } : {}),
      before: result.before,
      after: result.after,
      ...(result.kept.length > 0 ? { kept: result.kept } : {}),
      ...(result.warnings.length > 0 ? { warnings: result.warnings } : {}),
      ...(dryRun ? { dryRun: true } : {}),
      ...(mode.explain ? { claim: result.claim } : {}),
    };
    await emit(
      mode,
      envelope(
        "reanchor",
        payload,
        dryRun ? "re-run without --dry-run to apply" : "hibi check",
      ),
      () => misc.renderReanchor(result, style, mode, dryRun),
    );
    return 0;
  } catch (e) {
    return fail((e as Error).message, mode);
  }
}
