# cases/

Conformance data for the pure algorithms of hibi v5. Section 16 of
`design/SPEC.md` binds each file to a function. The values were recorded from
hibi v4 at tag `v0.7.0` (commit `04dafae`). The cases where v5 deliberately
differs from v4, listed in SPEC section 16.2, are already updated to the v5
value and are marked `v5-adjusted`. Every case is binding for v5, except the
points listed under "Open points".

## Case shape and origin

Every case has five keys:

| key | meaning |
|---|---|
| `name` | unique within its file |
| `input` | the arguments, in the shape described per file below |
| `expected` | the value v5 must produce |
| `source` | the v4 test the case comes from (`test/<file> › <describe> › <test>`), or `computed: <v4 source file> <function>`. A `v5-adjusted` case appends `| v5-adjusted: <SPEC section>: <how the value was derived>`. |
| `origin` | `copied`, `narrowed`, `computed` or `v5-adjusted` |

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
| `quote-selector.json` | `{text, start, end, startByte, endByte}` | `{exact, prefix, suffix, position: {start, end, startByte, endByte}}` or `{error: "empty span"}` |
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
| `empty span` | SPEC section 16.1, "fails on a span that is empty after trimming". The SPEC names no error code for it. | `buildSelectorBundle` throws `refusing to anchor an empty span` |

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

## Open points

These were not decided here. The affected cases still hold v4's form.

1. **Timestamps.** SPEC section 3.1 allows second precision and `Z` only.
   - Every expected instant and many input instants in `duration.json` and
     `recheck-boundary.json` are written with milliseconds, such as `.000Z`.
   - Three inputs have nonzero milliseconds: two duration starts and one
     boundary `now`.
   - One duration start has a `+02:00` offset.
2. **Rust grammar version.** The Rust streams were recorded with
   tree-sitter-rust 0.24.0. SPEC section 2.6 pins 0.24.2.

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

## Files added for v5 (SPEC section 16.3)

These files were not extracted from v4. Their cases use two more `origin`
values.

| origin | meaning |
|---|---|
| `rfc8785` | Input and expected value are taken from RFC 8785 itself. |
| `independent` | `expected` was computed by implementations that share no code with the Rust crate: the npm packages `canonicalize` 5.1.0 (JCS) and `@noble/hashes` 2.4.0 (pure-JavaScript BLAKE3), with the 100-bit base32 step written bit by bit. |

| File | `input` | `expected` |
|---|---|---|
| `canon.json` | `{json, record}`: JSON text, and `null` or the record type (`claim`, `mark`, `retirement`) it must deserialize as | `{canonical, utf8}`: the RFC 8785 serialization as a string and as hex UTF-8 bytes |
| `ids.json` | `{kind, body}`: a record type and a record body without `id` | `{canonical, blake3, id}`: the canonical body, the 64 hex digits of its BLAKE3 digest, and the id of SPEC section 3.5 |

The three `rfc8785` cases of `canon.json` are the primitive-types sample of
RFC 8785 section 3.2.2 with the bytes of section 3.2.4, the sorting sample of
section 3.2.3, and the number samples of appendix B (the NaN and Infinity
rows are left out because they are not JSON). The record cases of
`canon.json` are the records of `ids.json` written with indentation and keys
in reverse order.
