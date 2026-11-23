# cases/

Conformance data for the pure algorithms of hibi v5. Section 16 of
`design/SPEC.md` binds each file to a function. The values were recorded from
hibi v4 at tag `v0.7.0` (commit `04dafae`). The cases where v5 deliberately
differs from v4, listed in SPEC section 16.2, are already updated to the v5
value and are marked `v5-adjusted`. Every case is binding for v5. Two points
that were open when the cases were extracted are settled; "Settled points"
below says how.

## Case shape and origin

Every case has five keys:

| key | meaning |
|---|---|
| `name` | unique within its file |
| `input` | the arguments, in the shape described per file below |
| `expected` | the value v5 must produce |
| `source` | the v4 test the case comes from (`test/<file> › <describe> › <test>`), or `computed: <v4 source file> <function>`. A `v5-adjusted` case appends `| v5-adjusted: <SPEC section>: <how the value was derived>`. |
| `origin` | `copied`, `narrowed`, `computed` or `v5-adjusted` for the cases extracted from v4; `rfc8785`, `independent`, `spec-derived` or `synthetic` for cases added for v5 (see "Files added for v5" and "Cases added to extracted files") |

| origin | meaning |
|---|---|
| `copied` | A v4 test asserts exactly this value. The generator checked that v4 still produces it. |
| `narrowed` | A v4 test asserts a weaker property, such as a range, "not equal", "contains", or only some fields. `expected` is v4's exact output, and the generator checked that it satisfies the test's property. |
| `computed` | No v4 test asserts it. `expected` is v4's output for an input written for this extraction. |
| `v5-adjusted` | The SPEC overrides v4 here. `expected` was derived from v4's output by a mechanical step that `source` names. No value was written by hand. |

The values were produced by running v4's own functions under Bun 1.4.5, from
a checkout of `v0.7.0`. The generator script is kept outside this
repository. Two runs from an empty `cases/` directory gave byte-identical
JSON.

Fixture files under `fixtures/` are plain text and source files. Cases refer
to them by paths relative to this directory.

## Offsets and lengths

- **Byte fields are binding.** Every range has both units. `start` and `end`
  are UTF-16 code units, the unit v4 used. `startByte` and `endByte` are
  UTF-8 byte offsets into the same text. v5 uses the byte fields (SPEC
  section 16). The UTF-16 fields stay for reference. The same holds for
  `loc`/`locByte`, `bias`/`biasByte` and `index`/`indexByte`.
- **Lengths count characters in v5.** v5 counts lengths in Unicode scalar
  values (SPEC section 4.1). This covers the 48-character context, the
  32-character Bitap limit, the error budget and Levenshtein distance. v4
  counted UTF-16 units. The two agree on any text without characters outside
  the Basic Multilingual Plane (BMP).
- **How the v5 values were derived for other text.** For every case whose
  text has characters outside the BMP, v4 was run again on a remapped
  text. Each such character was replaced by one unused private-use BMP
  character, so that one UTF-16 unit equals one character. The results were
  then mapped back to the original text.
  - Where this changed the binding value, the case is `v5-adjusted`.
  - Where it did not, the case keeps its origin. Four cases were re-checked
    this way and are unchanged: `bitap.json`
    `unit-sensitive: index after an emoji is in UTF-16 units`, the two
    `locate.json` `non-ascii:` cases, and `regions.json` `non-ascii offsets`.
- **Stored positions in locate inputs.** In `locate.json`, `input.position`
  is the stored text-position selector. Its offsets refer to the text at
  record time, which is not always `input.text`. These are the cases named
  `recorded selectors:` or `code:`, and `moved quote found by its exact text`.

## Parameters

| SPEC name | v4 constant | value |
|---|---|---|
| `CONTEXT` | `TEXT_QUOTE_CONTEXT` | 48 |
| `AMBIGUOUS_MIN` | `AMBIGUOUS_MIN_QUOTE_LENGTH` | 8 |
| `MAX_BITS` | `MATCH_MAX_BITS` | 32 |
| `THRESHOLD` | `BITAP.matchThreshold` | 0.4 |
| `DISTANCE` | `BITAP.matchDistance` | 100000 |
| `ERROR_RATIO` | `FUZZY_ERROR_RATIO` | 0.4 |
| `ERROR_CAP` | `FUZZY_ERROR_CAP` | 256 |
| stream separator | `SEP` in `src/ast/hash.ts` | U+001F |

v4's `SAME_TEXT_SIMILARITY` (0.9) and `MOVE_AWARENESS_CHARS` (4) were not
extracted, because v5 has no similarity-based freshness and no `moved`
state.

## Files

SPEC section 16.1 names the function each file binds. The shapes are:

| File | `input` | `expected` |
|---|---|---|
| `normalize.json` | `{text}` | string |
| `levenshtein.json` | `{a, b}` | number |
| `similarity.json` | `{a, b}` | number (binary64) |
| `error-budget.json` | `{quoteLength}` | number |
| `within-budget.json` | `{located, exact}` | boolean |
| `bitap.json` | `{text, pattern, loc, locByte}` | `{index, indexByte}`, -1 for no match, or `{error: "PatternTooLong"}` |
| `fuzzy-locate.json` | `{text, quote: {exact, prefix, suffix}, bias, biasByte}` | `{found: false}` or `{found: true, start, end, startByte, endByte}` |
| `locate.json` | `{text, quote: {exact, prefix, suffix}, position?: {start, end, startByte, endByte}}` | `{status: "found" or "ambiguous", how, start, end, startByte, endByte}` or `{status: "missing"}` |
| `quote-selector.json` | `{text, start, end, startByte, endByte}` | `{exact, prefix, suffix, position: {start, end, startByte, endByte}}` or `{error: "empty span"}` or `{error: "span longer than 2048 characters"}` |
| `regions.json` | `{text, path}` or `{fixture, path}` | array of `{kind, text, start, end, startByte, endByte, line, headingPath}` |
| `duration.json` | `{start, duration}` | `{instant}` or `{error: "invalid_duration"}` |
| `recheck-boundary.json` | `{from, duration, now}` | `{expired}` |
| `ast-node.json` | `{fixture, language, quote, start, end, startByte, endByte}` | `{nodeType, start, end, startByte, endByte, semanticStream}` |
| `ast-equivalence.json` | `{a, b}`, each shaped like an `ast-node.json` input | `{semanticHashEqual}` |

Notes per file:

- **Field names.** In `locate.json` and `fuzzy-locate.json`, `quote.exact`
  is the selector's `quote` (SPEC section 5.2). `position.startByte` is its
  `pos`. For `locate.json` an `ambiguous` result carries the earliest tied
  hit as its range, as SPEC section 7.1 says.
- **`bitap.json`.** The pattern is longer than 32 characters only in the
  error case.
- **`regions.json`.** `path` selects the class (SPEC section 6.1):
  - null means `prose`, and so does a `.txt` path (`notes.txt`).
  - `.md` and `.mdx` mean `markdown`, compared lower-cased (`README.MD` is
    markdown).
  - Only the path's extension matters. A fixture's own file name is not
    used.

  `line` and `headingPath` are v4 output. v4 has no `endLine`. Every
  unit's `text` equals the document slice at `startByte..endByte`.
  - `fixtures/regions/pricing.md` is the document in `test/regions.test.ts`.
  - `fixtures/regions/real-*` are the documents in
    `test/regions-real.test.ts`.
  - `fixtures/regions/v4-*.mdx` are whole v4 documentation pages.
- **`duration.json` and `recheck-boundary.json`.** The instants are as v4's
  `toISOString` writes them, with milliseconds. See "Open points".
- **`ast-node.json`.** `semanticStream` is the `ast-v1` stream of SPEC
  section 8.4. `nodeType` and the node range come from v4's snapping. v4
  trims whitespace from the span, calls
  `descendantForIndex(start, max(start, end - 1))` on a UTF-16 tree, and
  climbs to the nearest named node.
- **`ast-equivalence.json`.** `semanticHashEqual` is the equality of the
  `ast-v1` digests of the two snapped nodes. Both sides of every pair have
  the same class, so this is the equality of their streams.
  - The per-language sets in `fixtures/ast/<language>/` compare a `base`
    file with one file per edit. The spans are a constant (`const`), a type
    (`type`, or `jsx` for TSX) and a function (`fn`).
  - The `ast.test:` cases copy `test/ast.test.ts`.
  - In the `precision:` and `languages:` cases, span `b` is where v4's
    locate step finds the recorded quote in the edited file.
- **Grammar versions.** Node types and streams depend on them. v4 used
  web-tree-sitter 0.27.0 with tree-sitter-typescript 0.23.2,
  tree-sitter-python 0.25.0, tree-sitter-rust 0.24.0, tree-sitter-go 0.25.0
  and tree-sitter-java 0.23.5. SPEC section 2.6 pins tree-sitter-rust
  =0.24.2. See "Open points".

## Error labels

The `error` values are labels, not v4's messages.

