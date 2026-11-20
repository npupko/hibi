---
name: hibi
description: >-
  Use hibi, the memory of which statements were checked against which
  evidence. Use at the start of a session in a repository that has .hibi/,
  before trusting a document or a comment that a claim covers, after changing
  code or docs that claims anchor to, before finishing a branch, and whenever
  the user mentions claims, stale docs, doc and code drift, or coverage of
  docs by checks.
allowed-tools: >-
  Bash(hibi status:*), Bash(hibi list:*), Bash(hibi cover:*),
  Bash(hibi version:*), Bash(hibi schema:*)
---

# hibi

hibi records that a statement was checked against evidence (a **claim**, with
a first **mark** holding the fingerprint of every side), and later reports
exactly what changed (`hibi status`) and what in a declared scope has no claim
(`hibi cover`). It is memory, not a gatekeeper. You are trusted to judge
whether a statement holds. hibi tells you what to re-check. When you cannot
decide, ask the user.

Every command exits 0 when it ran, whatever it found, and 1 on an error. In
your shell tool standard output is not a terminal, so each command prints one
JSON object. Add `--format human` for a shorter text form.

## When to use it

- At the start of a session in a repository with `.hibi/`: run `hibi status`.
- Before relying on a sentence in a document: `hibi status --loc 'file:docs/x.md'`
  tells you whether claims on it are fresh.
- After you changed code or docs: `hibi status` lists what your edit made stale.
- After you wrote or verified a statement: record it with `hibi claim`.
- Before finishing a branch: `hibi status --since main...` and
  `hibi cover --since main...`, then commit `.hibi/` with the change.

## The session loop

1. **Status.** `hibi status --detail` lists stale and expired claims, and
   drifted fresh claims, with the current text of every side. Each side that
   is not `same` has a `state` (`changed`, `missing`, `ambiguous`,
   `unavailable`) and a `reason`.
2. **Re-check** each stale or expired claim. Read the statement and the
   current text. Decide whether the statement still holds. Never run `next`
   without this step.
   **Refresh** each drifted claim. It is fresh, so the statement still holds
   as checked, but a side was found only approximately (its quote was
   reworded, or its symbol's container was renamed). Look at the unit the
   side resolves to now. If it is the right unit, run `next`
   (`hibi reanchor <id>`). If not, give the right anchor with `--subject` or
   `--evidence <n>=<anchor>`.
3. **Record what you found.**
   - Still holds: `hibi mark <id>`.
   - Holds, but a side moved or matches several units:
     `hibi reanchor <id> --subject <anchor>` or `--evidence <n>=<anchor>`.
   - The statement must change: fix the text first, then
     `hibi reanchor <id> --statement "..." --subject <anchor>`.
   - The statement is gone or no longer applies:
     `hibi retire <id> --note "why"`.
   - You cannot tell whether the document or the code is right: ask the user.
     Do not mark.
4. **Claim.** After you check a new statement, record it:
   `hibi claim "<statement>" --subject <anchor> --evidence <anchor>`.
   Running the same claim again, even after text near the quote changed,
   marks the existing live claim (`created: false`) instead of adding one.
5. **Cover.** `hibi cover` (or `--since main...`) lists units of the declared
   scopes that no claim covers. Claim the checkable ones. Excluding a unit
   in `.hibi/hibi.toml` needs a reason and the user's agreement.

Pass `--by agent:claude` (or set `HIBI_BY`) so marks record who checked.

## Reading `next`

Each listed claim has `next`, the command to run after a successful re-check,
or `null`:

| `next` | Meaning |
|---|---|
| `hibi mark <id>` | A side changed or the period expired. Re-check, then mark, or repair or retire. |
| `hibi reanchor <id> --subject <anchor>` | That side is missing or ambiguous. Find the unit and give its anchor. |
| `hibi reanchor <id>` | The claim is fresh but drifted: a side was found only approximately. Check what the side points to, then refresh the stored selector. |
| `hibi reanchor <id> --note <text>` | As above, when the plain form would recreate a retired claim. |
| `hibi providers` | A provider or sibling repository is not set up. Tell the user; setup is in their user config. |
| `null` | Fresh, retired, or unavailable for a reason you cannot fix (not text, too large, provider error). |

Plain `hibi status` lists stale, expired and drifted claims; the summary
counts drifted claims inside `fresh`, as in `2 fresh (1 drifted)`.
`hibi status <id>` shows one claim in any state.

## Anchors

| Compact anchor | Selects |
|---|---|
| `file:docs/retry.md::gives up after 5 attempts` | The unit that contains this exact quote |
| `file:src/retry.rs#MAX_ATTEMPTS` | The declaration with this symbol |
| `file:src/config.rs#Config.timeout_secs` | A member: struct field, enum variant, method |
| `file:src/retry.rs#delay_ms::200` | A quote inside a symbol |
| `file:docs/retry.md` | The whole file |
| `repo:api//src/lib.rs#timeout_secs` | A file in an allowed sibling repository |

- `claim` and a new side in `reanchor` need an exact match (`selector_not_exact`
  otherwise). A quote that differs only in whitespace counts as exact. `mark` accepts an approximately found side and prints its text so
  you see what you marked.
- A quote inside a code comment, including rustdoc, JSDoc and Javadoc, is
  refused (`quote_in_comment`): anchor the code the comment describes. A
  Python docstring can be quoted.
- A quote must lie inside one unit (`quote_spans_units`).
- Units of code: functions, methods, classes, structs, interfaces, traits,
  enums, types, variables, modules, impls, macros, top-level statements, and
  members: `field` for data members, `variant` for enum variants and Java enum
  constants. Rust attributes and TS decorators belong to the declaration they
  precede.
- Units of prose: `sentence`, `heading`, `list-item`, `table-row`, `fenced`.

## What stales a claim

Fingerprints are exact. Prose ignores whitespace changes. Code ignores
whitespace, line breaks, comments and a trailing comma, but a formatter that
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
echo '["c1-zfv9", {"claim": "c1-dryh", "note": "checked against MAX_ATTEMPTS"}]' | hibi mark --stdin
```

An anchor in a batch is a compact string or `{"loc": ..., "sel": {...}}`;
external schemes need the object form, for example
`{"loc": "ref:notes", "sel": {"key": "n1"}}`. Each `cover` item has an
`anchor` object you can use as `subject` unchanged.

## Rules

- Re-check before every `mark` and `reanchor`. Both record that you checked.
- Never edit files under `.hibi/` by hand.
- When the document and the code disagree and you cannot tell which is
  right, ask the user.
- `triage`, `suggest-evidence` and `status --judge-order` send text to
  TypeSafe's API and need `TYPESAFE_API_KEY`. Use them only when the user has
  set the key and asked for them. Their output is a ranking, never a finding.
- Provider commands and sibling directories are set up in the user's own
  config, never in `.hibi/hibi.toml`. Do not add a `command` to `hibi.toml`.

## References

- `references/cookbook.md`: worked tasks with real commands and outputs.
- `references/cli-reference.md`: every command and option, generated from the
  code.
