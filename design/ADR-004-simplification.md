# ADR-004: Simplification for an agent-first CLI

Status: implemented (decided in review on 2026-09-02, implemented the same day)

## Context

A full review of the code, docs, plugin, and a hands-on first-user run found that the core (fuzzy text anchor, AST hash, banner stamping, supersession edges) is sound and fast, while about 2.5k of 11.6k source lines served features no user exercised, and the CLI had 20 verbs and 54 flags where four verbs called the same engine function. The review also found trust-breaking bugs (see D12, D19).

Product answers that shaped the decisions:

- **Primary user is a coding agent.** Humans and CI are secondary. JSON is the contract; the terminal view is kept but not extended.
- **Flow:** the Claude Code skill sits in context; the user asks the agent to check the claims before or after a feature. Hooks are optional. Results must be visible to people without hibi installed, so committed banners stay core.
- **Agents author most claims. New claims are enforced by default.** A claim that cannot fail anything does not prevent false claims.
- **Document lifecycle** (one doc replacing another) is rare for the maintainer but kept minimal for others.
- **Verifiers stay opt-in** (`--run-verifiers`), because they execute repo-committed commands.
- **`coverage` is important**: it is how the user finds doc regions backed by no claim, which are candidates for simplification or removal.

## Decisions

Each decision lists what was implemented. Where the implementation differs from the review text, the difference is stated.

### D1. Behavioral axis: keep verifiers only
Removed the keyword classifier (`src/algo/behavioral.ts`), evidence sets and baselines (`src/engine/evidence.ts`, `src/ast/imports.ts`), `BehaviorScope`, `hibi ignore`, `test-suggest.ts`, doctor rates, and the states `unverified` and `at-risk`. A claim is behavioral iff it has a verifier. `Verdict.behavior` is absent unless a verifier ran under `check --run-verifiers`; values are `supported` and `refuted`.
Why: the regex matched most technical sentences; the evidence set produced a false at-risk in the dogfood run; the store had 3 of 12 claims behavioral only via the regex.

### D2. Remove exit code 3
`moved` exits 0 and appears as a warning in text and in `summary.warning`. `--fail-on warn` promotes it to 2. Exit codes: 0 clean, 2 gating, 1 operational error. `computeExitCode` in `src/engine/check.ts` and `src/core/gating.ts` lost the 3 branch.
Why: CI treats any non-zero as failure, so "moved never gates" was false in every pipeline.

### D3. Remove banner tamper detection
Dropped the `sha=` checksum from the END sentinel, `DocumentReport.tampered`, `--fail-on tamper`, and the FNV-1a vendor file. `--fail-on` values: `gating | warn | never`. The END regex still accepts an old `sha=` suffix so an existing banner is replaced once. The per-repo nonce stays.
Why: banners are regenerated on every `check --write`; deleting a banner or claim is not detected anyway; the docs described `tamper` wrongly.

### D4. Remove Rust SDK, TS SDK folder, semantic-advisor demo
Deleted `sdk/rust/` and its CI job, the Rust CodeQL entry, `sdk/ts/`, `resolvers/semantic-advisor.ts`, `src/resolver/builtin/semantic-advisor.ts`. `serveResolver`, the protocol types, and the model types are exported from `src/resolver/index.ts` as `@npupko/hibi/resolver` (`exports["./resolver"]` in `package.json`). The resolver test uses a 10-line echo resolver in `test/fixtures/`. The docs now say the built-in drift resolver is in-process and does not speak the wire protocol.

### D5. Remove unused model surface (one store version bump, v2 to v3)
Removed: `Document.frontmatterStatus`, `Document.pristine` and `record --pristine` (config globs cover it), `Edge.derived`, the reverse edge types `superseded-by` and `amended-by`, the `amends` edge, `Verifier.proves`, `VerdictEvidence.ref`, `VerdictEvidence.confidence`, `VerdictEvidence.selectorScores`, `Advisory.confidence`, `RemediationAction.effect` and `applicability`, the `inline-id` selector and `--inline-id`, `PRECISE_SELECTOR_KINDS`, `--detailed`, `attrs.reanchorDowngrade`, the `.claims/.gitignore` file. `path` and `glob` merged into one `coarse` selector (`{kind: "coarse", pattern}`), matched as an exact file, a directory prefix on a `/` boundary, or a glob. `ChangedEvidence.kind` is the enum `text | ast | value | verifier-source`. `VerdictEvidence` gained `similarity`.
`AuthoredTrust` became `verified: boolean`. It lives on the **Assertion**, not the Proposition: a proposition is shared across documents by fingerprint, so a per-claim confirmation belongs on the claim. `Assertion.enforcement` defaults to `enforced`.
`src/store/upgrade.ts` rewrites a v2 store in place once at `ClaimStore.open` (documents, propositions, claims, config), maps `authoredTrust === "verified"` to `Assertion.verified`, `amended` to `active`, `retracted` to `archived`, and prints a one-line notice on stderr. Schemas regenerated as `schemas/*.v3.json`; the v2 files are deleted.

