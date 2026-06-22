---
name: hibi
description: >-
  Set up and operate hibi — the deterministic CLI that flags documentation and
  AI-agent-instruction files when the code they describe has changed. hibi tracks
  "claims" (doc sentences anchored to specific code) in a committed .claims/ store
  and grades drift without running a model. Use this skill whenever the user wants
  to install or initialize hibi in a repo (`hibi init`), record/anchor a claim,
  run `hibi check` / `diff` / `query` / `status` / `list` / `suggest`, wire hibi
  into CI or a git hook, respond to a flagged claim (changed / orphaned / moved /
  at-risk / expired) using its remediation menu, reanchor or retire it, or manage
  doc lifecycle (supersede / amend / retract / archive). Trigger this skill even
  when the user does not say "hibi" but describes the problem it solves: docs
  drifting out of sync with code, stale READMEs or AGENTS.md/CLAUDE.md files,
  "keep the docs honest when code changes", verifying a doc is still current before
  trusting it, or anything involving a `.claims/` store or a `HIBI:BEGIN` banner.
allowed-tools: >-
  Bash(hibi check:*), Bash(hibi diff:*), Bash(hibi status:*),
  Bash(hibi query:*), Bash(hibi list:*),
  Bash(bunx hibi check:*), Bash(bunx hibi diff:*), Bash(bunx hibi status:*),
  Bash(bunx hibi query:*), Bash(bunx hibi list:*)
---

# Using hibi

## When to reach for hibi

Recognize these moments — each maps to one command. (Full worked examples with real
output in `references/cookbook.md`.)

1. **About to follow `CLAUDE.md` / `AGENTS.md` / a README?** Trust-check it first:
   `hibi status --doc CLAUDE.md`. If it gates, the instructions drifted from the code
   — don't blindly follow them.
2. **Just edited code?** `hibi diff --since origin/main` — it surfaces exactly which
   doc sentences your change invalidated, so the prose fix lands in the same PR.
3. **About to refactor a file?** `hibi query --path src/auth.ts` lists every doc claim
   anchored to it — the contracts you must not silently break.
4. **Onboarding a repo with docs but no claims?** `hibi init` then
   `hibi suggest --doc README.md` proposes anchorable claims so you don't hand-author them.

To triage at any time, `hibi list --state gating` is the lean "what's red?" view.

## The one rule

> **A flag means "re-verify", not "the doc is wrong."** hibi reports that the
> evidence under a claim moved. *You* decide what that means. It never edits prose or
> guesses intent — it is deterministic and **never runs a model in the check loop**,
> so the same working tree yields the same verdicts every time.

Most mistakes come from forgetting this and silencing a banner. Output is JSON by
default (the consumer is a machine); add `--pretty` when a human reads it.

## Step 0 — how to invoke hibi in this repo

Figure this out once and reuse it:

1. `command -v hibi` succeeds → a prebuilt binary is on PATH. Use `hibi …`.
2. Bun/JS repo, no binary: `bun add -d @npupko/hibi`, then `bunx hibi …`.
3. One-off: `bunx @npupko/hibi …`. (The npm package is **Bun-targeted** — `bunx`, not `npx`.)
4. No JS runtime: install the zero-dep binary —
   `curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | sh`.

Throughout, `hibi` stands for whichever form you settled on.

---

## The four workflows

Each is *the moment → the command → reading the result → acting*. `hibi check`,
`diff`, `query`, `status`, and `list` (without `--write`) are **read-only and safe to
run anytime**. `--write`, `record`, `reanchor`, `retire`, and the lifecycle verbs
modify the store or your docs — run those deliberately.

### 1. Trust-check before following instructions

```sh
hibi status --doc CLAUDE.md
```

`current: false` (exit 2) means a claim in that file gates — the code it describes
changed. The verdict tells you the shape: `doc: unchanged · code: changed` means the
instruction text is intact but the code drifted out from under it. **Re-verify before
acting on the instruction**, then follow its `remediation` (below). An explicit
invocation beats relying on auto-trigger here — run it before you trust a sensitive
instruction file.

### 2. Keep docs honest after a code change

```sh
hibi diff --since origin/main      # or --since HEAD before committing
```

