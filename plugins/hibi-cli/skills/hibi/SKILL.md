---
name: hibi
description: >-
  Set up and operate hibi, the deterministic CLI that flags documentation and
  AI-agent instruction files when the code they describe has changed. hibi
  tracks claims (a doc sentence anchored to a code span) in a committed
  .claims/ store and grades drift without running a model. Use this skill when
  the user wants to initialize hibi in a repo (`hibi init`), record claims, run
  `hibi check` (with `--doc`, `--since`, or `--overview`), `hibi list`, or
  `hibi coverage`, wire hibi into CI or a git hook, respond to a flagged claim
  (changed / orphaned / moved / ambiguous / expired / refuted) using its
  remediation menu, reanchor or retire a claim, or manage document lifecycle
  (supersede / archive). Trigger this skill even when the user does not say
  "hibi" but describes the problem: docs drifting out of sync with code, stale
  READMEs or AGENTS.md/CLAUDE.md files, "keep the docs honest when code
  changes", verifying a doc is still current before trusting it, or anything
  involving a `.claims/` store or a `HIBI:BEGIN` banner.
allowed-tools: >-
  Bash(hibi check:*), Bash(hibi list:*), Bash(hibi coverage:*),
  Bash(bunx hibi check:*), Bash(bunx hibi list:*), Bash(bunx hibi coverage:*)
---

# Using hibi

## Vocabulary

- **Claim**: one sentence in a document anchored to the code span it describes. One JSON file per claim under `.claims/`, committed.
- **Doc side / code side**: the two anchors of a claim. On `check`, each resolves to one of five **anchor states**: `unchanged`; `moved` (same text, new position); `changed` (text, structure, or a literal differs); `ambiguous` (several equal matches); `orphaned` (not found).
- **Enforcement**: `enforced` (default; may gate), `suggested` (`record --suggest`; advisory, never gates), `retired` (withdrawn by `hibi retire`).
- **Verifiers**: `--verifier kind:ref` commands, run only under `check --run-verifiers`; they set `behavior` to `supported` or `refuted`. Without a verifier there is no `behavior` field.
- **Gates**: an enforced claim gates on `changed`, `orphaned`, or `ambiguous` (either side), `expired` (ttl passed), or `refuted`. `moved` is a warning.

A flag means re-verify. hibi never edits prose.

## Invoking hibi

If `command -v hibi` succeeds, use `hibi`. In a Bun repo without a binary, use `bunx @npupko/hibi`. With no JS runtime: `curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | sh`.

Output is JSON when piped, human on a TTY (`--json` forces JSON). `--explain` adds evidence; `--no-hints` drops remediation. `hibi <cmd> --help` prints options without running the command.

## The five commands

**1. init** creates `.claims/`. Commit it.

```sh
hibi init
```

**2. record** anchors a sentence to code. Send a JSON array on stdin; keys mirror the flags in camelCase (`doc`, `docQuote`, `docRange`, `codeFile`, `codeQuote`, `codeRange`, `glob`, `suggest`, `verified`, `verifier`, `ttl`, `owner`). The batch is all or nothing. Quote the load-bearing token on the code side (the value, not the line). Set `verified: true` only after you confirmed the code backs the sentence.

```sh
echo '[{"doc":"README.md","docQuote":"Retries are capped at 5 attempts",
        "codeFile":"src/retry.ts","codeQuote":"MAX_ATTEMPTS = 5","verified":true}]' \
  | hibi record --from-file -
```

**3. check** resolves every claim against the working tree. `--doc <path>` scopes to one document, `--since <ref>` to files changed since a git ref, `--overview` prints a per-document table, `--write` stamps banners.

```sh
hibi check --since origin/main
```

**4. list** prints one row per claim. `--state all|gating|warning|clean|orphaned|suggested|stranded|duplicate`, `--path <p>` (either side), `--ids-only` for shell loops.

```sh
hibi list --path src/auth.ts
```

**5. reanchor** re-resolves a claim and stores the new baseline. A side that is not found is refused unless you pass a new span for it. `--suggest` lists candidate locations (read-only). `--doc <path> --doc-quote "…"` moves the claim to another document. `--dry-run` previews.

```sh
hibi reanchor asrt_1a2b3c4d --suggest
```

## JSON shapes

`check`:

```json
{ "ok": true, "action": "check", "schemaVersion": "v3", "exitCode": 2,
  "summary": { "total": 3, "gating": 1, "warning": 0, "clean": 2, "retired": 0, "expired": 0 },
  "verdicts": [{ "assertionId": "asrt_…", "doc": "unchanged", "code": "changed",
                 "expired": false, "gates": true,
                 "remediation": { "recommended": "update-claim",
                   "actions": [{ "id": "update-claim", "title": "…", "rationale": "…",
                                 "command": "hibi reanchor asrt_…" }] } }],
  "documents": [{ "path": "README.md", "lifecycle": "active", "suspect": [{ "propositionId": "prop_…", "status": "code:changed" }] }] }
```

`record`: `{ ok, action, schemaVersion, id, doc, code, enforcement, verified, warnings?, next }`. Batch: `{ ok, action, schemaVersion, batch: true, count, results: [{ id, doc, code, enforcement, warnings? }], next }`.

## When to run

- Before trusting an instruction file: `hibi check --doc CLAUDE.md`. Exit 2 means a sentence in it no longer matches the code.
- After a feature or refactor: `hibi check --since origin/main`. Fix the prose in the same change.
- To find doc sentences backed by no claim: `hibi coverage --doc README.md`. Uncovered regions get a claim or get removed.

## Verdict to action

Read `remediation.recommended`, find that action in `actions[]`, and run its `command`. When there is no `command`, do the work the `title` names, then run `hibi check` again.

| verdict | recommended | do |
|---|---|---|
| `code:changed` | `update-claim` | rewrite the sentence if wrong, then `hibi reanchor <id>` |
| `doc:changed` | `reverify-doc` | confirm the edited sentence against the code, then reanchor |
| both changed | `reconcile` | reconcile doc and code, then reanchor |
| `*:orphaned` | `reanchor` | `hibi reanchor <id> --suggest`, then reanchor with an explicit span, or `retire` |
| `*:moved`, `*:ambiguous` | `reanchor` | `hibi reanchor <id>` (wider span when ambiguous) |
| `behavior:refuted` | none | `fix-code` or `fix-claim`; never reanchor |
| `expired` | `reverify-and-rerecord` | re-verify, then re-record |

Never rewrite a sentence to silence a banner. Never delete a `.claims/` file; use `hibi retire <id>`.

## Exit codes

| exit | meaning |
|:---:|---|
| 0 | clean (`moved` warnings included, unless `--fail-on warn`) |
| 2 | gating: `changed`/`orphaned`/`ambiguous`, `expired`, or `refuted` on an enforced claim |
| 1 | operational error (no store, unknown flag or ref) |

`--fail-on gating|warn|never` moves the threshold.

## References

- `references/cli-reference.md`: every command, flag, and JSON shape.
- `references/cookbook.md`: six worked examples.
- `assets/hibi-ci.yml`: a GitHub Actions workflow.
