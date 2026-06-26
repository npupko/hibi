# hibi CLI reference

**Output is TTY-aware.** The default is a rich human view when stdout is a terminal
and compact JSON when piped/redirected/CI. **Agents and scripts should pass `--json`**
to force the compact JSON contract regardless of environment — the JSON shape is
consume. Other output flags: `--json --pretty` (indented JSON), `--pretty` (force
the rich human view even when piped), `--compact` (one line per claim),
`--color auto|always|never` (honors `NO_COLOR` / `FORCE_COLOR`), `--simple` (ASCII
symbols).

**Verbosity & advice** (the agent hot path): JSON is **concise by default** —
decision-first fields plus the `remediation` menu, with the bulky located evidence
dropped. Add **`--explain`** (alias `--detailed`) to append the `evidence` tail,
`advisories`, and the proposition `fingerprint`. Add **`--no-hints`** (or set
`HIBI_ADVICE=0`) to drop the `remediation` block entirely — the escape hatch for
noise-sensitive harnesses.

Every JSON payload carries `schemaVersion` (e.g. `"v1"`) so a consumer can branch on
it without reading a filename.

Global flags work on every command: `--cwd <dir>` (anchor root — run against another
repo root), `--store-dir <dir>` (store location, default `<anchor>/.claims`),
`--no-ast` (skip the tree-sitter analyzer; text/position anchors still work).

## Commands at a glance

| Command | Purpose |
|---------|---------|
| `init` | Create `.claims/` (config.json with version + nonce, and a cache `.gitignore`). |
| `record` | Write a span-first claim (Document upsert + Proposition + Assertion with a bidirectional Anchor). |
| `check` | Verify every claim; emit verdicts + summary; exit per contract. `--write` stamps banners. |
| `diff --since <ref>` | Same as check, but restricted to claims whose anchored file changed since `<ref>`. |
| `status` | No `--doc`: a repo-wide document health overview (every tracked doc, worst status, claim counts, owner, lifecycle). |
| `status --doc <p>` | "Is this one document current?" read-time gate (exit 2 if any verdict gates). |
| `query --path <p>` | List the claims anchored to / covering a path. Read-only. |
| `list [--state …]` | Triage: one lean row per claim (handle + status + severity + recommended). Read-only. `--state all\|gating\|warning\|clean`. |
| `completions <zsh\|bash\|fish>` | Print a shell completion script. |
| `coverage --doc <p>` | Report which blocks of a doc are backed by a claim vs uncovered (the grounding-audit worklist). Read-only. |
| `reanchor <claim-id>` | Re-resolve a claim against current content (new doc/code spans). |
| `retire <claim-id>` | Withdraw one claim (`enforcement` → `retired`); idempotent. A retired claim never gates/warns. |
| `supersede` | Author a `supersedes`/`amends` edge between two documents. |
| `retract --doc <p>` | Mark a document retracted (author withdrew it). |
| `archive --doc <p>` | Tombstone a document out of the read path; optional `--successor`. |
| `schema [--name <Name>]` | Emit generated JSON Schema(s) for the data model. |
| `version` / `help` | Version info / usage. |

## `record` flags

`record` is span-first: the claim text is read from the **documented span** (the doc
side) and anchored to zero or more **code spans**. Each side takes one of three
locators.

| Flag | Required | Notes |
|------|----------|-------|
| `--doc <path>` | yes | Repo-relative path of the document making the claim. |
| `--doc-quote <s>` | one doc locator | Exact substring of the documented sentence; matched fuzzily on later checks. |
| `--doc-range L42:L44` | one doc locator | 1-based line range (or char offsets `42:44`) on the doc side. |
| `--doc-line <n>` | one doc locator | 1-based line number, as an alternative. |
| `--code-file <path>` | — | A source file the claim anchors to. Must exist. Repeat the code locators below against it. |
| `--code-quote <s>` | one code locator | Exact substring to anchor on, on the code side. Must appear verbatim now. |
| `--code-range L1:L9` | one code locator | 1-based line range (or char offsets) on the code side. |
| `--code-line <n>` | one code locator | 1-based line number on the code side. |
| `--coarse` | — | With `--code-file`, anchor to the file/path only (navigational coverage). Coarse anchors never grade as drift. |
| `--glob <g>` | — | Anchor to a directory/glob edge (coarse) instead of a file. |
| `--inline-id <id>` | — | A hidden marker that *identifies* the record near the paragraph; aids re-anchoring, never restates the claim. |
| `--trust <level>` | — | `verified` \| `inferred` (default) \| `assumed`. `verified` requires a precise anchor **and** a ref. |
| `--enforce` | — | Shorthand for `--enforcement enforced`. Only `enforced` claims gate. |
| `--enforcement <e>` | — | `suggested` \| `enforced` \| `retired` \| `unanchored-legacy`. With neither flag the engine derives it: `verified` trust + a `@ref` + a precise anchor resolving on **both** sides → `enforced`, else `suggested`. An explicit value wins over `--enforce`. |
| `--claim-kind <k>` | — | Behavioral kind: `ordering` \| `retry` \| `complexity` \| `concurrency` \| `caching` \| `validation` \| `error-handling`. Routes Tier-3 classification. |
| `--verifier kind:ref` | — | Repeatable executable-evidence link (`kind` ∈ example/snapshot/contract/property/formal/command). A failing verifier → `refuted`. |
| `--owner <name>` | — | Defaults to git-blame author of the anchored line, else `unknown`. |
| `--ref <ref>` | — | The commit verified against. Defaults to the current ref (or `WORKTREE`). |
| `--ttl <iso>` | — | Expiry instant; past it the verdict sets the orthogonal `expired` flag. |
| `--text <sentence>` | legacy | Override that supplies the claim text directly instead of reading the doc span. Use only when no doc locator applies. |