`diff` evaluates only the claims whose anchored file changed since the ref (the
`changedFiles` are listed). Its `documents[]` array names exactly which docs your edit
broke; each gating `verdict` carries a `remediation` menu. Land the doc fix in the
same PR. Add `--explain` to see `changedEvidence` (*what* moved — e.g. the literal
value), or read the concise `changed` field for a one-line summary.

### 3. What does this code promise, before I touch it

```sh
hibi query --path src/auth.ts
```

Each hit is a claim anchored to that path, with its `assertion.id`, the documented
`proposition.textCache`, and which `side` matched. These are the contracts riding on
the file. Note the ids: after your edit you'll `reanchor` the ones still true and
`retire` the ones now wrong — no round-trip to rediscover them.

### 4. Onboard an existing repo fast

```sh
hibi init
hibi suggest --doc README.md
```

`suggest` writes one `suggested` (advisory, never-gating) doc-side record per
anchorable sentence, code side empty. For each one worth enforcing, pin its code and
promote it:

```sh
hibi reanchor <claim-id> --code-file src/retry.ts --code-quote "5"
hibi check        # confirm clean
```

---

## Responding to a flag — the verdict→action checklist

**A flag is the start of a decision, not an instruction to silence it. Never edit a
doc just to make the banner go away.** Drive the response off the machine-readable
`remediation` block that every drift verdict carries:

1. **Read `remediation.recommended`.** If it's a non-null action id, that's the
   unambiguous next step. Find that action in `remediation.actions[]`.
2. **If `recommended` is `null`, hibi can't infer intent** — read the flagged sentence
   *and* the current code, decide which action fits, and pick it from `actions[]`.
3. **Act on the chosen action:**
   - `effect: "deterministic"` → run its `command` verbatim (the claim id is
     pre-filled): `hibi reanchor <id>` (claim still true, anchor stale) or
     `hibi retire <id>` (claim obsolete).
   - `effect: "prose"` → there's no command; *you* do the work the `title`/`rationale`
     names (fix the code, rewrite the sentence, re-verify the behavior).
   - `applicability`: `auto` = safe to apply; `needs-review` = apply then check;
     `manual` = decide intent first.

The common shapes:

