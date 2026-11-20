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

The skill pre-approves only the commands that read: `hibi status`,
`hibi list`, `hibi cover`, `hibi version` and `hibi schema`. Commands that
write to `.hibi/` ask for permission as usual.
