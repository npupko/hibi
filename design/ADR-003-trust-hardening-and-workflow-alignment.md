# ADR-003 — Trust hardening, anchoring honesty, recovery ergonomics, and the author/verify/prune workflows

Status: accepted (owner confirmed each decision in the 2026-07-08 design interview)
Date: 2026-07-08
Deciders: project owner
Amends: PRD §14 (adds D22–D32), §19; docs/anchors.mdx, docs/concepts.mdx, docs/workflows.mdx
Inputs: full implementation audit of `src/` at v0.4.0 (HEAD `2358ba7`); the parallel.ai research
corpus (9 runs — urls in `docs/researches.local.md`); three new evidence efforts run for this
ADR, committed in full under `design/evidence/ADR-003/` and summarized in the Appendix: (i) a
literature/production survey of structural-vs-text anchoring, (ii) an empirical anchor-survival
experiment over 2,501 sentences from this repo's own doc-rewrite history (reproducible script
included), (iii) a survey of test-impact/coverage-artifact tooling.

---

## How to implement this ADR (read first — normative)

This ADR will be implemented by engineers/models who did not attend the design review. Rules:

1. **Do exactly what a Decision says. Do not extrapolate.** If a Decision does not mention a
   flag, field, file, or behavior, do not add it. Anything this review considered and did not
   adopt is either in a "Declined" note or intentionally out of scope.
2. **Error messages, flag names, field names, and JSON shapes given in this document are
   verbatim contracts.** Copy them character-for-character.
3. **If the code at implementation time contradicts a factual statement in this ADR** (a symbol
   was renamed, a file moved), stop, record the discrepancy in the PR description, and match the
   ADR's *intent* to the *current* symbol — do not invent a new mechanism.
4. **Never weaken a guard to make an existing fixture pass.** If a fixture, test, or dogfood
   claim violates a new guard, fix the fixture (widen the span, re-record the claim) — the guard
   is the product.
5. **`check` never writes to `.claims/`.** No decision in this ADR changes that. (`check
   --write` writes banners/frontmatter into *documents* only.) The only store writers remain
   `init`, `record`, `reanchor`, `retire`, `relocate`, `ignore`, `supersede`, `retract`,
   `archive`.
6. Existing invariants from ADR-001 and ADR-002 (fitness-function tests in `test/`) all remain
   in force. Run `bun test` after every phase.

Key file map (verified at HEAD `2358ba7`):

| Concern | File |
|---|---|
| Zod model, `MODEL_VERSION`, `Assertion`, `Advisory`, `Verifier`, `Document`, `StoreConfig` | `src/core/model.ts` |
| Grading constants (`AMBIGUOUS_MIN_QUOTE_LENGTH = 8`, `STRONG_TEXTQUOTE_SIMILARITY = 0.9`) | `src/algo/params.ts` |
| Verdict resolution + behavioral gate (`computeBehaviorRisk`) | `src/algo/resolve.ts` |
| Text-quote localization (Bitap cascade, `localizeTextQuote`, 48-char context) | `src/algo/localize.ts` |
| Import extraction (`extractImportSpecifiers`) | `src/ast/imports.ts` |
| Evidence-set construction (imports → files, hashing) | `src/engine/evidence.ts` |
| Store load/save, `.claims/` layout, `config.json` (`{"version","nonce"}`) | `src/store/store.ts` |
| Record engine (span-first validation; `--from-file` transactionality) | `src/engine/record.ts` |
| Reanchor engine (D15 downgrade at ~L264; `reanchorDowngrade` attrs) | `src/engine/reanchor.ts` |
| Check engine (read-only over store; banner writes at `options.write`) | `src/engine/check.ts` |
| Coverage engine (`CoverageResult`, `summary.uncoveredBlocks`) | `src/engine/coverage.ts` |
| Doctor (`DoctorRates`, `DoctorReport`, `computeRates`) | `src/engine/doctor.ts` |
| Remediation menu (`RemediationInput`, action builders) | `src/core/remediation.ts` |
| Resolver registry (advisory-drop rule at ~L133) | `src/resolver/registry.ts` |
| Resolver manifest (`ResolverSpec`, `Manifest`) | `src/resolver/manifest.ts` |
| CLI verbs + flags, exit codes (`EXIT_OPERATIONAL_ERROR = 1`; gating exit 2 via `report.exitCode`) | `src/cli/index.ts` |
| Schema generation | `scripts/gen-schemas.ts` (`bun run build:schemas`) |
| SDKs | `sdk/ts/`, `sdk/rust/` |
| Dogfood store (10 claims, owner `hibi-dogfood`) | `.claims/` |
| CI (dogfood step, schema-freshness check, rust-sdk job) | `.github/workflows/ci.yml` |
| Agent-facing skill (tracks the *shipped* CLI only) | `plugins/hibi-cli/skills/hibi/` |

---

## Context