| verdict | recommended | what it means → do |
|---|---|---|
| `code:moved` / `doc:moved` | `reanchor` | span relocated, content intact → `hibi reanchor <id>` |
| `*:ambiguous` | `reanchor` | matches several places → reanchor to a unique span |
| `code:changed` | `null` | code changed on purpose? fix prose / retire. Still true? reanchor |
| `doc:changed` | `null` | prose edited — **meaning may have inverted**; re-read the span and re-verify |
| `doc:changed` + `code:changed` | `null` | both moved → **reconcile** doc vs code; don't auto-decide |
| `*:orphaned` | `retire` | span deleted → retire/supersede (a bare reanchor can't resolve it) |
| `behavior:refuted` | `null` | a verifier failed → fix code or fix claim — **never reanchor** (it clears the gate without fixing the behavior) |
| `behavior:at-risk` | `null` | reachable code changed → re-verify the behavior |
| `+ expired` | (composed) | ttl passed → re-verify and re-record, on top of the above |

This counters the #1 mistake (silencing banners): you always re-verify, and you act on
a specific, deterministic command rather than rewriting prose to quiet a flag.

## Store hygiene: retiring a claim

`hibi record` **always appends** a new assertion (it only dedupes the *proposition* by
content fingerprint). It never updates the old one, so a stale assertion left behind
keeps flagging forever. Two clean responses:

- The claim still holds, its anchor is just stale → **`hibi reanchor <claim-id>`**.
- The claim is obsolete → **`hibi retire <claim-id>`** (flips `enforcement` to
  `retired`, keeps the audit trail, idempotent). **Do not** hand-delete
  `.claims/claims/<id>.json` — `retire` is the supported, reversible verb.

For a whole document going out of service, use the lifecycle verbs (auditable
banner/edge):

```sh
hibi supersede --new v2.md --old v1.md --type supersedes
hibi supersede --new v2.md --old v1.md --type amends --propositions prop_abc,prop_def
hibi retract --doc draft.md           # author withdrew it
hibi archive  --doc old.md --successor new.md   # tombstone out of the read path
```

## Recording a claim well

A good claim is a **falsifiable sentence about code behavior**, anchored to the
**specific code that would change if the sentence became false**. hibi is span-first:
the doc side names a span in the document (that span's text *is* the claim), the code
side names spans in the code that backs it.

```sh
hibi record \
  --doc README.md --doc-quote "Retries are capped at 5 attempts" \
  --code-file src/retry.ts --code-quote "5" \
  --trust verified --owner alice
```

- **Anchor the code side on the load-bearing token, as tightly as you can** — quote
  the *value* (`--code-quote "5"`), not the whole line. A whole-line quote keeps
  several selectors matching, so a changed value can grade `code:moved` instead of
  `code:changed` — and only `changed`/`orphaned`/`ambiguous` gate.
- The quote must appear **verbatim** at record time, or `record` fails.
- **`--trust`**: `verified` (you checked it; requires a precise anchor + a ref hibi
  fills from the current commit) · `inferred` (default) · `assumed`.
- **Only `enforced` claims gate.** `--enforce` forces it; else the engine derives
  enforcement from trust + resolution. Ladder: `suggested` · `enforced` · `retired` ·
  `unanchored-legacy`.
- **Behavioral claims** (ordering/retry/complexity/…): `--claim-kind <k>` plus
  repeatable `--verifier kind:ref`. A passing verifier → `supported`; a failing one →
  `refuted`.
- **`--coarse` / `--glob`** anchor for blast-radius only; never graded as drift.

Ask: *"If this code changed, should a reader re-check this sentence?"* If yes, anchor
it. If it's pure prose with no code behind it, don't record it. Full flag tables and
JSON shapes live in `references/cli-reference.md`.

## Exit codes

The exit code gates CI and hooks without parsing JSON:

| exit | when | what to do |
|:----:|------|------------|
| 0 | clean | nothing |
| 3 | `moved` / `behavior:at-risk` — found but drifted; advisory, **never gates** | re-verify; if still true, `reanchor` |
| 2 | gating: on an **enforced** claim, `changed`/`orphaned`/`ambiguous` (either side), `expired`, or `behavior:refuted` | follow the verdict→action checklist |
| 1 | operational error (no store, bad flag) | fix the invocation |

Under the default, *any* drift is non-zero (3 and 2 both fail CI). `--fail-on` moves
the threshold: `warn` escalates warnings to 2; `tamper` also fails on a hand-edited
banner; `never` always exits 0 (report-only — read the JSON). Note **`3` is
advisory** — don't treat it as a hard failure unless you opted into `--fail-on warn`.

## Setting up & committing the store

`hibi init` creates `.claims/` (`config.json` with a version + per-repo `nonce`, and
`.gitignore` for the regenerable `cache/`). **Track `.claims/` in git** — it's the
source of truth (documents, propositions, assertions, one JSON file per record).
Verdicts are never stored; they recompute on every `check`. Commit the store with the
code/docs it describes, but follow the usual rule of not committing unless asked.

## Hooks: status-on-start, diff-on-stop (pattern, not shipped config)

Wire hibi into your agent loop without shipping opinionated settings:

- **SessionStart** → `hibi status --doc CLAUDE.md` (or your instruction file). If it
  gates, the agent knows up front not to trust drifted instructions (workflow 1).
- **Stop / post-edit** → `hibi diff --since origin/main`. Surfaces docs the session's
  edits invalidated so they're fixed before the work is considered done (workflow 2).

Configure these in your own hook settings (e.g. a Claude Code `SessionStart` / `Stop`
hook running the command). hibi deliberately ships no installable hook config — you
own that policy.

## Going deeper

- **`references/cookbook.md`** — the four workflows as full worked examples with real
  captured JSON (concise *and* `--explain`) and how to read each verdict.
- **`references/cli-reference.md`** — every command, all flags, the concise vs
  `--explain` JSON shapes, the `remediation` block, `schemaVersion`, `--no-hints` /
  `HIBI_ADVICE`, exit-code edge cases, and store layout.
- **`assets/hibi-ci.yml`** — a GitHub Actions workflow using the official `hibi`
  action; copy to `.github/workflows/` and adjust `fail-on` / `since`.
- **Hosted docs** — <https://npupko.mintlify.app> (concepts, CLI reference, resolvers,
  SDKs) — the canonical human reference.
- **Resolvers** speak a JSONL-RPC protocol (`describe` | `resolve` | `verify`); the
  optional semantic resolver only *advises* — it never changes a verdict or exit code.
  Leave it off unless asked.
