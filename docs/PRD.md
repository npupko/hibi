# Hibi — PRD (final-product design)

> A standalone, **agent-facing** CLI (with a small reusable library core) that keeps a
> codebase's documentation and AI-agent-instruction files from silently going stale — so that
> automated agents never read a **superseded or outdated** document and act on it as if it were
> current.
>
> **Status:** complete, self-contained specification — the sole input for building the tool from
> scratch in a fresh, empty repository. The architecture, data model, contracts, algorithms, and
> parameters below are final and migration-free; every design decision is recorded, with its
> rationale, in the **Decision Log (§14)**.
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
  value so a `5 → 50` change trips even if nothing else moves. Which AST node kinds carry a literal is
  configured **per language grammar** (§6).
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

The **canonical model** is small and is defined **once in Zod (v4)** as the single source of truth;
the **versioned JSON Schema** (`schemas/*.v1.json`), the **TypeScript types**, and every language SDK
are **generated from it** (via `z.toJSONSchema`), and claim-store records are validated against it at
load. (Dependency rationale: §16.)

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
- **No `Run` / verdict-history entity** — §6 never persists verdicts (they are recomputed on every
  `check`); a stored run would contradict that.

Lineage of the shape: **Proposition/Assertion** ≈ Truth-Maintenance Systems' belief/justification
(Doyle 1979); **Document edges** ≈ ADR `superseded-by` / RFC `Obsoletes`; **Anchor selectors** ≈ W3C
Web Annotation `TextQuoteSelector` (+ tree-sitter for the structural selector).

## 6. How it works

**Persistence model — the anchor carries its own baseline; no committed verdict-lock; git is not on
the verdict path.** Authored records (Propositions + Assertions + Anchors) live in a **committed
claim store** beside the docs (see §8). **The anchor *is* the baseline snapshot:** because
re-localizing a claim requires its original selectors, every Anchor already carries the `text-quote`
(exact + prefix/suffix), the normalized-AST hash, and the extracted `value` as captured when the
claim was recorded. Freshness is therefore computed from **(the stored Anchor) vs (the current
working tree)** alone — the engine never reads a historical revision to reach a verdict. This keeps
`check` **fully offline** and correct under shallow CI clones (`git clone --depth=1`), where
historical commit objects are absent from the local repository. **git is used only for advisory
work** — `git diff --name-only <ref> HEAD` to scope the write-time loop (described below) and blame
for attribution — never to compute a verdict. **Computed verdicts are never persisted.** The
committed store holds only authored records and their baselines, written as **one file per claim** so
that merges stay scoped and meaningful, never a monolithic lockfile. A **git-ignored, regenerable
cache** (parsed trees, blob hashes) is permitted purely as a performance optimization.

**Drift detection (per precise Assertion), layered cheapest-first with corroboration:**
1. **Localize** — resolve each selector in the *current* tree: `text-position` (a hint) → `text-quote`
   (fuzzy match via the Bitap algorithm) → `ast-node` (**snapped to the smallest enclosing *named*
   tree-sitter node**, computed the same way at record time and at check time so that re-indentation
   and reformatting never shift the node boundary). Output: the current region + a **confidence
   derived from how many selectors agree**, or `ghost` when no selector can locate it.
2. **Detect change** — compare the located region against the **baseline stored in the Anchor** via
   the appropriate tier: a **text-normalized similarity** (base; a normalized edit-distance score, §17.2) and a **two-tier AST hash** — a
   *structural* hash (node kinds only; invariant under renames and whitespace) and a *semantic* hash
   (node kinds plus token **and literal** text, so a changed string or numeric literal is always
   caught). For `value` selectors, compare the extracted value using a **per-grammar extraction
   map**. (`text-quote` and `ast-node` are language-universal; `value` is the one selector that needs
   a small per-language configuration of which AST node kinds carry literals.)
