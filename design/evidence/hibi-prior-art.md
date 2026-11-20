# A Local Trust Ledger That Survives Change

## Executive Summary

- **Separate Identity From Content**: in-toto binds a statement to artifact digests, while source-control and code-analysis tools use differing match strategies; keep anchor resolution identity separate from a cryptographic fingerprint of the exact content reviewed, and never treat a successful reattachment as proof that content is unchanged [61].
- **Sign Observations, Then Evaluate Authority**: DSSE and in-toto provide signed envelope and statement patterns, but the signature alone does not decide whether an actor or review basis is authorized; bind each seal to a policy digest and enforce actor, role, basis, and threshold rules locally [62][57][18].
- **Make Freshness Computed, Not Editable**: require the newest policy-counting seal to cover the current fingerprint of the subject and every evidence anchor, and to remain within its validity window; treat changed, missing, ambiguous, expired, or unauthorized observations as distinct non-fresh outcomes [8][34].
- **Borrow Explicit Expiry Carefully**: TUF uses role-specific trust, metadata versions, and expiration to limit rollback and stale metadata; apply explicit expiry to time-sensitive claims, but do not force a network transparency service or a full TUF update system into a small offline tool [33].
- **Treat Traceability Links as Reviewable Evidence**: Doorstop flags changed fingerprints as suspect, and StrictDoc can report both unsupported code and requirements linked to removed code; the ledger should make stale and dangling links visible rather than silently repairing them [8][14].
- **Use Resource-Specific Anchors**: text quote context, semantic symbol indexes, AST differencing, Git history, and issue fingerprints solve different location problems; choose a provider adapter per resource, preserve a deterministic raw fingerprint, and surface ambiguity instead of guessing.
- **Measure Coverage in Both Directions**: declare an enumerable scope, then report claims whose subject or evidence lost backing as well as eligible units with no claim; CODEOWNERS approval rules do not by themselves prove every tracked file is owned [49][14].
- **Keep AI Review Provisional Unless Policy Says Otherwise**: AI-generated documentation can be proposed from code diffs, but a study reports that about one fifth of the best-performing LLM-generated comments were demonstrably inaccurate; require a distinct, authorized human or constrained process seal for consequential claims.

## 1. Attestation Patterns: A Valid Signature Is Not a Valid Review

Use the standards as modular parts, not as a single mandatory stack. An in-toto Statement has a subject array, a digest for each subject, a predicate type, and a predicate; the subject digest is the content binding, while the predicate carries the kind of claim being made [61]. The specification says subject artifacts are matched by digest regardless of content type, a useful warning against relying on filenames as identity [61]. An in-toto Envelope requires consumers to parse and verify the signed payload before relying on authenticated predicate data [62]. DSSE adds a precise pre-authentication encoding so signatures bind the payload type and exact payload bytes; implementations must use the verified payload bytes rather than reparse a potentially different representation [57].

| Model | Useful shape and control | Lesson for a local ledger |
|---|---|---|
| in-toto Statement and Layout | Statement: typed subject digests plus predicate. Layout steps specify expected materials, products, commands, keys, and thresholds; link metadata records materials and products [61][18]. | Borrow a stable subject digest, explicit predicate type, step expectations, and signer threshold. Keep policy requirements distinct from recorded observations. |
| DSSE | Envelope carries payload, payload type, and signatures; PAE binds payload type and exact payload bytes. Threshold acceptance can require a number of trusted keys [57]. | Use a canonical signed payload and verify the exact bytes before parsing. Do not make a key ID itself an authorization decision. |
| SLSA provenance and verification summary | Provenance binds output subjects and describes build definition, resolved dependencies, builder, and run details. The VSA records verifier identity, policy URI or digest, resource, and a PASSED or FAILED result [695.17-35, 695.49-52, 695.103; 694.65, 694.78-98]. | Borrow explicit observation context and policy identity. A pass result is a decision by a verifier, not a timeless property of the resource. |
| Sigstore and Rekor | Sigstore signs and verifies artifacts; Rekor supplies a transparency log and inclusion evidence. The log is designed to be append-only, with entries not mutated or removed [63][64][65]. | Make external transparency optional. A local append-only journal can record history, while signatures and policy establish integrity and authority. |
| W3C Verifiable Credentials | `validFrom` and `validUntil` participate in validation, and `credentialStatus` can identify suspended or revoked credentials; DIDs are optional [66]. | Model time validity and revocation/status explicitly, but do not require decentralized identity for a local repository. |
| TUF | Role delegation and key thresholds constrain who signs which targets; versioning and expiry limit rollback and stale metadata, and timestamp metadata binds a recent snapshot [33]. | Borrow scoped authority, thresholds, expiry, and anti-rollback thinking. Avoid reproducing TUF's repository update machinery unless distributing trusted metadata is itself a requirement. |

