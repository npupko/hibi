# Claim Engine — PRD (working title; final name TBD)

> A standalone, **agent-facing** CLI (with a small reusable library core) that keeps a
> codebase's documentation and AI-agent-instruction files from silently going stale — so that
> automated agents never read a **superseded or outdated** document and act on it as if it were
> current.
>
> **Status:** v1 design. This document is self-sufficient — it is intended as the sole input for
> building the tool from scratch in a fresh, empty repository. **§1–§9 are decided.** **§10 lists
> decisions deliberately left open** for a follow-up design pass; do **not** silently resolve them
> while building — implement around them or stop and ask.

---

## 1. Problem

Documentation and agent-instruction files — `README`s, architecture docs, ADRs,
`CLAUDE.md`/`AGENTS.md`/editor rule files — drift out of sync with the code they describe,
silently. In an agentic codebase this is acute: a coding agent (especially a *less capable* one)
reads such a file, treats it as "here is how things are **right now**", and acts on stale or
already-superseded information.

A document goes stale in two distinct ways:

1. **Code drift** — the code the document describes changed.
2. **Supersession** — a newer document amended or replaced it.

**Threat model (the design driver):** a naive consumer reads the **raw document file** and trusts
it. Therefore staleness must be both **(a) detectable** by the tool *and* **(b) visible in the
artifact itself**. A status that lives only in a side-channel the naive agent never consults
provides zero protection.

## 2. What it is (and isn't)

A deterministic engine that tracks **claims** — assertions anchored to code — detects when they
**drift** (code changed) or are **superseded** (a newer document amended/replaced them), and
**stamps lifecycle status into the documents themselves** so no consumer can read a stale one as
current. Exposed as a **JSON-first CLI** over a tiny, headless library core.

It is **not**:
- a documentation *generator* or LLM doc-rewriter (the engine never authors prose);
- a human documentation website/viewer;
- a general code-search / knowledge-graph / "find related code" engine;
- coupled to any specific consumer (a knowledge-map viewer, etc. may consume it later, but the
  tool stands alone).

## 3. Goals / Non-goals

**Goals**
- Detect, deterministically, when a tracked claim about code has gone **stale / moved / ghost**.
- Detect **document supersession** (one doc amends/replaces another) and propagate it.
- Make staleness **impossible for a naive agent to miss** — stamped into the document.
- Let the corpus be **kept honest in time**: flag, and support updating/archiving/removing, stale docs.
- Be **deterministic** (no model in the loop), **precise** (low false-positive rate), and
  **simple enough to fully understand and own** (no AI-slop).
- Be **extensible** by third parties via one narrow, stable seam (see §7).

