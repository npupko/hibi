# A Deterministic Evidence Ledger for Git and Agents

## Executive Summary

- **Keep Providers Small and Explicit**: Git and editor protocols show that subprocess providers can be extended without linking them into the CLI, while capability negotiation and long-lived workers add lifecycle complexity [13] [31] -> Adopt a small versioned JSON-lines protocol over stdio, launched only from trusted configuration.
- **Make Records the Source of Truth**: Beads combines a Git-tracked JSONL export with an untracked SQLite database and a field-aware merge driver; git-bug demonstrates a more sophisticated operation-based CRDT [36] [36] [34] -> Store canonical one-record-per-file JSON as the durable ledger, and rebuild a local SQLite index.
- **Prefer Cryptographic Fingerprints for CI**: BLAKE3 and SHA-256 are cryptographic hash choices, while xxHash explicitly is not suitable for security-sensitive uses [20] [16] [18] -> Use a domain-separated cryptographic digest for evidence and judgment keys; reserve xxHash for disposable local indexing.
- **Separate Canonical JSON from Text Semantics**: RFC 8785 is a JSON canonicalization scheme, while Unicode normalization is a separate standard and policy decision [8] [3] -> Preserve original evidence text and name/version every normalization profile used to derive alternate fingerprints.
- **Treat Agent UX as a Stable API**: Anthropic recommends distinct, well-described tools, strict inputs, concise outputs, and evaluation; OpenAI Structured Outputs constrains successful non-refusal responses to a supplied JSON Schema [9] [9] [5] -> Provide JSON output, actionable errors, batch operations, and a generated schema command.
- **Do Not Confuse Prompt Caching with Judgment Caching**: OpenAI prompt caching reuses computation for matching prompt prefixes; it does not itself provide a durable application-level judgment cache [10] -> Maintain a separate cache keyed by the complete canonical request and pinned model identity.
- **Bundle Parsers, Not Arbitrary Repository Code**: Tree-sitter has Rust bindings and supports parsers/languages supplied to the parser, while Wasmtime's security goal is sandboxed execution of untrusted Wasm [19] [35] -> Bundle the grammars needed by the binary; do not auto-run provider commands found in a repository.
- **Be Honest About 2026 Maintenance Signals**: The inspected crate documentation gives a dated 2026 release signal for Ariadne, but most pages do not expose a reliable last-release date [40] [40] -> Pin versions in Cargo.lock and verify release, MSRV, and security status at adoption instead of inferring maintenance from a `latest` docs page.

## 1. Providers: JSON-RPC over stdio, with explicit trust