3. **Grade with thresholds**, not a boolean: unchanged → `fresh`; located-but-moved (selectors agree
   it relocated) → `moved` (re-anchorable); region changed → `stale`; unlocatable → `ghost`; past
   `ttl` → `expired`. Selector **disagreement** lowers confidence and yields `moved`/re-verify,
   **never** a hard `stale` — keeping the suspect set tight.

**Precision tiers (all first-party; SCIP is *not* — §14):**
- **Tier 1 — text:** fuzzy `text-quote` localization + a text-normalized similarity score (§17.2).
- **Tier 2 — structural:** **tree-sitter** `ast-node` localization (snapped to the enclosing named
  node) + a **two-tier** normalized-AST hash (structural + semantic). Lightweight (a grammar, not a
  semantic indexer), deterministic, with grammars available for all mainstream languages
  (TypeScript, Python, Rust, Go, and more).
- **Tier 3 — semantic (optional, quarantined):** an LLM "is it still *true*?" resolver MAY be
  registered, but it is **opt-in, runs out-of-process, and never gates a deterministic verdict** — it
  can only *advise* (e.g. annotate a `fresh` region as "semantically suspect"). The deterministic
  verdict stands on its own. (§7.4, §11.)

**Deterministic boundary.** Tiers 1–2 detect *structural* change — to an anchored name, type, value,
signature, or its existence. They do **not** judge whether a natural-language *behavioral* claim is
still true (e.g. "sorts ascending", "retries on timeout", an "O(n)" complexity claim); a change that
alters behavior without touching the anchored region is, by design, graded `fresh`. Surfacing such
semantic claims for re-verification is exactly — and only — what the optional Tier-3 advisor is for.

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
- A **universal banner** — sentinel-delimited (`BEGIN`/`END`), **idempotent** (the engine locates the
  region between the sentinels and replaces it, so re-stamping an unchanged status produces no diff),
  **plain visible text** (better for a naive raw-file reader than a hidden comment) — is written into
  each affected document, listing the suspect Propositions and their status. The banner mechanism
  **must** satisfy four requirements: (1) the sentinels carry a **per-repository nonce**, so that a
  document which legitimately quotes the banner format (this specification itself, for example) is
  never mistaken for a banner and overwritten; (2) the `END` sentinel carries an **FNV-1a checksum**
  of the banner body, so that a hand-edit inside the banner is detected and the engine refuses to
  overwrite (or self-heals, per `--fail-on`); (3) markers are **line-anchored and version-tagged**;
  (4) **all whitespace inside the banner is engine-owned**. The result: re-stamping is byte-stable,
  a status update changes only the bytes between the sentinels, and clearing a banner restores the
  file to its exact pre-banner bytes — in any text format. (Exact sentinel strings, nonce derivation,
  locate regexes, comment styles, frontmatter placement, and the splice contract are in §17.5.)
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
- **Grading parameters.** Selector-fusion confidence is `C = Σ(wᵢ·sᵢ) / Σ(wᵢ)` over the selectors
  that resolved, with weights `ast-node 0.35 · text-quote 0.30 · value 0.20 · text-position 0.15`, a
  **minimum of two agreeing selectors** required (otherwise the verdict is `ghost`), a structural-only
  AST match credited as a partial `ast-node` score, and an active localization-gated **value veto**
  (both detailed in §17.3). Fuzzy
  matching (diff-match-patch) uses `Match_Threshold 0.4`, `Match_Distance 100000` (deliberately large,
  so a region relocated by hundreds of characters is still found), and a 48-character `text-quote`
  context window. Verdict bands: `C ≥ 0.8` → `fresh`; `0.5 ≤ C < 0.8` → `moved`; `0.2 ≤ C < 0.5` →
  `stale`; `C < 0.2` → `ghost`. The engine is tuned for **precision over recall**: it holds
  false-`stale` at or below ~2% and **never reports a drifted claim as `fresh`** — every missed drift
  is graded `moved` (i.e. *re-verify*), not clean. (A deeply-nested whitespace-only edit may
  occasionally over-flag to `moved`/`stale`; it never silently passes.) The full normative procedure —
  localization, hashing, fusion, grading, the per-grammar value map, and the banner format — is in **§17**.

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

