# Hibi v4 specification

Status: decided 2026-10-22, implemented 2026-10-22 on branch v4-redesign. This document is the contract for the TypeScript implementation and for a later Rust port. It replaced ADR-001 through ADR-004 and the historical PRD, which are deleted. Where this document and any older text disagree, this document wins. Where this document and the code disagree, the code was taken as correct and this document was amended to match it.

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
| `reference` | one span in another document | anchor resolution, plus judge `supports(sentence, referencedText)` whenever both sides locate, answered from the cache when the request is unchanged |
| `attested` | none required; optional `sources[]` pointers; optional `procedure` | `ttl` elapsed since the newest counting attestation; or submitted evidence judged `refutes` |

The audit also names two region classes that are never recorded: `unverifiable` (a stated fact nothing could confirm) and `not-a-claim` (instruction, opinion, narrative, heading). It reports two further row kinds, `unsure` and `pending`, when triage cannot decide (section 10).

Kind is explicit at record time. It is not inferred from the shape of the sides.

### 4.1 Cross-repository evidence

`config.repos` maps an alias to a directory: `{ "api": "../api" }`. A relative directory is resolved against the repository root; an absolute path is used as given. A side's `file` may be `api:src/limits.ts`. An aliased file has the form `<alias>:<path>` where the alias starts with a letter and contains letters, digits, `_`, `.`, or `-`, and the path does not start with `/` or `\`, so `https://…` and `C:\…` are not aliased files.

