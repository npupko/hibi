# Hibi v4 specification

Status: decided 2026-10-22, not yet implemented. This document is the contract for the TypeScript implementation and for a later Rust port. It replaces ADR-001 through ADR-004 and the historical PRD, which are deleted with the implementation. Where this document and any older text disagree, this document wins.

Evidence for the judge section: `design/evidence/typesafe-research-2026-10-22.md`.

## 1. Purpose

Hibi tracks **claims**: sentences in documents that assert something checkable. It tells an agent or a person which claims are stale, which documents cannot be trusted right now, which sentences contradict each other, and which sentences can never be verified and should be rewritten or removed.

Documents can be anything: a README, `CLAUDE.md`, an ADR, a marketing playbook, a process page. Evidence can be code, another document, or something only an agent with tools can reach. Hibi does not fetch, execute, or edit anything. Agents do.

The primary user is a coding agent with the hibi skill in context. Humans and CI are secondary. JSON is the contract.

## 2. Principles

1. **Hibi runs nothing and fetches nothing.** No verifiers, no external resolvers, no network except the judge API. An agent collects evidence and submits it.
2. **Same tree, same verdicts.** Deterministic passes are deterministic. Model verdicts are cached and never re-asked for the same input, so a second run cannot flip them.
3. **A flag means re-verify.** Hibi reports that evidence moved, expired, was refuted, or conflicts. It never rewrites prose and never declares a sentence false on its own authority.
4. **Nothing clears without evidence or a human.** An agent cannot vouch a claim fresh. Evidence judged by the model, or a human attestation, can.
5. **One knob per concept, no hidden rules.** Every automatic rule appears as a reason label in the report.
6. **Contracts are data.** The store schema, the judge request and response shapes, and the judge question definitions are JSON. Tests are JSON fixtures. The implementation language is replaceable.

## 3. Vocabulary

| term | meaning |
|---|---|
| document | a text file registered in the store |
| region | a unit the splitter produces from a document: sentence, heading, list item, table row, or whole fenced block |
| claim | one region in one document, with a kind, zero or more evidence sides, and attestations |
| evidence side | where the truth of the claim can be observed: a code span, a span in another document, or a pointer plus a procedure |
| anchor | the selectors that locate a span in a file |
| attestation | a record that someone or something stood behind the claim at a time, with a basis |
| verdict | the computed state of a claim at check time; never stored except for judge results |
| judge | the model-backed interface with three judgments; Jev is the shipped implementation |

## 4. Claim kinds

| kind | evidence side | what makes it stale |
|---|---|---|
| `code` | one or more code spans | anchor resolution on either side |
| `reference` | one span in another document | anchor resolution, plus judge `supports(sentence, referencedText)` when either text changed |
| `attested` | none required; optional `sources[]` pointers; optional `procedure` | `ttl` elapsed since the newest counting attestation; or submitted evidence judged `refutes` |

The audit also names two region classes that are never recorded: `unverifiable` (a stated fact nothing could confirm) and `not-a-claim` (instruction, opinion, narrative, heading).

Kind is explicit at record time. It is not inferred from the shape of the sides.

### 4.1 Cross-repository evidence

`config.repos` maps an alias to a local path: `{ "api": "../api" }`. A side's `file` may be `api:src/limits.ts`. If the alias is not checked out at that path, the side resolves to `unavailable`, which warns and never gates. Without aliases, nothing changes.

### 4.2 Procedure

An `attested` claim may carry:

```json
"procedure": {
  "instructions": "Open the Stripe dashboard, find the Pro plan, report its monthly price.",
  "steps": [{ "id": "price", "command": "stripe prices list --product prod_123 --json" }]
}
```

`instructions` is the contract and is required when `procedure` is present. `steps` is optional, Lobster-compatible YAML-as-JSON (`id`, `command`, `stdin`), and purely a hint. Hibi never runs either.

### 4.3 Sources

Any claim may carry `sources[]`, shaped after OKF:

```json
"sources": [{ "id": "stripe", "resource": "https://dashboard.stripe.com/products/prod_123", "title": "Stripe product" }]
```

`resource` is required per entry and is an opaque pointer. Hibi never dereferences it. Evidence submitted with a `source` matching a listed `id` or `resource` is recorded as coming from a declared source.

## 5. Attestations and ttl

Each claim has an append-only `attestations[]`:

```json
{ "by": "human:alice@example.com", "at": "2026-10-22T09:12:00Z",
  "basis": "vouch | evidence | ack", "note": "checked Stripe, still $29", "evidenceHash": "…" }
```

- `by` uses OKF actor form: `human:<id>`, `agent:<name>`, `process:<name>`. The CLI's `--by` is required on `evidence` and `attest`; `record` defaults `by` to `human:<git user.email>` unless `--by` is given.
- `basis: vouch` is a bare statement. `evidence` means the judge returned `supports` for submitted evidence. `ack` acknowledges a `review` or `conflict` finding with a note.
- **Counting rule.** `ttl` is measured from the newest attestation whose basis is `evidence`, or whose basis is `vouch` with a `human:` actor. An `agent:` or `process:` vouch is recorded and does not count. An `ack` never resets ttl.
- `ttl` is an ISO 8601 duration (`P90D`). It is required on `attested` claims and optional elsewhere. There is no store-wide default.
- `owner` is the actor to contact when the claim expires. It is separate from who attested.

## 6. Store

`.claims/` at the repository root, committed. Store `version: "v4"`. No upgrade path from v3.

```
.claims/
  config.json
  documents/<doc_id>.json
  claims/<claim_id>.json
  judge/<sha256>.json        # cached judge verdicts
```

Ids are `doc_` and `clm_` followed by 16 hex characters. There are no propositions. The claim holds its sentence.

### 6.1 config.json

```json
{
  "version": "v4",
  "repos": { "api": "../api" },
  "judge": { "model": "jev-1.14.0", "threshold": 0.8 },
  "ignore": ["docs/generated/**"]
}
```

### 6.2 Document

```json
{ "id": "doc_…", "path": "docs/pricing.md", "lifecycle": "active | superseded | archived",
  "supersededBy": "doc_…", "createdAt": "…" }
```

### 6.3 Claim

```json
{
  "id": "clm_…",
  "documentId": "doc_…",
  "kind": "code | reference | attested",
  "sentence": "Pro costs $29/month.",
  "doc": { "file": "docs/pricing.md", "selectors": [ /* text-quote, text-position */ ] },
  "evidence": [ { "file": "api:src/billing.ts", "selectors": [ /* text-quote, text-position, ast-node?, value? */ ] } ],
  "sources": [],
  "procedure": null,
  "owner": "human:alice@example.com",
  "ttl": "P90D",
  "enforcement": "enforced | suggested | retired",
  "attestations": [],
  "createdAt": "…",
  "ref": "<git HEAD at record>"
}
```

`evidence` holds code spans for `code` and exactly one document span for `reference`. It is empty for `attested`.

Selectors are unchanged from v3: `text-quote` (exact, 48-char prefix and suffix), `text-position`, `ast-node` (code only, optional), `value` (code only, literal inside the span), `coarse` (navigation only).

### 6.4 Judge cache entry

```json
{ "key": "<sha256 of model + canonical request JSON>",
  "model": "jev-1.14.0",
  "judgment": "triage | supports | conflict",
  "subject": { "claimId": "clm_…", "pairId": "clm_a:clm_b", "docId": "doc_…" },
  "request": { /* exact state and questions sent */ },
  "answers": { /* exact probabilities returned */ },
  "evidence": { "hash": "…", "source": "…", "summary": "…", "by": "agent:opus", "at": "…" },
  "at": "…" }
```

Raw evidence text is never stored. The summary, source, hash, and the judge's probabilities are.

## 7. Resolution

### 7.1 Anchors

The v3 cascade is kept as is for every span side, doc or evidence:

1. Locate the quote: exact occurrences ranked by stored context; else a fuzzy match near the stored position within `min(256, floor(len * 0.4))` edits.
2. Located, similarity at or above 0.9, no AST or value change, start within 4 chars: `unchanged`.
3. Same, at a new offset: `moved`.
4. Similarity below 0.9, or AST semantic hash changed, or in-span literal changed, or (prose) a numeric token in the sentence changed: `changed`, with a reason label.
5. Not found: `orphaned`.
6. Several exact occurrences with equal context scores and a quote of at least 8 chars: `ambiguous`.

Tree-sitter is optional. Missing grammar or missing binary means text, position, and value selectors only, with no warning. Grammars ship as an optional download.

### 7.2 Claim states

