# Claim Engine — PRD (final-product design)

> A standalone, **agent-facing** CLI (with a small reusable library core) that keeps a
> codebase's documentation and AI-agent-instruction files from silently going stale — so that
> automated agents never read a **superseded or outdated** document and act on it as if it were
> current.
>
> **Status:** final-product design ("what it looks like when it's done"). Every decision that the
> earlier draft left open (§10 of that draft) is **resolved here** and recorded in the **Decision
> Log (§14)**. This document is self-sufficient: it is intended as the sole input for building the
> tool from scratch in a fresh, empty repository.
>
> **"Final product, no shortcuts" means no *architectural* compromise — not a big-bang build.** The
> full architecture, contracts, and data model below are complete and migration-free; the
> *implementation* is sequenced into corroboration-validated layers (§13), because shipping a large
> unvalidated surface is itself a correctness hazard, not a virtue.

---

## 1. Problem

Documentation and agent-instruction files — `README`s, architecture docs, ADRs,
`CLAUDE.md`/`AGENTS.md`/editor rule files, runbooks, design notes — drift out of sync with the code
they describe, silently. In an agentic codebase this is acute: a coding agent (especially a *less
capable* one) reads such a file, treats it as "here is how things are **right now**", and acts on
stale or already-superseded information.

A document goes stale in two distinct ways:

1. **Code drift** — the code the document describes changed.
2. **Supersession** — a newer document amended or replaced it.

**Threat model (the design driver):** a naive consumer reads the **raw document file** and trusts
it. Therefore staleness must be both **(a) detectable** by the tool *and* **(b) visible in the
artifact itself**. A status that lives only in a side-channel the naive agent never consults
provides zero protection.

## 2. What it is (and isn't)

A **deterministic** engine that tracks **claims** — assertions anchored to code — detects when they
**drift** (code changed) or are **superseded** (a newer document amended/replaced them), and
**stamps lifecycle status into the documents themselves** so no consumer can read a stale one as
current. Exposed as a **JSON-first CLI** over a tiny, headless library core, extensible through a
single **out-of-process resolver protocol**.

It is **not**:
- a documentation *generator* or LLM doc-rewriter (the engine never authors prose);
- a human documentation website/viewer;
- a general code-search / knowledge-graph / "find related code" engine;
- a semantic "is this claim *true*?" judge (that is an optional, quarantined tier — §7.4 — never the
  core);
- coupled to any specific consumer (a knowledge-map viewer such as *atlas* or a code-graph product
  such as *Codescope* may consume it later, but the tool stands alone).

## 3. Goals / Non-goals

**Goals**
- Detect, deterministically, when a tracked claim about code has gone **stale / moved / ghost /
  expired**.
- Detect **document supersession** (one doc amends/replaces another) and propagate it.
- Make staleness **impossible for a naive agent to miss** — stamped into the document.
- Keep the corpus **honest in time**: flag, and support updating/archiving/retracting stale docs.
- Be **deterministic** (no model in the loop), **precise** (low false-positive rate via
  corroboration), and **simple enough to fully understand and own** (no AI-slop).
- Be **extensible by third parties in any language** via one narrow, stable seam (§7) — a wire
  protocol, not a language-locked plugin API.
- Work on **any documentation format**, by treating documents as text and never depending on a
  per-format parser in the core.