- **Containment.** An evidence file must be inside its root: the anchor root for a plain path, the alias directory for an aliased one. A path that resolves outside it (`../other/x.ts`, `api:../../x.ts`) has the label `path escapes its root`.
- **At check time**, a side whose alias is not defined in `config.repos` resolves to `unavailable` with the reason label `repo alias "x" is not defined in .claims/config.json repos`. A side whose alias is defined but whose directory does not exist resolves to `unavailable` with the label `sibling repo "x" is not checked out at <dir>`. A side whose file escapes its root resolves to `unavailable` with the label `path escapes its root` (in the verdict's reasons, `evidence: path escapes its root`). `unavailable` warns and never gates.
- **At record time**, an alias not defined in `config.repos` is refused, with an error naming the alias and the known aliases. An alias whose sibling repo is not checked out is also refused, because a claim is recorded against text on disk. A file that escapes its root is refused with `<file>: path escapes its root; an evidence file must be inside the anchor root` (or `inside the directory of repo alias "x"`). `reanchor` refuses an unknown alias, a sibling that is not checked out, and an escaping path in a new span the same way. Without a new span, `reanchor` keeps an evidence side whose sibling repo is not checked out unchanged and reports the warning `evidence side <file> kept unchanged: <reason>`.

Without aliases, nothing changes.

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

`resource` is required per entry and is an opaque pointer. Hibi never dereferences it. When evidence is submitted with a `source` equal to a listed `id` or `resource`, the result carries the reason label `source matches declared source "<id or resource>"`. The label changes no state.

## 5. Attestations and ttl

Each claim has an append-only `attestations[]`:

```json
{ "by": "human:alice@example.com", "at": "2026-10-22T09:12:00Z",
  "basis": "vouch | evidence | ack", "note": "checked Stripe, still $29", "evidenceHash": "…",
  "ack": { "pairId": "clm_a:clm_b", "sentenceHashes": ["…", "…"] } }
```

`note`, `evidenceHash`, and `ack` are optional. The `ack` field is `{ pairId, sentenceHashes: [a, b] }` or `{ judgeKey }`. It is required when `basis` is `ack` and is invalid for any other basis. Unknown keys are rejected.

- `by` uses OKF actor form: `human:<id>`, `agent:<name>`, `process:<name>`. The CLI's `--by` is required on `attest`. On `record` and `evidence`, every item needs an actor: the item's `by` key, or `--by`, which is the default for items without one. There is no default from git configuration; an item with neither is refused.
- `basis: vouch` is a bare statement. `record` appends a vouch by the recording actor when it creates a claim, and appends another each time the same sentence is re-recorded in the same document (section 11, `record`). `attest` without `--ack` appends a vouch.
- `basis: evidence` means the judge returned `supports` at or above the threshold for evidence submitted with `hibi evidence`, which accepts `attested` claims only (section 11), and the value rule (section 8.5) did not fire. The attestation carries the evidence hash and takes the agent's summary as `note`. Evidence whose request is already in the judge cache and whose cached verdict is `supports` appends an evidence attestation the same way; the judge is not asked again.
- `basis: ack` acknowledges a finding and requires a note.
  - `ack: { pairId, sentenceHashes }` acknowledges a contradiction between two claims. `pairId` is the two claim ids, sorted, joined with `:`. `sentenceHashes` holds the sentence hashes of the two claims in `pairId` order. The same attestation is written to both claims. It suppresses the pair only while both current sentences still hash to the stored values (sections 9.3 and 9.4).
  - `ack: { judgeKey }` acknowledges a `review` verdict. For an `attested` claim, `judgeKey` is the cache key of the claim's newest `supports` cache entry, where newest is the later of the entry's `at` and its evidence `at`, and the later entry wins a tie; it is an error when the claim has no `supports` entry. For a `reference` claim, `judgeKey` is the key of the request built from the claim's sentence and referenced span as they read now, the same request `check` sends (section 7.3). It is an error when there is no current `supports` verdict: when either side does not locate, or when that key is not in the cache (`run hibi check first`). While the claim's current judge verdict has that key, the verdict stays `review`, no longer warns, and the claim does not appear in the worklist (section 7.5). The verdict's reasons carry `review acknowledged by <by> at <at>[: <note>]; it no longer warns`.
- **Sentence hash.** sha256, lowercase hex, of the sentence after Unicode NFC normalization, replacing every run of whitespace with one space, and trimming.
- **Counting rule.** `ttl` is measured from the newest attestation whose basis is `evidence`, or whose basis is `vouch` with a `human:` actor. An `agent:` or `process:` vouch is recorded and does not count. An `ack` never resets ttl. Newest is the latest `at`; on equal times the later entry in the list wins. `attest` reports `countsTowardTtl`.
- `ttl` is an ISO 8601 duration (`P90D`, `PT12H`): at least one component, integer amounts, a time part only after `T`. It is required on `attested` claims and optional elsewhere. There is no store-wide default. A claim with a `ttl` and no counting attestation is expired. Otherwise it expires at the counting attestation's `at` plus the `ttl`, and is expired when now is at or after that instant. Years and months add calendar units in UTC and clamp the day to the last day of a shorter target month; weeks, days, hours, minutes, and seconds add fixed seconds, a day being 86,400 seconds.
- `owner` is optional, must be in actor form, and names the actor to contact when the claim expires. It is separate from who attested.

## 6. Store

`.claims/` at the repository root, committed. Store `version: "v4"`. No upgrade path from v3: a store with any other version is refused at open and by `init`, with the message ``this claim store is version <v>; hibi v4 has no upgrade path from it. Remove <dir>, run `hibi init`, and re-record.``

```
.claims/
  config.json
  documents/<doc_id>.json
  claims/<claim_id>.json
  judge/<sha256>.json        # cached judge verdicts
```

Ids are `doc_` and `clm_` followed by 16 lowercase hex characters. A claim id is random. A document id is derived from the document's repository-relative path: the xxHash64 of the path's UTF-8 bytes with seed 0, as 16 hex digits zero-padded on the left, so the same path always maps to the same document. There are no propositions. The claim holds its sentence.

**Paths.** Every path a user passes (`--doc`, `--code-file`, `--ref-file`, `--path`, `--from`, `--to`, `--successor`, and the `doc` and `file` keys of a `--from-file` item) is normalized before use: to a POSIX path relative to the anchor root, so `./docs/a.md`, `docs/a.md`, and `<root>/docs/a.md` are the same document. An absolute path is compared after resolving symlinks in it and in the root. An aliased file keeps its `alias:` prefix and has its relative part normalized (`api:./src/x.ts` is `api:src/x.ts`). The store keys documents and sides by the normalized form, and JSON output echoes it.

All store records are strict:
 unknown fields are rejected at load. JSON Schemas generated from the TypeScript model are in `schemas/*.v4.json` for `Selector`, `SelectorBundle`, `Attestation`, `Source`, `Procedure`, `Document`, `Claim`, `Config`, `JudgeCacheEntry`, `Remediation`, `RemediationAction`, and `Verdict`.

### 6.1 config.json

```json
{
  "version": "v4",
  "repos": { "api": "../api" },
  "judge": { "model": "jev-1.14.0", "threshold": 0.8 },
  "ignore": ["docs/generated/**"]
}
```

`judge.model` must match `jev-<major>.<minor>.<patch>`; an alias such as `jev-latest` is rejected. `judge.threshold` is a number in [0, 1]. `repos` defaults to `{}` and `ignore` to `[]`. `ignore` lists globs of documents hibi does not audit; `audit` refuses a matching document and names the glob. `init` writes every field with the defaults `jev-1.14.0` and `0.8`, and never overwrites an existing config.

### 6.2 Document

```json
{ "id": "doc_…", "path": "docs/pricing.md", "lifecycle": "active | superseded | archived",
  "supersededBy": "doc_…", "createdAt": "…" }
```

A document is created the first time a claim is recorded on its path.

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

`owner` and `ttl` are optional keys. `ref` is the git HEAD the claim was last recorded or reanchored against, or `WORKTREE` when there is none. `evidence` holds code spans for `code` (at least one) and exactly one document span for `reference`. It is empty for `attested`, which requires `ttl`. Every selector bundle has at least one selector.

Selectors:

- `text-quote`: `exact`, plus `prefix` and `suffix` of up to 48 characters of context each.
- `text-position`: `start` and `end` character offsets (UTF-16 code units) at record time.
- `ast-node` (code only, optional): language, node type, structural hash, semantic hash. Used for reason labels only.
- `value` (code only, optional): language, node kind, and the first literal inside the quoted span.

The `doc` bundle and the `reference` evidence bundle use `text-quote` and `text-position` only. There is no `coarse` selector.

### 6.4 Judge cache entry

```json
{ "key": "<sha256 hex>",
  "model": "jev-1.14.0",
  "judgment": "triage | supports | conflict",
  "subject": { "claimId": "clm_…", "pairId": "clm_a:clm_b", "docId": "doc_…" },
  "request": { /* exact state and questions sent */ },
  "answers": { /* exact System One answers returned */ },
  "evidence": { "hash": "…", "source": "…", "summary": "…", "by": "agent:opus", "at": "…",
                "values": [ { "kind": "money", "normalized": "29", "unit": "USD" } ] },
  "at": "…" }
```

- **Key.** The key is the sha256 hex of `${model}\n${canonicalJson(request)}`, where `model` is `config.judge.model` and `request` is the request as sent. The file is `judge/<key>.json`. `canonicalJson` sorts object keys by UTF-16 code unit, keeps array order, writes no whitespace, drops keys whose value is undefined, turns undefined array items into `null`, writes numbers and strings as `JSON.stringify` does, and throws on non-finite numbers and on values JSON cannot represent. The entry's `model` is the model the response reported.
- **Subject.** At least one of `claimId`, `pairId`, `docId` is set. `supports` entries name the claim, `conflict` entries the pair, `triage` entries the document. A `triage` entry holds one chunk request and its answers.
- **Answers.** `answers` is the raw System One `answers` object. Results are decoded from it at read time by the same code that decodes a live response, so a cached and a live answer are read identically and a changed threshold applies without asking again.
- **Evidence.** Raw evidence text submitted with `hibi evidence` is never stored. The stored `request.state.evidence` is replaced by `sha256:<hash>`, while the key is computed over the request as sent, so the stored request cannot reproduce the key. The `evidence` block keeps the hash, source, summary, actor, time, and `values`. For a `reference` claim the request is stored as sent, because the referenced text is committed repository text. When the same request is submitted again, the entry is rewritten with the new `evidence` block.
- **`evidence.values[]`** is optional: `{ kind, normalized, unit? }` for each number, money amount, version, and date found in the evidence text (section 8.5). It exists because the value rule compares the claim's sentence with the evidence, and `check` runs after the raw text is gone. Storing the extracted values lets `check` apply the rule to a cached verdict without the text. Only the values are kept, never the text around them. An entry without `values` is treated as having none.

## 7. Resolution

### 7.1 Anchors

The v3 cascade is kept as is for every span side, doc or evidence:

1. Locate the quote: exact occurrences ranked by stored context; else a fuzzy match near the stored position within `min(256, floor(len * 0.4))` edits.
2. Located, similarity at or above 0.9, no AST or value change, start within 4 chars: `unchanged`.
3. Same, at a new offset: `moved`.
4. Similarity below 0.9, or AST semantic hash changed, or in-span literal changed, or (where the structural checks do not run) a numeric token in the span changed: `changed`, with a reason label.
5. Not found: `orphaned`.
6. Several exact occurrences with equal context scores and a quote of at least 8 chars: `ambiguous`.

**Structural checks.** The AST and in-span literal checks in step 4 run on a code span only when tree-sitter can parse it: the analyzer is on and the file's language has a grammar. The grammars for TypeScript (`.ts`, `.js`, `.mjs`, `.cjs`), TSX (`.tsx`, `.jsx`), Python, Rust, Go, and Java are embedded in the binary as wasm; nothing is downloaded. `--no-ast` skips them. A code file in any other language gets the text selectors only (`text-quote`, `text-position`; no `ast-node` or `value` selector at record), with no warning.

**Numeric-token rule.** Where the structural checks do not run (a doc span, a `reference` span, a code span under `--no-ast`, or a code span with no grammar), a located span is `changed` with the label `a number in the sentence changed` when the numbers in it differ from the numbers in the stored quote. The located text is first widened to whole numbers at its edges, so a quote `= 5` that now matches the start of `= 50` reads `= 50`.

### 7.2 Claim states

Per claim, `check` produces a verdict:

| field | values |
|---|---|
| `doc` | `unchanged / moved / changed / ambiguous / orphaned` |
| `evidence` | same five, plus `unavailable`; the worst state over the evidence sides; absent when there is no evidence side |
| `judge` | `supported / refuted / review / pending`; absent when no judgment applies |
| `expired` | boolean, from the counting rule in section 5 |
| `conflicts` | list of `{ claimId, probability, reason }` from the contradiction pass; gating findings only |
| `gates` | boolean |
| `warns` | boolean; true only when the verdict warns and does not gate, so `gates` and `warns` are never both true |
| `reasons` | list of strings: every reason label, in this order: doc-side anchor labels (prefixed `doc: `), evidence-side anchor labels (prefixed `evidence: `), the expiry label, judge labels (prefixed `judge: `), the gating-rule labels, then a `worklist: …` label when the claim is in the worklist |

The verdict also carries `claimId`, `documentId`, `kind`, a `remediation` menu (recommended action id and actions with ready-to-run commands), and a `detail` block (located regions, similarity, changes). A claim whose `enforcement` is `retired` gets no judge state and no conflict findings.

### 7.3 When the judge is consulted

Every judgment is looked up in the judge cache first. The judge is asked only on a miss.

- `reference`: when the doc side and the evidence side are each `unchanged` or `moved` and both texts were located. The request is `{ claim, claim_context, evidence, evidence_source }` with the current sentence, its context (section 8.3), the referenced text, and the evidence file. The cache key covers all four, so a change to the sentence, its surrounding context, or the referenced text asks again, and anything else does not. In any other anchor state the judge is not asked and the reasons carry `judge: not consulted while the doc side is <state>` (or `evidence side`).
- `attested`: `check` does not ask. It reads the claim's newest cached `supports` entry (section 5 for newest). A claim with no entry has no judge state.
- `evidence` verb: once per request key `(model, sentence, context, evidence, source)`.
- contradiction pass: once per pair request key (section 9).
- `audit`: once per triage chunk request key.
- No API key, a rejected key, or a network failure after retries is `JudgeUnavailable`. It maps to `pending` for a `reference` claim (reason `pending: <message>`), to a `pending` item in `evidence` with the label `judge unavailable: <message>` (nothing is written for it), to `pending` rows for every region of the audit chunk, and to a `pending` conflict pair. Nothing else changes. There is no offline flag.
- An answer the judge gave but hibi cannot use is `JudgeError` (section 8.2). It never aborts a run and never exits `1`: it maps to `pending` for a `reference` claim and for an `evidence` item, both with the reason `judge error: <message>`; to `pending` rows for the audit chunk; and to a `pending` conflict pair, which warns on both claims with `conflict check pending with <id>: judge error`. Nothing is cached for it.
- A `reference` claim whose referenced span is longer than 32,000 characters is `pending` with the reason `referenced span is longer than 32,000 characters`; the judge is not asked. Submitted evidence has the same limit, enforced as a validation error (section 11).

### 7.4 Gating

| condition | effect in `check` | effect in `check --doc` |
|---|---|---|
| `changed / orphaned / ambiguous` on either side, enforced claim | gate | gate |
| `judge: refuted` | gate | gate |
| `conflicts` non-empty | gate | gate |
| `expired` | warn | gate |
| `moved` or `unavailable` on either side; `judge: pending`; `judge: review` that is not acknowledged | warn | warn |
| conflict `review` finding: `contradictory` below its bar | warn | warn |
| conflict pair the judge could not answer | warn | warn |
| `suggested` or `retired` claim | never | never |

A conflict review finding warns with the reason label `possible conflict with <id>: <reason>`. A pending pair warns on both claims with `conflict check pending with <id>: judge unavailable`. A pair whose answer could not be used warns with `conflict check pending with <id>: judge error`. A `conflicts[]` entry whose probability is below the threshold also warns, with a label beginning `possible conflict with <id>`; the pass produces gating entries only at or above it.

`check --doc` is the mode in which `expired` gates. It is the default whenever `--doc` is given. A `suggested` or `retired` claim never gates or warns, and when a rule would have fired its reasons say `<enforcement> claim: never gates or warns`.

Exit codes: `0` clean or warnings only; `2` gating: a `changed`, `orphaned`, or `ambiguous` side, `refuted`, a conflict, or `expired` under `--doc`; `1` operational error, including `check --doc` on a path with no registered claims. A judge that is unreachable or whose answer cannot be used is never an operational error; it yields `pending`. `--fail-on gating|warn|never`: `gating` is the default; under `warn`, any warning also exits `2`; under `never`, the exit is always `0`.

### 7.5 Worklist

`check` output includes `worklist[]`. A claim is listed when it is not `retired`, its document is `active`, and one of these reasons applies. When several apply, the first of `expired`, `review`, `pending`, `never-checked` is the reason, and the claim is listed once:

1. `expired`: any claim with a `ttl` that ran out, `code` claims included.
2. `review`: a `reference` or `attested` claim whose judge state is `review`, unless a `review` acknowledgement names the key of that verdict (section 5).
3. `pending`: a `reference` or `attested` claim whose judge state is `pending`.
4. `never-checked`: an `attested` claim with a `procedure` or at least one `sources` entry, and no `supports` cache entry. These are the claims an agent can collect evidence for.

A pure vouch claim (`attested`, no `procedure`, no `sources`) is listed only when it is expired: nothing tells an agent where to look, so the next step is a human vouch. A `code` claim is listed only when it is expired.


A claim on a superseded or archived document is never listed. The verdict's reasons gain a `worklist: …` label for the reason. Each item carries `claimId`, `sentence`, `kind`, `document` (the claim's document path), `procedure`, `sources`, `owner` (or null), the reason, and `lastEvidence` (`summary`, `source`, `at`, `by`) from the claim's newest cache entry that has an evidence block, when there is one. `list --state pending` returns the same set. A scoped check (`--doc`, `--since`) lists only claims in scope.

