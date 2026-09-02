/**
 * The CLI's option tables: one table per command plus the global options.
 * Everything that describes a flag lives here once. `parseArgs` (strict), the
 * per-command `--help`, the shell completions, and the generated CLI
 * reference (`scripts/gen-cli-reference.ts`) all read these tables.
 */

import { FAIL_ON } from "../engine/check.ts";
import { LIST_STATES } from "../engine/list.ts";

export interface OptionSpec {
  /** Long flag name without the leading dashes. */
  name: string;
  type: "string" | "boolean";
  /** Allowed values for an enum-valued string option. */
  values?: readonly string[];
  /** Placeholder shown in help, e.g. `<path>`. */
  placeholder?: string;
  /** May be repeated. */
  multiple?: boolean;
  help: string;
  required?: boolean;
}

export interface PositionalSpec {
  name: string;
  help: string;
  required?: boolean;
  values?: readonly string[];
}

export interface CommandSpec {
  name: string;
  summary: string;
  /** Longer description for the help page and the reference. */
  description?: string;
  positional?: PositionalSpec;
  options: OptionSpec[];
  examples?: string[];
  /** A deprecated alias: help says so and the command maps onto `aliasOf`. */
  aliasOf?: string;
  /** Extra usage lines for the help page. */
  usage?: string[];
}

export const FORMATS = ["human", "compact", "json", "json-pretty"] as const;
export type Format = (typeof FORMATS)[number];

/** Options every command accepts. */
export const GLOBAL_OPTIONS: OptionSpec[] = [
  {
    name: "format",
    type: "string",
    values: FORMATS,
    placeholder: "<f>",
    help: "Output format. Default: human on a terminal, json when piped.",
  },
  { name: "json", type: "boolean", help: "Alias for --format json." },
  {
    name: "explain",
    type: "boolean",
    help: "Add the evidence tail (regions, changedEvidence, similarity) to JSON verdicts and the full record result.",
  },
  {
    name: "no-hints",
    type: "boolean",
    help: "Drop the remediation menu (also HIBI_ADVICE=0).",
  },
  {
    name: "color",
    type: "string",
    values: ["auto", "always", "never"],
    placeholder: "<when>",
    help: "Color in human output (also NO_COLOR / FORCE_COLOR).",
  },
  {
    name: "cwd",
    type: "string",
    placeholder: "<dir>",
    help: "Anchor root the paths resolve against. Default: the current directory.",
  },
  {
    name: "store-dir",
    type: "string",
    placeholder: "<dir>",
    help: "Store location. Default: <anchor root>/.claims.",
  },
  {
    name: "no-ast",
    type: "boolean",
    help: "Skip tree-sitter; text resolution only.",
  },
  { name: "help", type: "boolean", help: "Print this help and exit." },
];

const DOC_SPAN: OptionSpec[] = [
  {
    name: "doc-quote",
    type: "string",
    placeholder: "<text>",
    help: "The documented sentence, verbatim. Must occur in --doc.",
  },
  {
    name: "doc-range",
    type: "string",
    placeholder: "L1:L3",
    help: "Inclusive 1-based line range of the documented sentence (or char offsets as 100:130).",
  },
];

const CODE_SPAN: OptionSpec[] = [
  {
    name: "code-file",
    type: "string",
    placeholder: "<path>",
    help: "The code file the sentence describes.",
  },
  {
    name: "code-quote",
    type: "string",
    placeholder: "<text>",
    help: "The code span, verbatim. Quote the load-bearing token, e.g. the value.",
  },
  {
    name: "code-range",
    type: "string",
    placeholder: "L1:L9",
    help: "Inclusive 1-based line range of the code span (or char offsets).",
  },
  {
    name: "glob",
    type: "string",
    placeholder: "<pattern>",
    help: "A coarse file or glob edge instead of a precise code span. Navigation only, never graded as drift.",
  },
];

const DRY_RUN: OptionSpec = {
  name: "dry-run",
  type: "boolean",
  help: "Compute the result without writing the store or any document.",
};

const VERIFIER_OPTIONS: OptionSpec[] = [
  {
    name: "run-verifiers",
    type: "boolean",
    help: "Execute declared verifiers (repo-committed commands). Off by default.",
  },
  {
    name: "verifier-timeout",
    type: "string",
    placeholder: "<seconds>",
    help: "Per-verifier timeout in seconds. Default 120.",
  },
];

