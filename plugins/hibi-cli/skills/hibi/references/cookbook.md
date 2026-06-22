# hibi cookbook — four worked workflows

One section per *moment* you reach for hibi: when to use it, the command, the real
output, and how to act. JSON is shown `--json --pretty` for readability; piped
(non-TTY) output is the same shape, compact. Concise is the default an agent reads;
`--explain` adds the evidence tail. `Setup` lines state the precondition that
produced the output — without them a `changed`/`gates` verdict looks unmotivated.

Shared fixture: `src/retry.ts` (`MAX_ATTEMPTS = 5`), `src/auth.ts`
(`TOKEN_TTL_MIN = 30`), a `README.md`, and a `CLAUDE.md` asserting "Auth tokens
expire after 30 minutes." Claims recorded enforced.

---

## 1. Trust-check before following agent instructions

**When** — about to act on `CLAUDE.md` / `AGENTS.md` / a README; confirm the code
still backs it first. (SessionStart-hook moment.)

**Run** — `hibi status --doc CLAUDE.md`

**Setup** — `TOKEN_TTL_MIN` in `src/auth.ts` changed 30 → 60 since the claim was recorded.

```json
{
  "ok": true,
  "action": "status",
  "schemaVersion": "v1",
  "doc": "CLAUDE.md",
  "found": true,
  "lifecycle": "active",
  "current": false,
  "suspect": [{ "propositionId": "prop_d618…", "status": "code:changed" }],
  "verdicts": [
    {
      "assertionId": "asrt_897e054b48d040db",
      "doc": "unchanged",
      "code": "changed",
      "expired": false,
      "gates": true,
      "remediation": {
        "recommended": null,
        "actions": [
          { "id": "retire",   "applicability": "manual",       "effect": "deterministic", "command": "hibi retire asrt_897e054b48d040db" },
          { "id": "fix-code", "applicability": "manual",       "effect": "prose" },
          { "id": "reanchor", "applicability": "needs-review", "effect": "deterministic", "command": "hibi reanchor asrt_897e054b48d040db" }
        ]
      },
      "notes": ["structural-only AST match (rename/whitespace)"]
    }
  ]
}
```

Exit code **2**.

**Act** — `current: false` + `gates: true` is the decision: the instruction drifted.
`doc: unchanged` + `code: changed` means the sentence is intact but the code moved
(now 60, the doc still says 30) — don't follow the 30-minute rule. `recommended:
null` because hibi can't know intent (deliberate change → fix the doc; accidental →
fix the code); decide, then run a menu `command`.

---

## 2. Keep docs honest after a code change

**When** — you just edited code; surface which doc sentences it invalidated so the
prose fix lands in the same PR. (Stop-hook moment.)

**Run** — `hibi diff --since origin/main` (here `--since HEAD`, after editing `src/auth.ts`)

```json
{
  "ok": true,
  "action": "diff",
  "schemaVersion": "v1",
  "ref": "c18e2e1…",
  "since": "HEAD",
  "changedFiles": ["src/auth.ts"],
  "exitCode": 2,
  "summary": { "total": 2, "gating": 2, "warning": 0, "clean": 0, "expired": 0 },
  "verdicts": [
    {
      "assertionId": "asrt_63f3d4e945eb4d79",
      "doc": "unchanged",
      "code": "changed",
      "gates": true,
      "remediation": {
        "recommended": null,
        "actions": [
          { "id": "retire",   "command": "hibi retire asrt_63f3d4e945eb4d79" },
          { "id": "fix-code" },
          { "id": "reanchor", "command": "hibi reanchor asrt_63f3d4e945eb4d79" }
        ]
      }
    }
  ],
  "documents": [
    { "path": "CLAUDE.md", "suspect": [{ "status": "code:changed" }] },
    { "path": "README.md", "suspect": [{ "status": "code:changed" }] }
  ]
}
```

**Act** — `diff` only evaluated claims touching `changedFiles`, so the unchanged
`retry` claim is absent — just the two auth claims the edit broke. `documents[].path`
names the files needing a prose fix.

**Need the evidence?** `--explain` adds `evidence` to each verdict:

```json
"evidence": {
  "docRegion": { "start": 62, "end": 97 },
  "codeRegions": [{ "start": 29, "end": 31 }],
  "confidence": 0.446,
  "selectorScores": [
    { "kind": "text-quote", "found": true,  "score": 0.5, "weight": 0.3 },
    { "kind": "value",      "found": false, "score": 0,   "weight": 0.2 }
  ],
  "changedEvidence": [
    { "path": "src/auth.ts", "kind": "value", "detail": "anchored value changed (was `30`)" }
  ]
},
"fingerprint": "cfe0afc35172d4af"
```

`changedEvidence` names *what* moved (the literal `30`) — fix the sentence without
re-reading the diff.

---

## 3. What does this code promise, before I touch it

**When** — about to refactor `src/auth.ts`; list the doc claims anchored to it (the
contracts you must not silently break).

**Run** — `hibi query --path src/auth.ts`

```json
{
  "ok": true,
  "action": "query",
  "schemaVersion": "v1",
  "path": "src/auth.ts",
  "count": 2,
  "hits": [
    { "assertion": { "id": "asrt_63f3…", "enforcement": "enforced" },
      "proposition": { "textCache": "Auth tokens expire after 30 minutes" },
      "documentPath": "README.md", "coarse": false, "side": "code" },
    { "assertion": { "id": "asrt_897e…", "enforcement": "enforced" },
      "proposition": { "textCache": "Auth tokens expire after 30 minutes" },
      "documentPath": "CLAUDE.md", "coarse": false, "side": "code" }
  ]
}
```

**Act** — two enforced claims ride on this file (`README.md`, `CLAUDE.md`), both
asserting the 30-minute TTL; changing that value reds both. `query` hands you the
`assertion.id`s up front, so after the edit you already know which to `reanchor`
(still true) or `retire` (now wrong).

---

## 4. Onboard an existing repo fast

**When** — a repo has docs but no claims; get to protected without hand-authoring
every anchor.

**Run** — `hibi init` then `hibi suggest --doc README.md`

`init` returns the store handle and the next step:

```json
{ "ok": true, "action": "init", "schemaVersion": "v1",
  "store": "…/.claims", "nonce": "34144670", "version": "v1",
  "next": "hibi suggest --doc <file>" }