| Pattern | What the primary sources establish | Fit and tradeoff for this ledger |
|---|---|---|
| Git credential and remote helpers | Credential helpers are invoked as programs with an operation argument; the helper exchanges attributes on stdin/stdout. Remote helpers use a line-oriented command stream, begin with `capabilities`, and advertise supported commands [1] [1] [13] [13]. | Good models for a tiny external-process interface. Capability discovery is useful; shell execution of configured commands is a serious trust boundary. |
| Git LFS custom transfer agents | One-line JSON metadata is exchanged over stdin/stdout; the process initializes, then handles requests while running [2] [2]. | A strong model for a persistent provider and batched work. More protocol machinery than a one-shot read needs, but amortizes process startup. |
| LSP / JSON-RPC | LSP uses a framed message header, `Content-Length`, JSON-RPC messages, and initialization-time capability exchange; the client manages server startup and shutdown [31] [31] [31]. | Mature framing and negotiation for a process expected to stay alive. More complexity than necessary if the only operations are resolve, enumerate, and render. |
| MCP | Current stdio transport launches a subprocess and sends newline-delimited JSON-RPC; stdout is reserved for protocol messages and stderr can carry logs [38]. The current versioning scheme carries protocol version metadata per request and supports capability handling [37]. | Useful if interoperability with agent tool ecosystems is a product goal. It is a broader tool protocol, not a minimal local evidence-provider contract. Prior verified MCP HTTP requirements also add transport-specific complexity [49]. |
| Terraform plugin protocol | Terraform documents a versioned Protocol Buffers/gRPC interface; major versions separate compatibility and minor versions add functionality [11]. | Appropriate for a larger provider ecosystem with generated RPC interfaces and compatibility requirements; excessive for a local CLI/library with a few deterministic calls. |
| Nushell plugins | Executable plugins use a versioned protocol over stdio or local sockets, can remain alive while in use, and are eventually stopped by an idle garbage collector [12] [12] [12]. | Demonstrates a workable long-lived process and version matching. Version mismatch can leave a plugin installed but its commands failing, so upgrade behavior needs design [12]. |
| Buildkite and pre-commit hooks | Buildkite describes hooks as scripts executed or sourced at points in the agent lifecycle, rather than a general request/response resource protocol [17]. | Good for lifecycle automation, validation, and policy checks; poor fit for reusable provider methods with typed results and per-request errors. |
| Cargo subcommands | Cargo's extension pattern is executable discovery by a `cargo-<name>` command, rather than a shared provider API. | Simple installation and command composition; define a separate protocol if the ledger needs typed `resolve`/`enumerate` calls. |
| Wasm components, Extism, Zed extensions | Wasmtime presents sandboxed execution of untrusted Wasm as a design goal [35]. Extism and editor extensions are host/plugin systems rather than a universal ledger-provider protocol. | Strong isolation potential when host capabilities are tightly controlled, but adds a runtime, component ABI, packaging, and versioning work. A sandbox is not permission to expose filesystem, network, or secrets by default. |
| Helix / Tree-sitter grammars | Tree-sitter's Rust binding exposes parsers and languages as inputs to parsing [19]. | Grammar loading extends parsing and language support; it does not by itself solve locating resource text, enumerating evidence units, or rendering them. Bundle required grammars instead of treating grammar loading as the provider architecture. |

**Recommendation.** Define a protocol version and three typed operations: `resolve(locator) -> {text, fingerprint, media_type}`, `enumerate(locator) -> [unit]`, and `render(locator, unit_id, format) -> {text, fingerprint}`. Use one JSON object per line, a request ID, structured `result` or `error` responses, UTF-8, a maximum message size, and a documented stdout/stderr split. A `hello` request should return protocol version, provider name/version, and capabilities; unknown optional fields should be ignored, while unknown required capabilities should fail clearly. Include a `batch` request so CI and agents can amortize process startup.

Use one-shot child processes for infrequent calls and a persistent worker for batch-heavy workflows. The Git LFS and Nushell examples demonstrate persistent-process patterns; LFS also shows the need to specify initialization, sequencing, and shutdown [2] [12]. **Never execute a provider command just because it appears in repository configuration.** Git credential configuration ultimately invokes a configured command through a shell [1] [1]. Treat repository-defined commands as arbitrary code execution: require explicit trust approval, default to disabled, support an allowlist or user-level provider registry, pass structured arguments without a shell, set timeouts and output limits, and avoid inherited secrets. The JSON-RPC envelope can be deterministic; provider output is deterministic only if the provider and its inputs are controlled.

## 2. Git storage: independent records plus a rebuildable index

