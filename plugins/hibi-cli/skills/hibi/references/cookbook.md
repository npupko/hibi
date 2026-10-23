# hibi cookbook

Eight recipes. Each gives the situation, the commands, the JSON they return, and what to do next. The JSON has the shape real runs return, trimmed to the keys the recipe uses; the judge's kinds and probabilities are examples. Ids are examples. Commands assume `hibi` is on PATH; otherwise use `bunx @npupko/hibi`.

Shared setup: a knowledge repo `handbook/` with `docs/pricing.md`, `docs/billing-policy.md`, and `AGENTS.md`, and a sibling code repo `../api` with `src/limits.ts` (`export const RATE_LIMIT_PER_MIN = 100;`).

---

## 1. Initialize a knowledge repo

**When**: a repo of docs (handbook, product, process, decisions) has no `.claims/` yet.

```sh
hibi init --instructions AGENTS.md
```

```json
{ "ok": true, "action": "init", "schemaVersion": "v4", "store": "/…/handbook/.claims", "version": "v4",
  "judge": { "model": "jev-1.14.0", "threshold": 0.8 },
  "instructions": { "file": "AGENTS.md", "appended": true },
  "next": "hibi audit --doc <file>" }
```

`--instructions` appends this line to the file (created if missing, skipped if already there):

```
Before relying on a document, run `hibi check --doc <file>`; exit 2 means a sentence in it is stale.
```

Then edit `.claims/config.json` when evidence lives in another repo or some documents should not be audited:

```json
{
  "version": "v4",
  "repos": { "api": "../api" },
  "judge": { "model": "jev-1.14.0", "threshold": 0.8 },
  "ignore": ["docs/generated/**"]
}
```

`judge.threshold` is the one confidence bar for every judgment. Export `TYPESAFE_API_KEY` before the next steps; without it every judgment is `pending`. Commit `.claims/`. `.claims/judge/` is a local cache that `.claims/.gitignore` excludes; CI restores it with an `actions/cache` step.

---

## 2. Audit a marketing page and record its claims

**When**: a page states prices, limits, or facts that drift without anyone noticing.

```sh
hibi audit --doc docs/pricing.md
```

```json
{
  "ok": true, "action": "audit", "schemaVersion": "v4", "doc": "docs/pricing.md",
  "summary": { "regions": 6, "covered": 0, "uncovered": 6,
    "byKind": { "code": 0, "reference": 1, "attested": 2, "unverifiable": 1,
                "not-a-claim": 2, "unsure": 0, "pending": 0 },
    "requests": 1 },
  "rows": [
    { "region": { "id": "r0", "line": 1, "kind": "heading", "text": "# Pricing" },
      "claimId": null, "kind": "not-a-claim", "probability": 0.94, "duplicateOf": null },
    { "region": { "id": "r1", "line": 3, "kind": "sentence", "text": "The Pro plan costs $29 per month." },
      "claimId": null, "kind": "attested", "probability": 0.94, "duplicateOf": null },
    { "region": { "id": "r2", "line": 5, "kind": "sentence", "text": "Acme is the most loved API platform." },
      "claimId": null, "kind": "unverifiable", "probability": 0.94, "duplicateOf": null },
    { "region": { "id": "r3", "line": 7, "kind": "sentence", "text": "Every plan allows 100 API requests per minute." },
      "claimId": null, "kind": "attested", "probability": 0.94, "duplicateOf": null },
    { "region": { "id": "r4", "line": 9, "kind": "sentence", "text": "Annual billing details are in the billing policy." },
      "claimId": null, "kind": "reference", "probability": 0.94, "duplicateOf": null },
    { "region": { "id": "r5", "line": 11, "kind": "sentence", "text": "Contact sales for a custom quote." },
      "claimId": null, "kind": "not-a-claim", "probability": 0.94, "duplicateOf": null }
  ]
}
```

A row with `kind: "unsure"` carries `unsure` (`kind` or `evidenceKind`) and `probabilities` instead of `probability`; decide it yourself.

**Act**, in this order:

1. Rewrite or remove the `unverifiable` rows first, so later edits do not move recorded spans. Line 5 becomes an attributed statement: "In our 2026 survey, most customers said they would recommend Acme." Never record an `unverifiable` region.
2. Decide each recordable row's kind and anchors yourself. The triage said `attested` for line 7, but the value lives in `../api`, so it is a `code` claim with an aliased file.
3. Record in one batch:

