# hibi-cli: Claude Code plugin

A Claude Code [Agent Skill](https://docs.claude.com/en/docs/agents-and-tools/agent-skills/overview) that teaches coding agents to use the [hibi](https://github.com/npupko/hibi) CLI ([docs](https://npupko.mintlify.app)): initialize the store, record claims as JSON on stdin, run `check` (with `--doc`, `--since`, `--overview`), `list`, and `coverage`, read the verdicts and exit codes, act on the remediation menu (`reanchor`, `retire`), manage document lifecycle (`supersede`, `archive`), and wire hibi into CI.

## Install

From inside Claude Code:

```
/plugin marketplace add npupko/hibi
/plugin install hibi-cli@hibi
```

Run `/plugin marketplace update` to pull the latest version after the repo changes.

## What's inside

```
plugins/hibi-cli/
├── .claude-plugin/plugin.json
└── skills/
    └── hibi/
        ├── SKILL.md                     # the skill (auto-discovered on install)
        ├── references/cli-reference.md  # generated from src/cli/options.ts
        ├── references/cookbook.md       # six worked examples with JSON
        └── assets/hibi-ci.yml           # drop-in GitHub Actions workflow
```

Claude loads the skill when you ask it to set up hibi, record or check claims, respond to a flagged doc, or wire hibi into CI. You can also invoke it as `/hibi-cli:hibi`.

The skill pre-approves the read-only commands (`hibi check`, `hibi list`, `hibi coverage`, and their `bunx hibi` forms). Mutating verbs prompt for permission.

## Develop

The canonical skill source lives in the hibi repo. Edit `skills/hibi/SKILL.md`, `references/cookbook.md`, or `assets/`, commit, and users get it on the next `/plugin marketplace update`. Do not edit `references/cli-reference.md` by hand; run `bun run build:cli-reference` at the repo root.