- **A single self-contained executable** built with `bun build --compile` — no separate runtime to
  install, with instant startup; the ideal artifact for dropping into any CI or git hook. Prebuilt
  executables (Linux/macOS/Windows, x64/arm64) on GitHub releases; installable from npm for
  JS-ecosystem users; `curl | sh` + a Homebrew formula; a thin **GitHub Action** wrapper for CI
  gating. If an even smaller, faster-starting artifact is later required, the engine core can be
  ported to Rust behind the unchanged CLI/JSON contract (a static `musl` binary).
- **Per-language resolver SDKs** generated from the protocol schema (TypeScript and Rust first), so
  resolver authors are never forced into the host language.
- JS-ecosystem consumers (e.g. atlas) read the executable's **JSON output / exit codes** like any
  other consumer — no host-language coupling.

## 13. Build sequencing (no architectural shortcuts; validate each layer)

The architecture above is complete and migration-free. Implement it in this order, validating
correctness (especially the suspect-set precision of §11.3) at each step:

1. **Core + contracts** — the Zod model → generated JSON Schema + TypeScript types; data model; Verdict; the kinded `Anchor` union.
2. **Tier-1 drift** — text-quote fuzzy localize + text-normalized hash, compared against the baseline
   stored in the Anchor; the claim store; the universal banner; `check` + exit codes.
3. **Supersession + lifecycle** — `amends`/`supersedes`, reverse-derivation, stamping; `supersede`,
   `query`, `diff`, `status`.
4. **Resolver protocol** — JSONL-RPC + TS & Rust SDKs; move the built-in drift & supersession logic
   behind the same contract; default-deny manifest.
5. **Tier-2 structural** — tree-sitter `ast-node` selector (snapped to the enclosing named node) +
   two-tier normalized-AST hash; corroboration & confidence grading across selectors; `value` selector.
6. **Tier-3 (optional)** — the quarantined semantic advisory resolver; additional language SDKs.

## 14. Decision Log (resolved; do not silently re-open)

- **D1 — Language & runtime → TypeScript on Bun for the engine; an optional Rust port reserved solely
  for static-binary distribution.** The engine's work is anchoring, fuzzy text matching, tree-sitter
  parsing, and JSON I/O — all first-class in the TypeScript/Bun ecosystem (`web-tree-sitter` for
  parsing, plus a small vendored Bitap matcher for fuzzy text — see D8). Because the baseline lives in the Anchor record rather than in git (§6), the
  engine has no hot-path dependency on an in-process git library — which removes the principal reason
  a determinism-critical CLI would otherwise reach for a systems language. A single statically-linked
  binary remains desirable for drop-in CI use (§12); that is a *packaging* concern, satisfied by
  `bun build --compile` or, if an even smaller/faster-starting artifact is later required, by porting
  the engine core to Rust (`dissimilar` + `tree-sitter` crates) behind the unchanged CLI/JSON
  contract. *Go and Zig are out of scope.* The JSON/CLI contract is language-agnostic, so consumers
  (including the Bun/TS *atlas*) are unaffected.
- **D2 — Authoring → agent-authored records.** "Retrofit" means an agent *authors* claims for
  existing prose; the engine never NLP-extracts claims (that would break determinism).
- **D3 — Carrier → dedicated claim store + universal banner; frontmatter optional.** Tracks any doc
  without defacing prose. *Inline microformat rejected as the universal carrier* (defaces
  human/third-party docs); *frontmatter rejected as universal* (markdown-only).
- **D4 — Data model → Document + Proposition + Assertion + composite Anchor (value-object); verdict
  ephemeral.** *Flat rejected* (conflates the three status kinds; no clean `amends` target). *4-way
  (+Evidence +Run) rejected* (Run contradicts the never-persist-verdicts rule; Evidence has no identity
  apart from its assertion).