## 8. Judge

### 8.1 Interface

```
triage(doc, regions[])  -> per region: { regionId, kind, kindProbabilities, kindConfidence,
                                         evidenceKind?, evidenceProbabilities? }
supports({ sentence, context, evidence, source })
                        -> { relation: supports|refutes|does_not_address, probabilities, confidence,
                             addresses, forced? }
conflict(a, b)          -> { relation: consistent|contradictory|unrelated, probabilities, confidence,
                             sameSubject, contradicts, forced? }
```

`a` and `b` are `{ text, doc, context }`. Every call returns `{ model, request, answers, result }`, so the exact request and raw answers can be cached. The interface is data in, data out. Question definitions (instructions and criteria) live in `src/judge/questions/*.json` and are loaded, not written in code. Jev is the only shipped implementation.

A fake implementation serves tests. It is scripted by sentence or pair text, and it encodes each scripted result as System One answers and decodes them with the Jev mappers, so a result produced by the fake and one produced by Jev take the same decode path.

### 8.2 Jev adapter

- Direct HTTP: `POST https://api.typesafe.ai/v1/systemone` (the base URL can be overridden with `TYPESAFE_BASE_URL`), bearer `TYPESAFE_API_KEY`, body `{ model, state, questions }`. The official JS SDK is not used; it fails under Bun as of 0.7.0.
- Retries: 408, 429, and every 5xx status (including 529), and connection errors and per-attempt timeouts, are retried up to 2 times (3 attempts). Backoff starts at 500 ms, doubles, caps at 5 s, with up to 25 percent downward jitter. A `retry-after` header (seconds or HTTP date) replaces the backoff; a value over 60 s is clamped to 60 s. Each attempt has a 10 s timeout. At most 8 requests are in flight in the process.
- Errors: a missing key, a 401 or 403, and exhausted retries are `JudgeUnavailable`. A 4xx other than 401, 403, 408, and 429 (400, 422, …), a 200 whose body is not JSON, a body without an `answers` object and a `model` string, and answers that cannot be decoded (a missing answer, the wrong answer type, an unknown Choice option, triage chunks answered by different models) are `JudgeError`. Neither aborts a run; both map to `pending` (section 7.3).
- `model` is the pinned versioned id from config, never an alias. `response.model` is stored with the verdict. Triage chunks must all be answered by the same model.
- Budgets, at an estimate of 4 characters per token over the request JSON: 48k tokens per request (model, state, and all questions) and 24k tokens for the state plus any one question. The model limits are 64k and 32k. Triage packs consecutive regions into chunks and closes a chunk before either budget would be exceeded. Each request's state holds `doc` plus only that chunk's regions, and question ids and `regions[i]` paths index within the request. A single region that does not fit is an error.
- Sentences and evidence are always passed as named state fields, never spliced into instructions.
- Choice option order is fixed and canonical.

