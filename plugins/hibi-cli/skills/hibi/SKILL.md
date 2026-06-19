---
name: hibi
description: >-
  Set up and operate hibi — the deterministic CLI that flags documentation and
  AI-agent-instruction files when the code they describe has changed. hibi tracks
  "claims" (doc sentences anchored to specific code) in a committed .claims/ store
  and grades drift without running a model. Use this skill whenever the user wants
  to install or initialize hibi in a repo (`hibi init`), record/anchor a claim,
  run `hibi check` / `diff` / `query` / `status` / `suggest`, wire hibi into CI or a
  git hook, respond to a flagged claim (changed / orphaned / moved / at-risk /
  expired), reanchor it, or manage doc lifecycle (supersede / amend / retract /
  archive). Trigger this skill even when
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

hibi is **span-first**: the doc side names a span *in the document* (that span's
text *is* the claim), and the code side names zero or more spans in the code that
backs it. The Anchor is bidirectional — `{ doc, code[] }`.

```sh
hibi record \
  --doc README.md --doc-quote "Retries are capped at 5 attempts" \
  --code-file src/retry.ts --code-quote "MAX_ATTEMPTS = 5" \
  --trust verified --owner alice
```

- **The doc span supplies the claim text** — pass `--doc-quote "<sentence>"`
  (or `--doc-range L42:L44` / `--doc-line <n>`). The documented span is the
  authoritative truth; hibi keeps a non-authoritative `textCache` for audit and
  re-anchoring only, and re-reads the live span at `check` time. Propositions
  dedupe by the content fingerprint of the confirmed text, so phrase the sentence
  as a standalone assertion, not "see below". (`--text` survives as a *legacy*
  override for a pristine string when there's no doc span to point at; prefer
  `--doc-quote`.)
- **Anchor the code side on the load-bearing token, as tightly as you can.** Pass
  `--code-file <f>` with `--code-quote` (or `--code-range L1:L9` / `--code-line <n>`)
  naming the exact substring whose change would falsify the claim — the literal `5`
  in `MAX_ATTEMPTS = 5`, a function signature, an enum member. hibi captures a
  redundant SelectorBundle from it (fuzzy `text-quote`, `text-position`, the
  enclosing tree-sitter `ast-node`, and the literal `value`), so a rename or reformat
  won't trip it but a real change will.
  **Tightness matters for the verdict, not just for locating:** quote the *value*
  (`--code-quote "5"`) rather than the whole line (`--code-quote "MAX_ATTEMPTS = 5"`).
  When you quote the whole line, the unchanged identifier (`MAX_ATTEMPTS`) keeps
  several selectors matching and a changed value often grades `code:moved` instead of
  `code:changed` — and only `changed` gates by default (see the verdict table).
  Anchoring a whole paragraph or an arbitrary line gives hibi nothing precise to grade.
- The quote must appear **verbatim** in the file at record time, or record fails.
- **`--trust`**: `verified` (you checked it — requires a precise anchor, and a ref
  that hibi fills in automatically from the current commit, so you don't normally
  pass `--ref`), `inferred` (default), or `assumed`.
- **Enforcement gates.** Only an **enforced** claim can produce a gating verdict.
  Pass `--enforce` (shorthand for `--enforcement enforced`) when the claim must hold
  CI red; otherwise the engine derives enforcement from trust + resolution. The
  enforcement ladder is `suggested` (advisory) · `enforced` (gates) · `retired`
  (withdrawn) · `unanchored-legacy` (migrated, never strongly enforced).
- **Behavioral claims** (ordering, retry, complexity, …): declare them with
  `--claim-kind <k>` and attach executable evidence with repeatable
  `--verifier kind:ref` (e.g. `--verifier example:tests/retry.test.ts`). A passing
  verifier upgrades the belief to `supported`; a failing one `refutes` it.