| Layout | Branch-merge behavior | Recommendation for tens of thousands of records |
|---|---|---|
| One JSON file per record | Independent additions and edits usually touch separate paths. A same-record concurrent edit still needs a semantic resolution rule. | Best default when records have stable IDs and history matters. Use a stable path derived from the record ID, not a mutable title or subject. |
| JSONL shards by subject | One record per line makes append-only additions easy to review, but same-shard edits and concurrent ordering can collide. | A reasonable compromise if file count or directory operations become a measured problem. Shard deterministically and avoid treating line-union as conflict resolution. |
| One large file | Every edit shares one merge surface; Git's line merge can resolve disjoint lines but not concurrent edits to the same region or reordered arrays. | Avoid for the canonical ledger at this scale. It increases conflict coupling and makes review noisier. |
| SQLite as the only committed store | Strong local querying and transactions, but opaque binary diffs make branch review and merges difficult. | Do not make it the Git source of truth unless the product adopts explicit database synchronization/merge semantics. |
| Canonical files plus derived SQLite | Text records remain reviewable; the index can be deleted and reconstructed. Beads documents JSONL as source of truth and its SQLite database as untracked [36]. | Recommended. Index by record ID, locator, fingerprint, status, and searchable text; invalidate or incrementally update by content digest. |

Git's built-in `union` merge driver includes lines from both branches but can leave added lines in random order; Git explicitly warns that the result should be verified [21]. A custom merge driver can receive base/current/other files and overwrite the current file with a result [21]. Use a custom ID-aware JSON merger only when you need field-level merges and are prepared to test conflict behavior; Beads reports matching records by identity, merging fields, and reserving conflict markers for unresolved conflicts [36].

The examples show a spectrum of approaches. Changesets stores distinct change descriptions as uniquely named files under `.changeset/` and later aggregates them [50]. Towncrier's fragment model is a useful precedent for per-change files, while Cargo and Bazel show why generated lock data can be committed when it captures exact reproducibility inputs: Cargo says `Cargo.lock` contains exact dependency information and should be committed for applications [24]; Bazel's `MODULE.bazel.lock` records module resolution and extension evaluation results [32]. DVC separates small Git metadata from a content-addressed cache, and Git-ignores the cache itself [48]. Terraform instead delegates state storage and optional locking to its backend [28] [28].

For conflict models beyond file-level merges, git-bug stores operations and derives current state by replay in deterministic order; divergent operation sets are unioned [34]. Jujutsu stores a logical conflict in commits and materializes markers on checkout, rather than forcing immediate resolution [15] [15]. These are powerful models but add complexity unnecessary for an initial ledger. Nx-style build caches are a cache analogy, not a model for editable evidence records; Bazel's lockfile is the stronger reproducibility analogue.

Serialize records deterministically: sorted object keys, fixed UTF-8, stable arrays where order is semantic, and explicit sorting by stable ID where it is not. RFC 8785 is the JSON Canonicalization Scheme; do not treat canonical JSON as an implicit text-normalization rule [8]. Keep the source record files authoritative and make SQLite rebuildable. At tens of thousands of small records, the index is worth it for filtering and joins, but it must never be required to reconstruct the committed ledger.

## 3. Fingerprints: cryptographic digests, versioned normalization

| Algorithm/profile | Appropriate role | CI/security assessment |
|---|---|---|
| BLAKE3 | Default content digest and derived stable IDs when the system controls the format. Its official project describes it as a cryptographic hash function [20]. | Good choice for fast hashing of many small records. Include a domain label and profile version in the bytes being hashed. |
| SHA-256 | Interoperability when another system or format already requires SHA-256; NIST lists SHA-256 in the SHA-2 family and describes collision, preimage, and second-preimage resistance as desired hash properties [16] [16]. | Good cryptographic choice for fingerprints that cross system boundaries. Do not change algorithms without an explicit digest-version migration. |
| xxHash | Fast, non-cryptographic hash for local lookup tables or disposable caches. Its maintainers explicitly say it is not a cryptographic hash and should not be used where resistance to attacks is required [18] [18]. | Do not let it decide that evidence is unchanged or let it gate CI acceptance. |

Store at least two notions of identity where useful: a raw-byte fingerprint of the fetched evidence and a normalized fingerprint for matching. A fingerprint used to gate CI should use a cryptographic digest over a versioned, unambiguous encoding, for example `ledger:text:v1\0` followed by canonical metadata and bytes. Store the algorithm and profile alongside the digest. A digest is a compact integrity check, not proof that the source is trustworthy or that the fetched text is accurate.

