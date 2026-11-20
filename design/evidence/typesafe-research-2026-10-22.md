# TypeSafe / Jev design research for hibi's `Judge` interface

Researched 2026-10-22 from https://docs.typesafe.ai (Markdown pages via `llms.txt`), the OpenAPI spec at https://api.typesafe.ai/openapi.json, the npm registry entry for `@typesafe-ai/sdk`, and the SDK source at https://github.com/typesafe-ai/typesafe-sdk-js (tag v0.7.0). Where a fact comes from an experiment I ran rather than the docs, it is marked **[experiment]**. Where a page did not say something, the note says "docs do not state".

Current model facts at research time (https://docs.typesafe.ai/models.md): Jev 1.14 (`jev-1.14.0`, released 2026-10-14) and Jev 1.13 (`jev-1.13.0`); `jev-latest` -> `jev-1.13.0`; `jev-preview` -> `jev-1.14.0`. Both: $0.042 per Mtok input, output tokens free, 64k per-request / 32k per-question context, text only.

---

## 1. Programming model in 10 lines

1. One endpoint: `POST https://api.typesafe.ai/v1/systemone`, bearer auth, body `{ model, state, questions }` (https://docs.typesafe.ai/api.md).
2. `state` is the content to judge: a string, a JSON object, or an array. The docs say to "use an object for most requests so each part of the state has a descriptive name" (https://docs.typesafe.ai/concepts/state.md).
3. `questions` is a map from IDs you choose to question objects. IDs are not sent to the model; "write the complete question in `instructions`, even when the ID seems self-explanatory" (https://docs.typesafe.ai/primitives.md).
4. Each question has `type` (`choice` | `score` | `noul`), `instructions` (string, object, or array), and `criteria` (Choice: map option -> description, max 255 options; Score: ordered array of 2-10 level descriptions; Noul: optional `{true, false}` descriptions) (https://docs.typesafe.ai/primitives.md, https://docs.typesafe.ai/primitives/advanced.md).
5. Point a question at part of a structured state with a backticked dot/index path, e.g. "Does `ticket.messages[0].text` request a refund?" (https://docs.typesafe.ai/primitives.md#reference-specific-fields).
6. Response: `{ model, answers, usage }`. `answers` is keyed by your IDs. Choice -> `{choice, probabilities, confidence}`; Score -> `{score, legend, probabilities, confidence}`; Noul -> `{noul}` only. `usage` = `{input_tokens, output_tokens}` (https://docs.typesafe.ai/api.md).
7. `response.model` is the versioned ID that answered, which "may differ from the alias supplied in the request" (OpenAPI `SystemOneResponse.model`).
8. Batching: "Each request evaluates one state against one or more questions. All questions see the same state and are evaluated independently" and "One question's answer is not hidden context for another" (https://docs.typesafe.ai/concepts/state.md, https://docs.typesafe.ai/primitives.md).
9. "There is no limit on the number of questions in a request. What limits a request is its size": state + all questions <= per-request context (64k), state + any one question <= per-question context (32k) (https://docs.typesafe.ai/primitives.md#how-many-questions-you-can-ask, https://docs.typesafe.ai/models.md#context-length).
10. The model never generates text; every answer is a distribution over the options you supplied (https://docs.typesafe.ai/concepts/system-one.md). Anything code can compute exactly (arithmetic, dates, counting, string matching) should stay in code (https://docs.typesafe.ai/model-jaggedness/jev-1.13.md).

---

## 2. Primitive choice per judgment, with draft questions

General prompting guidance applied below (https://docs.typesafe.ai/concepts/how-to-build-with-system-one.md, https://docs.typesafe.ai/primitives/noul.md#writing-a-noul-question, https://docs.typesafe.ai/primitives/choice.md#structured-instructions-and-criteria):
- One judgment per question; split compound judgments and combine in code.
- Choice when options are unordered categories; Noul when yes/no and the probability is the signal; Score only for an ordered spectrum.
- Phrase Nouls so a high value means yes; never invert.
- Describe options contrastively ("what", "not_for", "examples"); field names inside structured instructions/criteria are yours, "none are reserved", and the model sees them.
- "A `none of the above` option may not work well in many use cases. As an alternative, ask a companion Noul question in the same request that checks whether anything fits at all" (https://docs.typesafe.ai/primitives/choice.md).
- Put data that varies per question in a named field of a structured `instructions` object instead of splicing it into a string (https://docs.typesafe.ai/primitives/noul.md#structured-instructions).

### 2a. `triage(regions)`

**Primitive:** per region, one Choice for the region's kind, one Choice for the evidence kind (speculative; only read when kind is `factual_claim`), and one Noul per configured topic (multi-label; Choice can only select one). All questions for one document go in one request, with the document regions as the state. This is the Speculative fan-out pattern (https://docs.typesafe.ai/patterns/fan-out.md) applied the way https://docs.typesafe.ai/cookbooks/autoformat.md classifies every block of a document and https://docs.typesafe.ai/cookbooks/semantic_find.md asks about 218 line ids in one request.

**Cookbooks it maps to:** Structure recovery (autoformat; classify every block in one request), Parallel questions (one document, many questions), Hierarchical classification only if the topic list becomes a tree (then one Choice per level, beam over probabilities).

**State shape** (array of regions with stable ids; each question references its region by path):

```ts
const state = {
  doc: { path: "README.md", title: "hibi" },
  regions: [
    { id: "r0", kind: "heading", text: "## Caching", heading_path: ["hibi"] },
    { id: "r1", kind: "sentence", text: "hibi stores its cache in .hibi/cache.json.", heading_path: ["hibi", "Caching"] },
    // ...
  ],
};
```

**Questions, built in code per region index `i`:**

```ts
import { choice, noul } from "@typesafe-ai/sdk";

const kind = (i: number) => choice(
  { question: `What kind of statement is \`regions[${i}].text\`?`,
    focus: "Judge the sentence itself, not the section it is in." },
  {
    factual_claim: { what: "Asserts something about the software, its behavior, files, commands, or environment that is either true or false",
                     not_for: "Advice, instructions, or the author's preferences",
                     examples: ["The CLI exits with code 2 on stale claims.", "Config lives in hibi.toml."] },
    instruction:   { what: "Tells the reader to do something", examples: ["Run `hibi check` before committing."] },
    opinion:       { what: "A preference, recommendation, or value judgment", examples: ["We think this is the cleanest approach."] },
    narrative:     { what: "Framing, history, transitions, or headings with no checkable assertion", examples: ["In this section we cover caching."] },
  },
);

const evidenceKind = (i: number) => choice(
  { question: `If \`regions[${i}].text\` is a factual claim, what kind of evidence would verify it?`,
    focus: "Pick the single best source an agent would consult first." },
  {
    code:        { what: "A span of source code, config, or tests in this repository would confirm or refute it" },
    other_doc:   { what: "Another document in this repository states the same fact" },
    external:    { what: "A resource outside the repository: a dependency's docs, a web page, a registry" },
    nothing:     { what: "No evidence could settle it: a prediction, an intention, or a statement about unobservable state" },
  },
);

// Companion Noul so code does not have to trust a weak Choice winner.
const checkable = (i: number) => noul(
  `Could \`regions[${i}].text\` be shown true or false by looking at evidence?`,
  { true: "A specific, observable assertion", false: "Vague, subjective, or about intent" },
);

// Multi-label topics: one Noul per configured topic, threshold in code, keep up to 3 highest.
const topic = (i: number, t: { id: string; description: string }) => noul(
  { topic: t, question: `Is \`regions[${i}].text\` about the topic described in \`topic\`?` },
);
```

Reading in code: use `kind.choice` only when `kind.confidence` clears a threshold, otherwise treat as `narrative` (not checkable); read `evidenceKind` only when kind is `factual_claim`; topics = Nouls above 0.5 sorted desc, capped at 3. The jaggedness page says the model "leans toward the option that comes first" in some cases, so keep option order fixed and canonical across runs (https://docs.typesafe.ai/model-jaggedness/jev-1.13.md#choice-option-order).

**Sizing caveat:** every question for the document goes against one state, so a 100-region document with (2 Choices + 1 Noul + T topic Nouls) per region is 300 + 100T questions. Each structured question above is roughly 80-150 tokens, so the per-request 64k limit forces chunking at roughly 300-500 questions when the state is small. Code should split regions into batches that reuse the same state, which the docs explicitly allow: "spread them across several requests with the same `state`" (https://docs.typesafe.ai/primitives.md#how-many-questions-you-can-ask). The docs do not give a tokenizer; count by measuring `usage.input_tokens` on a calibration run.

### 2b. `supports(sentence, evidence)`

**Primitive:** one Choice. This is the citation-check cookbook's question (https://docs.typesafe.ai/cookbooks/citation_check.md).

**Correction, 2026-10-23:** the live cookbook's options are `supports` / `contradicts` / `says_nothing`, not `supports` / `refutes` / `does_not_address` as first written here. hibi sends the cookbook's option names (`src/judge/questions/supports.json`) and maps them to `supports` / `refutes` / `does_not_address` in code. The block below keeps the 2026-10-22 sketch with the cookbook's option names:

```ts
const state = {
  claim: sentence.text,
  claim_context: sentence.docContext,          // heading path or surrounding paragraph, kept short
  evidence: evidence.text,
  evidence_source: evidence.source,            // e.g. "src/cli.ts:41-47" or "https://..."
};

const questions = {
  relation: choice(
    { question: "How does `evidence` relate to `claim`?",
      focus: "Judge only what `evidence` says. Do not use outside knowledge." },
    {
      supports:     "The section states the claim or directly implies that it is true",
      contradicts:  "The section states the opposite of the claim or implies it is false",
      says_nothing: "The section does not address what the claim asserts, either way",
    },
  ),
  // Speculative companions, same request, read only when useful:
  addresses: noul("Does `evidence` address what `claim` asserts, either way?"),
  evidence_is_relevant_source: noul(
    "Is `evidence_source` the kind of place where the fact asserted in `claim` would be recorded?"),
};
```

(Replace "The section" with "`evidence`" in production; the cookbook's state keys were `claim` and `section`.)

Code: verdict = `relation.choice`; auto-accept when `relation.confidence >= 0.8` (the cookbook's `AUTO_ACCEPT`, with the advice "Start high, and lower the threshold as you see how the model does on your own documents"); below that, mark `uncertain` and let hibi surface it rather than flag stale. The `addresses` Noul is the docs' recommended companion for the weak "none of the above" case: when `addresses < 0.3`, prefer `does_not_address` even if the Choice leaned elsewhere.

**Cookbooks:** Double-checking citations (direct match); SDE cascade for the alternative decomposition into bad-is-true Nouls (https://docs.typesafe.ai/cookbooks/sde_cascade.md#appendix-a-what-makes-a-good-verifier-signal): "frame each question so the escalate case is the `true` case" and aggregate per-item flags with `max`. For hibi that would be `contradicted = noul("Does `evidence` state something incompatible with `claim`?")` as a second, independent signal; a high value there is the strongest stale signal regardless of the Choice.

What stays in code: exact string/regex matches (the citation cookbook marks a quote that is not in the source as `fabricated` with no model call), version-number and numeric comparisons, date comparisons (https://docs.typesafe.ai/model-jaggedness/jev-1.13.md).

### 2c. `conflict(a, b)`

**Primitive:** one Choice with `consistent` / `contradictory` / `unrelated`, plus a companion Noul `same_subject` as the gate. Choice rather than Score because the three outcomes are not ordered (`unrelated` is not between the other two). Entity alignment used a Score only because its middle level was a real ordered "maybe" (https://docs.typesafe.ai/cookbooks/entity_alignment.md).

```ts
const state = {
  a: { text: a.text, doc: a.docPath, context: a.context },   // context: heading path + at most a few neighbouring sentences
  b: { text: b.text, doc: b.docPath, context: b.context },
};

const questions = {
  relation: choice(
    { question: "How does `a.text` relate to `b.text`?",
      focus: "Use `a.context` and `b.context` only to resolve what each sentence refers to. Judge the two sentences, not the documents." },
    {
      consistent:    { what: "Both can be true at the same time; they agree or describe compatible facts about the same thing" },
      contradictory: { what: "They cannot both be true: they assert incompatible facts about the same thing",
                       examples: ["A: cache is in .hibi/cache.json. B: cache is in ~/.cache/hibi."] },
      unrelated:     { what: "They are about different things, so neither bears on the other" },
    },
  ),
  same_subject: noul("Are `a.text` and `b.text` about the same thing (the same file, command, option, behavior, or value)?"),
  contradicts:  noul("Does `b.text` assert something that cannot be true if `a.text` is true?",
                     { true: "Incompatible values, locations, behaviors, or conditions", false: "Compatible, or about different things" }),
};
```

Code: if `same_subject < 0.3` -> `unrelated` regardless of the Choice (the companion-Noul gate from the Choice page). Else verdict = `relation.choice`, with `contradicts` as an independent cross-check; disagreement between `relation.choice === "contradictory"` and `contradicts < 0.5` is a review case. Because option order can bias the answer (jaggedness page), either keep order fixed and canonical, or run a swapped-order copy of the Choice in the same request as a second speculative question and require agreement.

**Cookbooks:** Double-checking citations (same three-way relation shape); Knowledge graph entity alignment (pairwise state `{entity_a, entity_b}` with companion Nouls that say which field disagrees; one request per pair; ~8 concurrent workers); Re-ranking (one Noul per pair at scale: 1,200 pairs, 1.54M input tokens, $0.0645 on jev-1.12). See section 8 for why all-pairs is the wrong decomposition.

---

## 3. Confidence and thresholds

Source: https://docs.typesafe.ai/confidence.md unless noted.

- **Noul:** returns only `noul` = P(yes). "There is no separate `confidence` value for a Noul". A value near 0.5 is the model saying it is unsure. For a confidence-style number: `confidence = |2p - 1|`.
- **Choice:** `confidence = (p_max - 1/n) / (1 - 1/n)`. With 3 options this is `(3 * p_max - 1) / 2`, so confidence 0.8 means `p_max` ~ 0.867 and confidence 0.5 means `p_max` = 0.667. "Only the top probability counts, so (0.6, 0.3, 0.1) and (0.6, 0.2, 0.2) both have confidence 0.4." The docs also suggest two alternatives computed from the same `probabilities`: top probability (threshold per question, because its meaning depends on n) and top-to-second ratio ("Many real decisions come down to the top two candidates").
- **Score:** confidence = `max(0, 1 - weighted_distance_from_mode / MAD_uniform)`; adjacent-level splits cost less than far splits. Not needed for hibi's three judgments.
- **What confidence is not:** "confidence 1.0 means the returned distribution puts all its probability on one level. This describes the model's answer, not a guarantee that the answer is correct" (https://docs.typesafe.ai/primitives/score.md). "Higher confidence does not establish which answer is correct" (same page, on examples in criteria).
- **Calibration:** trained with RLCD so that "Outcomes assigned a probability of 0.8 should occur about 80% of the time"; "These rates describe groups of predictions, not a guarantee about any single answer" (https://docs.typesafe.ai/introduction/machine-learning-primer.md). "Calibration is measured across groups of predictions; it does not guarantee that an individual answer is correct" (https://docs.typesafe.ai/concepts/system-one.md). The score page adds that Jev 1.13 "score levels are weak in numerical calibration" (jaggedness page), which does not affect Choice/Noul use.
- **Picking thresholds:** "The correct threshold values depend on your domain and the performance of the model for your use case. Start with conservative thresholds, test with your own data, and adjust as you observe results." "Test thresholds by plotting confidence against accuracy on your data" (how-to-build). Three-way split: act / review / do not act; thresholds "scale with risk". Numbers used in the docs: Choice confidence 0.8 to auto-accept a citation verdict; Noul bands 0.2/0.8 (noul page) or 0.3/0.7 (consistency cookbook: "The band is illustrative; it is neither a calibrated guarantee nor an optimized threshold").
- **For hibi specifically:** a stale verdict is a false-positive-expensive action (it blocks or nags), so gate `refutes`/`contradictory` on a high threshold (Choice confidence >= 0.8, or `p_refutes >= 0.85`) and let everything in between be `uncertain`. The agent-skill page also warns against over-using confidence: "If all you care about is choosing the best option, you just need to choose the option with the highest confidence" (https://docs.typesafe.ai/agent-skill.md).

---

## 4. Determinism

- **No guarantee in the docs.** No page states that identical input yields identical output. The strongest statements are "System One is designed to return stable answers across repeated evaluations" (https://docs.typesafe.ai/concepts/how-to-build-with-system-one.md) and the Parallel questions cookbook: "Most answers came back identical across all 5 repeats either way, the same value on every call, std dev exactly 0.0" (https://docs.typesafe.ai/cookbooks/parallel_questions.md) -- "most", not all.
- **Measured run-to-run noise:** the Noul self-consistency cookbook (jev-1.13.0, 15 repeats of a 14-Noul request, 2026-09-11) reports "mean per-question probability standard deviation is 0.0102"; one question's values "span 0.43 to 0.53, crossing a 0.5 decision threshold"; the other 13 "stay on one side of that threshold throughout this run". It also says the setup "cannot separate sensitivity to the irrelevant field [a per-run uid in the state] from variation that would occur on identical requests", and "The model is no more deterministic for it" when using a review band (https://docs.typesafe.ai/cookbooks/consistency_noul_cookbook.md). The Choice variant reports mean std dev 0.0515 for TypeSafe vs 0.0012 for Haiku at temperature 0 on its moderation task (https://docs.typesafe.ai/cookbooks/consistency_choice_cookbook.md).
- **Temperature / seed:** no such request field exists. The OpenAPI `SystemOneRequest` has only `model`, `state`, `questions`, `assets`. The consistency cookbooks say "reasoning models and TypeSafe run without a temperature setting". Docs do not state how sampling works server-side.
- **Batching does not change answers:** questions are independent, so adding or removing questions from a request should not change other answers (parallel questions cookbook measured this). This means hibi can cache per question, not per request, and safely regroup questions across runs.
- **Option order can change answers** (jaggedness page). The cache key must therefore cover the exact serialized question, including option order and the field names inside structured instructions.
- **Model versioning:** aliases move; "An alias moves when a new release ships, so its answers can change without any change to your code. Each response's `model` field gives the versioned ID that answered ... If you have tuned confidence thresholds against one version, send that version's ID instead of the alias, and switch to the new version when you are ready" (https://docs.typesafe.ai/models.md#aliases). Versioned IDs "are accepted by the `model` field whether or not they appear in the list" from `GET /v1/models`. So: pin `jev-1.13.0` (or `jev-1.14.0`) in hibi's config, store `response.model` with each cached verdict, and invalidate or re-verify when the pinned version changes. Docs do not state a deprecation schedule for old versioned IDs.
- **Design consequence:** treat Jev as a nearly-stable but not bit-exact function. Cache by `hash(model_id, canonical_request_json)`; store the full `probabilities`, not just the label; make the stale/fresh decision in code from cached probabilities with a review band, so a second run cannot flip a verdict without a cache miss.

---

## 5. Limits that matter for 500 claims or 125k pairs

Hard limits (https://docs.typesafe.ai/models.md, https://docs.typesafe.ai/api/errors.md, OpenAPI):
- Context: 64k tokens per request (state + all questions), 32k per question (state + one question). `400` with `max_tokens_exceeded` when over. Long-context header `TypeSafe-Danger-Zone-Long-Context: true` raises both to 262k but "is slower and less accurate".
- Body <= 32 MiB; UTF-8 JSON, no BOM; <= 64 nesting levels; finite numbers only ("Send identifiers and exact large quantities as strings"). gzip or zstd body compression accepted.
- Choice: 1-255 options. Score: 1-10 levels in the schema, "should have at least two" per docs (the JS SDK rejects fewer than two). No limit on question count.
- Rate limits: per organization, "measured in tokens per second and requests per second", exact numbers only in the console (https://console.typesafe.ai/rate-limits); docs do not state default values. `429` and `529 Overloaded` -> exponential backoff; SDKs retry by default and honor `retry-after`. The entity-alignment cookbook's code comment: "small pool; the public endpoint rate-limits above roughly eight" concurrent workers; the re-rank cookbook used 12 workers.
- Latency: "Most queries complete in about 100 ms" (how-to-build); 111 ms mean round trip for a 14-Noul request (consistency cookbook); 0.27 s for 13 questions over a ~54k-character article vs 2.71 s summed for 13 single calls (parallel questions cookbook). Docs do not publish p95 figures.
- Pricing: $0.042 per million input tokens, output free, same price for long context. Per-request token accounting: the state is counted once per request, each question adds its own tokens. The re-rank cookbook measured ~1,280 input tokens per pair request (1,536,002 / 1,200) for legal passages; a short claim + short evidence request will be far smaller.
- Data handling: "Jev is not trained on customer requests or responses"; ZDR is "for enterprise customers"; retention details are only in the DPA, not in the docs (https://docs.typesafe.ai/models.md#data-handling, https://docs.typesafe.ai/legal.md).

Back-of-envelope (docs give the unit figures; the arithmetic is mine):
- **Triage, 500 claims:** if each region costs ~4 questions at ~120 tokens plus a shared state, a document of 50 regions is ~24k question tokens plus state per request, under 64k. 500 regions total -> roughly 10-20 requests, ~250k-400k tokens, about $0.01-0.02, a few seconds wall clock.
- **supports, 500 claims:** 500 requests (one per claim+evidence), ~300-1,500 tokens each -> 0.15M-0.75M tokens, $0.006-0.03. At ~0.1 s per request and 8 in flight, ~10 s.
- **conflict, 125k pairs one-request-per-pair:** ~125k x ~400 tokens = ~50M tokens = ~$2.10; wall clock at 8 concurrent and ~0.11 s each is ~30 minutes, and it is the requests-per-second limit, not cost, that dominates. Fan-out alternative (one claim as state, one structured Choice per candidate in `instructions`, as the noul page's `same_as_record_<id>` pattern does): 500 requests of up to ~499 questions each, each question carrying the candidate text (~100-150 tokens) -> ~50-75k tokens per request, which exceeds the 64k per-request limit for the longest claim lists and so must be split into ~2 requests per claim. Total tokens are similar to the pairwise approach (~30-40M), requests drop from 125k to ~1,000, wall clock drops to minutes. Either way, the docs' guidance is to shrink the candidate set in code first (section 8).

---

## 6. JavaScript SDK

Sources: https://docs.typesafe.ai/sdk/javascript.md, https://docs.typesafe.ai/sdk/javascript/changelog.md, https://docs.typesafe.ai/sdk/javascript/api/ pages, npm registry, SDK source v0.7.0.

- **Package:** `@typesafe-ai/sdk`. **Current version:** 0.7.0 (published 2026-10-14; `VERSION` export is `"0.7.0"`). Earlier: 0.6.0 (2026-09-15, Score criteria became an ordered list), 0.5.7 (2026-09-11, first public release). MIT. ESM + CJS + `.d.ts`. No runtime dependencies. `engines: { node: ">=22" }`; the docs say "Install the SDK (Node.js 22 or newer)".
- **Env vars** (`ENV` export): `TYPESAFE_API_KEY` (required unless `apiKey` passed), `TYPESAFE_BASE_URL` (default `https://api.typesafe.ai`), `TYPESAFE_DEFAULT_MODEL` (default `jev-latest`), `TYPESAFE_LOG_LEVEL` (default `warn`). "Empty or whitespace-only environment values are ignored."
- **Minimal call:**

```ts
import { TypeSafeClient, choice, noul } from "@typesafe-ai/sdk";
const client = new TypeSafeClient({ defaultModel: "jev-1.13.0" }); // reads TYPESAFE_API_KEY
const { answers, model, usage } = await client.systemOne({
  state: { claim: "...", evidence: "..." },
  questions: {
    relation: choice("How does `evidence` relate to `claim`?", { supports: "...", contradicts: "...", says_nothing: "..." }),
    addresses: noul("Does `evidence` address what `claim` asserts?"),
  },
});
answers.relation.choice; answers.relation.confidence; answers.relation.probabilities; answers.addresses.noul;
```

Answer types are inferred from the question map. `systemOne` returns an `APIPromise`; `model` and `headers` can be overridden per call (`RequestOptions`: `headers`, `retry`, `signal`, `timeout`). `client.models.list()` returns `ModelCard[]` (`name`, `description`, `release_date`).
- **Client config** (`TypeSafeClientConfig`): `apiKey`, `baseURL`, `defaultModel`, `defaultHeaders`, `timeout` (per attempt, ms, default 10000, "without a total retry budget"), `retry` (Partial<RetryPolicy>), `compression` (`"zstd"` on Node >= 22.15 else `false`; bodies under 1 KiB never compressed), `fetch` (custom implementation; default global `fetch`), `logger`, `logLevel`, `dangerouslyAllowBrowser`.
- **Retry defaults** (`RetryPolicy`): `maxRetries` 2; `backoffInitialMs` 500 doubled to `backoffMaxMs` 5000; `backoffJitter` 0.25; `httpStatuses` 408, 429, 500-599 (so 529 Overloaded is covered); `respectRetryAfter` true up to `maxRetryAfterMs` 60000; `apiConnectionError` true; `apiTimeoutError` true. Each retry sends `X-TypeSafe-Retry-Count`. The SDK also sends `User-Agent`/`X-TypeSafe-SDK: typesafe-sdk/0.7.0` and `X-TypeSafe-Runtime` (e.g. `bun/1.4.3 (darwin; arm64)`).
- **Error types:** base `TypeSafeError` (also thrown synchronously-as-rejection for invalid requests: null state, empty questions, Score with < 2 levels, empty Choice map, Noul with neither instructions nor criteria). `APIError` (`status`, `body`, `headers`, `requestId` from `x-typesafe-request-id`) with subclasses `BadRequestError` (400), `AuthenticationError` (401), `PermissionDeniedError`, `NotFoundError`, `UnprocessableEntityError` (422), `RateLimitError` (429, adds `retryAfterMs`), `InternalServerError`. `APIConnectionError` (DNS/TLS/closed), `APITimeoutError` (subclass, `timeoutMs`), `APIUserAbortError`. Docs do not list a dedicated class for 413/415/529; they fall under `APIError.fromResponse`. **[experiment]** a 401 from the live API under Node 26 produced `AuthenticationError` with body `{"detail":{"error_type":"authentication_error","message":"Cannot authenticate with the server. Please check your API key and try again."}}`.
- **Bun:** docs do not state Bun support; the SDK's `runtime.ts` does detect Bun for the `X-TypeSafe-Runtime` header. **[experiment, Bun 1.4.3, SDK 0.7.0, macOS arm64]:** the client constructs (and selects `compression: "zstd"`), but every request rejects with a plain `TypeError: Body already used` thrown from the SDK's `parseBody` (`res.text()`), not a `TypeSafeError`, with `compression: "zstd"` and with `compression: false` alike. Cause by code reading: `bufferResponse` acquires readers on `response.clone().body` and on `response.body`, releases the locks, and `parseBody` then calls `res.text()`; Bun treats the original body as consumed. The same `parseBody` call sits on the success path (dist `index.mjs` line 985 in 0.7.0), so a 2xx under Bun is expected to fail the same way (not observed: no API key was available to this session). The identical script under Node 26.12 behaves correctly. **Workaround validated on the 401 path under Bun:** pass a custom `fetch` that reads the body once and returns a Response-like object with `body: null`, `clone()` returning itself, and `text()/json()` serving the buffered string; the SDK then surfaced a proper `AuthenticationError`. Alternatives for hibi: run the SDK that way, call the HTTP API directly with Bun's `fetch` (the request/response shape is small and fully specified in the OpenAPI document; hibi would own retries on 408/429/5xx/529 and `retry-after`), or file the bug upstream at https://github.com/typesafe-ai/typesafe-sdk-js. Probe scripts: `/private/tmp/claude-501/-Users-random-Projects-npupko-hibi/7833bd27-2f21-4210-9ba6-538c45d68f80/scratchpad/buntest/probe2.ts` and `probe4.ts`.

---

## 7. Best practices and pitfalls from the docs that apply here

- "Ask for a judgment a knowledgeable person makes in a second given the right context." Narrow, literal, atomic questions; combine in code (https://docs.typesafe.ai/primitives.md).
- "`jev-1.13` answers the question you wrote, not the one you meant" -- state the exact condition; "When you look at a wrong answer and find yourself explaining what you really meant, that explanation is the missing half of the instruction" (jaggedness page).
- Keep math, counting, dates, string matching, and version comparisons in code. "Jev is not a calculator." Dates: "reads dates as text, not as ordered quantities." Numeric representations underperform semantic ones. Many doc claims are numeric or version-shaped; hibi should extract and compare those in code and only ask Jev whether the evidence talks about the same quantity.
- Context rot: "Accuracy falls as the state grows with content unrelated to the decision ... retrieve and filter in code first, and send only the fields the question needs." For `supports`, send the evidence span, not the file; for `conflict`, send a short context, not whole documents.
- Avoid indirection and double negatives; name the state parts by path.
- Adversarial content: "State is data, and `jev-1.13` does not treat it as hostile by default." Documentation and code comments are untrusted text; a sentence that argues for its own truth can move the answer. Be explicit in criteria and keep the final decision rule in code.
- Align `criteria` with `instructions`; a Noul whose `true` means "no" performs worse.
- Choice option order can bias; "reorder the options to double check that the answer stays consistent."
- Companion Noul instead of (or in addition to) a `none of the above` option.
- Structured criteria: same field names across options/levels "so the model can compare like with like"; examples only help "when they look like your real inputs"; "Choose examples with known expected levels, then test the revised descriptions on separate inputs before keeping them."
- Question IDs are not seen by the model; keep complete questions in `instructions`. Also "Additional properties on a request variable are forwarded, including `null` values" (SDK `SystemOneRequest`), so do not spread extra fields into the request object.
- Put the questions and threshold constants in one reviewable file: "The most important thing for humans to review is the questions and any threshold constants" (https://docs.typesafe.ai/agent-skill.md). Agents "aren't great at writing questions, so expect to edit collaboratively".
- English is the primary training language; other languages "have lower accuracy" (models page).
- Images are not relevant (text only without the danger-zone header).

---

## 8. What argues against the design assumptions

1. **All-pairs `conflict(a, b)` over 125k pairs is the wrong first decomposition.** Every scaling cookbook puts a cheap code filter before the model: BM25 shortlists of 30 before re-ranking ("Fast search reliably narrows the corpus down to a shortlist"), a "cheap but rough first pass" to 450 candidate pairs before entity alignment, exact string matching before the citation check. The how-to-build page says "Use code when you can" and the jaggedness page says to filter in code first. For hibi: pair only claims that share a topic (from triage), share an entity/path/identifier token, or are lexically similar; expect the candidate set to shrink by one to two orders of magnitude. Then either one request per surviving pair (entity-alignment shape) or one request per claim with one structured Choice per candidate in `instructions` (the noul page's `same_as_record_<id>` pattern), staying under 64k per request.
2. **Contradiction is two judgments, not one.** "same subject?" and "incompatible?" are separate properties; the docs say to ask them separately and combine in code. The Choice alone mixes them; the companion Nouls in 2c are not optional extras.
3. **Pairwise conflict is a code-owned graph problem once the model has spoken.** The model returns per-pair probabilities; transitivity, clustering, and "which of the two is stale" are code decisions. Jev cannot say which side is wrong; `supports()` against evidence does that.
4. **Triage topics as "1-3 from a list" cannot be a single Choice.** Choice selects one option. Use one Noul per topic (or read the Choice `probabilities` and take the top few above a floor, which the docs do with `probabilities` for secondary teams: "A second team with a real share of the probability gets a copy"). Noul-per-topic is cleaner and keeps each topic's threshold independent.
5. **Evidence-kind classification is partly computable.** If the region contains a code span, path, or command, the evidence kind is `code` by construction; ask Jev only when code cannot tell. The docs repeatedly say not to ask the model what code can compute.
6. **Deterministic caching cannot rely on identical re-answers.** No determinism guarantee exists; one of 14 Nouls crossed 0.5 across 15 repeats in the docs' own test. hibi's cache must be the source of truth for a verdict until its key (model version + canonical request) changes, and the fresh/stale decision must be made from stored probabilities with a review band, not by re-asking and expecting equality.
7. **`jev-latest` is not stable.** Pin a versioned ID in hibi's config; aliases "can change without any change to your code".
8. **Noul has no `confidence`.** If hibi's `Judge` interface returns a uniform `{verdict, confidence}`, compute `|2p - 1|` for Nouls in the adapter; the docs give this formula to put Nouls on the Choice scale.
9. **The SDK does not currently work under Bun** (section 6). The `Judge` adapter should either wrap `fetch` as described or call the HTTP API directly.
10. **Score is not needed for any of the three judgments.** None of them is an ordered spectrum with describable middle levels; the docs steer ordered-outcome designs toward Score only when the middle is a real outcome (entity alignment's "curator queue"). Hibi's "uncertain" is a confidence band in code, not a level.
11. **Numeric and date-shaped claims should bypass or pre-process before Jev.** A large share of doc staleness is version numbers, exit codes, flag names, and paths; the jaggedness page says Jev is unreliable on numeric precision and date ordering. Extract both sides in code, compare in code, and ask Jev only about the semantic part ("is this the same setting?").
