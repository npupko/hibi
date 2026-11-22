<p align="center">
  <img src="docs/logo/light.png" alt="hibi" width="160">
</p>

# hibi

hibi gives an AI coding agent memory that survives between sessions. The agent
checks a statement against evidence, for example a sentence in a document
against the function it describes, and records that check as a claim. Later,
`hibi status` reports exactly which claims need a new check because the
statement or its evidence changed, and `hibi cover` reports which parts of a
declared scope have no claim at all.

hibi is a single Rust binary. Its store is plain text under `.hibi/`, committed
with the code, so the memory follows the branch you are on.

## What hibi is and is not

- **It is memory.** hibi records what was checked, against which evidence, and
  what that evidence looked like. It reports what changed since.
- **It does not decide what is true.** The agent re-checks a statement with its
  own judgement and asks the user when it is unsure. Any re-check by anyone
  clears a claim.
- **It is not a gate.** Every command exits 0 when it ran, whatever it found,
  and 1 only on an error. There is no CI mode and no GitHub Action.
- **It is deterministic.** Freshness is exact fingerprint equality. A
  similarity score only proposes where a moved unit may have gone. It never
  resolves an anchor and never decides that nothing changed.
- **It stays offline.** Only the optional judge commands (`triage`,
  `suggest-evidence` and `status --judge-order`) use the network, and only
  when `TYPESAFE_API_KEY` is set. See [the judge](https://hibi.mintlify.app/judge).

## Install

No release of this version has been published yet. The latest release on
GitHub, 0.7.0, is the earlier TypeScript line and does not contain these
files. The commands below become valid when version 5.0.0 is released.

macOS and Linux, with the installer that cargo-dist builds:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/npupko/hibi/releases/download/v5.0.0/hibi-cli-installer.sh | sh
```

Windows, in PowerShell:

```bash
powershell -ExecutionPolicy Bypass -c "irm https://github.com/npupko/hibi/releases/download/v5.0.0/hibi-cli-installer.ps1 | iex"
```

Or with the repository's own script, which downloads the release tarball for
your platform, checks its SHA-256 sum and copies `hibi` into
`$HOME/.local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/npupko/hibi/main/scripts/install.sh | HIBI_VERSION=5.0.0 sh
```

From source, with Rust 1.90 or later, in a clone of this repository:

```bash
cargo install --locked --path crates/hibi-cli
```

hibi needs git 2.45.0 or later for `init`, `cover`, `--since`, sibling
repositories and the judge commands. Everything else works without git.

## Five minutes with hibi

A repository has a document and the code it describes:

```markdown docs/retry.md
# Retries

The client retries a failed download.
It gives up after 5 attempts.
The delay between attempts doubles each time, starting at 200 ms.
```

```rust src/retry.rs
/// How often a download is tried before the client gives up.
pub const MAX_ATTEMPTS: u32 = 5;

/// The delay before attempt `n`, in milliseconds.
pub fn delay_ms(n: u32) -> u64 {
    200 * 2u64.pow(n)
}
```

Create the store at the top of the git repository:

```bash
hibi init
```

`init` writes only under `.hibi/`: `hibi.toml`, three dot files and 32 empty
shard files. Declare which units must be covered by appending a scope to
`.hibi/hibi.toml`:

```toml
[[scope]]
name = "docs"
resource = "file:docs/**/*.md"
kinds = ["sentence", "list-item", "table-row"]
```

Ask what has no claim yet:

```bash
hibi cover
```

```text output
docs (file:docs/**/*.md; sentence, list-item, table-row): 3 units, 0 covered, 0 excluded, 3 uncovered
  file:docs/retry.md
        3  sentence  The client retries a failed download.
        4  sentence  It gives up after 5 attempts.
        5  sentence  The delay between attempts doubles each time, starting at 200 ms.
```

Read the code, confirm that the second sentence is true, and record the check.
The subject is the sentence; the evidence is the constant:

```bash
hibi claim "The client gives up after 5 attempts." \
  --subject "file:docs/retry.md::gives up after 5 attempts" \
  --evidence "file:src/retry.rs#MAX_ATTEMPTS"
```

```text output
c1-c5dm2jvhr8hb36rx5rgm  created  m1-1nhmgpeagp6bbwt6kh0j  subject file:docs/retry.md (sentence "It gives up after 5 attempts.", line 4); evidence 1 file:src/retry.rs (variable "MAX_ATTEMPTS", line 2)
```

Now someone changes `MAX_ATTEMPTS` to 3. In the next session the agent asks
what needs a new check:

```bash
hibi status
```

```text output
1 claim: 0 fresh, 1 stale, 0 expired, 0 retired

c1-c5dm2jvhr8hb36rx5rgm  stale  The client gives up after 5 attempts.
  marked 2026-11-22T22:31:29Z (m1-1nhmgpeagp6bbwt6kh0j)
  evidence 1  file:src/retry.rs  changed (fingerprint_differs)  variable "MAX_ATTEMPTS", line 2
    | pub const MAX_ATTEMPTS: u32 = 3;
  next: hibi mark c1-c5dm2jvhr8hb36rx5rgm

Re-check each statement before running its next command. If it no longer holds, fix the text or the code, or retire the claim with hibi retire.
```

`next` assumes the statement still holds. It does not: the code says 3. The
agent fixes the sentence to "It gives up after 3 attempts." and replaces the
claim with one that states the new fact. An id can be shortened to a unique
prefix:

```bash
hibi reanchor c1-c5dm \
  --statement "The client gives up after 3 attempts." \
  --subject "file:docs/retry.md::gives up after 3 attempts"
```

```text output
c1-c5dm2jvhr8hb36rx5rgm is retired and replaced by the new claim c1-2kfx78rwbb7wyxwst92f
  mark m1-qnnn2w60hevd1h14q3ek, retirement r1-fmpt7hwtdzded271kw2d
  was stale: subject file:docs/retry.md missing (not_exact); evidence 1 file:src/retry.rs changed (fingerprint_differs); now marked: make sure you re-checked them
    subject is now:
    | It gives up after 3 attempts.
    evidence 1 is now:
    | pub const MAX_ATTEMPTS: u32 = 3;
  subject file:docs/retry.md (sentence "It gives up after 3 attempts.", line 4)
    {"loc":"file:docs/retry.md","sel":{"quote":"gives up after 3 attempts","prefix":"It","suffix":".","within":["Retries","The client retries a failed download."]}}
  evidence 1 file:src/retry.rs (variable "MAX_ATTEMPTS", line 2)
    {"loc":"file:src/retry.rs","sel":{"symbol":"MAX_ATTEMPTS"}}
```

```bash
hibi status
```

```text output
2 claims: 1 fresh, 0 stale, 0 expired, 1 retired
nothing stale
```

Commit `.hibi/` with the change. The next session starts from this memory.

## Using hibi from a coding agent

The Claude Code plugin in [`plugins/hibi-cli`](plugins/hibi-cli) puts the
hibi skill in the agent's context. It does not install the binary:

```text
/plugin marketplace add npupko/hibi
/plugin install hibi-cli@hibi
```

With the skill, the agent runs `hibi status` at the start of a session. It
re-checks each listed statement, then marks or re-anchors the claim, or
retires it after asking the user, and records each new check with
`hibi claim`. When it cannot tell
whether the text or the code is right, it asks the user and does not mark.
Before finishing a branch it runs `hibi status --since main...`,
`hibi cover --since main...` and plain `hibi status`, and commits `.hibi/`.

In an agent's shell tool standard output is not a terminal, so each command
prints one JSON object instead, with the same facts and a `next` command per
claim. The exceptions are `hibi completions`, which prints a
shell script, `hibi schema <name>`, which prints the schema itself, and
`--help`, which prints text.

## The model in one screen

| Term | Meaning |
|---|---|
| Resource | Anything addressable by a locator: `file:docs/retry.md`, `repo:api//src/lib.rs` for a sibling repository, or `<scheme>:<name>` for an external provider. |
| Unit | One part of a resource: a sentence, list item, table row, heading or fenced block of prose; a function, method, class, struct, field, variant or other declaration of code; a block of plain text; the whole file. |
| Anchor | A locator plus a selector (a quote, a symbol, or a key) that finds one unit again after edits. |
| Claim | A statement, one subject anchor, zero or more evidence anchors, an optional recheck period and an optional note. Immutable: changing any of them makes a new claim. |
| Mark | A record that the claim was checked: the fingerprint of every side as it was then, the time, and optionally who. Append-only. |
| Retirement | A permanent record that a claim no longer applies, optionally naming the claim that replaces it. |
| Scope | A declaration in `.hibi/hibi.toml` that every unit of given kinds in matching resources must be covered by a claim. |

The core knows nothing about documents. A provider turns a locator into
units, their text and their fingerprints. The built-in file provider serves
`file:` and `repo:` locators: prose, plain text, and code in TypeScript,
TSX, Python, Rust, Go and Java. Any other tool can serve its own units,
as subjects or as evidence, through an external provider, a command that speaks a JSON-lines protocol over standard
input and output. Each user registers that command in their own config;
hibi never runs a command that a repository names.
[`hibi-provider-reference`](crates/hibi-provider-reference) is the reference
provider; see [external providers](https://hibi.mintlify.app/providers/external).

A claim is **fresh** when some mark's fingerprints equal the current
fingerprint of every side, and its recheck period, if any, has not run out. It
is **stale** when no mark matches, **expired** when a mark matches but the
period ran out, and **retired** when it was retired. Every side must resolve
exactly. A stored anchor finds its unit by name (`#MAX_ATTEMPTS`, the
declaration with that name) or by place (the quoted words, in their
section and paragraph, with the text right around them unchanged). When the
quoted words of a sentence are edited, the side is missing, and
`hibi status` lists the units it could mean as candidates. You read them,
re-check the statement, and re-anchor the claim with `hibi reanchor`. An
edit to a neighbouring sentence usually leaves the claim fresh; the
[limits](https://hibi.mintlify.app/concepts/anchors#limits) list the
exceptions.

## Learn more

- Documentation: <https://hibi.mintlify.app>
- Claude Code plugin with the hibi skill: [`plugins/hibi-cli`](plugins/hibi-cli)
- Command reference, generated from the code: [`cli-reference.md`](plugins/hibi-cli/skills/hibi/references/cli-reference.md)
- Specification: [`design/SPEC.md`](design/SPEC.md)
- Contributing: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Security policy: [`SECURITY.md`](SECURITY.md)

## License

MIT. See [LICENSE](LICENSE).
