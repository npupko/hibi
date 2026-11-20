# hibi command reference

This file is generated from the command definition by
`cargo run -p hibi-cli --example cli-reference`. Do not edit it by hand.

hibi records that a statement was checked against evidence. Later it reports which statements need a new check because their text or their evidence changed, and which parts of a declared scope have no claim at all.

hibi decides nothing about whether a statement is true. You check it, hibi remembers that you did and what the text and the evidence looked like then.

```
hibi [OPTIONS] <COMMAND>
```

## Global options

Every command accepts these.

- `--root <dir>`: Use this directory as the root (the directory that contains .hibi/) instead of searching upward from the working directory.
- `--format <format>`: Output format. Without this option the output is JSON when standard output is not a terminal, and text when it is. Values: `human` (Text for a person), `json` (One JSON object).
- `--now <timestamp>`: Use this instant as the current time, for reproducible runs. RFC 3339 with any offset, for example 2026-11-20T18:04:11Z. A fraction of a second is dropped.
- `--no-cache`: Do not read or write the cache database. Results are the same with and without the cache.
- `-h, --help`: Print help and exit with status 0.

## Anchors

```
An anchor names one unit of a resource: a sentence, a heading, a function.
  file:docs/retry.md                     the whole file
  file:src/retry.rs#fetch_with_retry     the declaration with this symbol
  file:docs/retry.md::capped at 30 s     the unit that contains this quote
  file:src/retry.rs#retry::MAX = 5       a quote inside a symbol
Use repo:<alias>//<path> for a file in a sibling repository. A path that
contains '#' or '::', and any selector for an external scheme, is given as
JSON on standard input (claim --stdin).
```

## Output

```
The result is JSON when standard output is not a terminal, or with
--format json: one object with "ok", "command" and the result keys, or
with "ok": false and "errors". Otherwise the result is text.
```

## Exit status

```
0 when the command ran, whatever it found. 1 on any error.
```

## Environment

```
HIBI_BY            default for --by
HIBI_CONFIG        path of the user config file
HIBI_CACHE_DIR     directory of the cache database
TYPESAFE_API_KEY   key for the judge commands (triage, suggest-evidence,
                   status --judge-order); nothing else uses the network
NO_COLOR           turn colour off in help output
```

## Commands