### 8.3 Judgments

**triage.** State: `{ doc: {path}, regions: [{id, kind, text, headingPath}] }`. Per region: a Choice `kind` over `factual_claim / instruction / opinion / narrative`, and a Choice `evidenceKind` over `code / other_doc / external / nothing` (read only when `kind` is `factual_claim`). There is no third question. A region is `code` by construction, with `evidenceKind` `code` at probability 1 and no `evidenceKind` question sent, when any of these holds: its kind is `fenced` or its text contains a fence marker; it contains an inline code span that is a command (starts with a flag, or a word followed by arguments) or holds a path (a slash-separated name, or a path token); or a prose token outside code spans and URLs is a path token. A path token is `./x`, `../x`, `~/x`, `/x`, `dir/`, `dir/file.ext`, `file.ext` with a known source or config extension, or `.dotfile`. `Name.js` (Node.js, Next.js) is a product name, not a file. The extension list and patterns are in `src/judge/jev.ts` (`isCodeByConstruction`). Mapping to audit output: `factual_claim` + `code` → `code`; + `other_doc` → `reference`; + `external` → `attested`; + `nothing` → `unverifiable`; any other `kind` → `not-a-claim`.

**supports.** State: `{ claim, claim_context, evidence, evidence_source }`. Choice `relation` uses the citation-check cookbook's option names `supports / contradicts / says_nothing`, which the adapter maps to `supports / refutes / does_not_address`. A Noul `addresses` accompanies it. Code rule: if `addresses < 0.3`, the result is `does_not_address` regardless of the Choice, with `forced: "addresses"`, and `confidence` is the Noul confidence of `addresses`. `probabilities` stay the Choice probabilities, under the hibi names. Otherwise `confidence` is the Choice confidence.

**conflict.** State: `{ a: {text, doc, context}, b: {text, doc, context} }`, with `a` the claim with the smaller id. Noul `same_subject`, Choice `relation` over `consistent / contradictory / unrelated`, Noul `contradicts`. Code rule: `same_subject < 0.3` means `relation: unrelated` with `forced: "same_subject"`, and `confidence` is the Noul confidence of `same_subject`. Otherwise `confidence` is the Choice confidence.

**Context.** `claim_context` and each side's `context` come from one function: the Markdown heading path above the sentence, outer to inner, joined with ` > `, a blank line, then the paragraph holding the sentence (the lines around it up to a blank line, a heading, or a fence line). The result is at most 600 characters. The sentence is never cut: the paragraph is trimmed around it first, with cut ends marked `…`, then the heading path from its outer end. A sentence of 600 or more characters, or one that cannot be located in the current document, is its own context.

### 8.4 Thresholds and outcomes

One threshold, `judge.threshold`, default 0.8, applied to Choice confidence `(p_max - 1/n) / (1 - 1/n)`. Noul confidence is `|2p - 1|` when one is needed.

| judgment | gates | `review` (warns) | clean |
|---|---|---|---|
| `supports` | `refutes` at or above threshold; the value rule | `refutes` or `supports` below threshold, or `does_not_address` | `supports` at or above threshold |
| `conflict` | `contradictory` at or above threshold, `same_subject` at or above threshold, and `contradicts` at or above 0.5; the value rule | `contradictory` with any of those three below its bar | otherwise |
| `triage` | never | never | n/a; probabilities are reported |

`supports` outcomes, first matching row, with the reason label each produces:

| rule | judge state | label |
|---|---|---|
| relation is `does_not_address` | `review` | `evidence did not address the sentence`, with ` (addresses=0.12)` appended when the Noul forced it |
| value rule fires (section 8.5) | `refuted` | `value differs: <label>` |
| `supports`, confidence at or above threshold | `supported` | `evidence supports the sentence (confidence 0.94 >= threshold 0.8)` |
| `refutes`, confidence at or above threshold | `refuted` | `evidence refutes the sentence (confidence 0.92 >= threshold 0.8)` |
| `supports` or `refutes` below threshold | `review` | `evidence leans supports, confidence 0.55 below threshold 0.8` |

`does_not_address` is `review`, not `pending`. The claim warns, is listed in the worklist with reason `review`, and is not cleared. It does not reset ttl. `pending` means only that the judge could not be reached.

In a `check`, judge labels are prefixed `judge: `. Review verdicts can be acknowledged (section 5).

### 8.5 Numbers, dates, versions

The model is not asked to compare quantities. Code extracts values from text with a fixed table of rules, applied in this order; a match overlapping text an earlier rule took is skipped, so `$29` is one money value and not also the number `29`:

| rule | kind | matches | normalized | unit |
|---|---|---|---|---|
| `iso-date` | date | `2026-10-22`, also the date part of a timestamp | the date | none |
| `month-day-year` | date | `October 22, 2026`, `Oct 22 2026` | `2026-10-22` | none |
| `v-version` | version | `v1.2.3`, `v4`, `v2.0.0-beta.1` | without `v` | none |
| `dotted-version` | version | three or more dotted parts, optional pre-release | as written | none |
| `keyword-version` | version | `version 1.2`, `release 3.10`, `ver 2.1` | the number | none |
| `currency-sign` | money | `$29`, `€ 5`, `£1,299.00`, `$5m`, `$2.5k`, `$1bn` | the number, times 1e3, 1e6, or 1e9 for a `k`, `m`, or `b`/`bn` suffix (`$5m` is `5000000`) | `USD`, `EUR`, `GBP` |
| `currency-code-after` | money | `29 USD` | the number | the code |
| `currency-code-before` | money | `USD 29` | the number | the code |
| `percent` | number | `5%`, `12.5 %` | the number | `%` |
| `duration-word` | number | `5 days`, `30 seconds`, `1 hour` | the number | singular unit word |
| `duration-short` | number | `90d`, `500ms`, `12h` (unit touches the number) | the number | unit word |
| `number` | number | `29`, `1,000`, `0.75` | the number | none |

