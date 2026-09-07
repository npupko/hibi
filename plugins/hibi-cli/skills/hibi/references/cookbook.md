# hibi cookbook

Six worked examples. Each states when to run the command, the command, the JSON it returns (shown as `--format json-pretty`; piped output is the same shape, compact), and what to do with the result.

Shared fixture: `src/retry.ts` (`MAX_ATTEMPTS = 5`), `src/auth.ts` (`TOKEN_TTL_MIN = 30`), a `README.md`, and a `CLAUDE.md` that says "Auth tokens expire after 30 minutes." All claims are enforced.

---

## 1. Trust-check an instruction file

**When**: before acting on `CLAUDE.md`, `AGENTS.md`, or a README. Setup: `TOKEN_TTL_MIN` changed from 30 to 60 after the claim was recorded.

**Run**: `hibi check --doc CLAUDE.md`

```json
{
  "ok": true,
  "action": "check",
  "schemaVersion": "v3",
  "ref": "c18e2e1",
  "doc": "CLAUDE.md",
  "found": true,
  "exitCode": 2,
  "summary": {
    "total": 1, "gating": 1, "warning": 0, "clean": 0, "retired": 0,
    "doc": { "unchanged": 1, "moved": 0, "changed": 0, "ambiguous": 0, "orphaned": 0 },
    "code": { "unchanged": 0, "moved": 0, "changed": 1, "ambiguous": 0, "orphaned": 0 },
    "behavior": { "supported": 0, "refuted": 0 },
    "expired": 0
  },
  "verdicts": [
    {
      "assertionId": "asrt_897e054b48d040db",
      "propositionId": "prop_d6185fc85e2a4e52",
      "documentId": "doc_5c9e4f99d90e2cfd",
      "doc": "unchanged",
      "code": "changed",
      "expired": false,
      "gates": true,
      "changed": "src/auth.ts value: value changed (was `30`)",
      "remediation": {
        "recommended": "update-claim",
        "actions": [
          { "id": "update-claim", "title": "Update the sentence, then reanchor",
            "rationale": "the code changed; if the sentence is now wrong, rewrite it, then run the command",
            "command": "hibi reanchor asrt_897e054b48d040db" },
          { "id": "reanchor", "title": "Reanchor as is",
            "rationale": "the sentence is still true; accept the new span",
            "command": "hibi reanchor asrt_897e054b48d040db" },
          { "id": "retire", "title": "Retire the claim",
            "rationale": "the claim is obsolete; withdraw it so it no longer gates",
            "command": "hibi retire asrt_897e054b48d040db" }
        ]
      },
      "notes": ["code: value changed (was `30`)"]
    }
  ],
  "documents": [
    { "id": "doc_5c9e4f99d90e2cfd", "path": "CLAUDE.md", "lifecycle": "active",
      "suspect": [{ "propositionId": "prop_d6185fc85e2a4e52", "status": "code:changed" }] }
  ]
}
```

Exit code 2.

**Act**: `doc: unchanged` and `code: changed` means the sentence is intact but the code moved. Do not follow the 30-minute rule. The recommended action is `update-claim`: read `src/auth.ts`, rewrite the sentence to say 60 minutes, then run `hibi reanchor asrt_897e054b48d040db`.

---

## 2. Check what a code change invalidated

**When**: after editing code, before the change is considered done. Setup: `src/auth.ts` was edited on this branch.

**Run**: `hibi check --since origin/main`

```json
{
  "ok": true,
  "action": "check",
  "schemaVersion": "v3",
  "ref": "c18e2e1",
  "since": "origin/main",
  "changedFiles": ["src/auth.ts"],
  "exitCode": 2,
  "summary": { "total": 2, "gating": 2, "warning": 0, "clean": 0, "retired": 0, "expired": 0 },
  "verdicts": [
    { "assertionId": "asrt_63f3d4e945eb4d79", "doc": "unchanged", "code": "changed", "gates": true,
      "remediation": { "recommended": "update-claim", "actions": [ "…" ] } },
    { "assertionId": "asrt_897e054b48d040db", "doc": "unchanged", "code": "changed", "gates": true,
      "remediation": { "recommended": "update-claim", "actions": [ "…" ] } }
  ],
  "documents": [
    { "path": "README.md", "lifecycle": "active", "suspect": [{ "propositionId": "prop_d618…", "status": "code:changed" }] },
    { "path": "CLAUDE.md", "lifecycle": "active", "suspect": [{ "propositionId": "prop_d618…", "status": "code:changed" }] }
  ]
}
```