- [`hibi init`](#hibi-init): Create the .hibi/ store in this git repository
- [`hibi claim`](#hibi-claim): Record a statement, what it is about and its evidence, as checked now
- [`hibi status`](#hibi-status): Report which claims need a new check and why
- [`hibi mark`](#hibi-mark): Record that claims were checked again and still hold
- [`hibi reanchor`](#hibi-reanchor): Replace a claim with one that has a new statement, anchors or period
- [`hibi retire`](#hibi-retire): Retire claims that no longer apply
- [`hibi cover`](#hibi-cover): List the units in the declared scopes that no claim covers
- [`hibi list`](#hibi-list): List stored claims, or show one claim with its history
- [`hibi triage`](#hibi-triage): Rank uncovered prose by how likely it states a checkable fact (judge)
- [`hibi suggest-evidence`](#hibi-suggest-evidence): Rank code units that may be evidence for a statement (judge)
- [`hibi providers`](#hibi-providers): Show the built-in file provider, sibling repositories and schemes
- [`hibi schema`](#hibi-schema): Print the JSON Schema of a record, an input or a command's output
- [`hibi completions`](#hibi-completions): Print a shell completion script
- [`hibi version`](#hibi-version): Print the hibi version

## hibi init

Create the .hibi/ store in this git repository.

```
hibi init [OPTIONS]
```

Create the .hibi/ store at the top level of the git repository that contains the working directory: hibi.toml, the claim and mark files, and the files that tell git how to merge them. Nothing is written outside .hibi/.

After init, declare scopes in .hibi/hibi.toml to say which documents and source files must be covered by claims.

## hibi claim

Record a statement, what it is about and its evidence, as checked now.

```
hibi claim [OPTIONS] [statement]
```

Record a claim: a statement, the unit it is about (the subject) and the units that support it (the evidence). hibi stores the claim and a first mark with the fingerprint of every side as it is now. Run this after you checked that the statement is true.

Every anchor must match exactly one unit. Running the same claim again adds a new mark and no second claim.

With --stdin, standard input is a JSON array of claims, written all or not at all:

```
[{"statement": "...", "subject": "file:docs/a.md::quoted text",
  "evidence": ["file:src/a.rs#name"], "recheck": "P90D",
  "note": "...", "by": "..."}]
```

An anchor is a compact string or an object {"loc": ..., "sel": {...}}. Only "statement" and "subject" are required. `hibi schema claim-input` prints the schema.

Arguments:

- `<statement>`: The statement that was checked.

Options:

- `--subject <anchor>`: What the statement is about: an anchor, usually the sentence in a document that makes the statement.
- `--evidence <anchor>`: A unit that supports the statement: an anchor, usually a function or another declaration. Repeat for several.
- `--recheck <duration>`: Check again after this long even if nothing changed: an ISO 8601 duration with whole numbers, for example P90D or P6M.
- `--note <text>`: A note stored with the claim: how to check it again, or where the truth lives.
- `--by <text>`: Who checked, stored with the mark for the record [default: the value of HIBI_BY]
- `--stdin`: Read a JSON array of claims from standard input. No other claim argument is allowed with it.

## hibi status

Report which claims need a new check and why.

```
hibi status [OPTIONS] [claim-id]...
```

Compare every claim with the current content. A claim is fresh when some mark matches the current fingerprint of every side, stale when none does, expired when a mark matches but the recheck period has passed, and retired when it was retired.

Without arguments the stale and expired claims are listed. Each listed claim shows the sides that are not the same as at its mark, and the next command to run. Re-check the statement before you run that command. The summary always counts every claim in the store.

Arguments:

- `<claim-id>`: Show exactly these claims, in any state. An id may be shortened to a unique prefix of at least 4 characters after "c1-". May be given several times.

Options:

- `--since <range>`: List only the stale claims that the changes since a commit caused. The range is A, A.. or A..HEAD (since commit A), or A... or A...HEAD (since the merge base of A and HEAD). The compared end is always the working tree. Cannot be combined with --state or with ids.
- `--loc <glob>`: Keep a claim only when the locator of one of its sides matches this glob, for example 'file:src/**'. Repeat for several.
- `--state <state>`: List claims in this state. Repeat for several [default: stale and expired] Values: `fresh` (Some mark matches the current content and is recent enough), `stale` (No mark matches the current content), `expired` (A mark matches the content, but the recheck period has passed), `retired` (The claim was retired).
- `--detail`: Include the current text of every side.
- `--judge-order`: Order the stale claims by how likely each is broken, most likely first, as rated by the judge model. Needs TYPESAFE_API_KEY and git. The order is the only thing that changes.

## hibi mark

Record that claims were checked again and still hold.

```
hibi mark [OPTIONS] [claim-id]...
```

Add a mark to each claim: a record that you checked the statement again against the current content and it still holds. The claims become fresh.

Every side must resolve to one unit. If a side is missing or matches several units, use `hibi reanchor` to point it at the right unit. If the statement is no longer true, fix the text or use `hibi retire`.

With --stdin, standard input is a JSON array, written all or not at all. Each element is a claim id or {"claim": "<id>", "note": "...", "by": "..."}. `hibi schema mark-input` prints the schema.

Arguments:

- `<claim-id>`: The claims that were checked again. An id may be shortened to a unique prefix. May be given several times.

Options:

- `--note <text>`: A note stored with each mark.
- `--by <text>`: Who checked, stored with each mark for the record [default: the value of HIBI_BY]
- `--stdin`: Read a JSON array of claim ids or objects from standard input. No other mark argument is allowed with it.

## hibi reanchor

Replace a claim with one that has a new statement, anchors or period.

```
hibi reanchor [OPTIONS] <claim-id>
```

Build a new claim from an existing one and retire the old one, in one write. Claims are never edited in place: a change of the statement, an anchor, the recheck period or the note gives a new claim id. The new claim gets a mark, so run this only after you checked the statement.

Sides you do not replace are resolved again from their stored anchors, and every stored selector is refreshed from the unit it resolves to. Without any option this refreshes the selectors of a claim whose text moved or was reworded slightly.

Evidence numbers start at 1 and always refer to the old claim's list.

Arguments:

- `<claim-id>`: The claim to replace. May be shortened to a unique prefix. Required.

Options:

- `--statement <text>`: The new statement.
- `--subject <anchor>`: A new anchor for the subject.
- `--evidence <n=anchor>`: A new anchor for evidence number n, for example 1=file:src/a.rs#name. Repeat for several.
- `--add-evidence <anchor>`: Add an evidence anchor at the end of the list. Repeat for several.
- `--drop-evidence <n>`: Remove evidence number n. Repeat for several.
- `--recheck <duration|none>`: A new recheck period, or "none" to remove it.
- `--note <text|none>`: A new note, or "none" to remove it.
- `--by <text>`: Who checked, stored with the mark and the retirement [default: the value of HIBI_BY]

## hibi retire

Retire claims that no longer apply.

```
hibi retire [OPTIONS] <claim-id>...
```

Retire each claim. A retired claim is kept in the store, is no longer checked and no longer counts as covering anything. Retirement is permanent.

Use this when a statement was removed or is no longer true. No provider is contacted.

Arguments:

- `<claim-id>`: The claims to retire. An id may be shortened to a unique prefix. Required. May be given several times.

Options:

- `--replaced-by <claim-id>`: The claim that takes the place of the retired ones. It must exist and must not be one of them.
- `--note <text>`: A note stored with each retirement, for example why.
- `--by <text>`: Who retired, stored for the record [default: the value of HIBI_BY]

## hibi cover

List the units in the declared scopes that no claim covers.

```
hibi cover [OPTIONS]
```

For every scope declared in .hibi/hibi.toml, count its units and list the ones no claim covers. A unit is covered when a side of a live claim resolves to it or to a unit nested in it, whatever the claim's state. Units matched by an exclusion are counted separately, with the reason given in hibi.toml.

Each listed unit comes with an anchor that can be passed to `hibi claim --stdin` as it is. This command needs git.

Options:

- `--scope <name>`: Report only this scope. Repeat for several [default: every scope]
- `--since <range>`: Report only units that were added or changed since a commit. The range is written as for `hibi status --since`. Only scopes on scheme "file" are evaluated.
- `--limit <n>`: List at most this many uncovered units per scope; 0 lists all. The counts are never cut. Default: `200`.

## hibi list

List stored claims, or show one claim with its history.

```
hibi list [OPTIONS] [claim-id]
```

Read the store and print claims as they are stored. Nothing is resolved and no state is computed, so this works when files or providers are missing.

Without an id: one line per live claim. With an id: the stored claim, its marks in time order and its retirement.

Arguments:

- `<claim-id>`: Show this claim as stored, with its marks and retirement. May be shortened to a unique prefix.

Options:

- `--loc <glob>`: Keep a claim only when the locator of one of its sides matches this glob. Repeat for several.
- `--text <substring>`: Keep a claim only when its statement contains this text. The match is case-sensitive.
- `--retired`: Include retired claims.

## hibi triage

Rank uncovered prose by how likely it states a checkable fact (judge).

```
hibi triage [OPTIONS]
```

Ask the judge model which uncovered sentences, list items, table rows and headings state a fact that could be confirmed by reading code or configuration, and where the evidence would be. The result is a ranking to read, not a finding: nothing is written to the store.

Needs TYPESAFE_API_KEY and git. The text of the listed units is sent to the judge service. Only files that git tracks, or that are untracked and not ignored, are sent.

Options:

- `--scope <name>`: Consider only this scope. Repeat for several [default: every scope]
- `--since <range>`: Consider only units that were added or changed since a commit. The range is written as for `hibi status --since`.
- `--limit <n>`: Print at most this many units; 0 prints all. Default: `50`.

## hibi suggest-evidence

Rank code units that may be evidence for a statement (judge).

```
hibi suggest-evidence [OPTIONS] --in <glob> <statement>
```

Ask the judge model which code units, among those matched by --in, decide whether a statement is true. The result is a list of units to read, never a finding: nothing is written to the store.

Needs TYPESAFE_API_KEY and git. The text of the candidate units is sent to the judge service. Only files that git tracks, or that are untracked and not ignored, are sent.

Arguments:

- `<statement>`: The statement to find evidence for. Required.

Options:

- `--in <glob>`: Where to look: a locator glob on scheme "file" or "repo", for example 'file:src/**/*.rs'. Repeat for several. Required.
- `--kind <kind>`: Consider only units of this kind. Repeat for several [default: function, method, class, struct, interface, trait, enum, type, variable]
- `--limit <n>`: Print at most this many candidates. Default: `10`.

## hibi providers

Show the built-in file provider, sibling repositories and schemes.

```
hibi providers [OPTIONS]
```

Show what can resolve locators here: the built-in file provider, every sibling repository alias of .hibi/hibi.toml with the directory your user config allows for it, and every external scheme with the command your user config registers for it. For anything that is not set up, the lines to add to your user config are printed.

Without --check no provider command is started.

Options:

- `--check <scheme>`: Start the provider registered for this scheme and run the protocol conformance cases against it. A failing case is reported and does not make the command fail.

## hibi schema

Print the JSON Schema of a record, an input or a command's output.

```
hibi schema [OPTIONS] [name]
```

Without a name: list the schema names. With a name: print that JSON Schema (draft 2020-12) and nothing else, in both output formats.

Names: the stored records (claim, mark, retirement, anchor, selector), the configuration files (config, user-config), the standard-input documents (claim-input, mark-input), the error output (error), the provider protocol (unit, provider-request, provider-response, conformance-case), and <command>-output for each command with JSON output.

Arguments:

- `<name>`: The schema to print [default: list the names]

## hibi completions

Print a shell completion script.

```
hibi completions [OPTIONS] <shell>
```

Print the completion script for the given shell to standard output. There is no JSON form.

Arguments:

- `<shell>`: The shell to print the script for. Required. Values: `bash`, `elvish`, `fish`, `powershell`, `zsh`.

## hibi version

Print the hibi version.

```
hibi version [OPTIONS]
```

Print the hibi version. The JSON form also gives the provider protocol versions, the store version, the fingerprint profiles, the grammar versions and the judge model.
