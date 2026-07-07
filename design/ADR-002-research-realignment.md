# ADR-002 — Research realignment: the behavioral tier, the carrier, and trust hardening

Status: proposed (awaiting owner confirmation; decisions drafted from research synthesis)
Date: 2026-07-07
Deciders: project owner (review delegated to research synthesis)
Amends: PRD §4, §5, §6, §7.1, §8, §9, §10, §13, §14, §17.6
Inputs: PRD.md; docs/ (15 pages); the parallel.ai research corpus (9 runs — urls in
`docs/researches.local.md`); a full implementation audit of `src/` at v0.2.3.

## Context

A comprehensive design review compared three artifacts that are supposed to agree — the PRD, the
published docs, and the shipped implementation — against the research corpus that the PRD claims as
its evidence base. Three headline findings force this revision:

1. **Tier 3 (behavioral) is a facade in the shipped code.** `behaviorScope` is persisted by
   `record` but nothing reads it — no reachability walk exists; the change-gate in
   `src/algo/resolve.ts` fires `at-risk` only when `code ∈ {changed, orphaned}`, code evidence
   changed, or `doc == changed` — exactly the conditions Axis 1 already flags. And no verifier
   runner ships, so `supported`/`refuted` are unreachable in a stock install. The tier's entire
   reason to exist — *behavior goes false while the anchored span is untouched* — is the part that
   is not implemented. Docs promising change-gated behavioral routing that does not exist is
   precisely the trust failure hibi exists to prevent.

