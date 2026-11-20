# hibi cookbook

Worked tasks. Every output below was copied from a real run of hibi 5.0.0 on
a small example repository with `docs/retry.md` and `src/retry.rs`. Outputs
are shown in the human format; in a shell tool without a terminal the same
facts come as one JSON object. Blocks marked `output` are output, not
commands.

## 1. Set up hibi in a repository

```bash
hibi init
```

Then declare what must be covered by appending a scope to `.hibi/hibi.toml`:

```toml
[[scope]]
name = "docs"
resource = "file:docs/**/*.md"
kinds = ["sentence", "list-item", "table-row"]
```

```bash
hibi cover
```

```text output
docs (file:docs/**/*.md; sentence, list-item, table-row): 3 units, 0 covered, 0 excluded, 3 uncovered
  file:docs/retry.md
        3  sentence  The client retries a failed download.
        4  sentence  It gives up after 5 attempts.
        5  sentence  The delay between attempts doubles each time, starting at 200 ms.
```

Commit `.hibi/`.

## 2. Record a statement you checked

You read `src/retry.rs` and confirmed `MAX_ATTEMPTS` is 5:

```bash
hibi claim "The client gives up after 5 attempts." \
  --subject "file:docs/retry.md::gives up after 5 attempts" \
  --evidence "file:src/retry.rs#MAX_ATTEMPTS"
```

```text output
c1-bvy8mfb476vwm9ftpx44  created  subject file:docs/retry.md (sentence "It gives up after 5 attempts.", line 4); evidence 1 file:src/retry.rs (variable "MAX_ATTEMPTS", line 2)
```

## 3. Start of a session: a statement is no longer true

```bash
hibi status
```

```text output
1 claim: 0 fresh, 1 stale, 0 expired, 0 retired

c1-bvy8mfb476vwm9ftpx44  stale  The client gives up after 5 attempts.
  evidence 1  file:src/retry.rs  changed (fingerprint_differs)  variable "MAX_ATTEMPTS", line 2
  next: hibi mark c1-bvy8mfb476vwm9ftpx44

Re-check each statement before running its next command.
```

Re-check: the code now says `MAX_ATTEMPTS: u32 = 3`. The statement is false,
so do not mark. If you know the code is right (the user changed it on
purpose), fix the document to "It gives up after 3 attempts." and replace the
claim. If you do not know which is right, ask the user.

```bash
hibi reanchor c1-bvy8 \
  --statement "The client gives up after 3 attempts." \
  --subject "file:docs/retry.md::gives up after 3 attempts"
```

```text output
c1-bvy8mfb476vwm9ftpx44 is retired and replaced by c1-dryhtmnz5njjpp2ppy6t
  mark m1-jqkfdpc9r6xa87gh92gs, retirement r1-8drhdychv0qew0t6s13j
  subject  {"loc":"file:docs/retry.md","sel":{"quote":"gives up after 3 attempts","prefix":"tries\n\nThe client retries a failed download.\nIt ","suffix":".\nThe delay between attempts doubles each time, ","pos":52}}
  evidence 1  {"loc":"file:src/retry.rs","sel":{"quote":"pub const MAX_ATTEMPTS: u32 = 3;","prefix":"a download is tried before the client gives up.\n","suffix":"\n\n/// The delay before attempt `n`, in milliseco","pos":62,"symbol":"MAX_ATTEMPTS"}}
```

```bash
hibi status
```

```text output
2 claims: 1 fresh, 0 stale, 0 expired, 1 retired
nothing stale
```

## 4. Claim an uncovered sentence from a batch

Anchors in a batch are compact strings or objects. A `cover` item's `anchor`
object can be used as `subject` unchanged.

```bash
hibi claim --stdin <<'EOF'
[{"statement": "The delay doubles each time, starting at 200 ms.",
  "subject": {"loc": "file:docs/retry.md",
              "sel": {"quote": "The delay between attempts doubles each time, starting at 200 ms."}},
  "evidence": ["file:src/retry.rs#delay_ms"],
  "by": "agent:claude"}]
EOF
```

```text output
c1-rj6kbh7y1yw79ywz4msv  created  subject file:docs/retry.md (sentence "The delay between attempts doubles each time, starting at 2…", line 5); evidence 1 file:src/retry.rs (function "delay_ms", lines 5-7)
```

