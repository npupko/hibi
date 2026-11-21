---
name: hibi
description: >-
  Use hibi, the memory of which statements were checked against which
  evidence. Use at the start of a session in a repository that has .hibi/,
  before trusting a document or a comment that a claim covers, after changing
  code or docs that claims anchor to, before finishing a branch, and whenever
  the user mentions claims, stale docs, doc and code drift, or coverage of
  docs by checks. Also use when the user asks to set up hibi, start tracking
  docs against code, or add hibi to a repository.
allowed-tools: >-
  Bash(hibi status:*), Bash(hibi list:*), Bash(hibi cover:*),
  Bash(hibi providers:*), Bash(hibi version:*), Bash(hibi schema:*)
---

# hibi

hibi records that a statement was checked against evidence (a **claim**, with
a first **mark** holding the fingerprint of every side), and later reports
which side of which claim changed (`hibi status`) and what in a declared scope
has no claim (`hibi cover`). It does not show what changed in a side: you read
that yourself before you mark. It is memory, not a gatekeeper. You are trusted to judge
whether a statement holds. hibi tells you what to re-check. When you cannot
decide, ask the user.

Every command exits 0 when it ran, whatever it found, and 1 on an error. In
your shell tool standard output is not a terminal, so each command prints one
JSON object. Add `--format human` for a shorter text form.

## Setup, when the repository has no `.hibi/`

Ask the user before each of these steps; they change the repository.

1. `hibi init` creates `.hibi/` at the top of the git repository.
2. Agree with the user which files must be covered, then append a scope to
   `.hibi/hibi.toml`:

   ```toml
   [[scope]]
   name = "docs"
   resource = "file:docs/**/*.md"
   kinds = ["sentence", "list-item", "table-row"]
   ```

3. `hibi cover` lists what has no claim. Claim what you check.
4. Commit `.hibi/` with the change.

## When to use it

- At the start of a session in a repository with `.hibi/`: run `hibi status`.
- Before relying on a sentence in a document:
  `hibi status --state fresh --state stale --state expired --loc 'file:docs/x.md'`
  lists every live claim on it with its state. Plain `status --loc` prints
  "no claim matches" both when every claim there is fresh and when there is
  no claim; `hibi list --loc 'file:docs/x.md'` tells the two apart.
- After you changed code or docs: `hibi status` lists what your edit made stale.
- After you wrote or verified a statement: record it with `hibi claim`.
- Before finishing a branch: `hibi status --since main...` and
  `hibi cover --since main...`, then commit `.hibi/` with the change.

## The session loop

1. **Status.** `hibi status --detail` lists stale and expired claims with the
   current text of every side. Each side that is not `same` has a `state`
   (`changed`, `missing`, `ambiguous`, `unavailable`) and a `reason`.
2. **Read the change.** For each changed side, look at what changed before
   you decide anything: `git diff -- <path>` for a file, or
   `hibi status --detail <id>` for the unit's current text. `status` says
   which side changed, and `mark` prints only "was stale: evidence 1 ...
   changed; now marked", so neither shows you the change.
3. **Re-check** the statement against what you read. Decide whether it still
   holds. Never run `next` without this step, and never mark claims in bulk
   without having read each change.
4. **A side not found exactly.** A side whose anchor is not found exactly is
   `missing` with reason `not_exact`, and lists `candidates`: units found by
   similarity, each with an `anchor` that selects it. A candidate is a guess.
   Read it. If it is the unit the claim is about, re-check the statement
   against it, then `hibi reanchor <id> --subject <anchor>` or
   `--evidence <n>=<anchor>` with that anchor. If none is right, find the
   unit and give its anchor. `mark` refuses such a claim (`side_missing`).