ADR-002 (D12–D21) is fully implemented and shipped in v0.4.0: the behavioral tier is real, the
carrier is pure Model C, the six documented workflows all map to shipped verbs, and all six
fitness-function tests exist. This review compared the shipped product against the full research
corpus a second time, ran new evidence-gathering where the corpus was thin, and audited hibi
against three owner-stated usage intents (author grounded docs for new code; verify built code
against a pre-existing plan; prune ungrounded prose). Findings:

1. **The store can silently misrepresent what was recorded.** `MODEL_VERSION` is still `"v1"`
   despite ADR-002's breaking schema pass, so a pre-D12 store and a current store are
   indistinguishable — the promised "doctor detects v1 stores and says so plainly" is
   unimplementable. Worse, no Zod object in `src/core/model.ts` is `.strict()`, so Zod v4
   silently strips unknown keys: a store carrying the removed `claimKind` loads cleanly with the
   field dropped. This is a miniature of the exact failure the carrier research's
   source-of-truth inversion targets.
2. **hibi's own docs promise a selector that does not exist.** `docs/anchors.mdx` (selector
   table, mermaid diagram, grammars note), `docs/concepts.mdx`, and five PRD passages describe a
   doc-side markdown structural-path selector — including as step 1 of the doc-side resolution
   cascade (PRD §15). No markdown grammar ships; every doc anchor rests on `text-quote` +
   `text-position` (+ optional `inline-id`). New evidence (Appendix A/B) shows building it would
   be net-negative; the promise is retracted instead (D22).
3. **ADR-002's headline machinery has no live self-check.** All 10 dogfood claims are Axis-1
   structural: zero `behavioral: true`, zero verifiers. The behavioral gate and the command
   runner are guarded only by synthetic fixtures. Dogfood `ref`s are a branch name
   (`feat/post-adr-002-refinements`), not a commit, contradicting `Assertion.ref`'s own doc
   comment ("the `@ref` (commit) last verified against").
4. **The advisory quarantine has no provenance schema.** The behavioral research is normative:
   "No hidden LLM state: model name, prompt hash, context hash, temperature, and output must be
   recorded if an LLM plugin is used." PRD §19 repeats this in prose, but the wire `Advisory`
   object is `{resolver, message, confidence?}` — nowhere structured to put provenance, and the
   registry cannot enforce what the schema cannot carry.
5. **Drift-repair ergonomics stop at detection.** An orphaned claim's only recovery aid is
   `relocate`'s verbatim search in a target the user must name. A pure move (identical sentence,
   new offset) can only be repaired by `reanchor`, which D15 punishes with a trust downgrade —
   perverse for a byte-shift that changes no evidence, and it pushes users toward fake
   attestations or permanently stale selectors.
6. **The owner's three usage intents are mechanically supported but unnamed**, and one is
   unenforceable: `coverage` always exits 0, so "this plan must be fully grounded" cannot gate
   CI.
7. **Residual research white space** (test-mapping, compound-proposition decomposition,
   record-time context expansion) needed adopt/decline decisions with recorded rationale. New
   evidence settled each (Appendix).

No backwards-compatibility constraint applies (beta; explicit owner statement 2026-07-08:
"rewrite > migration"). Complete rewrites of modules are in scope; no migration shims anywhere.

---

## Decisions

### D22 — The doc-side structural-path selector is retracted (never built; will not be built)

**What to do.** Remove every claim that a markdown structural-path / doc-side `ast-node`
selector exists or is planned, from: `docs/anchors.mdx` (the mermaid diagram's DOC branch, the
`ast-node` selector-table row's doc-side cell, the "Grammars" paragraph's doc-side sentence),
`docs/concepts.mdx` (the parenthetical "(on the doc side this is the markdown structural path
instead)"), and the PRD (find every occurrence: `rg -n -i "structural path" PRD.md docs/` —
rewrite each passage to describe the real doc bundle: `text-quote` (48-char context) +
`text-position` + optional `inline-id`). The doc side is format-agnostic by design; that is now
the *stated* design, not a gap.

D19's escape hatch is rewritten to rest on inline IDs only: "if doc-side orphan rate exceeds
~30% for typical edits, require inline IDs for high-severity claims."

**Reopen trigger (record verbatim in PRD §19):** revisit a structural doc selector only if
`doctor` reports `docOrphanedRate` > 0.30 on typical edits for real users AND inline IDs have
been tried and rejected by those users.

*Grounding (Appendix A + B).* The research line proposing it is an uncited design-table row
whose own failure-mode column reads "breaks under structural rewrite." Production systems
(Hypothes.is, Notion, Obsidian, Wikipedia, GitBook, Swimm) converged on content matching +
assigned IDs; heading anchors are the documented chronically-broken feature (Wikipedia maintains
a bot fleet repairing broken section links). Empirically on this repo (N=2,501): the text
cascade re-attaches 92.3%; the 48-char context resolved 100% of ambiguous cases (structural
disambiguation value: zero); heading-path rescue (1.4% of sentences) is cancelled by misleads
(0.9%) plus a 22% false-attach rate on genuine deletions; heading trails survived 0% of the
README rewrite — least stable exactly when needed. A large share of would-be "rescues" were
semantic rewordings where flagging is correct product behavior.