A useful case study is GitHub artifact attestations: their documentation describes signed claims about build provenance, and consumers can verify where and how software was built; GitHub uses Sigstore for this workflow [63]. The transferable design is not "sign anything." It is to bind an attestation to a specific subject digest, identify the predicate, and make verification an explicit consumer action. A local ledger can preserve these properties without requiring a public log for every local edit.

TUF is the complementary case. Its metadata has roles, delegations, threshold signatures, version checks, and expiration, specifically addressing trust scope and stale or rolled-back metadata [33]. That machinery is valuable when distributing repository trust roots, but heavier than a local review ledger needs. Implement a small role map such as `author`, `reviewer`, `release`, and `CI`, pin policy digests, and support thresholds only for claims whose risk justifies them. For timestamps, record a clear evaluation time and optional trusted timestamp proof; Sigstore distinguishes Rekor inclusion time and signed timestamp mechanisms [67].

## 2. Traceability Tools: Links Need Drift Signals and Human Meaning

These tools demonstrate a spectrum from manual identifier links to executable documentation and AI-generated updates. Their common limitation is that link resolution is not the same as semantic validity. A link can still point to a real function after that function's behavior has changed.

| Tool | How it links and detects drift | Concrete lesson or limitation |
|---|---|---|
| Doorstop | Requirement items use identifiers and links; changed fingerprints can mark a link suspect, while validation catches unknown references and unreviewed changes [8]. | Keep a reviewable `suspect` state. A changed fingerprint should trigger re-review, not automatic acceptance. |
| Sphinx-Needs | Needs have IDs and typed relations; link validation can enforce context such as approved requirements and compatible test links. | Validate relationship meaning, not only target existence. |
| StrictDoc | Traceability matrices connect requirements, code references, tests, and results; reports can show uncovered code and requirements linked to removed code, plus additions and removals in a change [14]. | Strong model for bidirectional coverage and change reports. |
| OpenFastTrace | Typed coverage reports distinguish covered and uncovered item types and can require selected requirement types [19]. | Make coverage criteria explicit by resource/unit type. |
| BMW TRLC and LOBSTER | A TRLC adapter maps typed records and fields into LOBSTER items; LOBSTER supports modular conversions and reporting. | Keep provider adapters separate from a common traceability representation. |
| Reqflow | Requirements can link to tests, references, and documents; matrix reports expose the relation set. | Stable IDs and matrices are practical, but IDs and relationship maintenance create authoring work. |
| tagref | Documentation includes tags and `[ref:...]` references; checks verify that references resolve to existing tags. | A lightweight link checker is useful, but a resolvable tag does not prove that surrounding prose remains true. |
| Swimm | Code snippets are checked for synchronization; its described algorithm considers markers, line numbers, token references, change size, and file history, ranking candidates and asking for reselection when uncertain. | Use multiple signals and an explicit ambiguity outcome. Do not silently patch a low-confidence match. |
| Doc Detective | Documentation can be expressed as user actions such as clicks, drags, element-existence checks, and URL status checks. | Execute procedural instructions as tests where possible; this checks usability paths, not every semantic statement. |
| doctest | Interactive examples are extracted, executed, and checked against expected output. | Make small snippets executable and treat failure as a reproducible drift signal. |
| Vale | Rules lint prose and style, including custom rules [41]. | Useful for wording consistency, not a proof of code-documentation correspondence. |
| Mintlify Autopilot | Vendor documentation describes monitoring code changes and proposing documentation updates through reviewable PR workflows [51]. | Treat generated edits as proposals; preserve an approval step and the code diff that caused the proposal. |
| DocuWriter-style checkers | Vendor material describes merge monitoring, mapping diffs to affected pages, scoring edits, and review or auto-apply options [52]. | These are product capability claims, not independent evidence of update correctness; avoid counting an AI patch as a seal by default. |
| Academic inconsistency detection | One study reports outdated code references in 28.9% of the top 1,000 GitHub projects it examined, using extracted references and revision comparison [24]. A separate study of 1,518 projects reports average F1 of 82.6% for its comment consistency method [45]. | Automated detection can be useful, but dataset and task boundaries matter; a suspect link is not necessarily a false statement. |