5. **Record what you found.**
   - Still holds: `hibi mark <id>`.
   - Holds, but the subject moved or matches several units:
     `hibi reanchor <id> --subject <anchor>`.
   - Holds, but evidence number n moved or matches several units:
     `hibi reanchor <id> --evidence <n>=<anchor>`.
   - The statement must change: fix the text first, then
     `hibi reanchor <id> --statement "..." --subject <anchor>`.
   - The sentence was deleted, or the statement no longer applies: ask the
     user before you retire, especially when the evidence still shows the
     fact is true (the document may have lost something it should say). Then
     `hibi retire <id> --note "why"`.
   - You cannot tell whether the document or the code is right: ask the user.
     Do not mark.
6. **Claim.** After you check a new statement, record it:
   `hibi claim "<statement>" --subject <anchor> --evidence <anchor>`.
   Running the same claim again, even after text near the quote changed,
   marks the existing live claim (`created: false`) instead of adding one,
   and reports what it was before (`accepted.was`, for example `stale`).
   Re-check before you do that, as for `mark`.
7. **Cover.** `hibi cover` (or `--since main...`) lists units of the declared
   scopes that no claim covers. Claim the checkable ones. To exclude one unit,
   with the user's agreement, add an entry to the scope's `exclude` list in
   `.hibi/hibi.toml`. The unit key is in the JSON output of `cover`
   (`items[].unit.key`); the text output prints no keys:
   `{ resource = "file:docs/retry.md", unit = "sentence:0863e2920c1f", reason = "a pointer, not a statement" }`.
   Leave out `unit` to exclude a whole file.

Pass `--by agent:claude` (or set `HIBI_BY`) so marks record who checked.

## Reading `next`

Each listed claim has `next`, the command to run after a successful re-check,
or `null`:

| `next` | Meaning |
|---|---|
| `hibi mark <id>` | A side changed or the period expired. Read the change, re-check, then mark, or repair or retire. |
| `hibi reanchor <id> --subject <anchor>` | The subject is missing or ambiguous. Read the candidates, choose the unit, and give its anchor. |
| `hibi reanchor <id> --evidence <n>=<anchor>` | Evidence number n is missing or ambiguous. Read the candidates, choose the unit, and give its anchor. |
| `hibi providers` | A provider or sibling repository is not set up. Tell the user; setup is in their user config. |
| `null` | Fresh, retired, or unavailable for a reason no command fixes. The side's `remediation` says what to do, for example fix a syntax error or declare a scheme. |

`next` never contains a candidate: `<anchor>` is yours to fill. Plain
`hibi status` lists stale and expired claims. `hibi status <id>` shows one
claim in any state.

## Renames

hibi keeps no alias for a renamed or moved unit. The side is `missing`, and
the claim is stale until you re-anchor it.

- **A declaration was renamed** (function `limit` to `queue_limit`): the side
  is `missing` (`not_exact`) and usually lists the renamed declaration as a
  candidate, with an anchor such as `file:src/jobs.py#queue_limit`. The name
  is part of its content, so `sameFingerprint` is `false`. Read it; if it is
  the same thing, re-check and run `hibi reanchor <id> --evidence <n>=<anchor>`
  with that anchor. A short declaration whose rename changed most of its
  first line can have no candidate (`selector_not_found`); find it yourself.
- **A container was renamed** (struct `Config` to `Settings`): a member's
  side is `missing` (`not_exact`) with the member under its new path as a
  candidate, such as `Settings.timeout_secs`, and `sameFingerprint: true`
  (text: "same content as at the last mark"). Its content is what was
  checked; re-anchor to it.
- **A file was renamed or moved**: the side is `missing`
  (`resource_not_found`) with no candidates. Give the new path with
  `--subject` or `--evidence <n>=<anchor>`.

## Anchors

| Compact anchor | Selects |
|---|---|
| `file:docs/retry.md::gives up after 5 attempts` | The unit that contains this exact quote |
| `file:src/retry.rs#MAX_ATTEMPTS` | The declaration with this symbol |
| `file:src/config.rs#Config.timeout_secs` | A member: struct field, enum variant, method |
| `file:src/retry.rs#delay_ms::200` | A quote inside a symbol |
| `file:docs/retry.md` | The whole file |
| `repo:api//src/lib.rs#timeout_secs` | A file in an allowed sibling repository |
| `ref:notes` | An external resource with the empty selector |

