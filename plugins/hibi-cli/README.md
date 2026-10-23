# hibi-cli: Claude Code plugin

A Claude Code [Agent Skill](https://docs.claude.com/en/docs/agents-and-tools/agent-skills/overview) that teaches coding agents to use the [hibi](https://github.com/npupko/hibi) CLI ([docs](https://hibi.mintlify.app)). hibi tracks checkable sentences (claims) in any document: READMEs, `CLAUDE.md`, process pages, pricing and marketing pages, decision records. Evidence can be code, another document, or something only an agent with tools can reach. The skill covers `init`, `audit --doc` to find what to record, `record` for `code`, `reference`, and `attested` claims, `check --doc` before trusting a document and `check --since` after a change, the worklist and `evidence` loop with the TypeSafe judge, `attest`, contradictions, expired claims, `reanchor` and `retire`, and CI.

## Install

From inside Claude Code:

```
/plugin marketplace add npupko/hibi
/plugin install hibi-cli@hibi
```

To pull the latest version after the repo changes, run `claude plugin update hibi-cli@hibi` in your shell, or select **Update now** on the plugin in the **Installed** tab of `/plugin`.

## What's inside

```
plugins/hibi-cli/
├── .claude-plugin/plugin.json
└── skills/
    └── hibi/
        ├── SKILL.md                     # the skill (auto-discovered on install)
        ├── references/cli-reference.md  # generated from src/cli/options.ts
        ├── references/cookbook.md       # eight worked recipes with JSON
        └── assets/hibi-ci.yml           # drop-in GitHub Actions workflow
```

Claude loads the skill when you ask whether a document is still true, before it relies on `CLAUDE.md` or `AGENTS.md`, after a feature, when docs or a knowledge repo drift, when documents contradict each other, or when you mention claims or `.claims/`. You can also invoke it as `/hibi-cli:hibi`.

The skill pre-approves `hibi check`, `hibi list`, `hibi audit`, and their `bunx @npupko/hibi` forms; they write nothing except judge cache entries. Mutating verbs prompt for permission. The judge needs `TYPESAFE_API_KEY`; without it, uncached judgments are `pending` and nothing fails.

## Develop

The canonical skill source lives in the hibi repo. Edit `skills/hibi/SKILL.md`, `references/cookbook.md`, or `assets/`, commit, and users get it when they update the plugin. Do not edit `references/cli-reference.md` by hand; run `bun run build:cli-reference` at the repo root.
