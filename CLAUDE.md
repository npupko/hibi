# CLAUDE.md

hibi is a Rust workspace: `crates/hibi` (the library, where every behaviour
lives), `crates/hibi-cli` (the `hibi` binary: argument parsing, output and
exit only) and `crates/hibi-provider-reference` (the reference external
provider). `design/SPEC.md` is the contract. Where code and spec disagree, one
of them is wrong; say which in the same change.

## Build and test

These five gates must pass before a commit (CONTRIBUTING.md explains them):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p hibi --no-default-features
cargo test -p hibi --no-default-features
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
- Every `hibi ...` line in a fenced code block must parse with the clap
  definition. `crates/hibi-cli/tests/doc_commands.rs` checks this (R43). A
  block that shows output rather than commands is marked with the word
  `output` after the language, for example ```` ```text output ````.
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