A doc locator (`--doc-quote`, `--doc-range`, or `--doc-line`) is required, or the
legacy `--text` override. The code side is optional — a doc-only `suggested` claim
may await a code target — but an **`enforced` claim must resolve both sides**, so
`record` throws (exit 1) when an enforced outcome can't anchor doc and code. If a
quote isn't found, record fails with a non-zero exit and an `{ ok: false, error }`
payload.

## Exit-code contract

| code | meaning |
|------|---------|
| `0` | all clean |
| `2` | a verdict **gates**: `changed`/`orphaned`/`ambiguous` (either side) or `expired` or `behavior:refuted` on an **enforced** claim; or tamper under `--fail-on tamper` |
| `3` | warning only: `moved` (either side) or `behavior:at-risk` — re-anchorable, nothing gating |
| `1` | operational error (no store, unknown command, bad input) |

Any non-zero exit fails a CI step, so under the default both the warning band (3) and
the gating band (2) fail the build; only a clean tree (0) passes. `--fail-on` selects
the exit threshold:
- `gating` (default): gating → 2, warning-only → 3, clean → 0.
- `warn`: escalate the warning band to 2 (treat re-anchorable drift as a hard failure).
- `tamper`: like `gating`, but a hand-edited banner also exits 2 (and hibi refuses to
  overwrite it); a gating verdict still exits 2.
- `never`: always exit 0, whatever the verdicts — a true report-only run; read the JSON
  for the result.

## How a change is graded (two axes)

A verdict carries two independent axes plus an orthogonal flag — there is no single
fresh/moved/stale enum:

- **Anchor resolution** (Axis 1), reported per side as `doc:…` and `code:…`:
  `unchanged` (found, identical) · `moved` (found, relocated, same content) ·
  `changed` (found, content differs) · `ambiguous` (matches several places) ·
  `orphaned` (span deleted / unresolvable).
- **Behavioral belief** (Axis 2), only on behavioral claims:
  `unverified` (resting) · `at-risk` (reachable evidence changed) ·
  `supported` (a verifier passed) · `refuted` (a verifier failed).
- **`expired`** — an orthogonal TTL flag, never a state.

A verdict reads e.g. `doc:unchanged · code:changed · behavior:at-risk`. The words
"drift" and "stale" are only the **human roll-up** in banner headlines — never
machine states. Per side, selectors are fused into a confidence
`C = Σ(wᵢ·sᵢ)/Σ(wᵢ)` over the selectors that resolved (weights: ast-node 0.35,
text-quote 0.3, value 0.2, text-position 0.15); the band plus override rules (a
**value veto** when the anchored value changed but the located text is still
≥ 0.9 similar; fewer than two resolving selectors forces `orphaned`) decide the
per-side `AnchorState`. Only `changed`/`orphaned`/`ambiguous` (and `refuted` on the
behavior axis) gate; `moved` and `at-risk` are advisory warnings. Read `notes` for
the reason a side was graded the way it was (e.g. `"value veto — anchored value
changed"`, `"structural-only AST match (rename/whitespace)"`).

## The remediation menu

Every drift verdict (and `list` row) carries a **deterministic remediation menu** —
the verdict→action lookup, surfaced so an agent doesn't re-derive next steps. It's a
*menu*, not a prescription: hibi cannot know whether a code change was deliberate, so
`recommended` is set only when the next step is unambiguous, and `actions` are ordered
safest/most-severe-first.

```
remediation: {
  recommended: "reanchor" | "retire" | … | null,   // the single best action id, or null
  actions: [ { id, title, applicability, effect, rationale, command? } ]
}
```

- **`applicability`** (Rust `Applicability`-style): `auto` (safe to apply) ·
  `needs-review` (apply, then review) · `manual` (decide intent first).
- **`effect`**: `deterministic` (hibi performs it — carries a ready-to-run
  `command` with the claim id pre-filled) · `prose` (you rewrite doc/code — no command).
- A `command` is **never** pre-filled when it cannot succeed: an orphan's `reanchor`
  needs an explicit `--doc-range`/`--code-file` target, so it carries no command and
  `recommended` points at `retire`/`supersede` instead.