Doorstop and StrictDoc illustrate two complementary decisions. Doorstop treats a fingerprint change as a reason to suspect a requirement link and permits validation of unreviewed changes [8]. That is a good default for trust: the system signals that backing changed, but does not pretend to know whether the requirement still holds. StrictDoc goes further by exposing both directions, including code with no requirement and requirements pointing to removed code [14].

Executable-doc tools make a different tradeoff. Doc Detective and doctest validate concrete instructions and examples by running them. They can provide stronger evidence than a prose link for the behavior they actually execute, but only cover the paths and examples that authors maintained. The 2024 comment study's finding that roughly one fifth of the best-performing LLM comments were demonstrably inaccurate, and that consistency detection did not significantly predict comment accuracy, is a reason to separate syntactic or link consistency from truth. Build the ledger to record those different evidence qualities rather than merging them into one green check.

## 3. Anchors: Resolve a Unit, Then Fingerprint What Was Seen

A stable locator answers "which unit is this?" A fingerprint answers "what exact state did the reviewer observe?" No technique solves both for every resource. Keep a resolver result, a raw or canonical content digest, and a confidence/status for reattachment as distinct fields.

| Technique | What it preserves | Best use and cost |
|---|---|---|
| W3C Web Annotation and Hypothesis fuzzy anchoring | Text quote selectors retain selected text; positional selectors provide a second clue, and reattachment can try multiple strategies with refinement/context. | Good for prose and generated docs. Repeated text, edits to context, and normalization can make a match ambiguous; return candidates, not a guessed identity. |
| Git blame and line-history tools | History can attribute lines and follow renames; move/copy detection options can help track lines across files. | Cheap history evidence for line-oriented review, but line attribution is not a durable semantic identity after deletion, replacement, or major refactoring. |
| SARIF and GitHub code scanning | Partial fingerprints and stable rule/path information help match findings between runs and avoid duplicate alerts. | Good for diagnostics with repeatable rules. The finding identity is tool/rule-specific, not a general identifier for the code unit. |
| SonarQube issue tracking | An issue hash uses first-line content while excluding whitespace, and fixed issues can be reopened when the issue reappears [6]. | Illustrates pragmatic normalization and lifecycle tracking; matching may be fragile when the first line changes. |
| Semgrep | Match-based and syntactic IDs are based on the rule match and metavariable values; the matching algorithm can evolve. | Useful for stable rule findings, but IDs are coupled to rule semantics and versions. |
| GumTree and difftastic | GumTree maps similar AST nodes and emits edit actions including moves; difftastic compares structural subtrees [17]. | Better than line numbers for refactoring-aware diffs, at higher parser and matching cost; repeated or heavily edited structures can be ambiguous. |
| tree-sitter | Incremental parsing reuses parser structure across edits, while node ranges and positions require adjustment; parser nodes are not durable cross-version IDs [55]. | Strong provider for syntax-aware resolution and fast reparsing, not a persistent identity scheme by itself. |
| SCIP, LSIF, Kythe, and stack graphs | SCIP exposes symbol identifiers; stack graphs model name resolution through symbol/name stacks [22][36]. | Prefer symbol-level anchors for APIs and declarations. Names, overloads, package moves, and renames still need aliases or migration rules; a semantic index locates symbols but does not guarantee identity through every rename. |

For code, the most robust practical design is a semantic anchor backed by an AST-aware resolver, with symbol identity when an index exists and AST diff/history as fallback. It tolerates reformatting and many moves better than a line number. Renames are the hard case: a name-based symbol key may change, so keep a migration alias when a reviewed rename maps the old symbol to the new one. For prose, use quote-plus-prefix/suffix or equivalent contextual selectors and retain position only as a hint. For Git objects, use the immutable object ID and do not fuzzy-match changed content. For external architecture or tool data, prefer the provider's persistent element identifier plus revision/digest. If the resolver returns zero or multiple plausible units, report missing or ambiguous.

This layered approach distinguishes stable identity from exact review. The in-toto model's digest-only subject binding makes exactness clear [61]; code-analysis tools show why a separate matching key is often needed to track findings between runs. Store both a raw content digest for strict freshness and, optionally, a normalized fingerprint for candidate matching. A normalized fingerprint can find likely continuity, but must never substitute for the raw digest when declaring a seal fresh.