For JSON, use RFC 8785 JCS for canonical serialization and separately specify how the application treats strings [8]. Unicode normalization forms are defined separately in Unicode Standard Annex #15 [3]. Preserve original source text for evidence; optionally compute an NFC-normalized matching fingerprint as a distinct profile. Likewise, do not silently collapse all whitespace: line breaks, indentation, and spacing may be meaningful in quotations, code, or diffs. If a profile maps CRLF and CR to LF or trims terminal whitespace, name and version that transformation, and retain the original bytes for audit.

For code, maintain a raw digest and, optionally, an AST-derived digest. Pin the grammar, parser version, language mode, and normalization policy in the AST profile. A comment/whitespace-insensitive AST can be useful for matching, but it can also erase meaningful documentation, directives, or formatting-sensitive content. Therefore it should be a secondary equivalence signal, not the evidence's only identity. Do not use a fuzzy similarity score as a CI integrity gate; use it to propose candidate links for human or model review.

## 4. Rust stack: small core, bundled grammars, cautious maintenance claims

| Area | Suggested choice | Evidence and caveat |
|---|---|---|
| Serialization and schema | `serde` + `serde_json` + `schemars` | Serde provides generic serialization/deserialization without runtime reflection [42]. Schemars derives JSON Schema from Rust types and follows Serde attributes, but generated schema layout may change between releases [14] [14] [14]. Snapshot the public schema and version it. |
| CLI and diagnostics | `clap` + Ariadne or Miette | Clap provides help, suggestions, and completions [23]. Ariadne provides compiler-style diagnostics; its docs warn that exact rendered layout can change, so snapshot tests should be intentional [40] [40]. Miette is a viable alternative; select one renderer rather than building a large diagnostic stack. |
| Parsing | `tree-sitter` plus grammar crates compiled into the executable | Tree-sitter's Rust binding creates parsers and assigns languages [19]. The docs expose a Wasm feature with Wasmtime-related dependencies, which is not needed if grammars are compiled in [19] [19]. Bundle only the grammars required by the ledger. |
| Git integration | `gix` when a library API is valuable; shell out to `git` for a narrow first version | `gix` provides a repository abstraction and warns that default features can be reduced for build time; it also describes trust behavior for configuration [26] [26] [26]. Shelling out avoids embedding a broad Git implementation but requires careful argument and environment handling. |
| Time and durations | `jiff` or `time` | Jiff focuses on high-level date/time APIs designed to be harder to misuse [41]. The `time` crate documents feature controls and date-range limits [47] [47]. Parse ISO 8601 durations with a tested, explicit grammar; do not accept ambiguous human durations in machine output. |
| Matching and text boundaries | `nucleo` for interactive fuzzy selection; `strsim` or Bitap-like matching for simple candidate ranking; `unicode-segmentation` for grapheme/word boundaries | These are optional UX/search aids, not evidence-integrity primitives. Sentence splitting is language-sensitive; use an explicit sentence segmenter or keep sentence boundaries as provider-supplied units instead of pretending word segmentation is sentence segmentation. |
| One HTTP request | `ureq` for a small synchronous integration; `reqwest` if an async HTTP stack is already present | Avoid pulling in an async runtime solely for a single network call unless concurrency or integration needs justify it. Put network responses behind the same provider/fingerprint interface. |
| Tests | `insta` for reviewed snapshots, plus ordinary deterministic unit tests | Snapshot test schemas, diagnostics, and CLI JSON. Keep golden snapshots stable and make intentional output changes visible in review. |

**2026 maintenance status:** The research pages support the crate capabilities above, but most `latest` API pages do not state a last release date or maintenance commitment. Ariadne's docs show version **0.6.0** and a date of **17 October 2026**, a recent maintenance signal [40] [40]. For other dependencies, do not infer active maintenance just from a current docs.rs page: check the crate's release history, open security advisories, MSRV, and repository activity when locking versions. Commit Cargo.lock for the CLI application, and keep dependency updates deliberate [24].

