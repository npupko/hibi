---
name: hibi
description: >-
  Set up and operate hibi — the deterministic CLI that flags documentation and
  AI-agent-instruction files when the code they describe has changed. hibi tracks
  "claims" (doc sentences anchored to specific code) in a committed .claims/ store
  and grades drift without running a model. Use this skill whenever the user wants
  to install or initialize hibi in a repo (`hibi init`), record/anchor a claim,
  run `hibi check` / `diff` / `query` / `status`, wire hibi into CI or a git hook,
  respond to a flagged claim (stale / ghost / moved / expired), or manage doc
  lifecycle (supersede / amend / retract / archive). Trigger this skill even when
  the user does not say "hibi" but describes the problem it solves: docs drifting
  out of sync with code, stale READMEs or AGENTS.md/CLAUDE.md files, "keep the docs
  honest when code changes", verifying a doc is still current before trusting it,
  or anything involving a `.claims/` store or a `HIBI:BEGIN` banner.
---

# Using hibi

hibi keeps docs and agent-instruction files from silently going stale. You record
**claims** — sentences in a doc that assert how the code behaves — and anchor each
to the exact code that backs it. When that code changes, `hibi check` flags the
claim so no human and no agent trusts a page that has fallen out of sync.

The whole point is captured in one rule, and most mistakes come from forgetting it:

> **A flag means "re-verify", not "the doc is wrong."** hibi reports that the
> evidence under a claim moved. *You* decide what that means. It never edits a
> doc's prose or guesses intent — it is deterministic and never runs a model in
> the check loop, so the same working tree yields the same verdicts every time.

Output is JSON by default (the consumer is a machine). Add `--pretty` when a human
needs to read it.

## Step 0 — How to invoke hibi in this repo

Figure this out once and reuse it; don't re-derive it per command.

1. `command -v hibi` succeeds → a prebuilt binary is on PATH. Use `hibi …`.
2. Otherwise, in a Bun/JS repo, add it as a dev dependency and run via Bun:
   `bun add -d @npupko/hibi`, then invoke `bunx hibi …`.
3. One-off, no install: `bunx @npupko/hibi …`.

The npm package is **Bun-targeted** (`bunx`, not `npx`). For a repo with no JS
runtime, install the zero-dependency binary instead:
`curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | sh`.

Throughout this doc, `hibi` stands for whichever form you settled on.

## Setting up a fresh repo

```sh
hibi init        # creates .claims/ with config.json (version + per-repo nonce)
                 # and .claims/.gitignore (ignores the regenerable cache/)
```

**Track `.claims/` in git** (don't add it to `.gitignore`; init already ignores
only the regenerable `cache/`). It is the source of truth — authored documents,
propositions, and assertions, one JSON file per record so merges stay scoped.
Verdicts are *not* stored; they are recomputed live on every `check`. Commit the
store alongside the docs and code it describes, as part of your normal commit flow,
so the claims travel with the change that affects them — but follow the usual rule
of not committing unless the user asked you to.

The `nonce` in `config.json` is a per-repo marker baked into every banner sentinel,
so a doc that merely quotes `HIBI:BEGIN` in an example can't be mistaken for a real
banner. Don't hand-edit it.

After init, seed claims for the docs that matter (next section), then optionally
wire CI (`assets/hibi-ci.yml` is a ready-to-drop-in GitHub Actions workflow using
the official action) or a pre-commit hook running `hibi diff --since HEAD`.

## Recording a claim well

This is the part that takes judgment, and it's where the value lives. A good claim
is a **falsifiable sentence about code behavior**, anchored to the **specific code
that would change if the sentence became false**.

```sh
hibi record \
  --doc README.md \
  --text "Retries are capped at 5 attempts" \
  --file src/retry.ts --quote "MAX_ATTEMPTS = 5" \
  --trust verified --owner alice
```

- **`--text`** is the claim — the timeless meaning. Propositions dedupe by the
  content fingerprint of this text, so phrase it as a standalone assertion, not
  "see below".
