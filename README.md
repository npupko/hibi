<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/hibi-wordmark-dark.png">
    <img alt="Hibi 日々" src="assets/logo/hibi-wordmark-transparent.png" width="300">
  </picture>
</p>

<p align="center"><em>Catch documentation that no longer matches your code.</em></p>

Hibi tracks **claims**: sentences in your docs and AI-agent instructions that assert how the code behaves. You anchor each claim to the code it describes. When that code changes, `hibi check` flags the claim and can stamp a status banner into the doc, so no reader and no agent acts on a page that has fallen out of sync with the source.

Run it in CI, in a git hook, or as a pre-edit lookup an agent makes before it trusts a doc.

## Install

```sh
# Prebuilt single-file executable (no runtime needed)
curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | sh

# Or, in a Bun/JS project
bun add hibi
```

## Quick start

```sh
hibi init                       # create .claims/ (with a per-repo banner nonce)

# Record a claim and anchor it to the constant that backs it
hibi record \
  --doc README.md --text "Retries are capped at 5 attempts" \
  --file src/retry.ts --quote "MAX_ATTEMPTS = 5" --trust verified --owner alice

hibi check                      # verify every claim
hibi check --write              # verify, and stamp status banners into affected docs
hibi diff --since origin/main   # what did this change invalidate?
hibi query --path src/retry.ts  # before editing: which claims cover this file?
hibi supersede --new v2.md --old v1.md --type supersedes
hibi status --doc README.md     # is this doc still current?
```

Output is JSON by default. Add `--pretty` for human reading.

### Exit codes

| code | meaning |
|------|---------|
| `0`  | all clean |
| `2`  | suspect present (`stale` / `ghost` / `expired`) |
| `3`  | `moved`-only (re-anchorable warning) |
| `1`  | operational error |

Tune strictness with `--fail-on suspect|moved|tamper|never`.

## How it works

Each claim carries several redundant anchors to the code it describes:

- the quoted code text, matched fuzzily so it survives small edits and moves;
- its byte position, as a cheap hint;
- the enclosing syntax node, parsed with tree-sitter, so reformatting alone does not trip it;
- any literal value it mentions, so changing `MAX_ATTEMPTS = 5` to `50` flags the claim even when nothing else moves;
- an optional `path` or `glob` for coarse coverage, used to size blast radius.

On `hibi check`, Hibi re-finds each anchor in your current files and grades how far it moved. When the anchors agree, you get a confident verdict; when they disagree, Hibi asks you to re-verify instead of guessing. Verdicts are computed live and kept out of the store.

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

## Develop

```sh
bun install
bun run build:grammars      # copy official tree-sitter wasm into grammars/
bun test                    # the full suite
bun run build               # single-file executable at dist/hibi
```

The data model lives once in Zod (`src/core/model.ts`). The JSON Schemas (`schemas/*.v1.json`) and SDK types come from it via `bun run build:schemas`.

---

For the data model, verdict algorithm, and design rationale, read [`docs/PRD.md`](docs/PRD.md).
