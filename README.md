<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/hibi-wordmark-dark.png">
    <img alt="Hibi 日々" src="assets/logo/hibi-wordmark-transparent.png" width="300">
  </picture>
</p>

<p align="center"><em>hibi (日々) — "day after day." Documentation kept honest continuously, so it never quietly goes stale.</em></p>

# Hibi 日々

A **deterministic, agent-facing CLI** (with a small reusable library core) that keeps a codebase's
documentation and AI-agent-instruction files from silently going stale — so automated agents never
read a **superseded or outdated** document and act on it as if it were current.

It tracks **claims** (assertions anchored to code), detects when they **drift** (the code changed) or
are **superseded** (a newer doc amended/replaced them), and **stamps lifecycle status into the
documents themselves** so no consumer can read a stale one as current.

> Full design rationale, data model, and normative algorithms: [`docs/PRD.md`](docs/PRD.md).

## Why it's trustworthy

- **Determinism is the product.** No model in the engine loop. The optional semantic tier *advises*;
  it never decides (§7.4, §11.1).
- **Suspect, not false.** A verdict means "the evidence moved — re-verify," never "the claim is false."
- **Precision over recall.** A tight, trustworthy suspect set: coarse anchors are never stale;
  grading uses thresholds with cross-selector corroboration; selector disagreement → re-verify, not
  hard-stale; a drifted claim is **never** reported `fresh`.
- **Universal by construction.** Documents are treated as text — any format works, no per-format parser.
- **Offline & shallow-clone safe.** The anchor *is* the baseline; `check` never reads git history.

## Install

```sh
# Prebuilt single-file executable (no runtime needed)
curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | sh

# Or, in a Bun/JS project
bun add hibi
```

## Quickstart

```sh
hibi init                       # create .claims/ (with a per-repo banner nonce)

# Record a claim: "Retries are capped at 5 attempts" anchored to the constant in code
hibi record \
  --doc README.md --text "Retries are capped at 5 attempts" \
  --file src/retry.ts --quote "MAX_ATTEMPTS = 5" --trust verified --owner alice

hibi check                      # verify all claims (exit 0 clean / 2 suspect / 3 moved)
hibi check --write              # also stamp status banners into affected documents
hibi diff --since origin/main   # the write-time loop: what did this change invalidate?
hibi query --path src/retry.ts  # before-edit: what claims cover this file?
hibi supersede --new v2.md --old v1.md --type supersedes
hibi status --doc README.md     # read-time gate: "is this current?"
```

JSON is the default output (the consumer is a machine); add `--pretty` for humans.

### Exit codes (§9)

| code | meaning |
|------|---------|
| `0`  | all clean |
| `2`  | suspect present (`stale` / `ghost` / `expired`) |
| `3`  | `moved`-only (re-anchorable warning) |
| `1`  | operational error |

Strictness is tunable with `--fail-on suspect|moved|tamper|never`.

## How it works (the short version)

A claim is a **Proposition** (timeless meaning) plus **Assertions** (verification instances). Each
Assertion carries a composite **Anchor** — a bundle of redundant selectors spanning the precision
spectrum:

- `text-quote` (W3C TextQuoteSelector — fuzzy, survives moves),
- `text-position` (a cheap hint),
- `ast-node` (tree-sitter, snapped to the enclosing named node — survives reformatting),
- `value` (an extracted literal, so a `5 → 50` change trips even when nothing else moves),
- `path` / `glob` (coarse — navigation & blast-radius only; **never** reported stale).

On `check`, the engine re-localizes each selector against the **current working tree** (the Anchor is
its own baseline — git is never on the verdict path), grades drift with thresholds, and **fuses
confidence from how many selectors agree**. Verdicts are recomputed live, never stored.

Status is made impossible to miss via a **universal, sentinel-delimited, idempotent banner** stamped
into the document itself (nonce-guarded, FNV-1a-checksummed, byte-stable, restores pristine bytes on
clear) — plus an optional machine-readable frontmatter status for markdown.

## Extending it (any language)

The single extension seam is an **out-of-process resolver protocol** — JSONL-RPC over stdio. A
resolver declares the anchor `kind`s it handles and answers `describe` / `resolve`. Built-in
code-anchor drift is itself a resolver behind this contract; third parties add more, in any language,
gated by a **default-deny manifest** (`.claims/resolvers.json`). Thin SDKs ship for
[TypeScript](sdk/ts) and [Rust](sdk/rust).

```jsonc
// .claims/resolvers.json — opt in to the Tier-3 semantic advisor (advises, never gates)
{ "resolvers": [
    { "name": "semantic-advisor", "command": "bun", "args": ["run", "resolvers/semantic-advisor.ts"] }
] }
```

## Develop

```sh
bun install
bun run build:grammars      # copy official tree-sitter wasm into grammars/
bun test                    # the full suite
bun run build               # single-file executable → dist/hibi
```

The canonical data model lives once in Zod (`src/core/model.ts`); JSON Schemas (`schemas/*.v1.json`)
and SDK types are generated from it (`bun run build:schemas`).