### D6. Value selector: literal inside the quoted span only
`extractValueFrom` takes the quoted span and only returns a literal whose byte range lies inside it, at record time and at check time. No literal, no `value` selector, no veto. The veto stays because it is what catches `= 5` becoming `= 7` at 94% text similarity.

### D7. Merge `diff` and `status` into `check`
`check [--since <ref>] [--doc <path>] [--overview] [--write] [--fail-on] [--run-verifiers] [--verifier-timeout]`. `--doc` scopes to one document's files and filters the report to that document id (`src/engine/status.ts` computes the scope); its exit code goes through `computeExitCode` and honors `--fail-on`. `--overview` renders the per-document table in human output; the JSON is the same check report. `diff` and `status` remain for one release as aliases that print a deprecation notice to stderr. `action.yml` lost the `command` input; `since` maps to `check --since`.

### D8. Fold `query` into `list`, remove `doctor`
`list [--state all|gating|warning|clean|orphaned|suggested|stranded|duplicate] [--path <p>] [--ids-only]`. `--path` matches either side, including coarse patterns, and rows carry `side`. Rows also carry `text` (the sentence) and `enforcement`, which `query` used to return. `stranded` is a live claim on a superseded or archived document; `duplicate` is a sentence asserted by more than one live claim. The `check --overview` footer shows the store version and the duplicate count.

### D9. One output selector
`--format human|compact|json|json-pretty` (default: human on a TTY, json when piped). `--json` stays as an alias. Removed `--pretty`, `--compact`, `--detailed`; `--simple` became `HIBI_ASCII=1`. Kept `--color`, `--explain`, `--no-hints`, `--ids-only`. `schema` prints indented JSON unless the format is `json`.

### D10. Per-command option tables, strict parsing
`src/cli/options.ts` holds one table per command plus the global options (`{name, type, values?, placeholder?, multiple?, help, required?}`). Parsing is `parseArgs` with `strict: true`; an unknown flag or a flag without a value is a one-line error with exit 1; enum values and required options are validated by one shared helper. `hibi <cmd> --help` prints that command's table and never runs the command. `src/cli/completions.ts` and `scripts/gen-cli-reference.ts` (writing `docs/cli-reference.mdx` and the plugin's `references/cli-reference.md`) are generated from the tables; CI regenerates and fails on drift.

### D11. `record` at 13 flags
`--doc`, `--doc-quote`, `--doc-range`, `--code-file`, `--code-quote`, `--code-range`, `--glob`, `--suggest`, `--verified`, `--verifier` (repeatable), `--ttl`, `--owner`, `--from-file`. Dropped `--doc-line`, `--code-line`, `--coarse`, `--enforce`, `--enforcement`, `--trust`, `--behavioral`, `--no-behavioral`, `--pristine`, `--inline-id`, `--ref` (filled from git HEAD). `--from-file -` (JSON on stdin, keys in camelCase) is the documented path for agents. The piped `record` result is `{id, doc, code, enforcement, verified, warnings?}` unless `--explain`.

### D12. `reanchor` safety and remediation order
`reanchor` resolves every side once (`resolveSides`) and refuses when a side resolves `orphaned` and no explicit new span was given for that side; `buildSelectorBundle` refuses an empty span. The result carries `before` and `after` quotes per side and `warnings`. `--suggest` ranks candidates for the doc quote across registered documents and for each code quote across files of the same language (by extension, skipping `node_modules`, `.git`, `.claims`, `dist`); each candidate has a `side`. For a changed code side the menu is `update-claim` (recommended, "Update the sentence, then reanchor"), `reanchor` (as is), `retire`; for a changed doc side `reverify-doc`, `reanchor`, `retire`; for both `reconcile`, `reanchor`, `retire`. An orphan recommends `reanchor` with the `--suggest` command, then `retire`, then `supersede`.
The reanchor attestation downgrade (`verified` to `inferred` without `--ref`) was dropped with `--ref`: the ref is always the current HEAD, and `verified` is a recorded fact that does not affect gating.

### D13. Lifecycle: three verbs
`retire <id>`; `supersede --from <old> --to <new> [--dry-run]`, which authors the `supersedes` edge, flips the old document to `superseded`, and in the same pass reanchors every live claim whose current sentence appears verbatim in the new document (the rest are `misses`; `strandedClaims` lists what is left); `archive --doc <p> [--successor <p>] [--dry-run]`. Removed `retract`, `relocate`, `--type amends`, `--propositions`. `DocumentLifecycle` is `active | superseded | archived`. The lifecycle ops take a small `LifecycleDeps` (read a doc, reanchor a claim) so `Engine.relocate` could move into `src/engine/supersede.ts`.