| label | meaning | v4 behaviour |
|---|---|---|
| `PatternTooLong` | SPEC section 7.3 | `matchMain` throws `Bitap pattern too long` |
| `invalid_duration` | SPEC section 9.4 | `addDuration` throws `not an ISO 8601 duration: …` |
| `empty span` | SPEC section 7.4, error `EmptySpan` of `quote_selector`: "If `quote` is empty after trimming WS, the result is the error `EmptySpan`." The file provider, its only caller, reports it as the provider error `internal` (sections 6.7 and 16.1). | `buildSelectorBundle` throws `refusing to anchor an empty span` |
| `span longer than 2048 characters` | SPEC section 7.4, error `TooLong` of `quote_selector`: a span longer than `MAX_QUOTE_CHARS` (2,048) characters. The empty check runs first. | none: v4 had no length limit |

## Cases changed for v5 (SPEC section 16.2)

All the changes below were applied by the generator.

- **`ast-equivalence.json`.** Nine pairs now have
  `semanticHashEqual: true`. They are `fn: comment-inside` for typescript,
  tsx, python, go and java, and `fn: trailing-comma` for typescript, python,
  rust and go. `tsx fn: prettier-jsx` stays `false`.
- **`ast-node.json`.** 55 cases have a new `semanticStream`:
  - 47 lose comment tokens.
  - 12 lose a trailing `,:,`.
  - 4 of these lose both: the `trailing-comma` function cases of
    typescript, python, rust and go.
  - Each case's `source` lists the removed tokens. The generator checked
    that the new stream is v4's walk with exactly the comment subtrees and
    the trailing commas removed.
- **`levenshtein.json`.** `unit-sensitive: ASCII letter vs emoji is one substitution`
  is now 1.
- **`normalize.json`.** U+FEFF is now kept and U+0085 is now collapsed:
  `BOM U+FEFF is not whitespace` and `NEL U+0085 is whitespace`.
- **`quote-selector.json`.** A new case,
  `non-ascii: context of 48 characters outside the BMP`, stores 48 emoji of
  prefix and 48 of suffix. v4 stored 24 of each.
- **`regions.json`.** The two CRLF cases are fed LF text and re-recorded
  from v4. They are `computed`, because v4 produced the values from the LF
  input.

Renamed cases, because the old name stated something false for v5:

| file | old name | new name |
|---|---|---|
| normalize.json | `BOM U+FEFF is whitespace in JS` | `BOM U+FEFF is not whitespace` |
| normalize.json | `NEL U+0085 is NOT whitespace in JS` | `NEL U+0085 is whitespace` |
| levenshtein.json | `unit-sensitive: two emoji differing only in the low surrogate cost 1 in UTF-16 units` | `unit-sensitive: two different emoji are one substitution` |
| levenshtein.json | `unit-sensitive: ASCII letter vs emoji costs 2 in UTF-16 units` | `unit-sensitive: ASCII letter vs emoji is one substitution` |
| quote-selector.json | `non-ascii: context counted in UTF-16 units` | `non-ascii: context of 48 characters, all in the BMP` |
| regions.json | `frontmatter with CRLF in .md` | `frontmatter in .md (former CRLF case, fed as LF)` |
| regions.json | `CRLF line endings keep \r inside multi-line units` | `multi-line units (former CRLF case, fed as LF)` |

## Settled points

Both were open when the cases were extracted. The affected cases keep the
form they were recorded in, and are binding in that form.

1. **Timestamps.** SPEC section 3.1 allows second precision and `Z` only
   for stored `at` fields and for `now`. The duration functions are not
   bound by that: SPEC section 9.4 says `duration::parse` and
   `duration::add` accept any instant, including one with a fraction of a
   second or an offset, and keep the fraction. So the instants of
   `duration.json` and `recheck-boundary.json` that are written with
   milliseconds (such as `.000Z`, three of them nonzero) and the one
   duration start with a `+02:00` offset are valid inputs, and the tests
   bind them as written.
2. **Rust grammar version.** The Rust streams were recorded with
   tree-sitter-rust 0.24.0. SPEC section 2.6 pins 0.24.2, and every Rust
   case passes with 0.24.2, so the recorded streams are binding as they
   are.

## Left out, and why

- **The 0.9 similarity rule and everything built on it.** This covers the
  `unchanged`/`moved`/`changed` states, the move threshold, the
  numeric-token rule, reason labels, state ranks and the live-quote
  widening. v5 decides freshness by exact fingerprint equality.
- **The `value` selector and value extraction.** v5 drops them.
- **The structural hash and the xxHash64 values.** v5 keeps only the
  semantic stream and hashes it with BLAKE3.
- **v4's expiry counting rule, expiry labels and the gating, judge and
  conflict tests.**
- **An empty quote.** v4 refuses it at record time.

## v4 behaviour that looked wrong or surprising

This is the list from the extraction, with what v5 does now.

