# hibi CLI reference

Output is JSON by default; add `--pretty` for human-readable output. Global flags
work on every command: `--pretty`, `--cwd <dir>` (run against another repo root),
`--no-ast` (skip the tree-sitter analyzer; Tier-1 text/position anchors still work).

## Commands at a glance

| Command | Purpose |
|---------|---------|
| `init` | Create `.claims/` (config.json with version + nonce, and a cache `.gitignore`). |
| `record` | Write a code-anchored claim (Document upsert + Proposition + Assertion/Anchor). |
| `check` | Verify every claim; emit verdicts + summary; exit per contract. `--write` stamps banners. |
| `diff --since <ref>` | Same as check, but restricted to claims whose anchored file changed since `<ref>`. |
| `query --path <p>` | List the claims anchored to / covering a path. Read-only. |
| `status --doc <p>` | "Is this one document current?" read-time gate (exit 2 if suspect). |
| `supersede` | Author a `supersedes`/`amends` edge between two documents. |
| `retract --doc <p>` | Mark a document retracted (author withdrew it). |
| `archive --doc <p>` | Tombstone a document out of the read path; optional `--successor`. |
| `schema [--name <Name>]` | Emit generated JSON Schema(s) for the data model. |
| `version` / `help` | Version info / usage. |

## `record` flags

| Flag | Required | Notes |
|------|----------|-------|
| `--doc <path>` | yes | Repo-relative path of the document making the claim. |
| `--text <sentence>` | yes | The claim. Propositions dedupe by the fingerprint of this text. |
| `--file <path>` | yes (unless `--coarse`) | The source file the claim anchors to. Must exist. |
| `--quote <str>` | one region selector | Exact substring to anchor on; matched fuzzily on later checks. Must appear verbatim now. |
| `--line <n>` | one region selector | 1-based line number, as an alternative to `--quote`. |
| `--start <byte> --end <byte>` | one region selector | Byte range, as an alternative. |
| `--coarse` | — | Anchor to the file/path only (navigational coverage). Coarse anchors are never graded stale. |
| `--trust <level>` | — | `verified` \| `inferred` (default) \| `assumed`. `verified` requires a precise anchor **and** a ref. |
| `--owner <name>` | — | Defaults to git-blame author of the anchored line, else `unknown`. |
| `--ref <ref>` | — | The commit verified against. Defaults to the current ref (or `WORKTREE`). |
| `--ttl <iso>` | — | Expiry instant; past it the computed state is `expired`. |

A region selector (`--quote`, `--line`, or `--start/--end`) is required unless
`--coarse`. If the quote isn't found, record fails with a non-zero exit and an
`{ ok: false, error }` payload.

## Exit-code contract

| code | meaning |
|------|---------|
| `0` | all clean |
| `2` | suspect present (`stale` / `ghost` / `expired`); or tamper under `--fail-on tamper` |
| `3` | `moved`-only (re-anchorable warning, nothing suspect) |
| `1` | operational error (no store, unknown command, bad input) |

Any non-zero exit fails a CI step, so the default already gates on both `moved` (3)
and suspect (2). `--fail-on` controls *classification*, not whether CI fails:
- `suspect` (default): suspect → 2, moved-only → 3.
- `moved`: escalate moved-only to 2 (treat re-anchorable drift as a hard failure).
- `tamper`: also return 2 when a banner was hand-edited, and refuse to overwrite it.
- `never`: intended as report-only. **Caveat (current implementation):** suspect
  drift still exits 2 and moved-only still exits 3 under `never`, so it does not make
  a drifted run pass — for a truly non-failing run, read the JSON and ignore the code.

## How a change is graded (`stale` vs `moved`)

Selectors are fused into a confidence `C = Σ(wᵢ·sᵢ)/Σ(wᵢ)` over the selectors that
resolved (weights: ast-node 0.35, text-quote 0.3, value 0.2, text-position 0.15),
then banded: `C ≥ 0.8` → `fresh`, `0.5–0.8` → `moved`, `0.2–0.5` → `stale`, `< 0.2`
→ `ghost`. Two special rules override the bands: a **value veto** forces `stale`
when the anchored value changed *and* the located text is still ≥ 0.9 similar to the
baseline quote; fewer than two resolving selectors forces `ghost`. Practical upshot:
a value change under an otherwise-unchanged line keeps text/position/ast partially
matching, so it often lands in `moved` (≈ 0.5) rather than `stale` unless you anchor
the value tightly. Both are flags; `stale` is the harder "suspect" state.

## JSON output shapes

**`record`** → `{ ok, action:"record", document, proposition, assertion, dedupedProposition }`.
The `assertion.anchor.selectors` array holds the redundant baseline: `text-quote`
(`exact`/`prefix`/`suffix`), `text-position` (`start`/`end`), `ast-node`
(`language`/`nodeType`/`structuralHash`/`semanticHash`), and `value`
(`language`/`nodeKind`/`value`). `dedupedProposition: true` means the claim text
matched an existing proposition.

**`check` / `diff`** →
```
{ ok, action, ref,
  verdicts: [ { assertionId, propositionId, documentId, ref, state,
                confidence, region:{start,end}, selectorScores:[{kind,found,score,weight}],
                notes:[…], advisories:[…] } ],
  documents: [ { id, path, lifecycle, suspect:[{propositionId,state}],
                 bannerAction?, tampered?, frontmatterStatus? } ],
  summary: { fresh, moved, stale, ghost, expired, total },
  exitCode }
```
`diff` also includes `since` and `changedFiles`. Read `notes` for the reason a
claim was graded the way it was (e.g. `"value veto — anchored value changed"`,
`"structural-only AST match (rename/whitespace)"`).

**`query`** → `{ ok, action:"query", path, count, hits:[ { assertion, proposition,
documentPath, coarse } ] }`. This is how you find the `assertion.id` to remove from
`.claims/claims/<id>.json` when retiring an obsolete claim.

**`status`** → `{ ok, action:"status", doc, found, lifecycle, current,
suspect:[…], verdicts:[…] }`; exit 2 when the doc is suspect.

## Store layout (`.claims/`)

```
.claims/
  config.json                  { version, nonce }
  documents/<id>.json          one per document (path, lifecycle, edges)
  propositions/<id>.json       one per claim text (text, authoredTrust, fingerprint)
  claims/<assertionId>.json    the Assertion + its baseline Anchor
  .gitignore                   ignores cache/ (regenerable)
```

Commit everything except `cache/`. Records are plain JSON, one file each — safe to
inspect, and editable by hand when you need to retire an individual assertion that
no CLI verb targets.

## `schema`

`hibi schema` lists the available schema names; `hibi schema --name Verdict` (or
`Assertion`, `Anchor`, `Document`, `Proposition`, `Selector`, …) prints the
draft-2020-12 JSON Schema generated from the Zod model. Useful when building
tooling that produces or consumes store records.