Currency codes are `USD EUR GBP JPY CAD AUD CHF`. A magnitude suffix is read only on a currency-sign amount, so `$5m` is five million dollars and never five minutes. A number is normalized by removing thousands commas and leading zeros and trailing fractional zeros (`1,299.50` is `1299.5`). Regular expressions, boundary rules, and unit tables are in `src/engine/values.ts` (`VALUE_RULES`) and are checked by `test/values.test.ts`; a port must pass those tests.

A value's **group** is its kind plus its unit: `number`, `number %`, `number day`, `money USD`, `version`, `date`. Two texts are compared only within groups present in both. A group differs when the two texts share no value in it, so `$29` against `$39` differs, `$29 or $49` against `$29` does not, and `5 days` against `5%` is not compared. Versions are compared after dropping a leading `v` and trailing `.0` parts, so `v1.2` equals `1.2.0`. The first differing group, ordered money, number, version, date and then by group name, names the label: `numbers differ: 29 vs 39` (money uses `numbers`), `versions differ: 1.2.0 vs 1.3.0`, `dates differ: 2026-10-01 vs 2026-11-01`. Several values in one group are listed comma-separated.

**On `supports`**, the unitless `number` group is excluded: a bare number in the evidence (a count, an id, a line number) says too little to refute. Only money, unit-bearing numbers (`%`, durations), versions, and dates count. The rule fires when the relation is not `does_not_address` and the sentence and the evidence hold differing values in a shared group other than `number`. It yields `refuted` with the label `value differs: <label>` and takes precedence over the Choice outcome. The evidence values come from `evidence.values` for a cached evidence verdict and from the live referenced text for a `reference` claim.

**On `conflict`**, every group counts, the unitless `number` group included. The rule fires only after
 `same_subject` reaches the threshold and the Noul did not force `unrelated`. It yields a conflict finding with probability 1 and the label from the comparison, and it replaces the relation outcome, including a `consistent` duplicate outcome.

## 9. Contradiction and duplicate pass

The pass runs over the live claims of active documents: enforcement other than `retired`, on a document whose lifecycle is `active`. Pairs are formed over all of them. A whole-store `check` and `check --since` judge every pair. `check --doc` and `audit` judge only the pairs with at least one claim in the document; a finding on such a pair may name a claim in another document.

### 9.1 Pairing in code

Two live claims form a pair when either rule holds. Claims in the same document pair by the same rules. There are no topics and no configuration.

- **Content tokens.** A token is a case-folded run of Unicode letters, at least 3 letters long, not in the fixed English stop-word list (`STOP_WORDS` in `src/engine/pairing.ts`). Digits do not make tokens.
- **Rare-token rule.** The claims share a rare token. A token is rare when it appears in fewer than `max(3, min(ceil(0.05 n), 10))` live claims, where `n` is the number of live claims. The floor of 3 means a token shared by exactly two claims is always rare. The ceiling of 10 keeps a token shared by hundreds of claims in a large store from pairing them all.
- **Values rule.** Both sentences contain a value in the same group (kind plus unit, section 8.5) and the claims share any content token, rare or not.

Each pair has a `pairId` (the two ids sorted and joined with `:`; side `a` is the smaller id) and a `why` label: `shared rare tokens: a, b` for the first rule, `both state <group> values and share: x, y` for the second, joined with `; ` when both hold. Pairs are sorted by `pairId`.

### 9.2 Judge

Each pair runs `conflict` once, with sides in `pairId` order, each carrying its sentence, document path, and context (section 8.3). The result is cached by the request key. At most 8 pair requests are in flight. A pair the judge cannot answer (`JudgeUnavailable`) or whose answer cannot be used (`JudgeError`) is `pending` and does not stop the pass.

### 9.3 Outcomes and findings

The pass returns one outcome with one reason label for every pair, in `pairs[]` as `{ pairId, why, outcome, reason }`. The first matching row decides:

| outcome | rule | example label |
|---|---|---|
| `pending` | the judge was unavailable, or its answer could not be used | `judge unavailable: <message>`, `judge error: <message>` |
| `unrelated` | the `same_subject` Noul forced `unrelated` | `not the same subject (same_subject=0.12)` |
| `conflict` | `same_subject` at or above threshold and the values differ (section 8.5) | `numbers differ: 29 vs 39` |
| `conflict` | `contradictory`, with confidence, `same_subject`, and `contradicts` each at or above their bar (threshold, threshold, 0.5) | `contradictory (p=0.93, same_subject=0.95)` |
| `review` | `contradictory` with any of those below its bar | `review: contradictory, confidence=0.55 below threshold 0.80` |
| `duplicate` | `consistent` with `same_subject` at or above threshold | `consistent and same subject (same_subject=0.95)` |
| `clean` | anything else | `unrelated (confidence=0.80, same_subject=0.40)` |
| `acked` | a `conflict` or `review` outcome covered by an `ack` on either claim | `acknowledged: numbers differ: 29 vs 39` |

The `review` label lists every bar missed, for example `contradicts=0.30 below 0.50, same_subject=0.60 below threshold 0.80`.

`conflict` and `review` become findings on both claims, with the probability of `contradictory` (1 when the values differ). A `conflict` finding becomes an entry in `conflicts[]` and gates; a `review` finding warns (section 7.4). `duplicate` becomes a duplicate candidate on both claims, with the probability of `consistent`. `audit` reports the candidate with the highest probability, ties to the smaller claim id, as `duplicateOf`. `list --state duplicate` also includes claims whose sentences are equal after whitespace normalization, with the `why` label `same sentence`. `check` reports `summary.conflictPass` as `{ pairs, requests, pending, scope }`: the pairs in scope, the judge calls answered in this run, the pairs pending, and `scope`, which is `store` for a whole-store or `--since` check and `doc` for `check --doc`.

### 9.4 Clearing

`attest <id> --by <actor> --ack conflict:<other-id> --note <text>` on either claim records an `ack` on both, shaped `{ pairId, sentenceHashes }` (section 5). A pair whose `conflict` or `review` outcome an ack covers becomes `acked` and produces no finding. The ack covers the pair while the sentence hashes of both claims, as they read now, equal the stored ones in `pairId` order. Changing either sentence ends the suppression, and the pair is judged again; the cached verdict answers unless the request bytes changed. The ack never resets ttl.

## 10. Audit

`audit --doc <path>` splits the document into regions, matches regions to existing claims by anchor, runs `triage` on uncovered regions, and emits one row per region. It refuses a document that does not exist or matches `config.ignore`. It runs the contradiction pass for `duplicateOf` over the pairs with a claim in the document only (section 9).