const CHECK_OPTIONS: OptionSpec[] = [
  {
    name: "since",
    type: "string",
    placeholder: "<ref>",
    help: "Check only the claims touching a file changed since this git ref.",
  },
  {
    name: "doc",
    type: "string",
    placeholder: "<path>",
    help: "Check only the claims on this document.",
  },
  {
    name: "overview",
    type: "boolean",
    help: "Render the per-document table instead of the per-claim report (human output).",
  },
  {
    name: "write",
    type: "boolean",
    help: "Stamp a status banner into each suspect document and clear it from clean ones.",
  },
  {
    name: "fail-on",
    type: "string",
    values: FAIL_ON,
    placeholder: "<level>",
    help: "When to exit 2: gating (default), warn (also on moved), never.",
  },
  ...VERIFIER_OPTIONS,
];

export const COMMANDS: CommandSpec[] = [
  {
    name: "init",
    summary: "Create the claim store (.claims/) with a per-repo banner nonce.",
    options: [],
    examples: ["hibi init"],
  },
  {
    name: "record",
    summary:
      "Record a claim: a doc sentence anchored to the code it describes.",
    description:
      "The doc span's text is the claim. A new claim is enforced (it gates) unless --suggest is passed. Agents should send one JSON object per claim on stdin with --from-file -; keys mirror the flags in camelCase (doc, docQuote, docRange, codeFile, codeQuote, codeRange, glob, suggest, verified, verifier, ttl, owner).",
    usage: [
      "hibi record --doc <path> (--doc-quote <text> | --doc-range L1:L3) --code-file <path> (--code-quote <text> | --code-range L1:L9) [options]",
      "hibi record --from-file <path|->",
    ],
    options: [
      {
        name: "doc",
        type: "string",
        placeholder: "<path>",
        help: "The document making the claim.",
      },
      ...DOC_SPAN,
      ...CODE_SPAN,
      {
        name: "suggest",
        type: "boolean",
        help: "Record an advisory claim that never gates (allows a coarse or empty code side).",
      },
      {
        name: "verified",
        type: "boolean",
        help: "Mark that you confirmed the code backs the sentence.",
      },
      {
        name: "verifier",
        type: "string",
        placeholder: "kind:ref",
        multiple: true,
        help: 'A verifier, e.g. command:"bun test retry". Runs only under check --run-verifiers. Repeatable.',
      },
      {
        name: "ttl",
        type: "string",
        placeholder: "<iso-8601>",
        help: "Instant after which the claim is expired and gates.",
      },
      {
        name: "owner",
        type: "string",
        placeholder: "<name>",
        help: "Attribution recorded on the claim.",
      },
      {
        name: "from-file",
        type: "string",
        placeholder: "<path|->",
        help: "Record a JSON array of claim specs (- reads stdin). All or nothing.",
      },
    ],
    examples: [
      'hibi record --doc README.md --doc-quote "Retries are capped at 5 attempts" --code-file src/retry.ts --code-quote "MAX_ATTEMPTS = 5"',
      'echo \'[{"doc":"README.md","docQuote":"Retries are capped at 5 attempts","codeFile":"src/retry.ts","codeQuote":"MAX_ATTEMPTS = 5"}]\' | hibi record --from-file -',
    ],
  },
  {
    name: "check",
    summary:
      "Verify every claim against the working tree and exit 0 (clean), 2 (gating), or 1 (error).",
    description:
      "Read-only unless --write. --since scopes to files changed since a git ref; --doc scopes to one document; --overview renders the per-document table. A moved span is a warning: it exits 0 unless --fail-on warn.",
    options: CHECK_OPTIONS,
    examples: [
      "hibi check",
      "hibi check --since origin/main",
      "hibi check --doc CLAUDE.md",
      "hibi check --write --fail-on warn",
    ],
  },
  {
    name: "diff",
    summary: "Deprecated alias for check --since <ref>.",
    aliasOf: "check",
    options: CHECK_OPTIONS.filter(
      (o) => o.name !== "doc" && o.name !== "overview",
    ),
  },
  {
    name: "status",
    summary: "Deprecated alias for check --doc <path> or check --overview.",
    aliasOf: "check",
    options: CHECK_OPTIONS.filter(
      (o) => o.name === "doc" || o.name === "fail-on",
    ),
  },
  {
    name: "list",
    summary:
      "One lean row per claim, filtered by state or by the file it anchors.",
    options: [
      {
        name: "state",
        type: "string",
        values: LIST_STATES,
        placeholder: "<state>",
        help: "Filter: all (default), gating, warning, clean, orphaned, suggested, stranded (live on a superseded or archived doc), duplicate (sentence claimed more than once).",
      },
      {
        name: "path",
        type: "string",
        placeholder: "<path>",
        help: "Only claims anchored to or covering this doc or code path (either side).",
      },
      {
        name: "ids-only",
        type: "boolean",
        help: "Print bare claim ids, one per line, for shell loops.",
      },
    ],
    examples: [
      "hibi list --state gating",
      "hibi list --path src/auth.ts",
      "hibi list --state orphaned --ids-only",
    ],
  },
  {
    name: "coverage",
    summary:
      "Which sentences of a document are backed by a claim and which are not.",
    options: [
      {
        name: "doc",
        type: "string",
        placeholder: "<path>",
        help: "The document to measure.",
        required: true,
      },
      {
        name: "fail-uncovered",
        type: "boolean",
        help: "Exit 2 when any region is uncovered.",
      },
    ],
    examples: ["hibi coverage --doc README.md"],
  },
  {
    name: "reanchor",
    summary:
      "Re-resolve a claim's spans and store the new baseline; or list candidate locations with --suggest.",
    description:
      "Without span flags each side re-localizes through its stored selectors. A side that is not found is refused unless an explicit new span is given for it. --doc moves the claim to a different document (requires a doc span).",
    positional: {
      name: "claim-id",
      help: "The claim to reanchor.",
      required: true,
    },
    options: [
      {
        name: "doc",
        type: "string",
        placeholder: "<path>",
        help: "Move the doc anchor to this document (with --doc-quote or --doc-range).",
      },
      ...DOC_SPAN,
      ...CODE_SPAN,
      {
        name: "suggest",
        type: "boolean",
        help: "Read-only: rank candidate locations for the stored doc and code quotes.",
      },
      DRY_RUN,
    ],
    examples: [
      "hibi reanchor asrt_1a2b3c4d",
      "hibi reanchor asrt_1a2b3c4d --suggest",
      'hibi reanchor asrt_1a2b3c4d --doc docs/retry.md --doc-quote "Retries are capped at 5 attempts"',
    ],
  },
  {
    name: "retire",
    summary: "Withdraw a claim so it no longer gates. Idempotent.",
    positional: {
      name: "claim-id",
      help: "The claim to retire.",
      required: true,
    },
    options: [DRY_RUN],
    examples: ["hibi retire asrt_1a2b3c4d"],
  },
  {
    name: "supersede",
    summary:
      "Mark a document superseded by another and relocate its live claims to the successor.",
    description:
      "A claim whose sentence appears verbatim in --to moves there (same id, code side, history). The rest are reported as misses for a manual reanchor or retire.",
    options: [
      {
        name: "from",
        type: "string",
        placeholder: "<path>",
        help: "The old document.",
        required: true,
      },
      {
        name: "to",
        type: "string",
        placeholder: "<path>",
        help: "The new document.",
        required: true,
      },
      DRY_RUN,
    ],
    examples: ["hibi supersede --from design-v1.md --to design-v2.md"],
  },
  {
    name: "archive",
    summary:
      "Move an obsolete document to archive/ and leave a tombstone at its path.",
    options: [
      {
        name: "doc",
        type: "string",
        placeholder: "<path>",
        help: "The document to archive.",
        required: true,
      },
      {
        name: "successor",
        type: "string",
        placeholder: "<path>",
        help: "The document the tombstone points to.",
      },
      DRY_RUN,
    ],
    examples: ["hibi archive --doc old.md --successor new.md"],
  },
  {
    name: "schema",
    summary:
      "Print the JSON Schema of a model or protocol type, or the list of names.",
    options: [
      {
        name: "name",
        type: "string",
        placeholder: "<Name>",
        help: "A schema name, e.g. Assertion, Verdict, ResolveParams.",
      },
    ],
    examples: ["hibi schema", "hibi schema --name Verdict"],
  },
  {
    name: "completions",
    summary: "Print a shell completion script.",
    positional: {
      name: "shell",
      help: "zsh, bash, or fish.",
      required: true,
      values: ["zsh", "bash", "fish"],
    },
    options: [],
    examples: ["hibi completions zsh > ~/.zfunc/_hibi"],
  },
  {
    name: "version",
    summary: "Print the hibi version and schema version.",
    options: [],
  },
];