- **D5 — Precision → layered + corroborating: text → tree-sitter AST → optional quarantined
  semantic.** Confidence from selector agreement.
- **D6 — CLI surface → as §9.** JSON schema-as-source-of-truth; explicit exit-code contract;
  out-of-process JSONL-RPC resolver SDK.
- **D7 — Enums → as §10.** Adds TTL→`expired` and `retracted` for the final product.
- **D8 — Fuzzy anchoring → a *vendored* Bitap/Myers matcher (the diff-match-patch algorithm) +
  official `tree-sitter` grammars via `web-tree-sitter`.** The diff-match-patch *algorithm* is stable
  and standard, but no implementation is dependency-healthy — Google's original is **archived** and
  every JS fork is either dormant or under 100 stars — so the ~150-line matcher is **vendored into the
  engine** (consistent with §11.5: own every part; no fragile third-party dependency on the verdict
  path). `tree-sitter`/`web-tree-sitter` and the per-language grammars are taken from the **official
  tree-sitter org** as **prebuilt wasm shipped in the official `tree-sitter-LANG` packages** (the
  tree-sitter CLI is only a fallback for a language whose package lacks wasm) — never a third-party
  grammar bundle. git is accessed through the CLI for `diff --name-only` and blame only — advisory,
  never on the verdict path (§6). Fixed parameters are in §10; full dependency grounding in §16.
  (A Rust port would use the `dissimilar` and `tree-sitter` crates.)
- **D9 — Distribution → as §12.**
- **D10 — Name → "Hibi" (日々).** *hibi* = "day after day"; the product name, npm package, and CLI command are all `hibi`.
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
  deliberate differences are what define us — Drift checks file content against a single stored AST
  signature (**no git history needed**) and yields a **binary** stale/not-stale; we likewise need no
  git history to reach a verdict, but store a **richer per-claim baseline** (text-quote + AST + value,
  one file per claim, never a monolithic lock) that enables **fuzzy re-localization** and a graded
  **`fresh/moved/stale/ghost/expired`** verdict with corroboration-based confidence, plus **document
  supersession** + **in-file status stamping** — none of which Drift does.
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
- **`driftdev.sh`** — a TypeScript-compiler doc-drift checker: it extracts an API spec from source,
  runs rules over JSDoc/markdown, and validates executable `@example` blocks, emitting JSON. Worth
  studying, but **language-locked** to TypeScript — exactly the limitation the out-of-process resolver
  seam (§7) exists to avoid; its executable-`@example` validation is a natural future third-party
  resolver.
- **ReqToCode** — embeds requirement identifiers into *code* as compiler-checked constants, so a
  broken trace is a broken build. The mirror image of this tool: it enforces by instrumenting
  **code**, where we stamp **documents** — instructive as a contrast (and as the reason we reject
  in-code instrumentation: it defaces the source), not a model to copy.

## 16. Dependencies (grounded)

Every runtime dependency is either a Bun runtime built-in or a reputable, actively-maintained,
non-archived project; anything else is vendored. No capability forces an unknown, abandoned, or
single-maintainer package, and none forces a change to the vision. Star counts are approximate,
included as evidence of health.