## 4. Assurance Models: Reuse Their Vocabulary, Not a Single Boolean

Assurance systems distinguish claims, evidence, assessments, and outcomes. A trust ledger should do the same. In particular, do not treat "the claim is fresh," "the evidence supports it," "the policy passed," and "the actor is trusted" as synonyms.

| Model | Vocabulary or result structure | What to reuse |
|---|---|---|
| GSN and SACM | GSN-style goals and supporting argument elements map to SACM claims, artifact references, assertions, evidence, inferences, and support relations [32]. | Represent a claim separately from evidence and the inference/policy that says why evidence supports it. |
| OSCAL assessment results | Assessment logs capture actions and times; observations carry evidence; findings and risks record control-oriented results. | Preserve observations, evidence, findings, and risks. Do not invent a universal status enum where the model records contextual assessment results. |
| OPA and conftest | Rego policies evaluate structured input; conftest reports policy violations and test pass/fail, with coverage as a separate report dimension. | Keep policy evaluation deterministic and separate from claim truth or evidence freshness. |
| SBOM/VEX | VEX statements use statuses such as `affected`, `not_affected`, `fixed`, and `under_investigation`; a `not_affected` statement requires a justification or impact statement, and affected/fixed states can carry action context. | Use explicit domain status plus required justification. A status without its subject and explanation is not actionable. |
| Requirements/test matrices | Requirement identifiers link to tests and results, allowing a requirement-to-test coverage view [14]. | Treat test existence, execution result, and requirement satisfaction as separate facts. |

GSN/SACM are useful when a repository claim needs a reasoned argument rather than a bare link: a claim can have evidence and an explicit inference or support relation [32]. A local ledger does not need the full assurance-case graph for every comment, but it should allow a claim to say what evidence supports it and what policy makes that support count.

VEX and OPA show why a single `valid: true` field loses important meaning. VEX distinguishes affectedness and investigation state and can require justification for a not-affected conclusion. OPA/conftest instead report whether a policy evaluation passes or finds violations. An assessment's pass result says the policy evaluated positively for an input; it does not say that all dependencies are current, a review was authorized, or a scope is fully covered. Represent state as independent dimensions: resolution, freshness, evidence assessment, authorization, and coverage. Then derive an overall status for display while retaining the underlying reasons.

## 5. Reverse Coverage and Diff-Scoped Review

A scope must define a unit universe that a provider can enumerate. Without that, "100% covered" is a count of known claims divided by an unknown denominator. Documentation coverage extensions can enumerate API objects with missing docs. StrictDoc provides the most direct pattern among the reviewed traceability tools: report uncovered code and requirements linked to removed code, then compare added, modified, and removed requirements in a change [14]. OpenFastTrace's explicit covered/uncovered item types are another useful pattern.

| Coverage problem | Recommended computation | Existing pattern |
|---|---|---|
| A claim lost subject or evidence backing | Resolve all anchors at the new revision; compare current fingerprints with the newest counting seal; report changed, missing, or ambiguous sides separately. | Doorstop marks changed fingerprints suspect; StrictDoc exposes removed-code links [8][14]. |
| A new eligible unit has no claim | Enumerate scope units at the new revision, subtract eligible units with a qualifying subject claim, then report the remainder. | StrictDoc's uncovered-code report and OpenFastTrace's uncovered type report [14]. |
| A previously covered unit was removed | Report it as removed or retired, not as a fresh claim; require explicit disposition if policy says deletion needs review. | StrictDoc flags requirements pointing to removed code [14]. |
| A team ownership rule misses files | Enumerate tracked files and check the applicable ownership mapping; report unowned paths. | CODEOWNERS routes review/approval to listed owners, but the file alone does not guarantee every tracked file has an owner [49]. |
| A test or document link exists but does not prove behavior | Record that the link resolves, then separately record test execution or review status. | Doctest and Doc Detective execute examples/actions rather than merely checking links. |

For a `base..head` change, build the unit inventory at both revisions and run the provider's diff. For every affected claim, resolve the old and new anchor, compare the old sealed digest with the current digest, and classify the result as changed, lost, ambiguous, or unchanged. Then enumerate the new scope and find eligible units with no qualifying claim. This produces two independent lists: invalidated backing and newly uncovered material. A removal should be reported as a lifecycle event, not silently dropped from the denominator. StrictDoc's change report for added, modified, and removed requirements supports this change-oriented design [14].

