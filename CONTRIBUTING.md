# Contributing to Hibi

Hibi (日々) is a deterministic, agent-facing CLI that catches documentation drift by tracking claims anchored to code. Contributions of any size are welcome, from a typo fix to a new resolver.

If anything here is unclear or out of date, open an issue.

## Prerequisites

- **[Bun](https://bun.sh) 1.0 or newer.** The test suite, build, and tooling run on it. Node.js is not needed.
- No compiler. The tree-sitter grammars ship as prebuilt WebAssembly.

## Getting started

```sh
git clone https://github.com/npupko/hibi.git
cd hibi

bun install              # install dependencies
bun run build:grammars   # copy the official tree-sitter wasm into grammars/
bunx lefthook install    # one-time: install the pre-commit hook
```

Bun does not run the `prepare` lifecycle script, so the pre-commit hook is not wired up by `bun install` alone. Run `bunx lefthook install` once after cloning. The hook runs Biome on staged files plus a typecheck.

`bun run build:grammars` is required before the tests pass.

## Development

```sh
bun test            # run the full test suite
bunx tsc --noEmit   # typecheck
bun run lint        # Biome lint
bun run format      # Biome format (writes changes in place)
bun run check:biome # Biome check (lint + format, read-only)
bun run cli -- …    # run the in-tree CLI (bun run src/cli/index.ts …)
```

CI runs `biome ci .`, so keep `bun run check:biome` clean locally.

## Generated files

Three artifacts are generated. Do not hand-edit them; regenerate and commit them with the change that affects them.

| Source | Generated | Command |
|---|---|---|
| `src/core/model.ts` (Zod) and `src/resolver/protocol.ts` | `schemas/*.v3.json` | `bun run build:schemas` |
| `src/cli/options.ts` (the per-command option tables) | `docs/cli-reference.mdx` and `plugins/hibi-cli/skills/hibi/references/cli-reference.md` | `bun run build:cli-reference` |
| `src/cli/options.ts` | shell completions (`hibi completions <shell>`) | at runtime |

CI fails if a generated file is out of date.

## Documentation

The docs site lives in [`docs/`](docs) and is built with [Mintlify](https://mintlify.com). Pages are `.mdx`; [`docs/docs.json`](docs/docs.json) holds the navigation.

Preview locally from inside `docs/`:

```sh
npm i -g mint          # the Mintlify CLI (requires Node 20.17 or newer; the only Node dependency in this repo)
cd docs
mint dev               # live preview at http://localhost:3000
mint broken-links      # validate internal links before pushing
```

The published site deploys through the Mintlify GitHub App on a push to the default branch. The dashboard's content directory is set to `docs`.

When adding a page, create the `.mdx` file under `docs/` and add its slug to a group in `docs/docs.json` under `navigation.groups`. Do not name a navigation entry `api`; Mintlify reserves that path.

Writing rules for the docs and the skill: plain prose, short sentences, no metaphor. Use `state` for axis values, `enforced` (not `confirmed`), `retire` for claims and `archive` for documents, `stranded` for a live claim on a superseded or archived document, and `suggested` for the advisory enforcement.

## When `hibi check` fails in CI

Hibi dogfoods itself: this repo has a committed `.claims/` store binding sentences in `docs/` to the constants and enums in `src/` they describe (state vocabularies in `src/core/model.ts`, resolution constants in `src/algo/params.ts`, the gates rule in `src/core/gating.ts`). CI runs the in-tree engine, read-only:

```sh
bun run src/cli/index.ts check --fail-on gating
```

If your change edits one of those anchored constants or doc sentences, this step exits 2. That is the gate working. To fix it:

1. **See what drifted:** `bun run src/cli/index.ts check --doc <the-doc>` names the suspect claims and their remediation menu.
2. **Decide which side is right.** If the doc is the spec, fix the code. If the code is now correct, update the sentence in `docs/`.
3. **Reanchor the claim:** run the `command` from the menu, usually `bun run src/cli/index.ts reanchor <claim-id>`. Reanchor only after you have re-verified the sentence against the code.

Re-run `check` until it exits 0, then push. An obsolete claim is withdrawn with `bun run src/cli/index.ts retire <claim-id>`, not by deleting its `.claims/` file.

## Commit conventions

Hibi uses [Conventional Commits](https://www.conventionalcommits.org/). Releases are automated from commit messages via release-please, so the convention is required.

Common prefixes: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `build:`, `ci:`, `test:`. Use `feat!:` or a `BREAKING CHANGE:` footer for breaking changes.

```text
feat: add --fail-on warn threshold to check
fix: reanchor moved claims when the enclosing node is renamed
docs: clarify exit codes in the README
```

## Pull requests

Before opening a PR:

- [ ] `bun test` passes
- [ ] `bunx tsc --noEmit` is clean
- [ ] `bun run lint` is clean
- [ ] `bun run build:schemas` was run if you touched `src/core/model.ts` or `src/resolver/protocol.ts`
- [ ] `bun run build:cli-reference` was run if you touched `src/cli/options.ts`
- [ ] the PR title follows Conventional Commits (it becomes the squashed commit and feeds release-please)

Keep PRs to one logical change. Describe what changed and why; link any related issue.

## Resolvers

External resolvers use the `@npupko/hibi/resolver` export (`src/resolver/index.ts`). If your change affects the wire protocol (`src/resolver/protocol.ts`) or the data model, regenerate the schemas and update the echo resolver under `test/` that the protocol tests use.

## Code of Conduct

By participating in this project you agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Do not report security vulnerabilities through public issues or PRs. See [SECURITY.md](SECURITY.md) for how to report them privately.

## License

By contributing to Hibi, you agree that your contributions will be licensed under the [MIT License](LICENSE).