2. **The fixed `claimKind` enum is an over-interpretation of the research.** No report in the
   corpus proposes a fixed behavioral taxonomy. The canonical behavioral report treats
   "sort/retry/cache/validate/thread-safe/O(n)" as *keyword triggers for a binary
   behavioral-candidate flag*, with `claim_kind` illustrated as an open tag list. The bonus report
   is explicitly adversarial: the only system with strong numbers over a fixed taxonomy (BIV,
   F1 0.946) operates on a closed 29-capability skill domain, and the report's stated takeaway is
   *"taxonomic approaches scale poorly to open-ended doc prose; hibi's users write unstructured
   claims."* The shipped 7-value enum (`ordering/retry/complexity/concurrency/caching/validation/
   error-handling`) has no per-kind semantics anywhere in the engine — its only consumed bit of
   information is "this claim is behavioral". A closed enum with implied per-kind behavior the
   engine ignores misleads authors.

3. **The PRD is the stale artifact.** It declares itself "final and complete" but has been
   superseded by the implementation on: resolver RPC method names (`localize`/`detect` →
   `describe`/`resolve`/`verify`), resolver purity (the engine now hands file contents to pure
   resolvers), the text-quote context window (32 → 48 chars), the claimKind count, and the CLI
   surface (8 verbs → ~18). For a tool whose thesis is "documents must not silently outlive the
   thing they describe," the spec drifting silently from the code is disqualifying.

Secondary findings folded into this revision: the compact instruction-file banner is promised as
load-bearing but `StoreConfig.instructionFiles` is dead code; the two carrier research runs
genuinely disagree on anchoring granularity (atomic-sentence vs paragraph-default) and the PRD
silently picked the aggressive side; `reanchor` resets state with no attestation — exactly
Fiberplane's admitted relink-to-clear-CI gaming hole; `unanchored-legacy` and `--text` (Model A)
are reserved surface no code path produces and no user needs (alpha, zero migration corpus);
verifier kinds diverge from the research's evidence-type list without recorded rationale.

No backwards-compatibility constraint applies (alpha; explicit owner statement). Complete rewrites
of modules are in scope.

## Decisions

### D12 — `claimKind` fixed enum → `behavioral` boolean flag

**Replace** `Assertion.claimKind?: enum(7)` **with** `Assertion.behavioral?: boolean`:

- **absent** → the deterministic keyword heuristic classifies (as today, but returning a boolean
  *behavioral-candidate*, not a kind). Heuristic buckets, per the bonus behavioral report:
  keyword list ("sort", "retry", "O(n)", "cache", "idempotent", "thread-safe", "validate"),
  comparison/ordering language, temporal/sequencing language, exception/error language.
- **`true`** → behavioral regardless of wording (author declaration wins).
- **`false`** → author opt-out; the heuristic is skipped entirely. *New capability* — today a
  false-positive heuristic match cannot be silenced.

A claim is behavioral iff `behavioral === true`, OR (`behavioral` absent AND heuristic matches),
OR `verifiers[]` is non-empty. **`behavioral: false` requires `verifiers[]` to be empty** — a
verifier is itself a behavioral declaration (the strongest one: an action, not a label), so the
combination is a contradictory record, rejected by a Zod schema refinement at `record` time and at
store load (never resolved by silent precedence: `false`-wins would leave dead, never-run
verifiers — the inert-config pattern this ADR purges — and verifiers-wins would make the stored
flag a lie). The record-time error points at the legitimate levers for "keep the verifier, silence
the noise": narrow `behaviorScope` or `hibi ignore`. Authors who want to label the *kind* of
behavior may use the
existing open `attrs` bag (e.g. `attrs.kind = "retry"`); the engine does not interpret it, and the
schema does not pretend it does.

CLI: `--claim-kind <k>` → `--behavioral` / `--no-behavioral`.

*Grounding:* canonical behavioral report ("explicitly declared by the author, suggested by a
classifier, or inferred by a deterministic keyword heuristic" — classification is a soft input,
"a classification step, not a verdict step"); bonus report's anti-taxonomy finding quoted in
Context. *Declined:* an open-string `claimKind` — a kind label with no engine semantics is a
misleading contract; `attrs` already carries uninterpreted labels honestly.

### D13 — Verifier kinds become open strings; a built-in `command` runner ships; verifiers run only on explicit opt-in

- `Verifier.kind: z.enum([example, snapshot, contract, property, formal, command])` →
  `z.string().min(1)`. Kinds are a **resolver-matching key**, not a core taxonomy: a runner
  resolver declares the `verifierKinds` it handles; the engine dispatches by string match. Docs
  list *conventional* kinds (`command`, `example`, `snapshot`, `contract`, `property`,
  `metamorphic`, `formal`) as recommendations, not schema.
- **A built-in `command` runner ships in-tree** (an out-of-process resolver like every other): it
  executes `ref` as a child-process command with a timeout; exit 0 → `supported`, non-zero →
  `refuted`. This makes the only gating-eligible behavioral state (`refuted`) reachable in a stock
  install — closing facade-gap #3.
- **Security model (normative):** verifiers execute repo-committed commands, so they are a
  supply-chain surface. They run **only** under `check --run-verifiers` (never on `status`,
  `query`, or plain `check`), and external runner resolvers still require the default-deny
  manifest. An agent-hook or read-time gate must never trigger arbitrary command execution.

*Grounding:* canonical behavioral report — evidence types are open ("example, snapshot, contract,
property, metamorphic, manual"; formal explicitly "a supported external evidence type, not a
required built-in tier"; core "stays evidence-agnostic"); execution-grounding scored 420/500, the
highest-trust option. The shipped 6-kind enum matched no source and classified things that could
never run. *Declined:* shipping six runner integrations (huge surface, zero users); a `manual`
kind (manual attestation is what `authoredTrust: verified` + `--ref` already is).

### D14 — Change-gate v2: file-level import reachability with stored evidence baselines

The gate finally detects what Axis 1 cannot: evidence changes *outside* the anchored span.

- **`behaviorScope` is redefined** (the call-graph fields were never read):
  `{ rootSymbols[], reachableDepth, include[], exclude[] }` →
  `{ include?: glob[], exclude?: glob[], depth?: 0|1|2 }` (default depth **1**).
- **Evidence set** for a behavioral claim, computed at check time: the anchored node + the
  anchored file + files imported by the anchored file to `depth` (import extraction via the
  existing tree-sitter grammars — deterministic; no call graph) + `include` globs (config files,
  fixtures) − `exclude` globs + declared verifier source paths where resolvable.
- **Baselines:** the Assertion stores `evidenceBaseline: { path → xxHash64 }`, captured at
  `record`/`reanchor` time — consistent with "the anchor carries its own baseline; git is not on
  the verdict path" (§6). At check time the evidence set is recomputed from *current* imports;
  a file whose hash differs, **or that has no stored baseline** (a newly added import — itself a
  change), counts as changed evidence.
- **Firing rule:** `at-risk` iff the claim is behavioral AND (the anchored node's semantic hash
  changed OR any evidence file changed OR a verifier source changed). **`doc:changed` is removed
  from the gate** — a doc-side edit is Axis 1's job (the sentence changed), not withdrawn support
  for the behavior; today it double-fires both axes on one signal.
- **Noise controls (normative, from the canonical report):** one banner entry per claim per
  change-set; every `at-risk` names the changed evidence path and suggests a verifier; a
  suppression verb `hibi ignore --claim <id> --reason <text>` (records the acknowledged
  `{path → hash}` map of the currently-changed evidence plus the required reason; the suppression
  lapses automatically when any acknowledged path's hash moves again or a new evidence path
  appears — a single `--until <hash>` would be ambiguous across multiple changed paths; while
  active, the at-risk contributes nothing to exit codes, including under `--fail-on warn`, and is
  surfaced as `suppressed: true` in JSON); `doctor` reports the behavioral flag-rate, with the
  research's rollback trigger stated: if >30% of behavioral claims flag on a typical commit,
  tighten the gate.

*Grounding:* change-gated routing 395/500; bonus report — "direct + one transitive level,
configurable"; static call graphs "miss dynamic dispatch, reflection, indirect calls" so
file-level dependency fallback is the honest mechanism; canonical report — scope must include
non-call edges (config, literals, schemas), hence `include` globs. *Declined:* full call-graph
reachability over `rootSymbols` (poor ground truth per the research, high complexity, silent
selection errors); keeping the inert fields.

### D15 — `reanchor` attestation (anti-gaming)

`reanchor` currently rewrites selectors and resets state to `unchanged` with no evidence — the
exact relink-to-clear-CI hole Fiberplane admits to. Now:

- `reanchor --ref <new-ref>` asserts re-verification: selectors and evidence baselines refresh,
  `authoredTrust` is retained, the new `ref` is recorded.
- `reanchor` **without** `--ref` still re-anchors, but **downgrades `authoredTrust`
  `verified` → `inferred`** and records the downgrade in the assertion — the claim is findable
  again, but nobody has re-attested that it is *true*. The banner/status surfaces the downgrade.

*Grounding:* bonus carrier report names Fiberplane's failure mode as a direct warning "for any
hibi reanchor/acknowledge command." *Declined:* blocking reanchor without `--ref` (too hostile to
the doc:moved repair loop, which is legitimately evidence-free).

### D16 — Carrier simplification: pure Model C; Model A, `--text`, and `unanchored-legacy` are removed

The store is bidirectional sidecar anchors + optional inline IDs, full stop. `record --text`, the
Model-A migration path, and the `unanchored-legacy` enforcement member are deleted from schema,
CLI, and docs. PRD §18-B pre-authorized this: "if pristine-doc tracking proves rare, drop the
Model-A fallback and simplify to pure C." There is no migration corpus (alpha; zero external
users); pristine/read-only docs are still trackable — sidecar anchors point *into* a file without
modifying it (banner policy for them is D17).

*Declined:* keeping the reserved surface "just in case" — dead enum members and legacy flags are
exactly the AI-slop §11.5 bans.

### D17 — Pristine-document banner policy: never stamp files hibi does not own

A Document may be marked **pristine** (`record --pristine`, or store-config globs, e.g. vendored
docs, third-party specs). `check --write` never stamps a banner or frontmatter into a pristine
document; its verdicts surface only via JSON/`status`/exit codes. The threat model is served by
the read-time gate (`hibi status --doc`) for these files.

*Grounding:* canonical carrier report, verbatim requirement: "do NOT stamp banners into files hibi
cannot modify"; sidecar + external reports instead.

### D18 — Compact instruction-file banner: implement it (it is load-bearing, not optional)

`StoreConfig.instructionFiles` (dead today) becomes read: documents matching it (default globs:
`CLAUDE.md`, `AGENTS.md`, `.cursorrules`, `.github/copilot-instructions.md`, configurable) get the
**single-line pointer banner** (`STALE — N claim(s); run \`hibi status --doc <p>\``) instead of
the full block. The attention-budget research this rests on is unambiguous: added bytes in an
always-loaded instruction file dilute instruction-following, and repeated boilerplate is learned
as noise.