**Act**: only claims touching `changedFiles` were evaluated. `documents[].path` names the two files that need a prose fix. Fix both sentences in the same PR, then run the `command` from each verdict. Add `--explain` to see `evidence.changedEvidence` when the one-line `changed` field is not enough. An unknown `--since` ref exits 1.

---

## 3. List the claims on a file before a refactor

**When**: about to change `src/auth.ts`.

**Run**: `hibi list --path src/auth.ts`

```json
{
  "ok": true,
  "action": "list",
  "schemaVersion": "v3",
  "state": "all",
  "path": "src/auth.ts",
  "count": 2,
  "claims": [
    { "claimId": "asrt_63f3d4e945eb4d79", "propositionId": "prop_d6185fc85e2a4e52",
      "text": "Auth tokens expire after 30 minutes", "documentPath": "README.md",
      "codePath": "src/auth.ts", "status": "unchanged", "severity": "clean", "gates": false,
      "enforcement": "enforced", "recommended": null, "side": "code" },
    { "claimId": "asrt_897e054b48d040db", "propositionId": "prop_d6185fc85e2a4e52",
      "text": "Auth tokens expire after 30 minutes", "documentPath": "CLAUDE.md",
      "codePath": "src/auth.ts", "status": "unchanged", "severity": "clean", "gates": false,
      "enforcement": "enforced", "recommended": null, "side": "code" }
  ]
}
```

**Act**: two enforced sentences depend on this file. Keep the ids. After the edit, `reanchor` the ones still true and `retire` the ones now wrong. `--path` also accepts a doc path; `side: "doc"` then marks the claims that live on it.

---

## 4. Grounding audit: coverage, then record

**When**: a doc exists but has no claims.

**Run**: `hibi init`, then `hibi coverage --doc README.md`

```json
{
  "ok": true,
  "action": "coverage",
  "schemaVersion": "v3",
  "doc": "README.md",
  "summary": { "regions": 3, "covered": 0, "uncovered": 3, "coverageRatio": 0 },
  "regions": [
    { "range": { "start": 0, "end": 7 }, "preview": "# Title", "covered": false, "claimIds": [] },
    { "range": { "start": 9, "end": 56 }, "preview": "Requests are retried up to 5 times before failing.", "covered": false, "claimIds": [] },
    { "range": { "start": 58, "end": 94 }, "preview": "We think retries improve reliability.", "covered": false, "claimIds": [] }
  ],
  "next": "ground or remove the uncovered regions: `hibi record --from-file -`"
}
```

**Act**: regions are sentences. For each uncovered region decide: record a claim (a code span backs it) or remove the sentence (nothing backs it). The third region is opinion, not a claim; cut it. Record the grounded set in one batch on stdin:

```sh
echo '[
  { "doc": "README.md",
    "docQuote": "Requests are retried up to 5 times before failing.",
    "codeFile": "src/retry.ts", "codeQuote": "MAX_ATTEMPTS = 5", "verified": true }
]' | hibi record --from-file -
```

```json
{
  "ok": true,
  "action": "record",
  "schemaVersion": "v3",
  "batch": true,
  "count": 1,
  "results": [
    { "id": "asrt_a1b2c3d4e5f60718", "doc": "README.md", "code": "src/retry.ts", "enforcement": "enforced", "verified": true }
  ],
  "next": "hibi check"
}
```

The batch is all or nothing. A missing `codeFile`, a quote that does not occur in its file, or a doc quote shorter than 8 characters fails the whole batch with exit 1. An ambiguous doc quote records with a warning in `warnings`. Run `hibi coverage --doc README.md` again to confirm the ratio rose, then `hibi check`. A missing doc path is exit 1. `--fail-uncovered` exits 2 while any region is uncovered, which turns "this plan is fully grounded" into a CI gate.

---

## 5. Triage, then retire

**When**: anytime you want the gating set without a full report.

**Run**: `hibi list --state gating`