1. **Empty stored context.** An empty prefix scores as a perfect context
   match at the start of the text, and an empty suffix at its end. Kept, as
   SPEC section 7.1 intends. See `locate.json`
   `empty stored context scores a hit at the text start or end as a perfect context match`.
2. **A no-op strip in v4's `normalizeText`.** v4 strips leading whitespace
   per line, which has no effect. Its result always equals collapsing
   whitespace runs, checked on 200,000 random strings. v5's `normalize` is
   defined without the strip.
3. **Whitespace set.** v4 used JavaScript's `\s`. Changed: v5 uses Unicode
   White_Space.
4. **UTF-16 lengths.** v4 counted lengths in UTF-16 units, so a stored
   prefix could split a character. Changed: v5 counts characters.
5. **A digit appended to a quoted number is still an exact hit.**
   `MAX_ATTEMPTS = 5` is found inside `MAX_ATTEMPTS = 50`. Only the
   fingerprint sees the change. See `ast-equivalence.json`
   `precision: literal-5-to-50`.
6. **Comments inside the hashed node.** v4 hashed them in five of six
   languages. Changed: `ast-v1` skips comments.
7. **Trailing commas.** v4 hashed a trailing comma. Changed: `ast-v1` skips
   it. Other punctuation a formatter adds still counts: `tsx fn: prettier-jsx`.
8. **Content literals match by type name**, including the TypeScript
   keywords `number` and `string`. Kept, per SPEC section 8.4.
9. **Fences swallow the prose of their block.** Kept.
10. **A table separator row without a leading pipe is glued to the header
    row.** Kept.
11. **Table cells are split into sentences, and a `---` line under text is
    a thematic break.** Kept.
12. **CRLF.** v4 kept `\r` inside units. Changed: the provider converts CRLF
    to LF before splitting (SPEC section 4.2).
13. **The Bitap threshold is inclusive, and quotes of one or two characters
    have an error budget of 0.** Kept.
14. **Ambiguous results carry the earliest tied range.** Kept.

## Files added for v5 (SPEC section 16.1)

These files were not extracted from v4. Their cases, and the cases added to
extracted files (next section), use four more `origin` values.

| origin | meaning |
|---|---|
| `rfc8785` | Input and expected value are taken from RFC 8785 itself. |
| `independent` | `expected` was computed by implementations that share no code with the Rust crate: the npm packages `canonicalize` 5.1.0 (JCS) and `@noble/hashes` 2.4.0 (pure-JavaScript BLAKE3), with the 100-bit base32 step written bit by bit, and, for `bitap.json`, the npm package `diff-match-patch` 1.0.5 under Bun 1.4.5. |
| `spec-derived` | `expected` was derived by hand from the SPEC section that `source` names, and `source` gives the derivation step by step. |
| `synthetic` | A judge request and response pair written by hand in the shapes the vendor documents, not recorded from the live Jev API. Used only under `judge/`, until recorded pairs replace it. |

| File | `input` | `expected` |
|---|---|---|
| `canon.json` | `{json, record}`: JSON text, and `null` or the record type (`claim`, `mark`, `retirement`) it must deserialize as | `{canonical, utf8}`: the RFC 8785 serialization as a string and as hex UTF-8 bytes |
| `ids.json` | `{kind, body}`: a record type and a record body without `id` | `{canonical, blake3, id}`: the canonical body, the 64 hex digits of its BLAKE3 digest, and the id of SPEC section 3.5 |
| `fingerprint.json` | `{profile: "prose-v1", text}` or `{profile: "ast-v1", class, stream}` | `{fingerprint}`: the string of SPEC section 8 |
| `units-code.json` | `{fixture, path}`: a file under `fixtures/units/` and a path whose extension selects the class | `{class, units}`: the class, and every unit in source order (start ascending, then end descending) as `{kind, symbol, key, parent, start, end, line, endLine}`. `endLine` is the line of the unit's last character. A content key (`statement`, `block`) is written `<kind>:*`, with `~n` for a duplicate, and matches any 12 lower-case hex digits there. |
| `resolve.json` | `{path, fixture or text, sel}`: the locator path (its extension selects the class), the file's content from a file under `fixtures/resolve/` or inline (inline text keeps CR LF and a BOM byte for byte), and the selector. With neither `fixture` nor `text` the file does not exist. | `{units}`, every unit `resolve` returns in order as `{key, approx, identity, refuse, sel}`, where `identity` and `refuse` are `null` when absent and `sel` is the completed selector of a unit returned alone and the printed anchor of every other unit; or `{error}` with a provider error code. Content keys are written `<kind>:*` as in `units-code.json`. |
| `freshness.json` | `{recheck, retired, now, marks: [{at, sides: [fingerprint]}], sides}`: the claim's `recheck`, whether it is retired, `now`, its marks, and the current resolution of each side as `{unit: <fingerprint>}`, `{units: 0}` (no unit), `{units: 2}` (several), `{error: <provider error code or "failed">}`, `{noProvider: "scheme_not_declared" or "provider_not_registered"}` or `{invalidUnit: true}`. A retired claim has no `sides`. | `{state, mark, expiresAt, sides: [{state, reason}]}`: the claim state, the reference mark as an index into `input.marks` (`null` for none, `{greaterIdOf: [indexes]}` for a tie on `at`), `expiresAt` of SPEC section 15.2, and each side's state and reason |
| `coverage.json` | `{loc, units: [{key, kind, parent}], kinds, resolved, exclude: [{resource, unit}]}`: one resource, its unit list, the scope's kinds, the keys that claim sides resolve to, and the scope's exclusions in declaration order | `{covered, excluded, uncovered, matched}`: the scope units by status, each list in unit order, and the number of units each exclusion excluded |

