# Changelog

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