**Regions.** Blocks are separated by blank lines; a fenced block stays one block, and one `fenced` region, even with blank lines inside it. Inside a block, a line that opens a heading, list item, blockquote, or table row starts a new piece. Each piece is split at a sentence terminator (`.`, `!`, `?`, optionally followed by a closing quote, parenthesis, or bracket) followed by whitespace. A region's kind is `heading`, `list-item`, `table-row`, `fenced`, or `sentence` (blockquote and prose lines are sentences). Offsets are UTF-16 code units. Ids are `r<index>` in document order. `headingPath` is the Markdown headings above the region; a heading's own path holds its ancestors, not itself.

**Covered.** A region is covered when it overlaps the located doc span of a live claim on that document. The judge is not asked about covered regions.

Output:

```json
{ "doc": "docs/pricing.md",
  "summary": { "regions": 12, "covered": 3, "uncovered": 9,
               "byKind": { "code": 2, "reference": 0, "attested": 1, "unverifiable": 1,
                           "not-a-claim": 3, "unsure": 1, "pending": 1 },
               "requests": 1 },
  "rows": [
    { "region": { "id": "r4", "line": 42, "kind": "sentence", "text": "Pro costs $29/month." },
      "claimId": "clm_…" | null,
      "kind": "code | reference | attested | unverifiable | not-a-claim | unsure | pending",
      "probability": 0.91,
      "duplicateOf": "clm_…" | null } ] }
```

- A covered row has the claim's `claimId` and kind, no `probability`, and `duplicateOf` from the contradiction pass (it runs on demand).
- An uncovered row takes its kind from triage (section 8.3). The decision is taken only when the `kind` Choice confidence is at or above `judge.threshold` and, for a `factual_claim`, the `evidenceKind` Choice confidence is too. `kind` is checked first. A region that is `code` by construction passes the second check.
- `probability` is the probability of the triage kind the judge picked (`kindProbabilities[kind]`), not of the evidence kind.
- Otherwise the row is `unsure`. It carries `unsure: "kind" | "evidenceKind"`, naming the Choice that fell short, and `probabilities: { kind: {…}, evidenceKind?: {…} }` instead of `probability`. `evidenceKind` is present when the top kind is `factual_claim`.
- A `pending` row means the judge was unavailable or its answer for the region's chunk could not be used (section 7.3); every region of that chunk is `pending`. It has neither `probability` nor `probabilities`.

- `summary.requests` is the number of triage requests the judge answered in this run. Cache hits are not counted, so a second run over the same text reports 0. `byKind` has a key for every row kind.

No advice field in JSON. The human renderer prints a to-do line per row from a fixed mapping: covered → nothing; uncovered `code / reference / attested` → "record"; `unverifiable` → "rewrite as an attributed opinion, or remove"; `not-a-claim` → nothing; `unsure` on `kind` → "unsure: read it yourself"; `unsure` on `evidenceKind` → "unsure: evidence kind"; `pending` → "pending: judge unavailable". The mapping lives in one function and in the docs.

Recording is never automatic. The agent decides anchors and sources and calls `record`.

`--fail-uncovered` exits 2 when any uncovered row has kind `code`, `reference`, or `attested`. Such a row exists only when triage was confident, so `unsure` and `pending` rows do not trigger it.

## 11. Commands

Every command accepts the global options: `--format human|compact|json|json-pretty`, `--json` (alias for `--format json`), `--explain`, `--no-hints` (also `HIBI_ADVICE=0`), `--color auto|always|never` (also `NO_COLOR`, `FORCE_COLOR`), `--cwd <dir>` (anchor root, default the current directory), `--store-dir <dir>` (default `<anchor root>/.claims`), `--no-ast`, `--help`. Unknown flags are errors. `HIBI_ASCII=1` selects ASCII symbols.

| verb | flags | role |
|---|---|---|
| `init` | `--instructions <path>` | create `.claims/` and `config.json`; with `--instructions`, append one line to an agent instruction file |
| `record` | `--doc <path>`, `--doc-quote <text>` or `--doc-range <range>`, `--kind code\|reference\|attested`, `--code-file <path>` (repeatable) with `--code-quote <text>` or `--code-range <range>`, `--ref-file <path>` with `--ref-quote <text>` or `--ref-range <range>`, `--source <id>=<resource>` (repeatable), `--instructions <text>`, `--suggest`, `--ttl <duration>`, `--owner <actor>`, `--by <actor>` (required unless every item has `by`), `--from-file <path\|->` | record one claim from flags, or a batch from JSON |
| `check` | `--since <ref>`, `--doc <path>`, `--overview`, `--fail-on gating\|warn\|never` | sections 7 and 9 |
| `list` | `--state all\|gating\|warning\|clean\|pending\|expired\|conflict\|duplicate\|suggested\|stranded`, `--path <path>`, `--ids-only` | one row per claim |
| `audit` | `--doc <path>` (required), `--fail-uncovered` | section 10 |
| `reanchor <claim-id>` | `--doc <path>` with `--doc-quote` or `--doc-range`, `--code-file`, `--code-quote`, `--code-range`, `--ref-file`, `--ref-quote`, `--ref-range`, `--suggest`, `--dry-run` | re-resolve the spans and store the new baseline; `--suggest` is read-only |
| `retire <claim-id>` | `--dry-run` | withdraw; idempotent |
| `supersede` | `--from <path>`, `--to <path>` (both required), `--dry-run` | as v3, matching sentences verbatim |
| `archive` | `--doc <path>` (required), `--successor <path>`, `--dry-run` | as v3; a path that is not a registered document is an error |
| `evidence` | `--from-file <path\|->` (required), `--by <actor>` (required unless every item has `by`) | submit evidence for an `attested` claim |
| `attest <claim-id>` | `--by <actor>` (required), `--note <text>`, `--ack conflict:<claim-id>\|review` | vouch, or acknowledge a finding |
| `schema` | `--name <Name>` | print a JSON Schema, or the list of names |
| `completions <shell>` | none | zsh, bash, or fish |
| `version` | none | print version and schema version |

Every payload is wrapped as `{ ok: true, action, schemaVersion, …payload, next? }`. An operational error exits `1`; in JSON output it prints `{ ok: false, error, errors? }`, where `errors` lists one problem per line for a rejected batch. This holds for every command, including `check`, `list`, and a `--from-file` input that cannot be read or is not valid JSON.

**init.** Creates `.claims/` with `config.json` and the `documents`, `claims`, and `judge` directories, and does not overwrite an existing config. `--instructions <path>` appends this exact line to the file, creating the file if it is missing, and does nothing when the file already contains the line:

```
Before relying on a document, run `hibi check --doc <file>`; exit 2 means a sentence in it is stale.
```

The result reports `instructions: { file, appended }`.

**record.** `by` is required: the item's `by` key, or `--by`, which is the default for items without `by` (section 5). There is no default from git. One item schema serves `--from-file` (a JSON array on stdin or in a file) and the flags, which build one item. With `--from-file`, no other record flag is allowed. Item keys:

