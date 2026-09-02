/**
 * Generate the CLI reference from the option tables (`src/cli/options.ts`)
 * into `docs/cli-reference.mdx` and the plugin's
 * `references/cli-reference.md`. Derived artifacts, never hand-edited; CI
 * fails when they drift from the tables.
 */
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  COMMANDS,
  type CommandSpec,
  GLOBAL_OPTIONS,
  type OptionSpec,
} from "../src/cli/options.ts";
import { MODEL_VERSION } from "../src/core/model.ts";

const ROOT = join(import.meta.dir, "..");
const TARGETS = [
  {
    path: join(ROOT, "docs", "cli-reference.mdx"),
    frontmatter:
      '---\ntitle: "CLI reference"\ndescription: "Every hibi command and flag, generated from the option tables."\n---\n\n',
  },
  {
    path: join(
      ROOT,
      "plugins",
      "hibi-cli",
      "skills",
      "hibi",
      "references",
      "cli-reference.md",
    ),
    frontmatter: "# hibi CLI reference\n\n",
  },
];

function flag(o: OptionSpec): string {
  const ph = o.type === "string" ? ` ${o.placeholder ?? "<value>"}` : "";
  return `\`--${o.name}${ph}\``;
}

function optionRows(options: OptionSpec[]): string[] {
  const rows = ["| Flag | Meaning |", "|---|---|"];
  for (const o of options) {
    const values = o.values
      ? ` One of: ${o.values.map((v) => `\`${v}\``).join(", ")}.`
      : "";
    const req = o.required ? " Required." : "";
    const rep = o.multiple ? " Repeatable." : "";
    rows.push(`| ${flag(o)} | ${o.help}${values}${req}${rep} |`);
  }
  return rows;
}

function commandSection(cmd: CommandSpec): string[] {
  const lines: string[] = [];
  lines.push(`## \`hibi ${cmd.name}\``, "");
  lines.push(cmd.summary, "");
  if (cmd.aliasOf) {
    lines.push(
      `Deprecated alias of \`hibi ${cmd.aliasOf}\`. It prints a notice on stderr and will be removed in a later release.`,
      "",
    );
  }
  if (cmd.description) lines.push(cmd.description, "");
  lines.push("```sh");
  if (cmd.usage) lines.push(...cmd.usage);
  else {
    const pos = cmd.positional
      ? cmd.positional.required
        ? ` <${cmd.positional.name}>`
        : ` [${cmd.positional.name}]`
      : "";
    lines.push(
      `hibi ${cmd.name}${pos}${cmd.options.length ? " [options]" : ""}`,
    );
  }
  lines.push("```", "");
  if (cmd.positional) {
    const values = cmd.positional.values
      ? ` One of: ${cmd.positional.values.map((v) => `\`${v}\``).join(", ")}.`
      : "";
    lines.push(
      `\`<${cmd.positional.name}>\`: ${cmd.positional.help}${values}`,
      "",
    );
  }
  if (cmd.options.length > 0) {
    lines.push(...optionRows(cmd.options), "");
  }
  if (cmd.examples?.length) {
    lines.push("```sh", ...cmd.examples, "```", "");
  }
  return lines;
}

export function renderReference(): string {
  const lines: string[] = [];
  lines.push(
    "This page is generated from `src/cli/options.ts` by `bun run build:cli-reference`. Do not edit it by hand.",
    "",
    `Schema version: \`${MODEL_VERSION}\`. Every JSON envelope carries \`ok\`, \`action\`, and \`schemaVersion\`; most carry a \`next\` hint.`,
    "",
    "Exit codes: `0` clean, `2` gating (a `changed`, `orphaned`, `ambiguous`, `expired`, or `refuted` verdict on an enforced claim, or `moved` under `--fail-on warn`), `1` operational error (bad flag, no store, missing file).",
    "",
    "Run `hibi <command> --help` for the same tables in the terminal; it never runs the command.",
    "",
    "## Global options",
    "",
    "Accepted by every command.",
    "",
    ...optionRows(GLOBAL_OPTIONS),
    "",
    "Environment: `HIBI_ASCII=1` uses ASCII symbols in human output; `HIBI_ADVICE=0` drops remediation hints; `NO_COLOR` and `FORCE_COLOR` are honored.",
    "",
  );
  for (const cmd of COMMANDS) lines.push(...commandSection(cmd));
  return `${lines.join("\n").trimEnd()}\n`;
}

export async function generateCliReference(): Promise<string[]> {
  const body = renderReference();
  const written: string[] = [];
  for (const t of TARGETS) {
    await writeFile(t.path, t.frontmatter + body);
    written.push(t.path);
  }
  return written;
}

if (import.meta.main) {
  const written = await generateCliReference();
  for (const f of written) console.log(`wrote ${f}`);
}
