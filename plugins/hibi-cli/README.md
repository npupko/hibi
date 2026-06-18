# hibi-cli — Claude Code plugin

A Claude Code [Agent Skill](https://docs.claude.com/en/docs/agents-and-tools/agent-skills/overview)
that teaches coding agents to use the [hibi](https://github.com/npupko/hibi) CLI
([docs](https://npupko.mintlify.app)): bootstrap it in a fresh repo, record
well-anchored documentation claims, run the everyday `check` / `diff` / `query` /
`status` loops, read verdicts and exit codes, respond to flagged claims, wire hibi
into CI, and manage doc lifecycle.

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
        ├── SKILL.md                 # the skill (auto-discovered on install)
        ├── references/cli-reference.md
        └── assets/hibi-ci.yml       # drop-in GitHub Actions workflow
```

Claude loads the skill when you ask it to set up hibi, record or check claims,
respond to a flagged doc, or wire hibi into CI. You can also invoke it as
`/hibi-cli:hibi`.

## Develop

The canonical skill source lives here in the hibi repo. Edit `skills/hibi/SKILL.md`
(and its `references/` and `assets/`), commit, and users get it on the next
`/plugin marketplace update`.
