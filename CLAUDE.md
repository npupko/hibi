# CLAUDE.md

hibi is a Rust workspace: `crates/hibi` (the library, where every behaviour
lives), `crates/hibi-cli` (the `hibi` binary: argument parsing, output and
exit only) and `crates/hibi-provider-reference` (the reference external
provider). `design/SPEC.md` is the contract. Where code and spec disagree, one
of them is wrong; say which in the same change.

## Build and test

These gates must pass before a commit. They are the list in CONTRIBUTING.md,
which explains them; keep the two lists the same.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p hibi --no-default-features --all-targets -- -D warnings
cargo test --workspace
cargo build -p hibi --no-default-features
cargo test -p hibi --no-default-features
for f in file git-store command cache judge; do
  cargo build -p hibi --no-default-features --features "$f"
  cargo test -p hibi --no-default-features --features "$f"
done
```

`cases/` holds data cases for the pure algorithms; `cargo test` runs them.
Do not change their expected values to make a test pass.

## Generated files

Never edit these by hand. Regenerate and commit them with the change:

```bash
cargo run -p hibi-cli --example write-schemas
cargo run -p hibi-cli --example cli-reference
```

They write `schemas/*.schema.json` and
`plugins/hibi-cli/skills/hibi/references/cli-reference.md`. Tests fail when
either differs from the code.

## Documentation

`README.md`, `docs/*.mdx` (the Mintlify site, configured in `docs/docs.json`)
and `plugins/hibi-cli/` are the user documentation.

- Every example output must be pasted from a real run of the binary. Run
  commands in a scratch git repository, not in this one.
- Every `hibi ...` line in a fenced code block, and every inline code span
  that starts with `hibi `, must parse with the clap definition.
  `crates/hibi-cli/tests/doc_commands.rs` checks this (R43). A block that
  shows output rather than commands is marked with the word `output` after
  the language, for example ```` ```text output ````. In an inline span a
  placeholder such as `<id>` is read as a word, and a span of only `hibi`
  and a subcommand names that command. A span that is not a command line is
  written with double backticks, which render the same and are not checked.
- Check the site with the Mintlify CLI, pinned because `mint@latest` failed
  to install on 2026-11-21. Copy `docs/` to a scratch directory and run there:

  ```bash
  npx --yes mint@4.3.80 validate
  npx --yes mint@4.3.80 broken-links
  ```

- The words listed in the spec's leftover rule (R42) do not appear outside
  `design/`, `cases/`, `CHANGELOG.md` and the generated release workflow.

## Using hibi on this repository

Build the binary with `cargo build -p hibi-cli` and run `target/debug/hibi`.
When the repository has a `.hibi/` store, run `hibi status` at the start of a
session and `hibi status --since main...` before finishing, and follow the
skill in `plugins/hibi-cli/skills/hibi/SKILL.md`: re-check each listed claim,
then mark, reanchor or retire it, and ask the user when you cannot tell
whether the docs or the code are right. Commit `.hibi/` with the change.

## Commits

Conventional Commits (`feat`, `fix`, `docs`, `test`, `refactor`, `build`,
`ci`, `chore`). Releases and `CHANGELOG.md` are generated from them.