Per claim, `check` produces:

| axis | values |
|---|---|
| `doc` | `unchanged / moved / changed / ambiguous / orphaned` |
| `evidence` | same five, plus `unavailable`; absent when there is no evidence side |
| `judge` | `supported / refuted / review / pending`; absent when no judgment applies |
| `expired` | boolean, from the counting rule in section 5 |
| `conflicts` | list of `{ claimId, probability, reason }` from the contradiction pass |

### 7.3 When the judge is consulted

- `reference`: when the cache has no entry for `(model, sentence, referencedText)`. Unchanged texts mean no call.
- `attested` with submitted evidence: on `hibi evidence`, once per `(model, sentence, evidenceHash)`.
- contradiction pass: once per `(model, pair texts)`.
- No API key, or a network failure: the result is `pending`. Nothing else changes. There is no offline flag.

### 7.4 Gating

| condition | effect in `check` | effect in `check --doc` |
|---|---|---|
| `changed / orphaned / ambiguous` on either side, enforced claim | gate | gate |
| `judge: refuted` at or above threshold | gate | gate |
| `conflicts` non-empty at or above threshold | gate | gate |
| `expired` | warn | gate |
| `moved`, `review`, `pending`, `unavailable` | warn | warn |
| `suggested` or `retired` claim | never | never |

Exit codes: `0` clean or warnings, `2` gating, `1` operational error. `--fail-on gating|warn|never`.

### 7.5 Worklist

`check` output includes `worklist[]`: every `attested` claim that is `expired` or has never had evidence, plus every `reference` or `attested` claim in `review` or `pending`. Each item carries `claimId`, `sentence`, `kind`, `procedure`, `sources`, `owner`, the last evidence summary and time, and the reason it is listed. `list --state pending` returns the same set.

## 8. Judge

### 8.1 Interface

```
triage(doc, regions[])        -> per region: { kind, kindProbability, evidenceKind, checkable }
supports(sentence, context, evidence, source) -> { relation: supports|refutes|does_not_address, confidence, probabilities, addresses }
conflict(a, b)                -> { relation: consistent|contradictory|unrelated, confidence, probabilities, sameSubject }
```

The interface is data in, data out. Question definitions (instructions and criteria) live in `judge/questions/*.json` and are loaded, not written in code. Jev is the only shipped implementation. A fake implementation serves tests from fixtures.

### 8.2 Jev adapter

- Direct HTTP: `POST https://api.typesafe.ai/v1/systemone`, bearer `TYPESAFE_API_KEY`, body `{ model, state, questions }`. The official JS SDK is not used; it fails under Bun as of 0.7.0.
- Retries on 408, 429, 5xx, 529 with exponential backoff honoring `retry-after`; at most 8 requests in flight.
- `model` is the pinned versioned id from config, never an alias. `response.model` is stored with the verdict.
- Requests stay under 64k tokens; triage chunks regions across requests with the same state.
- Sentences and evidence are always passed as named state fields, never spliced into instructions.
- Choice option order is fixed and canonical.

### 8.3 Judgments

**triage.** State: `{ doc: {path}, regions: [{id, kind, text, headingPath}] }`. Per region: a Choice `kind` over `factual_claim / instruction / opinion / narrative`, a Choice `evidenceKind` over `code / other_doc / external / nothing` (read only when `kind` is `factual_claim`), and a Noul `checkable`. Regions that contain a code span, a path, or a command are `code` by construction and skip the `evidenceKind` question. Mapping to audit output: `factual_claim` + `code` → `code`; + `other_doc` → `reference`; + `external` → `attested`; + `nothing` → `unverifiable`; any other `kind` → `not-a-claim`.

**supports.** State: `{ claim, claim_context, evidence, evidence_source }`. Choice `relation` with the citation-check cookbook's three options, plus Noul `addresses`. Code rule: if `addresses < 0.3`, the result is `does_not_address` regardless of the Choice.

**conflict.** State: `{ a: {text, doc, context}, b: {text, doc, context} }`. Noul `same_subject`, Choice `relation` over `consistent / contradictory / unrelated`, Noul `contradicts`. Code rule: `same_subject < 0.3` means `unrelated`. `relation = contradictory` with `contradicts < 0.5` is `review`.

### 8.4 Thresholds and outcomes