*Declined:* building a heading-path selector (evidence above); storing a heading trail as an
advisory schema field (schema surface for a display nicety); report-time heading-trail display
(out of scope — do not build).

### D23 — Record-time doc-quote guard (replaces the never-adopted context-expansion idea)

**What to do.** In `src/engine/record.ts` and `src/engine/reanchor.ts`, after the doc span is
resolved (span-first), validate the doc-side `text-quote`:

1. **Length floor:** if the exact quote's length < `AMBIGUOUS_MIN_QUOTE_LENGTH` (import from
   `src/algo/params.ts`; value 8), reject with exactly:
   `doc quote is shorter than 8 characters — too short to anchor reliably. Record a wider span (--doc-range) that covers the full sentence.`
2. **Uniqueness:** count exact occurrences of the quote in the document text. If > 1, score each
   occurrence with the stored 48-char prefix/suffix context (reuse the scoring in
   `src/algo/localize.ts`; do not write a new similarity function). If the best-scoring
   occurrence is not strictly better than the second best, reject with exactly:
   `doc quote occurs N times in <docPath> and the surrounding context does not select a single occurrence. Record a wider span (--doc-range), or add an inline ID and re-record.`
   (Substitute `N` and `<docPath>`.)

Apply identically in `record`, `record --from-file` (per-spec validation — one failing spec
fails the whole batch, per D21 transactionality; do not partially write), and `reanchor`.

*Grounding (Appendix B).* All 34 ambiguous cases in the 2,501-sentence experiment were
degenerate short fragments (e.g. `` `refuted`. ``); the existing 48-char context resolved 100%
of genuine repeats. The failure to prevent is bad anchors at birth, not insufficient context.

*Declined:* record-time context *expansion* (widening stored prefix/suffix until unique) — the
ambiguity it solves occurred 0 times after context matching, and wider stored context is *more*
brittle to nearby edits.

### D24 — Orphan recovery suggestions: `hibi reanchor <id> --suggest`

**What to do.** Add a `--suggest` boolean flag to the `reanchor` verb (`src/cli/index.ts`, case
`"reanchor"`; engine work in `src/engine/reanchor.ts`). Semantics:

- Read-only. Never writes the store or any document. Exit code 0 always (operational errors
  still exit 1). Incompatible with `--ref` / `--doc-range` / other mutation flags: if combined,
  fail with `--suggest is read-only and cannot be combined with mutation flags.`
- Takes the claim's stored doc-side `text-quote` exact string (from the anchor — NOT the
  proposition `textCache`).
- Runs the existing `localizeTextQuote` cascade against the current content of **every Document
  registered in the store** (skip files missing on disk).
- Collects the best region per document with its similarity; keeps candidates with
  similarity ≥ 0.5; sorts by similarity descending, then document path ascending, then region
  start ascending; caps at 5.
- JSON output (verdict-first house style):
  `{ "action": "reanchor-suggest", "claimId": "<id>", "candidates": [ { "doc": "<path>", "start": <int>, "end": <int>, "similarity": <number>, "snippet": "<region text, trimmed to 120 chars>" } ] }`
  Human rendering: one row per candidate. Zero candidates is a valid result (empty array).
- Remediation wiring: in `src/core/remediation.ts`, the doc-orphaned action currently built by
  `reanchorToTarget()` (which deliberately carries no `command`) gains
  `command: "hibi reanchor <id> --suggest"` and its rationale becomes:
  `the span was deleted — run --suggest to list candidate targets, then re-anchor with an explicit --doc-range`.

*Grounding.* The carrier research: cached text is "for migration, diff explanation, and recovery
suggestions"; Brush et al.: rescue must be human-confirmed — confident wrong re-attachment rates
worse with users than an honest orphan. Suggestion-only keeps D15's attestation semantics as the
only path that actually moves an anchor.

*Declined:* searching git history (`git log -S`) for candidates — deferred to PRD §19 (git stays
off all verdict-adjacent paths; current-docs search covers the doc-renamed/merged cases);
auto-applying the top candidate (the Fiberplane gaming hole).

### D25 — D15 amendment: attestation-free exact re-anchor (pure-move repair)

**What to do.** In `src/engine/reanchor.ts`, where the D15 downgrade is computed (currently
`const downgrade = ...` near L264: downgrade iff `--ref` absent and trust is `verified`), add an
exception. Skip the downgrade (retain `authoredTrust: verified`, write no `reanchorDowngrade`
attr) iff **both**:

1. The re-resolved doc span's `text-quote` exact string is **byte-identical** to the previously
   stored `text-quote` exact, and it resolved **uniquely at similarity 1.0** (a pure move: same
   sentence, new offset); and
