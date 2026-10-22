# Contributing to Hibi

Hibi (日々) is an agent-facing CLI that tracks checkable sentences in documents and reports which ones are stale. Contributions of any size are welcome, from a typo fix to a new judge fixture.

If anything here is unclear or out of date, open an issue.

## Prerequisites

- **[Bun](https://bun.sh) 1.0 or newer.** The test suite, build, and tooling run on it. Node.js is not needed.
- No compiler. The tree-sitter grammars are the official prebuilt WebAssembly files; `bun run build:grammars` copies them into `grammars/`.
- No API key. The tests use a fake judge. `TYPESAFE_API_KEY` is only needed to try the real judge by hand.

## Getting started

```sh
git clone https://github.com/npupko/hibi.git
cd hibi

bun install              # install dependencies
bun run build:grammars   # copy the official tree-sitter wasm into grammars/
bunx lefthook install    # one-time: install the pre-commit hook
```

Bun does not run the `prepare` lifecycle script, so the pre-commit hook is not wired up by `bun install` alone. Run `bunx lefthook install` once after cloning. The hook runs Biome on staged files plus a typecheck.

`bun run build:grammars` is required before the tests pass: the analyzer imports the wasm files from `grammars/` and embeds them in the build.

## Development

```sh
bun test            # run the full test suite
bunx tsc --noEmit   # typecheck
bun run lint        # Biome lint
bun run format      # Biome format (writes changes in place)
bun run check:biome # Biome check (lint + format, read-only)
bun run cli -- …    # run the in-tree CLI (bun run src/cli/index.ts …)
bun run build       # grammars, schemas, CLI reference, then dist/hibi
```

CI runs `biome ci .`, so keep `bun run check:biome` clean locally.

## Generated files

Three artifacts are generated. Do not hand-edit them; regenerate and commit them with the change that affects them.

| Source | Generated | Command |
|---|---|---|
| `src/core/model.ts` (Zod) | `schemas/*.v4.json` | `bun run build:schemas` |
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

Writing rules for the docs and the skill: plain prose, short sentences, no metaphor. Use `state` for axis values, `enforced` (not `confirmed`), `retire` for claims and `archive` for documents, `stranded` for a live claim on a superseded or archived document, `suggested` for the advisory enforcement, and the actor forms `human:`, `agent:`, `process:`. Every flag and JSON key in the docs must exist in the code.

## The judge in tests

No test calls TypeSafe. Two layers stand in for it:

- `FakeJudge` (`src/judge/fake.ts`) answers from a script. Its answers use the Jev wire shape and are decoded by the same mappers as the real adapter, so a cache entry it writes reads back exactly like one from Jev.
- `test/fixtures/judge/` holds, per judgment, the exact request body the Jev adapter builds (`<judgment>.request.json`) and a response in the documented shape (`<judgment>.response.json`). `test/judge-jev.test.ts` fails when a built request differs from its fixture. A change to `src/judge/questions/*.json` or to request building must update these files on purpose; it also changes cache keys, so every cached verdict for that judgment is asked again.

The fixture responses are synthetic; see `test/fixtures/judge/README.md`. `test/e2e.test.ts` runs the whole CLI loop against a local server that answers like System One, with `TYPESAFE_BASE_URL` pointing at it.

To try the real judge, set `TYPESAFE_API_KEY` and run the in-tree CLI in a scratch repository (`bun run src/cli/index.ts audit --doc README.md`, `check`, `evidence`). Recorded responses may replace the synthetic fixtures; say so in the PR.

## Commit conventions

Hibi uses [Conventional Commits](https://www.conventionalcommits.org/). Releases are automated from commit messages via release-please, so the convention is required.

Common prefixes: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `build:`, `ci:`, `test:`. Use `feat!:` or a `BREAKING CHANGE:` footer for breaking changes.

```text
feat: add --fail-on warn threshold to check
fix: report pending when the judge returns 401
docs: clarify exit codes in the README
```

## Pull requests

Before opening a PR:

- [ ] `bun test` passes
- [ ] `bunx tsc --noEmit` is clean
- [ ] `bun run lint` is clean
- [ ] `bun run build:schemas` was run if you touched `src/core/model.ts`
- [ ] `bun run build:cli-reference` was run if you touched `src/cli/options.ts`
- [ ] the PR title follows Conventional Commits (it becomes the squashed commit and feeds release-please)

Keep PRs to one logical change. Describe what changed and why; link any related issue.

## Code of Conduct

By participating in this project you agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Do not report security vulnerabilities through public issues or PRs. See [SECURITY.md](SECURITY.md) for how to report them privately.

## License

By contributing to Hibi, you agree that your contributions will be licensed under the [MIT License](LICENSE).
