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
  similarity score is used only to find a moved unit, never to decide that
  nothing changed.
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
c1-bvy8mfb476vwm9ftpx44  created  subject file:docs/retry.md (sentence "It gives up after 5 attempts.", line 4); evidence 1 file:src/retry.rs (variable "MAX_ATTEMPTS", line 2)
```

Now someone changes `MAX_ATTEMPTS` to 3. In the next session the agent asks
what needs a new check:

```bash
hibi status
```

```text output
1 claim: 0 fresh, 1 stale, 0 expired, 0 retired

c1-bvy8mfb476vwm9ftpx44  stale  The client gives up after 5 attempts.
  evidence 1  file:src/retry.rs  changed (fingerprint_differs)  variable "MAX_ATTEMPTS", line 2
  next: hibi mark c1-bvy8mfb476vwm9ftpx44

Re-check each statement before running its next command.
```

`next` assumes the statement still holds. It does not: the code says 3. The
agent fixes the sentence to "It gives up after 3 attempts." and replaces the
claim with one that states the new fact. An id can be shortened to a unique
prefix:

```bash
hibi reanchor c1-bvy8 \
  --statement "The client gives up after 3 attempts." \
  --subject "file:docs/retry.md::gives up after 3 attempts"
```

```text output
c1-bvy8mfb476vwm9ftpx44 is retired and replaced by c1-dryhtmnz5njjpp2ppy6t
  mark m1-jqkfdpc9r6xa87gh92gs, retirement r1-8drhdychv0qew0t6s13j
  subject  {"loc":"file:docs/retry.md","sel":{"quote":"gives up after 3 attempts","prefix":"tries\n\nThe client retries a failed download.\nIt ","suffix":".\nThe delay between attempts doubles each time, ","pos":52}}
  evidence 1  {"loc":"file:src/retry.rs","sel":{"quote":"pub const MAX_ATTEMPTS: u32 = 3;","prefix":"a download is tried before the client gives up.\n","suffix":"\n\n/// The delay before attempt `n`, in milliseco","pos":62,"symbol":"MAX_ATTEMPTS"}}
```

```bash
hibi status
```

```text output
2 claims: 1 fresh, 0 stale, 0 expired, 1 retired
nothing stale
```

Commit `.hibi/` with the change. The next session starts from this memory.

When standard output is not a terminal, as in an agent's shell tool, every
command prints one JSON object instead, with the same facts and a `next`
command per claim.

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

A claim is **fresh** when some mark's fingerprints equal the current
fingerprint of every side, and its recheck period, if any, has not run out. It
is **stale** when no mark matches, **expired** when a mark matches but the
period ran out, and **retired** when it was retired.

## Learn more

- Documentation: <https://hibi.mintlify.app>
- Claude Code plugin with the hibi skill: [`plugins/hibi-cli`](plugins/hibi-cli)
- Command reference, generated from the code: [`cli-reference.md`](plugins/hibi-cli/skills/hibi/references/cli-reference.md)
- Specification: [`design/SPEC.md`](design/SPEC.md)
- Contributing: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Security policy: [`SECURITY.md`](SECURITY.md)

## License

MIT. See [LICENSE](LICENSE).