The `resolve.json` cases are `spec-derived`. Their expectations were derived
by hand from SPEC section 6.7 and the text, in the steps each `source`
lists: which units, `approx`, `identity`, `refuse` and `within`, and the
span of each quote inside its unit. A script then cut the in-unit windows
(the last and first 48 characters of the normalized unit text before and
after the quote, trimmed) from that span. Nothing was copied from the
implementation's output. The test
resolves each case through `FileProvider` on a temporary working tree, then
again without a cache and twice through a cache, and requires the same
answer every time (R40, R53).

The `fingerprint.json` digests were computed with `@noble/hashes` 2.4.0
under Bun, with NFC from `String.prototype.normalize` and the WS set of SPEC
section 4.1 written out as a character class. The `ast-v1` streams are
copied from `ast-node.json`, or written for the case.

The `units-code.json` cases are `spec-derived`. Their expectations were
derived by hand from the fixture sources, SPEC section 6.4 and the grammars' `grammar.js` and
`node-types.json`, without running a parser, and then compared with the
implementation. The binding also checks every content key against the
unit's own fingerprint: `<kind>:` plus the first 12 hex digits of its digest,
with `~n` on the n-th duplicate in source order.

Rulings on section 6.4 made after these cases were written changed six of
them. They stay `spec-derived`, because the new values were derived by hand
as well. Each `source` names the ruling and the edit after
`adjusted by a later ruling:`.

| case | ruling | edit |
|---|---|---|
| `rust: impl naming, trait and mod members, struct and enum members, an outer attribute in its item's unit, use as a statement` (was `... attributes and use as statements`) | outer attributes belong to the item; members are units; `field` | the `#[derive(Debug)]` statement is gone and `struct:Cache` starts at it; fields `Cache.size`, `Bits.i`, `Bits.f`; variants `Mode.Fast`, `Mode.Slow`; `Store.LIMIT` and `Cache.EMPTY` are `field` |
| `java: field declarators, overloads, enum body declarations, record, annotation type` | annotation elements are methods; members are units; `field` | `method:Marker.value`; variants `Mode.FAST`, `Mode.SLOW`; record components `Point.x`, `Point.y`; every class, interface and enum field declarator is `field` |
| `typescript: wrappers, containers, overloads, bare namespace` | members are units; `field` | variants `Color.Red`, `Color.Green`; `Shape.name`, `Base.label`, `Cache.count` are `field` |
| `tsx: jsx components, var with two declarators, class fields` | `field` | `Panel.state`, `Props.title` are `field` |
| `python: decorated definitions, assignments, nested classes, duplicate def` | `field` | the class-body assignments are `field` |
| `go: grouped and single specs, var_spec_list groups, method receivers, duplicate init` | members are units | `field:Cache.items` |

Cases added for those rulings, `spec-derived`: one per language for the
members of named containers (`fixtures/units/members.*`; the TypeScript
fixture is also read as TSX), and `rust: modules 33 deep` for
`MAX_CONTAINER_DEPTH` (`fixtures/units/depth.rs`). `resolve.json` gained six
`rust members:` cases on `fixtures/resolve/shapes.rs`: a quote on a field or
variant line resolves to that member, a quote on the header line to the
container.

Cases added for later rulings, `spec-derived`, derived by hand the same
way: `java: a varargs record component is a field named by its declarator`
(`fixtures/units/record-varargs.java`); member forms that already worked:
TypeScript and TSX member names and construct signatures
(`member-names.ts`, read as both classes), Java initializer blocks and
enum-constant body fields (`initializers.java`), Go type sets and embedded
generic fields (`type-sets.go`), Python augmented and multi-name assignments
in a class body (`class-statements.py`); `MAX_CONTAINER_DEPTH` for Python
and Java (`depth.py`, `depth.java`), and for Go a case showing that the cap
cannot be reached, because a field's struct type is not a container
(`nested-struct.go`).