export const COMMAND_NAMES = COMMANDS.map((c) => c.name);

export function commandSpec(name: string): CommandSpec | undefined {
  return COMMANDS.find((c) => c.name === name);
}

/** The full option set a command parses: its own table plus the globals. */
export function optionsFor(cmd: CommandSpec): OptionSpec[] {
  return [...cmd.options, ...GLOBAL_OPTIONS];
}

/** `parseArgs` config derived from a command's tables. */
export function parseArgsOptions(
  cmd: CommandSpec,
): Record<string, { type: "string" | "boolean"; multiple?: boolean }> {
  const out: Record<
    string,
    { type: "string" | "boolean"; multiple?: boolean }
  > = {};
  for (const o of optionsFor(cmd)) {
    out[o.name] = { type: o.type, ...(o.multiple ? { multiple: true } : {}) };
  }
  return out;
}

/**
 * Validate enum-valued options and required options after parsing. Returns
 * the first error message, or undefined.
 */
export function validateValues(
  cmd: CommandSpec,
  values: Record<string, unknown>,
): string | undefined {
  for (const o of optionsFor(cmd)) {
    const v = values[o.name];
    if (v === undefined) {
      if (o.required) return `${cmd.name} requires --${o.name}`;
      continue;
    }
    if (o.values) {
      const list = Array.isArray(v) ? v : [v];
      for (const item of list) {
        if (!o.values.includes(String(item))) {
          return `--${o.name} expects ${o.values.join("|")} (got: ${String(item)})`;
        }
      }
    }
  }
  return undefined;
}