**Non-goals (v1)**
- Rewriting document prose to "fix" it (that is the *agent's* job; the engine flags + the agent edits).
- Any LLM/embedding/semantic judgment inside the engine.
- A human-facing GUI/viewer.
- A plugin marketplace, installer, or SDK.
- Tracking anything that isn't code-anchored (URLs, arbitrary external sources) — see §10.

## 4. Core concepts

**Claim.** A free-text **assertion about code**, carrying an **anchor**, an **authored status**, and
the **commit it was last verified against** (`@ref`). Example, illustrative only:

```
- [verified] Retries are capped at 5 attempts · src/retry.ts#L88 `MAX_ATTEMPTS` · @a3f9e21
```

**Anchor.** *Where* a claim is pinned. Two granularities, with **different semantics**:
- **Precise anchor** (file + line(s) + a **quoted snippet**) → the unit that is **drift-checked**.
- **Coarse anchor** (a file / glob / directory) → an **edge**: navigation and blast-radius only
  ("which decisions bear on this module?"). **Coarse edges are never reported as stale** — this
  distinction is the primary defense against over-flagging (see §8).

**Status (two kinds, never conflated):**
- **Authored trust** (set by the author): `verified` · `inferred` · `assumed`. `verified` requires
  evidence (an anchor/`@ref`).
- **Computed** (set by the engine, **never authored**): `fresh` · `moved` · `stale` · `ghost`.

**Verdict.** The engine's per-claim result: one of `fresh / moved / stale / ghost`, plus evidence
(located region, commit info). Verdicts mean **"suspect — re-verify"**, never **"the claim is
false"** (see §8).

**Supersession.** A **typed document→document edge**, **authored forward on the new document**
(`amends` = partial / `supersedes` = full replacement), with the **reverse edge derived** by the
engine onto the older document. Documents are **mutable** (they may be amended, archived, or
removed).

**Document lifecycle.** `active → amended → superseded → archived` (plus the per-claim computed
states above). Lifecycle status is **stamped into the document** (frontmatter + a human/agent-
visible banner) so a naive reader sees it; the stamp must be **idempotent/deterministic** so it
does not churn diffs.

**Resolver.** The single extension seam (see §7): a small interface that knows how to take an
anchor of a given `kind`, locate it in the current repo, and return a `Verdict`. Built-in resolvers
ship in-tree; third parties add more against the same interface.

## 5. How it works

**Drift detection (per precise claim), layered cheapest-first:**
1. **Localize** the anchor in the *current* tree (line/position → fuzzy snippet match). Output:
   current region + confidence, or `ghost` (not locatable / file gone).
2. **Detect change** at the located region vs. its state at `@ref`. The baseline is **derived from
   git on demand** (`git show @ref:path`) — recomputed live, **never stored**.
3. **Grade with a threshold**, not a boolean: unchanged → `fresh`; located-but-moved → `moved`
   (re-anchorable); region changed → `stale`; unlocatable → `ghost`.

**Persistence model — no committed lockfile.** The claim record (assertion + anchor + status +
`@ref`) is **self-describing**; git is the baseline/time-machine; freshness is **recomputed on
every check**. A regenerable, **git-ignored** cache is permitted purely as a performance
optimization — never a committed source of truth, never a `*.lock`.

**Supersession.** Authoring `amends`/`supersedes` on the new document causes the engine to (a)
**derive the reverse edge** onto the older document and (b) mark the affected old claim(s)
`superseded`. An old document can legitimately receive **both** signals at once — *superseded in
part* **and** *code-drift* — and both are surfaced.

**Lifecycle remediation — graduated by danger:**
- `amended` / superseded-in-part → **stamp a banner + flip frontmatter status**; keep the file.
- `superseded` / obsolete-in-full → **archive** (move out of the read path) or remove, leaving a
  tombstone/redirect to the successor.
- `stale` (claims drifted) → banner + flag the specific claims to re-verify.

**Division of labor (hard rule).** The **engine** owns *status, edges, lifecycle stamping, and
archival* (deterministic bookkeeping) and **flags** content. The **agent** (or human) does any
**prose rewriting**, then re-runs the engine to re-verify. The engine never writes prose.

**The write-time loop (the killer mechanic).** On a code or doc change (git hook / CI), re-run the
check and report **exactly which claims/documents that change invalidated** — so drift is closed at
authoring time, not discovered weeks later in review.

## 6. Interface

- A **CLI**, **JSON-first** (structured output + meaningful exit codes), quiet by default — the
  consumer is a machine, not a human terminal. (Human-pretty output is not a v1 concern.)
- The capabilities v1 must provide (described functionally; **exact verb/flag names and JSON schema
  are open — §10**):
  - **check** — verify a repo's claims; emit per-claim verdicts + per-document lifecycle status; exit non-zero on stale/ghost.
  - **record a claim** — write a new code-anchored claim.
  - **query by path** — "what claims are anchored to / cover this file or region?" (before-edit lookup).
  - **report invalidations for a change** — "what did this diff (`--since <ref>`) invalidate?" (the write-time loop).
  - **declare supersession** — author an `amends`/`supersedes` edge; derive the reverse; stamp status.
- A **read-time status check** a harness can call to ask "is this document current?" *before*
  feeding it to a naive agent — belt-and-suspenders to the in-file banner.

## 7. Architecture

Modeled on the layering discipline of `earendil-works/pi`: a **tiny headless core**, **variety
pushed down into a registry**, **consumers stacked on top**, **strictly upward dependencies**.

```
  CONSUMERS (read the JSON output; future — out of v1 scope)
    ├─ CI gate            fail build on drift
    ├─ MCP shim           serve verdicts to agents
    └─ (a viewer, etc.)   consume claim records + verdicts as data
                ▲
  ─────────────┼──────────────────────────────────────────────
  ENGINE        the loop / CLI: walk docs → run resolvers → drift + supersession
                → stamp lifecycle → emit JSON
                EXTENSION SEAM:  registerResolver(kind, resolver)
                ▲                                  ◄── built-in resolvers register here;
  ─────────────┼─────────────────────────────────     third parties add more (same interface)
  CORE          tiny, headless: the data model + the Resolver/Verdict contract.
                NO I/O, NO CLI, NO UI.
```

- **The Resolver registry is the one extension seam** (the analogue of pi's provider registry).
  Code-anchor drift and document-supersession are *themselves* resolvers; future resolvers (AST,
  URL, …) plug into the same table. This is what makes the tool extensible for everyone.
- **Consumers depend down on the core's types and read the engine's JSON output.** Nothing reaches
  upward.
- **Build it as ONE package first**, with clean internal module boundaries drawn along these future
  split lines. **Split into separately-published packages only when a real second consumer needs to
  import the core** — premature package-splitting is over-engineering. (`pi` is multiple packages
  *now*, after thousands of commits; it did not start that way.)

## 8. Principles & constraints (the discipline)

1. **Determinism is the product.** No model in the engine loop. The moment "is it stale?" becomes a
   probabilistic judgment, the value (a trustworthy, repeatable signal) is gone.
2. **Suspect, not false.** The engine can only ever compute "the evidence moved — re-verify," never
   "the claim is now false." Confirming falsity is a human/agent act, outside the engine.
3. **Over-flagging is the #1 failure mode.** The hard, valuable work is a **tight, trustworthy
   suspect set** — coarse edges are navigational (never stale); grade with thresholds; minimise
   false positives. A noisy engine wastes agent turns and gets switched off.
4. **Tiny core; "if it isn't core, it's a resolver or a consumer."** Keep the core data contract
   small (a handful of types). Resist hook points and config that aren't earning their keep.
5. **No AI-slop.** Every part must be small enough to be fully understood and owned. Using AI to
   write it is fine; shipping generated code nobody understands is not.
6. **Ship a thin vertical slice first** (see §9). The most likely failure is over-design with
   nothing runnable — explicitly guarded against.

## 9. v1 scope — the vertical slice

Build the **smallest end-to-end thing that delivers both staleness triggers**, before any breadth.

**In v1:**
- The core data model (claim + anchor + status + `@ref`; Verdict).
- The drift pipeline of §5 (localize → change-detect → git baseline → threshold) for **one
  precision tier** (see §10‑D5).
- Document supersession: `amends`/`supersedes` edges, reverse-derivation, status stamping.
- Lifecycle: in-file status stamp (frontmatter + banner), idempotent; archive as an explicit action.
- The CLI capabilities of §6 (functional set), JSON output + exit codes.
- The write-time "what did this change invalidate?" report.
- One built-in resolver for code-anchor drift; one for document supersession. **Both shipped
  in-tree behind the Resolver interface** (the seam exists; the marketplace does not).

**Explicitly deferred (NOT v1):** MCP shim; LLM re-anchoring or "is it still true?" judgment;
AST/tree-sitter resolver; multi-source assertions / confidence scoring; richer provenance; any
human viewer; plugin installers / SDK; non-code anchor kinds.

## 10. Open decisions — resolve in a follow-up grilling pass (do NOT silently decide)

Each carries a recommendation, but **none is settled.**

- **D1 — Language & runtime.** *(Flagged by you.)* Candidates: TypeScript/Bun, Rust, Go, Zig.
  **Recommendation: TypeScript on Bun** — (1) maximum reuse of an existing TS drift implementation;
  (2) ecosystem fit with the likely consumers and with the `pi`-style plugin model (runtime-loaded
  factory modules, *no build step* — trivial in TS, painful in Rust/Go/Zig); (3) `bun build
  --compile` gives a single self-contained binary with fast startup for git-hook/CI use; (4) the
  native-speed argument does not bite at v1 (hundreds of claims, not millions; git shell-out
  dominates). Choose a native language (**lean Rust**) *only* if a hard requirement appears: a
  zero-runtime static binary for environments without Bun, or proven perf limits on very large
  monorepos. **Zig is not recommended** (immature markdown/diff/git libraries; worst for
  "extensible-for-everyone"; highest slop risk). **Note:** this PRD's plugin ergonomics and any
  library suggestions assume TS/Bun and must be revisited if a native language is chosen.
- **D2 — Authoring mode.** Retrofit claims onto *existing* prose docs · agents author claims as
  *records* · both. *(Lean: agent-authored records; retrofitting flowing prose is an
  effort/precision trap. But the ADR/README use-case implies some retrofit — genuinely open.)*
- **D3 — Where claims live & granularity.** Frontmatter (file/section) · inline markers
  (footnote/comment) · a dedicated claim store. *(Lean: frontmatter for file/section + a claim
  store for precise claims; **no quote-sidecar**, to avoid a second drift problem.)* Includes the
  **storage format** (markdown frontmatter / JSON / TOML / …).
- **D4 — Data-model shape.** One flat claim · split **proposition vs. assertion-envelope**
  (status/`@ref`/owner) · the full 4-way (proposition/assertion/evidence/run). *(Lean: the
  proposition/assertion split — it cleanly supports supersession & multiple sources — but skip the
  4-way for v1.)*
- **D5 — Drift precision tier.** Snippet-fuzzy localization + **text-normalized** region hash ·
  add **normalized-AST** hashing (needs a parser per language) · add an **LLM** semantic tier.
  *(Lean: snippet-fuzzy + text-normalized hash for v1; AST/LLM are later resolvers.)*
- **D6 — Exact CLI surface.** Verb names, flags, and the precise **JSON output schema** (and exit-
  code contract). *(The functional set is fixed in §6; the surface is not.)*
- **D7 — Exact status/lifecycle enums.** Final authored-trust set, computed-state set, and
  lifecycle set (e.g. whether to add `retracted` / `expired` / TTL semantics).
- **D8 — Fuzzy-anchoring implementation.** Library/algorithm for localization (e.g. a
  diff-match-patch–style fuzzy match) — language-dependent (see D1).
- **D9 — Distribution.** Single compiled binary · package-manager install · both — depends on D1.
- **D10 — Name.** The product name.

## 11. Prior art (reference for the builder — study, don't copy)

- **Fiberplane Drift** (`github.com/fiberplane/drift`) — doc↔code drift via AST-hash + git SHA in a
  lockfile; the closest existing tool. Study its anchoring; note we deliberately reject the
  committed lockfile (§5).
- **Doorstop** (`github.com/doorstop-dev/doorstop`) — the engine *shape*: fingerprint-per-link,
  recompute → "suspect", explicit re-baseline. Open issue #564 ≈ our exact gap.
- **Hypothesis fuzzy anchoring + W3C TextQuoteSelector + Google `diff-match-patch`** — the robust
  text re-anchoring model (`exact`+`prefix`+`suffix`, fuzzy re-match, "orphaned" = stale).
- **RFC `Obsoletes`/`Updated-by`; ADR `superseded-by` + `adr-tools`** — the supersession data model
  (forward-authored, reverse-derived).
- **Truth/Reason-Maintenance Systems** (Doyle, 1979) — the conceptual ancestor: track claims +
  justifications, retract when support is withdrawn.
- **`earendil-works/pi`** — the architecture model: tiny headless core, one extension seam, strictly
  upward deps, "if it isn't core, it's an extension."

---

*This PRD intentionally fixes only what has been decided. Everything in §10 is an explicit invitation
to a follow-up design pass — build the §9 slice around those seams, and do not collapse the open
decisions without one.*