Cases added for the ruling on shared declarations, `spec-derived`, derived
by hand: `go: grouped const, var and type specs are units of their own; a
spec without an expression is a unit` (`fixtures/units/shared.go`) and
`rust: the leading attribute of an extern block with several items belongs
to its items, not to a statement` (`fixtures/units/shared.rs`). units-code
has no fingerprints, so the fingerprint effects of that ruling are pinned by
unit tests in `code.rs`.

`resolve.json` was rewritten for the identity rule of SPEC section 6.7
(decision 142). It pins every branch of `resolve_new` and `resolve_stored`,
the printed anchors, and the scenarios the rule was designed against (each
case is named after its scenario). These groups of the earlier rule were
removed, because the rule they pinned no longer exists:

- context scoring of stored anchors (`stored anchor:` cases where the
  stored prefix, suffix or position chose between equal hits): windows are
  now cut inside the unit and either reproduce or not, and no position is
  stored;
- `duplicated declarations` and `twin declarations` cases with scope and
  whole-text contexts: duplicated symbols are now resolved by `count` and
  the quote's in-unit windows, and identical twins cannot be told apart;
- candidates ordered by context score and the `stored context:` cases of
  `contextChanged`: there is no score and no `contextChanged`;
- the `missed symbol:` case and the moved-function case on `report.py`: a
  symbol that names nothing gives every declaration of it (none) and the
  units of its quote's exact hits, and the engine adds candidates by content;
- default quotes in completed selectors: a typed `#name` stores no quote.

Decision 144 (a code file that does not parse is `unavailable` for every
selector, with no fallback to `block` units) removed two more cases:
`python with a syntax error: a quote: the text block` in `resolve.json`,
which became `... a quote: unavailable`, and
`python with a syntax error: class text, block units` in `units-code.json`,
because `code::analyze` no longer returns block units for a code class.
`resolve.json` gained the `python with a syntax error:` cases for stored
anchors on a broken file and their recovery on the fixed text.

The rulings that store `within` whenever a unit has a container, with
`count` when the path names several, and that mark a unit no printed
anchor selects alone (`refuse: quote_not_distinguishable`), changed six
cases, each derived again by hand from the new rule; three were renamed to
say what they now pin:

- `markdown: a quote in a piece whose name is shared: no within` became
  `... within with count 2`: the completed selector holds the shared piece
  name and `count: 2`;
- `markdown: a quote in two identical sentences: ambiguous, identical printed
  anchors` became `... each marked as not selectable alone`, and
  `markdown: stored: within names two containers: candidates` became
  `... count says one: candidates`; in these and in
  `markdown: stored: two units reproduce the stored windows: ambiguous` the
  two identical sentences carry `within` with `count: 2` and the mark;
- `Go twin init: a new anchor gives both, with identical printed anchors`
  became `... each marked as not selectable alone`, and
  `Go twin init: the printed anchor cannot tell them apart: candidates`
  marks both twins.

`resolve.json` gained a case for each repro of those rulings: the cases
named `markdown B1:`, `typescript B1:`, `markdown B3:`, `python B4:`,
`python I1:` and `markdown I1:`, a typed quote longer than the limit only by
its whitespace, a symbol anchor whose quote spans two members, and a quote
in front matter. Each stored selector in them is the completed selector of
the claim-time text, derived by hand, resolved against the edited text.