### D19 — Granularity guidance + orphan-rate observability

The two carrier research runs disagree: canonical wants atomic sentence-level propositions; bonus
recommends paragraph-level default because sentence anchors are the least robust and no production
system anchors at sentence level. Resolution:

- `record` already accepts arbitrary spans; **docs stop prescribing sentence-only** and present
  the tradeoff: atomic sentence for enforced, gating claims (one independently verifiable
  proposition per record); paragraph spans are first-class for lower-stakes coverage.
- The research's kill-switch metric becomes observable: **`doctor` reports doc-side
  orphaned/moved/changed rates**. The documented escape hatch: if doc-side orphan rate exceeds
  ~30% for typical edits, require inline IDs (or structural-path-first anchoring) for
  high-severity claims.
- The shipped hardening stays and gets documented (it was silent): a single near-exact
  `text-quote` match (similarity ≥ 0.9) satisfies the two-selector minimum on its own; quotes
  shorter than 8 chars are ineligible for `ambiguous`.

### D20 — The PRD is demoted; docs + schemas are the living contract; hibi dogfoods itself

- `PRD.md` drops its "final, complete, sole input" claim and becomes what it actually is: the
  **design record** — problem, principles, decision log, evidence. The behavioral contract lives
  in `docs/` + `schemas/*.v1.json` + the test suite (`test/precision-rate.test.ts` is the §10
  precision contract, executable).