```sh
hibi record --from-file - <<'EOF'
[
  {"doc":"docs/pricing.md","docQuote":"The Pro plan costs $29 per month.","kind":"attested",
   "ttl":"P90D","owner":"human:alice@example.com","by":"agent:claude",
   "sources":[{"id":"stripe","resource":"https://dashboard.stripe.com/products/prod_123","title":"Stripe: Pro product"}],
   "procedure":{"instructions":"Open the Stripe Pro product and report its monthly price in USD.",
     "steps":[{"id":"price","command":"stripe prices list --product prod_123 --json"}]}},
  {"doc":"docs/pricing.md","docQuote":"Every plan allows 100 API requests per minute.","kind":"code",
   "code":[{"file":"api:src/limits.ts","quote":"RATE_LIMIT_PER_MIN = 100"}],"by":"agent:claude"},
  {"doc":"docs/pricing.md","docQuote":"Annual billing details are in the billing policy.","kind":"reference",
   "ref":{"file":"docs/billing-policy.md","quote":"Annual plans are billed once a year, up front."},"by":"agent:claude"}
]
EOF
```

```json
{ "ok": true, "action": "record", "schemaVersion": "v4", "batch": true, "count": 3, "updated": 0,
  "results": [
    { "id": "clm_517bbfaa1ac94263", "kind": "attested", "doc": "docs/pricing.md", "evidence": null, "enforcement": "enforced", "updated": false, "existingClaims": [] },
    { "id": "clm_d55da8f0827540d4", "kind": "code", "doc": "docs/pricing.md", "evidence": "api:src/limits.ts", "enforcement": "enforced", "updated": false, "existingClaims": [] },
    { "id": "clm_f4cdf35aa5df43d4", "kind": "reference", "doc": "docs/pricing.md", "evidence": "docs/billing-policy.md", "enforcement": "enforced", "updated": false, "existingClaims": [] }
  ],
  "next": "hibi check" }
```

A batch with any invalid item records nothing and lists every problem, one line each. After the batch is recorded, `hibi audit --doc docs/pricing.md` shows the three rows with their `claimId`.

If you edit the document after recording and a claim reports `doc: "moved"`, run `hibi reanchor <id>` to store the new position, or `hibi reanchor --moved --doc docs/pricing.md` for every moved claim of the document.

---

## 3. Run the worklist from a scheduled job

**When**: attested claims need re-checking on a schedule, without a person starting it.

Right after recipe 2, `hibi check --json` lists the attested claim as `expired`, because it was recorded by an agent and an agent vouch does not count. Run without `TYPESAFE_API_KEY`, the worklist also holds the reference claim, as `pending`:

```json
"worklist": [
  { "claimId": "clm_517bbfaa1ac94263", "sentence": "The Pro plan costs $29 per month.", "kind": "attested",
    "document": "docs/pricing.md",
    "procedure": { "instructions": "Open the Stripe Pro product and report its monthly price in USD.",
                   "steps": [{ "id": "price", "command": "stripe prices list --product prod_123 --json" }] },
    "sources": [{ "id": "stripe", "resource": "https://dashboard.stripe.com/products/prod_123", "title": "Stripe: Pro product" }],
    "owner": "human:alice@example.com", "reason": "expired" },
  { "claimId": "clm_f4cdf35aa5df43d4", "sentence": "Annual billing details are in the billing policy.", "kind": "reference",
    "document": "docs/pricing.md", "procedure": null, "sources": [], "owner": null, "reason": "pending" }
]
```

`reason` is one of `expired`, `never-checked`, `review`, `pending`:

| reason | which claims |
|---|---|
| `expired` | any claim with a `ttl` that ran out or that no attestation counts toward, `code` claims included |
| `review` | a `reference` or `attested` claim whose judge state is `review` and not acknowledged |
| `pending` | a `reference` or `attested` claim the judge could not answer: no key, no network, an unusable answer (`judge error: …` in the verdict's `reasons`), or a referenced span over 32,000 characters |
| `never-checked` | an `attested` claim with a `procedure` or `sources` and no judged evidence yet; a claim with neither is listed only when expired or in `review` |

A `pending` reference claim needs only a run with a working judge; there is nothing to collect. `lastEvidence: { summary, source, at, by }` appears once evidence for the claim has been judged.

A Lobster workflow (`hibi-worklist.lobster`) that a scheduler runs nightly. hibi runs none of these steps; the workflow does. `claude -p` is one possible agent; any agent that reads the worklist on stdin and prints an evidence array works.

```yaml
name: hibi-worklist
steps:
  - id: worklist
    command: hibi check --json --fail-on never | jq '[.worklist[] | select(.kind == "attested")]'
  - id: collect
    command: >-
      claude -p --allowedTools "Bash(stripe prices list:*)"
      "The JSON on stdin is a hibi worklist. For each item, follow procedure.instructions
      (steps are hints). Print only a JSON array of
      {claimId, evidence, source, summary}: evidence is the observed text that bears on the
      sentence, at most 32000 characters, with no secrets or tokens; source is the sources[].id
      you used; summary is one line. Skip items you could not check."
    stdin: $worklist.stdout
  - id: submit
    command: >-
      input=$(cat); [ "$input" = "[]" ] ||
      printf '%s' "$input" | hibi evidence --from-file - --by agent:nightly
    stdin: $collect.stdout
  - id: expired
    command: hibi list --state expired --json
```

`--fail-on never` keeps `check` at exit 0 so the workflow continues. The `jq` filter keeps `attested` items only, because `hibi evidence` accepts attested claims only and rejects a batch naming a `code` or `reference` claim, which can also carry a procedure. `submit` skips an empty array because `hibi evidence` rejects an empty batch with exit 1. `--by agent:nightly` applies to every item without a `by` key. Run it with `lobster run hibi-worklist.lobster`.

The `submit` step returns:

```json
{ "ok": true, "action": "evidence", "schemaVersion": "v4",
  "results": [
    { "claimId": "clm_517bbfaa1ac94263", "judge": "supported", "relation": "supports",
      "confidence": 0.9249999999999999, "probabilities": { "supports": 0.95, "refutes": 0.03, "does_not_address": 0.02 },
      "reasons": ["evidence supports the sentence (confidence 0.92 >= threshold 0.8)",
                  "source matches declared source \"stripe\""],
      "attested": true, "cached": false }
  ],
  "summary": { "supported": 1, "refuted": 0, "review": 0, "pending": 0, "requests": 1 },
  "next": "hibi check" }
```

| `judge` | effect |
|---|---|
| `supported` | an `evidence` attestation is appended; ttl restarts; the claim leaves the worklist |
| `refuted` | nothing is attested; the next `check` gates the claim (fix the sentence or the source of truth). Evidence holding a different money amount, unit-bearing number, version, or date than the sentence is `refuted` in code with `value differs: …`, unless the judge answers `does_not_address`; bare unitless numbers in the evidence are not compared |
| `review` | below threshold or `does_not_address`; collect evidence that addresses the sentence directly, or `hibi attest <id> --by <actor> --ack review --note "<why>"` |
| `pending` | no key, no network, or a judge error (`judge error: …` in `reasons`); nothing written; resubmit |

`evidence` exits 0 for every verdict. Commit `.claims/` after the run: the claim file gained an attestation. `.claims/judge/` gained a cache entry, which stays local. The raw evidence text is not stored, only its sha256, the source, the summary, and the numbers, money amounts, versions, and dates in it.

The `expired` step of the workflow lists claims still expired after the `submit` step, for recipe 5.

---

## 4. Handle a contradiction

**When**: `check` exits 2 with `conflicts[]` on a verdict. Setup: `docs/faq.md` says "The Pro plan costs $39 per month." and is recorded as an attested claim.

```sh
hibi check --json
```

```json
{
  "exitCode": 2,
  "summary": { "gating": 2, "conflicts": 2, "conflictPass": { "pairs": 1, "requests": 1, "pending": 0, "scope": "store" } },
  "verdicts": [
    { "claimId": "clm_cd53af142f2f4ad3", "kind": "attested", "doc": "unchanged", "expired": true,
      "conflicts": [{ "claimId": "clm_517bbfaa1ac94263", "probability": 1, "reason": "numbers differ: 29 vs 39" }],
      "gates": true, "warns": false,
      "remediation": { "recommended": "reconcile", "actions": [
        { "id": "reconcile", "title": "Reconcile with clm_517bbfaa1ac94263",
          "rationale": "this sentence contradicts clm_517bbfaa1ac94263; rewrite whichever is wrong" },
        { "id": "ack", "title": "Acknowledge the conflict with clm_517bbfaa1ac94263", "rationale": "…",
          "command": "hibi attest clm_cd53af142f2f4ad3 --by <actor> --ack conflict:clm_517bbfaa1ac94263 --note \"<why>\"" },
        { "id": "attest", "title": "Have a human re-check and vouch", "rationale": "…", "command": "…" },
        { "id": "evidence", "title": "Collect evidence and submit it", "rationale": "…", "command": "…" } ] },
      "reasons": [
        "expired: ttl P90D is set and no attestation counts (only evidence or a human vouch resets ttl)",
        "conflicts with clm_517bbfaa1ac94263 (p=1.00 at or above threshold 0.8): numbers differ: 29 vs 39",
        "expired warns in a whole-store check and gates under check --doc",
        "worklist: expired; re-check the claim and submit evidence, or have a human attest" ] }
  ]
}
```

The other claim carries the mirror finding. A claim that conflicts with several others gets one `ack` action per other claim. `hibi list --state conflict` returns both. A pair the judge could not answer is not a finding: each claim warns with one reason, `conflict checks pending: <n>`, counting its pending pairs, and `summary.conflictPass.pending` counts the pairs. Once the judge says both sentences share a subject and rates `contradicts` at or above 0.5, differing numbers, versions, or dates are decided in code (`numbers differ`, probability 1); other contradictions come from the judge with their probabilities in `reason`.

**Act**: find out which sentence is wrong. In this walkthrough, the price claim has judged Stripe evidence for $29 (recipe 3), so the FAQ is wrong. Pick one:

- Fix the wrong sentence, then `hibi reanchor clm_cd53af142f2f4ad3 --doc-quote "The Pro plan costs $29 per month."` to accept the new text. Plain `hibi reanchor <id>` relocates only a side that reads `moved`; an edited sentence reads `changed` and needs `--doc-quote`.
- Remove the duplicate sentence (here the FAQ now says "Current prices are on the pricing page."), then `hibi retire clm_cd53af142f2f4ad3`. The claim's doc anchor is `orphaned` until you retire it.
- If both sentences are true (for example, they describe different plans), acknowledge it: `hibi attest clm_cd53af142f2f4ad3 --by human:alice@example.com --ack conflict:clm_517bbfaa1ac94263 --note "the FAQ describes the annual plan"`. The ack is written to both claims and lapses when either sentence changes.

After the FAQ sentence is removed and its claim retired, `check` returns `exitCode: 0`, `summary.retired: 1`, `summary.conflicts: 0` for this walkthrough. Do not reword either sentence just to make the numbers match without checking the source.

---

## 5. Handle an expired attested claim

**When**: an attested claim's ttl ran out. Setup: `docs/process/releases.md` says "Releases ship every Tuesday.", recorded with `ttl`, `owner: "human:ops@example.com"`, and a procedure. `check --doc` gates on it:

```sh
hibi check --doc docs/process/releases.md --json
```

```json
{
  "exitCode": 2,
  "verdicts": [
    { "claimId": "clm_67d8afc4c50b4abb", "expired": true, "gates": true,
      "reasons": [
        "expired: ttl P30D ran out at 2026-11-21T18:21:46.275Z, counted from the vouch by human:alice@example.com at 2026-10-22T18:21:46.275Z",
        "expired gates under check --doc",
        "worklist: expired; re-check the claim and submit evidence, or have a human attest" ],
      "remediation": { "recommended": null, "actions": [
        { "id": "attest", "title": "Have a human re-check and vouch", "rationale": "…",
          "command": "hibi attest clm_67d8afc4c50b4abb --by human:<you> --note \"<what you checked>\"" },
        { "id": "evidence", "title": "Collect evidence and submit it", "rationale": "…", "command": "…" } ] } }
  ],
  "worklist": [
    { "claimId": "clm_67d8afc4c50b4abb", "sentence": "Releases ship every Tuesday.", "kind": "attested",
      "document": "docs/process/releases.md",
      "procedure": { "instructions": "Read the release calendar and report the weekdays of the last three releases." },
      "sources": [], "owner": "human:ops@example.com", "reason": "expired" }
  ]
}
```

In a whole-store `check`, expired only warns; under `check --doc` it gates.

**Act**:

1. If you can follow the procedure with your tools, submit evidence (recipe 3). Judged support resets the ttl.
2. If you cannot, contact the `owner`. hibi sends nothing; use the channel the team uses, for example an issue built from `hibi list --state expired --json` (rows carry `claimId`, `sentence`, `owner`, `ttl`, `expiresAt`).
3. The owner checks and vouches:

```sh
hibi attest clm_67d8afc4c50b4abb --by human:ops@example.com --note "calendar: last three releases were on Tuesdays"
```

```json
{ "ok": true, "action": "attest", "schemaVersion": "v4",
  "claimIds": ["clm_67d8afc4c50b4abb"],
  "attestation": { "by": "human:ops@example.com", "at": "2026-11-24T09:12:00.000Z", "basis": "vouch",
                   "note": "calendar: last three releases were on Tuesdays" },
  "countsTowardTtl": true, "next": "hibi check" }
```

After the human vouch, `check --doc docs/process/releases.md` exits 0 for this walkthrough, and the claim's `list` row shows the new `expiresAt`. An agent never runs this with a `human:` actor. `--by agent:<name>` is recorded with `countsTowardTtl: false` and does not clear the expiry. After a human vouch, a claim with a procedure stays in the worklist as `never-checked` until evidence for it is judged; that is a reminder, not a warning. A claim with no procedure and no sources would leave the worklist.

If the sentence is no longer true, rewrite it and run `hibi reanchor <id> --doc-quote "<new sentence>"`, or remove it and `hibi retire <id>`.

---

## 6. Cross-repo evidence with `repos` aliases

**When**: a sentence in the knowledge repo describes code in another repository.

With `"repos": { "api": "../api" }` in `.claims/config.json`, a code or ref `file` may be `api:src/limits.ts` (recipe 2). Record refuses an alias that is not in `repos` (below), and an alias whose directory is missing ("sibling repo \"api\" is not checked out at ../api; …"), because a claim is recorded against text on disk. Exit 1:

```json
{ "ok": false,
  "error": "record: 1 problem; nothing was recorded.\n  item 0: unknown repo alias \"web\" in web:src/x.ts; add it to \"repos\" in .claims/config.json (known: api)",
  "errors": ["item 0: unknown repo alias \"web\" in web:src/x.ts; add it to \"repos\" in .claims/config.json (known: api)"] }
```

At check time:

| sibling repo | verdict | effect |
|---|---|---|
| checked out, span intact | `evidence: "unchanged"` | clean |
| not checked out | `evidence: "unavailable"`, reason `evidence: sibling repo "api" is not checked out at ../api` | warns, never gates |
| value changed (`100` to `120`) | `evidence: "changed"`, `changed: "api:src/limits.ts value: value changed (was \`100\`)"` | gates; `recommended: "update-claim"` |

An evidence file must stay inside its root: a plain path inside the knowledge repo, an aliased path inside the alias directory. Record refuses `../api/src/limits.ts` (`path escapes its root; an evidence file must be inside the anchor root.`) and `api:../handbook/AGENTS.md`; use `api:src/limits.ts`. If a stored side escapes its root, `check` reports it `unavailable` with `evidence: path escapes its root`.

`hibi reanchor <id>` while the sibling is not checked out keeps that evidence side as stored and says so in `warnings[]` (`evidence side api:src/limits.ts kept unchanged: sibling repo "api" is not checked out at ../api`); the doc side is still re-resolved. A remediation command that names `--code-file api:src/limits.ts` keeps that span as stored and lists it in `kept` (`evidence: repository api not checked out; kept as stored`); it exits 0 when another side was relocated.

`hibi list --path api:src/limits.ts` finds the claims anchored there. `check --since <ref>` diffs only the knowledge repo, so it does not select claims whose evidence changed in a sibling repo; run a full `hibi check` when the sibling moved. In CI, check out the sibling at the aliased path (recipe 7), or its claims stay `unavailable`.

---

## 7. CI with the GitHub Action

**When**: pull requests should fail when they make a recorded sentence stale.

Copy `assets/hibi-ci.yml` to `.github/workflows/hibi.yml`. The action's inputs:

| input | default | effect |
|---|---|---|
| `fail-on` | `gating` | `gating`, `warn`, or `never` |
| `since` | empty | `check --since <ref>` |
| `doc` | empty | `check --doc <path>`; expired claims then gate |
| `working-directory` | `.` | the directory holding `.claims/` |

The job exits 0 when every claim is clean or only warns, 2 when a claim gates (any warning also fails under `fail-on: warn`; neither fails under `fail-on: never`), and 1 on an operational error. A `doc` input naming a document with no registered claims is such an error, so the job fails until the document is audited and recorded.

Pass `TYPESAFE_API_KEY` as a repository secret in the step's `env`. Without it, judgments not already in the `.claims/judge/` cache are `pending`, which warns and does not fail the build under `fail-on: gating`. The `actions/cache` step in `assets/hibi-ci.yml` restores `.claims/judge/` before the check, and saves it at the end of the job only when the job succeeds and no cache exists yet for the current claim files.

For a knowledge repo with `repos` aliases such as `"api": "../api"`, check the repo out with `path: handbook`, each sibling next to it with `path: api`, and set `working-directory: handbook`. The asset shows the commented steps.

---

## 8. `check --doc` as a pre-trust gate

**When**: before an agent follows `CLAUDE.md`, `AGENTS.md`, or any document. `hibi init --instructions CLAUDE.md` adds the instruction line (recipe 1). Setup: `CLAUDE.md` says "Auth tokens expire after 30 minutes.", recorded against `src/auth.ts` (`TOKEN_TTL_MIN = 30`); the code then changed to 60.

```sh
hibi check --doc CLAUDE.md --json
```

```json
{
  "ok": true, "action": "check", "schemaVersion": "v4", "doc": "CLAUDE.md", "exitCode": 2,
  "verdicts": [
    { "claimId": "clm_6b0ef79f25f54415", "kind": "code", "doc": "unchanged", "evidence": "changed",
      "expired": false, "conflicts": [], "gates": true, "warns": false,
      "changed": "src/auth.ts value: value changed (was `30`)",
      "remediation": { "recommended": "update-claim", "actions": [
        { "id": "update-claim", "title": "Update the sentence, then reanchor",
          "rationale": "the evidence changed; if the sentence is now wrong, rewrite it, then run the command",
          "command": "hibi reanchor clm_6b0ef79f25f54415 --code-file src/auth.ts --code-quote \"TOKEN_TTL_MIN = 60\"" },
        { "id": "reanchor", "title": "Reanchor as is", "rationale": "…", "command": "hibi reanchor clm_6b0ef79f25f54415 --code-file src/auth.ts --code-quote \"TOKEN_TTL_MIN = 60\"" },
        { "id": "retire", "title": "Retire the claim", "rationale": "…", "command": "hibi retire clm_6b0ef79f25f54415" } ] },
      "reasons": ["evidence: value changed (was `30`)", "evidence: identifiers or literals renamed"] }
  ],
  "documents": [{ "id": "doc_5c9e4f99d90e2cfd", "path": "CLAUDE.md", "lifecycle": "active",
                  "suspect": [{ "claimId": "clm_6b0ef79f25f54415", "status": "evidence:changed" }] }]
}
```

| result | meaning |
|---|---|
| exit 0 | every claim in the document is clean or only warns |
| exit 2 | at least one claim gates: changed, orphaned, ambiguous, refuted, contradicted, or expired |
| exit 1, `no claims are registered for CLAUDE.md; run hibi audit --doc CLAUDE.md` | the document has no claims; hibi says nothing about its sentences; audit it before relying on it |

The path is normalized relative to the current directory, so `--doc ./CLAUDE.md` checks the same document and the JSON echoes `"doc": "CLAUDE.md"`. Under `--doc`, the contradiction pass judges only pairs with a claim in this document (`summary.conflictPass.scope: "doc"`); a finding may still name a claim in another document.

**Act**: do not follow the 30-minute rule. Read `src/auth.ts`, rewrite the sentence to say 60 minutes, then `hibi reanchor clm_6b0ef79f25f54415 --doc-quote "Auth tokens expire after 60 minutes." --code-file src/auth.ts --code-quote "TOKEN_TTL_MIN = 60"`. Both sides read `changed`, so plain `hibi reanchor <id>` would leave them as stored and exit 1. The result's `after` shows the new doc quote and code quote, and `check --doc CLAUDE.md` exits 0. Never rewrite the sentence without reading the code, and never delete the claim file; `hibi retire` withdraws a claim that no longer applies.