**Non-goals (permanent — these are deliberate rejections, not deferrals)**
- Rewriting document prose to "fix" it (that is the *agent's* job; the engine flags + the agent edits).
- Any **embedding / vector** judgment of staleness (wrong tool — see §14).
- Any **LLM/semantic judgment inside the core** (an optional, clearly-quarantined tier may *advise*,
  but never *gates* a deterministic verdict — §7.4).
- A first-party **SCIP / semantic-symbol indexer** (the structural tier is tree-sitter; SCIP serves
  navigation, which this tool is not — §14).
- A human-facing GUI/viewer; a plugin marketplace, installer, or hosted service.

## 4. Core concepts

**Claim.** A free-text **assertion about code**, modeled as a **Proposition** (the timeless meaning)
plus one or more **Assertions** (source-owned verification instances). Example, illustrative only:

```
"Retries are capped at 5 attempts"      ← Proposition
  asserted [verified] @a3f9e21, owner=alice, anchor=⟨src/retry.ts · `MAX_ATTEMPTS` · …⟩   ← Assertion
```

**Anchor (composite, multi-selector).** *Where* a claim is pinned, stored as a **bundle of
redundant, independently-resolvable selectors** spanning the precision spectrum. No single selector
is robust to all edits, so the engine resolves the most robust available, **falls back** down the
chain, and **cross-corroborates** — confidence is a function of selector agreement. Selector kinds:

- **`text-quote`** — exact + prefix + suffix snippet (W3C TextQuoteSelector); fuzzy-matchable,
  survives moves. *The base selector; always present for a precise anchor.*
- **`text-position`** — line/char range; a cheap first guess and corroboration hint.
- **`ast-node`** — the enclosing construct via **tree-sitter**; survives relocation/reformatting.
- **`value`** — for claims about a specific value (e.g. `MAX_ATTEMPTS == 5`), an extracted structured
  value so a `5 → 50` change trips even if nothing else moves.
- **`path` / `glob`** *(coarse)* — a file / directory / glob → an **edge**: navigation and
  blast-radius only ("which decisions bear on this module?"). **Coarse anchors are never reported as
  stale** — the primary defense against over-flagging (§11).

Anchors are a **discriminated union on `kind`**; the resolver registry (§7) dispatches on it. New
kinds (e.g. a third-party `scip-symbol`) plug in without a migration.

**Status (three kinds, never conflated):**
- **Authored trust** (set by the author, lives on the Proposition/Assertion): `verified` ·
  `inferred` · `assumed`. `verified` requires evidence (an anchor + `@ref`).
- **Computed** (set by the engine, **never authored**, lives on the Assertion, **ephemeral**):
  `fresh` · `moved` · `stale` · `ghost` · `expired`.
- **Document lifecycle** (set by the engine from edges/actions, lives on the Document):
  `active → amended → superseded → archived`, plus `retracted` (author withdrew).

**Verdict.** The engine's per-Assertion result: one of the computed states, plus evidence (located
region, confidence, commit info, which selectors agreed). Verdicts mean **"suspect — re-verify"**,
never **"the claim is false"** (§11). Verdicts are **recomputed live, never stored**.

**Supersession (granular).** A **typed document edge**, **authored forward on the new document**,
with the **reverse edge derived** by the engine:
- `supersedes` (full) → targets a **Document** (the old doc → `superseded`; archive it).
- `amends` (partial) → targets **one or more Propositions** in a Document (the doc stays in the read
  path; only the named propositions flip; lifecycle → `amended`).

An old document can legitimately receive **both** signals at once — *superseded in part* **and**
*code-drift* — and both are surfaced.

**Resolver.** The single extension concept: a unit that handles one or more anchor `kind`s, locates
them in the current repo, and returns a `Verdict`. Built-in resolvers ship in-tree; third parties
add more **out-of-process, in any language**, against the same wire protocol (§7).

## 5. Data model

The **canonical model** is small and is the **source of truth as versioned JSON Schema**
(`schemas/*.v1.json`); the Rust types and every language SDK are **generated from it**.

- **Document** `{ id, path, lifecycle, edges[], frontmatterStatus? }` — a file. Owns lifecycle and
  supersession edges.
- **Proposition** `{ id, text, authoredTrust, fingerprint }` — the timeless meaning; the target of
  `amends`; the dedup unit. **Identity is authored/explicit** (`id`/content `fingerprint`), **never
  similarity-computed** (that would smuggle non-determinism back in).