CODEOWNERS offers a cautionary case: it can require review by designated owners for matching paths, but ownership assignment is not equivalent to an exhaustive coverage audit. The ledger should therefore make scope enumeration a provider responsibility and show exclusions explicitly. Allow exceptions, but make each exception an anchored, policy-governed record with an owner and rationale; do not hide exclusions inside path globs.

## 6. Recommended Core: Four Immutable Records and Derived Status

Keep the core small: resources identify providers and locators; anchors describe how a unit is re-found; claims state a proposition and link subject/evidence anchors; seals append an actor's decision over exact observed fingerprints; scopes define which units must be covered. The examples below are illustrative JSON. The `initialObservation` in an anchor is a historical hint, not a mutable current value; current resolution is computed by the provider. The seal, not the anchor record, is authoritative for what was observed at review time. This separation borrows digest binding from in-toto and exact-byte envelope verification from DSSE [61][57].

**Anchor example**

```json
{
  "id": "anchor:renderer.render",
  "resource": {
    "provider": "tree-sitter-typescript",
    "locator": "src/renderer.ts"
  },
  "selector": {
    "kind": "symbol",
    "symbol": "render"
  },
  "fingerprintSpec": {
    "algorithm": "sha256",
    "view": "raw-unit-bytes"
  },
  "initialObservation": {
    "algorithm": "sha256",
    "value": "<hex-digest>"
  },
  "resolverVersion": "<provider-version>"
}
```

**Claim example**

```json
{
  "id": "claim:renderer.output-escapes-html",
  "statement": "render escapes untrusted text before inserting it into HTML",
  "subject": "anchor:renderer.render",
  "evidence": [
    "anchor:test.render-escapes-html",
    "anchor:docs.output-escaping"
  ],
  "policy": {
    "id": "policy:security-review",
    "digest": "sha256:<policy-digest>"
  }
}
```

**Seal example**

```json
{
  "id": "seal:01J-example",
  "claim": "claim:renderer.output-escapes-html",
  "actor": {
    "id": "agent:security-reviewer",
    "kind": "agent",
    "keyId": "<trusted-key-id>"
  },
  "basis": "human_vouch",
  "decision": "accept",
  "observed": [
    {
      "anchor": "anchor:renderer.render",
      "role": "subject",
      "algorithm": "sha256",
      "fingerprint": "<subject-digest>"
    },
    {
      "anchor": "anchor:test.render-escapes-html",
      "role": "evidence",
      "algorithm": "sha256",
      "fingerprint": "<evidence-digest>"
    },
    {
      "anchor": "anchor:docs.output-escaping",
      "role": "evidence",
      "algorithm": "sha256",
      "fingerprint": "<documentation-digest>"
    }
  ],
  "policyDigest": "sha256:<policy-digest>",
  "issuedAt": "<timestamp>",
  "expiresAt": "<timestamp-or-null>",
  "signature": {
    "format": "DSSE",
    "payloadType": "application/vnd.example.trust-ledger.seal+json",
    "value": "<signature-envelope>"
  }
}
```

**Scope example**

```json
{
  "id": "scope:public-types",
  "resource": {
    "provider": "typescript-project-index",
    "locator": "src/"
  },
  "universe": {
    "unitKind": "exported-symbol",
    "selector": "all-exported-symbols"
  },
  "coverageRule": {
    "claimSubjectMustResolveToUnit": true,
    "minimumCountingSeals": 1,
    "policy": "policy:public-api"
  },
  "exclusions": []
}
```

The claim state machine should be computed at a repository revision and evaluation time. First resolve each anchor to zero, one, or multiple units. Zero means `missing`; multiple means `ambiguous`; both prevent freshness. For a resolved claim, find the newest seal that meets the policy's actor, basis, signature, decision, and policy-digest requirements. If there is no such seal, report `unsealed` or `unauthorized`. If the newest counting seal is expired, report `expired`; if any observed fingerprint differs from its current anchor, report `stale`; if authorized counting seals conflict, report `disputed`; otherwise report `fresh`. Separately compute scope status as `covered`, `uncovered`, `removed`, or `excluded`. Do not fall back to an older approval after a newer authorized rejection or an expired latest counting seal.

