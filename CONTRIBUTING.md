# Contributing to Hibi

Thanks for your interest in Hibi (日々)! Hibi is a deterministic, agent-facing
CLI that catches documentation drift by tracking *claims* anchored to code.
It's small, focused, and built to stay that way — so contributions of any size,
from a typo fix to a new resolver, are genuinely welcome.

This guide covers everything you need to get a change merged. If anything here
is unclear or out of date, open an issue — that's a contribution too.

## Prerequisites

- **[Bun](https://bun.sh) ≥ 1.0** — Hibi targets the Bun runtime, and the test
  suite, build, and tooling all run on it. You do not need Node.js.
- A C-free toolchain for the core CLI: the tree-sitter grammars ship as
  prebuilt WebAssembly, so there's nothing to compile for everyday work.
- **Rust + Cargo** only if you plan to touch the Rust SDK (see below).

## Getting started

```sh
git clone https://github.com/npupko/hibi.git
cd hibi

bun install              # install dependencies
bun run build:grammars   # copy the official tree-sitter wasm into grammars/
bunx lefthook install    # one-time: install the pre-commit hook
```

> **Why `bunx lefthook install`?** Bun does **not** automatically run the
> `prepare` lifecycle script, so the pre-commit hook isn't wired up by
> `bun install` alone. Run this once after cloning. The hook runs Biome on your
> staged files plus a typecheck, catching most CI failures before you push.

`bun run build:grammars` is required before the tests will pass, so don't skip it.

## Development

```sh
bun test            # run the full test suite
bunx tsc --noEmit   # typecheck
bun run lint        # Biome lint
bun run format      # Biome format (writes changes in place)
bun run check:biome # Biome check (lint + format, read-only)
```

CI runs `biome ci .`, so keeping `bun run check:biome` clean locally keeps you
in sync with the pipeline.

## Editing the data model

The data model lives in exactly one place: the Zod schema in
[`src/core/model.ts`](src/core/model.ts). The JSON Schemas in `schemas/` and the
SDK types are **generated** from it — never hand-edit them.

After any change to `src/core/model.ts`, regenerate and commit the schemas:

```sh
bun run build:schemas
```

CI fails if `schemas/` is stale, so regenerating is not optional. Commit the
updated `schemas/*.json` alongside your model change.

## Commit conventions

Hibi uses **[Conventional Commits](https://www.conventionalcommits.org/)**, and
releases are automated from commit messages via release-please. Commits that
don't follow the convention won't be released correctly, so this is required.

Common prefixes: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `build:`,
`ci:`, `test:`. Use `feat!:` or a `BREAKING CHANGE:` footer for breaking changes.

Examples:

```text
feat: add --fail-on tamper threshold to check
fix: re-anchor moved claims when the enclosing node is renamed
docs: clarify exit codes in the README
```

## Pull requests

Before opening a PR, make sure:

- [ ] `bun test` passes
- [ ] `bunx tsc --noEmit` is clean
- [ ] `bun run lint` is clean
- [ ] schemas are regenerated (`bun run build:schemas`) if you touched `src/core/model.ts`
- [ ] your **PR title** follows Conventional Commits (it becomes the squashed commit and feeds release-please)

Keep PRs focused — one logical change per PR makes review fast and keeps the
release history readable. Describe what changed and why; link any related issue.

## The Rust SDK

The Rust SDK lives in [`sdk/rust`](sdk/rust) and builds independently of the
core CLI:

```sh
cd sdk/rust
cargo build --examples
```

If your change affects the wire protocol or the data model, update both the
TypeScript SDK (`sdk/ts`) and the Rust SDK so they stay in lockstep.

## Code of Conduct

By participating in this project you agree to abide by our
[Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Please do **not** report security vulnerabilities through public issues or PRs.
See [SECURITY.md](SECURITY.md) for how to report them privately.

## License

By contributing to Hibi, you agree that your contributions will be licensed
under the [MIT License](LICENSE), the same license that covers the project.