- **Assertion** `{ id, propositionId, documentId, owner, ref, anchor, ttl?, attrs }` — one
  verification instance. Carries the composite **Anchor** (value-object), the `@ref` last verified
  against, and optional `ttl`.
- **Anchor** *(value-object on the Assertion)* — the multi-selector bundle of §4.
- **Verdict** *(ephemeral, never persisted)* — computed live on `check`.

**Deliberately excluded entities** (rejected on principle, not scope):
- **No `Evidence` entity** — an anchor has no identity apart from its assertion; it is a value-object
  (mirrors W3C's selector-inside-target).
- **No `Run` / verdict-history entity** — §6 mandates recompute-live; a stored run contradicts the
  persistence model.

Lineage of the shape: **Proposition/Assertion** ≈ Truth-Maintenance Systems' belief/justification
(Doyle 1979); **Document edges** ≈ ADR `superseded-by` / RFC `Obsoletes`; **Anchor selectors** ≈ W3C
Web Annotation `TextQuoteSelector` (+ tree-sitter for the structural selector).

## 6. How it works

**Persistence model — recompute-live, no committed lockfile.** Authored records (Propositions +
Assertions + Anchors) live in a **git-ignored-optional, regenerable claim store** beside the docs
(see §8); they are **self-describing**. **git is the baseline/time-machine**: the state at `@ref` is
derived on demand via an **in-process git library (gitoxide/`gix`)** — equivalent to `git show
@ref:path` with no subprocess — **recomputed live, never stored**.
Freshness is **recomputed on every check**. A **fat, git-ignored, regenerable cache/index** (parsed
trees, blob hashes, prior locations) is permitted **purely** as a performance optimization — never a
committed source of truth, never a `*.lock`.

**Drift detection (per precise Assertion), layered cheapest-first with corroboration:**
1. **Localize** — resolve each selector in the *current* tree: `text-position` (hint) → `text-quote`
   (fuzzy) → `ast-node` (tree-sitter). Output: current region + a **confidence derived from how many
   selectors agree**, or `ghost` (not locatable across all selectors / file gone).
2. **Detect change** — compare the located region against its state at `@ref` (derived live from
   git) via the appropriate tier: **text-normalized hash** (base) and **normalized-AST hash**
   (structural). For `value` selectors, compare the extracted value directly.
3. **Grade with thresholds**, not a boolean: unchanged → `fresh`; located-but-moved (selectors agree
   it relocated) → `moved` (re-anchorable); region changed → `stale`; unlocatable → `ghost`; past
   `ttl` → `expired`. Selector **disagreement** lowers confidence and yields `moved`/re-verify,
   **never** a hard `stale` — keeping the suspect set tight.

**Precision tiers (all first-party; SCIP is *not* — §14):**
- **Tier 1 — text:** fuzzy `text-quote` localization + text-normalized region hash.
- **Tier 2 — structural:** **tree-sitter** `ast-node` localization + normalized-AST hash. Lightweight
  (grammar, not a semantic indexer), deterministic, cheap to run at two commits, ~universal grammars.
- **Tier 3 — semantic (optional, quarantined):** an LLM "is it still *true*?" resolver MAY be
  registered, but it is **opt-in, runs out-of-process, and never gates a deterministic verdict** — it
  can only *advise* (e.g. annotate a `fresh` region as "semantically suspect"). The deterministic
  verdict stands on its own. (§7.4, §11.)

**Supersession.** Authoring `amends`/`supersedes` on the new document causes the engine to (a)
**derive the reverse edge**, (b) set the old Document's lifecycle, and (c) mark the affected
Proposition(s) `superseded`. Both supersession and code-drift are surfaced together when both apply.

**Lifecycle remediation — graduated by danger:**
- `amended` / superseded-in-part → **stamp banner + flip frontmatter status**; keep the file.
- `superseded` / obsolete-in-full → **archive** (move out of the read path) or remove, leaving a
  tombstone/redirect to the successor.
- `stale` / `ghost` / `expired` (claims drifted) → banner + flag the specific claims to re-verify.
- `retracted` → banner noting the author withdrew the claim.

**Division of labor (hard rule).** The **engine** owns *status, edges, lifecycle stamping, and
archival* (deterministic bookkeeping) and **flags** content. The **agent** (or human) does any
**prose rewriting**, then re-runs the engine to re-verify. The engine never writes prose.

**The write-time loop (the killer mechanic).** On a code or doc change (git hook / CI), re-run the
check and report **exactly which claims/documents that change invalidated** — so drift is closed at
authoring time, not discovered weeks later in review.

## 7. Architecture & extension seam

Modeled on the layering discipline of `earendil-works/pi`: a **tiny headless core**, **variety
pushed down into a registry**, **consumers stacked on top**, **strictly upward dependencies**.

```
  CONSUMERS (read the JSON output / call the SDK; out of this repo's scope)
    ├─ CI gate            fail build on drift (exit codes)
    ├─ MCP shim           serve verdicts to agents
    └─ a viewer (atlas)   consume claim records + verdicts as data
                ▲
  ─────────────┼──────────────────────────────────────────────
  ENGINE        the loop / CLI: walk docs → run resolvers → drift + supersession
                → stamp lifecycle → emit JSON
                EXTENSION SEAM:  out-of-process resolver protocol (JSONL-RPC)
                ▲                                  ◄── built-in resolvers register in-tree;
  ─────────────┼─────────────────────────────────     third parties add more, in ANY language,
  CORE          tiny, headless: the data model + the     against the same wire protocol + SDK.
                Resolver/Verdict contract. NO I/O, NO CLI, NO UI.
```

### 7.1 The Resolver seam is a wire protocol, not a language-locked plugin API
Extension is **out-of-process**: a resolver is a process that speaks **JSONL-RPC over stdio**,
declares the anchor `kind`s it handles, and answers `localize` / `detect` requests with `Verdict`s.
This makes the tool **extensible for everyone in any language** (an in-process plugin API would only
admit plugins in the host language — *less* "for everyone"), and isolates third-party code (a slow/crashing resolver
is timed-out and cannot corrupt a determinism-critical engine). Resolvers are declared via a
**default-deny manifest**. **Thin SDKs are generated per language** (TS and Rust first) from the protocol
schema. Code-anchor drift and document-supersession are *themselves* resolvers, shipped in-tree
behind the same contract.

### 7.2 Kinded anchors
The core `Anchor` is a discriminated union on `kind` (§4). The engine dispatches each kind to the
resolver(s) that declare it. Built-in kinds: `text-quote`, `text-position`, `ast-node`, `value`,
`path`/`glob`. Additional kinds (e.g. a community `scip-symbol`) require **no core change**.

### 7.3 Document handling is universal, not per-format
There is **no per-format document-parser seam**. Documents are treated as **text**: claims are
located by the same text/AST anchoring used for code, and status is stamped via a **universal,
sentinel-delimited, idempotent banner** (§8) that works in any text file. Markdown **frontmatter** is
an *optional* machine-readable enhancement where it exists — never a dependency.

### 7.4 The semantic tier is quarantined
Any embedding/LLM capability exists **only** as an opt-in Tier-3 resolver (§6) that **advises and
never gates**. The deterministic core has **no model in the loop** (§11.1). This boundary is
load-bearing and permanent.

### 7.5 One package first
Build as **one package** with clean internal module boundaries along these split lines. Split into
separately-published packages only when a real second consumer needs to import the core — premature
package-splitting is over-engineering.

## 8. Where claims live & how status is stamped

**Carrier — a dedicated claim store + a universal in-doc banner.** Authored records live in a
**claim store beside the docs** (e.g. `.claims/`), format-agnostic, mapping 1:1 to the §5 model. This
is **not** the forbidden freshness-lockfile (§6) — it stores authored **records**, not computed
verdicts. The store lets the engine track **any** document — including pristine, human-facing, or
third-party docs — **without rewriting its prose**.

**Stamping (satisfies the threat model regardless of carrier):**
- A **universal banner** — sentinel-delimited (`BEGIN`/`END`), **idempotent** (find-and-replace
  between sentinels → no diff churn), **plain visible text** (better for a naive raw-file reader than
  a hidden comment) — is written into each affected document, listing the suspect Propositions and
  their status.
- For markdown, an **optional** machine-readable `frontmatterStatus` is also written.

*(Inline line-microformat carriers — claims authored as lines inside the doc, à la atlas — were
considered and rejected as the universal carrier: they require injecting machine syntax into every
tracked doc body, which defaces human/third-party prose. The store + banner is universal; a project
that fully controls its docs may still layer inline authoring on top, but the engine does not require
it. See §14.)*

## 9. Interface

- A **CLI**, **JSON-first** (structured output + meaningful exit codes), quiet by default — the
  consumer is a machine. JSON shapes are the versioned schemas of §5 (`--json` is the default for
  machine paths; human-pretty output is secondary).
- **Verbs:**
  - **`check`** — verify a repo's claims; emit per-Assertion verdicts + per-Document lifecycle;
    exit-code per the contract below.
  - **`record`** — write a new code-anchored claim (Proposition + Assertion + composite Anchor) to
    the store.
  - **`query --path <p>`** — "what claims are anchored to / cover this file or region?"
    (before-edit lookup; includes coarse edges for blast-radius).
  - **`diff --since <ref>`** — "what did this change invalidate?" (the write-time loop).
  - **`supersede`** — author an `amends`/`supersedes` edge; derive the reverse; stamp status.
  - **`status [--doc <p>]`** — a **read-time** check a harness calls *before* feeding a document to a
    naive agent ("is this current?") — belt-and-suspenders to the in-file banner.
- **Exit-code contract:** `0` = all clean; `2` = suspect present (`stale`/`ghost`/`expired`); `3` =
  `moved`-only (re-anchorable warning); `1` = operational error. Strictness is tunable (`--fail-on`).
- **Extension SDK:** the out-of-process resolver protocol (§7.1), with generated per-language SDKs.

## 10. Status, lifecycle & TTL enums (final)

- **Authored trust:** `verified` · `inferred` · `assumed`. (`verified` requires an anchor + `@ref`.)
- **Computed (engine-only, ephemeral):** `fresh` · `moved` · `stale` · `ghost` · `expired`.
- **Document lifecycle:** `active` · `amended` · `superseded` · `archived` · `retracted`.
- **TTL:** an Assertion may carry an optional `ttl`; past it the computed state is `expired`
  (time-based re-verification, independent of code drift).

## 11. Principles & constraints (the discipline)

1. **Determinism is the product.** No model in the engine loop. The optional semantic tier advises;
   it never decides. The moment "is it stale?" becomes probabilistic, the value — a trustworthy,
   repeatable signal — is gone.
2. **Suspect, not false.** The engine computes "the evidence moved — re-verify," never "the claim is
   false." Confirming falsity is a human/agent act.
3. **Over-flagging is the #1 failure mode.** The valuable work is a **tight, trustworthy suspect
   set**: coarse edges are navigational (never stale); grade with thresholds; **corroborate across
   selectors and let agreement set confidence**; selector disagreement → re-verify, not hard-stale.
4. **Tiny core; "if it isn't core, it's a resolver or a consumer."** Keep the data contract small.
   Resist hook points and config that aren't earning their keep.
5. **No AI-slop.** Every part must be small enough to be fully understood and owned.
6. **Universal by construction.** Treat documents as text; never depend on a per-format parser in the
   core.

## 12. Distribution

- **A single statically-linked binary** (musl; `cargo build --release`) — tiny and zero-runtime, the
  ideal artifact for dropping into any CI or environment, with instant startup. Prebuilt binaries
  (Linux/macOS/Windows, x64/arm64) on GitHub releases; `cargo install`; `curl | sh` + a Homebrew
  formula; a thin **GitHub Action** wrapper for CI gating.
- **Per-language resolver SDKs** generated from the protocol schema (TS and Rust first), so resolver
  authors are never forced into the host language.
- JS-ecosystem consumers (e.g. atlas) read the binary's **JSON output / exit codes** like any other
  consumer — no host-language coupling.

## 13. Build sequencing (no architectural shortcuts; validate each layer)

The architecture above is complete and migration-free. Implement it in this order, validating
correctness (especially the suspect-set precision of §11.3) at each step:

1. **Core + contracts** — schemas → generated Rust types; data model; Verdict; the kinded `Anchor` union.
2. **Tier-1 drift** — text-quote fuzzy localize + text-normalized hash, against live git; the claim
   store; the universal banner; `check` + exit codes.
3. **Supersession + lifecycle** — `amends`/`supersedes`, reverse-derivation, stamping; `supersede`,
   `query`, `diff`, `status`.
4. **Resolver protocol** — JSONL-RPC + TS & Rust SDKs; move the built-in drift & supersession logic
   behind the same contract; default-deny manifest.
5. **Tier-2 structural** — tree-sitter `ast-node` selector + normalized-AST hash; corroboration &
   confidence grading across selectors; `value` selector.
6. **Tier-3 (optional)** — the quarantined semantic advisory resolver; additional language SDKs.

## 14. Decision Log (resolved; do not silently re-open)

- **D1 — Language & runtime → Rust.** For the *final* product the decisive factor is the artifact:
  a tiny, statically-linked, zero-runtime binary is the platonic CLI for dropping into anyone's CI
  or git hooks, with instant startup and the rigor a determinism-critical engine deserves. Pure-Rust
  git (`gix`) reads baselines in-process (no shell-out), and `tree-sitter` / `dissimilar` are
  first-class. The owner has working Rust and has accepted owning the engine in it. *TypeScript/Bun
  considered and rejected for the final product* (its edge was owner-velocity, not end-state quality;
  `bun build --compile` binaries are large and embed a runtime). *Go rejected* (no owner fluency).
  *Zig rejected* (immature ecosystem). The JSON/CLI contract is language-agnostic, so consumers
  (incl. the Bun/TS atlas) are unaffected.
- **D2 — Authoring → agent-authored records.** "Retrofit" means an agent *authors* claims for
  existing prose; the engine never NLP-extracts claims (that would break determinism).
- **D3 — Carrier → dedicated claim store + universal banner; frontmatter optional.** Tracks any doc
  without defacing prose. *Inline microformat rejected as the universal carrier* (defaces
  human/third-party docs); *frontmatter rejected as universal* (markdown-only).
- **D4 — Data model → Document + Proposition + Assertion + composite Anchor (value-object); verdict
  ephemeral.** *Flat rejected* (conflates the three status kinds; no clean `amends` target). *4-way
  (+Evidence +Run) rejected* (Run contradicts recompute-live; Evidence has no identity apart from its
  assertion).
- **D5 — Precision → layered + corroborating: text → tree-sitter AST → optional quarantined
  semantic.** Confidence from selector agreement.
- **D6 — CLI surface → as §9.** JSON schema-as-source-of-truth; explicit exit-code contract;
  out-of-process JSONL-RPC resolver SDK.
- **D7 — Enums → as §10.** Adds TTL→`expired` and `retracted` for the final product.
- **D8 — Fuzzy anchoring → `dissimilar` (dtolnay's diff-match-patch port) / `similar` + tree-sitter
  grammars; git via `gix` (pure-Rust, no C dep), with `git2` as fallback.** (Google's diff-match-patch
  was archived 2024-08-05; maintained Rust ports exist — a non-issue.)
- **D9 — Distribution → as §12.**
- **D10 — Name → provisional "Claim Engine."** Open for the owner; shortlist offered out-of-band.
- **SCIP — rejected as first-party.** Its differentiator over tree-sitter (cross-file semantic symbol
  graph) serves navigation/blast-radius, which §2 says this tool is not; its costs (heavy per-language
  indexer, code-only, two-commit indexing) are permanent. The kinded-anchor seam leaves the door open
  for a *third-party* `scip-symbol` resolver; we neither ship nor maintain it. The same verdict covers
  the whole **code-index family** — LSIF and **Meta's Glean** (`glean.software`): per-language indexers
  serving code *navigation*, not doc-staleness — complementary at most, never a substitute.
- **Embeddings / vector DBs — rejected as the mechanism.** Staleness is a deterministic
  change-over-versions problem (git's domain), not a similarity problem; embeddings are
  non-deterministic (model-version dependent), blind to small-but-critical changes (constants,
  operators), and noisy on neutral edits (renames) — the inverse of the tight suspect set §11.3
  demands. Embeddings own retrieval/navigation, which this tool explicitly is not.
- **Codescope coupling — rejected.** Codescope is an *example* of a similar tool, not an alignment
  target. Its canonical schema is built on the opposite persistence philosophy (accumulating
  SQLite store with stored Run/Evidence; positional-only `Location` that assumes a live SCIP index)
  and coupling would invert the dependency direction. We borrow only *patterns* (schema-as-source-of-
  truth + SDK codegen; the provider/JSONL-RPC/manifest shape).
- **Extension model — out-of-process protocol + per-language SDKs, not in-process plugins.**

## 15. Prior art (reference for the builder — study, don't copy)

- **Fiberplane Drift** — the closest existing tool and our true sibling: it binds docs/specs to code,
  anchors via **tree-sitter + git**, and detects staleness with an **AST-fingerprint** (XxHash3 of the
  normalized AST) stored in a committed **`drift.lock`**, checked in CI. Study its anchoring; the
  deliberate differences are what define us — Drift checks file content against a stored signature
  (**no git history needed**) and yields a **binary** stale/not-stale, whereas we **store nothing**
  (git is the baseline/time-machine, §6), add **fuzzy text-quote re-localization** over the AST tier,
  grade a **`fresh/moved/stale/ghost/expired`** verdict with corroboration-based confidence, and add
  **document supersession** + **in-file status stamping** — none of which Drift does.
- **Doorstop** — the engine *shape*: fingerprint-per-link, recompute → "suspect", explicit
  re-baseline.
- **Hypothesis fuzzy anchoring + W3C TextQuoteSelector + Google `diff-match-patch`** — the robust
  text re-anchoring model; the multi-selector / fallback idea (§4) generalizes it.
- **tree-sitter** — the lightweight structural tier (Tier-2): grammars, not a semantic indexer.
- **RFC `Obsoletes`/`Updated-by`; ADR `superseded-by` + `adr-tools`** — the supersession data model
  (forward-authored, reverse-derived).
- **Truth/Reason-Maintenance Systems** (Doyle, 1979) — the conceptual ancestor: track claims +
  justifications, retract when support is withdrawn (≈ Proposition/Assertion).
- **`earendil-works/pi`** — the architecture model: tiny headless core, one extension seam, strictly
  upward deps.
- **The code-index family — SCIP / LSIF / Sourcegraph / Meta's Glean** (`glean.software`; the code
  indexer, not the namesake enterprise-search product) — studied and *not* adopted: they index code
  symbols for *navigation*; we are not a code index (§2, §14). **Codescope** — a similar typed-claim
  product, studied and *not* adopted (§14).

---

*This PRD fixes the final product. The build is sequenced (§13) for validation, not scope reduction.
Decisions in §14 are resolved; re-open one only with a deliberate design pass, not silently.*