- **Dogfooding:** hibi runs on its own repo — claims recorded binding `docs/` statements (enums,
  exit codes, grading constants) to `src/core/model.ts` / `src/algo/params.ts`, wired into CI.
  The 32-vs-48 drift this review caught is exactly the class of bug hibi exists to catch; it
  should have been caught by hibi.

## Alternatives considered (whole-review level)

- **Cut Tier 3 entirely** (research score 360/500 — "honest but leaves the gap"): seriously
  considered given the facade finding; rejected because D13+D14 are a modest, well-scoped build
  (a command runner + import extraction over grammars already shipped) and execution-grounding +
  change-gated routing were the two *highest*-scored options in the corpus (420, 395). Cutting
  would abandon the product's stated differentiator right when its missing pieces are cheap.
- **Keep the facade** — rejected as the dishonest middle; it is the trust failure the product
  polices.
- **Gating LLM / advisory-LLM-in-core** — re-affirmed declined (METAMON F1 0.58; judge consistency
  0.57 under sampling; 63% wrong-answer acceptance). Unchanged from D5/§18-A.
- **Paragraph-only carrier** — rejected; span choice is the author's, with observability (D19)
  instead of a forced default.

## Roadmap

Phased so each layer is validated before the next (per §13 discipline). "AC" = acceptance criteria.

