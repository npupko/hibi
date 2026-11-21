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
  `hibi cover --since main...` (after copying text, plain `hibi cover`; see
  step 7), then commit `.hibi/` with the change.

## The session loop

1. **Status.** `hibi status --detail` lists stale and expired claims with the
   current text of every side. Each side that is not `same` has a `state`
   (`changed`, `missing`, `ambiguous`, `unavailable`) and a `reason`.

   A stored anchor identifies its unit by **name** (`#name`: the
   declarations with that name, while the name has as many as when the
   claim was made) or by **place** (a quote: those words in their scope,
   with the text right around them unchanged). The scope is every container
   the stored path names: all sections with the same heading path, all
   paragraphs with the same first sentence, all declarations sharing a
   symbol (overloads, a Python function defined twice, Rust `Id.fmt` in two
   `impl` blocks). Anything else is a candidate for you to choose. A copy in
   another scope is only a candidate. A copy in the same scope can take the
   claim over without notice, and so can a pasted copy once the original is
   renamed (last rows). The full list of limits is at
   https://hibi.mintlify.app/concepts/anchors#limits. What an edit does to a side:

   | Edit | Side | Then |
   |---|---|---|
   | The body of a declaration anchored `#name` | `changed` | Read the change, re-check, `hibi mark` |
   | A code line quoted by a bare quote commented out | `missing` (`not_exact`) | Read, `hibi reanchor` |
   | The line of a `#name::<line>` anchor edited, removed or commented out | `changed`, "the stored quote is no longer in this unit"; `mark` refuses | Read it, re-check, `hibi reanchor` with the new line |
   | Other lines of a `#name::<line>` declaration | `changed` | Read the change, re-check, `hibi mark` |
   | A sentence edited at its quoted words, or right around a quoted phrase | `missing` (`not_exact`) | Read the candidates and the file, re-check, `hibi reanchor` |
   | A long sentence edited away from its quote (not in the first 120 characters of a paragraph's first sentence) | `changed` | Read the change, re-check, `hibi mark` |
   | Another sentence edited, not the first of its paragraph or list item | nothing | |
   | The first sentence of a paragraph or list item edited, deleted or inserted, the first cell of a row edited, a paragraph split or joined, a paragraph left with one sentence, an ordered list renumbered, a blockquote reflowed | the claims on that paragraph, item or row are `missing` (`not_exact`); the right sentence usually listed first | Read, `hibi reanchor` |
   | A heading renamed, a section or paragraph pasted again under the same heading, a sentence moved to another paragraph, a function around a quote renamed | `missing` (`not_exact`), unless another container now has the name (next rows) | Read, `hibi reanchor` |
   | A declaration anchored `#name` renamed or deleted | `missing` (`selector_not_found`), usually with no candidate | Find it yourself, `hibi reanchor` |
   | A second declaration of the name added | `ambiguous` or `missing` (`not_exact`), the declarations listed | Read, `hibi reanchor`. In Python a later `def` of a name replaces an earlier one when the module runs, and the earlier, unused one is listed first: check which one the code uses. |
   | Text copied within its scope (a sentence, list item, table row, or a line into another declaration of the same name), the original then edited | nothing: the claim follows the copy and stays fresh; with overloads only the line numbers differ | Read the line; `hibi cover` lists the edited original as uncovered |
   | A paragraph or section pasted again, then the original's first sentence or heading edited | nothing: only the copy has the name now, and the claim is fresh on it | Same check |
   | A step removed from one of two numbered lists in a section | a claimed item can now read the same as the other list's item, and the claim is fresh on that | Same check |
2. **Read the change.** For a `changed` side, `status` prints the unit's text
   as it is now, and `hibi status --detail <id>` prints every side's text.
   That is the current text, not a diff: for the change itself, run
   `git diff -- <path>` for an edit that is not committed, or
   `git log -p -- <path>` for a committed one. `mark` and `reanchor` print
   the text they accepted ("evidence 1 is now:"); read it. `reanchor` marks
   every changed side it keeps, not only the side it re-points, so before
   any `reanchor` run `hibi status --detail <id>` and read every changed
   side.
3. **Re-check** the statement against what you read. Decide whether it still
   holds. Never run `next` without this step, and never mark claims in bulk
   without having read each change.
4. **A side that is missing, ambiguous or unavailable.** Act on its `reason`:

   | `reason` | What to do |
   |---|---|
   | `not_exact` | The stored anchor no longer identifies one unit. `candidates` lists units it could mean: first units with the marked content (`sameFingerprint: true`), then hibi's guesses. A copy of the text has `sameFingerprint: true` too, so read each one and the file. If one is the unit the claim is about, re-check the statement against it, then run `next` with its `anchor` or with an anchor of your own. A printed `anchor` quotes the unit's first line or whole text, so it can widen a short quote to a whole sentence, or move a `#name::<line>` to another line, without saying so; when the claim is about particular words (a number, a name, one line of code), anchor them yourself (`<loc>::<words>`, `<loc>#<name>::<line>`). After you re-anchor to a candidate with the marked content, check that it is the original and not a copy: look at its line in the file, or run `hibi cover` and see whether the original is now uncovered. `status` cannot tell you, because the claim is fresh either way. When the detail says "the name and the quoted text now point to different units", the quoted line moved into another declaration: the first candidate has the name, the others hold the line. The right unit may not be listed; then find it yourself. |
   | `selector_not_found` | Nothing found: a declaration anchored `#name` was renamed or removed, or the unit was rewritten or moved to another file (only the side's own file is searched). Find it with `git diff` or `git log -p`, re-check, and re-anchor. If it is gone, ask the user before you retire. |
   | `resource_not_found` | The file was moved, renamed or deleted. Find the new path, for example with `git log --stat -- <old path>`, then re-anchor; if it was deleted, ask the user before you retire. |
   | `several_units` | The anchor matches several units, listed in `candidates`, for example a name that now has a second declaration. Choose the one the statement is about and re-anchor to its `anchor`. A candidate with `anchor: null` is identical, where it stands, to another one, and cannot be anchored alone: anchor the unit that contains them (the `#name` of the enclosing declaration, or the whole file). |
   | `selector_refused` | The provider refuses the stored anchor, or no longer allows one of its selector keys; `detail` says which. Re-anchor the side with an anchor the message allows, or ask the user before you retire. |
   | `resource_unavailable` | The resource cannot be read now; `detail` says why, for example a syntax error on line 7. A code file that does not parse cannot be anchored at all until it parses. Fix the cause if your change made it, then run `hibi status` again; if it can never parse, ask the user to exclude it from the scope. Do not mark. |
   | `too_large`, `not_text` | The file is over a size limit (2 MiB for code, 8 MiB for other files, 100,000 units) or is not UTF-8. Anchor the statement to another file, or ask the user before you retire. |
   | `profile_mismatch` | The fingerprint profile changed, for example after an external provider renamed its profile, or after a hibi upgrade. Re-check and mark. |
   | `provider_failed` | `next` is `hibi providers --check <scheme>`, which starts the provider and reports what fails. Tell the user, and run it when they ask. Do not mark. |
   | `provider_not_registered`, `repo_not_allowed` | `next` is `hibi providers`. Run it and tell the user: providers and sibling repositories are set up in their user config. |
   | `scheme_not_declared` | The scheme is no longer declared in `.hibi/hibi.toml`. Ask the user whether to declare it again or retire the claim. |
   | `repo_not_found`, `provider_error` | The sibling repository is missing, or the provider answered with an error. `remediation` says what to do; tell the user. |

   `mark` refuses a claim with a side that is missing, ambiguous or refused:
   it never follows a candidate.
5. **Record what you found.**
   - Still holds: `hibi mark <id>`.
   - Holds, but the subject moved or matches several units:
     `hibi reanchor <id> --subject <anchor>`. This also marks every other
     changed side of the claim: re-check those first. When several kept
     sides need a new anchor, `reanchor` fails once, naming them all, and
     its fix line repeats your arguments with an `<anchor>` for each.
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
   For code write `#name`, or `#name::<line>` to say which line the claim is
   about; for prose, and code outside any declaration, a bare quote. A bare
   quote inside a function follows the text, and `claim` prints a note
   (`notes` in JSON) with the `#name::<line>` anchor that follows the
   declaration instead.
   Running the same claim again (the same statement, `recheck` and `note`,
   and sides on the same units, however you wrote the anchors) marks the
   existing live claim (`created: false`) instead of adding one. It reports
   what the claim was before (`accepted.was`, for example `stale`).
   Re-check before you do that, as for `mark`. Two elements of one batch
   that are the same claim make one claim. When the same claim was retired,
   `claim` fails with `claim_retired`: a retirement is a decision, so ask
   the user before you record it again with a different `--note`. When a
   side of the existing claim is missing or ambiguous, `claim` writes a
   second claim: re-anchor the existing one instead.
7. **Cover.** `hibi cover` (or `--since main...`) lists units of the declared
   scopes that no claim covers. Claim the checkable ones. To exclude one unit,
   with the user's agreement, add an entry to the scope's `exclude` list in
   `.hibi/hibi.toml`. The unit key is in the JSON output of `cover`
   (`scopes[].items[].unit.key`); the text output prints a key only in the
   hint under a unit that cannot be anchored alone:
   `{ resource = "file:docs/retry.md", unit = "sentence:0863e2920c1f", reason = "a pointer, not a statement" }`.
   A prose unit's key comes from its content, so the exclusion stops
   matching when that sentence is reworded, and `cover` lists it again.
   Leave out `unit` to exclude a whole file: it is then not read at all, its
   units are not counted, it is not listed as unavailable, and a claim on it
   does not show as covered in that scope. That is the remedy, with the
   user's agreement, for a file `cover` lists as unavailable for good (does
   not parse, too large, not text).
   An item printed with "cannot be anchored alone" (`anchor: null` in JSON)
   is identical, where it stands, to another unit. A claim on the unit that
   contains it records a statement about them, but does not cover them. To
   stop `cover` listing them, ask the user to exclude each one by the key
   the hint names, or to exclude the whole file.
   `hibi cover --since <range>` lists only units whose key and content are new since
   the base, so a sentence copied within its file, with the original then
   edited, is not listed even when no claim covers it. After copying text,
   run `hibi cover` without `--since`.

Pass `--by agent:claude` (or set `HIBI_BY`) so marks record who checked.

## Reading `next`

Each listed claim has `next`, the command to run after a successful re-check,
or `null`:

| `next` | Meaning |
|---|---|
| `hibi mark <id>` | A side changed or the period expired. Read the change, re-check, then mark, or repair or retire. |
| `hibi reanchor <id> --subject <anchor>` | The subject is missing or ambiguous, its stored anchor is refused, or its stored line is gone. Read the candidates or the unit, choose, and give its anchor. |
| `hibi reanchor <id> --evidence <n>=<anchor>` | Evidence number n is missing or ambiguous, its stored anchor is refused, or its stored line is gone. Read the candidates or the unit, choose, and give its anchor. |
| `hibi providers` | A provider or sibling repository is not set up. Tell the user; setup is in their user config. |
| `hibi providers --check <scheme>` | A provider failed. The command starts it and reports what fails. Tell the user, and run it when they ask. |
| `null` | Fresh, retired, or unavailable for a reason no command fixes. The side's `remediation` says what to do, for example fix a syntax error or declare a scheme. |

`next` never contains a candidate: `<anchor>` is yours to fill. Plain
`hibi status` lists stale and expired claims. `hibi status <id>` shows one
claim in any state.

## Renames

hibi keeps no alias for a renamed or moved unit. The claim is stale until
you re-anchor it.

- **A declaration anchored `#name` was renamed** (function `limit` to
  `queue_limit`): the side is `missing` (`selector_not_found`) with no
  candidate, because the name is part of its content. Find it yourself, for
  example with `git diff`; if it is the same thing, re-check and run
  `hibi reanchor <id> --evidence <n>=file:src/jobs.py#queue_limit`.
- **A function around a bare quote was renamed**: the side is `missing`
  (`not_exact`) and lists the function that holds the quote. Read it and
  re-anchor.
- **A container was renamed** (struct `Config` to `Settings`): a member's
  side is `missing` with the member under its new path as a candidate, such
  as `Settings.timeout_secs`, and `sameFingerprint: true` (text: "same
  content as at the reference mark"). Its content is what was checked;
  re-anchor to it.
- **A heading was renamed**: every claim under it is `missing` (`not_exact`)
  with its unchanged sentence listed first. Re-anchor each.
- **A file was renamed or moved**: the side is `missing`
  (`resource_not_found`) with no candidates. Give the new path with
  `--subject` or `--evidence <n>=<anchor>`.

## Anchors

| Compact anchor | Selects |
|---|---|
| `file:docs/retry.md::gives up after 5 attempts` | The unit that contains this exact quote |
| `file:src/retry.rs#MAX_ATTEMPTS` | The declaration with this symbol |
| `file:src/config.rs#Config.timeout_secs` | A member: struct field, enum variant, method |
| `file:src/retry.rs#delay_ms::200 * 2u64.pow(n)` | The declaration, and the line of it the claim is about |
| `file:docs/retry.md` | The whole file |
| `repo:api//src/lib.rs#timeout_secs` | A file in an allowed sibling repository |
| `ref:notes` | An external resource with the empty selector |

Rules for a new anchor (in `claim`, and a replaced or added side in
`reanchor`):

- It must match exactly (`selector_not_exact` otherwise). A quote that differs
  only in whitespace counts as exact.
- A quote that occurs in more than one unit is `side_ambiguous`, whatever its
  length. The error lists each candidate with an anchor you can use, or says
  that the units cannot be told apart.
- With `#symbol::quote`, the quote must occur inside that declaration;
  otherwise the anchor is `selector_not_exact`. A doc comment above the
  declaration is outside it.
- A `#symbol` works only in code. On a markdown, prose or text file it is
  refused (`invalid_anchor`): "this kind of file has no symbols; anchor it
  with a quote".
- A `#symbol` that several declarations share (overloads, a function
  defined twice) is `side_ambiguous`, and the error lists each one with a
  JSON anchor that selects it alone, by a line only it holds. Such an anchor
  resolves while the name has that many declarations, and its scope is all
  of them: a copy of its line in another of them, the original then edited,
  takes the claim. Read the line the side shows.
- Units that are identical, with identical text around them, in one
  scope cannot be anchored one by one: `claim` answers `side_ambiguous`
  with each unit marked "cannot be told apart". Anchor the unit that
  contains them (the enclosing `#name`, or the whole file). In `cover`, ask
  the user to exclude each one by its key, or the whole file; a claim on
  the containing unit does not cover them. (`quote_not_distinguishable` is a guard behind this
  that the file provider does not produce in practice.)
- Two evidence anchors that resolve to the same unit are refused
  (`invalid_argument`), however they are written.
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
echo '["c1-xqhb", {"claim": "c1-2axv", "note": "checked against delay_ms"}]' | hibi mark --stdin
```

An anchor in a batch is a compact string, `{"loc": ..., "sel": {...}}`, or
that object as a JSON string, as candidates and errors print it.
Use the object form for a selector other than the empty one on an external
scheme, for example `{"loc": "ref:notes", "sel": {"key": "n1"}}`, and for a
path that contains `#` or `::`. Each `cover` item has an `anchor` object you
can use as `subject` unchanged, or `null` for a unit that cannot be anchored
alone.

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
