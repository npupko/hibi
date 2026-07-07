---
name: hibi
description: >-
  Set up and operate hibi — the deterministic CLI that flags documentation and
  AI-agent-instruction files when the code they describe has changed. hibi tracks
  "claims" (doc sentences anchored to specific code) in a committed .claims/ store
  and grades drift without running a model. Use this skill whenever the user wants
  to install or initialize hibi in a repo (`hibi init`), record/anchor a claim,
  run `hibi check` / `diff` / `query` / `status` / `list` / `coverage`, wire hibi
  into CI or a git hook, respond to a flagged claim (changed / orphaned / moved /
  at-risk / expired) using its remediation menu, reanchor, retire, or suppress it
  (`hibi ignore`), or manage
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
4. **Onboarding a repo with docs but no claims?** Run the **grounding audit**: `hibi init`
   then `hibi coverage --doc README.md` shows which blocks no claim backs — ground the ones
   code supports, prune the rest (workflow 4 below).
5. **About to delete, rename, split, or merge a doc that has claims?** Enumerate them
   first — `hibi query --path <doc>` lists every claim anchored to it. Relocate the
   survivors and retire the rest *before* you `rm` the file ("Deleting, renaming, or
   consolidating a doc" below), so the deletion orphans nothing. The claims are content;
   migrate them in the same change as the prose.

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

## Core workflows

Each is *the moment → the command → reading the result → acting*. `hibi check`,
`diff`, `query`, `status`, `coverage`, and `list` (without `--write`) are **read-only and safe to
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

`--path` is **side-agnostic**: point it at a **doc** (`hibi query --path docs/design.md`)
and the hits are the claims that *live on* that doc (`side: "doc"`) — the exact set you
must relocate or retire before you delete, rename, or consolidate it (next section).

### 4. Onboard an existing repo fast — the grounding audit

hibi does **not** auto-extract claims from prose (it's deterministic — no model in the
loop). **You** do the audit; `coverage` gives you the deterministic worklist, and `check`
guards the result forever after.

```sh
hibi init
hibi coverage --doc README.md
```

`coverage` segments the doc into blocks and reports each as **covered** (a claim's doc
anchor lands in it) or **uncovered** — with a `coverageRatio` that climbs as you ground or
trim. It reports a *structural fact*; the judgment on each uncovered block is yours. Walk
the `regions` where `covered:false` and decide **ground-or-prune**:

- **Ground it** — the block states a checkable rule a code span backs (a normative rule,
  a specific value/identifier, a behavior). Read the code, then anchor doc-span *and*
  code-span together. Skip blocks that are pure rationale, opinion, or background — those
  aren't claims.
- **Prune it** — nothing in the code backs it, or the code contradicts it → it's
  ungrounded/stale prose. Remove or rewrite it (you're the editor — §"the agent edits"),
  and say what you cut and why. This is how the doc gets shorter and everything left is
  confirmed.

Author the grounded blocks in **one pass** — a JSON array of specs, no shell-quoting of
verbatim spans, validated and transactional (the whole batch rolls back on any bad item):

```sh
hibi record --from-file claims.json    # array of {doc, docQuote, codeFile, codeQuote, trust, …}; - = stdin
```

Each spec's keys mirror the flags in camelCase; `doc` + one doc-span key are required.
Choose `trust` honestly per block: `verified` (you confirmed the code backs it → gating)
or `inferred` (→ `suggested`, advisory). Propositions dedup by fingerprint, so the same
sentence from two files shares one meaning. Re-run `hibi coverage --doc README.md` to watch
the ratio climb, then `hibi check` to confirm clean. This is the low-friction path when an
agent grounds many docs at once.

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
| `*:orphaned` | `retire` | span gone from this file → if it **moved to another file**, `hibi reanchor <id> --doc <new>` (or `--code-file <new>`) relocates it; if truly deleted, retire/supersede |
| `behavior:refuted` | `null` | a verifier failed → fix code or fix claim — **never reanchor** (it clears the gate without fixing the behavior) |
| `behavior:at-risk` | `null` | the claim's evidence set changed → re-verify; still true? `hibi ignore --claim <id> --reason "…"` acknowledges it (auto-lapses on the next evidence change) |
| `+ expired` | (composed) | ttl passed → re-verify and re-record, on top of the above |

This counters the #1 mistake (silencing banners): you always re-verify, and you act on
a specific, deterministic command rather than rewriting prose to quiet a flag.

To triage a *category* rather than a single verdict, `hibi list --state <s>` filters the
store: `gating` (what's red), `warning`, `clean`, `orphaned` (claims with an orphaned doc
**or** code side — the ones to relocate or retire), and `suggested` (non-gating advisory
claims awaiting a code pin), plus `all`. Add `--ids-only` to any `list` (or `query`) to
get a bare, newline-delimited, deduped claim-id list with no JSON — feed it straight into
a shell loop (e.g. `for id in $(hibi list --state orphaned --ids-only); do …`).

## Store hygiene: retiring a claim

`hibi record` **always appends** a new assertion (it only dedupes the *proposition* by
content fingerprint). It never updates the old one, so a stale assertion left behind
keeps flagging forever. Three clean responses:

- The claim still holds, its anchor is just stale → **`hibi reanchor <claim-id>`**.
  Reanchoring is an **attestation**: pass `--ref <commit|pr>` when you actually
  re-verified the claim and trust is retained; without `--ref` the re-anchor still
  lands but `verified` trust downgrades to `inferred` (recorded and surfaced) — a bare
  reanchor can't silently clear a gate.
- The claim still holds but its sentence (or code) **moved to a different file** →
  **`hibi reanchor <claim-id> --doc <new-file> --doc-quote "…"`** (and/or `--code-file
  <new-file>`). This *relocates* the same claim — same id, owner, trust, history, and the
  other side untouched — so you never retire-and-recreate across a doc split, rename,
  extraction, or promotion. The old file is left intact as audit and no longer carries
  the claim, so deleting it orphans nothing.
- The claim is obsolete → **`hibi retire <claim-id>`** (flips `enforcement` to
  `retired`, keeps the audit trail, idempotent).

**`retire` is terminal, and a retired claim is inert.** `check` ignores it entirely — it
never gates, never warns, and never reports drift, even if the file it pointed at is
later deleted (a retired-then-orphaned record is harmless audit, not a problem to fix).
So **do not hand-delete `.claims/**`** to "clean up" retired records: there is nothing to
clean — the records are already invisible to every verdict, and they are your audit
trail. If you genuinely must remove a record (the rare exception — e.g. it captured a
secret), `git rm` the file in a commit so history retains the trail; never live-edit the
store. `retire` is the supported, reversible verb.

When **many** claims must move at once — a whole doc folded into another — don't loop
`reanchor` by hand. **`hibi relocate --from <old-doc> --to <new-doc>`** re-homes every
live (non-retired) claim stranded on `--from` onto `--to` in one pass: each claim whose
documented sentence appears **verbatim** in `--to` is moved (id, code side, trust,
history kept; only `documentId` changes), and any whose sentence is absent is reported in
`misses[]` for a manual `reanchor`/`retire` — never silently dropped. Supports
`--dry-run`. This is the batch tool for the consolidation playbook below.

For a whole document going out of service, use the lifecycle verbs (auditable
banner/edge):

```sh
hibi supersede --new v2.md --old v1.md --type supersedes
hibi supersede --new v2.md --old v1.md --type amends --propositions prop_abc,prop_def
hibi retract --doc draft.md           # author withdrew it
hibi archive  --doc old.md --successor new.md   # tombstone out of the read path
```

These record a **document-to-document edge** and flip lifecycle — they do **not** move
the claims anchored to the old doc. They now **report** any live claims left behind in
`strandedClaims: string[]`; when non-empty, `next` points at `hibi relocate --from <old>
--to <new>` (`retract` has no successor, so its `next` is `hibi relocate --from <doc>
--to <newDoc>  # or: hibi retire <id>`). The report is advisory — these verbs never
auto-move claims, so an empty `strandedClaims` confirms nothing was left behind. If the
claims should *follow* to the new doc (e.g. you promoted a draft and will delete the old
file), `relocate` them first; `supersede`/`archive`/`retract` only mark the document.

### Store health: `hibi doctor`

`hibi check` only grades *live* claims against the *current* tree, so dead state stays
invisible to it. **`hibi doctor`** is the periodic store-health sweep that surfaces what
`check` hides. It is **purely informational and always exits 0** — it never gates a build
or hook, so it's safe to run unconditionally. It reports, with `counts` and a
`healthy:boolean`:

- `orphanedAnchors[{claimId,side,path}]` — claims whose doc or code side no longer
  resolves.
- `suggestedNoCode[{claimId,docPath}]` — any live claim that landed `suggested` with no
  precise code pin (e.g. recorded doc-only, or its code anchor never resolved).
- `staleDocClaims[{claimId,docPath,lifecycle}]` — live claims sitting on a superseded /
  retracted / amended / archived doc (the ones lifecycle verbs left behind).
- `duplicatePropositions[{fingerprint,propositionIds,claimIds}]` — the same proposition
  recorded more than once.
- `rates` — the behavioral **flag-rate** (`behavioralFlagRate`) and doc-side drift
  rates (`docOrphanedRate`/`docMovedRate`/`docChangedRate`). Sustained >30% is the
  documented signal to tighten `behaviorScope` (flag-rate) or add inline IDs to
  high-severity claims (orphan rate).

`next` routes to the most pressing category (e.g. `hibi list --state orphaned`). Run it
when you want a full picture of accumulated cruft rather than the per-tree `check` view.

## Deleting, renaming, or consolidating a doc

The most-missed workflow. When a doc with claims is removed, renamed, split, or folded
into another file, **its claims are content** — migrate them in the *same change* as the
prose, in this order. **Relocate before you `rm`:**

1. **Batch-relocate** every live claim from the doomed doc onto its successor in one pass:

   ```sh
   hibi relocate --from <old-doc> --to <new-doc>     # add --dry-run to preview first
   ```

   `relocate` re-homes each claim whose current documented sentence appears **verbatim**
   in `<new-doc>` — keeping the claim id, code side, trust, and history, only moving the
   `documentId`. It never silently drops anything: a claim whose sentence is **absent**
   from the new doc lands in `misses[{claimId,reason}]` instead of `relocated[]`. The
   envelope is `{ ok, action:"relocate", from, to, relocated, misses, next }`.
2. **Resolve the misses by hand** — each is a sentence that *didn't* carry over verbatim
   (reworded, split, or cut). For each `misses[].claimId`, decide and run one:
   - the proposition survives under different wording in the new doc → `hibi reanchor
     <id> --doc <new-doc> --doc-quote "<span in new doc>"` (add `--enforce` to promote).
   - the proposition was dropped → `hibi retire <id>`.
3. **Mark the document** — `hibi supersede --new <new-doc> --old <old-doc> --type
   supersedes` (or `archive`/`retract`). This records the doc-to-doc edge **but moves no
   claims** — steps 1–2 are what move them. These lifecycle verbs **report** any live
   claims still on the old doc in `strandedClaims[]` and point `next` back at `hibi
   relocate` — they never auto-fix, so an empty `strandedClaims` is your confirmation the
   doc is clear to delete.
4. **Now delete the file.** Every claim already lives on the new doc or is retired, so the
   `rm` orphans nothing.

> **Don't author fresh claims for propositions that already exist on a doc you're
> retiring** — that duplicates the record and discards the original's id, trust, and
> history. `relocate` (or `reanchor`) the existing claim instead.

**Orphaned is not retired.** Skip step 2 and the claims become `*:orphaned`: a non-enforced
orphan happens not to gate, but it is **dead cruft pointing at a file that no longer
exists** — not the intended audit trail (that's what `retire` leaves). The only clean
terminal states are *relocated* (still true) or *retired* (obsolete) — never *orphaned by
a deletion you could have handled in the same change*.

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
- **Confirm the anchor is load-bearing.** After recording an `enforced` claim, perturb the
  anchored token (e.g. flip the value), run `hibi check` — it should exit **2** — then
  revert. A claim that *doesn't* flip the gate is anchored to the wrong span and protects
  nothing.
- **`--trust`**: `verified` (you checked it; requires a precise anchor + a ref hibi
  fills from the current commit) · `inferred` (default) · `assumed`.
- **Only `enforced` claims gate.** `--enforce` forces it; else the engine derives
  enforcement from trust + resolution. Ladder: `suggested` · `enforced` · `retired`.
  When a claim lands `suggested`, the JSON carries
  `warning:"recorded as suggested — won't gate the build; pass --enforce to make it
  gating"` — heed it and re-record (or `reanchor … --enforce`) if you meant it to gate.
- **Mind the dedup hint.** Propositions dedup by fingerprint, so recording a sentence
  that's already claimed reuses the existing proposition: the JSON then carries
  `existingClaims: string[]` and `next:"this proposition is already claimed — did you
  mean \`hibi reanchor\`?"`. That's the signal you were about to duplicate a claim —
  `reanchor` the existing id (relocating/re-pinning it) instead of authoring a second.
- **Behavioral claims** ("retries", "sorts ascending", …): the keyword heuristic
  classifies these automatically; `--behavioral` declares it explicitly and
  `--no-behavioral` opts out. Declaring a `--verifier` also marks the claim behavioral,
  so `--no-behavioral` plus a verifier is rejected as contradictory. Attach evidence
  with repeatable `--verifier kind:ref` (`kind` is an open string matched against
  runner-declared kinds; `command` has a built-in runner). Verifiers execute **only**
  under `hibi check --run-verifiers` — a deliberate supply-chain gate; plain
  `check`/`status` never spawn one. Pass → `supported`; fail → `refuted`.
- **Recording many at once?** Use `hibi record --from-file <p|->` with a JSON array of
  specs instead of one `record` per claim — it dodges shell-quoting of verbatim spans and
  validates every item before writing any. The lowest-friction path for grounding a doc
  set (see "Onboard an existing repo fast").
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
A **suppressed** at-risk (`hibi ignore`) drops out of exit codes entirely — even under
`--fail-on warn` — and shows as `suppressed: true` in the JSON until it lapses.

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

- **`references/cookbook.md`** — the workflows as full worked examples with real
  captured JSON (concise *and* `--explain`), the doc-consolidation playbook, and how to
  read each verdict.
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