| Capability | Decision | Package (health) | Alternatives considered | Why |
|---|---|---|---|---|
| CLI arg parsing | **built-in** | `node:util.parseArgs` (Bun native) | commander 28k★, yargs 11k★, citty, cac | subcommands + typed flags suffice; commander (0-dep) is the only sanctioned upgrade if UX outgrows it |
| Glob / path matching | **built-in** | `Bun.Glob` | picomatch | `match()` + `scanSync()` cover coarse anchors and corpus walking |
| Content hashing | **built-in** | `Bun.hash.xxHash64`, `node:crypto` | xxhash-wasm, crypto-js | fast non-crypto fingerprint, zero-dep |
| Fuzzy locate (Bitap) | **vendor** | ~150 lines, owned | see *Vendored* below | no healthy package does fuzzy *substring location* |
| Structural parsing | **dep (official)** | `web-tree-sitter` ~26k★, official tree-sitter org, 0 deps | native `tree-sitter` node binding (needs native build) | official WASM binding; portable into the single binary |
| Language grammars | **dep (official)** | `tree-sitter-{typescript,python,rust,go,java}` (official org) | third-party wasm bundles (rejected — `tree-sitter-wasms` is 16★) | official packages ship prebuilt wasm; see plan below |
| Schema → types + validation | **dep (single source)** | `zod` v4 ~43k★, 0 runtime deps | TypeBox + ajv; json-schema-to-typescript (9 deps) + ajv | one source for types + runtime validation + `z.toJSONSchema` (JSON-Schema export) |
| Frontmatter (optional) | **vendor (+ opt dep)** | `---` splitter; `js-yaml` 6.6k★ only if a YAML body must be parsed | gray-matter (just wraps js-yaml) | frontmatter is optional (§7.3); keep it dep-light |
| Resolver protocol (JSONL-RPC) | **vendor** | line-framing + dispatch | jsonrpc-lite (dormant 2022), vscode-jsonrpc (heavy, LSP-coupled) | trivial and on the isolation boundary |
| SDK codegen (deferred, Layer 6) | **dep (dev-time)** | `quicktype` ~14k★ | hand-rolled templates | build-time only, never on the verdict path |
| UUID / TTL / git / logging | **built-in + CLI** | `crypto.randomUUID`, `Date`, `git` CLI, `console` | uuid, dayjs, simple-git | git is CLI-only and advisory (§6) |