```json
{ "doc": "docs/pricing.md",
  "docQuote": "Pro costs $29/month.",
  "docRange": "L12:L12",
  "kind": "code | reference | attested",
  "code": [{ "file": "src/x.ts", "quote": "…", "range": "L1:L3" }],
  "ref": { "file": "docs/other.md", "quote": "…", "range": "L1:L3" },
  "sources": [{ "id": "stripe", "resource": "https://…", "title": "…" }],
  "procedure": { "instructions": "…", "steps": [{ "id": "…", "command": "…", "stdin": "…" }] },
  "ttl": "P90D", "owner": "human:alice@example.com", "by": "agent:opus", "suggest": false }
```

- `doc` and `kind` are required. Exactly one of `docQuote` and `docRange` is required. A `range` is `L<a>:L<b>` (inclusive 1-based lines) or `<a>:<b>` (character offsets). Each span in `code` and `ref` has `file` and exactly one of `quote` and `range`.
- `code` is required, with at least one span, for kind `code`, and invalid for other kinds. `ref` is required for kind `reference` and invalid for other kinds. Neither is allowed for `attested`.
- `ttl` is required for `attested`. `owner` and `by` must be in actor form. An item with no `by` and no `--by` is refused with `by is required: pass --by <actor> or a by key, …`.
- Paths are normalized (section 6), and an evidence file that escapes its root is refused (section 4.1). `suggest: true` records the claim with enforcement `suggested`; otherwise `enforced`.
- Unknown keys are rejected in the item, in spans, in `sources[]`, in `procedure`, and in `steps[]`. `sources[].resource`, `procedure.instructions`, and `steps[].id` and `command` must be non-empty strings.
- Every item is validated before anything is written, and every problem is reported as one line, prefixed `item <i>:` for `--from-file`. The batch is all or nothing: if any item fails when it is applied, everything the batch wrote is rolled back.
- The doc span's text is the claim's sentence. A quote shorter than 8 characters is refused. A quote is located at its first occurrence. When the quote occurs more than once, the stored context must pick one occurrence, or the item is refused; when it does pick one, the result carries a warning.
- One claim exists per normalized sentence per document. Recording a sentence already recorded on the same document updates that claim in place: kind, sentence, anchors, evidence, `ref`, and enforcement are replaced; `sources`, `procedure`, `owner`, and `ttl` are replaced when given; and a vouch is appended. `existingClaims` in the result lists claims on other documents with the same sentence.
- Result: `{ id, kind, doc, evidence (first evidence file or null), enforcement, warnings? }`; a batch returns `{ batch: true, count, results: [...] }`.

**check.** JSON fields: `ref`, `exitCode`, `summary`, `verdicts`, `worklist`, `documents`, plus `since` and `changedFiles` under `--since`, and `doc` (the normalized path) under `--doc`. `--since` scopes verdicts to claims touching a file changed since the git ref; `--doc` scopes them to one document. The contradiction pass judges every pair for a whole-store or `--since` check, and only the pairs with a claim in the document under `--doc` (section 9). `check --doc` on a path with no registered claims exits `1` with `no claims are registered for <path>; run hibi audit --doc <path>`. `check` writes only judge cache entries.

```
summary: { total, gating, warning, clean, retired,
           doc: {unchanged, moved, changed, ambiguous, orphaned},
           evidence: {…the five, unavailable},     claims with no evidence side are not counted
           judge: {supported, refuted, review, pending},   claims with no judge state are not counted
           expired, conflicts, worklist,
           conflictPass: { pairs, requests, pending, scope: "store" | "doc" } }
verdicts[]: { claimId, documentId, kind, doc, evidence?, judge?, expired, conflicts, gates, warns,
              changed?, remediation?, reasons, detail? }
documents[]: { id, path, lifecycle, suspect: [{ claimId, status }] }
```

`changed` is a one-line summary of the first span change. `remediation` is dropped under `--no-hints`. `detail` appears only under `--explain`. `retired` counts retired claims, which are none of gating, warning, or clean. A claim that is not retired is a `suspect` of its document when it gates or warns, or when the document is superseded or archived. Its `status` is its most severe tag, in this order: `evidence:orphaned`, `doc:orphaned`, `evidence:ambiguous`, `doc:ambiguous`, `evidence:changed`, `doc:changed`, `judge:refuted`, `conflict`, `expired`, `evidence:moved`, `doc:moved`, `judge:review`, `judge:pending`, `conflict:review`, `conflict:pending`, `evidence:unavailable`, `superseded`, `archived`. It is never `unchanged`. `--overview` renders the per-document table in human output.

**Remediation commands.** Every `command` in a remediation action runs as written once its placeholders (`<actor>`, `<you>`, `<why>`, `<text>`, `<where>`, `<name>`) are filled in. The acks are `hibi attest <id> --by <actor> --ack review --note "<why>"` and, one action per other claim, `hibi attest <id> --by <actor> --ack conflict:<other-id> --note "<why>"`. The human vouch is `hibi attest <id> --by human:<you> --note "<what you checked>"`. The `evidence` action pipes one item into `hibi evidence --from-file - --by agent:<name>` and is offered only for `attested` claims; a `reference` claim is judged from the referenced document.

**list.** One row per claim: `{ claimId, documentId, kind, sentence, enforcement, owner, ttl, expiresAt, side?, state, duplicates? }`. `state` is the verdict outcome `gating`, `warning`, `clean`, or `retired`. `owner`, `ttl`, and `expiresAt` are null when absent; `expiresAt` is null without a ttl or a counting attestation. `side` (`doc` or `evidence`) appears under `--path`, which keeps claims whose doc or evidence side names the path. `duplicates` (`{ claimId, why }`) appears under `--state duplicate`. The result is `{ state, path?, count, claims }`. Rows sort gating, warning, clean, retired.

| `--state` | rows |
|---|---|
| `all` | every claim |
| `gating`, `warning`, `clean` | claims whose verdict has that state |
| `pending` | the worklist (section 7.5) |
| `expired` | live claims whose ttl ran out |
| `conflict` | live claims with a conflict or review finding |
| `duplicate` | live claims with a duplicate candidate or the same sentence as another live claim |
| `suggested` | claims recorded with `--suggest` |
| `stranded` | live claims on a superseded or archived document |

**evidence.** Input is a JSON array, or one object, of `{ claimId, evidence, source, summary, by }`. An item without `by` takes `--by`; `--by` is required only when some item has no `by`. Evidence applies to `attested` claims only. An item naming a `code` or `reference` claim is a validation error: `claim <id> is a <kind> claim; evidence applies to attested claims, and reference claims are judged from the referenced document`. The whole batch is validated before the judge is asked, and one invalid item rejects the batch and writes nothing: unknown keys are rejected, all five fields must be strings, the claim must exist, be `attested`, and not be retired, `evidence` must be non-empty and at most 32,000 characters, `source` and `summary` must be non-empty, and `by` must be in actor form. For each item the judge answers `supports` (or the cache does), the verdict is stored, the outcome follows section 8.4 including the value rule, and an `evidence` attestation is appended when the outcome is `supported`. A `pending` item writes nothing; its reason is `judge unavailable: <message>` or `judge error: <message>` (section 7.3). Result: `{ results: [{ claimId, judge, relation, confidence, probabilities, reasons, attested, cached }], summary: { supported, refuted, review, pending, requests } }`, with `relation`, `confidence`, and `probabilities` null when pending, and `requests` the judge calls answered in this run.