If any element fails, nothing is written and each failure carries its index:

```json output
{"ok":false,"command":"claim","errors":[{"code":"side_missing","message":"the subject (file:docs/nope.md) resolves to no unit (resource_not_found): not_found: file:docs/nope.md: no such file","remediation":"Point the anchor at text that exists in the resource.","index":1}]}
```

## 5. A refactor that keeps behaviour

On branch `tidy-retry`, `200 * 2u64.pow(n)` became `200 << n`:

```bash
hibi status --since main
```

```text output
3 claims: 1 fresh, 1 stale, 0 expired, 1 retired
since main (base 0975387b5c7ae039d9730f55063277ccbe3f35a6)

c1-rj6kbh7y1yw79ywz4msv  stale  The delay doubles each time, starting at 200 ms.
  evidence 1  file:src/retry.rs  changed (fingerprint_differs)  function "delay_ms", lines 5-7
  next: hibi mark c1-rj6kbh7y1yw79ywz4msv

Re-check each statement before running its next command.
```

Re-check: `200 << n` equals `200 * 2^n`. The statement holds:

```bash
hibi mark c1-rj6k --note "200 << n equals 200 * 2^n" --by agent:claude
```

```text output
c1-rj6kbh7y1yw79ywz4msv  marked  m1-qbc53e2ce40fx505bvkz
```

## 6. New text on a branch with no claim

The branch also added a "Timeouts" section:

```bash
hibi cover --since main
```

```text output
since main (base 0975387b5c7ae039d9730f55063277ccbe3f35a6)
docs (file:docs/**/*.md; sentence, list-item, table-row): 1 unit, 0 covered, 0 excluded, 1 uncovered
  file:docs/retry.md
        9  sentence  Each attempt times out after 10 seconds.
```

The timeout is set by an HTTP library, not by code in this repository, so
there is no evidence to anchor. Record it with a recheck period and a note
on where the truth lives:

```bash
hibi claim "Each attempt times out after 10 seconds." \
  --subject "file:docs/retry.md::times out after 10 seconds" \
  --recheck P90D \
  --note "The timeout is set by the HTTP client defaults; check its release notes."
```

When the period runs out the claim is listed as expired:

```text output
c1-1y9x18za6d6qb7q5q3nx  expired  Each attempt times out after 10 seconds.
  recheck P90D ran out at 2026-10-30T09:00:00Z
  next: hibi mark c1-1y9x18za6d6qb7q5q3nx
```

## 7. A sentence was deleted

```bash
hibi status
```

```text output
3 claims: 1 fresh, 1 stale, 0 expired, 1 retired

c1-dryhtmnz5njjpp2ppy6t  stale  The client gives up after 3 attempts.
  subject  file:docs/retry.md  missing (selector_not_found)
  next: hibi reanchor c1-dryhtmnz5njjpp2ppy6t --subject <anchor>

Re-check each statement before running its next command.
```

If the sentence moved, give its new anchor with `--subject`. Here it was
removed on purpose, so retire the claim:

```bash
hibi retire c1-dryh --note "The docs no longer state the attempt limit."
```

```text output
c1-dryhtmnz5njjpp2ppy6t  retired  r1-peqhq128s20p7rrw1e2p
```

## 8. A sentence was reworded slightly

"doubles each time" became "doubles every time". The stored quote is found
only approximately:

```text output
c1-xc82944rsnhdf8wancz9  stale  The delay doubles each time, starting at 200 ms.
  subject  file:docs/retry.md  changed (fingerprint_differs), found approximately  sentence "The delay between attempts doubles every time, starting at …", line 5
  next: hibi mark c1-xc82944rsnhdf8wancz9
```

The meaning is unchanged, so mark it. `mark` shows the text it accepted:

```bash
hibi mark c1-xc82 --by agent:claude
```

```text output
c1-xc82944rsnhdf8wancz9  marked  m1-rt3t4bhwerew2sgyy0ag
  subject  file:docs/retry.md  same, found approximately  sentence "The delay between attempts doubles every time, starting at …", line 5
    | The delay between attempts doubles every time, starting at 200 ms.
  next: hibi reanchor c1-xc82944rsnhdf8wancz9
```