| Basis | Default counting rule |
|---|---|
| `exact_match` | Counts only for claims whose policy says digest equality is sufficient; it proves no more than unchanged observed content. |
| `model_judged_evidence` | Record model/version and evidence/prompt digests. Default to non-counting or require a human/process threshold; never treat a model label as authority. |
| `human_vouch` | Counts only when the principal is authorized for the claim's policy and the seal covers all sides. |
| `acknowledgement` | Records awareness, not approval; should not count toward freshness unless a policy explicitly defines an acknowledgement-only claim. |
| `process` or CI verification | Counts only for an identified, policy-authorized workflow with pinned policy and tool versions. |

For all seal types, verify the signature over exact payload bytes, bind the policy digest and every observed anchor fingerprint, and preserve seals append-only. DSSE's exact-byte rule, TUF's delegated roles and thresholds, and VC validity/status properties provide concrete design patterns [57][33]. Let policies decide whether expiry is required: a documentation claim may be content-driven, while an operational or release claim may need time limits.

## 7. Failure Modes: Make Uncertainty Visible

| Failure mode | Why it breaks trust | Mitigation |
|---|---|---|
| Fuzzy match is mistaken for identity | Quote or AST similarity can reattach to the wrong repeated or moved unit. | Return `ambiguous` when multiple candidates fit; require human confirmation and a new seal. Use selectors for resolution, never as a substitute for a content digest. |
| Fingerprint is mistaken for semantic truth | Exact equality proves the reviewed bytes are unchanged, not that a claim was correct or that the evidence logically supports it. | Keep basis and policy explicit; distinguish `exact_match`, model judgment, and human vouch. A consistency check is not a correctness oracle. |
| AI-generated updates auto-count as review | Generated comments can be plausible and inaccurate; product workflow descriptions often describe suggestions, not measured correctness. | Treat generated patches as proposals; require an authorized review seal for consequential claims. |
| Link resolves but context is wrong | A UID, tag, or file can still exist after the referenced behavior changes. | Add type/context constraints, fingerprints, test evidence, and explicit suspect states [8]. |
| Incomplete scope yields false 100% coverage | A matrix can cover every recorded requirement but omit eligible source units. | Enumerate units from the provider and report the denominator, exceptions, and failures to enumerate. Use reverse checks such as StrictDoc's uncovered-code report [14]. |
| Any valid signature is trusted | A cryptographically valid seal may come from an unauthorized actor or stale policy. | Map trusted keys to roles and allowed bases; pin policy digests; use threshold rules for high-risk claims [18][33]. |
| Expiry or revocation is ignored | A once-valid seal or credential may no longer be acceptable. | Evaluate expiry/status at a recorded time and expose revoked/suspended or expired states [33]. |
| Provider behavior changes silently | Parser, index, normalization, or diff changes can alter unit resolution and fingerprints. | Record provider and resolver versions; treat resolver upgrades as migrations; retain raw bytes or immutable object IDs where possible. |
| Review history is overwritten | Updating one mutable `approved` flag erases who saw what and why. | Append new seals, preserve old observations, and derive current state from history and current resource snapshots. Rekor's append-only design is a useful precedent [64]. |

## Synthesis

Across these systems, the key design split is between **binding**, **resolution**, **judgment**, and **coverage**. In-toto, DSSE, SLSA, and Sigstore emphasize binding a statement to an exact subject and authenticating its envelope; W3C credentials and TUF emphasize validity windows, status, delegated authority, and rollback resistance [61][62][33]. Traceability tools such as Doorstop, StrictDoc, and OpenFastTrace focus on relationships, suspect changes, and matrix coverage; documentation test tools focus on executable steps; AI sync tools propose edits but still need review [8][14].

The strategies differ on at least three dimensions. **Identity** ranges from immutable object digests to fuzzy text selectors and semantic symbols. **Evidence strength** ranges from a resolvable link, to a matching fingerprint, to an executed test or authorized human judgment. **Operational cost** rises from line-based fingerprints to parser/index providers, multi-party signature policies, and explicit audit workflows [61][36][33]. This is why one universal anchor algorithm or one global `valid` boolean would be a poor abstraction.

For a small local tool, the strongest practical design is an append-only set of signed seals over exact fingerprints, a provider-specific anchor resolver with explicit ambiguity, a policy engine that decides which actors and bases count, and a scope enumerator that computes both lost backing and uncovered units. Prefer semantic symbol and AST-aware location for code, quote/context for prose, immutable object IDs for Git objects, and provider-owned IDs for external models. Use raw digests for freshness and normalized or similarity signals only to suggest reattachment. Make AI judgment a separate evidence basis, not a silent upgrade to human approval. The ledger then answers a useful bounded question: who accepted which claim, based on which exact material, under which policy, and what changed since.

