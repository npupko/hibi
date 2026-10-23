/**
 * What the CLI entry (`index.ts`) and the command modules (`commands/*.ts`)
 * share: the command shape, output writing, operational failure, and the
 * small value helpers.
 */
import { readFile } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import type { Engine, StoreLocation } from "../index.ts";
import type { OutputMode } from "./render/mode.ts";
import { makeStyle, type Style } from "./render/style.ts";

export const EXIT_OPERATIONAL_ERROR = 1;

export type Values = Record<string, string | boolean | string[] | undefined>;

export type Token = {
  kind: string;
  name?: string;
  value?: string;
};

/** What every command gets from the entry: the resolved output mode and store location. */
export interface CommandContext {
  mode: OutputMode;
  style: Style;
  anchorRoot: string;
  loc: string | StoreLocation;
  /** Open the store; reports "run hibi init" (exit 1) when there is none. */
  open: () => Promise<Engine>;
}

/** The parsed argv of one command. */
export interface CommandArgs {
  values: Values;
  positionals: string[];
  tokens: Token[];
}

/** The process exit code. */
export type CommandResult = number;

export type Command = (
  ctx: CommandContext,
  args: CommandArgs,
) => Promise<CommandResult>;

/**
 * Write to stdout or stderr and resolve once the write's callback reports it
 * written. Every CLI write goes through here and is awaited before
 * `process.exit`: a write to a pipe is asynchronous, and exiting earlier
 * drops whatever did not fit in the pipe buffer. A reader that closed early
 * (EPIPE, e.g. `| head`) wants nothing more, so that error is ignored.
 */
export function writeOut(
  stream: NodeJS.WriteStream,
  text: string,
): Promise<void> {
  return new Promise((resolve, reject) => {
    if (text.length === 0) return resolve();
    stream.write(text, (e) => {
      if (e && (e as { code?: string }).code !== "EPIPE") reject(e);
      else resolve();
    });
  });
}
export const print = (text: string) => writeOut(process.stdout, text);
export const printErr = (text: string) => writeOut(process.stderr, text);

export async function out(value: unknown, pretty: boolean): Promise<void> {
  await print(`${JSON.stringify(value, jsonReplacer, pretty ? 2 : 0)}\n`);
}
function jsonReplacer(_k: string, v: unknown) {
  return typeof v === "bigint" ? v.toString() : v;
}

export async function emitIds(ids: string[]): Promise<void> {
  const seen = new Set<string>();
  const list: string[] = [];
  for (const id of ids) {
    if (seen.has(id)) continue;
    seen.add(id);
    list.push(id);
  }
  if (list.length > 0) await print(`${list.join("\n")}\n`);
}

function isHuman(mode: OutputMode): boolean {
  return mode.kind === "rich" || mode.kind === "compact";
}

/** Emit a result: the human renderer in rich/compact mode, else JSON. */
export async function emit(
  mode: OutputMode,
  value: unknown,
  human: () => string | Promise<string>,
): Promise<void> {
  if (isHuman(mode)) await print(await human());
  else await out(value, mode.kind === "json-pretty");
}

/** Report an operational error and exit 1. `errors` (one line each) also goes into the JSON. */
export async function fail(
  message: string,
  mode: OutputMode,
  errors?: string[],
): Promise<never> {
  if (isHuman(mode)) {
    await printErr(`${makeStyle(mode.color).red("error:")} ${message}\n`);
  } else {
    await out(
      { ok: false, error: message, ...(errors ? { errors } : {}) },
      mode.kind === "json-pretty",
    );
  }
  process.exit(EXIT_OPERATIONAL_ERROR);
}

export function absPath(root: string, p: string): string {
  return isAbsolute(p) ? p : join(root, p);
}

export function str(v: unknown): string | undefined {
  return v === undefined ? undefined : String(v);
}

/** Read `--from-file <path|->`: stdin for `-`, else a file under the anchor root. */
export async function readFromFile(
  root: string,
  fromFile: string,
): Promise<string> {
  if (fromFile === "-") return Bun.stdin.text();
  try {
    return await readFile(absPath(root, fromFile), "utf8");
  } catch (e) {
    const code = (e as { code?: string }).code;
    if (code === "ENOENT")
      throw new Error(`--from-file ${fromFile}: no such file`);
    throw e;
  }
}