One threshold, `judge.threshold`, default 0.8, applied to Choice confidence `(p_max - 1/n) / (1 - 1/n)`. Noul confidence is `|2p - 1|` when one is needed.

| judgment | gates | `review` (warns) | clean |
|---|---|---|---|
| `supports` | `refutes` at or above threshold | `refutes` below threshold, or `does_not_address` | `supports` at or above threshold |
| `conflict` | `contradictory` at or above threshold and `same_subject` at or above threshold | `contradictory` below threshold, or the `contradicts` disagreement | otherwise |
| `triage` | never | never | n/a; probabilities are reported |

`does_not_address` leaves the claim `pending` and repeats it in the worklist with the note "evidence did not address the sentence". It does not reset ttl.

### 8.5 Numbers, dates, versions

The model is not asked to compare quantities. After `same_subject` passes for a pair, code extracts numeric tokens, version strings, and dates from both sentences. Differing values yield `contradictory` with probability 1.0 and the reason label `numbers differ: 29 vs 39`. The same extraction runs on `supports`: if the evidence contains a value for the same quantity and it differs, the result is `refutes` with the label `value differs`.

## 9. Contradiction and duplicate pass

1. **Pairing in code.** Two live claims are paired when they share a rare content token (a token appearing in fewer than 5 percent of claims, after stop-word removal and case folding), or both contain a number, version, or date and share any noun token. Claims in the same document are paired by the same rule. No topics, no configuration.
2. **Judge.** Each new pair runs `conflict` once; cached by pair texts and model.
3. **Findings.** `contradictory` at or above threshold flags both claims with `conflicts[]` naming the other. `consistent` with `same_subject` at or above threshold is a duplicate candidate, reported by `audit` as `duplicateOf`.
4. **Clearing.** `attest <id> --ack --note` on either claim records an `ack` on both and suppresses that pair until either sentence changes.

## 10. Audit

`audit --doc <path>` splits the document into regions, matches regions to existing claims by anchor, runs `triage` on uncovered regions, and emits one row per region:

```json
{ "region": { "line": 42, "kind": "sentence", "text": "Pro costs $29/month." },
  "claimId": "clm_…" | null,
  "kind": "code | reference | attested | unverifiable | not-a-claim",
  "probability": 0.91,
  "evidenceHint": "Stripe product catalog, or billing config in the api repo",
  "duplicateOf": "clm_…" | null }
```

No advice field in JSON. The human renderer prints a to-do line per row from a fixed mapping: covered → nothing; `code / reference / attested` uncovered → "record"; `unverifiable` → "rewrite as an attributed opinion, or remove"; `not-a-claim` → nothing. The mapping lives in one function and in the docs.

Recording is never automatic. The agent decides anchors and sources and calls `record`.

`--fail-uncovered` exits 2 when any uncovered region has kind `code`, `reference`, or `attested` at or above threshold.

## 11. Commands

| verb | role |
|---|---|
| `init` | create `.claims/` and `config.json`; offer to append one line to `CLAUDE.md` or `AGENTS.md`: run `hibi check --doc <file>` before relying on a document |
| `audit --doc <p> [--fail-uncovered]` | section 10 |
| `record --from-file -` | JSON array on stdin; each item: `doc`, `docQuote`, `kind`, and per kind `code: [{file, quote}]`, `ref: {file, quote}`, `sources`, `procedure`, plus `ttl`, `owner`, `by`, `suggest` |
| `check [--doc <p>] [--since <ref>] [--fail-on …]` | sections 7 and 9; JSON has `summary`, `verdicts`, `worklist`, `documents` |
| `evidence --from-file -` | JSON array: `[{claimId, evidence, source, summary, by}]`; runs `supports`, stores the cache entry and an attestation |
| `attest <id> --by <actor> [--note] [--ack]` | human vouch, or acknowledge a `review` or `conflict` finding |
| `list [--state all\|gating\|warning\|clean\|pending\|expired\|conflict\|duplicate\|suggested\|stranded] [--path <p>] [--ids-only]` | one row per claim |
| `reanchor <id> [--suggest] [--dry-run]` | as v3; refuses to reanchor an orphaned side without a new span |
| `retire <id>` | withdraw |
| `supersede --from <old> --to <new> [--dry-run]` | as v3, matching sentences verbatim |
| `archive --doc <p> [--successor <p>] [--dry-run]` | as v3 |
| `schema [--name <N>]`, `completions <shell>`, `version` | as v3 |