Rules for a new anchor (in `claim`, and a replaced or added side in
`reanchor`):

- It must match exactly (`selector_not_exact` otherwise). A quote that differs
  only in whitespace counts as exact.
- A quote that occurs in more than one unit is `side_ambiguous`, whatever its
  length. The error lists each candidate with an anchor you can use.
- With `#symbol::quote`, the quote must occur inside that declaration;
  otherwise the anchor is `selector_not_exact`. A doc comment above the
  declaration is outside it.
- A `#symbol` works only in code. On a markdown, prose or text file it is
  refused: "this kind of file has no symbols; anchor it with a quote".
- A quote inside a code comment, including rustdoc, JSDoc and Javadoc, is
  refused (`quote_in_comment`): anchor the code the comment describes. A
  Python docstring is a string and can be quoted.
- A quote must lie inside one unit (`quote_spans_units`). A quote copied from
  a member line may keep its indentation and a trailing `,` or `;`.
- A `file:` path must be one git lists (tracked, or untracked and not
  ignored), spelled with the same letter case. Ignored files and paths under
  `.git/` or `.hibi/` cannot be anchored.

`mark` checks only that every side resolves to exactly one unit. It never
follows a candidate.

Units of code: functions, methods, classes, structs, interfaces, traits,
enums, types, variables, modules, impls, macros, top-level statements, and
members: `field` for data members, `variant` for enum variants and Java enum
constants. Rust attributes and TS decorators belong to the declaration they
precede. Units of prose: `sentence`, `heading`, `list-item`, `table-row`,
`fenced`.

## What stales a claim

Fingerprints are exact. Prose ignores whitespace changes. Code ignores
whitespace, line breaks, comments and a trailing comma, but a Python
backslash line continuation counts, and a formatter that
changes semicolons, quote style or parentheses stales claims; re-check and
mark them. A reverted change makes a claim fresh again.

## Batches

`claim` and `mark` take a JSON array on standard input with `--stdin`, written
all or nothing. Failing elements are reported with their `index`.

```bash
hibi claim --stdin <<'EOF'
[{"statement": "The client gives up after 3 attempts.",
  "subject": "file:docs/retry.md::gives up after 3 attempts",
  "evidence": ["file:src/retry.rs#MAX_ATTEMPTS"],
  "by": "agent:claude"}]
EOF
```

```bash
echo '["c1-zfv9", {"claim": "c1-mdm1", "note": "checked against delay_ms"}]' | hibi mark --stdin
```

An anchor in a batch is a compact string or `{"loc": ..., "sel": {...}}`.
Use the object form for a selector other than the empty one on an external
scheme, for example `{"loc": "ref:notes", "sel": {"key": "n1"}}`, and for a
path that contains `#` or `::`. Each `cover` item has an `anchor` object you
can use as `subject` unchanged.

## Rules

- Re-check before every `mark` and `reanchor`. Both record that you checked.
- Never edit `.hibi/claims/` or `.hibi/marks/` by hand. `.hibi/hibi.toml` is
  edited by hand, with the user's agreement.
- When the document and the code disagree and you cannot tell which is
  right, ask the user.
- `triage`, `suggest-evidence` and `status --judge-order` send text to
  TypeSafe's API and need `TYPESAFE_API_KEY`. Use them only when the user has
  set the key and asked for them. Their output is a ranking, never a finding.
- `hibi providers --check <scheme>` starts the provider program registered in
  the user's config. Run it only when the user asks.
- Provider commands and sibling directories are set up in the user's own
  config, never in `.hibi/hibi.toml`. Do not add a `command` to `hibi.toml`.

## References

- `references/cookbook.md`: worked tasks with real commands and outputs.
- `references/cli-reference.md`: every command and option, generated from the
  code.
