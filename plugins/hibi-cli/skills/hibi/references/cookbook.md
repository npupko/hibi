# hibi cookbook — four worked workflows

Each section is a *moment* you reach for hibi, the one command you run, the real
output it returns, and how to act on it. All JSON below was captured by running
the CLI; it is `--json --pretty` for readability — piped (non-TTY) output is the
same shape, compact. The concise default is what an agent reads on the hot path;
`--explain` adds the evidence tail.

The scenario repo: a service with `src/retry.ts` (`MAX_ATTEMPTS = 5`) and
`src/auth.ts` (`TOKEN_TTL_MIN = 30`), a `README.md`, and a `CLAUDE.md` that tells
the agent "Auth tokens expire after 30 minutes." Claims are recorded enforced.

---

## 1. Trust-check before following agent instructions

**The moment.** You're about to act on `CLAUDE.md` / `AGENTS.md` / a README. Before
you trust what it says about the code, confirm the code still backs it. (This is the
SessionStart-hook moment — see SKILL.md.)

**The command.**

```sh
hibi status --doc CLAUDE.md
```

**What you get** (after the token TTL changed from 30 → 60 in `src/auth.ts`):

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

Exit code is **2**.

**How to read it.** `current: false` and `gates: true` is the decision: the
instruction drifted. The doc text is intact (`doc: unchanged`) but the code it
points at changed (`code: changed`) — the file now says 60 minutes, the instruction
still says 30. **Do not blindly follow the 30-minute rule.** `recommended` is
`null` because hibi can't know intent: maybe the TTL change was deliberate (fix the
doc), maybe accidental (fix the code). You decide, then run one of the menu's
`command`s.

---

## 2. Keep docs honest after a code change

**The moment.** You just edited code. Surface exactly which doc sentences your change
invalidated, so the prose fix lands in the same PR. (This is the Stop-hook moment.)

**The command.**

```sh
hibi diff --since origin/main        # here: --since HEAD after editing src/auth.ts
```

**What you get** (concise — one verdict shown; the change touched two auth claims):

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

`diff` only evaluated claims touching `changedFiles` (the write-time loop), so the
unchanged `retry` claim isn't in the report — just the two auth claims your edit
broke. The `documents` array names exactly which files need a prose fix.

**Need the evidence?** Add `--explain`. The verdict then carries `evidence`:

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

`changedEvidence` tells you *what* moved (the literal `30`), so you can fix the
sentence without re-reading the diff.

---

## 3. What does this code promise, before I touch it

**The moment.** You're about to refactor `src/auth.ts`. Before you change it, list every
doc claim anchored to it — the contracts you must not silently break.

**The command.**

```sh
hibi query --path src/auth.ts
```

**What you get** (trimmed to the decision fields):

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

**How to read it.** Two enforced claims ride on this file — one in `README.md`, one in
`CLAUDE.md`, both asserting the 30-minute TTL. If your refactor changes that value,
both go red. `query` hands you the `assertion.id`s up front, so after the edit you
already know which claims to `reanchor` (still true) or `retire` (now wrong) — no
round-trip to discover them.

---

## 4. Onboard an existing repo fast

**The moment.** A repo has docs but no claims. Get from zero to protected without
hand-authoring every anchor.

**The commands.**

```sh
hibi init
hibi suggest --doc README.md
```

**What you get.** `init` returns the store handle and the next step:

```json
{ "ok": true, "action": "init", "schemaVersion": "v1",
  "store": "…/.claims", "nonce": "34144670", "version": "v1",
  "next": "hibi suggest --doc <file>" }
```

`suggest` proposes one `suggested` (advisory, never-gating) doc-side record per
anchorable sentence — the doc side is filled in, the code side is empty, waiting for
you to pin it:

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

**How to act.** For each suggestion worth enforcing, pin its code side and promote it:

```sh
hibi reanchor asrt_db44… --code-file src/retry.ts --code-quote "5"
# or re-record it precisely with --trust verified --enforce
```

Then `hibi check` to confirm everything is clean. You've gone from unprotected docs
to a tracked, gating store without hand-writing a single anchor from scratch.

---

## Triage: what needs attention right now

Across all four workflows, `hibi list` is the fast "what's red?" view — one lean row
per claim, no full report to parse:

```sh
hibi list --state gating
```

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

Each row carries the `claimId` the next command needs. Decide intent, then act —
e.g. retire the obsolete CLAUDE.md claim:

```json
$ hibi retire asrt_897e054b48d040db
{ "ok": true, "action": "retire", "schemaVersion": "v1",
  "assertion": { "id": "asrt_897e…", "enforcement": "retired", … },
  "alreadyRetired": false, "claimId": "asrt_897e…", "next": "hibi check" }
```

A retired claim no longer gates; `retire` is idempotent (`alreadyRetired: true` on a
second call).
