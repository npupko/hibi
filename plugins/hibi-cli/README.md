# hibi-cli plugin for Claude Code

This plugin bundles the `hibi` Agent Skill. The skill teaches Claude Code to
use [hibi](https://github.com/npupko/hibi) as memory across sessions:

- run `hibi status` at the start of a session and re-check each stale claim;
- record the result with `hibi mark`, `hibi reanchor` or `hibi retire`, and
  ask the user when it cannot tell whether a document or the code is right;
- record new checks with `hibi claim`;
- find units with no claim with `hibi cover`, and limit both reports to a
  branch with `--since`.

The plugin does not install hibi. Install the binary first; see the
[quickstart](https://hibi.mintlify.app/quickstart).

## Install the plugin

In Claude Code:

```text
/plugin marketplace add npupko/hibi
/plugin install hibi-cli@hibi
```

## Contents

| Path | What it is |
|---|---|
| `skills/hibi/SKILL.md` | The skill: when to use hibi, the session loop, reading `next`, anchors, batches |
| `skills/hibi/references/cookbook.md` | Worked tasks with real commands and outputs |
| `skills/hibi/references/cli-reference.md` | Every command and option. Generated from the code by `cargo run -p hibi-cli --example cli-reference`; do not edit by hand |

## Permissions

The skill's `allowed-tools` lists `hibi status`, `hibi list`, `hibi cover`,
`hibi providers`, `hibi version` and `hibi schema`. Claude Code runs those
without asking only during the turn that invokes the skill; the grant clears
when you send your next message. Commands that write to `.hibi/` ask for
permission as usual.

Two of the listed commands can do more than read, because an allow rule
matches any arguments after the command name and cannot exclude a flag:

- `hibi status --judge-order` sends claim statements and the text of their
  sides to TypeSafe's API. It needs `TYPESAFE_API_KEY`, so it fails without
  the key.
- `hibi providers --check <scheme>` starts the provider program registered
  in your user config.

The skill tells the agent to run either only when you ask. Claude Code
permission settings can also hold deny rules such as
`Bash(hibi status --judge-order:*)`; deny rules are evaluated before allow
rules, but a rule matches the command words as written, so it does not catch
the flag in another position.
