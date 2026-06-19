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
> **Research-grounded design.** Two foundational choices are settled against the prior-art and
> empirical evidence in **§18**: (1) the *carrier* — the **current document span is the source of
> truth**, with the store holding **bidirectional anchors** (doc-side + code-side) as pointers, not
> copies, so drift is caught on both sides; (2) the *behavioral tier* — deterministic, change-gated
> **Behavioral Risk Routing** plus an optional **executable-evidence** seam, with **no model on the
> verdict path** (§11.1). The design is aligned with the **foundational documentation-practice
> research** that informs hibi (anti-staleness, small-corpus retrieval, context-engineering, lean
> formats, agent-consumable density): top-of-file banner placement, a compact banner for
> attention-budget instruction files, the agent-hook consumer story, verdict-first JSON, and a named
> cross-repo boundary (§18-D). Options weighed and **declined** are recorded in §18 and §14;
> separately-implementable resolvers/consumers are catalogued in §19. Everything lives in this one
> file; the standalone state-vocabulary decision record (`design/ADR-001-state-vocabulary.md`) is the
> one intentional exception.
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

A document drifts out of sync in three distinct ways:

1. **Code drift** — the code the document describes changed.
2. **Doc drift** — the document's own prose changed (an agent or human edited, weakened, or deleted
   the very sentence that was verified) so the artifact now asserts something never checked against
   code. Symmetric to code drift, and just as silent.
3. **Supersession** — a newer document amended or replaced it.

**Threat model (the design driver):** a naive consumer — increasingly an AI coding agent editing in a
loop — reads the **raw document file** and trusts it. Therefore staleness must be both **(a)
detectable** by the tool *and* **(b) visible in the artifact itself**. A status that lives only in a
side-channel the naive agent never consults provides zero protection. This threat model has a direct
consequence: *the current document span is the thing the agent reads, so it — not a copy in the
store — must be the source of truth the engine verifies* (§6, §8, §18).

## 2. What it is (and isn't)

A **deterministic** engine that tracks **claims** — assertions that bind a **document span** to the
**code** it describes — detects when either side **drifts** (the code changed, *or* the documented
sentence itself changed) or is **superseded** (a newer document amended/replaced them), and **stamps
lifecycle status into the documents themselves** so no consumer can read a stale one as current.
Exposed as a **JSON-first CLI** over a tiny, headless library core, extensible through a single
**out-of-process resolver protocol**.

It is **not**:
- a documentation *generator* or LLM doc-rewriter (the engine never authors prose);
- a human documentation website/viewer;
- a general code-search / knowledge-graph / "find related code" engine;
- a semantic "is this claim *true*?" judge — the engine surfaces *behavioral risk* (deterministically,
  on change) and runs *author-supplied executable checks*, but it never asks a model to adjudicate
  truth; an LLM may participate only as an opt-in, out-of-process **advisor** that never gates a
  verdict (§7.4, §11.1, §18-A);
- coupled to any specific consumer (a knowledge-map viewer such as *atlas* or a code-graph product
  such as *Codescope* may consume it later, but the tool stands alone).

## 3. Goals / Non-goals

**Goals**
- Detect, deterministically, when a tracked claim has drifted on **either side** — the **code** moved
  or changed (`code:moved / changed / orphaned`) **or** the **documented sentence** itself was edited,
  relocated, or deleted (`doc:changed / moved / orphaned`) — one shared **anchor-resolution**
  vocabulary applied per side. The current document span is the source of truth; the store holds
  anchors, not an authoritative prose copy.
- **Route behavioral risk deterministically**: when a claim describes behavior the structural tiers
  cannot prove (e.g. "retries with backoff", "sorts ascending"), flag it **only when reachable
  evidence changed** — never on wording alone — and let author-supplied **executable checks** upgrade
  risk to a real pass/fail verdict.
- Detect **document supersession** (one doc amends/replaces another) and propagate it.
- Make staleness **impossible for a naive agent to miss** — stamped into the document, at agent-edit
  time / pre-commit / PR, not only as an offline audit.
- Keep the corpus **honest in time**: flag, and support updating/archiving/retracting stale docs.
- Be **deterministic** (no model on the verdict path), **precise** (low false-positive rate via
  corroboration and change-gating), and **simple enough to fully understand and own** (no AI-slop).
- Be **extensible by third parties in any language** via one narrow, stable seam (§7) — a wire
  protocol, not a language-locked plugin API.
- Work on **any documentation format**, by treating documents as text and never depending on a
  per-format parser in the core.

