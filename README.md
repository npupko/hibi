<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/npupko/hibi/main/assets/logo/hibi-wordmark-dark.png">
    <img alt="Hibi 日々" src="https://raw.githubusercontent.com/npupko/hibi/main/assets/logo/hibi-wordmark-transparent.png" width="300">
  </picture>
</p>

<p align="center"><em>Find the sentences in your documents that are no longer true.</em></p>

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
| `code` | one or more code spans | an anchored span changed, disappeared, or matches several places |
| `reference` | one span in another document | the span changed, or the judge finds it no longer supports the sentence |
| `attested` | outside the repo (a dashboard, a vendor page, a person) | its `ttl` ran out since the last judged evidence or human vouch, or submitted evidence refutes it |

## Install

```sh
# Prebuilt single-file executable (no runtime needed)
curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | sh

# Or, in a Bun/JS project
bun add @npupko/hibi        # then: bunx hibi …
bunx @npupko/hibi --help    # or run it once without installing
```

The judge reads `TYPESAFE_API_KEY`. Without it, everything else works and uncached judgments are `pending`, a warning. A judge error (an answer hibi cannot use) is `pending` too; it never aborts a run.

## Quick start

```sh
hibi init --instructions AGENTS.md   # create .claims/; add the check line to AGENTS.md
hibi audit --doc README.md           # one row per region: covered, or what kind of claim it could be

# Record a claim of each kind
hibi record --doc README.md --doc-quote "Retries are capped at 5 attempts." \
  --kind code --code-file src/retry.ts --code-quote "MAX_ATTEMPTS = 5" --by agent:opus
hibi record --doc README.md --doc-quote "Refunds are issued within two weeks of a request." \
  --kind reference --ref-file docs/billing.md --ref-quote "Refunds are issued within 14 days." --by agent:opus
hibi record --doc docs/pricing.md --doc-quote 'The Pro plan costs $29 per month.' \
  --kind attested --ttl P90D --source stripe=https://dashboard.stripe.com/products/prod_123 \
  --instructions "Open the Stripe dashboard and report the Pro monthly price." --by human:alice@example.com

hibi check                      # every claim, the contradiction pass, and the worklist
hibi check --doc CLAUDE.md      # can this one document be trusted now? (exit 1 if it has no claims)
hibi check --since origin/main  # what did this change invalidate?
hibi list --state pending       # the worklist: claims an agent should re-check

# Submit evidence an agent collected; the judge decides whether it supports the claim
echo '[{"claimId":"clm_…","evidence":"Pro plan: $29.00 per month","source":"stripe",
        "summary":"Stripe lists Pro at $29/month"}]' | hibi evidence --from-file - --by agent:opus

hibi attest <claim-id> --by human:alice@example.com --note "checked, still true"
hibi attest <claim-id> --by human:alice@example.com --ack conflict:<other-id> --note "different plans"
```

`--by` is required on `record` and `attest`; there is no default actor. Agents record in batches with `hibi record --from-file - --by agent:<name>` (a JSON array on stdin, all or nothing; an item's own `by` overrides `--by`). `hibi evidence` accepts `attested` claims only; a `reference` claim is judged from the referenced document. Paths are stored as POSIX paths relative to the anchor root, so `./README.md` and `README.md` are the same document.

Commands: `init`, `record`, `check`, `list`, `audit`, `reanchor`, `retire`, `supersede`, `archive`, `evidence`, `attest`, `schema`, `completions`, `version`. `hibi <cmd> --help` prints that command's options.

Output is human on a terminal and JSON when piped. `--format human|compact|json|json-pretty`, `--json`, `--explain` (detail tail), `--no-hints` (drop the remediation menu), and `--color auto|always|never` override it.

### Exit codes

| code | meaning |
|------|---------|
| `0`  | clean, or warnings only |
| `2`  | gating: a `changed` / `orphaned` / `ambiguous` side, a `refuted` judgment, or a contradiction on an enforced claim; `expired` also gates under `check --doc`; under `--fail-on warn`, any warning |
| `1`  | operational error, including `check --doc` on a path with no registered claims |

Warnings are `moved`, `review`, `pending`, `unavailable`, and `expired` in a whole-store check. `--fail-on gating|warn|never` sets when to exit 2. `record --suggest` makes a claim advisory.

## How it works

Each claim has a doc side (the sentence) and, for `code` and `reference`, evidence sides. Each side stores selectors against one file: the quoted text with 48 characters of context, its position, and on code sides the enclosing tree-sitter node hash and the literal inside the span, so changing `MAX_ATTEMPTS = 5` to `50` flags the claim. The tree-sitter grammars are embedded in the binary; `--no-ast` skips them, and a language without a grammar gets text selectors only. On `check`, each side resolves to `unchanged`, `moved`, `changed`, `ambiguous`, or `orphaned`; a side in a sibling repo that is not checked out is `unavailable`.

A model judge (TypeSafe's Jev, over HTTP) answers three questions: what kind of claim a region is (`audit`), whether evidence supports a sentence (`reference` claims and `evidence`), and whether two sentences contradict each other (the contradiction pass). Every answer is cached in `.claims/judge/` by model and exact request, so a second run asks nothing and cannot flip a verdict. Numbers, money, versions, and dates are compared in code. One threshold, `judge.threshold` in `.claims/config.json`, decides when a judgment gates. Every rule that fires is a reason label in the output.

Attestations are append-only and name an actor (`human:`, `agent:`, `process:`). A claim's `ttl` counts from the newest judged evidence or human vouch; an agent vouch is recorded and does not count.

## What you can rely on

- **Same tree, same verdicts.** Anchor resolution is deterministic, and judge verdicts are cached and never re-asked.
- **A flag means re-verify.** Hibi reports that evidence moved, expired, was refuted, or conflicts. It never rewrites prose.
- **Nothing clears without evidence or a human.** An agent cannot vouch a claim fresh.
- **Any text format.** Markdown, plain text, AsciiDoc, and instruction files work without a per-format parser.

## Use it with Claude Code

Hibi ships a [Claude Code](https://claude.com/claude-code) skill that teaches coding agents the loop: check a document before trusting it, audit and record, work the worklist, and submit evidence. The repo doubles as a plugin marketplace:

```
/plugin marketplace add npupko/hibi
/plugin install hibi-cli@hibi
```

Invoke it as `/hibi-cli:hibi`, or let Claude load it when you ask about hibi. The source lives in [`plugins/hibi-cli`](plugins/hibi-cli).

## Develop

`bun install`, `bun run build:grammars`, then `bun test` (the judge is a fake; no key needed). `bun run build` writes a single-file executable to `dist/hibi`. Details are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the dev setup, commit conventions, and PR expectations. Please follow our [Code of Conduct](CODE_OF_CONDUCT.md), and report security issues per [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE) © Nick Pupko

---

The design contract is [`design/SPEC.md`](design/SPEC.md).