For distribution, use `cargo-dist` to automate release artifacts and installers; its docs state that it can build and publish npm packages [45], and its installer guide includes Homebrew among package-manager routes [39]. Offer `cargo-binstall` as a Rust-user convenience when compatible prebuilt artifacts are published, while retaining `cargo install` as the fallback. For npm, publish per-platform binary packages or an installer that selects a matching artifact; document supported OS/architecture combinations and verify checksums. Homebrew is a convenient macOS route; signed release artifacts and reproducible build metadata matter more than maximizing installer count.

## 5. Agent-facing CLI: strict schemas, useful errors, compact output

Anthropic's tool-design guidance recommends a small set of distinct, high-impact tools rather than wrapping every endpoint; it emphasizes semantic names, explicit descriptions, strict data models, and measuring tool changes through evaluations [9] [9] [9]. It also recommends controlling response quantity and offering concise/detailed output where useful, because irrelevant output consumes context [9]. OpenAI Structured Outputs constrains completed, non-refusal output to a supplied JSON Schema, while applications still need to handle refusals and other cases [5].

Expose a small composable command surface such as `ledger resolve`, `ledger enumerate`, `ledger render`, `ledger record`, `ledger verify`, and `ledger judge`. Every read command should support `--format json`; JSON must have stable fields, IDs, ordering, and explicit schema version. Add `ledger schema` to print the exact command input/output schemas and examples, and `ledger help --json` for discoverability by agents. The schema command is an engineering recommendation based on explicit, machine-readable tool descriptions, not a feature that Anthropic or OpenAI specifically mandates for CLIs.

Errors should be structured on JSON output, with a stable `code`, a concise `message`, optional `details`, and an actionable `remediation`; human mode can render the same data using Ariadne/Miette. Keep stdout for machine-readable results and stderr for progress, warnings, and diagnostics. Use stable exit-code classes for success, validation/input error, not-found, provider failure, conflict, and internal failure. The model-provider tool-design sources focus on tool contracts and returned content, not shell exit-code conventions, so these codes are a CLI contract recommendation rather than a quoted vendor rule.

Support batch input from stdin and one batch response so agents do not need N subprocess round trips. Make `record`, `verify`, and cache writes idempotent: repeating the same operation with the same input must not duplicate records or change timestamps unnecessarily. Default to concise output with IDs, status, and a short reason; provide `--detail` for evidence excerpts and provenance. Offer pagination or filters on potentially large results, and return stable ordering. Anthropic explicitly endorses response quantity controls such as pagination, range selection, filtering, and truncation to control context use [9].

Test agent usability empirically. Anthropic recommends prototyping and evaluating with real tasks and verifiable outcomes rather than assuming a tool design works [9] [9]. Build tests that simulate an agent discovering the schema, making a malformed call, receiving remediation, retrying, and obtaining a result without reading the whole repository.

## 6. Judgment caching: key the whole request, replay exact outputs

Do not confuse provider prompt caching with the ledger's judgment cache. OpenAI documents prompt caching as reusing work when requests share a prompt prefix [10]. That may reduce repeated prompt-processing cost, but it is not a durable mapping from a request to an already accepted claim judgment.

Use a canonical cache key over all judgment-affecting inputs: provider/source fingerprint, unit ID, canonicalized evidence text or digest, full prompt/template version, output schema version, model provider and exact model snapshot ID, decoding parameters, tool versions, and any policy/configuration that can change the verdict. Apply a domain prefix such as `ledger:judgment:v1\0`; serialize the request deterministically and hash it with BLAKE3 or SHA-256. Store the complete canonical request or its components, output, validation result, model identifier, timestamp, and provenance so a cache hit is auditable. If any keyed input changes, miss the cache rather than returning a stale judgment.

