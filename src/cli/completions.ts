/**
 * Shell completion generators, derived from the option tables so they can
 * never drift from what the parser accepts.
 */

import { COMMANDS, GLOBAL_OPTIONS, optionsFor } from "./options.ts";

export type Shell = "zsh" | "bash" | "fish";

const VERBS = COMMANDS.filter((c) => !c.aliasOf).map((c) => c.name);
const GLOBAL_FLAGS = GLOBAL_OPTIONS.map((o) => `--${o.name}`);

function flagsFor(verb: string): string[] {
  const cmd = COMMANDS.find((c) => c.name === verb);
  return cmd ? optionsFor(cmd).map((o) => `--${o.name}`) : GLOBAL_FLAGS;
}

function zsh(): string {
  const verbCases = VERBS.map((v) => {
    const flags = flagsFor(v)
      .map((f) => `'${f}'`)
      .join(" ");
    return `        ${v}) _values 'flag' ${flags} ;;`;
  }).join("\n");
  return `#compdef hibi
# hibi zsh completions (generated from the option tables). Source this file or place it on your $fpath.
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
  return `# hibi bash completions (generated from the option tables). Source this file from ~/.bashrc.
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
    "# hibi fish completions (generated from the option tables). Place in ~/.config/fish/completions/hibi.fish",
    "complete -c hibi -f",
  ];
  for (const v of VERBS) {
    const summary = COMMANDS.find((c) => c.name === v)?.summary ?? `hibi ${v}`;
    lines.push(
      `complete -c hibi -n '__fish_use_subcommand' -a ${v} -d '${summary.replace(/'/g, "")}'`,
    );
  }
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