- **`--coarse`** (with `--code-file`) or **`--glob <g>`** anchors to a file/path for
  blast-radius coverage only; coarse anchors are navigational and are *never* graded
  as drift. Use a precise span whenever a specific line backs the claim.
- `--owner` defaults to the git-blame author of the anchored line; `--ttl` sets an
  expiry instant (an orthogonal flag, not a state); `--ref` records the commit
  verified against.

When in doubt about whether something is anchorable, ask: *"If this code changed,
should a reader re-check this sentence?"* If yes, it's a claim — anchor it to that
code. If it's pure prose with no code behind it, don't record it.

## The everyday loops

`hibi check` (without `--write`) is **read-only and safe to run anytime** — use it
freely. `--write` and `diff --write` **modify your docs** (stamp/remove banners,
set markdown frontmatter status); run those only when you intend to update docs,
and review the resulting diff.

```sh
hibi query   --path src/retry.ts   # BEFORE editing code: which claims cover this file?
hibi diff    --since origin/main   # AFTER a change: what did it invalidate? (CI / write-time)
hibi check                         # verify every claim; exit code is the gate
hibi check   --write               # verify, and stamp banners into affected docs
hibi status  --doc README.md       # read-time gate: is this one doc still current? (exit 2 if not)
hibi suggest --doc README.md       # propose anchorable claims from a doc (as 'suggested' records)
```

The natural agent rhythm: **`query` before you edit code** (so you know which docs
your change touches), **`diff --since`** or **`check`** after, and **`status`**
before you trust a doc you're about to act on. Use **`suggest --doc <p>`** to seed
a doc that has no claims yet — it proposes anchorable sentences as advisory
`suggested` records you then refine and enforce.

## Reading verdicts and exit codes

A verdict is **two axes plus a flag**, not one fused word — and it's **verdict-first**
(the decision fields lead, the bulky `evidence` trails). A verdict reads e.g.
`doc:unchanged · code:changed · behavior:at-risk`.

- **Axis 1 — anchor resolution**, reported *per side* as `doc:…` and `code:…`:
  `unchanged` (found, identical) · `moved` (found, relocated) · `changed` (found,
  content differs) · `ambiguous` (matches several places) · `orphaned` (span gone).
- **Axis 2 — behavioral belief**, only on behavioral claims (else `n/a`):
  `unverified` (resting) · `at-risk` (reachable evidence changed) · `supported`
  (a verifier passed) · `refuted` (a verifier failed).
- **`expired`** is an orthogonal TTL flag, never a state.
- **"drift" / "stale"** are only the human roll-up in banner headlines — never
  machine states.

The exit code lets hibi gate CI and hooks without parsing JSON:

| exit | when                                                                                              | what to do |
|:----:|---------------------------------------------------------------------------------------------------|------------|
| 0    | clean — anchors resolve `unchanged`, no expired/refuted                                            | nothing |
| 3    | `moved` (either side) or `behavior:at-risk` — found but drifted; advisory, never gates             | re-verify; if still true, `reanchor` against current code |
| 2    | gating: on an **enforced** claim, `changed`/`orphaned`/`ambiguous` (either side), `expired`, or `behavior:refuted` | **re-verify** — is the sentence still true? then fix the prose/code and reanchor or retire |
| 1    | operational error (no store, bad flag)                                                             | fix the invocation |

**`changed` vs `moved` is about confidence — but only `changed`/`orphaned`/`ambiguous`
gate.** hibi fuses its selectors into a confidence score and bands the anchor state:
high → `unchanged`, medium (relocated/reformatted) → `moved`, low/meaningful-change →
`changed`, none → `orphaned`. A small edit to a value (`5` → `50`) keeps the quote
nearly identical but trips the value veto → `changed`; a pure relocation or reformat
under an otherwise-unchanged line stays `moved`. Don't read `moved` as "fine" — it
means re-verify, just with less certainty than `changed`. **`moved` and `at-risk`
never gate** (exit 3); only enforced claims gate at all.