**Phase 0 — Record & repair (this ADR's commit).**
Doc bug fixes: `lifecycle.mdx` proposition-id format; `ci.mdx` stale action pin;
`anchors.mdx`/`verdicts.mdx` document the strong-quote exception and the 8-char ambiguity floor;
PRD amendments (header, D12–D20 in §14, stale facts: RPC names, 48-char, resolver purity, verb
list, `--text`/`unanchored-legacy` removal, §17.6 v2). AC: docs and PRD no longer contradict the
code or each other except where this roadmap says "not yet implemented."

**Phase 1 — Schema v2 (one breaking pass; regenerate `schemas/` + both SDKs).**
`behavioral?: boolean` replaces `claimKind` (D12); `Verifier.kind` open string (D13);
`behaviorScope` redefined + `evidenceBaseline` added (D14); `unanchored-legacy` removed from
`Enforcement`, `--text` removed (D16); `Document.pristine` (D17). No migration shim — `hibi init`
fresh stores; `doctor` detects v1 stores and says so plainly.
AC: `bun run gen-schemas` clean; model tests updated; Rust SDK compiles.

**Phase 2 — Engine.**
Gate v2 in `src/algo/resolve.ts` + import extraction in `src/ast/` (per-grammar import-node map,
sibling of `value-map.ts`) + evidence hashing at record/reanchor (D14); reanchor attestation
(D15); pristine skip in `check --write` (D17); compact banner path in `src/banner/banner.ts`
reading `instructionFiles` (D18).
AC — **the anti-facade invariant (new fitness test):** a fixture where the anchored span is
untouched but an imported file changed must yield `code:unchanged · behavior:at-risk`. Plus:
neutral edits in unrelated files never fire `at-risk` (extend `precision-rate.test.ts`); reanchor
without `--ref` downgrades trust; pristine doc bytes untouched by `check --write`; instruction
file gets a one-line banner.

**Phase 3 — Verifier runner (D13).**
Built-in `command` runner resolver + `check --run-verifiers`; `run-verifier` remediation entry
un-stubbed. AC: e2e — a claim with `command:bun test retry` reaches `supported` on pass and
`refuted` (exit 2) on fail; plain `check`/`status` never spawn verifier processes.

**Phase 4 — Workflow hardening.**
`hibi ignore --claim --reason` suppression verb (acknowledged `{path → hash}` map, auto-lapse);
`doctor` metrics: behavioral flag-rate (30% trigger) and doc-side orphan/moved rates (D19's
kill-switch observability).
AC: doctor JSON exposes both rates; ignore records reason + acknowledged hashes in the store.

**Phase 5 — Docs alignment pass** (each page updated in the phase that changes its behavior; this
phase is the sweep): `behavioral.mdx` (D12/D13/D14 — full rewrite of the claim-kind section),
`cli-reference.mdx` (flags), `concepts.mdx`, `anchors.mdx` (behaviorScope), `verdicts.mdx` (gate
semantics), `banners.mdx` (compact + pristine), `resolvers.mdx` (runner + security model),
`lifecycle.mdx` (reanchor attestation), `workflows.mdx`/`ci.mdx` (`--run-verifiers`, `ignore`),
`quickstart.mdx`. Fix `src/algo/localize.ts`'s stale "32 chars" comments while there.

**Phase 6 — PRD demotion + dogfooding (D20).**
Rewrite the PRD header/§14 framing as the design record; record hibi claims on hibi's own docs
(enums → `model.ts`, constants → `params.ts`, exit codes → `gating.ts`) and add `hibi check` to
this repo's CI.
AC: a deliberate constant change in `params.ts` turns the repo's own CI red via hibi.

## Fitness functions

- **Anti-facade invariant:** `behavior:at-risk` must be reachable while `code:unchanged` (import
  drift fixture). Tier 3 must detect strictly more than Axis 1 or it does not ship.
- **No-taxonomy invariant:** the model exports no closed enum of behavioral kinds; `Verifier.kind`
  is an open string. (Guards against the enum quietly returning.)
- **No-contradiction invariant:** `Assertion.parse` rejects `behavioral: false` with a non-empty
  `verifiers[]`; a test asserts both the `record`-time error and the store-load rejection, and
  that the error message names the two legitimate noise levers (`behaviorScope`, `hibi ignore`).
- **Verifier safety invariant:** no code path outside `check --run-verifiers` spawns a verifier
  process (test asserts the runner is never invoked on `status`/`query`/plain `check`).
- **Attestation invariant:** `reanchor` without `--ref` never leaves `authoredTrust: verified`
  intact.
- **Pristine invariant:** `check --write` over a pristine doc is byte-identical on disk.
- **Existing invariants retained:** ADR-001's parallelism/no-collision/gating tests; the ≤2%
  false-`changed` precision contract; drift never graded `unchanged`.

## Consequences

- The behavioral tier becomes real: `at-risk` gains detection power Axis 1 lacks, and
  `refuted`/`supported` are reachable without third-party resolvers.
- Schema v2 is breaking (alpha; no shim). SDKs and generated schemas regenerate; consumers of
  `claimKind`/`unanchored-legacy` must move.
- The PRD stops being load-bearing; drift between spec and code becomes a CI failure via
  dogfooding rather than a review finding.
- Surface *shrinks* net: one enum deleted, one enum member deleted, one legacy flag deleted, two
  dead schema fields replaced by two consumed ones.