**attest.** `--by` must be in actor form. Without `--ack` it appends a vouch; `--note` is optional but must be non-empty when given. With `--ack` a note is required. `--ack review` acknowledges the claim's current `supports` verdict: for an `attested` claim the newest `supports` cache entry, for a `reference` claim the entry for the request built from its sentence and referenced span as they read now (section 5). It is an error when there is no such verdict. `--ack conflict:<other-id>` acknowledges a contradiction and writes the ack to both claims; it must name a different claim. A retired claim cannot be attested. Result: `{ claimIds, attestation, countsTowardTtl }`, where `claimIds` has two entries for a conflict ack and `countsTowardTtl` is true only for a `human:` vouch.

**reanchor.** Without span flags each side re-localizes through its stored selectors and the baseline is rewritten. A side that does not locate is refused unless a new span is given for it. `--doc` moves the claim to another document and requires `--doc-quote` or `--doc-range`. `--code-file` with `--code-quote` or `--code-range` replaces a `code` claim's spans; `--ref-file` with `--ref-quote` or `--ref-range` replaces a `reference` claim's span. A flag for the wrong kind is an error: `claim <id> is a <kind> claim; --code-file, --code-quote, and --code-range are for code claims` (with a pointer to the `--ref-*` flags for a `reference` claim), and `…; --ref-file, --ref-quote, and --ref-range are for reference claims`. An evidence side in a sibling repo that is not checked out is kept unchanged, with a warning (section 4.1). The sentence is refreshed from the new doc span.

**archive.** `--doc` must be a registered document (one with a recorded claim); otherwise the command fails with `<path> is not a registered document; nothing to archive`.

Removed from v3:
 `coverage` (folded into `audit`), `check --write`, `--run-verifiers`, `--verifier`, `--verifier-timeout`, `--verified`, `--glob`, the `coarse` selector, `resolvers.json`, `diff`, `status`.

Output rules: human on a TTY, JSON when piped, `--format`, `--json`, `--explain`, `--no-hints`, `--color`, `HIBI_ASCII`. Option tables in `src/cli/options.ts` drive strict parsing, per-command help, the generated reference, and completions.

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
| coarse selector | navigation only, nobody used it |

## 13. Implementation plan

Language: TypeScript on Bun, as the reference implementation. A Rust port is expected later and reimplements against this document, the JSON Schemas in `schemas/`, the judge question files in `src/judge/questions/`, and the fixtures in `test/fixtures/`.

Order, all done on branch v4-redesign:

1. Model and store v4 (`src/core/model.ts`, `schemas/*.v4.json`), judge cache, delete propositions, verifiers, resolvers, banner, upgrade code. Done.
2. Judge interface, fake implementation, question definitions as JSON, Jev HTTP adapter with retries, fixtures for every judgment. Done.
3. Engine: resolution (unchanged cascade), states and gating per section 7, expiry counting rule, worklist. Done.
4. Contradiction pass with code pairing and the numeric rule. Done.
5. `audit` on top of the region splitter. Done.
6. CLI: new and changed verbs, option tables, generated reference. Done.
7. Plugin skill rewritten from this document. `docs/` and README rewritten from scratch. Old ADRs and PRD deleted. Done.
8. Fix pass: judge errors map to `pending`, runnable remediation commands, path normalization and containment, evidence for `attested` claims only, the value rule and pairing bounds, `record --by` required; this document, `docs/`, README, and the plugin skill synced to it. Done.

Everything above is done. One step remains:

9. CI dogfood returns when `.claims/` is re-initialized in this repository. Tune nothing until fixtures exist for the failures observed.

Open items the user must supply: a `TYPESAFE_API_KEY` for live fixtures. The fixtures in `test/fixtures/judge/` are synthetic: the requests are the exact bodies the adapter sends, and the responses are plausible shapes with made-up probabilities. Everything in section 8 beyond request shapes was taken from the docs, not from live runs.

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
| 11 | keep anchor cascade, option tables, lifecycle verbs; tree-sitter optional (see 45) |
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
| 24 | the `coarse` selector and the `--glob` flag are removed: they were navigation only and nobody used them |
| 25 | cache entries store `evidence.values[]` (kind, normalized, unit) so the value rule runs on a cached verdict after the raw evidence text is gone |
| 26 | `does_not_address` is `review`, not `pending`: the judge answered, so the claim warns and is listed, and `pending` means only that the judge was unreachable |
| 27 | audit reports `unsure` rows, with the Choice that fell short, when either the kind or the evidence-kind confidence is below the threshold; `evidenceHint` is dropped |
| 28 | pairing has a second rule: both sentences state a value in the same group (kind plus unit) and share a content token |
| 29 | an `ack` names what it acknowledges: `{ pairId, sentenceHashes }` for a conflict, written to both claims and lapsing when either sentence changes; `{ judgeKey }` for a review |
| 30 | the worklist excludes retired claims, `code` claims that are not expired (amended by 44), claims on superseded or archived documents, and acknowledged reviews;
 each claim is listed once with the first of `expired, review, pending, never-checked` |
| 31 | the fake judge emits System One-shaped answers and the Jev mappers decode them, so there is one decode path for live, cached, and fake answers |
| 32 | the value rule beats the Choice on `supports`, and on `conflict` it replaces the relation outcome, including a duplicate |
| 33 | the cache key is computed over the request as sent, and the stored request redacts submitted evidence to `sha256:<hash>` |
| 34 | an unknown repo alias is `unavailable` at check time and refused at record time; so is a sibling that is not checked out at record time |
| 35 | a contradiction below its bar warns with `possible conflict with <id>` and a pair the judge could not answer warns with `conflict check pending`; neither gates |
| 36 | a review acknowledgement leaves the judge state `review`; it stops the warning and the worklist entry, not the verdict |
| 37 | a judge answer hibi cannot use (`JudgeError`: a 4xx other than 401 or 403, a malformed body, undecodable answers) is `pending` like an unreachable judge; it never exits 1 |
| 38 | `hibi evidence` accepts `attested` claims only; a `reference` claim is judged from the referenced document, and remediation offers the `evidence` action only for `attested` claims |
| 39 | `check --doc` on a path with no registered claims exits 1 and points to `hibi audit --doc` |
| 40 | every user path is normalized to a POSIX path relative to the anchor root, and JSON echoes the normalized form; an evidence file must stay inside the anchor root or its alias directory |
| 41 | `record --by` is required, as a flag or an item key; there is no default from git |
| 42 | on `supports` the value rule excludes the unitless `number` group; on `conflict` every group counts |
| 43 | the rare-token cutoff is `max(3, min(ceil(0.05 n), 10))`; `check --doc` and `audit` judge only the pairs with a claim in the document |
| 44 | `never-checked` lists only `attested` claims with a `procedure` or `sources`; a pure vouch claim is listed only when expired, and `expired` applies to any claim with a ttl |
| 45 | tree-sitter grammars are embedded in the binary; `--no-ast` skips them; other languages get text selectors only, and the numeric-token rule covers code spans without structural checks |

