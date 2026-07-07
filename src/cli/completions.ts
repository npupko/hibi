/**
 * Hand-rolled shell completion generators (§9 `completions <zsh|bash|fish>`).
 * Static MVP: the verb list + the global and per-verb flags. `parseArgs` has no
 * Tab adapter, so this emits a self-contained script per shell — no new
 * dependency, matching hibi's vendor-tiny philosophy.
 */

export type Shell = "zsh" | "bash" | "fish";

/** The verbs offered at the first argument position. */
const VERBS = [
  "init",
  "record",
  "check",
  "diff",
  "status",
  "query",
  "list",
  "suggest",
  "reanchor",
  "retire",
  "supersede",
  "retract",
  "archive",
  "schema",
  "completions",
  "version",
  "help",
] as const;

/** Flags available on every command (the output mode + store/anchor globals). */
const GLOBAL_FLAGS = [
  "--json",
  "--pretty",
  "--compact",
  "--explain",
  "--detailed",
  "--no-hints",
  "--color",
  "--simple",
  "--cwd",
  "--store-dir",
  "--no-ast",
];

/** Per-verb flags (beyond the globals) offered after the verb. */
const VERB_FLAGS: Record<string, string[]> = {
  record: [
    "--doc",
    "--doc-quote",
    "--doc-range",
    "--doc-line",
    "--inline-id",
    "--code-file",
    "--code-quote",
    "--code-range",
    "--code-line",
    "--coarse",
    "--glob",
    "--from-file",
    "--trust",
    "--enforce",
    "--enforcement",
    "--behavioral",
    "--no-behavioral",
    "--pristine",
    "--verifier",
    "--owner",
    "--ref",
    "--ttl",
  ],
  check: ["--write", "--fail-on", "--run-verifiers", "--verifier-timeout"],
  diff: ["--since", "--write", "--fail-on"],
  status: ["--doc"],
  query: ["--path"],
  list: ["--state"],
  suggest: ["--doc", "--since"],
  reanchor: [
    "--doc",
    "--doc-quote",
    "--doc-range",
    "--doc-line",
    "--code-file",
    "--ref",
  ],
  retire: [],
  supersede: ["--new", "--old", "--type", "--propositions"],
  retract: ["--doc"],
  archive: ["--doc", "--successor"],
  schema: ["--name"],
  completions: [],
};

function flagsFor(verb: string): string[] {
  return [...(VERB_FLAGS[verb] ?? []), ...GLOBAL_FLAGS];
}

function zsh(): string {
  const verbCases = VERBS.map((v) => {
    const flags = flagsFor(v)
      .map((f) => `'${f}'`)
      .join(" ");
    return `        ${v}) _values 'flag' ${flags} ;;`;
  }).join("\n");
  return `#compdef hibi
# hibi zsh completions — source this file or place it on your $fpath.
_hibi() {
  local -a verbs
  verbs=(${VERBS.map((v) => `'${v}'`).join(" ")})
  if (( CURRENT == 2 )); then
    _describe 'command' verbs
    return
  fi
  case "\${words[2]}" in
${verbCases}
    *) _values 'flag' ${GLOBAL_FLAGS.map((f) => `'${f}'`).join(" ")} ;;
  esac
}
_hibi "$@"
`;
}

function bash(): string {
  const verbCases = VERBS.map((v) => {
    return `    ${v}) opts="${flagsFor(v).join(" ")}" ;;`;
  }).join("\n");
  return `# hibi bash completions — source this file (e.g. from ~/.bashrc).
_hibi() {
  local cur prev verb opts
  cur="\${COMP_WORDS[COMP_CWORD]}"
  verb="\${COMP_WORDS[1]}"
  if [ "$COMP_CWORD" -eq 1 ]; then
    COMPREPLY=( $(compgen -W "${VERBS.join(" ")}" -- "$cur") )
    return 0
  fi
  case "$verb" in
${verbCases}
    *) opts="${GLOBAL_FLAGS.join(" ")}" ;;
  esac
  COMPREPLY=( $(compgen -W "$opts" -- "$cur") )
}
complete -F _hibi hibi
`;
}

function fish(): string {
  const lines: string[] = [
    "# hibi fish completions — place in ~/.config/fish/completions/hibi.fish",
    "complete -c hibi -f",
  ];
  // Verb completions at position 1 (no command seen yet).
  for (const v of VERBS) {
    lines.push(
      `complete -c hibi -n '__fish_use_subcommand' -a ${v} -d 'hibi ${v}'`,
    );
  }
  // Per-verb flag completions.
  for (const v of VERBS) {
    for (const f of flagsFor(v)) {
      const long = f.replace(/^--/, "");
      lines.push(
        `complete -c hibi -n '__fish_seen_subcommand_from ${v}' -l ${long}`,
      );
    }
  }
  return `${lines.join("\n")}\n`;
}

export function completionScript(shell: Shell): string {
  switch (shell) {
    case "zsh":
      return zsh();
    case "bash":
      return bash();
    case "fish":
      return fish();
  }
}

export function isShell(s: string | undefined): s is Shell {
  return s === "zsh" || s === "bash" || s === "fish";
}