```

`suggest` writes one `suggested` (advisory, never-gating) doc-side record per
anchorable sentence — doc side filled, code side empty, awaiting a pin:

```json
{
  "ok": true, "action": "suggest", "schemaVersion": "v1",
  "doc": "README.md", "count": 2,
  "created": [
    { "proposition": { "textCache": "Requests are retried up to 5 times before failing." },
      "assertion":   { "id": "asrt_db44…", "enforcement": "suggested",
                       "anchor": { "doc": { "file": "README.md" }, "code": [] } } },
    { "proposition": { "textCache": "Auth tokens expire after 30 minutes." },
      "assertion":   { "id": "asrt_71cc…", "enforcement": "suggested",
                       "anchor": { "doc": { "file": "README.md" }, "code": [] } } }
  ],
  "next": "hibi check"
}
```

**Act** — for each suggestion worth enforcing, pin its code side and promote it, then
`hibi check`:

```sh
hibi reanchor asrt_db44… --code-file src/retry.ts --code-quote "5"
# or re-record precisely with --trust verified --enforce
```

---

## Triage: what needs attention right now

**Run** — `hibi list --state gating` (the fast "what's red?" view — one lean row per
claim, no full report to parse):

```json
{
  "ok": true, "action": "list", "schemaVersion": "v1",
  "state": "gating", "count": 2,
  "claims": [
    { "claimId": "asrt_63f3…", "documentPath": "README.md", "codePath": "src/auth.ts",
      "status": "code:changed", "severity": "gating", "gates": true, "recommended": null },
    { "claimId": "asrt_897e…", "documentPath": "CLAUDE.md", "codePath": "src/auth.ts",
      "status": "code:changed", "severity": "gating", "gates": true, "recommended": null }
  ]
}
```

**Act** — each row carries the `claimId` the next command needs. Decide intent, then
act — e.g. retire the obsolete CLAUDE.md claim:

```json
$ hibi retire asrt_897e054b48d040db
{ "ok": true, "action": "retire", "schemaVersion": "v1",
  "assertion": { "id": "asrt_897e…", "enforcement": "retired", … },
  "alreadyRetired": false, "claimId": "asrt_897e…", "next": "hibi check" }
```

A retired claim no longer gates; `retire` is idempotent (`alreadyRetired: true` on a
second call).
