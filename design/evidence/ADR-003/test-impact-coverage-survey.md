# Test-impact & coverage-artifact survey (evidence for ADR-003 D26)

Date: 2026-07-08. Question: should hibi ship a built-in resolver that maps an anchored code
location to the tests exercising it, sourced from coverage artifacts — or is that a
plugin/deferred capability? What do working systems require?

**Answer: defer the coverage-artifact resolver; the committed-artifact premise fails twice.
Adopt the static reverse-import walk instead (fresh by construction).**

## Why committed coverage artifacts cannot work

1. **Merged coverage artifacts contain no per-test attribution.** lcov's `TN:<name>` is a single
   optional label for a whole tracefile section (set once per run via `--test-name`); `DA:`
   lines are aggregate counts. Istanbul `coverage-final.json` is per-file aggregate counters.
   Teamscale states it outright: "Most coverage tools do not provide coverage information on a
   per-test level" — which is why they invented a proprietary testwise format. From a standard
   `lcov.info` the strongest true statement is "some test in a prior run executed this file."
   https://linux.die.net/man/1/geninfo · https://docs.teamscale.com/howto/providing-testwise-coverage/
2. **The artifact is normatively absent from the repo.** GitHub's canonical Node `.gitignore`
   excludes `coverage`, `*.lcov`, `.nyc_output`; ecosystem guidance is generate-in-CI, upload to
   Codecov/Coveralls, never commit. The resolver's common case would be "no input found."

**No surveyed system — commercial, OSS, or academic — operates from a committed coverage
artifact.**

## What working systems require

- **Fresh per-run instrumentation** (per-test coverage): pytest-testmon (self-refreshing
  `.testmondata` on every run); Datadog Test Impact Analysis (per-test coverage collected on
  every instrumented run; skips only on exact content match; unskippable markers; default branch
  excluded; disclosed runtime overhead); Azure DevOps TIA (dynamic map from a baseline run,
  refreshed server-side; unknown file type → run all; Microsoft recommends periodic full-suite
  runs because maps drift); Teamscale (bespoke profiler with test start/stop REST signaling).
- **Static dependency graph recomputed at query time, no coverage**: Jest
  `--findRelatedTests`/`--changedSince` (haste-map import graph from the current filesystem),
  Vitest `--changed`, Nx affected, Bazel/Google TAP (reverse build-graph at HEAD). Documented
  miss modes: dynamic requires, DI/plugin registries.
- **ML over test-result history** (abandoning coverage): Facebook predictive test selection
  (budgets for misses: >99.9% faulty changes caught), Launchable (retrained several times a
  week). https://arxiv.org/abs/1810.05286
- **Reliability literature**: Rothermel & Harrold 1997 — selection is safe only when the
  dependency data corresponds exactly to the old version being diffed; the collection phase
  regenerates the map every cycle by definition. Even fresh static class-level maps miss ~5.9%
  of tests vs dynamic baselines (Legunsen et al., FSE 2016, STARTS vs Ekstazi). A stale snapshot
  map is strictly worse than either.
- **Requirements-traceability practice** (Doorstop, sphinx-needs/Melexis, ISO 26262): links
  requirements to tests exclusively via manually assigned IDs/relations — never derived from
  coverage. This is the shape of hibi's declared verifier.
- **Doc/drift tools prior art**: none. Swimm (token/snippet auto-sync, no test mapping),
  DeepDocs (LLM over git diff, no test mapping).

## Verdict (as adopted in D26)

- **Deferred** (PRD §19 spec): a resolver over per-test-attributed artifacts (coverage.py
  dynamic contexts, Teamscale testwise JSON) with a hard freshness gate (artifact commit ==
  HEAD, else refuse) — naturally a CI-side plugin fed by a same-run instrumentation step.
  Reopen triggers: (1) a mainstream runner emits per-test attribution in a default format;
  (2) a hibi CI context guarantees a same-commit context-enabled artifact; (3) repeated user
  requests to name covering tests.
- **Adopted**: deterministic static reverse-import walk (test files whose depth-≤2 import
  closure contains the anchored file), advisory-only, surfaced in the declare-a-verifier
  remediation. Fresh by construction against the working tree; the industry-validated primitive
  (Jest/Vitest/Nx pattern); acceptable miss modes for a suggestion.