### D14. Explicit resolution cascade (validated against prior art)
`src/algo/resolve.ts` replaced the weighted fusion (`fusion.ts`, `WEIGHTS`, `BANDS`, `MIN_AGREEING_SELECTORS`, `STRUCTURAL_ONLY_SCORE`, `positionFound`) with ordered rules per side:
1. Locate the stored quote (`src/algo/localize.ts`): every exact occurrence, ranked by the stored 48-char prefix/suffix context; if none, the Bitap cascade near the stored position, accepted only within the fuzzy error budget.
2. Located, normalized similarity at or above `SAME_TEXT_SIMILARITY` (0.9), no AST or value change, start within `MOVE_AWARENESS_CHARS` (4) of the stored offset: `unchanged`.
3. Same, at a new offset: `moved`.
4. Semantic AST hash changed, in-span literal changed, or similarity below 0.9: `changed`, with a reason label. Both AST hashes are kept as labels: structural equal and semantic different is "identifiers or literals renamed", both different is "restructured". For prose (no AST), a changed numeric token inside the sentence is also `changed` ("a number in the sentence changed"); this was added because a `5` to `7` prose edit sits above the similarity floor and would otherwise pass as unchanged.
5. Not found: `orphaned`.
6. Several exact occurrences with equal context scores and a quote of at least 8 chars: `ambiguous`. A shorter repeated quote is picked by position.
The error budget is `min(FUZZY_ERROR_CAP = 256, floor(len * FUZZY_ERROR_RATIO = 0.4))` edits, in `src/algo/params.ts`. Tree-sitter grammars load lazily per language (`TreeSitterAnalyzer.load`), driven by the files each command touches. A rename now grades `changed`, not `moved`/`orphaned`.
Prior art read: Hypothesis fuzzy anchoring (ordered cascade; position weight 2 of 92; error budget min(256, len/2); context 40 of 92), W3C Web Annotation Data Model 4.2 (pick one selector; multiple matches = all), Gerrit ported comments (degrade precision rather than guess), GumTree-family two-tier matching.

### D15. Docs and skill
SKILL.md under 800 words: vocabulary, the five commands an agent needs, the JSON shapes, when to run. CLI reference generated (D10) into `docs/` and `plugins/`. `PRD.md` moved to `design/PRD-historical.md` with a status line. `docs/concepts.mdx` merged into `docs/verdicts.mdx`. Stale statements fixed (schema version, PRD link, remediation ids, banner headline, vocabulary). Exit-code tables are 0/2/1 everywhere.

### D16. Distribution
Dropped `packaging/hibi.rb`. Kept the curl installer, npm, the plugin marketplace, and `action.yml` with inputs `fail-on`, `since`, `write`, `run-verifiers`, `working-directory`.

### D17. `coverage`
Kept. Dropped executable-block tagging. Prose splits at sentence level (terminator followed by whitespace); headings, list items, blockquote lines, and table rows are their own regions; fenced blocks stay whole. A missing document is exit 1. Summary keys are `regions`, `covered`, `uncovered`, `coverageRatio`. `--fail-uncovered` kept.

### D18. Internal cleanup, no behavior change
`Engine.relocate` and the `status` scope moved into `src/engine/`. One `codeSideOf` and one `spanSpec` in the CLI; one `envelope()` helper for every JSON result; `RecordCall.code` is `CodeTarget[]`; one `rankSeverity` in `render/symbols.ts`; the duplicate `oneLine` and `stripPlain` are gone. A non-advisory external resolver may not claim a built-in kind (`text-quote`, `text-position`, `ast-node`, `value`, `coarse`) unless the manifest entry sets `override: true`; the kinds are dropped with a stderr warning. `VerifyParams` no longer carries `files`. Test-only exports (`fuseConfidence`, `bandConfidence`, `planRecord`, `amendedPropositions`, `liveClaimsOnDocument`) are gone.

### D19. Correctness fixes
Retired claims are counted in `summary.retired`, never in `clean`, and never gate or warn. `record` rejects a nonexistent `--code-file` (exit 1). `check --since <unknown ref>` exits 1 (`git rev-parse --verify` first). A doc quote that occurs more than once but is disambiguated by its context returns a warning in the result; one the context cannot disambiguate is still refused. Piped `record` returns the lean summary unless `--explain`. `--doc-range 3` and other malformed spans return a one-line error.

## Consequences

- Command count 20 to 12: `init`, `record`, `check`, `list`, `coverage`, `reanchor`, `retire`, `supersede`, `archive`, `schema`, `completions`, `version`. `diff` and `status` are deprecated aliases for one release.
- Exit codes 0, 2, 1. `--fail-on gating|warn|never`.
- Store version v3 with a one-time in-place upgrade at open.
- Changelog must call out: exit 3 removed; `suggested` no longer the default; removed verbs and flags; `authoredTrust` replaced by `verified` on the claim; a rename now grades `changed`; coverage summary keys renamed; the `record` JSON result is lean by default.
- Source went from about 10.1k lines to about 7.3k lines under `src/`.

## Order followed

1. Model and store (D1, D3, D5), then the cascade (D14, D6).
2. Engine modules (D2, D8, D12, D13, D17, D18, D19).
3. CLI tables, parsing, help, completions, generated reference (D10, D7, D9, D11).
4. Removals and distribution (D4, D16), tests, docs (D15), dogfood, own-store upgrade.