## References

1. *doctest — Test interactive Python examples — Python 3.15.2 documentation*. https://docs.python.org/3/library/doctest.html
2. *GitHub - bmw-software-engineering/lobster: Lightweight Open BMW Software Traceability Evidence Report · GitHub*. https://github.com/bmw-software-engineering/lobster
3. *lobster/packages/lobster-tool-trlc/README.md at main · bmw-software-engineering/lobster · GitHub*. https://github.com/bmw-software-engineering/lobster/blob/main/packages/lobster-tool-trlc/README.md
4. *SARIF support for code scanning - GitHub Docs*. https://docs.github.com/en/code-security/code-scanning/integrating-with-code-scanning/sarif-support-for-code-scanning
5. *SLSA • Provenance*. https://slsa.dev/spec/v1/provenance/
6. *Issue management solution*. https://docs.sonarsource.com/sonarqube-community-build/user-guide/issues/solution-overview
7. *SLSA • Verification Summary Attestation (VSA)*. https://slsa.dev/spec/v1.1/verification_summary
8. *Integrity Checks*. https://doorstop.readthedocs.io/en/latest/cli/validation.html
9. *Assessment Results Model*. https://pages.nist.gov/OSCAL/learn/concepts/layer/assessment/assessment-results
10. *Tests overview*. https://docs.doc-detective.com/docs/tests/overview
11. *Vulnerability Exploitability eXchange (VEX) - Status Justifications*. https://www.cisa.gov/sites/default/files/publications/VEX_Status_Justification_Jun22.pdf
12. *dsse/protocol.md at master · secure-systems-lab/dsse · GitHub*. https://github.com/secure-systems-lab/dsse/blob/master/protocol.md
13. *Web Annotation Data Model*. https://www.w3.org/TR/annotation-model
14. *Features | StrictDoc*. https://strictdoc-project.github.io/features
15. *How does Swimm’s Auto-sync feature work? - Swimm*. https://swimm.io/blog/how-does-swimm-s-auto-sync-feature-work
16. *Fuzzy Anchoring : Hypothesis*. https://web.hypothes.is/blog/fuzzy-anchoring
17. *Tree Diffing - Difftastic Manual*. https://difftastic.wilfred.me.uk/tree_diffing.html
18. *specification/in-toto-spec.md at v1.0 · in-toto/specification · GitHub*. https://github.com/in-toto/specification/blob/v1.0/in-toto-spec.md
19. *XML Tracing Report | OpenFastTrace*. https://openfasttrace.itsallcode.org/user_guide/reference/xml_tracing_report.html
20. *Remove duplicate findings*. https://docs.semgrep.dev/semgrep-code/remove-duplicates
21. *need items ¶*. https://sphinx-needs.readthedocs.io/en/latest/directives/need.html
22. *SCIP - a better code indexing format than LSIF | Sourcegraph*. https://sourcegraph.com/blog/announcing-scip
23. *Detecting outdated code element references in software repository documentation | Empirical Software Engineering | Springer Nature Link*. https://link.springer.com/article/10.1007/s10664-023-10397-6
24. *Wait, wasn’t that code here before? Detecting Outdated Software Documentation*. https://arxiv.org/pdf/2307.04291
25. *GitHub - stepchowfun/tagref: Manage cross-references in your code. · GitHub*. https://github.com/stepchowfun/tagref
26. *Reqflow*. https://goeb.github.io/reqflow
27. *Are your comments outdated? Towards automatically detecting code-comment consistency M*. https://arxiv.org/pdf/2403.00251
28. *Policy Testing | Open Policy Agent*. https://openpolicyagent.org/ecosystem/by-feature/policy-testing
29. *Conftest*. https://www.conftest.dev/
30. *Roles and metadata | TUF*. https://updateframework.com/docs/metadata
31. *Verifiable Credentials Data Model v2.1*. https://w3c.github.io/vc-data-model/index.html
32. *Relationship of GSN to SACM*. https://scsc.uk/file/gc-main/GSN-and-SACM-v1-1083.pdf
33. *The Update Framework Specification*. https://theupdateframework.github.io/specification/v1.0.23
34. *SLSA • Verification Summary Attestation (VSA)*. https://slsa.dev/spec/v1.2/verification_summary
35. *Fine-grained and Accurate Source Code Differencing*. https://notes.billmill.org/images/gumtree.pdf
36. *Stack Graph Docs*. https://github.github.com/stack-graph-docs
37. *CycloneDX v1.7 JSON Reference*. https://cyclonedx.org/docs/latest
38. *Tutorial: Auto-update documentation when code changes*. https://mintlify.com/docs/guides/automate-agent
39. *Console Tracing Report | OpenFastTrace*. https://openfasttrace.itsallcode.org/user_guide/reference/console_tracing_report.html
40. *sphinx.ext.coverage – Collect doc coverage stats — Sphinx documentation*. https://www.sphinx-doc.org/en/master/usage/extensions/coverage.html
41. *Introduction*. https://docs.vale.sh/
42. *GitHub - goeb/reqflow: Requirement Traceability Tool · GitHub*. https://github.com/goeb/reqflow
43. *Keep Docs Updated with Auto-Sync 🏊 Swimm Developer Community*. https://docs.swimm.io/features/keeping-docs-up-to-date
44. *Identifying Inaccurate Descriptions in LLM-generated Code Comments via Test Execution*. https://arxiv.org/abs/2406.14836
45. *Code Comment Inconsistency Detection Based on Confidence Learning | IEEE Transactions on Software Engineering*. https://dl.acm.org/doi/10.1109/TSE.2024.3358489
46. *spec/OPENVEX-SPEC.md at main · openvex/spec · GitHub*. https://github.com/openvex/spec/blob/main/OPENVEX-SPEC.md
47. *Code Comment Inconsistency Detection and Rectification Using a Large Language Model | Proceedings of the IEEE/ACM 47th International Conference on Software Engineering*. https://dl.acm.org/doi/10.1109/ICSE55347.2025.00035
48. *OSCAL Assessment Results Model v1.1.3 JSON Format Reference*. https://pages.nist.gov/OSCAL-Reference/models/v1.1.3/assessment-results/json-reference
49. *About code owners*. https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners
50. *Node ID Uniqueness · tree-sitter/tree-sitter · Discussion #3549 · GitHub*. https://github.com/tree-sitter/tree-sitter/discussions/3549
51. *Keep documentation up-to-date with Autopilot*. https://www.mintlify.com/docs/autopilot
52. *Automated Documentation for Software Teams | DocuWriter.ai*. https://www.docuwriter.ai/automated-documentation
53. *Git - git-blame Documentation*. https://git-scm.com/docs/git-blame
54. *When Comments Mislead: An Empirical Study of Outdated Comments in LLM-Based Code Completion | Proceedings of the 41st IEEE/ACM International Conference on Automated Software Engineering*. https://dl.acm.org/doi/10.1145/3832783.3834359
55. *Advanced Parsing - Tree-sitter*. https://tree-sitter.github.io/tree-sitter/using-parsers/3-advanced-parsing.html
56. *A Stable Lossless Syntax Tree for Real-Time Collaborative Programming*. https://drops.dagstuhl.de/storage/00lipics/lipics-vol372-ecoop2026/LIPIcs.ECOOP.2026.5/LIPIcs.ECOOP.2026.5.pdf
57. *dsse/envelope.md at master · secure-systems-lab/dsse · GitHub*. https://github.com/secure-systems-lab/dsse/blob/master/envelope.md
58. *interrogate: explain yourself — Python docstring coverage (v1.7.0)*. https://interrogate.readthedocs.io/
59. *scip/scip.proto at main · scip-code/scip · GitHub*. https://github.com/scip-code/scip/blob/main/scip.proto
60. *lobster/documentation/user-manual.md at main · bmw-software-engineering/lobster · GitHub*. https://github.com/bmw-software-engineering/lobster/blob/main/documentation/user-manual.md
61. *attestation/spec/v1/statement.md at main · in-toto/attestation · GitHub*. https://github.com/in-toto/attestation/blob/main/spec/v1/statement.md
62. *attestation/spec/v1/envelope.md at main · in-toto/attestation · GitHub*. https://github.com/in-toto/attestation/blob/main/spec/v1/envelope.md
63. *Artifact attestations*. https://docs.github.com/en/actions/concepts/security/artifact-attestations
64. *Rekor - Sigstore*. https://docs.sigstore.dev/logging/overview
65. *Sigstore Bundle Format - Sigstore*. https://docs.sigstore.dev/about/bundle
66. *Verifiable Credentials Data Model v2.1*. https://www.w3.org/TR/vc-data-model-2.1
67. *Timestamps - Sigstore*. https://docs.sigstore.dev/cosign/verifying/timestamps
