<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/npupko/hibi/main/assets/logo/hibi-wordmark-dark.png">
    <img alt="Hibi 日々" src="https://raw.githubusercontent.com/npupko/hibi/main/assets/logo/hibi-wordmark-transparent.png" width="300">
  </picture>
</p>

<p align="center"><em>Catch documentation that no longer matches your code.</em></p>

<p align="center"><a href="https://npupko.mintlify.app"><strong>Documentation</strong></a></p>

<p align="center">
  <a href="https://github.com/npupko/hibi/actions/workflows/ci.yml"><img src="https://github.com/npupko/hibi/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://www.npmjs.com/package/@npupko/hibi"><img src="https://img.shields.io/npm/v/@npupko/hibi.svg" alt="npm version"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
  <!-- Uncomment once the first OpenSSF Scorecard run has published results:
  <a href="https://securityscorecards.dev/viewer/?uri=github.com/npupko/hibi"><img src="https://api.securityscorecards.dev/projects/github.com/npupko/hibi/badge" alt="OpenSSF Scorecard"></a>
  -->
</p>

Hibi tracks **claims**: sentences in your docs and AI-agent instructions that assert how the code behaves. You anchor each claim to the code it describes. When that code changes, `hibi check` flags the claim and can stamp a status banner into the doc, so no reader and no agent acts on a page that has fallen out of sync with the source.

Run it in CI, in a git hook, or as a pre-edit lookup an agent makes before it trusts a doc.

## Install

```sh
# Prebuilt single-file executable (no runtime needed)
curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | sh

# Or, in a Bun/JS project
bun add @npupko/hibi
```

## Quick start

```sh
hibi init                       # create .claims/ (with a per-repo banner nonce)

# Record a claim: anchor the doc sentence to the constant that backs it (span-first)
hibi record \
  --doc README.md --doc-quote "Retries are capped at 5 attempts" \
  --code-file src/retry.ts --code-quote "MAX_ATTEMPTS = 5" --trust verified --owner alice

hibi record --from-file claims.json  # batch-author many claims (a JSON array; - = stdin)

hibi check                      # verify every claim
hibi check --write              # verify, and stamp status banners into affected docs
hibi diff --since origin/main   # what did this change invalidate?
hibi query --path src/retry.ts  # before editing: which claims cover this file?
hibi suggest --doc README.md    # propose anchorable claims from a doc (suggested records)
hibi reanchor <claim-id> --doc-quote "…" --code-file src/retry.ts  # re-resolve a claim
hibi reanchor <claim-id> --doc docs/retry.md --doc-quote "…"  # relocate it to another file
hibi relocate --from v1.md --to v2.md  # batch-move every live claim from one doc to another
hibi supersede --new v2.md --old v1.md --type supersedes
hibi doctor                     # store-health report (orphans, stale, duplicates; always exits 0)
hibi status                     # repo-wide document health overview
hibi status --doc README.md     # is this one doc still current?
```

**Output is TTY-aware.** Run hibi in a terminal and you get a rich, grouped-by-document
report with color and symbols; pipe or redirect it (or run it in CI) and you get compact
JSON — so the machine contract is unchanged. Override with the flag vocabulary:

| flag | output |
|------|--------|
| _(default)_ | rich human view on a TTY, compact JSON when piped/redirected/CI |
| `--json` | force compact JSON (the machine contract; what agents read) |
| `--json --pretty` | indented JSON |
| `--pretty` | force the rich human view, even when piped |
| `--compact` | one line per claim (human) |
| `--color auto\|always\|never` | color control (also honors `NO_COLOR` / `FORCE_COLOR`) |
| `--simple` | ASCII symbols instead of unicode |

`hibi completions <zsh\|bash\|fish>` prints a shell completion script.

### Exit codes

| code | meaning |
|------|---------|
| `0`  | all clean |
| `2`  | gating: `changed` / `orphaned` / `ambiguous` / `expired` / `refuted` on an enforced claim |
| `3`  | warning: `moved` or `at-risk` (re-anchorable / advisory) |
| `1`  | operational error |

Tune strictness with `--fail-on gating|warn|tamper|never`.

## How it works

Each claim carries a **bidirectional anchor**: a doc-side bundle (the documented sentence) and one or more code-side bundles (the code it describes). Each side bundles several redundant selectors against one file:

- the quoted text, matched fuzzily so it survives small edits and moves;
- its byte position, as a cheap hint;
- the enclosing syntax node, parsed with tree-sitter, so reformatting alone does not trip it;
- any literal value it mentions, so changing `MAX_ATTEMPTS = 5` to `50` flags the claim even when nothing else moves;
- an optional `path` or `glob` for coarse coverage, used to size blast radius.

On `hibi check`, Hibi re-finds each side in your current files and grades the result on two independent axes: **anchor resolution** per side (`unchanged` · `moved` · `changed` · `ambiguous` · `orphaned`), and, on behavioral claims, a **behavioral belief** (`unverified` · `at-risk` · `supported` · `refuted`). A verdict reads e.g. `doc:unchanged · code:changed · behavior:at-risk`. When the selectors agree, you get a confident verdict; when they disagree, Hibi asks you to re-verify instead of guessing. Verdicts are computed live and kept out of the store.

## What you can rely on

- **Deterministic.** No model runs in the check loop, so the same working tree yields the same verdicts every time. The optional semantic resolver advises and nothing more.
- **A flag means re-verify.** Hibi reports that the evidence under a claim moved. It never declares a doc wrong on its own.
- **Any file format.** Hibi treats docs as text, so Markdown, plain text, AsciiDoc, or anything else works without a per-format parser.
- **Offline and shallow-clone safe.** The anchor is its own baseline, so `check` reads your files, never git history.

## Extend it

Hibi finds drift through an out-of-process resolver protocol (JSONL-RPC over stdio). The built-in code-anchor resolver speaks the same contract, so you can add your own in any language. SDKs ship for [TypeScript](sdk/ts) and [Rust](sdk/rust). Resolvers stay off until you list them in `.claims/resolvers.json`:

```jsonc
// .claims/resolvers.json: opt in to the optional semantic advisor (it advises, it does not gate)
{ "resolvers": [
    { "name": "semantic-advisor", "command": "bun", "args": ["run", "resolvers/semantic-advisor.ts"] }
] }
```

## Use it with Claude Code

Hibi ships a [Claude Code](https://claude.com/claude-code) skill that teaches coding
agents to use it: set up the store in a fresh repo, record well-anchored claims, run
the check/diff/query loops, respond to flagged claims, and wire CI. The repo doubles
as a plugin marketplace, so installing takes two commands:

```
/plugin marketplace add npupko/hibi
/plugin install hibi-cli@hibi
```

Claude loads the skill when you ask it to work with hibi. You can also invoke it as
`/hibi-cli:hibi`. The source lives in [`plugins/hibi-cli`](plugins/hibi-cli).

## Develop

```sh
bun install
bun run build:grammars      # copy official tree-sitter wasm into grammars/
bun test                    # the full suite
bun run build               # single-file executable at dist/hibi
```

The data model lives once in Zod (`src/core/model.ts`). The JSON Schemas (`schemas/*.v1.json`) and SDK types come from it via `bun run build:schemas`.

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the dev setup, commit conventions, and PR expectations. Please follow our [Code of Conduct](CODE_OF_CONDUCT.md), and report security issues per [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE) © Nick Pupko

---

For the data model, verdict algorithm, and design rationale, read [`docs/PRD.md`](docs/PRD.md).