function flagUsage(o: OptionSpec): string {
  const ph = o.type === "string" ? ` ${o.placeholder ?? "<value>"}` : "";
  return `--${o.name}${ph}`;
}

function optionLines(options: OptionSpec[]): string[] {
  const width = Math.max(...options.map((o) => flagUsage(o).length), 0) + 2;
  return options.map((o) => {
    const values = o.values ? ` [${o.values.join("|")}]` : "";
    return `  ${flagUsage(o).padEnd(width)}${o.help}${values}`;
  });
}

/** The `hibi <cmd> --help` text. */
export function commandHelp(cmd: CommandSpec): string {
  const lines: string[] = [];
  lines.push(`hibi ${cmd.name}: ${cmd.summary}`);
  if (cmd.aliasOf) {
    lines.push(
      `  This alias prints a deprecation notice and will be removed. Use \`hibi ${cmd.aliasOf}\`.`,
    );
  }
  if (cmd.description) lines.push("", cmd.description);
  lines.push("", "Usage:");
  if (cmd.usage) {
    for (const u of cmd.usage) lines.push(`  ${u}`);
  } else {
    const pos = cmd.positional
      ? cmd.positional.required
        ? ` <${cmd.positional.name}>`
        : ` [${cmd.positional.name}]`
      : "";
    lines.push(
      `  hibi ${cmd.name}${pos}${cmd.options.length ? " [options]" : ""}`,
    );
  }
  if (cmd.positional) {
    lines.push("", "Arguments:");
    const values = cmd.positional.values
      ? ` [${cmd.positional.values.join("|")}]`
      : "";
    lines.push(`  <${cmd.positional.name}>  ${cmd.positional.help}${values}`);
  }
  if (cmd.options.length > 0) {
    lines.push("", "Options:", ...optionLines(cmd.options));
  }
  lines.push("", "Global options:", ...optionLines(GLOBAL_OPTIONS));
  if (cmd.examples?.length) {
    lines.push("", "Examples:", ...cmd.examples.map((e) => `  ${e}`));
  }
  return `${lines.join("\n")}\n`;
}

/** The top-level `hibi --help` text. */
export function usage(): string {
  const shown = COMMANDS.filter((c) => !c.aliasOf);
  const width = Math.max(...shown.map((c) => c.name.length)) + 2;
  const lines = [
    "hibi: deterministic doc/code claim tracking",
    "",
    "Usage: hibi <command> [options]",
    "       hibi <command> --help",
    "",
    "Commands:",
    ...shown.map((c) => `  ${c.name.padEnd(width)}${c.summary}`),
    "",
    "Global options:",
    ...optionLines(GLOBAL_OPTIONS),
    "",
    "Exit codes: 0 clean · 2 gating (changed/orphaned/ambiguous/expired/refuted on an enforced claim, or moved under --fail-on warn) · 1 error",
    "Env: HIBI_ASCII=1 for ASCII symbols · HIBI_ADVICE=0 to drop remediation hints",
  ];
  return `${lines.join("\n")}\n`;
}
