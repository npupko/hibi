<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/npupko/hibi/main/assets/logo/hibi-wordmark-dark.png">
    <img alt="Hibi 日々" src="https://raw.githubusercontent.com/npupko/hibi/main/assets/logo/hibi-wordmark-transparent.png" width="300">
  </picture>
</p>

<p align="center"><em>Catch document sentences that are no longer true.</em></p>

<p align="center"><a href="https://hibi.mintlify.app"><strong>Documentation</strong></a></p>

<p align="center">
  <a href="https://github.com/npupko/hibi/actions/workflows/ci.yml"><img src="https://github.com/npupko/hibi/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://www.npmjs.com/package/@npupko/hibi"><img src="https://img.shields.io/npm/v/@npupko/hibi.svg" alt="npm version"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
  <!-- Uncomment once the first OpenSSF Scorecard run has published results:
  <a href="https://securityscorecards.dev/viewer/?uri=github.com/npupko/hibi"><img src="https://api.securityscorecards.dev/projects/github.com/npupko/hibi/badge" alt="OpenSSF Scorecard"></a>
  -->
</p>

Hibi tracks **claims**: checkable sentences in documents such as a README, `CLAUDE.md`, an ADR, or a pricing page. It reports which claims are stale, which documents cannot be trusted right now, which sentences contradict each other, and which sentences nothing could verify. Hibi runs nothing and fetches nothing; an agent collects evidence and submits it.

A claim has one of three kinds:

| kind | where its truth is observed | what makes it stale |
|------|----------------------------|---------------------|
| `code` | one or more code spans | an anchored span changed, moved out, or disappeared |
| `reference` | one span in another document | the span changed, or the judge says it no longer supports the sentence |
| `attested` | outside the repo (a dashboard, a person) | its `ttl` ran out since the last evidence or human attestation, or submitted evidence refutes it |

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
hibi init --instructions AGENTS.md   # create .claims/; add the check line to AGENTS.md
hibi audit --doc README.md           # one row per region: covered, or what kind of claim it is

# Record a claim of each kind
hibi record --doc README.md --doc-quote "Retries are capped at 5 attempts" \
  --kind code --code-file src/retry.ts --code-quote "MAX_ATTEMPTS = 5"
hibi record --doc README.md --doc-quote "Refunds follow the billing guide." \
  --kind reference --ref-file docs/billing.md --ref-quote "Refunds are issued within 14 days."
hibi record --doc docs/pricing.md --doc-quote "Pro costs $29/month." \
  --kind attested --ttl P90D --source stripe=https://dashboard.stripe.com/products/prod_123 \
  --instructions "Open the Stripe dashboard and report the Pro monthly price."

# Agents record in batches: a JSON array on stdin
echo '[{"doc":"README.md","docQuote":"Retries are capped at 5 attempts","kind":"code",
        "code":[{"file":"src/retry.ts","quote":"MAX_ATTEMPTS = 5"}]}]' | hibi record --from-file -

hibi check                      # every claim, the contradiction pass, and the worklist
hibi check --doc CLAUDE.md      # can this one document be trusted now?
hibi check --since origin/main  # what did this change invalidate?
hibi list --state pending       # the worklist: claims an agent should re-check

# Submit evidence an agent collected; the judge decides whether it supports the claim
echo '[{"claimId":"clm_…","evidence":"Pro: $29.00 / month","source":"stripe",
        "summary":"Stripe lists Pro at $29/month"}]' | hibi evidence --from-file - --by agent:opus

hibi attest <claim-id> --by human:alice@example.com --note "checked, still true"
hibi attest <claim-id> --by human:alice@example.com --ack conflict:<other-id> --note "different plans"
hibi reanchor <claim-id>        # re-resolve a claim and store the new baseline
hibi retire <claim-id>          # withdraw a claim
hibi supersede --from v1.md --to v2.md
hibi archive --doc old.md --successor new.md
```

Commands: `init`, `audit`, `record`, `check`, `evidence`, `attest`, `list`, `reanchor`, `retire`, `supersede`, `archive`, `schema`, `completions`, `version`. `hibi <cmd> --help` prints that command's options.

**Output is TTY-aware.** In a terminal you get a human-readable report; piped or in CI you get JSON.

| flag | output |
|------|--------|
| `--format human\|compact\|json\|json-pretty` | pick the shape (default: `human` on a TTY, `json` when piped) |
| `--json` | alias for `--format json` |
| `--explain` | add the detail tail to each verdict |
| `--no-hints` | drop the remediation menu (also `HIBI_ADVICE=0`) |
| `--color auto\|always\|never` | color control (also honors `NO_COLOR` / `FORCE_COLOR`) |

`HIBI_ASCII=1` switches to ASCII symbols. `hibi completions <zsh\|bash\|fish>` prints a shell completion script.

### Exit codes

| code | meaning |
|------|---------|
| `0`  | clean, or warnings only |
| `2`  | gating: a `changed` / `orphaned` / `ambiguous` span, a `refuted` judgment, or a contradiction on an enforced claim; `expired` also gates under `check --doc` |
| `1`  | operational error |

Warnings are `moved`, `review`, `pending`, `unavailable`, and `expired` in a whole-store check. Tune strictness with `--fail-on gating|warn|never`. New claims are enforced by default; `record --suggest` makes one advisory.

## How it works

Each claim has a doc side (the sentence) and, for `code` and `reference`, evidence sides. Each side stores selectors against one file: the quoted text with 48 characters of context, its character position, and on code sides the enclosing tree-sitter node hash and the literal inside the span, so changing `MAX_ATTEMPTS = 5` to `50` flags the claim. On `hibi check`, each side resolves to `unchanged`, `moved`, `changed`, `ambiguous`, or `orphaned`; a side in a sibling repo that is not checked out is `unavailable`.

A model judge (TypeSafe's Jev, reached over HTTP with `TYPESAFE_API_KEY`) answers three questions: whether a region is a claim and of which kind (`audit`), whether evidence supports a sentence (`reference` claims and `evidence`), and whether two sentences contradict each other (the contradiction pass). Every answer is cached in `.claims/judge/` by model and exact request, so a second run asks nothing and cannot flip a verdict. Without a key, an unanswered judgment is `pending`, a warning. Numbers, money, versions, and dates are compared in code, not by the model. One threshold in `.claims/config.json` decides when a judgment gates. Every rule that fires appears as a reason label in the verdict.

## What you can rely on

- **Same tree, same verdicts.** Anchor resolution is deterministic, and judge verdicts are cached and never re-asked.
- **A flag means re-verify.** Hibi reports that evidence moved, expired, was refuted, or conflicts. It never rewrites prose.
- **Nothing clears without evidence or a human.** An agent cannot vouch a claim fresh; judged evidence or a `human:` attestation can.
- **Any text format.** Markdown, plain text, AsciiDoc, and instruction files work without a per-format parser.

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

The data model lives once in Zod (`src/core/model.ts`). The JSON Schemas (`schemas/*.v4.json`) come from it via `bun run build:schemas`. The CLI reference in `docs/` and `plugins/` is generated from the option tables via `bun run build:cli-reference`.

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the dev setup, commit conventions, and PR expectations. Please follow our [Code of Conduct](CODE_OF_CONDUCT.md), and report security issues per [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE) © Nick Pupko

---

The design contract is [`design/SPEC.md`](design/SPEC.md).
