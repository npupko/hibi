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

Hibi tracks **claims**: a sentence in a doc or an AI-agent instruction file, anchored to the code it describes. When either side changes, `hibi check` flags the claim and can stamp a status banner into the doc, so a reader without hibi still sees the flag.

Run it in CI, in a git hook, or as the check an agent makes before it trusts a doc.

## Install

```sh
# Prebuilt single-file executable (no runtime needed)
curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | sh

# Or, in a Bun/JS project
bun add @npupko/hibi        # then: bunx hibi …
bunx @npupko/hibi --help    # or run it once without installing
```

## Quick start

```sh
hibi init                       # create .claims/ (with a per-repo banner nonce)

# Record a claim: anchor the doc sentence to the constant that backs it
hibi record \
  --doc README.md --doc-quote "Retries are capped at 5 attempts" \
  --code-file src/retry.ts --code-quote "MAX_ATTEMPTS = 5" --verified --owner alice

# Agents record in batches: a JSON array on stdin, keys mirror the flags in camelCase
echo '[{"doc":"README.md","docQuote":"Retries are capped at 5 attempts",
        "codeFile":"src/retry.ts","codeQuote":"MAX_ATTEMPTS = 5","verified":true}]' \
  | hibi record --from-file -

hibi check                      # verify every claim
hibi check --write              # verify, and stamp status banners into suspect docs
hibi check --doc CLAUDE.md      # is this one doc still current?
hibi check --since origin/main  # what did this change invalidate?
hibi check --overview           # per-document table
hibi list --path src/retry.ts   # before editing: which claims depend on this file?
hibi list --state gating        # one row per gating claim
hibi coverage --doc README.md   # which sentences have no claim?
hibi reanchor <claim-id>        # re-resolve a claim and store the new baseline
hibi reanchor <claim-id> --suggest   # candidate locations for a lost span
hibi retire <claim-id>          # withdraw a claim
hibi supersede --from v1.md --to v2.md   # replace a doc and relocate its claims
hibi archive --doc old.md --successor new.md
```

Twelve commands: `init`, `record`, `check`, `list`, `coverage`, `reanchor`, `retire`, `supersede`, `archive`, `schema`, `completions`, `version`. `hibi <cmd> --help` prints that command's options.

**Output is TTY-aware.** In a terminal you get a human-readable report; piped or in CI you get JSON.

| flag | output |
|------|--------|
| `--format human\|compact\|json\|json-pretty` | pick the shape (default: `human` on a TTY, `json` when piped) |
| `--json` | alias for `--format json` |
| `--explain` | add the evidence tail to each verdict |
| `--no-hints` | drop the remediation menu (also `HIBI_ADVICE=0`) |
| `--color auto\|always\|never` | color control (also honors `NO_COLOR` / `FORCE_COLOR`) |

`HIBI_ASCII=1` switches to ASCII symbols. `hibi completions <zsh\|bash\|fish>` prints a shell completion script.

### Exit codes

| code | meaning |
|------|---------|
| `0`  | clean, or only `moved` warnings |
| `2`  | gating: `changed` / `orphaned` / `ambiguous` / `expired` / `refuted` on an enforced claim |
| `1`  | operational error |

Tune strictness with `--fail-on gating|warn|never`. New claims are enforced by default; `record --suggest` makes one advisory.

## How it works

Each claim has a doc side (the documented sentence) and one or more code sides. Each side stores several selectors against one file:

- the quoted text with 48 characters of context, located exactly or by a fuzzy match within a fixed error budget;
- its character position, as a locate bias and the `moved` tiebreaker;
- on the code side, the enclosing tree-sitter node with a two-tier hash, used for reason labels;
- on the code side, the literal inside the quoted span, so changing `MAX_ATTEMPTS = 5` to `50` flags the claim;
- an optional `--glob` for coarse coverage, used for navigation only.

On `hibi check`, each side resolves through an ordered cascade to one of five states: `unchanged`, `moved`, `changed`, `ambiguous`, `orphaned`. A claim with a `--verifier` also gets a `behavior` of `supported` or `refuted` under `check --run-verifiers`. Every suspect verdict carries a remediation menu with the next command filled in. Verdicts are computed live and never stored.

## What you can rely on

- **Deterministic.** No model runs in the check loop. The same working tree yields the same verdicts.
- **A flag means re-verify.** Hibi reports that the text or code under a claim moved. It never declares a doc wrong on its own.
- **Any file format.** Docs are text, so Markdown, plain text, AsciiDoc, and instruction files work without a per-format parser.
- **Offline and shallow-clone safe.** The anchor is its own baseline, so `check` reads your files, not git history.

## Extend it

The built-in drift resolver runs in-process. External resolvers run out-of-process over JSONL-RPC on stdio and can grade new anchor kinds, run verifiers, or attach advisories. They stay off until listed in `.claims/resolvers.json`. Write one in TypeScript with the SDK export:

```ts
import { serveResolver } from "@npupko/hibi/resolver";
```

See the [resolver docs](https://npupko.mintlify.app/resolvers) for the protocol and the `override` flag.

## Use it with Claude Code

Hibi ships a [Claude Code](https://claude.com/claude-code) skill that teaches coding agents to record claims, run `check`, and act on the remediation menu. The repo doubles as a plugin marketplace:

```
/plugin marketplace add npupko/hibi
/plugin install hibi-cli@hibi
```

Claude loads the skill when you ask it to work with hibi. You can also invoke it as `/hibi-cli:hibi`. The source lives in [`plugins/hibi-cli`](plugins/hibi-cli).

## Develop

```sh
bun install
bun run build:grammars      # copy official tree-sitter wasm into grammars/
bun test                    # the full suite
bun run build               # single-file executable at dist/hibi
```

The data model lives once in Zod (`src/core/model.ts`). The JSON Schemas (`schemas/*.v3.json`) come from it via `bun run build:schemas`. The CLI reference in `docs/` and `plugins/` is generated from the option tables via `bun run build:cli-reference`.

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the dev setup, commit conventions, and PR expectations. Please follow our [Code of Conduct](CODE_OF_CONDUCT.md), and report security issues per [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE) © Nick Pupko

---

For the original design rationale, read [`design/PRD-historical.md`](design/PRD-historical.md). The current simplification decisions are in [`design/ADR-004-simplification.md`](design/ADR-004-simplification.md).