- **Anchor on the load-bearing token, as tightly as you can.** Pass `--quote` with
  the exact substring whose change would falsify the claim — the literal `5` in
  `MAX_ATTEMPTS = 5`, a function signature, an enum member. hibi captures redundant
  selectors from it (fuzzy text, byte position, the enclosing tree-sitter node, and
  the literal value), so a rename or reformat won't trip it but a real change will.
  **Tightness matters for the verdict, not just for locating:** quote the *value*
  (`--quote "5"`) rather than the whole line (`--quote "MAX_ATTEMPTS = 5"`). When you
  quote the whole line, the unchanged identifier (`MAX_ATTEMPTS`) keeps several
  selectors matching and a changed value often grades the softer `moved` instead of
  `stale` (both are flags, but `stale` is the hard "suspect" state CI gates on by
  default — see the verdict table). Anchoring a whole paragraph or an arbitrary line
  gives hibi nothing precise to grade.
- The quote must appear **verbatim** in the file at record time, or record fails.
  Alternatives: `--line <n>` (1-based) or `--start <byte> --end <byte>`.
- **`--trust`**: `verified` (you checked it — requires a precise anchor, and a ref
  that hibi fills in automatically from the current commit, so you don't normally
  pass `--ref`), `inferred` (default), or `assumed`.
- **`--coarse`** anchors to a file/path for blast-radius coverage only; coarse
  anchors are navigational and are *never* graded stale. Use a precise anchor
  whenever a specific line backs the claim.
- `--owner` defaults to the git-blame author of the anchored line; `--ttl` sets an
  expiry instant; `--ref` records the commit verified against.

When in doubt about whether something is anchorable, ask: *"If this code changed,
should a reader re-check this sentence?"* If yes, it's a claim — anchor it to that
code. If it's pure prose with no code behind it, don't record it.

## The everyday loops

`hibi check` (without `--write`) is **read-only and safe to run anytime** — use it
freely. `--write` and `diff --write` **modify your docs** (stamp/remove banners,
set markdown frontmatter status); run those only when you intend to update docs,
and review the resulting diff.

```sh
hibi query --path src/retry.ts   # BEFORE editing code: which claims cover this file?
hibi diff  --since origin/main   # AFTER a change: what did it invalidate? (CI / write-time)
hibi check                       # verify every claim; exit code is the gate
hibi check --write               # verify, and stamp banners into affected docs
hibi status --doc README.md      # read-time gate: is this one doc still current? (exit 2 if not)
```

The natural agent rhythm: **`query` before you edit code** (so you know which docs
your change touches), **`diff --since`** or **`check`** after, and **`status`**
before you trust a doc you're about to act on.

## Reading verdicts and exit codes

Every assertion gets a computed `state`. The exit code lets hibi gate CI and hooks
without parsing JSON:

| state     | exit | meaning                                            | what to do |
|-----------|:----:|----------------------------------------------------|------------|
| `fresh`   | 0    | anchors agree; the claim is still backed           | nothing |
| `moved`   | 3    | code was found but drifted (relocated/reformatted), or a value changed under otherwise-matching code | re-verify; if still true, refresh the anchor by re-recording against current code |
| `stale`   | 2    | a precise anchor changed meaningfully (e.g. the literal value changed) | **re-verify** — is the sentence still true? then fix and retire/replace the claim |
| `ghost`   | 2    | the anchored code/file is gone — nothing locatable | find where the behavior went (re-anchor) or update/retract the doc |
| `expired` | 2    | the claim's TTL passed                             | re-verify and re-record (resets the ref) or extend the ttl |

Exit `1` is an operational error (no store, bad flag).

**`stale` vs `moved` is about confidence, and both are flags.** hibi fuses its
selectors into a confidence score and bands it: high → `fresh`, medium → `moved`,
low → `stale`, none → `ghost`. So *how much* of the anchored code still matches
decides the band. A small edit to a value (`5` → `50`) keeps the quote nearly
identical and trips the value veto → `stale`; a larger change, or a change under an
otherwise-unchanged line, may keep enough matching to land in `moved`. Don't read
`moved` as "fine" — it means re-verify, just with less certainty than `stale`.