Then store an exact selector again:

```bash
hibi reanchor c1-xc82
```

```text output
c1-xc82944rsnhdf8wancz9 is retired and replaced by c1-5zahcae904kx9wzy1331
  mark m1-q04aqk8nk4dtbepd8xke, retirement r1-e8ydptkkdv1rw8r9v29f
  subject  {"loc":"file:docs/retry.md","sel":{"quote":"The delay between attempts doubles every time, starting at 200 ms.","prefix":" failed download.\nIt gives up after 3 attempts.\n","suffix":"\n\n## Timeouts\n\nEach attempt times out after 10 s","pos":79}}
  evidence 1  {"loc":"file:src/retry.rs","sel":{"quote":"pub fn delay_ms(n: u32)","prefix":" The delay before attempt `n`, in milliseconds.\n","suffix":"\n    -> u64\n{\n    // doubles each time\n    200 *","pos":181,"symbol":"delay_ms"}}
```

## 9. Anchor to a struct field, not its doc comment

A quote inside a doc comment is refused, because comments are not part of the
fingerprint:

```text output
$ hibi claim "A job runs at most 3 times." --subject "file:README.md::at most 3 times" --evidence "file:src/config.rs#Config::Seconds before a request"
error: quote_in_comment: evidence 1 (file:src/config.rs): the quote lies wholly or partly inside a code comment
  fix: Anchor the code the comment describes.
```

Anchor the field itself by its symbol:

```text output
$ hibi claim "Timeout is 30" --subject "file:README.md::at most 3 times" --evidence "file:src/config.rs#Config.timeout_secs"
c1-0v8jc46fz77efrkx852p  created  subject file:README.md (sentence "A job runs at most 3 times.", line 3); evidence 1 file:src/config.rs (field "Config.timeout_secs", line 5)
```

A Python docstring is a string and can be quoted:
`file:src/jobs.py#Job::Jobs run at most 3 times`.

## 10. A unit served by an external provider

The repository declares the scheme in `.hibi/hibi.toml` (`[schemes.ref]`) and
the user registers its command in their own user config. A selector other
than the empty one is given as JSON:

```bash
echo '[{"statement": "Downloads are retried at most 3 times.", "subject": {"loc": "ref:notes", "sel": {"key": "n1"}}, "evidence": ["file:src/retry.rs#MAX_ATTEMPTS"]}]' | hibi claim --stdin
```

```text output
c1-cbv9mtv2w2g3zh2bk056  created  subject ref:notes (note "Downloads are retried at most 3 times."); evidence 1 file:src/retry.rs (variable "MAX_ATTEMPTS", lines 2-3)
```

Without the user's registration the side is unavailable and nothing runs.
Tell the user; do not edit their config unasked:

```text output
c1-cbv9mtv2w2g3zh2bk056  stale  Downloads are retried at most 3 times.
  subject  ref:notes  unavailable (provider_not_registered)
  next: hibi providers
```

## 11. Look at a claim's history

```bash
hibi list c1-bvy8
```

```text output
c1-bvy8mfb476vwm9ftpx44
  statement: The client gives up after 5 attempts.
  subject: {"loc":"file:docs/retry.md","sel":{"quote":"gives up after 5 attempts","prefix":"tries\n\nThe client retries a failed download.\nIt ","suffix":".\nThe delay between attempts doubles each time, ","pos":52}}
  evidence 1: {"loc":"file:src/retry.rs","sel":{"quote":"pub const MAX_ATTEMPTS: u32 = 5;","prefix":"a download is tried before the client gives up.\n","suffix":"\n\n/// The delay before attempt `n`, in milliseco","pos":62,"symbol":"MAX_ATTEMPTS"}}
1 mark:
  m1-v1rs1r1kthxjmdw4ywdz  2026-11-20T21:23:47Z  by agent:claude
    file:docs/retry.md  b3:prose-v1:0233c89ebf2069292e2778bdc7176396
    file:src/retry.rs  b3:ast-v1:fb3a4a42c90a9093fe98969b8a0018c0
retired: r1-j1e3gttzxk7g5qswbygb  2026-11-20T21:24:12Z  replaced by c1-dryhtmnz5njjpp2ppy6t
```
