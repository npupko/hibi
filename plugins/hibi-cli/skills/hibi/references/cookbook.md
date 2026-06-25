# hibi cookbook — worked workflows

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

---

## 5. Consolidate or delete a doc without orphaning its claims

**When** — you're folding `design-v1.md` into `design-v2.md` and deleting the old file
(a rename, split, or merge is the same shape). The claims anchored to the old doc are
content; they must move with it or the deletion strands them.

**Run** — first enumerate what lives on the doomed doc: `hibi query --path design-v1.md`

```json
{
  "ok": true, "action": "query", "schemaVersion": "v1",
  "path": "design-v1.md", "count": 2,
  "hits": [
    { "assertion": { "id": "asrt_a1b2…", "enforcement": "enforced" },
      "proposition": { "textCache": "Retries are capped at 5 attempts." },
      "documentPath": "design-v1.md", "coarse": false, "side": "doc" },
    { "assertion": { "id": "asrt_c3d4…", "enforcement": "enforced" },
      "proposition": { "textCache": "Tokens expire after 30 minutes." },
      "documentPath": "design-v1.md", "coarse": false, "side": "doc" }
  ]
}
```

`side: "doc"` confirms these are claims *on* the doc (not code that the doc describes).
Two ids to resolve — read the count, don't guess it.

**Act** — batch-relocate every live claim onto the new doc in one pass. `relocate`
re-homes each claim whose documented sentence appears **verbatim** in `--to`, and
reports the rest as `misses` (never silently dropped):

```json
$ hibi relocate --from design-v1.md --to design-v2.md
{
  "ok": true, "action": "relocate", "schemaVersion": "v1",
  "from": "design-v1.md", "to": "design-v2.md",
  "relocated": [
    { "claimId": "asrt_a1b2…", "doc": "design-v2.md", "code": "src/retry.ts" }
  ],
  "misses": [
    { "claimId": "asrt_c3d4…", "reason": "sentence not found verbatim in design-v2.md" }
  ],
  "next": "hibi reanchor or retire the 1 missed claim, then hibi supersede"
}
```

The retry sentence carried over verbatim → moved (same id, code side, trust, history;
only `documentId` changed). The 30-minute TTL was reworded or cut in the new design, so
it's a **miss** — `relocate` left it on the old doc rather than guess. **Handle each
miss by hand:** here the TTL was dropped, so retire it (had it merely been reworded, you'd
`hibi reanchor asrt_c3d4… --doc design-v2.md --doc-quote "<new wording>"` instead):

```sh
hibi retire asrt_c3d4…
# the document edge — now reports an empty strandedClaims, so the doc is clear to delete
hibi supersede --new design-v2.md --old design-v1.md --type supersedes
# only now is it safe to delete the old file — nothing is left anchored to it
rm design-v1.md
hibi check        # exit 0 — no orphans
```

`supersede` confirms nothing was left behind — `strandedClaims` is empty because
`relocate` + `retire` cleared the old doc. Had you skipped them, it would list the live
ids and point `next` at `hibi relocate --from design-v1.md --to design-v2.md`:

```json
{ "ok": true, "action": "supersede", "schemaVersion": "v1",
  "edge": { "from": "design-v1.md", "to": "design-v2.md", "type": "supersedes" },
  "strandedClaims": [], "next": "hibi check" }
```

**The trap to avoid:** skipping `relocate`/`retire` and deleting `design-v1.md` anyway.
The claims become `doc:orphaned` — and a non-enforced orphan *looks* harmless (it won't
gate) but it's dead cruft pointing at a deleted file, not an audit trail. Equally wrong:
hand-authoring brand-new claims on `design-v2.md` for propositions that already existed
on `design-v1.md` — that duplicates the record and drops its history. Relocate; don't
re-create.

---

## 6. Store-health triage with `hibi doctor`

**When** — periodically, or before a cleanup pass: you want the dead state `check`
can't show. `check` only grades *live* claims against the *current* tree; `doctor`
sweeps the whole store for accumulated cruft. It is **purely informational and always
exits 0** — safe to run unconditionally.

**Run** — `hibi doctor`

**Setup** — `design-v1.md` was superseded but one live claim was never relocated; a
`suggest`-created claim never got a code pin; and a sentence was recorded twice.

```json
{
  "ok": true, "action": "doctor", "schemaVersion": "v1",
  "healthy": false,
  "counts": { "orphanedAnchors": 1, "suggestedNoCode": 1, "staleDocClaims": 1, "duplicatePropositions": 1 },
  "orphanedAnchors":     [{ "claimId": "asrt_77aa…", "side": "code", "path": "src/old.ts" }],
  "suggestedNoCode":     [{ "claimId": "asrt_db44…", "docPath": "README.md" }],
  "staleDocClaims":      [{ "claimId": "asrt_c3d4…", "docPath": "design-v1.md", "lifecycle": "superseded" }],
  "duplicatePropositions": [
    { "fingerprint": "cfe0afc35172d4af", "propositionIds": ["prop_d618…"], "claimIds": ["asrt_897e…", "asrt_63f3…"] }
  ],
  "next": "hibi list --state orphaned"
}
```

Exit code **0** (always — `doctor` never gates).

**Act** — work the categories, top of `next` first:

- `orphanedAnchors` → the side stopped resolving; relocate it (`reanchor … --code-file
  <new>`) if it moved, or `retire` if the code's gone.
- `suggestedNoCode` → a `suggest` record awaiting a code pin: `reanchor … --code-file …
  --code-quote …` (and `--enforce` to gate), or `retire` if it'll never be backed.
- `staleDocClaims` → live claims on a superseded/retracted/archived doc — exactly what
  the lifecycle verbs leave behind; `hibi relocate --from design-v1.md --to design-v2.md`
  re-homes them.
- `duplicatePropositions` → the same sentence claimed twice; `retire` the redundant id.

`healthy: false` says the store has work; an all-zero `counts` with `healthy: true` is a
clean store.

---

## 7. Preview a bulk change with `--dry-run`

**When** — you're about to reanchor (or relocate/retire/supersede) many claims at once
and want to see the outcome before any write. `--dry-run` runs the full resolution and
returns the *would-be* result, but writes nothing.

**Run** — preview, loop, then apply. Drive the loop off `--ids-only` (bare,
newline-delimited claim ids — no JSON to parse):

```sh
# what's orphaned? — bare ids, ready for a shell loop
$ hibi list --state orphaned --ids-only
asrt_a1b2c3d4e5f60718
asrt_99887766aabbccdd

# preview the reanchor for each before touching the store
$ hibi reanchor asrt_a1b2c3d4e5f60718 --doc design-v2.md \
    --doc-quote "Retries are capped at 5 attempts." --dry-run
{ "ok": true, "action": "reanchor", "schemaVersion": "v1",
  "dryRun": true,
  "claimId": "asrt_a1b2…", "doc": "unchanged", "code": "unchanged",
  "next": "re-run without --dry-run to apply" }
```

`dryRun: true` and the `next: "re-run without --dry-run to apply"` mark it as a preview —
the store is untouched. The verdict (`doc: unchanged`) is the real result you'd get, so
you can confirm the new span resolves cleanly before committing to it.

**Act** — once the previews look right, replay without `--dry-run` (the same `--ids-only`
loop applies the change for real):

```sh
for id in $(hibi list --state orphaned --ids-only); do
  hibi reanchor "$id" --doc design-v2.md --doc-quote "…"   # add the matching span per id
done
hibi check
```

`--dry-run` is available on `reanchor`, `retire`, `supersede`, and `relocate` — the
write verbs — so any bulk migration can be rehearsed before it lands.