Removed from v3: `coverage` (folded into `audit`), `check --write`, `--run-verifiers`, `--verifier`, `--verifier-timeout`, `--verified`, `--glob` as a flag (`coarse` stays as a selector), `resolvers.json`, `diff`, `status`.

Output rules are unchanged: human on a TTY, JSON when piped, `--format`, `--json`, `--explain`, `--no-hints`, `--color`, `HIBI_ASCII`. Option tables, strict parsing, generated reference, and completions stay.

## 12. Removed capabilities and why

| removed | reason |
|---|---|
| banners and `--write` | the trust call is `check --doc`; an HTML comment is invisible in rendered docs and only reached raw-file readers; it required a write path, a nonce, a comment-syntax table, and caused diff churn |
| verifiers | hibi executes nothing; the agent-run procedure loop replaces them |
| external resolver protocol | the judge interface is the one extension point; no resolver was ever shipped |
| propositions | duplicate detection comes from sentence text and the conflict pass; supersede matches text verbatim |
| topics | pairing in code needs none; a maintained list is metadata nobody asked for |
| `verified: boolean` | replaced by attestations with actors and bases |
| store upgrade code | no compatibility with v3 |
| exit code 3, tamper detection, `--fail-on tamper` | already gone in v3; stay gone |

## 13. Implementation plan

Language: TypeScript on Bun, as the reference implementation. A Rust port is expected later and reimplements against this document, the JSON Schemas, and the fixtures.

Order:

1. Model and store v4 (`src/core/model.ts`, `schemas/*.v4.json`), judge cache, delete propositions, verifiers, resolvers, banner, upgrade code.
2. Judge interface, fake implementation, question definitions as JSON, Jev HTTP adapter with retries, fixtures for every judgment.
3. Engine: resolution (unchanged cascade), states and gating per section 7, expiry counting rule, worklist.
4. Contradiction pass with code pairing and the numeric rule.
5. `audit` on top of the region splitter.
6. CLI: new and changed verbs, option tables, generated reference.
7. Plugin skill rewritten from this document. `docs/` and README rewritten from scratch. Old ADRs and PRD deleted.
8. Dogfood on this repository and on one knowledge-style folder; tune nothing until fixtures exist for the failures observed.

Open items the user must supply: a `TYPESAFE_API_KEY` for live fixtures. Everything in section 8 beyond request shapes was taken from the docs, not from live runs.

## 14. Decisions log

| # | decision |
|---|---|
| 1 | evidence kinds on day one: same-repo spans, attestation with ttl, cross-claim contradiction; external evidence via agent-run procedures, never fetched by hibi |
| 2 | judge verdicts gate above a threshold and are cached by canonical request; never re-asked |
| 3 | any repo layout; optional cross-repo aliases; missing sibling is `unavailable`, non-gating |
| 4 | audit triages regions with the judge; author markers not required; recording stays with the agent |
| 5 | kinds: `code`, `reference`, `attested`; `unverifiable` and `not-a-claim` are audit classes |
| 6 | procedure is prose plus optional Lobster-shaped steps; hibi runs neither |
| 7 | attestations are append-only with OKF actor ids; ttl counts from evidence or a human vouch only |
| 8 | OKF is a reference for shapes, not a parsed format |
| 9 | judge behind a small interface; Jev via direct HTTP; pinned model id |
| 10 | no backward compatibility; same repo and name; store v4 |
| 11 | keep anchor cascade, option tables, lifecycle verbs; tree-sitter optional |
| 12 | TypeScript now, Rust later; contracts as JSON |
| 13 | `coverage` folds into `audit`; `evidence` and `attest` stay separate verbs |
| 14 | no topics; pair in code, gate with `same_subject` |
| 15 | `expired` warns in whole-store `check`, gates in `check --doc`; no grace window |
| 16 | one threshold at 0.8 confidence; three-way outcome gate / review / clean |
| 17 | worklist lives inside `check`; no separate verb |
| 18 | audit JSON carries `kind`, not advice; the renderer prints the to-do line |
| 19 | banners removed |
| 20 | no default ttl; required on `attested` |
| 21 | evidence stored as hash, source, summary; raw text discarded |
| 22 | propositions removed |
| 23 | docs, README, skill, and design folder rewritten; old ADRs deleted |
