# Contributing to hibi

hibi records that a statement was checked against evidence, and later reports
what changed since the check. It is a Rust workspace with one library crate
and two binaries. Contributions of any size are welcome.

If anything here is unclear or out of date, open an issue.

## Prerequisites

- **Rust 1.90 or newer**, installed with [rustup](https://rustup.rs). 1.90 is
  the minimum supported version (`rust-version` in `Cargo.toml`). CI checks
  it, and runs everything else on the latest stable release.
- **git 2.45.0 or newer.** hibi starts `git` as a child process for `init`,
  `cover`, `--since` and `repo:` locators, and the tests exercise those paths.
- A C compiler. The tree-sitter grammars and the bundled SQLite are compiled
  from C sources by their build scripts.
- No API key. The tests never call the judge service. `TYPESAFE_API_KEY` is
  only needed to try `triage`, `suggest-evidence` and `status --judge-order`
  by hand.

## Layout

| Path | Contents |
|---|---|
| `crates/hibi/` | The library: every behaviour of the specification lives here. |
| `crates/hibi-cli/` | The `hibi` binary: argument parsing, output rendering and exit status only. |
| `crates/hibi-provider-reference/` | The reference provider used by the provider conformance cases. |
| `design/SPEC.md` | The specification. Code, schemas, docs and case files derive from it. |
| `cases/` | Conformance data for the pure algorithms. |
| `schemas/` | JSON Schemas, generated. |

## The five gates

Every change must pass these locally before you open a pull request. CI runs
the same commands.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p hibi --no-default-features
cargo test -p hibi --no-default-features
```

The last two check that the library still builds and passes its tests with
its optional features (`file`, `git-store`, `command`, `cache`, `judge`)
turned off, the way an embedder can use it. They name the package with
`-p hibi` because `hibi-cli` turns every feature on, and a `--workspace`
build would turn them on for the library too.

## The specification

`design/SPEC.md` is the contract. Where the code and the specification
disagree, one of them is wrong, and the fix changes both in the same pull
request. A change in behaviour starts with a change to the specification.

## Conformance cases

The files under `cases/` are binding conformance data. Section 16 of the
specification names the function each file tests, and `cases/README.md`
describes the shape of every file and where its values came from.

- Do not edit a case to make a test pass. If an implementation disagrees with
  a case, the implementation is wrong, or the case is wrong and the
  specification says so. Raise the second kind in an issue or in the pull
  request description.
- Every case file must be loaded by a test. `cargo test` fails otherwise
  (requirement R1). The bindings are in `crates/hibi/tests/cases/main.rs`.

To add a case:

1. Add an object to the right file with the five keys `name`, `input`,
   `expected`, `source` and `origin`. `name` is unique within the file.
   `source` and `origin` say where `expected` came from, using the values
   listed in `cases/README.md`. Never write `expected` by running the code
   under test.
2. Put any fixture file under `cases/fixtures/` and refer to it by a path
   relative to `cases/`.
3. Run `cargo test --workspace`.

To add a case file, also list it in section 16 of the specification and in
`cases/README.md`, and bind it in the `bindings!` table of
`crates/hibi/tests/cases/main.rs`.

The tree-sitter grammar crates are pinned with `=` because they are part of
the `ast-v1` fingerprint (specification section 8.4). Changing one of them
can change fingerprints and the expected values of the case files, so it is
done by hand in its own pull request, never by an automatic update.

## Generated files

Two outputs are generated from the code by examples of `hibi-cli`. Do not edit
them by hand. Regenerate them and commit the result with the change that
affects them. CI fails when a committed file differs from the generator's
output.

| Generated | Command |
|---|---|
| `schemas/<name>.schema.json` | `cargo run -p hibi-cli --example write-schemas` |
| `plugins/hibi-cli/reference/cli.md` | `cargo run -p hibi-cli --example cli-reference` |

The examples are not shipped in the binary.

## Commit messages

hibi uses [Conventional Commits](https://www.conventionalcommits.org/).
Releases and the changelog are generated from commit messages, so the format
is required.

Common types: `feat`, `fix`, `docs`, `refactor`, `test`, `build`, `ci`,
`chore`. Mark a breaking change with `!` after the type, or with a
`BREAKING CHANGE:` footer.

```text
feat(cli): add --since to cover
fix(locate): keep the earliest tied range for an ambiguous quote
docs: describe the provider timeout
```

## Pull requests

Keep a pull request to one logical change. Describe what changed and why, and
link the related issue. The pull request title becomes the squashed commit
message, so it follows Conventional Commits too.

## Releases

All three crates share one version, `[workspace.package] version` in
`Cargo.toml`. Releases are automated, and nothing is published to crates.io.

1. On every push to `main`, [release-plz](https://release-plz.dev) opens or
   updates a release pull request. It sets the next version from the
   Conventional Commits since the last tag, updates `Cargo.toml` and
   `Cargo.lock`, and adds a section to `CHANGELOG.md`. The section lists the
   commits that touch any of the three crates. Configuration:
   `release-plz.toml`.
2. Merging that pull request makes release-plz push the tag `vX.Y.Z`.
3. The tag starts `.github/workflows/release.yml`.
   [cargo-dist](https://github.com/axodotdev/cargo-dist) builds the `hibi`
   binary for every target in `dist-workspace.toml`. It then creates the
   GitHub Release, with the changelog section as its notes, the release
   tarballs, checksums and the shell and PowerShell installers.

`.github/workflows/release.yml` is generated by cargo-dist from
`dist-workspace.toml`. Do not edit it by hand. After changing
`dist-workspace.toml` or the cargo-dist version, regenerate it with the
`dist` tool of the version in `cargo-dist-version`, and commit the result.
The release workflow fails on any pull request where the file is out of date.

```sh
dist generate          # rewrite the generated workflow
dist generate --check  # fail if it is out of date
```

Publishing the `hibi` library to crates.io is off. To turn it on, set
`git_only = false` for the `hibi` package in `release-plz.toml`, add a
`CARGO_REGISTRY_TOKEN` secret, and pass it to the release job. Publish the
first version by hand with `cargo publish -p hibi`.

## Code of Conduct

By taking part in this project you agree to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Do not report security vulnerabilities in public issues or pull requests.
[SECURITY.md](SECURITY.md) says how to report them privately.

## License

By contributing to hibi you agree that your contributions are licensed under
the [MIT License](LICENSE).