2. The code side re-resolves against its stored baseline as `unchanged` (reuse the same code-side
   resolution `check` uses; the graded state must be exactly `unchanged` — not `moved`, not
   `changed`).

Anything fuzzier — any similarity < 1.0, any code-side state other than `unchanged`, any doc
text difference — downgrades exactly as D15 shipped. `reanchor --ref <ref>` behavior is
unchanged. `check` gains no healing path (Rule 5).

*Grounding.* A byte-shift is evidence-neutral — there is nothing to re-attest, so charging trust
for repairing it pushes users toward fake `--ref`s or permanently degrading selectors (the store
drifts toward the fuzzy-match floor). The exception is mechanically gameable only by *not
changing anything*, which is not a gaming vector. Brush et al.: pure moves are the one case
content matching recovers with 100% reliability.

*Declined:* healing under `check --write` (breaks the "check never writes the store" §6
invariant); a separate `heal` verb (the `hibi list --state moved --ids-only` pipe into
`reanchor` already batches this — document it, don't build it).

### D26 — Test mapping: coverage-artifact resolver deferred; deterministic reverse-import test suggestions adopted

**Deferred (record in PRD §19, verbatim spec):** a resolver that reads per-test-attributed
coverage artifacts (coverage.py dynamic-contexts DB, Teamscale testwise JSON) with a hard
freshness gate (artifact's recorded commit == HEAD, else refuse with "stale — regenerate").
**Do not build it now.** Reopen triggers (record all three): (1) a mainstream runner
(vitest/jest/bun/pytest) emits per-test attribution in a default report format; (2) hibi gains a
CI execution context guaranteeing a same-commit, context-enabled artifact; (3) repeated user
requests to name covering tests for an anchor.

*Grounding (Appendix C).* Standard lcov/istanbul artifacts carry **no per-test attribution**
(lcov's `TN:` is one label per run), and coverage artifacts are normatively gitignored — the
promised output is unconstructible from the proposed input. Zero surveyed systems operate from
committed coverage snapshots; all production test-impact systems use fresh per-run
instrumentation (testmon, Datadog, Teamscale, Azure TIA), a fresh static graph (Jest, Nx,
Bazel), or manually declared IDs (all requirements-traceability practice — which is exactly
hibi's declared verifier).

**Adopted:** a deterministic **reverse-import test suggestion**, advisory-only, computed fresh
from the working tree at check time:

- New pure function (suggested home: `src/engine/test-suggest.ts`):
  `suggestTests(anchoredFile: string, repoRoot: string): string[]`.
- Candidate test files: paths matching `**/*.test.*` or `**/*.spec.*`, or any file under a
  directory named `test/`, `tests/`, or `__tests__/`, honoring the same file-walk/ignore rules
  the engine already uses for documents.
- A candidate matches when the anchored file is in the candidate's import closure at depth ≤ 2
  (the test file imports it directly, or imports a file that imports it). Reuse
  `extractImportSpecifiers` (`src/ast/imports.ts`) and the specifier→file resolution already
  used by `src/engine/evidence.ts`. Do not write a new import parser or resolver.
- Result: matching test file paths, sorted lexicographically, capped at 3.
- Wiring: `RemediationInput` (`src/core/remediation.ts`) gains
  `suggestedTests?: string[]` (optional). `src/engine/check.ts` populates it **only** when a
  verdict's `behavior` is `at-risk` or `refuted` AND the assertion's `verifiers` array is empty
  — computed lazily (build the test-file import index at most once per check run, and only if at
  least one verdict qualifies). The existing declare-a-verifier remediation action appends to
  its rationale: `— tests that exercise this code: <p1>, <p2>, <p3>` (omit the clause entirely
  when the list is empty).
- This never touches verdicts, exit codes, or the store.

*Grounding (Appendix C).* The static reverse-dependency walk is the industry-validated primitive
(Jest `--findRelatedTests`, Vitest `--changed`, Nx affected) — fresh by construction, no
artifact; its known miss modes (dynamic import, DI) are acceptable for an advisory suggestion.

### D27 — Compound-proposition lint: declined

No lint, no doctor row, no record-time warning for "this claim may bundle two propositions."
The only deterministic signal available (one proposition anchored in ≥2 code files) cannot
distinguish a compound sentence from one fact legitimately corroborated in two places; an
ambiguous heuristic nag violates the noise discipline D14 made normative and the §11.3
precision-over-recall principle. Granularity remains authored judgment with D19's guidance +
`doctor` observability; D23's quote guard already catches the practical symptom (degenerate
spans). Record this decision in PRD §14 so the idea does not resurface.

### D28 — Store schema v2: strict models, version-gated load, loud failure, no shim

**What to do, in order:**

1. `src/core/model.ts`: `MODEL_VERSION` `"v1"` → `"v2"`.
2. Every `z.object({...})` in `src/core/model.ts` and `src/resolver/manifest.ts` becomes
   `z.strictObject({...})` (or `.strict()` — one mechanism, applied uniformly). Note: on
   `Assertion`, strictness must be applied to the object *before* the existing `.refine(...)`
   (a `.refine` returns a `ZodEffects`, which has no `.strict()`). `z.record(...)` fields
   (`attrs`, `evidenceBaseline`, `suppressed.paths`) keep their open value types — records are
   not objects; do not change them.
3. `src/store/store.ts`: on load, before parsing any claim file, read `config.json` and require
   `version === MODEL_VERSION`. On mismatch, fail with exactly:
   `this store was written by hibi model <found> and this binary requires <MODEL_VERSION>. hibi ships no migration (beta): re-run 'hibi init' and re-record, or use a matching hibi version.`
   Delete/rewrite the stale comment near `store.ts` L228–232 and the "may predate schema v2"
   error at ~L243 to match the new reality (version detection is now real).
4. `doctor` (`src/engine/doctor.ts` + its renderer): report the store version string in
   `DoctorReport` (new field `storeVersion: string`).
5. Regenerate schemas: `bun run build:schemas` must emit `schemas/*.v2.json` (verify
   `scripts/gen-schemas.ts` derives filenames from `MODEL_VERSION`; if hard-coded, fix the
   script). Delete the `*.v1.json` files. Regenerate/adjust both SDKs (`sdk/ts`, `sdk/rust`)
   until they compile; CI's schema-freshness and rust-sdk jobs must pass.
6. The repo's own `.claims/` store is re-initialized and re-recorded in Phase 3 (D32) — do not
   hand-edit its JSON to "v2".

*Grounding.* Owner statement: "we don't care about backward compatibility… rewrite > migration."
ADR-002 Phase 1 promised loud v1 detection and never delivered the mechanism; strict parsing is
what makes "the store cannot silently misrepresent what was recorded" true.

*Declined:* auto-migration shim (no corpus; dead surface per §11.5); permissive load with a
doctor warning (the store staying able to lie is the failure itself).

### D29 — Advisory provenance: schema field + registry enforcement

**What to do:**

1. `src/core/model.ts`, `Advisory` (~L433): add optional field
   `provenance: z.strictObject({ model: z.string(), promptHash: z.string(), contextHash: z.string(), params: z.record(z.string(), z.unknown()).optional() }).optional()`.
2. `src/resolver/manifest.ts`, `ResolverSpec`: add `modelBacked: z.boolean().default(false)`.
3. `src/resolver/registry.ts`, in the advisory handling path (where a declared-advisory
   resolver's verdicts are already dropped, ~L133): when the resolver's spec has
   `modelBacked: true`, drop every advisory lacking `provenance`, and surface one stderr warning
   per run per resolver, verbatim:
   `dropped <N> advisories from <resolver>: modelBacked resolvers must attach provenance (model, promptHash, contextHash).`
4. Document the field in `docs/resolvers.mdx` (security-model section).

*Grounding.* The behavioral research's noise-control list is normative ("No hidden LLM state…
must be recorded if an LLM plugin is used"). PRD §19 promises it in prose; the wire schema
cannot carry it, so the quarantine is unenforceable. Cheapest to add before any third-party
advisor exists.

### D30 — `hibi coverage --doc <p> --fail-uncovered`

**What to do.** Add a boolean `--fail-uncovered` flag to the `coverage` verb
(`src/cli/index.ts`, case `"coverage"`). When set and `result.summary.uncoveredBlocks > 0`, the
command exits with the gating exit code **2** (reuse the same constant/pathway `check` uses for
gating — not `EXIT_OPERATIONAL_ERROR`). JSON and human output are unchanged except the exit
code. Without the flag, behavior is exactly as today (exit 0).

*Grounding.* Owner usage intent 2 (verify built code against a pre-existing plan): "the plan
must be fully grounded" becomes CI-enforceable — uncovered blocks are unimplemented or
unpruned plan items.

*Declined:* a `--min <pct>` threshold flag (percentage targets invite gaming and have no
principled value; full grounding is the only meaningful gate for a plan doc — add thresholds
only if real users ask).

### D31 — Workflow set: restructure into Author / Verify / Maintain / Prune; four new moments

**What to do.** Rewrite `docs/workflows.mdx` so the moments are grouped by intent. Keep the six
existing moments' content (updated for this ADR's features); add four new moments. The complete
target set (titles may be polished; commands are contracts):

**Author**
1. *Write grounded docs for a fresh feature* (new): write the doc → `hibi coverage --doc
   <new-doc>` lists ungrounded blocks → author the claim set → `hibi record --from-file
   claims.json` (transactional) → done when coverage is clean. Behavioral sentences get
   `command:` verifiers at authoring time.
2. *Verify built code against a pre-existing plan* (new): `hibi coverage --doc plan.md
   --fail-uncovered` in CI. State the honesty rule verbatim: hibi never judges whether code
   implements a sentence — the author/agent judges by *anchoring* each promise to its
   implementing code (a sentence you cannot anchor is an unimplemented plan item); hibi makes
   the judgment enforceable and guards it afterward. Behavioral promises get verifiers;
   `hibi check --run-verifiers` proves them.

**Verify / Maintain** (existing, updated)
3. Trust-check before following an instruction file (`hibi status --doc`).
4. Keep docs honest in the same PR (`hibi diff --since`).
5. Blast radius before a refactor (`hibi query --path`).
6. Onboard an existing repo (`hibi init` + `coverage`).
7. Consolidate a doc (`hibi relocate`).
8. Silence a known-good at-risk (`hibi ignore`).
9. *Recover an orphaned claim* (new): `hibi reanchor <id> --suggest` → inspect candidates →
   `hibi reanchor <id> --doc-range …` (attestation rules apply) or `hibi retire <id>`. Note the
   pure-move case: a byte-identical span found elsewhere re-anchors without trust downgrade
   (D25).

**Prune**
10. *Prune the ungrounded* (new): two deterministic signals — `hibi coverage --doc <p>`
    uncovered blocks (never grounded: ground or cut) and `hibi list --state orphaned` code-side
    orphans (grounding died: retire the claim, cut the sentence). State the caveat verbatim:
    "uncovered" means "no claim recorded," not "no code backs it" — hibi provides the worklist,
    the author makes the prune call.

Also update moment 8 (ignore) and the triage section for D24/D25 remediation text changes. The
agent-facing skill (`plugins/hibi-cli/skills/hibi/` — SKILL.md, `references/cookbook.md`,
`references/cli-reference.md`) gains matching recipes for the four new moments and the new
flags, **in Phase 4 only** (the skill tracks the shipped CLI, never the aspirational spec).

### D32 — Dogfood hardening: the behavioral tier checks itself

**What to do (Phase 3):**

1. Re-initialize `.claims/` as a v2 store and re-record all existing dogfood claims, with
   `ref` = the full commit SHA at record time (`git rev-parse HEAD`), never a branch name.
2. Add **two behavioral claims with command verifiers** (owner `hibi-dogfood`,
   `enforcement: enforced`, `behavioral: true`):
   - A sentence in `docs/behavioral.mdx` describing the change-gate, anchored to
     `computeBehaviorRisk` in `src/algo/resolve.ts`, with verifier
     `{ "kind": "command", "ref": "bun test test/behavioral-gate.test.ts", "proves": "the gate fires on evidence drift while the anchored span is untouched" }`.
   - The precision-contract sentence in `docs/verdicts.mdx` (≤2% false-changed), anchored to the
     rate constants in `src/algo/params.ts`, with verifier
     `{ "kind": "command", "ref": "bun test test/precision-rate.test.ts", "proves": "the ≤2% false-changed precision contract holds" }`.
3. `.github/workflows/ci.yml`, dogfood step: the existing `check … --fail-on gating` invocation
   adds `--run-verifiers`.
4. `doctor` gains a thin-evidence observability row: new `DoctorReport` field
   `thinEvidenceBehavioral: { assertionId: string; evidencePaths: number }[]` plus
   `counts.thinEvidenceBehavioral`, listing behavioral claims whose `evidenceBaseline` contains
   ≤ 1 path (the gate is watching almost nothing — suggest `behaviorScope.include` globs or
   `depth`). **Observability only: it does not affect `healthy` and never gates.**
5. New regression test `test/no-taxonomy.test.ts` (the fitness function ADR-002 described but
   never got): parse the generated v2 JSON schemas and assert (a) the Assertion schema has no
   property named `claimKind`; (b) `Verifier.kind` is `{"type":"string","minLength":1}` with no
   `enum`; (c) the `Enforcement` enum is exactly `["suggested","enforced","retired"]`.

*Grounding.* ADR-002 D20's own standard: the class of drift hibi polices must be caught by hibi,
not by review. The behavioral tier and the runner were the one shipped subsystem with no live
self-check; Appendix C's "evidence-set silent-miss" warning lands here as observability.

---

## Roadmap

Each phase ends with `bun test` green and CI green. Do not start a phase before the previous
one's acceptance criteria (AC) pass.

**Phase 0 — Record & retract (docs only, no code).**
Commit this ADR. PRD: add D22–D32 to the §14 decision log (one-paragraph summaries each,
pointing here); purge/rewrite every "structural path" passage (D22); add the D26 deferred
test-mapping spec + the D22 reopen trigger to §19. Docs: `anchors.mdx` + `concepts.mdx`
retractions (D22).
AC: `rg -n -i "structural path" docs/ PRD.md README.md` → zero matches outside `design/` and
CHANGELOG history. Docs describe exactly two-selectors-plus-optional-inline-id on the doc side.

**Phase 1 — Schema v2 (D28, D29; one breaking pass).**
Model version bump; strict objects; store-load version gate with the verbatim error; `Advisory.
provenance`; `ResolverSpec.modelBacked`; registry drop rule with the verbatim warning; doctor
`storeVersion`; regenerate `schemas/*.v2.json` (delete v1); both SDKs compile.
AC: a fixture store with `config.json` `{"version":"v1"}` fails to load with the verbatim D28
message; a claim JSON with an extra key `claimKind` fails Zod parsing; a `modelBacked` resolver
returning a provenance-less advisory yields the verbatim warning and no advisory in the report;
schema-freshness CI check passes.

**Phase 2 — Engine (D23, D24, D25, D26-adopted, D30).**
Quote guard in record/reanchor/from-file; `reanchor --suggest` + remediation command;
D25 exact-match exception; `src/engine/test-suggest.ts` + `RemediationInput.suggestedTests` +
check wiring; `coverage --fail-uncovered`.
AC (each is a test):
- record with a 7-char quote fails with the verbatim length-floor message; record of a
  twice-occurring sentence whose context cannot disambiguate fails with the verbatim ambiguity
  message; a `--from-file` batch containing one such spec writes nothing.
- `reanchor <id> --suggest` on an orphaned fixture prints ranked candidates, exits 0, and the
  store directory is byte-identical before/after.
- reanchor without `--ref` on a pure-move fixture (identical sentence at a new offset, code
  unchanged) retains `authoredTrust: verified` and writes no `reanchorDowngrade`; the same
  reanchor on a reworded fixture downgrades (both branches asserted — extends
  `test/attestation.test.ts`).
- a behavioral at-risk claim with empty `verifiers[]` whose anchored file is imported by a
  fixture test file gets that test path in the declare-verifier remediation rationale; a claim
  with declared verifiers gets none.
- `coverage --doc <fixture> --fail-uncovered` exits 2 with ≥1 uncovered block and 0 when fully
  covered.
- a new invariant test asserts `check` (all flag combinations, including `--write` and
  `--run-verifiers`) leaves `.claims/` byte-identical.

**Phase 3 — Dogfood (D32).**
Store re-init as v2; re-record with SHA refs; the two behavioral+verifier claims; CI
`--run-verifiers`; doctor thin-evidence row; `test/no-taxonomy.test.ts`.
AC: repo CI runs `hibi check --run-verifiers … --fail-on gating` green; deliberately changing a
constant in `src/algo/params.ts` turns CI red (retained from ADR-002 Phase 6); deliberately
breaking `test/behavioral-gate.test.ts` makes the dogfood step exit 2 via `refuted`;
`doctor --json` exposes `storeVersion` and `thinEvidenceBehavioral`.

**Phase 4 — Docs + skill sweep (D31 + every page a prior phase touched).**
`workflows.mdx` restructure (Author/Verify/Maintain/Prune, ten moments, verbatim caveats from
D31); `cli-reference.mdx` (`--suggest`, `--fail-uncovered`, provenance, `modelBacked`);
`anchors.mdx` (quote guard, D25 exception); `verdicts.mdx` (suggestedTests in remediation);
`lifecycle.mdx` (reanchor attestation update); `resolvers.mdx` (provenance/modelBacked);
`ci.mdx` (`--run-verifiers` dogfood pattern, `coverage --fail-uncovered` plan gating);
`quickstart.mdx` if verb summaries changed. Skill update (SKILL.md + references + cookbook
recipes for the four new moments) happens in this phase and no earlier; the new/updated skill
statements join the dogfood claim set where they bind CLI facts.
AC: every flag added in Phases 1–3 appears in `cli-reference.mdx` and the skill's
`references/cli-reference.md`; `hibi check` over the repo's own store is green.

Release: cut **0.5.0** after Phase 4 (breaking: schema v2).

---

## Fitness functions (add to the retained ADR-001/ADR-002 set)

- **Version invariant:** loading a store whose `config.json` version ≠ `MODEL_VERSION` fails
  with the D28 message (test).
- **Strictness invariant:** any unknown key in any stored object fails the parse (test with an
  injected `claimKind`).
- **Retraction invariant:** `rg -i "structural path"` finds nothing in `docs/`, `PRD.md`,
  `README.md` (CI grep or test).
- **Quote-guard invariant:** the two verbatim D23 rejections are asserted, at `record`,
  `reanchor`, and `record --from-file` (batch writes nothing).
- **Suggestion-only invariant:** `reanchor --suggest` leaves `.claims/` byte-identical.
- **Attestation invariant (amended):** without `--ref`, trust downgrades **unless** the D25
  exact-match+code-unchanged exception holds; both branches tested.
- **Check-purity invariant:** no flag combination of `check` mutates `.claims/`.
- **Provenance invariant:** a `modelBacked` resolver's provenance-less advisory is dropped with
  the D29 warning; a provenance-carrying one passes through.
- **Coverage-gate invariant:** `--fail-uncovered` exits 2 iff `uncoveredBlocks > 0`.
- **No-taxonomy invariant (now a real test):** `test/no-taxonomy.test.ts` as specified in D32.
- **Advisory-suggestion purity:** `suggestedTests` never appears on a verdict for a claim with
  declared verifiers, and its presence never changes exit codes.

## Consequences

- The store finally cannot lie about what was recorded (strict v2, version-gated), at the cost
  of a breaking release — accepted explicitly (beta, rewrite > migration).
- The doc side is *honestly* format-agnostic: the docs now describe the shipped bundle, and the
  retraction is evidence-recorded rather than silent.
- Drift repair gains ergonomics without gaining gaming surface: suggestions are read-only, and
  the only attestation-free re-anchor is the provably-evidence-neutral one.
- The behavioral tier now checks itself in this repo's CI; a broken gate or runner turns the
  build red without human review.
- Net new CLI surface: two flags (`--suggest`, `--fail-uncovered`). Net schema surface: two
  optional fields (`Advisory.provenance`, `ResolverSpec.modelBacked`), one report field
  (`storeVersion`), one doctor row (`thinEvidenceBehavioral`). Everything else in this ADR is
  either subtraction (retractions, declines) or docs.

---

## Appendix — Evidence gathered for this ADR (2026-07-08)

**A. Literature/production survey: structural vs text anchoring** (full report with sources:
`design/evidence/ADR-003/structural-vs-text-anchoring-survey.md`): No study quantifies a re-attachment gain from adding
structural selectors to a quote+context bundle in text documents. Brush et al. (CHI 2001 /
MSR-TR-2000-95, MSR-TR-2001-107): pure content matching found 100% of moved-but-unchanged
anchors; users increasingly preferred orphaning over rescue as text changed (+0.72 correlation);
their follow-up algorithm deliberately ignores document structure and beat text search.
Hypothes.is fuzzy anchoring: XPath range is a fast path for unchanged docs, never the robustness
mechanism; their 20,953-annotation orphan study measured attachment by text search alone.
Phelps & Wilensky (WWW9 2000) is the strongest pro-structure result (742/754 repositioned) but
small, self-run, un-ablated — and their own paper notes sibling insertion invalidates tree walks
(a markdown block-index's exact failure mode). Production convergence: Notion (block UUIDs),
Obsidian (`^block-id`; heading links are the known-broken feature), Wikipedia (Template:Anchor +
bot fleet for broken section links), GitBook (documents heading-anchor breakage), Swimm (content
signals), Hypothes.is (quote+context) — content matching for re-attachment, assigned IDs for
durability, explicit orphaning below confidence.

**B. Empirical anchor-survival experiment on this repo** (full tables:
`design/evidence/ADR-003/anchor-survival-experiment.md`; reproducible script:
`design/evidence/ADR-003/anchor-survival-experiment.ts`; method: replay 6 heavy doc-rewrite commit pairs, 33 file pairs, N=2,501 old-version
sentences; simulate hibi's exact anchor per sentence — quote + 48-char context + position — and
re-attach in the new version using hibi's actual `localizeTextQuote`, cross-checked with a
bigram-Dice approximation; compute heading trails and test rescue/mislead): exact-unique 81.3%;
ambiguous 1.4%, of which the 48-char context resolved 100% (all ambiguity was degenerate short
fragments); fuzzy-rescued 11.0%; orphaned 6.3%. Heading trails survived 92.1% overall but 0% in
the README rewrite and 61% in the SKILL.md restructure; heading-path rescue 21.7% of orphans
(1.4% of all sentences) vs 0.9% mislead + 22.3% false-attach risk on genuine deletions. A
human-judged sample of orphans split ~60/40 reworded-with-semantic-change vs truly deleted —
i.e., a large share of would-be rescues are cases where flagging is correct.

**C. Test-impact / coverage-artifact survey** (full report with sources:
`design/evidence/ADR-003/test-impact-coverage-survey.md`):
lcov/istanbul formats carry no per-test attribution (lcov `TN:` labels a whole run); coverage
artifacts are normatively gitignored (GitHub Node template excludes `coverage`, `*.lcov`,
`.nyc_output`); no surveyed system operates from committed snapshots. Per-test mapping systems
all instrument fresh per run (pytest-testmon, Datadog TIA, Teamscale testwise, Azure TIA — the
latter two with run-all fallbacks because maps drift); static-graph selectors (Jest
`--findRelatedTests`, Vitest `--changed`, Nx affected, Bazel) recompute from the current tree;
requirements-traceability practice (Doorstop, sphinx-needs) links requirements to tests via
manually assigned IDs — the shape of hibi's declared verifier. Static RTS coarsening alone
produces ~5.9% missed-test safety violations (Legunsen et al., FSE 2016); stale snapshot maps
are strictly worse.
