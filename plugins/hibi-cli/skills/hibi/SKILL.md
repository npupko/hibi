---
name: hibi
description: >-
  Operate hibi, the CLI that tracks checkable sentences (claims) in documents
  and reports which are stale, refuted, expired, or contradicted. Use before
  trusting CLAUDE.md, AGENTS.md, or any doc; after a feature; when docs or a
  knowledge repo go stale; for process, marketing, or decision docs that drift; for
  contradictions between docs; for anything about claims or .claims/.
allowed-tools: >-
  Bash(hibi check:*), Bash(hibi list:*), Bash(hibi audit:*),
  Bash(bunx @npupko/hibi check:*), Bash(bunx @npupko/hibi list:*),
  Bash(bunx @npupko/hibi audit:*)
---

# hibi

A claim is one document region (sentence, list item, table row) that asserts something checkable. hibi never edits prose, fetches, or runs commands.

## Vocabulary

| kind | evidence side | stale when |
|---|---|---|
| `code` | code spans | an anchor `changed`, `orphaned`, `ambiguous` |
| `reference` | one span in another doc | an anchor change, or the judge finds no support |
| `attested` | none; optional `sources`, `procedure`; `ttl` required | ttl elapsed, or evidence refutes it |

Anchor states: `unchanged`, `moved`, `changed`, `ambiguous`, `orphaned`; evidence also `unavailable`. Judge: `supported`, `refuted`, `review` (unsure, or evidence did not address it), `pending` (judge not reached, or its answer unusable).

Attestations are append-only `{by, at, basis: vouch|evidence|ack, note}`; actors are `human:<id>`, `agent:<name>`, `process:<name>`. Counting rule: ttl runs from the newest `evidence` attestation or `human:` vouch. Agent and process vouches and acks never count. A claim of any kind with a ttl and nothing counting is `expired`.

`check` lists a `worklist`: claims that are `expired`; reference or attested claims in `review` or `pending`; attested claims with a `procedure` or `sources` and no judged evidence (`never-checked`). `list --state pending` is the same set.

## Invoking

Use `hibi`, else `bunx @npupko/hibi`. JSON when piped; `--json` forces it. Every flag: `references/cli-reference.md`. The judge needs `TYPESAFE_API_KEY`; without it uncached judgments are `pending`, a warning, never a failure. Verdicts cache in `.claims/judge/`; commit `.claims/` whole. Paths are normalized relative to the repo root; evidence files must stay inside it or their alias directory.

## The loop

**a. Before trusting a doc:** `hibi check --doc <path>`. Exit 2: a claim in it gates; do not rely on its gating sentences. Exit 1 with `no claims are registered for <path>`: audit it.

**b. After a change:** `hibi check --since origin/main`. Fix prose in the same change.

**c. Audit:** `hibi audit --doc <path>`, one row per region. `claimId` set: covered. Else `kind`: `code`/`reference`/`attested` → record; `unverifiable` → rewrite as attributed opinion or remove; `not-a-claim` → nothing; `unsure` → decide from `probabilities`; `pending` → no judge. Never record an `unverifiable` region.

**d. Record** (stdin array, all or nothing):

```sh
hibi record --from-file - <<'EOF'
[{"doc":"docs/pricing.md","docQuote":"Pro costs $29/month.","kind":"attested","ttl":"P90D",
  "owner":"human:alice@example.com","by":"agent:claude",
  "sources":[{"id":"stripe","resource":"https://dashboard.stripe.com/products/prod_123"}],
  "procedure":{"instructions":"Open the Stripe Pro product and report its monthly price.",
    "steps":[{"id":"price","command":"stripe prices list --product prod_123 --json"}]}}]
EOF
```

| key | rule |
|---|---|
| `doc`, `kind` | required |
| `docQuote` / `docRange` | exactly one; `L3:L3` or offsets `100:130` |
| `code` | code only: `[{file, quote or range}]` |
| `ref` | reference only: `{file, quote or range}` |
| `sources` | `[{id?, resource, title?}]`, never fetched |
| `procedure` | `{instructions, steps?}`; steps are hints |
| `ttl` | ISO 8601 (`P90D`); required on attested |
| `owner` | actor to contact on expiry |
| `by` | required (here or `--by`); use `agent:<name>` |
| `suggest` | `true`: never gates |

Quote the load-bearing token, not the whole line. `file` may be `alias:path` via `config.repos`. Agent-recorded claims with a ttl start expired.

**e. Worklist:** `hibi check --json` → `worklist[]: {claimId, sentence, kind, document, procedure, sources, owner, lastEvidence?, reason}`. Follow `procedure.instructions` with your tools, then:

```sh
hibi evidence --from-file - --by agent:claude <<'EOF'
[{"claimId":"clm_…","evidence":"Pro plan: $29.00 / month","source":"stripe","summary":"Stripe lists Pro at $29/month"}]
EOF
```

`evidence`: observed text, at most 32,000 characters, never secrets; only its hash is stored. `summary`: one stored line. `--by` is required unless every item has `by`. Attested claims only. `results[].judge`: `supported` appends an evidence attestation and resets ttl; `refuted` gates; `review` needs more pointed evidence; `pending` writes nothing. Exit 0 regardless.

**f. Attest:** `hibi attest <id> --by <actor> --note "…"`. A `human:` vouch resets ttl; an agent vouch returns `countsTowardTtl: false`. Never vouch as `human:`; ask the `owner`. `--ack conflict:<otherId>` or `--ack review` acknowledges a finding; it needs `--note` and never resets ttl. `--ack review` fails when there is no current supports verdict.

**g. Verdicts.**

| enforced claim | `check` | `check --doc` |
|---|---|---|
| `changed`, `orphaned`, `ambiguous`, `refuted`, conflict | gate | gate |
| `expired` | warn | gate |
| `moved`, `review`, `pending`, `unavailable` | warn | warn |

Exit 0 clean or warnings, 2 gating (any warning under `--fail-on warn`), 1 error.

`check` verdict: `{claimId, kind, doc, evidence?, judge?, expired, conflicts[], gates, warns, changed?, reasons[], remediation}`. `reasons[]` names every rule applied. `remediation.actions[].command` runs once you fill its `<…>` placeholders.

| flag | do |
|---|---|
| `changed` | fix the sentence if wrong, then `hibi reanchor <id>` |
| `orphaned` | `hibi reanchor <id> --suggest`, reanchor with a span, or retire |
| `refuted` | fix sentence or evidence; never reanchor |
| conflict | rewrite the wrong sentence, or ack |
| `expired` | submit evidence (attested), or ask the owner |

Never rewrite a sentence to silence a flag. Never delete a `.claims/` file; use `hibi retire <id>`.

Recipes: `references/cookbook.md`. CI: `assets/hibi-ci.yml`.