The next rulings (a stored quote in a comment is not resolved in P either;
a refresh keeps the stored `within`; a printed anchor is also tried with
the unit's section alone) changed no case and added fifteen: the cases named
`java F1:`, `python F1:`, `rust F1:` and `typescript F1:`, two on a sentence
whose piece name reads as a subsection, and one on a refresh that keeps its
`within`.

The ruling that in-unit windows in code read every comment as one space
changed the expected value of fifteen cases, each derived again by hand
under it: the generator states each unit's comments by their text, read
off the fixture, and the windows replace each with a space. The cases are
the six `rust:` cases on `retry` and `twice` whose windows held `// cap
here` or `// step(1); first`, `typescript: quote in a comment: refused`,
the three `python:` cases on `area` whose windows held `# pi times r
squared`, `python B4: A2: ...`, and the four `... F1:` claim cases in Java,
Python and Rust. Five stored cases whose input is one of those completed
selectors changed their input with it and kept their expected value. Four
cases were added: a quote next to a comment and its stored form after the
comment is edited, the same for two declarations (A2), and headings that
differ by an intraword underscore.

The cases kept from the earlier file (the empty selector, symbols, quotes,
refusals, members, boundaries, decoding, errors) were derived again for the
new selector shape.

The `freshness.json` and `coverage.json` cases are `spec-derived`: each
`expected` was written by hand from SPEC sections 9.2, 9.3 and 10.2, and
`source` gives the rule applied. They bind `engine::evaluate`, and
`engine::covered_keys` with `engine::classify`.

### `judge/`: synthetic, not recorded

The three files under `judge/` are **synthetic**. They were written by hand
from SPEC sections 14.2 to 14.5 and from the answer schemas of the vendor's
OpenAPI document (`https://api.typesafe.ai/openapi.json`, read on
2026-11-20). No case was recorded from the live Jev API. Every case has
origin `synthetic`, and its `source` names the SPEC sections and schemas it
was written from. SPEC section 16.3 names request and response pairs
recorded from the live judge API as the one kind of case data still
missing, and these hand-written files stand in for them until someone with
a `TYPESAFE_API_KEY` records real ones.

| file | `input` | `expected` |
|---|---|---|
| `judge/triage.json` | `{document, units: [{headings, text}], response}`: the document locator, the units of one request in order, and a response body | `{request, answers, results}`: the exact request body, each answer as hibi reads it (`{noul}`, `{choice, p}`, or `null` when unusable, which fails the request), and per unit `{checkable, band, evidence}`, or `null` when an answer is unusable |
| `judge/suggest-evidence.json` | `{statement, candidates: [{where, code}], response}` | `{request, answers, results: [{p, fits}], noneFits}`, with `results` and `noneFits` `null` when an answer is unusable |
| `judge/order.json` | `{statement, content, response}`: one stale claim's statement and side texts | `{request, answers}` |

They bind `judge::questions` (the request shapes) and `judge::read_answer`
with `judge::Band` and `judge::EvidenceLabel` (reading the answers). Packing
into requests, the cache and the transport are tested against a local
scripted server in `crates/hibi/tests/judge/`.

The four `rfc8785` cases of `canon.json` are the primitive-types sample of
RFC 8785 section 3.2.2 with the bytes of section 3.2.4, the sorting sample of
section 3.2.3, the number samples of appendix B, and the appendix B row
`43143ff3c1cb0959` (`1424953923781206.2`, "Round to even", note 4) as a case
of its own. Every other appendix B row is in the number-samples case. The
NaN and Infinity rows are left out because they are not JSON. The record
cases of `canon.json` are the records of `ids.json` written with indentation
and keys in reverse order.

## Cases added to extracted files

These cases pin steps of SPEC sections 6.3, 7 and 8.4 that no extracted
case exercised. Each one was checked to fail when its step is changed.

| file | cases | origin | step pinned |
|---|---|---|---|
| `fuzzy-locate.json` | `collapsed_end: the last index scanned is at + 2 * len(quote) + 15`, `collapsed_end: index at + 2 * len(quote) + 16 is not scanned` | `spec-derived` | the `collapsed_end` limit `at + 2 * len(quote) + 16` (section 7.2) |
| `fuzzy-locate.json` | `prefix fallback matches the last 32 characters of the prefix` | `spec-derived` | the prefix fallback takes the last `MAX_BITS` characters (section 7.2) |
| `fuzzy-locate.json` | `bias is clamped to len(text) - 1: a candidate exactly at the distance limit is found`, `bias is clamped to len(text) - 1: a candidate one past the distance limit is not found` | `spec-derived` | `bias = clamp(pos, 0, len(text) - 1)` (section 7.2) |
| `bitap.json` | `equal exact matches at equal distance on both sides of loc: the left one wins` | `independent` | the left edge `max(1, 2 * loc - best)` after a match right of `loc` is inclusive (section 7.3) |
| `bitap.json` | `the only exact occurrence is more than 40000 characters from loc: no match` | `independent` | the initial threshold is the minimum of the exact-match scores and `THRESHOLD`, never more (section 7.3) |
| `quote-selector.json` | `a span of exactly 2048 characters is valid`, `a span of 2049 characters is refused as too long`, `a whitespace span of 2049 characters is empty, not too long` | `spec-derived` | `MAX_QUOTE_CHARS` and the order of the `EmptySpan` and `TooLong` checks (section 7.4) |
| `regions.json` | `four-backtick fence is not closed by a three-backtick line` | `spec-derived` | a fence closes only on a run of its marker at least as long as the opener (section 6.3) |
| `regions.json` | `a fence line with an info string does not close an open fence` | `spec-derived` | a closing fence holds only the marker character (section 6.3) |
| `regions.json` | `BOM before --- frontmatter does not prevent recognition` | `spec-derived` | a leading U+FEFF does not prevent frontmatter recognition (section 6.3) |
| `regions.json` | `+++ TOML frontmatter is skipped in .md` | `spec-derived` | `+++` frontmatter is recognised like `---` (section 6.3) |
| `regions.json` | `+++ frontmatter is not closed by ---` | `spec-derived` | frontmatter closes only on its own delimiter (section 6.3) |
| `regions.json` | `thematic breaks with spaces or tabs between the characters separate blocks` | `spec-derived` | a thematic break may have spaces or tabs between its characters (section 6.3) |
| `regions.json` | `mixed characters are not a thematic break` | `spec-derived` | a thematic break uses one character (section 6.3) |
| `regions.json` | `closing # run needs whitespace before it` | `spec-derived` | a closing `#` run is stripped only after WS (section 6.3) |
| `regions.json` | `closing # run after whitespace is stripped; a heading of only a # run has empty text` | `spec-derived` | heading text with a closing `#` run, and a heading of only a `#` run (section 6.3) |
| `regions.json` | `frontmatter delimiters with trailing whitespace` | `spec-derived` | frontmatter delimiters are compared with trailing WS removed (section 6.3) |
| `regions.json` | `two backticks are not a fence` | `spec-derived` | a fence needs at least three marker characters (section 6.3) |
| `regions.json` | `tag line needs a letter first and whitespace before attributes` | `spec-derived` | the tag-line pattern (section 6.3) |
| `regions.json` | `two dashes are not a thematic break` | `spec-derived` | a thematic break needs at least three characters (section 6.3) |
| `regions.json` | `> without a following space starts a piece` | `spec-derived` | `>\s?` starts a piece (section 6.3) |
| `regions.json` | `table separator whose last cell has no trailing pipe is dropped` | `spec-derived` | the optional last cell of the table separator (section 6.3) |
| `regions.json` | `+ is a list marker at a line start` | `spec-derived` | `+` as a list marker (section 6.3) |
| `regions.json` | `list marker needs whitespace after it` | `spec-derived` | a list marker needs WS after it (section 6.3) |
| `regions.json` | `endLine of multi-line units` | `spec-derived` | `endLine` is the line of the unit's last character (sections 6.2 and 6.3); these new `regions.json` cases carry `endLine`, which the binding compares when present |
| `ast-node.json` | the eight `pins: ... is a content literal` cases (`fixtures/ast/pins/`) | `spec-derived` | `raw_string_literal`, `char_literal`, `float_literal` (Rust), `rune_literal`, `float_literal`, `imaginary_literal`, `raw_string_literal` (Go) and `float` (Python) push `=` and the normalized text (section 8.4) |
| `ast-equivalence.json` | the thirteen `pins: rust …` and `pins: python …` cases on `fixtures/ast/tuples/` (`(u8,)` vs `(u8)`, tuple pattern, tuple expression, call arguments, two-element tuple, tuple struct fields; `x[1,]`, two-element subscript, tuple pattern, case pattern, call arguments, tuple, list) | `spec-derived` | the only `,` of a tuple-like node (Rust `tuple_expression`, `tuple_type`, `tuple_pattern`; Python `tuple`, `tuple_pattern`, `subscript`) is kept before the closing delimiter; any other trailing comma is still skipped (section 8.4) |
| `ast-node.json` | `pins: rust the comma of (u8,) is in the stream`, `pins: rust the comma of (1,) is in the stream`, `pins: python the comma of x[1,] is in the stream`, `pins: python the comma of (1,) is in the stream` | `spec-derived` | the same rule, shown in the stream (section 8.4) |
| `ast-equivalence.json` | `pins: rust a comma before > is skipped`, `pins: rust a comma before ] is skipped`, `pins: rust a comma followed by a comment and then ) is skipped`, `pins: rust a changed element is not skipped (control)` | `spec-derived` | the trailing-comma rule for `>` and `]`, and its "not counting comment nodes" clause (section 8.4) |

The `spec-derived` values were also computed by section 7.2 written in
JavaScript over `diff-match-patch` 1.0.5 `match_main`. Both agree. All text
in these cases is in the BMP.

Two steps of section 7.3 cannot be pinned by any case: moving `start` to
`max(1, 2 * loc - best)`, and the initial threshold tightening from the
exact matches. `diff-match-patch` marks both as speed-ups, and removing
either one does not change `match_main`'s result. A comparison of
`diff-match-patch` with a copy that lacks one or both steps found no
difference on 300,000 random inputs. The cases above pin the boundaries of
those steps instead: a `start` further right, or a threshold above
`THRESHOLD`, changes the result.
