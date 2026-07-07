# Changelog

## [0.3.0](https://github.com/npupko/hibi/compare/v0.2.3...v0.3.0) (2026-06-26)


### ⚠ BREAKING CHANGES

* replace `suggest` with deterministic `coverage` + agent grounding audit ([#31](https://github.com/npupko/hibi/issues/31))

### Features

* replace `suggest` with deterministic `coverage` + agent grounding audit ([#31](https://github.com/npupko/hibi/issues/31)) ([bb535c7](https://github.com/npupko/hibi/commit/bb535c70a065c031f2f8242cfeaa2a683051948e))


### Reverts

* **docs:** restore Hibi brand theme, logo, and favicon ([#29](https://github.com/npupko/hibi/issues/29)) ([c6f8f7e](https://github.com/npupko/hibi/commit/c6f8f7e6e5832c64f07533772e14ec13a238cfb2))

## [0.2.3](https://github.com/npupko/hibi/compare/v0.2.2...v0.2.3) (2026-06-25)


### Features

* harden against silent-orphan failures ([#27](https://github.com/npupko/hibi/issues/27)) ([251582f](https://github.com/npupko/hibi/commit/251582fb45e1ddb32b9e87dd3e3d7a37758c0f1c))

## [0.2.2](https://github.com/npupko/hibi/compare/v0.2.1...v0.2.2) (2026-06-23)


### Features

* **cli:** cross-file claim relocation (reanchor --doc) + batch record --from-file ([5608dc0](https://github.com/npupko/hibi/commit/5608dc011f11845fcc75eaba14849e26be0384cb))
* **cli:** relocate claims across files (reanchor --doc) + batch record --from-file ([975c68d](https://github.com/npupko/hibi/commit/975c68d521d40f2867e0fcd77645ba17ada5267d))


### Bug Fixes

* **cli:** harden doc relocation and batch record per review ([09602d4](https://github.com/npupko/hibi/commit/09602d49980780322fd1199c3599a1b6dce2160f))

## [0.2.1](https://github.com/npupko/hibi/compare/v0.2.0...v0.2.1) (2026-06-22)


### Features

* agent-optimized JSON, remediation menu, retire/list, and a scenario-led skill ([481ac27](https://github.com/npupko/hibi/commit/481ac2717c0d8a4bd4a72da7be23bf0bc801bff4))
* **cli:** lean decision-first JSON, --explain/--no-hints, retire + list ([ca4abd6](https://github.com/npupko/hibi/commit/ca4abd645fd44fb08de147db3283d263223af2f5))
* **cli:** TTY-aware human output, repo-wide status overview, completions ([0773036](https://github.com/npupko/hibi/commit/0773036f19b43ac8535196e92466cbbb43b676e5))
* **core:** deterministic remediation menu, retire + list engines ([91bcf6e](https://github.com/npupko/hibi/commit/91bcf6e7bb33166f964a30334bae033a0c240c79))


### Bug Fixes

* **docs:** use transparent cream wordmark for dark-mode logo ([ec1fa93](https://github.com/npupko/hibi/commit/ec1fa9386d7d9881e5e255a28e46e07f4cd10332))

## [0.2.0](https://github.com/npupko/hibi/compare/v0.1.2...v0.2.0) (2026-06-19)


### ⚠ BREAKING CHANGES

* the computed-state enum, the unidirectional Anchor, the Proposition.text field, the single-axis Verdict, and the old --fail-on/--text CLI surface are all replaced with no compatibility shim (greenfield, per ADR-001).

### Features

* rewrite the engine to the two-axis state model (ADR-001) ([3775cc1](https://github.com/npupko/hibi/commit/3775cc145ed37cb4d42e0034d245ff811380bb6c))


### Bug Fixes

* **action:** correct the fail-on strictness vocabulary ([ba53b76](https://github.com/npupko/hibi/commit/ba53b76b0decc6d33399ccdbdbddba5c29bf5a11))
* **sdk:** update Rust echo example to the two-axis verdict shape ([d3cdb49](https://github.com/npupko/hibi/commit/d3cdb494781a4c5ef01d2817cda67fd44f34852a))

## [0.1.2](https://github.com/npupko/hibi/compare/v0.1.1...v0.1.2) (2026-06-18)


### Features

* **lib:** add in-process library facade ([afb70bc](https://github.com/npupko/hibi/commit/afb70bc6e9f2e0f5c0ddc58f917ed4bc33eaf806))


### Bug Fixes

* **ast:** load web-tree-sitter runtime wasm by its 0.26 filename ([f566a0d](https://github.com/npupko/hibi/commit/f566a0d9893904696cd21b5c9f626ccf80d7c0f3))
* **check:** honor --fail-on never and tamper in exit code ([1b78c10](https://github.com/npupko/hibi/commit/1b78c104ff13a7fc162959924e644309c9ad65ba))
* **check:** honor --fail-on never and tamper in exit code ([0593091](https://github.com/npupko/hibi/commit/0593091b66c666272b0346d892afc196555f1f28)), closes [#9](https://github.com/npupko/hibi/issues/9)

## [0.1.1](https://github.com/npupko/hibi/compare/v0.1.0...v0.1.1) (2026-06-18)


### Features

* **brand:** adopt Hibi wordmark logo + production asset kit ([9ca56dd](https://github.com/npupko/hibi/commit/9ca56ddb91550f0e64ddb5ddecbe1fe5671be2ca))
* initial claim-engine project structure ([e26a131](https://github.com/npupko/hibi/commit/e26a131bfcdbaec54782d1ce38f5aae44513639b))


### Bug Fixes

* **action:** run scoped @npupko/hibi package in bunx fallback ([6aa75b9](https://github.com/npupko/hibi/commit/6aa75b968e5971f9cc05679a21e222678287ad26))
* **ci:** publish to npm with --ignore-scripts ([8e6bcf5](https://github.com/npupko/hibi/commit/8e6bcf570ff45a5e9ea3e3887f71971ed35f841e))
* **cli:** derive version from package.json instead of hardcoding ([80b146a](https://github.com/npupko/hibi/commit/80b146a03bdb361b655f66f659e1b8dcc06eb8f0))
* **install:** verify downloaded binary against SHA256SUMS.txt ([2445981](https://github.com/npupko/hibi/commit/244598116fb05289fff816dbee5bf8e356cf5caa))
* **npm:** drop dangling module field and ship the TS SDK ([f8d37b0](https://github.com/npupko/hibi/commit/f8d37b0ae1206855429d652f9988f58e7bf22060))