Pin explicit model snapshots rather than floating aliases wherever the provider supports them. Treat the model ID as a required cache-key input, and preserve the exact request and response. A pinned ID improves reproducibility of the requested model version, but does not promise bit-for-bit deterministic generation across all service changes or runtime conditions. Do not rely on a seed alone as a guarantee of semantic determinism; validate outputs against the declared schema and keep nondeterministic judgments out of automatic source-of-truth updates unless policy explicitly permits them.

Keep transient model cache entries out of ordinary Git commits by default: they add churn, may contain sensitive prompt/evidence material, and can create misleading merge conflicts. Put them in a local ignored SQLite cache and optionally export CI cache artifacts keyed by the complete digest. If a judgment is accepted as durable evidence, commit that reviewed judgment as a normal ledger record with its input digest, model ID, prompt/schema version, and provenance; do not commit an opaque cache database. CI replay should use a supplied cache artifact or committed accepted judgment, never silently call a different model version when the exact key is absent. This separates deterministic replay from fresh model evaluation.

## Synthesis

The right architecture combines the inspectability of Git-native files, the query performance of a disposable database index, and the strict process boundary of a minimal provider protocol. The protocols that best match the core need are Git's small helper conventions, LFS's persistent JSON-line worker, and LSP's explicit initialization/capability model; MCP is valuable when broader agent interoperability is required, while Terraform's gRPC protocol is disproportionate for this scoped local CLI [13] [2] [31] [38] [11]. The key tradeoff is simplicity versus ecosystem reuse: define only the resource operations the ledger needs, but copy the proven ideas of explicit framing, versioning, capabilities, and process shutdown.

For persistence, one record per stable ID minimizes cross-record merge coupling; JSONL plus a carefully designed merge driver is a viable alternative when flat exports or streaming matter. Beads shows a middle ground with JSONL source data, an untracked SQLite database, and an identity-aware merge driver [36] [36]. git-bug shows that CRDT operation logs can make merges converge, but this entails an operation model and deterministic replay machinery [34]. For a first Rust implementation, deterministic files plus a rebuildable SQLite index offer a better complexity/performance balance.

For identity and reproducibility, keep evidence, normalized matching, AST equivalence, and model-judgment cache keys as distinct, versioned concepts. BLAKE3 or SHA-256 can protect CI decisions; xxHash cannot provide the same adversarial integrity properties [20] [16] [18]. For the user interface, publish a stable schema and compact JSON by default; for model judgments, preserve the request/model/provenance and replay exact cached outputs. Across all layers, do not execute repository-supplied code, rewrite evidence silently, or treat a performance cache as authoritative evidence.

## References

