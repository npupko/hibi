/**
 * The CLI's option tables: one table per command plus the global options.
 * Everything that describes a flag lives here once. `parseArgs` (strict), the
 * per-command `--help`, the shell completions, and the generated CLI
 * reference (`scripts/gen-cli-reference.ts`) all read these tables.
 */

import { FAIL_ON } from "../engine/check.ts";
import { MAX_EVIDENCE_CHARS } from "../engine/evidence.ts";
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
    help: "Add the detail tail (regions, changes, similarity) to JSON verdicts and the full record result.",
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
    name: "fail-on",
    type: "string",
    values: FAIL_ON,
    placeholder: "<level>",
    help: "When to exit 2: gating (default), warn (also on moved), never.",
  },
];

export const COMMANDS: CommandSpec[] = [
  {
    name: "init",
    summary: "Create the claim store (.claims/) and its config.json.",
    options: [],
    examples: ["hibi init"],
  },
  {
    name: "record",
    summary:
      "Record a claim: a doc sentence anchored to the code it describes.",
    description:
      "The doc span's text is the claim's sentence. A new claim is enforced (it gates) unless --suggest is passed. The claim gets one vouch attestation by --by, or by human:<git user.email> when --by is absent. Agents should send one JSON object per claim on stdin with --from-file -; keys mirror the flags in camelCase (doc, docQuote, docRange, codeFile, codeQuote, codeRange, glob, suggest, ttl, owner, by).",
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
        name: "ttl",
        type: "string",
        placeholder: "<duration>",
        help: "ISO 8601 duration after which the claim needs re-attesting, e.g. P90D.",
      },
      {
        name: "owner",
        type: "string",
        placeholder: "<actor>",
        help: "The actor to contact when the claim expires: human:<id>, agent:<name>, or process:<name>.",
      },
      {
        name: "by",
        type: "string",
        placeholder: "<actor>",
        help: "The actor vouching for the claim. Default: human:<git user.email>.",
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
      "Read-only. --since scopes to files changed since a git ref; --doc scopes to one document; --overview renders the per-document table. A moved span is a warning: it exits 0 unless --fail-on warn.",
    options: CHECK_OPTIONS,
    examples: [
      "hibi check",
      "hibi check --since origin/main",
      "hibi check --doc CLAUDE.md",
      "hibi check --fail-on warn",
    ],
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
        help: "Filter: all (default), gating, warning, clean, orphaned, suggested, stranded (live on a superseded or archived doc).",
      },
      {
        name: "path",
        type: "string",
        placeholder: "<path>",
        help: "Only claims anchored to or covering this doc or evidence path (either side).",
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
    name: "audit",
    summary:
      "One row per region of a document: the claim that covers it, or the judge's triage kind.",
    description:
      "Splits the document into regions, matches them to live claims by the claim's doc anchor, and triages uncovered regions with the judge (cached). Kinds: code, reference, attested (record them), unverifiable (rewrite as an attributed opinion, or remove), not-a-claim, unsure (triage confidence below judge.threshold), pending (judge unavailable). Recording is never automatic.",
    options: [
      {
        name: "doc",
        type: "string",
        placeholder: "<path>",
        help: "The document to audit.",
        required: true,
      },
      {
        name: "fail-uncovered",
        type: "boolean",
        help: "Exit 2 when any uncovered region is code, reference, or attested.",
      },
    ],
    examples: [
      "hibi audit --doc README.md",
      "hibi audit --doc docs/pricing.md --fail-uncovered",
    ],
  },
  {
    name: "reanchor",
    summary:
      "Re-resolve a claim's spans and store the new baseline; or list candidate locations with --suggest.",
    description:
      "Without span flags each side re-localizes through its stored selectors. A side that is not found is refused unless an explicit new span is given for it. --doc moves the claim to a different document (requires a doc span). The claim's sentence is refreshed from the new doc span.",
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
        help: "Read-only: rank candidate locations for the stored doc and evidence quotes.",
      },
      DRY_RUN,
    ],
    examples: [
      "hibi reanchor clm_1a2b3c4d5e6f7a8b",
      "hibi reanchor clm_1a2b3c4d5e6f7a8b --suggest",
      'hibi reanchor clm_1a2b3c4d5e6f7a8b --doc docs/retry.md --doc-quote "Retries are capped at 5 attempts"',
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
    examples: ["hibi retire clm_1a2b3c4d5e6f7a8b"],
  },
  {
    name: "supersede",
    summary:
      "Mark a document superseded by another and relocate its live claims to the successor.",
    description:
      "A claim whose sentence appears verbatim in --to moves there (same id, evidence, attestations). The rest are reported as misses for a manual reanchor or retire. The old document records supersededBy.",
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
    name: "evidence",
    summary:
      "Submit evidence an agent collected; the judge decides whether it supports each claim.",
    description: `Reads a JSON array (or one object) of {claimId, evidence, source, summary, by}; an item without by takes --by. The whole batch is validated before the judge is asked; one bad item rejects it. A supports verdict at or above judge.threshold appends an evidence attestation, which resets the ttl. The verdict is cached by model, sentence, context, evidence, and source; raw evidence text is never stored. Without TYPESAFE_API_KEY or with the network down, an item is pending and nothing is written for it. Evidence is limited to ${MAX_EVIDENCE_CHARS} characters per item.`,
    usage: ["hibi evidence --from-file <path|-> --by <actor>"],
    options: [
      {
        name: "from-file",
        type: "string",
        placeholder: "<path|->",
        help: "JSON array of evidence items (- reads stdin). All or nothing.",
        required: true,
      },
      {
        name: "by",
        type: "string",
        placeholder: "<actor>",
        help: "The actor submitting the evidence, for items without by: human:<id>, agent:<name>, or process:<name>.",
        required: true,
      },
    ],
    examples: [
      'echo \'[{"claimId":"clm_1a2b3c4d5e6f7a8b","evidence":"Pro plan: $29.00 / month","source":"stripe prices list --product prod_123","summary":"Stripe lists Pro at $29/month"}]\' | hibi evidence --from-file - --by agent:opus',
    ],
  },
  {
    name: "attest",
    summary:
      "Vouch for a claim, or acknowledge a review or conflict finding with a note.",
    description:
      "Without --ack this records a vouch. Only a human: vouch resets the ttl; an agent: or process: vouch is recorded and does not. --ack review acknowledges the claim's newest supports verdict; --ack conflict:<id> acknowledges a contradiction with another claim and records the ack on both. An ack needs --note and never resets the ttl.",
    positional: {
      name: "claim-id",
      help: "The claim to attest.",
      required: true,
    },
    options: [
      {
        name: "by",
        type: "string",
        placeholder: "<actor>",
        help: "The attesting actor: human:<id>, agent:<name>, or process:<name>.",
        required: true,
      },
      {
        name: "note",
        type: "string",
        placeholder: "<text>",
        help: "What was checked, or why the finding is acceptable. Required with --ack.",
      },
      {
        name: "ack",
        type: "string",
        placeholder: "<finding>",
        help: "Acknowledge a finding: conflict:<claim-id> or review.",
      },
    ],
    examples: [
      'hibi attest clm_1a2b3c4d5e6f7a8b --by human:alice@example.com --note "checked Stripe, still $29"',
      'hibi attest clm_1a2b3c4d5e6f7a8b --by human:alice@example.com --ack conflict:clm_9f8e7d6c5b4a3210 --note "different plans"',
    ],
  },
  {
    name: "schema",
    summary: "Print the JSON Schema of a model type, or the list of names.",
    options: [
      {
        name: "name",
        type: "string",
        placeholder: "<Name>",
        help: "A schema name, e.g. Claim, Verdict, Config.",
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
  const width = Math.max(...COMMANDS.map((c) => c.name.length)) + 2;
  const lines = [
    "hibi: deterministic doc/code claim tracking",
    "",
    "Usage: hibi <command> [options]",
    "       hibi <command> --help",
    "",
    "Commands:",
    ...COMMANDS.map((c) => `  ${c.name.padEnd(width)}${c.summary}`),
    "",
    "Global options:",
    ...optionLines(GLOBAL_OPTIONS),
    "",
    "Exit codes: 0 clean · 2 gating (changed/orphaned/ambiguous/expired on an enforced claim, or moved under --fail-on warn) · 1 error",
    "Env: HIBI_ASCII=1 for ASCII symbols · HIBI_ADVICE=0 to drop remediation hints",
  ];
  return `${lines.join("\n")}\n`;
}