```json
{
  "ok": true,
  "action": "list",
  "schemaVersion": "v3",
  "state": "gating",
  "count": 1,
  "claims": [
    { "claimId": "asrt_897e054b48d040db", "propositionId": "prop_d6185fc85e2a4e52",
      "text": "Auth tokens expire after 30 minutes", "documentPath": "CLAUDE.md",
      "codePath": "src/auth.ts", "status": "code:changed", "severity": "gating", "gates": true,
      "enforcement": "enforced", "recommended": "update-claim" }
  ]
}
```

**Act**: the sentence in `CLAUDE.md` is no longer needed (the README covers it), so withdraw the claim:

```sh
hibi retire asrt_897e054b48d040db
```

```json
{ "ok": true, "action": "retire", "schemaVersion": "v3",
  "id": "asrt_897e054b48d040db", "alreadyRetired": false,
  "next": "hibi check" }
```

A retired claim never gates, never warns, and is excluded from `clean`. `retire` is idempotent. Do not delete the `.claims/` file by hand. Other filters: `--state orphaned`, `--state stranded` (live claims on a superseded or archived doc), `--state duplicate` (a sentence claimed more than once), `--state suggested`. Add `--ids-only` for a shell loop.

---

## 6. Supersede a document, then fix a miss

**When**: `design-v2.md` replaces `design-v1.md`. Setup: the retry sentence was copied verbatim into v2; the token sentence was reworded.

**Run**: `hibi supersede --from design-v1.md --to design-v2.md`

```json
{
  "ok": true,
  "action": "supersede",
  "schemaVersion": "v3",
  "from": "design-v1.md",
  "to": "design-v2.md",
  "relocated": [
    { "claimId": "asrt_a1b2c3d4e5f60718", "doc": "unchanged", "code": "unchanged" }
  ],
  "misses": [
    { "claimId": "asrt_c3d4e5f6a7b80912",
      "reason": "documented sentence not found in design-v2.md; reanchor with an explicit span or retire" }
  ],
  "strandedClaims": ["asrt_c3d4e5f6a7b80912"],
  "dryRun": false,
  "next": "reanchor or retire each missed claim, then hibi check"
}
```

**Act**: the relocated claim kept its id, code side, and history; only its document changed (`relocated[].doc` and `.code` are its anchor states after the move). The miss is still live on the superseded doc (`hibi list --state stranded` shows it). Find where its sentence went:

```sh
hibi reanchor asrt_c3d4e5f6a7b80912 --suggest
```

```json
{
  "ok": true,
  "action": "reanchor-suggest",
  "schemaVersion": "v3",
  "id": "asrt_c3d4e5f6a7b80912",
  "candidates": [
    { "side": "doc", "file": "design-v2.md", "start": 210, "end": 251, "similarity": 0.83,
      "snippet": "Tokens are valid for 30 minutes after issue." },
    { "side": "code", "file": "src/auth.ts", "start": 29, "end": 47, "similarity": 1,
      "snippet": "TOKEN_TTL_MIN = 30" }
  ]
}
```

`--suggest` is read-only. It ranks candidates for the doc quote across every registered document and for each code quote across files in the same language. Move the claim to the reworded sentence:

```sh
hibi reanchor asrt_c3d4e5f6a7b80912 --doc design-v2.md \
  --doc-quote "Tokens are valid for 30 minutes after issue."
```

```json
{
  "ok": true,
  "action": "reanchor",
  "schemaVersion": "v3",
  "id": "asrt_c3d4e5f6a7b80912",
  "doc": "unchanged",
  "code": "unchanged",
  "before": { "doc": { "file": "design-v1.md", "quote": "Tokens expire after 30 minutes." },
              "code": [{ "file": "src/auth.ts", "quote": "TOKEN_TTL_MIN = 30" }] },
  "after":  { "doc": { "file": "design-v2.md", "quote": "Tokens are valid for 30 minutes after issue." },
              "code": [{ "file": "src/auth.ts", "quote": "TOKEN_TTL_MIN = 30" }] },
  "next": "hibi check"
}
```

If no candidate fits, the sentence was dropped: `hibi retire asrt_c3d4e5f6a7b80912`. When `strandedClaims` is empty the old file can be deleted or archived (`hibi archive --doc design-v1.md --successor design-v2.md`) without orphaning anything. Add `--dry-run` to `supersede`, `reanchor`, `retire`, or `archive` to preview.