**Vendored** (small, owned per §11.5; no healthy dependency exists for these): the Bitap fuzzy-locate
matcher (the diff-match-patch algorithm — §10's parameters presume its exact semantics), the FNV-1a
banner checksum (§8), the JSONL-RPC line-framing and dispatch (§7.1), the banner stamping logic (§8),
and the optional frontmatter `---` splitter.

**Grammar acquisition** (the one non-trivial external cluster). The official `tree-sitter-LANG` npm
packages are published by the same maintainers as `web-tree-sitter`, point at the `tree-sitter/<lang>`
repos, and **each ships prebuilt, loadable `.wasm`** — so no emscripten/docker build and no
third-party bundle are needed. The plan:
1. Add the official grammar packages as **pinned, exact-version** dependencies.
2. At build time, copy each `node_modules/tree-sitter-LANG/*.wasm` into a tracked `grammars/`
   directory (the filename carrying the version) and load via `Language.load(path)`. This keeps
   `check` fully offline (§6) and survives `bun build --compile` into the single binary (§12).
3. Pin grammar versions and upgrade them in lockstep with `web-tree-sitter` (wasm ABI compatibility);
   the per-grammar `value`-extraction map (§4/§6) is keyed to the pinned version and lives in-tree.
4. A language whose package ever lacks wasm falls back to building it with the tree-sitter CLI —
   documented, but not needed for the initial TypeScript/Python/Rust/Go/Java set.

**Net runtime footprint:** `web-tree-sitter` + the official grammar packages + `zod` — all org-backed
with zero or near-zero transitive dependencies — plus the five small vendored pieces. Nothing unknown,
legacy, archived, or abandoned.

## 17. Algorithm reference (normative)

§6 is the conceptual design; this section pins the exact procedures and constants an implementation
must reproduce. Where any value here would differ from a prototype, this section is authoritative.

### 17.1 Localization
- **Bitap cascade** (re-locate a `text-quote` in the current text): `Match_Threshold 0.4`,
  `Match_Distance 100000`; the search bias is the stored `text-position` start clamped to
  `[0, len−1]`. The Bitap word size caps a pattern at **32 characters**, so: (1) if `exact` ≤ 32
  chars, match it directly and take `[at, at+len(exact))`; (2) if longer, match the **first 32 chars**
  at the bias to fix the start, set the end to `at+len(exact)`, then refine the end by matching up to
  **32 chars of the suffix**; (3) fallback — match the **last 32 chars of the prefix** and begin the
  region just after it.
- **Snap to enclosing named node** (`ast-node` selector; applied identically at record and check
  time): take the located span, **trim leading and trailing whitespace off the span** (if it
  collapses, keep one character), then select the **smallest enclosing *named* tree-sitter node** that
  fully contains the trimmed span (the deepest named node in pre-order descent). The whitespace trim is
  what makes the chosen node — and therefore its hash — invariant to re-indentation.

### 17.2 Region comparison
- **Text tier is a normalized similarity, not hash-equality.** Normalize both the located text and the
  baseline `exact`: per line strip leading whitespace, then collapse interior whitespace runs to a
  single space. Similarity `= max(0, 1 − editDistance / maxLen)` (Levenshtein), returning `1` on
  post-normalization equality. A pure reindent/reflow scores `1.0`.
- **Structural tier is a two-tier AST fingerprint** (there is no single-hash mode). Serialize the
  snapped node by pre-order DFS over **all** children (including anonymous token nodes), in source
  order, with **no** child sorting and no trivia dropping:
  - *structural* stream — the `type` (kind) of every node; invariant under renames, literals, and whitespace.
  - *semantic* stream — for a leaf, `type + ":" + text`; for an internal node, `type`; and for any
    **content-literal kind** additionally `"=" + whitespace-collapsed text` (some grammars hide a
    literal's body from its leaves, so a `"a"→"b"` string change would otherwise collide).
  - **Content-literal kinds (verbatim):** `string, string_literal, interpreted_string_literal,
    raw_string_literal, char_literal, rune_literal, number, integer, float, integer_literal,
    float_literal, int_literal, imaginary_literal`.
  - Fingerprints use **xxHash64** (§16); collision resistance is the only requirement.
- **Value tier:** compare extracted values (17.4) by **whitespace-collapsed string equality**.

### 17.3 Confidence fusion & grading
- `C = Σ(wᵢ·sᵢ) / Σ(wᵢ)` taken over the selectors that **resolved (found) only** — *not* all four.
  (Normalizing over all weights would stop a rename, where `ast-node`+`text-quote` agree, from clearing
  the `stale` band.) Weights: `ast-node 0.35 · text-quote 0.30 · value 0.20 · text-position 0.15`.
  Fewer than **two** found selectors → `ghost` (confidence forced to 0).
- **"Found" per selector:** `text-quote` — Bitap located a region. `text-position` — the content at
  the baseline offset has text-similarity **≥ 0.6**. `ast-node`/`value` — a positive match is always
  found; a **total mismatch (score 0) counts as found only if `text-position` is found**
  (*position-corroboration*). This last rule is the ghost-detection mechanism: a deleted region's
  spurious Bitap neighbour fails the 0.6 position cross-check, so the mismatch cannot manufacture
  two-selector agreement, and the verdict falls to `ghost` rather than `stale`.
- **Structural-only match:** when the structural hash matches but the semantic hash differs (a rename
  or whitespace change), the `ast-node` selector's score is `S = 0.40` (not a full `1.0`), then
  weighted by `0.35` in fusion — keeping renames out of the `stale` band without a forced full match.
- **Value veto (active):** if `value` is found with score 0 (the value changed) **and** `text-quote`
  is found with similarity **≥ 0.9** (high confidence we are at the right place), force
  `verdict = stale, confidence = 0.3`.
- **Verdict bands:** `C ≥ 0.8 → fresh` · `0.5 ≤ C < 0.8 → moved` · `0.2 ≤ C < 0.5 → stale` ·
  `C < 0.2 → ghost`. A `fresh` result is **downgraded to `moved`** when the located start differs from
  the baseline start by **more than 4 characters** (move-awareness). `expired` is determined **before**
  fusion from `ttl` versus the current time, independent of confidence.

### 17.4 Per-grammar value-extraction map
The `value` selector identifies a literal by AST node kind, per grammar. Extraction: pre-order DFS over
**named** children, take the **first** matching literal and stop; `array`/collection literals have all
whitespace stripped at extraction; booleans and null/none are treated as scalars (the literal text is
the value); if nothing matches, the `value` selector is omitted from the bundle.

| Language | scalar / number kinds | string kinds | array / collection kinds |
|---|---|---|---|
| TypeScript | `number, true, false, null, undefined` | `string` | `array` |
| Python | `integer, float, true, false, none` | `string` | `list, tuple, set, dictionary` |
| Rust | `integer_literal, float_literal, boolean_literal` | `string_literal, char_literal, raw_string_literal` | `array_expression` |
| Go | `int_literal, float_literal, imaginary_literal, true, false, nil` | `interpreted_string_literal, raw_string_literal, rune_literal` | `composite_literal` |
| Java | `decimal_integer_literal, hex_integer_literal, decimal_floating_point_literal, true, false, null_literal, character_literal` | `string_literal` | `array_initializer` |

(The Java row is derived from the grammar's node names and must be verified against the pinned
`tree-sitter-java` version before first use.)

### 17.5 Banner format
- **Sentinels** are line-anchored, version-tagged, and **nonce-bearing**: BEGIN = `HIBI:BEGIN
  v1 <nonce>`, END = `HIBI:END v1 <nonce> sha=<8-hex>`, each optionally prefixed by the file's
  comment opener. The **`<nonce>` is a short random identifier generated once per repository at
  claim-store initialization and stored in the store config**; embedding it in every sentinel is what
  guarantees a document that merely quotes the banner format (this specification included) is never
  matched and overwritten. Line-anchoring and the version tag alone do not close that hole.
- **Locate** by whole-line regex (comment prefix optional), e.g. BEGIN
  `^[ \t]*(?:#|//)?[ \t]*HIBI:BEGIN[ \t]+v\d+[ \t]+<nonce>[ \t]*$` and the END equivalent
  ending `[ \t]+sha=[0-9a-f]{8}[ \t]*$`. Use the first valid BEGIN and the first valid END after it.
- **Checksum** `sha` = **FNV-1a (32-bit; offset basis `0x811c9dc5`, prime `0x01000193`), 8 hex chars**
  of the banner body, recorded on the END line (outside the body it covers). One canonical FNV-1a
  variant is used wherever a non-crypto checksum is needed. On re-stamp the engine recomputes it; on
  mismatch the banner was hand-edited → refuse to overwrite under `--fail-on` tamper, else the fresh
  stamp wins (the engine owns the region).
- **Comment styles:** markdown → HTML block `<!-- … -->`; `#`-per-line for python/shell/yaml/toml;
  `//`-per-line for ts/js/rust/c/go/java; none for plain `.txt`.
- **Placement:** for the HTML style only, if the file opens with a `---` YAML frontmatter fence, insert
  the banner **after** the closing fence; otherwise at the top of the file.
- **Idempotent splice (the engine owns all spacing):** the banner block carries no leading/trailing
  blank line; the splicer normalizes the head to end in exactly one `\n`, then the banner, then `\n\n`
  (or a single `\n` at EOF), then the remainder with leading newlines trimmed. Removal reverses this
  exactly. Re-stamping identical content is byte-for-byte stable.
- **Body:** suspect Propositions **sorted by `id`** (a stable total order, never by similarity), one
  per line as `[STATUS] (id) text`, under a default headline
  `STALE DOCUMENT — N suspect claim(s) — re-verify before trusting.` No timestamps or run-ids
  (determinism). Banner statuses may also include the lifecycle states `superseded`, `amended`,
  `retracted` alongside the computed ones.
- **Actions:** `insert` (no banner present) · `replace` (present, content changed) · `remove` (empty
  payload → restore pristine bytes) · `noop` (present and identical).

---

*This document is the complete and self-contained specification for the tool: the architecture, data
model, contracts, algorithms, and parameters are final. The build is sequenced (§13) for validation,
not scope reduction.*