`--no-hints` / `HIBI_ADVICE=0` omits the whole `remediation` block.

## JSON output shapes

All payloads lead `{ ok, action, schemaVersion, … }`. Mutating verbs add `next` (the
exact follow-up command) and the resulting `claimId` where one applies.

**`record`** → `{ ok, action:"record", schemaVersion, document, proposition,
assertion, dedupedProposition, claimId, next }`. The Anchor is bidirectional:
`assertion.anchor.doc` is the doc-side `SelectorBundle` (`{ file, selectors[] }`) and
`assertion.anchor.code[]` an array of code-side bundles. Selector kinds: `text-quote`
(`exact`/`prefix`/`suffix`), `text-position` (`start`/`end`), `ast-node`
(`language`/`nodeType`/`structuralHash`/`semanticHash`), `value`
(`language`/`nodeKind`/`value`), `inline-id` (`id`), and coarse `path`/`glob`.
`dedupedProposition: true` means the claim text matched an existing proposition (the
`record`-always-appends footgun made visible).

**`check` / `diff` / `status`** → decision-first envelope. **Concise (default):**
```
{ ok, action, schemaVersion, ref, exitCode,
  summary: { total, gating, warning, clean, doc{…}, code{…}, behavior{…}, expired },
  verdicts: [ { assertionId, propositionId, documentId,
                doc, code, behavior?, expired, gates,
                changed?,                 // 1-line "path kind" on at-risk/refuted
                remediation,              // the menu (null when clean; absent under --no-hints)
                notes:[…] } ],
  documents: [ { id, path, lifecycle, suspect:[{propositionId,status}],
                 bannerAction?, tampered?, frontmatterStatus? } ] }
```
**`--explain`** adds, per verdict: `evidence:{ docRegion?, codeRegions:[…], confidence,
selectorScores:[{kind,found,score,weight}], changedEvidence:[{path,kind,detail?}],
ref? }`, `advisories:[…]`, and `fingerprint`. `diff` also includes `since` and
`changedFiles`. The behavioral carve-out: `at-risk`/`refuted` verdicts keep a 1-line
`changed` summary (path + kind) on the **concise** path, so you learn *what* changed
without `--explain`.

**`list`** → `{ ok, action:"list", schemaVersion, state, count, claims:[ { claimId,
propositionId, documentPath, codePath, status, severity, gates, recommended } ] }`.
Rows are most-severe-first; each carries the `claimId` the next command needs.

**`query`** → `{ ok, action:"query", schemaVersion, path, count, hits:[ { assertion,
proposition, documentPath, coarse, side } ] }`. This is how you find the
`assertion.id` before an edit.

**`coverage`** → `{ ok, action:"coverage", schemaVersion, doc, summary:{ blocks,
coveredBlocks, uncoveredBlocks, coverageRatio }, regions:[ { range:{start,end}, preview,
covered, claimIds } ], next }`. Reports a structural fact per block (a claim's doc anchor
lands in it, or not); the `covered:false` regions are the ground-or-prune worklist.

**`reanchor`** → `{ ok, action:"reanchor", schemaVersion, assertion, doc, code,
claimId, next }`. Re-resolves `<claim-id>` against current content; `doc`/`code` are
the post-reanchor `AnchorState`s (should settle to `unchanged`).

**`retire`** → `{ ok, action:"retire", schemaVersion, assertion, alreadyRetired,
claimId, next }`. Flips `enforcement` to `retired`; idempotent (`alreadyRetired: true`
on a second call). `status` (single doc) reports exit 2 when any verdict gates, exit 3
on a `moved`/`at-risk` warning, else 0.

## Store layout (`.claims/`)

```
.claims/
  config.json                  { version, nonce, instructionFiles? }
  documents/<id>.json          one per document (path, lifecycle, edges)
  propositions/<id>.json       one per claim text (textCache, authoredTrust, fingerprint)
  claims/<assertionId>.json    the Assertion + its bidirectional Anchor
  .gitignore                   ignores cache/ (regenerable)
```

Commit everything except `cache/`. Records are plain JSON, one file each — safe to
inspect. **To withdraw a single claim, use `hibi retire <claim-id>`** (flips
`enforcement` to `retired`, keeping the audit trail) — never hand-delete a
`claims/<id>.json` file. Note the Proposition's `textCache` is **non-authoritative**:
the live doc span (re-read at check time via the doc-side anchor) is the truth.

## `schema`

`hibi schema` lists the available schema names; `hibi schema --name Verdict` (or
`Assertion`, `Anchor`, `SelectorBundle`, `Selector`, `Document`, `Proposition`,
`Remediation`, `RemediationAction`, `Verifier`, `BehaviorScope`, `Edge`,
`StoreConfig`) prints the draft-2020-12 JSON Schema generated from the Zod model. Useful when building tooling that produces or
consumes store records.