**Non-goals (permanent — these are deliberate rejections, not deferrals)**
- Rewriting document prose to "fix" it (that is the *agent's* job; the engine flags + the agent edits).
- Any **embedding / vector** judgment of staleness (wrong tool — see §14).
- **NLP/LLM claim *extraction* that auto-enforces** — the engine may *suggest* candidate claims from
  prose, but a durable, enforced claim requires explicit confirmation (agent tool-call or human); the
  evidence shows unsupervised extraction is too noisy to gate on (§18-B).
- Any **LLM/semantic judgment on the verdict path** (an optional, clearly-quarantined advisor may
  *explain* or *triage*, but never *gates* a deterministic verdict — §7.4, §11.1, §18-A).
- A first-party **SCIP / semantic-symbol indexer** (the structural tier is tree-sitter; SCIP serves
  navigation, which this tool is not — §14).
- A human-facing GUI/viewer; a plugin marketplace, installer, or hosted service.

## 4. Core concepts

**Claim.** A **binding between a documented sentence and the code it describes**, modeled as a
**Proposition** (the timeless meaning) plus one or more **Assertions** (source-owned verification
instances). The Proposition's text is **read from the current document span at verification time**,
not held as an authoritative copy in the store (§5, §8, §18-B). Example, illustrative only:

```
"Retries are capped at 5 attempts"      ← Proposition (text resolved live from README.md span)
  asserted [verified] @a3f9e21, owner=alice,
    anchor=⟨doc: README.md · §"Retries…" · code: src/retry.ts · `MAX_ATTEMPTS` · …⟩   ← Assertion
```

**Anchor (bidirectional, composite, multi-selector).** *Where* a claim is pinned **on both sides** —
a **doc-side** selector bundle (the sentence in the document) and **one or more code-side** bundles
(the code it describes). Each side is a **bundle of redundant, independently-resolvable selectors**
spanning the precision spectrum; no single selector is robust to all edits, so the engine resolves
the most robust available per side, **falls back** down the chain, and **cross-corroborates** —
confidence is a function of selector agreement. Mirroring the doc bundle on the code bundle is what
closes the doc-side drift gap and satisfies the single-source-of-truth principle: the *current*
artifact span is authoritative, the stored quote is only anchoring material and an audit cache, never
the truth (§18-B). Selector kinds (each side draws the kinds that fit it):

- **`text-quote`** — exact + prefix + suffix snippet (W3C TextQuoteSelector; 32-char context per the
  hypothes.is fuzzy-anchoring model — §17.1); fuzzy-matchable, survives moves. *The base selector on
  both sides; always present for a precise anchor.*
- **`text-position`** — line/char range; a cheap first guess and corroboration hint **only** (never
  sole identity — it is brittle under insertions, §18-B).
- **`ast-node`** — the enclosing construct via **tree-sitter**: a code symbol on the code side, or a
  **document structural path** (markdown heading/block path, parsed with the same tree-sitter
  machinery) on the doc side; survives relocation/reformatting.
- **`value`** — *(code side)* for claims about a specific value (e.g. `MAX_ATTEMPTS == 5`), an
  extracted structured value so a `5 → 50` change trips even if nothing else moves. Which AST node
  kinds carry a literal is configured **per language grammar** (§6).
- **`inline-id`** *(optional, owned docs only)* — a hidden marker (e.g. `<!-- hibi:claim id=… -->`)
  that *identifies* the record near the paragraph; it stabilizes re-anchoring but **never restates the
  claim**, and is never required (§8, §18-B). If marker and prose disagree, the prose wins.
- **`path` / `glob`** *(coarse)* — a file / directory / glob → an **edge**: navigation and
  blast-radius only ("which decisions bear on this module?"). **Coarse anchors are never reported as
  stale** — the primary defense against over-flagging (§11). *(An **opt-in, non-gating**
  "uncovered-change" advisory — a coarse edge whose code changed while **no** precise claim covers it —
  is catalogued as a deferred resolver in §19; it surfaces a suggestion, never a gating `changed`.)*

Selectors are a **discriminated union on `kind`**; the resolver registry (§7) dispatches on it. New
kinds (e.g. a third-party `scip-symbol`) plug in without a migration.

**Status (four kinds, never conflated):**
- **Authored trust** (set by the author, lives on the Proposition/Assertion): `verified` ·
  `inferred` · `assumed`. `verified` requires evidence (an anchor + `@ref`).
- **Enforcement** (the record's creation-lifecycle, set by the workflow — §9): `suggested` (a
  candidate, advisory only) · `enforced` (confirmed by an agent tool-call or human; may stamp/gate) ·
  `retired` (withdrawn). Only `enforced` claims can produce a gating verdict or a strong banner;
  `suggested` claims appear as low-pressure review tasks. This split is the deterministic answer to
  "auto-extraction is too noisy to gate on" (§18-B).
- **Computed** (set by the engine, **never authored**, lives on the Assertion, **ephemeral**) — **two
  axes** resolved per `check`, plus one orthogonal flag. The two axes answer two different questions
  and so carry two deliberately different vocabularies (§18-C):
  - *Anchor resolution — "can I still find the span, and is it unchanged?"* One vocabulary applied to
    **each side** (reported `doc:…` / `code:…`): `unchanged` (found, identical) · `moved` (found,
    relocated) · `changed` (found, content differs) · `ambiguous` (matches in several places) ·
    `orphaned` (span deleted/unresolvable — *missing source*, never silent freshness). Borrowed from
    git (`unchanged`/`moved`), the W3C annotation model (`ambiguous`), and hypothes.is (`orphaned`).
  - *Behavioral belief — "do we still believe the documented behavior holds?"* (absent on
    non-behavioral claims): `unverified` (behavioral, untested, nothing changed — resting) · `at-risk`
    (reachable evidence changed, belief no longer justified — re-verify) · `supported` (a linked
    verifier passed) · `refuted` (a linked verifier failed). Borrowed from claim verification (FEVER's
    `supported`/`refuted`) and reason-maintenance (support-withdrawn → `at-risk`). Only `refuted` may
    gate (§7.4, §18-A).
  - *Flag (orthogonal, time-based):* `expired` — past the Assertion's `ttl`, independent of either axis.
- **Document lifecycle** (set by the engine from edges/actions, lives on the Document):
  `active → amended → superseded → archived`, plus `retracted` (author withdrew).

**Verdict.** The engine's per-Assertion result: the per-side anchor states (`doc:…` and `code:…`), the
behavioral state, the `expired` flag, plus evidence (located regions on both sides, confidence, commit
info, which selectors agreed, and any changed-evidence list that triggered behavioral risk). Verdicts
mean **"suspect — re-verify"**, never **"the claim is false"** (§11). Verdicts are **recomputed live,
never stored**. *"Drift"* (and the colloquial *"stale"*) is the **human roll-up** for "any claim that
needs attention" — it is **not** a machine state, so it never collides with the precise per-side
states above.

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
- **Proposition** `{ id, textCache, authoredTrust, fingerprint }` — the timeless meaning; the target
  of `amends`; the dedup unit. `textCache` is a **non-authoritative copy** of the documented sentence,
  kept only for audit, diffing, and `orphaned`-claim recovery; **the authoritative text is the current
  doc span**, re-read at `check` time via the doc-side anchor (§4, §8, §18-B). **Identity is
  authored/explicit** (`id` / content `fingerprint` of the confirmed text), **never
  similarity-computed** (that would smuggle non-determinism back in).
- **Assertion** `{ id, propositionId, documentId, owner, ref, anchor, enforcement, claimKind?,
  verifiers[], behaviorScope?, ttl?, attrs }` — one verification instance. Carries the **bidirectional
  Anchor** (value-object), the `enforcement` state (§4), the `@ref` last verified against, optional
  `ttl`, and:
  - **`verifiers[]`** *(optional)* — executable-evidence links that upgrade behavioral risk to a real
    verdict: each is `{ kind, ref, proves? }` where `kind ∈ example | snapshot | contract | property |
    formal | command` and `ref` names a test/command. If a verifier runs and fails →
    `refuted`; if none is declared, a claim is **never** marked `supported` (§7.4,
    §18-A). Verifiers are executed by an out-of-process runner resolver (§7), never in core.
  - **`behaviorScope`** *(optional)* — for behavioral claims, the deterministic blast-radius the
    change-gate watches: `{ rootSymbols[], reachableDepth, include[], exclude[] }` (callees, imports,
    config files, literals). Absent → the change-gate falls back to the anchored node + its file.
  - **`claimKind`** *(optional)* — the author's declaration that a claim is behavioral, and of what
    kind (ordering / retry / complexity / concurrency / caching / validation / …). It drives the
    Tier-3 behavioral classification explicitly (§17.6); absent, a deterministic keyword heuristic
    classifies. A label, never a verdict.
  - **`attrs`** — an open key/value bag for resolver-specific metadata the core does not interpret;
    keeps the core contract small while letting resolvers carry their own state (§11.4).
- **Anchor** *(value-object on the Assertion)* — `{ doc: SelectorBundle, code: SelectorBundle[] }`,
  the bidirectional bundle of §4. A `SelectorBundle` is the multi-selector list for one side.
- **Verdict** *(ephemeral, never persisted)* — the three computed dimensions (§4), computed live on
  `check`.

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
re-localizing a claim requires its original selectors, every Anchor already carries — **on both the
doc and code sides** — the `text-quote` (exact + prefix/suffix), the normalized-AST/structural-path
hash, and (code side) the extracted `value` as captured when the claim was confirmed. Freshness is
therefore computed from **(the stored Anchor) vs (the current working tree — both the document and
the code)** alone — the engine never reads a historical revision to reach a verdict. This keeps
`check` **fully offline** and correct under shallow CI clones (`git clone --depth=1`), where
historical commit objects are absent from the local repository. **git is used only for advisory
work** — `git diff --name-only <ref> HEAD` to scope the write-time loop (described below) and blame
for attribution — never to compute a verdict. **Computed verdicts are never persisted.** The
committed store holds only authored records and their baselines, written as **one file per claim** so
that merges stay scoped and meaningful, never a monolithic lockfile. A **git-ignored, regenerable
cache** (parsed trees, blob hashes) is permitted purely as a performance optimization.

**Drift detection (per precise Assertion), layered cheapest-first with corroboration. The doc side is
resolved *first* — a claim whose source span is gone or changed must not be verified against code as
if it still existed (§18-B):**
0. **Resolve the doc anchor & extract the current claim text** — localize the doc-side bundle in the
   *current* document (same Bitap/structural machinery as the code side). Outcomes (`doc:…`): exact →
   `unchanged`, extract the live text as the Proposition's authoritative text; same quote relocated →
   `moved` (auto-update selectors, continue); only-fuzzy or exact-text differs → `changed`
   (the sentence may now assert something unverified — re-verify before strong enforcement); several
   matches → `ambiguous`; none → `orphaned` (stop — do not verify the stale `textCache` as
   live truth).
1. **Localize the code side** — resolve each selector in the *current* tree: `text-position` (a hint)
   → `text-quote` (fuzzy match via the Bitap algorithm) → `ast-node` (**snapped to the smallest
   enclosing *named* tree-sitter node**, computed the same way at record time and at check time so
   that re-indentation and reformatting never shift the node boundary). Output: the current region + a
   **confidence derived from how many selectors agree**, or `orphaned` when no selector can locate it.
2. **Detect change** — compare the located region against the **baseline stored in the Anchor** via
   the appropriate tier: a **text-normalized similarity** (base; a normalized edit-distance score, §17.2) and a **two-tier AST hash** — a
   *structural* hash (node kinds only; invariant under renames and whitespace) and a *semantic* hash
   (node kinds plus token **and literal** text, so a changed string or numeric literal is always
   caught). For `value` selectors, compare the extracted value using a **per-grammar extraction
   map**. (`text-quote` and `ast-node` are language-universal; `value` is the one selector that needs
   a small per-language configuration of which AST node kinds carry literals.)
3. **Grade with thresholds**, not a boolean (`code:…`): identical → `unchanged`; located-but-relocated
   (selectors agree it moved) → `moved` (re-anchorable); region content differs → `changed`;
   unlocatable → `orphaned`; past `ttl` → the `expired` flag. Selector **disagreement** lowers
   confidence and yields `moved`/re-verify, **never** a hard `changed` — keeping the suspect set tight.
4. **Behavioral risk routing (change-gated, deterministic)** — for a claim classified behavioral,
   set `at-risk` **only if** evidence in its `behaviorScope` actually changed (anchored node,
   reachable callees, imports, config, literals, or a linked verifier source); otherwise it stays
   `unverified` (resting). If the claim links `verifiers[]`, dispatch them to the runner resolver: any
   failure → `refuted` (may gate); all pass → `supported`. **Wording alone never fires**: a keyword in
   the prose, with nothing changed, is not a signal (§7.4, §18-A).

**Precision tiers (all first-party; SCIP is *not* — §14). Tiers 1–2 run on *both* sides of the
anchor:**
- **Tier 1 — text:** fuzzy `text-quote` localization + a text-normalized similarity score (§17.2).
- **Tier 2 — structural:** **tree-sitter** `ast-node` localization (snapped to the enclosing named
  node; the doc side uses the markdown structural path) + a **two-tier** normalized-AST hash
  (structural + semantic). Lightweight (a grammar, not a semantic indexer), deterministic, with
  grammars available for all mainstream languages (TypeScript, Python, Rust, Go, and more).
- **Tier 3 — behavioral risk routing (optional, change-gated, deterministic):** for behavioral claims
  the structural tiers cannot prove, deterministically set `at-risk` **only when reachable
  evidence changed**, and run author-supplied **executable verifiers** (doctest/rustdoc-style
  examples, snapshots, contracts, properties, formal checks, or a command) to produce a real
  `refuted`/`supported`. No model is on this path. An LLM or formal-methods tool MAY
  additionally be registered as an **opt-in, out-of-process advisor** that only *explains* or
  *triages* — it **never gates** a verdict and never marks a claim verified (§7.4, §11, §18-A). A
  wording-only keyword advisor is declined as too noisy to be useful (§14 D5).

**Deterministic boundary.** Tiers 1–2 detect *structural* change — to an anchored name, type, value,
signature, or its existence — on both sides. They do **not** judge whether a natural-language
*behavioral* claim is still true (e.g. "sorts ascending", "retries on timeout", an "O(n)" complexity
claim); a change that alters behavior without touching the anchored region would otherwise be graded
`unchanged`. The evidence is unambiguous that **truth requires an oracle** — only execution (a test,
snapshot, contract, property, or formal check) or a non-deterministic model can adjudicate behavioral
truth, and no published model-based approach is reliable enough to gate on (METAMON F1 0.58;
LLM-judges drop to ~0.57 consistency under sampling and accept up to ~63% of wrong answers — §18-A).
hibi therefore splits the job honestly: Tier-3 **routes attention** deterministically (flag a
behavioral claim only when reachable evidence changed) and **runs author-supplied executable checks**
where they exist; it leaves *judging truth* to the agent already in the loop (which has the reasoning
the engine deliberately excludes) or to an opt-in advisor that never gates. It never silently calls an
untested behavioral claim `supported`.

**Supersession.** Authoring `amends`/`supersedes` on the new document causes the engine to (a)
**derive the reverse edge**, (b) set the old Document's lifecycle, and (c) mark the affected
Proposition(s) `superseded`. Both supersession and code-drift are surfaced together when both apply.

**Lifecycle remediation — graduated by danger:**
- `amended` / superseded-in-part → **stamp banner + flip frontmatter status**; keep the file.
- `superseded` / obsolete-in-full → **archive** (move out of the read path) or remove, leaving a
  tombstone/redirect to the successor.
- `code:changed` / `code:orphaned` / `expired` (code drifted) → banner + flag the specific claims to re-verify.
- `doc:changed` / `doc:ambiguous` (the documented sentence changed) → banner noting the prose diverged
  from the verified claim; re-confirm or retire.
- `doc:orphaned` (the documented sentence was deleted) → the tracked claim no longer exists in the
  file; prompt to relocate or retire it (do **not** keep stamping a sentence the doc no longer makes).
- `refuted` (a linked executable check failed) → strong banner / configurable gate.
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
    ├─ agent hooks        SessionStart: `hibi status --doc CLAUDE.md` gates/injects before the
    │                     agent trusts it;  Stop: `hibi diff` detects → agent drafts the prose
    │                     edit a human merges (the deterministic half of the report's hook loop)
    ├─ MCP shim           serve LIVE-recomputed verdicts to agents — never a cached index (a
    │                     cached index would re-introduce the drift hibi exists to kill — §18-D)
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
schema. Code-anchor drift, doc-anchor drift, and document-supersession are *themselves* resolvers,
shipped in-tree behind the same contract; the **executable-verifier runner** and any **LLM/formal
advisor** are out-of-process resolvers too (the runner gates via `refuted`; the advisor never
gates — §7.4).

### 7.2 Kinded selectors
A claim's `Anchor` is bidirectional (`{ doc, code[] }`, §4); within each side, `selector` is a
discriminated union on `kind`. The engine dispatches each kind to the resolver(s) that declare it.
Built-in kinds: `text-quote`, `text-position`, `ast-node` (code symbol or doc structural path),
`value`, `inline-id`, `path`/`glob`. Additional kinds (e.g. a community `scip-symbol`) require **no
core change**.

### 7.3 Document handling is universal, not per-format
There is **no per-format document-parser seam**. Documents are treated as **text**: claims are
located by the same text/AST **doc-side** anchoring used for code (the document span is the source of
truth — §8), and status is stamped via a **universal, sentinel-delimited, idempotent banner** (§8)
that works in any text file. Markdown **frontmatter** and an optional inline claim-ID marker are
*optional* machine-readable enhancements where the doc is owned — never a dependency, and never the
carrier (§18-B).

### 7.4 The behavioral tier is quarantined; the verdict path has no model
Tier-3 **Behavioral Risk Routing** is deterministic: it change-gates attention and orchestrates
author-supplied executable verifiers through the resolver protocol (§6). Any embedding/LLM/formal
capability exists **only** as an opt-in, out-of-process **advisor** that **explains or triages and
never gates** — it cannot set, clear, or override a computed verdict, and cannot mark a claim
verified. The deterministic core has **no model on the verdict path** (§11.1). This boundary is
load-bearing and permanent; the evidence in §18-A shows why crossing it would forfeit the product's
reason to exist (reproducible, auditable trust) for an unreliable signal.

### 7.5 One package first
Build as **one package** with clean internal module boundaries along these split lines. Split into
separately-published packages only when a real second consumer needs to import the core — premature
package-splitting is over-engineering.

## 8. Where claims live & how status is stamped

**Carrier — a bidirectional-anchor store (the doc is the source of truth) + a universal in-doc
banner.** Authored records live in a **claim store beside the docs** (e.g. `.claims/`),
format-agnostic, mapping 1:1 to the §5 model. This is **not** the forbidden freshness-lockfile (§6) —
it stores authored **records** (durable IDs, bidirectional anchors, verifier links, evidence,
enforcement state), **not computed verdicts and not the authoritative claim prose**. The store must
**not** treat a copied sentence as canonical: the artifact an agent actually reads is the *current*
document — so a duplicated copy can outlive
the sentence it claims to represent. The store holds **pointers**; the current document span supplies
the claim text at `check` time (§4, §6, §18-B). This still lets the engine track **any** document —
pristine, human-facing, or third-party — **without rewriting its prose** (the anchor points *into* the
file; it does not modify it).

**Carrier model (the resolved hybrid — §18-B):**
- **Default — Model C (bidirectional sidecar anchors):** the record points into both the current doc
  span and the current code; the doc is authoritative. Closes the doc-side drift gap; works on any
  text format; touches no file.
- **Optional — inline IDs (Model B) for *owned* docs:** a hidden `<!-- hibi:claim id=… -->` marker
  may stabilize re-anchoring for high-value owned docs. Never required; identifies the record, never
  restates the claim; if marker and prose disagree, the prose wins.
- **Legacy — Model A (external text copy) for migration / read-only only:** retained solely to import
  old records and to track files hibi cannot anchor into. A migrated copy is reanchored into the
  document; if no unique span is found it becomes `unanchored-legacy` and is **excluded from strong
  enforcement** rather than silently verified.

**Cross-repo.** The store and the per-repo banner nonce (§17.5) are **per-repository** artifacts;
`check` operates within one repo's working tree. A multi-repo workspace (the common agent setup) is
served by the cheapest mechanism the foundational research endorses — **symlink a shared docs +
`.claims/` location into each repo, or run hibi per-repo** — not by a bespoke cross-repo indexer
(over-engineering, §18-D). First-class cross-repo claim resolution is explicitly out of v1 scope; the
boundary is named, not silently assumed.

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
- **Attention-budget files** (agent-instruction files — `CLAUDE.md`/`AGENTS.md`/editor-rule files):
  the banner collapses to a **single-line pointer** and the full suspect detail is served only via the
  JSON/`status` side-channel (§9, §17.5). The always-loaded instruction file must stay lean — the
  research is explicit that extra bytes in such a file dilute the model's instruction-following
  (§18-D). The instruction-file path set is configurable; the default is `CLAUDE.md`, `AGENTS.md`, and
  editor rule files.

*(Inline line-microformat carriers — claims authored as machine syntax inside the doc, à la atlas —
are **declined as the universal carrier**: they deface human/third-party prose and cannot track
read-only or pristine files. A single, hidden, prose-free **inline ID** (not authored claim syntax) is
allowed as an *optional stabilizer* on owned docs. The sidecar bidirectional anchor is the universal
carrier; inline IDs only strengthen re-anchoring where a team opts in. See §14 D3.)*

## 9. Interface

- A **CLI**, **JSON-first** (structured output + meaningful exit codes), quiet by default — the
  consumer is a machine. JSON shapes are the versioned schemas of §5 (`--json` is the default for
  machine paths; human-pretty output is secondary).
- **Verbs:**
  - **`check`** — verify a repo's claims (doc side first, then code, then behavioral risk/verifiers);
    emit per-Assertion verdicts + per-Document lifecycle; exit-code per the contract below.
  - **`record`** — write a new claim. **Span-first:** it takes a **document span** (e.g.
    `--doc README.md --doc-range L42:L44`) plus a code target (`--file src/x.ts --code-quote "…"`),
    **reads the claim text from the document**, builds both anchor bundles, and refuses to create an
    `enforced` record unless both sides resolve uniquely. (`--text` as an authoritative input is
    removed — the doc is the source of truth, §8, §18-B; a free-text override survives only for
    legacy/pristine Model-A records.)
  - **`suggest [--doc <p>] [--since <ref>]`** — scan new or changed docs for **atomic, anchorable,
    verifiable** candidate claims (code identifiers, config defaults, literals, CLI/code examples,
    RFC-2119 normative sentences), skipping rationale/opinion/background, and emit them as
    `suggested` records for confirmation. Never auto-enforces (extraction is too noisy to gate —
    §18-B); the one exception is deterministic executable examples and explicit literal checks.
  - **`reanchor <claim-id>`** — re-resolve both anchors against the current doc and code, update the
    selectors, and reset state to `unchanged`; for an `orphaned` claim, require a new location or
    `retire`.
  - **`query --path <p>`** — "what claims are anchored to / cover this file or region?"
    (before-edit lookup; includes coarse edges for blast-radius). Resolves **both** directions —
    docs covering a code path, and code covered by a doc.
  - **`diff --since <ref>`** — "what did this change invalidate?" (the write-time loop), on either
    side.
  - **`supersede`** — author an `amends`/`supersedes` edge; derive the reverse; stamp status.
  - **`status [--doc <p>]`** — a **read-time** check a harness or an **agent SessionStart hook** calls
    *before* feeding a document to a naive agent ("is this current?"); it is the deterministic gate
    the foundational research wants in a hook (guaranteed execution, not advisory) — belt-and-suspenders
    to the in-file banner (§7, §18-D).

  **Output shape (agent-consumable — §18-D).** JSON objects **lead with the decision** (`status`,
  `gates`, `side`) and trail the bulky evidence (located regions, confidence, selector agreement,
  changed-evidence list), so a truncated or transcript-embedded read still surfaces the verdict first.
  Each claim's **`owner` and time-since-last-verified** are reported in the JSON/`status` side-channel
  for triage — **never** stamped into the banner, which stays terse and timestamp-free for determinism
  (§17.5). A standing **claims index / manifest** (an `llms.txt`-style map of tracked docs → claims →
  code → status) is a **deferred consumer-side projection** of `check --json`, catalogued in §19 — not
  a core verb.

  **Agent tool-call path.** `record`, `suggest`, and `reanchor` are designed to be called by an agent
  mid-edit (the lowest-friction creation mode); the engine still validates that the proposed doc and
  code spans resolve before accepting an `enforced` record (§18-B). **Executable verifiers** declared
  on a claim run through the out-of-process runner resolver (§7), never in core.
- **Exit-code contract:** `0` = all clean; `2` = suspect present — any of `changed`/`orphaned`/`ambiguous`
  (either side), `expired`, or `refuted` — on an `enforced` claim; `3` = `moved`/`at-risk`-only
  (re-anchorable or advisory warning); `1` = operational error. `suggested` claims never set a failing
  code. Strictness is tunable (`--fail-on`).
- **Extension SDK:** the out-of-process resolver protocol (§7.1), with generated per-language SDKs.

## 10. Status, lifecycle & TTL enums (final)

- **Authored trust:** `verified` · `inferred` · `assumed`. (`verified` requires an anchor + `@ref`.)
- **Enforcement (record lifecycle):** `suggested` · `enforced` · `retired` (plus `unanchored-legacy`
  for un-reanchorable migrated copies). Only `enforced` may gate or stamp a strong banner.
- **Computed — anchor resolution (engine-only, ephemeral; one vocabulary per side, `doc:…`/`code:…`):**
  `unchanged` · `moved` · `changed` · `ambiguous` · `orphaned`.
- **Computed — behavioral belief (engine-only, ephemeral; absent on non-behavioral claims):**
  `unverified` · `at-risk` · `supported` · `refuted`. Only `refuted` may gate; `at-risk` is advisory.
- **Flag — `expired`** (TTL elapsed; orthogonal to both axes).
- *Colloquial roll-up (human-facing, not a machine state):* **drift** / **stale** = "any claim that
  needs attention" — used in banner headlines, never stored.
- **Document lifecycle:** `active` · `amended` · `superseded` · `archived` · `retracted`.
- **TTL:** an Assertion may carry an optional `ttl`; past it the computed state is `expired`
  (time-based re-verification, independent of code drift).
- **Grading parameters.** Selector-fusion confidence is `C = Σ(wᵢ·sᵢ) / Σ(wᵢ)` over the selectors
  that resolved, with weights `ast-node 0.35 · text-quote 0.30 · value 0.20 · text-position 0.15`, a
  **minimum of two found selectors** required (otherwise the verdict is `orphaned`), a structural-only
  AST match credited as a partial `ast-node` score, and an active localization-gated **value veto**
  (both detailed in §17.3). Fuzzy
  matching (diff-match-patch) uses `Match_Threshold 0.4`, `Match_Distance 100000` (deliberately large,
  so a region relocated by hundreds of characters is still found), and a 32-character `text-quote`
  context window. Verdict bands: `C ≥ 0.8` → `unchanged`; `0.5 ≤ C < 0.8` → `moved`; `0.2 ≤ C < 0.5` →
  `changed`; `C < 0.2` → `orphaned`. The engine is tuned for **precision over recall**: it holds
  false-`changed` at or below ~2% and **never reports a drifted claim as `unchanged`** — every missed
  drift is graded `moved` (i.e. *re-verify*), not clean. (A deeply-nested whitespace-only edit may
  occasionally over-flag to `moved`/`changed`; it never silently passes.) The full normative procedure —
  localization, hashing, fusion, grading, the per-grammar value map, and the banner format — is in **§17**.

## 11. Principles & constraints (the discipline)

### 11.1 Determinism is the product
No model on the verdict path. Tier-3 routes attention deterministically and runs executable checks;
any LLM/formal advisor explains or triages but never decides. The moment "is it stale?" becomes
probabilistic, the value — a trustworthy, repeatable signal — is gone. *This is a settled constraint,
grounded in evidence rather than assumption:* model-based behavioral verification tops out at
F1 ≈ 0.58 and LLM judges are
non-reproducible and trivially foolable (§18-A), so gating on one would trade the product's only
durable advantage for an unreliable signal.

### 11.2 Suspect, not false
The engine computes "the evidence moved — re-verify," never "the claim is false." Confirming falsity
is a human/agent act.

### 11.3 Over-flagging is the #1 failure mode
The valuable work is a **tight, trustworthy suspect set**: coarse edges are navigational (never
stale); grade with thresholds; **corroborate across selectors and let agreement set confidence**;
selector disagreement → re-verify, not hard-stale.

### 11.4 Tiny core — "if it isn't core, it's a resolver or a consumer"
Keep the data contract small. Resist hook points and config that aren't earning their keep.

### 11.5 No AI-slop
Every part must be small enough to be fully understood and owned.

### 11.6 Universal by construction
Treat documents as text; never depend on a per-format parser in the core.

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

1. **Core + contracts** — the Zod model → generated JSON Schema + TypeScript types; data model;
   Verdict (three computed dimensions); the **bidirectional** `Anchor` (doc + code selector bundles);
   the kinded selector union; enforcement state.
2. **Tier-1 drift, both sides** — text-quote fuzzy localize + text-normalized similarity on the
   **code side and the doc side**, compared against the baselines stored in the Anchor; **doc-first
   verification order** (resolve doc span → extract live text → verify code); the claim store
   (pointers, not authoritative prose); span-first `record`; the universal banner; `check` + exit
   codes.
3. **Supersession + lifecycle** — `amends`/`supersedes`, reverse-derivation, stamping; doc-side
   states (`doc:changed`/`doc:orphaned`/…); `supersede`, `query`, `diff`, `status`, `reanchor`.
4. **Resolver protocol** — JSONL-RPC + TS & Rust SDKs; move the built-in drift & supersession logic
   behind the same contract; default-deny manifest; the **runner-resolver capability** for executable
   verifiers.
5. **Tier-2 structural** — tree-sitter `ast-node` selector (snapped to the enclosing named node) +
   two-tier normalized-AST hash; the markdown structural-path selector for the doc side; corroboration
   & confidence grading across selectors; `value` selector.
6. **Tier-3 behavioral + onboarding** — deterministic change-gated **Behavioral Risk Routing**
   (call-graph/dependency reachability over `behaviorScope`); executable-verifier orchestration
   (`example`/`snapshot`/`contract`/`property`/`formal`/`command`); `suggest` for new-doc onboarding;
   optional LLM/formal **advisor** resolvers (advisory-only); additional language SDKs.

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
- **D2 — Authoring → agent-authored, span-first, suggest-then-confirm.** An agent (or human)
  *authors* claims for existing prose; the engine never auto-*enforces* NLP-extracted claims.
  (a) `record` is **span-first** — it derives claim text from the document, never from a passed
  string, so the doc stays the source of truth; (b) `suggest` may propose candidates from prose, but
  they enter as `suggested` and require explicit confirmation before becoming `enforced`. Fully-auto
  durable creation is allowed only for deterministic executable examples and explicit literal checks.
  *Declined:* auto-enforcing unsupervised NLP-extracted claims — too noisy to gate (LSI trace-link
  recovery ≈ 77% precision / 60% recall, or 100% recall at 16% precision; §18-B).
- **D3 — Carrier → bidirectional sidecar anchors; the *document span* is the source of truth;
  universal banner; inline ID & frontmatter optional.** The store keeps a dedicated claim record
  beside the docs, holding **pointers (doc-side + code-side anchor bundles), not a copy of the claim
  prose** — a duplicated sentence in the store would be a second truth that outlives the doc
  edits/deletions it claims to represent (the doc-side drift gap). The carrier is the **hybrid: C
  default + optional inline ID for owned docs + A for migration/read-only** (weighted 450/500).
  *Options declined:* **(A)** external text-copy + code-only anchor (285) — cannot detect doc-side
  drift; **(B)** inline microformat as the universal carrier (345) — defaces prose and cannot track
  pristine/third-party/read-only docs; **(C)** pure bidirectional sidecar with no fallback (410) —
  degrades on pristine/read-only docs. Prior art: W3C Web Annotation + hypothes.is (side-channel
  selectors, ~22% real-world orphan rate → deletion must be an explicit state), Fiberplane Drift
  (side-channel AST fingerprinting, code-side only), Swimm (inline for *owned* docs). (§18-B.)
- **D4 — Data model → Document + Proposition + Assertion + *bidirectional* Anchor (value-object);
  verdict ephemeral.** The Anchor is `{ doc, code[] }`; the Proposition carries a **non-authoritative
  `textCache`** (live text comes from the doc span); the Assertion carries `enforcement`, optional
  `claimKind`, `verifiers[]`, and `behaviorScope` — all **value-objects**, not first-party entities,
  preserving "if it isn't core, it's a resolver or a consumer." *Declined:* a **flat** model
  (conflates the status kinds; no clean `amends` target); a **+Evidence/+Run** model (a stored Run
  contradicts the never-persist-verdicts rule; Evidence has no identity apart from its assertion).
- **D5 — Precision → layered + corroborating: text → tree-sitter AST → change-gated behavioral risk
  routing (+ executable evidence).** Confidence from selector agreement. Tier-3 is deterministic,
  change-gated **Behavioral Risk Routing** — flag a behavioral claim only when reachable evidence
  changed — plus optional **executable verifiers** that yield a real `refuted`/`supported`. "Truth
  requires an oracle": no model gates. *Options declined* (§18-A weighted table): a **wording-only
  regex advisor** (270) — fires on a keyword regardless of change, generating permanent noise that
  erodes trust in the deterministic verdicts (up to ~96% spurious-warning rates for noisy analyzers);
  **dropping behavioral handling entirely** (360) — leaves the gap; an **advisory LLM** (305) and a
  **gating LLM** (240) — non-deterministic (best published behavioral-verification F1 ≈ 0.58). Chosen:
  change-gated routing (395) as the default tier + execution-grounding (420) as the upgrade; LLM/formal
  only as non-gating advisory resolvers.
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
- **D11 — Visual identity → the `hibi` wordmark + 日々 seal.** Lowercase `hibi` in a geometric
  slab-mono, paired with a cinnabar seal carrying the kanji 日々 — the seal deliberately echoes the
  status banner the engine stamps into docs. Palette: cinnabar `#D6452F`, sumi ink `#1B1B1A`,
  rice-paper `#F3EDE1`, cream `#F0E8D5`. The asset kit (wordmark in light/dark/transparent, square
  mark, favicons + `.ico`, Apple/PWA icons, 1200×630 OG card) and its usage notes live in
  `assets/logo/` (see `assets/logo/README.md`). Assets are **raster** — Nano Banana Pro has no native
  alpha, so transparency comes from a chroma-key green screen keyed with ImageMagick; a hand-drawn
  **SVG redraw is the reserved next step** if infinitely-scalable output is later required.
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
  **`unchanged/moved/changed/ambiguous/orphaned`** (+ `expired`) verdict with corroboration-based confidence, plus **document
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
- **Swimm** — the closest commercial code-coupled-docs tool; notable for *moving from* an external
  `.swm` JSON store *to* inline `sw.md` Smart Tokens for owned docs. Validates code-anchored docs and
  change-gated auto-sync; the limit (cannot track pristine/third-party docs, and snippets can stay
  "live" while a behavioral claim about a callee goes false) is exactly what motivates our hybrid
  carrier (§8, D3) and the behavioral boundary (§6).
- **Hypothes.is orphaned-annotation study** (Quantifying Orphaned Annotations, 2015) — empirical
  grounding for doc-side anchoring: ~22% of annotations detached under real web churn, only ~12%
  recoverable. The lesson encoded in §17.1 and the `orphaned` state: **anchor-unresolvable is
  itself a drift signal**, and uncertainty must be surfaced, never hidden.
- **METAMON** (arXiv 2502.02794) + the LLM-as-judge literature — the empirical case *against* a
  model on the verdict path: best-published doc-behavior verification is F1 ≈ 0.58 (P 0.72 / R 0.48);
  judge consistency collapses under sampling and accepts up to ~63% of wrong answers. Study, do not
  adopt as a gate (§7.4, §11.1, §18-A). **Cascade** (arXiv 2604.19400) — doc↔code inconsistency via
  generated tests; strong on balanced data (P 0.88) but weak under the realistic imbalance where true
  drift is rare — reinforcing "prefer few high-confidence enforced claims" (§18-B).
- **Executable documentation — Python doctest, rustdoc, mdBook, Pact, Hypothesis (property tests).**
  The deterministic oracles behind Tier-3's executable-evidence types; each verifies behavior where a
  runnable check exists, and is a natural out-of-process verifier resolver (§6, §7).
- **Docs-as-code traceability — S-CORE, Doorstop, Sphinx-Needs.** Confirm the pattern of durable IDs
  + links + bidirectional matrices over copied prose, and that *links prove coverage, not truth* —
  hence anchors **and** verification, never anchors alone (§18-B).

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
- **Doc-side cascade & outcome classification** (resolve the doc-side bundle in the *current*
  document; the hypothes.is fuzzy-anchoring order — §15): (1) structural-path selector (markdown
  heading/block path) → (2) `text-position` hint → (3) context-first fuzzy (Bitap on 32-char
  prefix/suffix, then verify the intervening exact text) → (4) exact-only fuzzy. Classify the result:
  exact unique hit, identical text → `doc:unchanged` (extract the live span as the authoritative claim
  text); same quote at a new offset → `doc:moved` (rewrite selectors); only a fuzzy hit, or the exact
  text differs from the stored quote → `doc:changed`; >1 acceptable hit → `doc:ambiguous`; no hit at any
  level → `doc:orphaned`. **Doc resolution runs before code verification; a `doc:orphaned`/`doc:changed`
  result must not let the stale `textCache` be verified as live truth** (§6, §18-B).

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
  the `changed` band.) Weights: `ast-node 0.35 · text-quote 0.30 · value 0.20 · text-position 0.15`.
  Fewer than **two** found selectors → `orphaned` (confidence forced to 0).
- **"Found" per selector:** `text-quote` — Bitap located a region. `text-position` — the content at
  the baseline offset has text-similarity **≥ 0.6**. `ast-node`/`value` — a positive match is always
  found; a **total mismatch (score 0) counts as found only if `text-position` is found**
  (*position-corroboration*). This last rule is the orphan-detection mechanism: a deleted region's
  spurious Bitap neighbour fails the 0.6 position cross-check, so the mismatch cannot manufacture
  two-selector agreement, and the verdict falls to `orphaned` rather than `changed`.
- **Structural-only match:** when the structural hash matches but the semantic hash differs (a rename
  or whitespace change), the `ast-node` selector's score is `S = 0.40` (not a full `1.0`), then
  weighted by `0.35` in fusion — keeping renames out of the `changed` band without a forced full match.
- **Value veto (active):** if `value` is found with score 0 (the value changed) **and** `text-quote`
  is found with similarity **≥ 0.9** (high confidence we are at the right place), force
  `verdict = changed, confidence = 0.3`.
- **Verdict bands:** `C ≥ 0.8 → unchanged` · `0.5 ≤ C < 0.8 → moved` · `0.2 ≤ C < 0.5 → changed` ·
  `C < 0.2 → orphaned`. An `unchanged` result is **downgraded to `moved`** when the located start differs from
  the baseline start by **more than 4 characters** (move-awareness). A unique-but-multiply-matched quote
  yields `ambiguous`. The `expired` flag is determined **before** fusion from `ttl` versus the current
  time, independent of confidence.

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
  mismatch the banner was hand-edited → refuse to overwrite under `--fail-on` tamper, else the new
  stamp wins (the engine owns the region).
- **Comment styles:** markdown → HTML block `<!-- … -->`; `#`-per-line for python/shell/yaml/toml;
  `//`-per-line for ts/js/rust/c/go/java; none for plain `.txt`.
- **Placement (top-of-file is load-bearing, not cosmetic — §18-D):** the banner **MUST** occupy the
  **first lines of the file** — within the first ~30 lines an agent reliably attends to. The
  *lost-in-the-middle* attention curve means a warning buried mid-file is effectively invisible to the
  very agents hibi protects (long `CLAUDE.md`/`AGENTS.md` files are read top-down and the middle is
  skimmed). Concretely: if the file opens with a `---` YAML frontmatter fence, insert **immediately
  after** the closing fence; otherwise at the very top. The banner is **never** placed in the middle or
  at the end of a file, in any carrier.
- **Compact body for attention-budget files (§8):** for agent-instruction files, the body MAY collapse
  to a **single-line pointer** (`STALE — N claim(s); run \`hibi status --doc <p>\``), with the full
  suspect list emitted only to the JSON/`status` side-channel — so the always-loaded instruction file
  is not bloated with banner prose (the research shows added bytes in an instruction file dilute the
  model's attention — §18-D).
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

### 17.6 Behavioral risk routing (Tier-3, deterministic)
- **Classification (not a verdict).** A claim is *behavioral-candidate* if `claimKind` declares it,
  or a deterministic heuristic matches ordering/retry/complexity/concurrency/caching/validation/error
  language. Misclassification only changes whether the change-gate is *consulted*; it never produces a
  verdict by itself.
- **Change-gate (the firing rule).** Set `at-risk` **iff** the claim is behavioral-candidate
  **and** at least one of: the anchored code node's semantic hash changed; a node within the claim's
  `behaviorScope` changed (direct callees + transitive callees to `reachableDepth`, default 2; included
  imports/config/literals); or a linked verifier's source changed. A clean working tree, or a change
  outside scope, leaves the claim `unverified` (resting). Wording alone never fires.
- **Verifier dispatch.** If `verifiers[]` is present, dispatch each to its runner resolver over
  JSONL-RPC (§7): any non-pass → `refuted` (gating-eligible on an `enforced` claim); all pass →
  `supported`; runner absent/not-run → stay at `at-risk` (or `unverified` if the gate did not fire).
  The engine never executes verifiers in-process.
- **Noise controls (normative).** One banner entry per claim per change-set (dedupe, not per-run);
  `at-risk` is advisory severity, `refuted` may gate; every behavioral banner line names
  the **changed evidence path** that triggered it and suggests adding/running a verifier; a route may
  be suppressed only with `--ignore at-risk --claim <id> --until <hash>` plus a recorded reason.
- **Advisor provenance.** If an opt-in LLM/formal advisor annotates a claim, its output is recorded
  with model/prompt-hash/context-hash and is marked non-gating; it can never set a computed state.

## 18. Design rationale & evidence

This section records the design rationale for the foundational choices — the options weighed, the
tradeoffs, and the empirical evidence — so each decision is auditable rather than asserted. Each rests
on an exhaustive prior-art and empirical survey, and each *strengthens* the determinism thesis rather
than relaxing it.

### 18-A. The behavioral tier — change-gated routing + executable evidence

**Question.** Should hibi verify natural-language *behavioral* claims ("retries with backoff", "sorts
ascending", "O(n)", "thread-safe"), and if so how — given the structural tiers can be `unchanged` while a
behavioral claim has silently gone false?

**Finding (decisive).** *Truth requires an oracle.* No production documentation tool verifies
behavioral truth; the academic best case is modest and the model-based options are non-deterministic:

| Option | Trust/precision | Behavioral recall | Determinism | Edit-loop cost | Noise control | Verdict |
|---|---|---|---|---|---|---|
| Wording-only regex advisor | very low | low | high | low | very poor | **Declined** — fires on wording, never on change |
| No behavioral handling | n/a | none | high | none | perfect | **Declined** — honest but leaves the gap |
| **Change-gated attention routing** | good | medium | **high** | low | **strong** | **Default tier** |
| **Execution-grounded verifiers** | **highest where present** | medium | **high** | medium | strong | **Evidence upgrade** |
| Advisory LLM | medium | high | low | high | medium | Opt-in plugin only |
| Gating LLM | low | high | **none** | high | poor | **Declined** |
| Formal verification | highest where spec exists | low | high | very high | strong | Resolver, not core |

**Key evidence.** METAMON (arXiv 2502.02794): F1 **0.58** (P 0.72 / R 0.48) on 9,482 Defects4J pairs —
half of real inconsistencies missed; the best published result, still not gateable. LLM-as-judge:
consistency falls to ~**0.57** under sampling, criteria removal drops human-correlation 0.666→0.591,
and audits show up to **63%** acceptance of intentionally wrong answers. Alert fatigue: noisy analyzers
report up to **96%** spurious warnings, and trust loss in one tier spreads to all of a tool's output
(Google Tricorder; static-analysis suppression studies). Agents do **not** self-correct against stale
docs (six-figure failures from inverted procedures; "hallucinating an architecture that no longer
exists"). Division of labor: the engine *routes attention*, the agent (already in the loop, with the
reasoning the engine omits) *judges truth*, executable checks *certify*.

**Decision.** Ship **deterministic, change-gated Behavioral Risk Routing** as the default Tier-3
(§17.6) with **executable evidence** as the upgrade path; LLM/formal only as advisory resolvers.
*Declined:* a wording-only regex advisor, and any model on the verdict path. **What would change it:**
a replicated, cross-language behavioral verifier at F1 > 0.85, or a locally-reproducible
deterministic-decoding judge, would justify a stronger (still non-gating) advisory role.

### 18-B. The carrier — the document span is the source of truth

**Question.** Where should a claim's source of truth live, and how are claims created, so that editing
or deleting the documented sentence is detectable?

**Finding.** A copied prose string in the store would be a *second truth* that outlives the sentence.
The artifact an agent reads is the current document, so it must be authoritative and the store must
hold pointers. No surveyed system does sentence-level *bidirectional* drift, but the building blocks
are proven.

| Criterion (weight) | A: external copy | B: inline microformat | C: bidirectional anchors | **Hybrid C+** |
|---|---|---|---|---|
| Detects doc-side drift (20) | ✗ | ✓ | ✓ | ✓ |
| Detects code-side drift (15) | ✓ | ~ | ✓ | ✓ |
| Pristine/third-party docs (10) | ✓ | ✗ | ~ | ✓ |
| Low authoring friction (15) | ~ | ~ | ~ | ✓ |
| Anchor robustness (15) | ~ | ✓ | ✓ | ✓ |
| Single source of truth (—) | ✗ | ✓ | ✓ | ✓ |
| **Weighted /500** | **285** | **345** | **410** | **450** |

**Key evidence.** W3C Web Annotation + hypothes.is: side-channel selectors (TextQuote + position +
range), reattached via a 4-level fuzzy cascade; the orphaned-annotation study (20,953 highlights)
measured ~**22%** detach under real churn with only ~**12%** recoverable → *deletion must be an
explicit state, not silent freshness*. Fiberplane Drift: production side-channel AST fingerprinting
(`drift.lock`) — but code-side only, and re-link can clear a gate without fixing prose. Swimm: moved
external `.swm` → inline `sw.md` for *owned* docs (single source) but cannot track pristine docs.
Trace-link extraction (LSI): ~**77% P / 60% R** (or 100% R at **16% P**) → auto-suggest, don't
auto-enforce. Human RTM vetting can even *degrade* a good matrix → keep confirmation small and
precision-first.

**Decision.** The **hybrid: Model C default** (bidirectional sidecar anchors; document span
authoritative) **+ optional inline IDs for owned docs + Model A for migration/read-only**;
**span-first `record`**, **suggest-then-confirm** creation with `suggested`/`enforced`/`retired`
states, doc-first verification order, and explicit doc-side states incl. `doc:orphaned`. **What would
change it:** if doc-side fuzzy anchoring proves too noisy in practice (orphan rate > ~30%), require
inline IDs for high-severity claims; if pristine-doc tracking proves rare, drop the Model-A fallback
and simplify to pure C.

### 18-C. The state vocabulary — ubiquitous language for the computed model

**Question.** What is the clearest ubiquitous language for the computed model?

**Finding.** The computed model is **two axes that answer two different questions**, and each question
has an established term-of-art vocabulary in prior art. Two smells to avoid: naming the *same* outcome
differently on the doc and code sides, and letting a freshness metaphor collide with the separate
`expired` (TTL) flag.

- **Axis 1 — anchor resolution** ("can I find the span, unchanged?") is a *localization* question.
  One vocabulary, applied per side: **`unchanged · moved · changed · ambiguous · orphaned`**. Sources:
  git status (`unchanged`/`moved` — the most widely-understood "resolve a tracked thing" vocabulary),
  the W3C Web Annotation discussion (`ambiguous` for the multiple-match case), hypothes.is (`orphaned`
  is its canonical term for an annotation that can no longer anchor — applied to *both* sides).
  `changed` is chosen over `stale` so `unchanged`/`changed` form a clean antonym pair and the word
  "drift/stale" is freed as the human roll-up rather than overloading a leaf state.
- **Axis 2 — behavioral belief** ("do we still believe it?") is an *epistemic/verification* question,
  so it gets a different vocabulary: **`unverified · at-risk · supported · refuted`**. Sources: the
  FEVER claim-verification standard (`supported`/`refuted` — our unit is literally a *claim*); the
  SMT-solver / Frama-C / Nagios convergence on an explicit "not-established" value (→ `unverified`,
  also dodging the clash with authored-trust `verified`); reason-maintenance / JTMS "support withdrawn
  when a justification changes" (→ `at-risk`, the change-gated state no off-the-shelf vocabulary names).
  Non-behavioral claims carry **no** behavioral state (absence, displayed `n/a`) — no surveyed system
  gives "out of scope" a peer status.
- **Orthogonal:** `expired` is a time flag, not an anchor state; `drift`/`stale` is the colloquial
  human roll-up, never a stored machine state.

**Decision.** Two axes, two borrowed vocabularies, one applied per side; verdicts read
`doc:unchanged · code:changed · behavior:at-risk`. **What would change it:** if `at-risk`'s hyphen
proves awkward in serialization, the single-token fallback is `unsupported` (FEVER-adjacent); if teams
find `changed` too bland, `stale` is the cited runner-up (cost: it re-overloads the human roll-up).
The full options table, prior-art attribution, and the vocabulary invariants live in the standalone
**`design/ADR-001-state-vocabulary.md`** (kept as a separate record by design).

### 18-D. Documentation-practice alignment (the foundational research)

hibi serves an established body of documentation-practice research on keeping a multi-repo,
agent-driven documentation corpus honest and lean (anti-staleness architecture, small-corpus
retrieval, Claude-Code context engineering, lean doc-format specs, agent-consumable doc density). The
design embodies it — in several places hibi is the *mechanized enforcement* of a principle that
research could only state as a manual convention:

- **Single source of truth** ≡ the carrier inversion (§8): one canonical span, pointers not copies —
  which also makes hibi immune by construction to the research's "the index drifts from its source"
  failure mode (the store re-reads the live document, §6).
- **"file:line → symbol names"** ≡ the tree-sitter named-node anchor (§4, §17.1).
- **"executable beats prose"** ≡ the executable-verifier seam (§4, §17.6).
- **Curation gate / never let the agent edit instruction files unreviewed** ≡ engine-never-authors +
  suggest→enforce (§6, §4, §8).
- **Path-coupling** ≡ the bidirectional code-side anchor + coarse `path`/`glob` edges (§4): a claim is
  tied to the code it describes, so a change to that code surfaces the claim.
- **Decision-vs-design / immutable supersession** ≡ the document lifecycle + typed edges (§4, §10).
- **hibi *is* the re-verification a "freshness stamp" needs** — the research admits a `last_verified`
  date is rubber-stampable *because nothing re-verifies it*; hibi is that process, so a green hibi
  status is evidence-backed, not a hand-typed date.

**Documentation-practice refinements in the design:** top-of-file banner placement as a load-bearing
requirement (§17.5, lost-in-the-middle); a compact banner + side-channel detail for attention-budget
instruction files (§8); the agent-hook consumer story and the "MCP serves live verdicts, not a cached
index" clarification (§7); verdict-first JSON with `owner` and last-verified age in the side-channel,
never the banner (§9); and a named cross-repo boundary (§8).

**Boundaries the research validates (deliberate refusals):** no embeddings/vector/RAG (a similarity
tool for a change-over-versions problem); no doc-count / size / bloat linting *in core* (file-quality
is a separate axis from claim-drift — keep it a consumer); TTL stays an orthogonal backstop, never a
time-first `last_verified` governance field; no model on the verdict path.

Separate, opt-in capabilities are catalogued in §19.

## 19. Additional resolvers & consumers (opt-in, deferred)

These are **not** core. Each is an independently-implementable unit behind an existing seam — an
out-of-process **resolver** (§7.1) or a JSON-output **consumer** (§7) — recorded here so it can be
built later without touching the tiny core (§11.4). None of them gates a
verdict; the only gate remains a deterministic `refuted` (executable verifier) or an anchor-drift
(`changed`/`orphaned`/…) on an `enforced` claim.

**Resolvers (out-of-process, default-deny manifest — §7.1):**
- **`uncovered-change` advisory** — *trigger:* a coarse `path`/`glob` edge whose code changed while
  **no precise claim** covers it. *Output:* a non-gating suggestion ("code in `<edge>` changed;
  consider recording a claim"). *Deferred/opt-in because:* it closes the one coverage gap precise
  anchors structurally cannot see, but it risks the over-flagging §11.3 guards against — so it ships
  behind `--fail-on` opt-in, severity-3 at most, and never turns a coarse edge into a gating `changed`
  (§4).
- **`import-ref` drift resolver** — *trigger:* an `@path`-style import or pointer reference inside an
  owned instruction file whose target moved or was deleted. *Output:* `orphaned` for the broken
  reference. *Deferred because:* it edges toward the navigation territory §2 disclaims, so it stays a
  third-party resolver, never core; it closes the context-engineering "@-import drift" gap.
- **Behavioral advisor (LLM / formal)** — already defined as the quarantined Tier-3 advisor (§7.4): an
  opt-in resolver that *explains/triages* a behavioral claim, recording model/prompt/context
  provenance, and **never** sets a computed state. Listed here for completeness; its deterministic
  siblings, the **verifier runners** (`example`/`snapshot`/`contract`/`property`/`formal`/`command` —
  §4, §17.6), are the gating-eligible members of the same family (via `refuted`).

**Consumers (read `check --json`; out of repo scope — §7):**
- **Claims index / `llms.txt` emitter** — a standing manifest (tracked docs → claims → code anchors →
  status) projected from `check --json`. *A consumer, not core:* it authors no prose, runs no model,
  builds no semantic graph — a thin formatter, so per §7.5/§11.4 it stays a consumer (or a documented
  `jq` recipe), promoted to a first-class verb only if a real consumer needs it. It strengthens the
  threat model: one map an agent reads before trusting any doc.
- **Doc-age / re-review roll-up** — a consumer-side report ("document X: N expired/at-risk claims; no
  claim confirmed since `<ref>`") that drives the research's periodic-review practice without adding a
  new core state; built over existing verdicts + `ttl`.

---

*This document is the complete and self-contained specification for the tool: the architecture, data
model, contracts, algorithms, and parameters are final. The build is sequenced (§13) for validation,
not scope reduction. Design rationale — the options weighed and declined — is in §18; alignment with
the foundational documentation-practice research is in §18-D; deferred, separately-implementable
resolvers and consumers are in §19. The standalone state-vocabulary decision record is
`design/ADR-001-state-vocabulary.md`.*