1. *Git - gitcredentials Documentation*. https://git-scm.com/docs/gitcredentials
2. *git-lfs/docs/custom-transfers.md at main · git-lfs/git-lfs · GitHub*. https://github.com/git-lfs/git-lfs/blob/main/docs/custom-transfers.md
3. *UAX #15: Unicode Normalization Forms*. https://unicode.org/reports/tr15/
4. *Prompt caching - Claude Platform Docs*. https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching
5. *Structured model outputs*. https://developers.openai.com/api/docs/guides/structured-outputs
6. *Advanced usage - OpenAI API*. https://platform.openai.com/docs/advanced-usage/reproducible-outputs
7. *Using tools*. https://developers.openai.com/api/docs/guides/tools
8. *RFC 8785: JSON Canonicalization Scheme (JCS) | RFC Editor*. https://www.rfc-editor.org/rfc/rfc8785
9. *Writing effective tools for AI agents \ Anthropic*. https://www.anthropic.com/research/writing-tools-for-agents
10. *Prompt caching*. https://developers.openai.com/api/docs/guides/prompt-caching
11. *Terraform plugin protocol | Terraform | HashiCorp Developer*. https://developer.hashicorp.com/terraform/plugin/terraform-plugin-protocol
12. *Plugins | Nushell*. https://www.nushell.sh/book/plugins.html
13. *Git - gitremote-helpers Documentation*. https://git-scm.com/docs/gitremote-helpers
14. *schemars - Rust*. https://docs.rs/schemars/latest/schemars/
15. *Conflicts - Jujutsu docs*. https://docs.jj-vcs.dev/latest/conflicts/
16. *Hash Functions | CSRC*. https://csrc.nist.gov/projects/hash-functions
17. *Buildkite agent hooks*. https://buildkite.com/docs/agent/hooks
18. *GitHub - Cyan4973/xxHash: Extremely fast non-cryptographic hash algorithm · GitHub*. https://github.com/Cyan4973/xxHash
19. *tree_sitter - Rust*. https://docs.rs/tree-sitter/latest/tree_sitter/
20. *GitHub - BLAKE3-team/BLAKE3: the official Rust and C implementations of the BLAKE3 cryptographic hash function · GitHub*. https://github.com/BLAKE3-team/BLAKE3
21. *Git - gitattributes Documentation*. https://git-scm.com/docs/gitattributes
22. *pre-commit*. https://pre-commit.com/#creating-new-hooks
23. *clap - Rust*. https://docs.rs/clap/latest/clap/
24. *Cargo.toml vs Cargo.lock - The Cargo Book*. https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html
25. *miette - Rust*. https://docs.rs/miette/latest/miette/
26. *gix - Rust*. https://docs.rs/gix/latest/gix/
27. *External Tools - The Cargo Book*. https://doc.rust-lang.org/cargo/reference/external-tools.html
28. *Backends: State Storage and Locking | Terraform | HashiCorp Developer*. https://developer.hashicorp.com/terraform/language/state/backends
29. *Configuration | Extism - make all software programmable. Extend from within.*. https://extism.org/docs/concepts/configuration
30. *Developing Extensions {#developing-extensions}*. https://zed.dev/docs/extensions/developing-extensions
31. *Specification*. http://microsoft.github.io/language-server-protocol/specifications/lsp/3.18/specification
32. *Bazel Lockfile*. https://bazel.build/external/lockfile
33. *An Application with Plugins - Wasmtime*. https://docs.wasmtime.dev/wasip2-plugins.html
34. *GitHub - git-bug/spec · GitHub*. https://github.com/git-bug/spec
35. *Security - Wasmtime*. https://docs.wasmtime.dev/security.html
36. *Git Integration*. https://github.com/steveyegge/beads/raw/refs/heads/main/docs/GIT_INTEGRATION.md
37. *Versioning and Compatibility - Model Context Protocol*. https://modelcontextprotocol.io/specification/2026-07-28/basic/lifecycle
38. *stdio*. https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio
39. *Install - dist*. https://axodotdev.github.io/cargo-dist/book/install.html
40. *ariadne - Rust*. https://docs.rs/ariadne/latest/ariadne/
41. *jiff - Rust*. https://docs.rs/jiff/latest/jiff/
42. *serde - Rust*. https://docs.rs/serde/latest/serde/
43. *Remote caching | Nx*. https://nx.dev/ci/features/remote-cache
44. *Tutorial ¶*. https://towncrier.readthedocs.io/en/stable/tutorial.html
45. *npm - dist*. https://axodotdev.github.io/cargo-dist/book/installers/npm.html
46. *GitHub - cargo-bins/cargo-binstall: Binary installation for rust projects · GitHub*. https://github.com/cargo-bins/cargo-binstall
47. *time - Rust*. https://docs.rs/time/latest/time/
48. *Internal Files | Data Version Control · DVC*. https://dvc.org/doc/user-guide/project-structure/internal-files
49. *Streamable HTTP*. https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http
50. *changesets/docs/detailed-explanation.md at main · changesets/changesets · GitHub*. https://github.com/changesets/changesets/blob/main/docs/detailed-explanation.md