**Exit codes and CI — the part people get wrong.** Any drift is non-zero, so the
default `hibi check` already fails a CI step on *both* `moved` (3) and suspect (2);
only a fully clean tree returns 0. `--fail-on` doesn't decide whether CI fails — it
decides how hibi *classifies* the result: `--fail-on moved` escalates a moved-only
run from 3 to 2 (treat re-anchorable drift as a hard failure). Use it when you want
the build red on *any* drift and don't want to depend on a reader distinguishing 2
from 3. Note: as currently implemented, suspect drift exits 2 even under
`--fail-on never`/`tamper`, so those modes do **not** make a drifted run pass — if
you need a report-only run, read the JSON and ignore the exit code.

A `check`/`diff` report carries a `summary` (counts per state), a `verdicts` array
(each with `selectorScores` and `notes` explaining *why* — e.g. `"value veto —
anchored value changed"` or `"structural-only AST match (rename/whitespace)"`), and
a `documents` array with each doc's `suspect` list. Read the `notes` before deciding
what a flag means.

## Responding to a flag — the core workflow

A flag is the start of a decision, not an instruction to silence it. **Never edit a
doc just to make the banner go away.** Read the flagged sentence *and* the current
code, then pick one of three outcomes:

1. **The claim is still true; the code just moved or was reformatted** (`moved`,
   sometimes `stale` after a rename). Refresh the anchor: re-`record` the same claim
   against the current code so its baseline matches today's tree.

2. **The code changed on purpose, so the sentence is now wrong.** Fix the prose (or
   fix the code, if the doc is the spec). Then **retire the obsolete claim** — see
   the store-hygiene note below — because the old assertion keeps pointing at the
   old evidence.

3. **The claim is obsolete — the feature was removed or the doc replaced.** Use the
   lifecycle commands: `retract`, `archive`, or `supersede`.

### Store hygiene: retiring a claim (important)

`hibi record` **always appends a new assertion** — it only dedupes the *proposition*
by text. It does not update or remove the old one. So if you re-record a changed
claim and leave the original, **the original keeps flagging stale/ghost forever**.

To genuinely retire one obsolete claim, remove its assertion from the store:

```sh
hibi query --path <file>          # find the assertion: its id, proposition text, and anchor value
rm .claims/claims/<assertionId>.json   # the store is plain JSON, one file per record
hibi check                        # confirm it's clean
```

There is no `hibi` verb to retire a single assertion — you edit the store directly,
which is safe because it's plain per-record JSON. Removing an assertion can leave
its proposition orphaned in `.claims/propositions/` (e.g. the now-false
"…100 requests…" text). Orphans are **harmless to `check`** — it grades assertions,
not propositions — so leaving them is fine; remove the matching
`.claims/propositions/<id>.json` too only if you want a tidy store and no other
assertion references that proposition id.

For a whole document going out of service, prefer the lifecycle verbs over deleting
files — they leave an auditable banner/edge:

```sh
hibi supersede --new v2.md --old v1.md --type supersedes   # v1 fully replaced
hibi supersede --new v2.md --old v1.md --type amends --propositions prop_abc,prop_def
hibi retract  --doc draft.md      # author withdrew it
hibi archive  --doc old.md --successor new.md   # move it out of the read path (tombstone)
```

## Banners

With `--write`, hibi stamps a fenced banner at the top of each suspect doc
(`HIBI:BEGIN … HIBI:END`, carrying the repo nonce and a checksum) listing the
flagged claims, and — for markdown that already has frontmatter — sets a `status:`
field. When the claims become `fresh` again, the next `hibi check --write` removes
the banner. If someone hand-edited a banner body, `--fail-on tamper` makes hibi
refuse to overwrite it and fail instead.

## Going deeper

- **Hosted docs** — the full Hibi documentation lives at <https://npupko.mintlify.app>
  (concepts, CLI reference, resolvers, SDKs). Point users there for the canonical reference.
- **`references/cli-reference.md`** — every command, all flags, the full JSON output
  shapes (record / check / query), exit-code contract, and the `schema` command for
  emitting the data-model JSON Schemas. Read it when you need an exact flag or want
  to parse a report field you haven't seen.
- **`assets/hibi-ci.yml`** — a GitHub Actions workflow using the official `hibi`
  action; copy it to `.github/workflows/` and adjust `fail-on` / `since`.
- The optional **semantic resolver** (`.claims/resolvers.json`) only *advises*; it
  never changes a verdict or the exit code. Leave it off unless asked.