**Exit codes and CI — the part people get wrong.** Any drift is non-zero, so the
default `hibi check` fails a CI step on *both* the exit-3 warning (`moved`/`at-risk`)
and the exit-2 gate; only a fully clean tree returns 0. `--fail-on` doesn't decide
whether CI fails — it decides how hibi *classifies* the result: `--fail-on warn`
escalates a warning-only run from 3 to 2 (treat re-anchorable drift as a hard
failure). Use it when you want the build red on *any* drift and don't want to depend
on a reader distinguishing 2 from 3. Note: a gating verdict exits 2 even under
`--fail-on never`/`tamper`, so those modes do **not** make a gated run pass — if you
need a report-only run, read the JSON and ignore the exit code.

A `check`/`diff` report carries a `summary` (counts), a `verdicts` array (each
verdict-first: `{ doc, code, behavior?, expired, gates, evidence{…}, notes }`, where
`evidence.selectorScores` and `notes` explain *why* — e.g. `"value veto — anchored
value changed"` or `"structural-only AST match (rename/whitespace)"`), and a
`documents` array with each doc's `suspect` list (status strings like `code:changed`).
Read the `notes` before deciding what a flag means.

## Responding to a flag — the core workflow

A flag is the start of a decision, not an instruction to silence it. **Never edit a
doc just to make the banner go away.** Read the flagged sentence *and* the current
code, then pick one of three outcomes:

1. **The claim is still true; the code just moved or was reformatted** (`doc:moved`/
   `code:moved`, sometimes `changed` after a rename). Refresh the anchor with
   **`hibi reanchor <claim-id>`** — re-resolve the claim against current content so
   its baseline matches today's tree. Pass `--doc-quote …` / `--code-file …` to point
   it at the relocated span if the auto-resolution can't find it.

2. **The code changed on purpose, so the sentence is now wrong** (`changed`, or
   `behavior:refuted`). Fix the prose (or fix the code, if the doc is the spec). Then
   **retire the obsolete claim** — see the store-hygiene note below — because the old
   assertion keeps pointing at the old evidence.

3. **The claim is obsolete — the feature was removed or the doc replaced**
   (`orphaned`). Use the lifecycle commands: `retract`, `archive`, or `supersede`.

### Store hygiene: retiring a claim (important)

`hibi record` **always appends a new assertion** — it only dedupes the *proposition*
by content fingerprint. It does not update or remove the old one. So if you record a
fresh claim and leave the stale original, **the original keeps flagging
`changed`/`orphaned` forever**. Prefer **`hibi reanchor <claim-id>`** when the claim
still holds and you just need its baseline refreshed.

To genuinely retire one obsolete claim, mark its assertion `retired` (it then no
longer gates) — or remove it from the store outright:

```sh
hibi query --path <file>          # find the assertion: its id, proposition text, and anchor
rm .claims/claims/<assertionId>.json   # the store is plain JSON, one file per record
hibi check                        # confirm it's clean
```

There is no `hibi` verb to retire a single assertion in place — you edit the store
directly (set `enforcement` to `retired`, or delete the file), which is safe because
it's plain per-record JSON. Removing an assertion can leave its proposition orphaned
in `.claims/propositions/` (e.g. the now-false "…100 requests…" text). Orphans are
**harmless to `check`** — it grades assertions, not propositions — so leaving them is
fine; remove the matching `.claims/propositions/<id>.json` too only if you want a tidy
store and no other assertion references that proposition id.

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
field. When the claims resolve clean again (anchors `unchanged`, nothing gating),
the next `hibi check --write` removes the banner. If someone hand-edited a banner body, `--fail-on tamper` makes hibi
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
- **Resolvers** speak a JSONL-RPC protocol (`describe` | `resolve` | `verify`);
  `verify` runs the executable `--verifier`s of a behavioral claim and reports the
  `behavior` state. The optional **semantic resolver** (`.claims/resolvers.json`)
  only *advises* — it never changes a verdict or the exit code. Leave it off unless
  asked.
