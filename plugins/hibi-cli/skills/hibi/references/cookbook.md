# hibi cookbook

Worked tasks. Every output below was copied from a real run of hibi 5.0.0 on
a small example repository with `docs/retry.md` and `src/retry.rs`; the tasks
follow each other in that repository. Outputs are shown in the human format;
in a shell tool without a terminal the same facts come as one JSON object.
Blocks marked `output` are output, not commands.

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
c1-c5dm2jvhr8hb36rx5rgm  created  m1-pdfqery1ae16y8n6r17r  subject file:docs/retry.md (sentence "It gives up after 5 attempts.", line 4); evidence 1 file:src/retry.rs (variable "MAX_ATTEMPTS", line 2)
```

## 3. Start of a session: a statement is no longer true

```bash
hibi status
```

```text output
1 claim: 0 fresh, 1 stale, 0 expired, 0 retired

c1-c5dm2jvhr8hb36rx5rgm  stale  The client gives up after 5 attempts.
  marked 2026-11-21T08:59:34Z (m1-pdfqery1ae16y8n6r17r)
  evidence 1  file:src/retry.rs  changed (fingerprint_differs)  variable "MAX_ATTEMPTS", line 2
    | pub const MAX_ATTEMPTS: u32 = 3;
  next: hibi mark c1-c5dm2jvhr8hb36rx5rgm

Re-check each statement before running its next command. If it no longer holds, fix the text or the code, or retire the claim with hibi retire.
```

Re-check: the code now says `MAX_ATTEMPTS: u32 = 3`. The statement is false,
so do not mark. If you know the code is right (the user changed it on
purpose), fix the document to "It gives up after 3 attempts." and replace the
claim. If you do not know which is right, ask the user.

```bash
hibi reanchor c1-c5dm \
  --statement "The client gives up after 3 attempts." \
  --subject "file:docs/retry.md::gives up after 3 attempts"
```

```text output
c1-c5dm2jvhr8hb36rx5rgm is retired and replaced by the new claim c1-2kfx78rwbb7wyxwst92f
  mark m1-hqfvdhzhh3k5hf7hanm4, retirement r1-kzkbw96d81jyq607m4sy
  was stale: subject file:docs/retry.md missing (not_exact); evidence 1 file:src/retry.rs changed (fingerprint_differs); now marked: make sure you re-checked them
    evidence 1 is now:
    | pub const MAX_ATTEMPTS: u32 = 3;
  subject file:docs/retry.md (sentence "It gives up after 3 attempts.", line 4)
    {"loc":"file:docs/retry.md","sel":{"quote":"gives up after 3 attempts","prefix":"It","suffix":".","within":["Retries","The client retries a failed download."]}}
  evidence 1 file:src/retry.rs (variable "MAX_ATTEMPTS", line 2)
    {"loc":"file:src/retry.rs","sel":{"symbol":"MAX_ATTEMPTS"}}
```

```bash
hibi status
```

```text output
2 claims: 1 fresh, 0 stale, 0 expired, 1 retired
nothing stale
```

## 4. Claim an uncovered sentence from a batch

Anchors in a batch are compact strings, objects, or objects written as JSON
strings, as candidates print them. A `cover` item's `anchor`
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
c1-xwck2bdefbx39g56m7dn  created  m1-vbakkjbbz7xhfn7hz799  subject file:docs/retry.md (sentence "The delay between attempts doubles each time, starting at 2…", line 5); evidence 1 file:src/retry.rs (function "delay_ms", lines 5-7)
```

If any element fails, nothing is written and each failure carries its index:

```json output
{"ok":false,"command":"claim","errors":[{"code":"side_missing","message":"the subject (file:docs/nope.md) resolves to no unit (resource_not_found): file:docs/nope.md is not in git's file list of the root (tracked, or untracked and not ignored)","remediation":"Point the anchor at text that exists in the resource.","index":1}]}
```

## 5. A refactor that keeps behaviour

On branch `tidy-retry`, `200 * 2u64.pow(n)` became `200 << n`:

```bash
hibi status --since main
```

```text output
3 claims: 1 fresh, 1 stale, 0 expired, 1 retired
since main (base 32b3fd00695881dadd52fbd5d012948dfb58754f)

c1-xwck2bdefbx39g56m7dn  stale  The delay doubles each time, starting at 200 ms.
  marked 2026-11-21T08:59:35Z (m1-vbakkjbbz7xhfn7hz799)
  evidence 1  file:src/retry.rs  changed (fingerprint_differs)  function "delay_ms", lines 5-7
    | pub fn delay_ms(n: u32) -> u64 {
    |     200 << n
    | }
  next: hibi mark c1-xwck2bdefbx39g56m7dn

Re-check each statement before running its next command. If it no longer holds, fix the text or the code, or retire the claim with hibi retire.
```

Re-check: `200 << n` equals `200 * 2^n`. The statement holds:

```bash
hibi mark c1-xwck --note "200 << n equals 200 * 2^n" --by agent:claude
```

```text output
c1-xwck2bdefbx39g56m7dn  marked  m1-t1r68ygfdz8m7zfjymxm
  was stale: evidence 1 file:src/retry.rs changed (fingerprint_differs); now marked: make sure you re-checked it
    evidence 1 is now:
    | pub fn delay_ms(n: u32) -> u64 {
    |     200 << n
    | }
```

## 6. New text on a branch with no claim

The branch also added a "Timeouts" section:

```bash
hibi cover --since main
```

```text output
since main (base 32b3fd00695881dadd52fbd5d012948dfb58754f)
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
  --note "The timeout is set by the HTTP client defaults; check its release notes." \
  --now 2026-08-01T09:00:00Z
```

`--now` dated this example's mark in the past. When the period runs out the
claim is listed as expired:

```text output
c1-xqhbms40kkesq10rxjen  expired  Each attempt times out after 10 seconds.
  marked 2026-08-01T09:00:00Z (m1-z265p8tcpb68zsvzet3x), recheck P90D ran out at 2026-10-30T09:00:00Z
  next: hibi mark c1-xqhbms40kkesq10rxjen
```

## 7. The same claim again, with another anchor

The claim on "It gives up after 3 attempts." was made with the quote
`gives up after 3 attempts`. Claiming it again with the whole sentence marks
the existing claim instead of creating a second one, because every side
resolves to the same unit:

```bash
hibi claim "The client gives up after 3 attempts." \
  --subject "file:docs/retry.md::It gives up after 3 attempts." \
  --evidence "file:src/retry.rs#MAX_ATTEMPTS"
```

```text output
c1-2kfx78rwbb7wyxwst92f  marked  m1-6hscmehqkbswsx2a3wbw  subject file:docs/retry.md (sentence "It gives up after 3 attempts.", line 4); evidence 1 file:src/retry.rs (variable "MAX_ATTEMPTS", line 2)
```

This works only while the existing claim's sides resolve. After its
heading is renamed, for example, the subject is `missing`, and the same
command writes a second claim; re-anchor the existing claim instead.

## 8. A documented sentence was edited

"doubles each time" became "doubles every time". The claim quoted the whole
sentence, so its quoted words changed: the subject is `missing`
(`not_exact`), and the edited sentence is listed as a candidate. An edit
elsewhere in a long sentence, away from a shorter quote, gives `changed`
instead, and then `mark` is the step, as for code:

```bash
hibi status
```

```text output
4 claims: 1 fresh, 1 stale, 1 expired, 1 retired

c1-xqhbms40kkesq10rxjen  expired  Each attempt times out after 10 seconds.
  marked 2026-08-01T09:00:00Z (m1-z265p8tcpb68zsvzet3x), recheck P90D ran out at 2026-10-30T09:00:00Z
  next: hibi mark c1-xqhbms40kkesq10rxjen

c1-xwck2bdefbx39g56m7dn  stale  The delay doubles each time, starting at 200 ms.
  marked 2026-11-21T08:59:35Z (m1-t1r68ygfdz8m7zfjymxm)
  subject  file:docs/retry.md  missing (not_exact)
    the stored anchor no longer identifies one unit
    stored: quote "The delay between attempts doubles each time, starting at 200 ms."
    1 candidate; each is a guess, read it before you anchor to it:
    - sentence "The delay between attempts doubles every time, starting at …", line 5
      | The delay between attempts doubles every time, starting at 200 ms.
      anchor: {"loc":"file:docs/retry.md","sel":{"quote":"The delay between attempts doubles every time, starting at 200 ms.","prefix":"","suffix":"","within":["Retries","The client retries a failed download."]}}
  next: hibi reanchor c1-xwck2bdefbx39g56m7dn --subject <anchor>

Re-check each statement before running its next command. If it no longer holds, fix the text or the code, or retire the claim with hibi retire.
```

A candidate is a guess, so `mark` refuses the claim:

```bash
hibi mark c1-xwck --by agent:claude
```

```text output
error: side_missing: the subject (file:docs/retry.md) resolves to no unit (not_exact): the stored anchor no longer identifies one unit; 1 candidate, each a guess to check before use: sentence "The delay between attempts doubles every time, starting at 200 ms.", line 5: {"loc":"file:docs/retry.md","sel":{"quote":"The delay between attempts doubles every time, starting at 200 ms.","prefix":"","suffix":"","within":["Retries","The client retries a failed download."]}}
  fix: hibi reanchor c1-xwck2bdefbx39g56m7dn --subject <anchor>
```

Read the candidate. It is the same sentence, and the statement still holds,
so anchor the subject to it. Had the edit made the statement false, you
would fix the text or retire the claim instead. The candidate's anchor works, and so does a quote
of your own:

```bash
hibi reanchor c1-xwck --subject 'file:docs/retry.md::doubles every time'
```

```text output
c1-xwck2bdefbx39g56m7dn is retired and replaced by the new claim c1-2axv59jx0ztvfjd1adn9
  mark m1-ftgmt0z1dxzkah7p07z4, retirement r1-9vaxgfrhvjw5fbgnz6se
  was stale: subject file:docs/retry.md missing (not_exact); now marked: make sure you re-checked it
  subject file:docs/retry.md (sentence "The delay between attempts doubles every time, starting at …", line 5)
    {"loc":"file:docs/retry.md","sel":{"quote":"doubles every time","prefix":"The delay between attempts","suffix":", starting at 200 ms.","within":["Retries","The client retries a failed download."]}}
  evidence 1 file:src/retry.rs (function "delay_ms", lines 5-7)
    {"loc":"file:src/retry.rs","sel":{"symbol":"delay_ms"}}
```

The old claim is retired into the new one, and `status` no longer lists it.

## 9. A sentence was deleted

```bash
hibi status
```

```text output
5 claims: 1 fresh, 1 stale, 1 expired, 2 retired

c1-2kfx78rwbb7wyxwst92f  stale  The client gives up after 3 attempts.
  marked 2026-11-21T08:59:35Z (m1-6hscmehqkbswsx2a3wbw)
  subject  file:docs/retry.md  missing (selector_not_found)
    stored: quote "gives up after 3 attempts"
  next: hibi reanchor c1-2kfx78rwbb7wyxwst92f --subject <anchor>

c1-xqhbms40kkesq10rxjen  expired  Each attempt times out after 10 seconds.
  marked 2026-08-01T09:00:00Z (m1-z265p8tcpb68zsvzet3x), recheck P90D ran out at 2026-10-30T09:00:00Z
  next: hibi mark c1-xqhbms40kkesq10rxjen

Re-check each statement before running its next command. If it no longer holds, fix the text or the code, or retire the claim with hibi retire.
```

If the sentence moved, give its new anchor with `--subject`. Here it was
removed on purpose, so retire the claim:

```bash
hibi retire c1-2kfx --note "The docs no longer state the attempt limit."
```

```text output
c1-2kfx78rwbb7wyxwst92f  retired  r1-yxm06xchd4ax4ggtgh2n
```

## 10. Anchor to a struct field, not its doc comment

In another repository, a quote inside a doc comment is refused, because
comments are not part of the fingerprint:

```text output
$ hibi claim "The request timeout is stored in whole seconds." --subject "file:README.md::stored in whole seconds" --evidence "file:src/config.rs#Config::Seconds before a request"
error: quote_in_comment: evidence 1 (file:src/config.rs): the quote lies wholly or partly inside a code comment
  fix: Anchor the code the comment describes.
```

Anchor the field itself by its symbol:

```text output
$ hibi claim "The request timeout is stored in whole seconds." --subject "file:README.md::stored in whole seconds" --evidence "file:src/config.rs#Config.timeout_secs"
c1-796bxye5q996vzq2w0y9  created  m1-1tvh90een18kkqkwy2ew  subject file:README.md (sentence "The request timeout is stored in whole seconds.", line 4); evidence 1 file:src/config.rs (field "Config.timeout_secs", line 5)
```

A Python docstring is a string and can be quoted:
`file:src/jobs.py#Job::Jobs run at most 3 times`.

## 11. A unit served by an external provider

The repository declares the scheme in `.hibi/hibi.toml` (`[schemes.ref]`) and
the user registers its command in their own user config. A selector other
than the empty one is given as JSON:

```bash
echo '[{"statement": "Downloads are retried at most 3 times.", "subject": {"loc": "ref:notes", "sel": {"key": "n1"}}, "evidence": ["file:src/retry.rs#MAX_ATTEMPTS"]}]' | hibi claim --stdin
```

```text output
c1-kh88fbb9hctgd1v08923  created  m1-hcx8k9x68n3bah0p62q2  subject ref:notes (note "Downloads are retried at most 3 times."); evidence 1 file:src/retry.rs (variable "MAX_ATTEMPTS", line 2)
```

Without the user's registration the side is unavailable and nothing runs.
Tell the user; do not edit their config unasked:

```text output
c1-kh88fbb9hctgd1v08923  stale  Downloads are retried at most 3 times.
  marked 2026-11-21T08:49:17Z (m1-hcx8k9x68n3bah0p62q2)
  subject  ref:notes  unavailable (provider_not_registered)
    the user config has no provider command for scheme ref
  next: hibi providers
```

## 12. Look at a claim's history

```bash
hibi list c1-c5dm
```

```text output
c1-c5dm2jvhr8hb36rx5rgm
  statement: The client gives up after 5 attempts.
  subject: {"loc":"file:docs/retry.md","sel":{"quote":"gives up after 5 attempts","prefix":"It","suffix":".","within":["Retries","The client retries a failed download."]}}
  evidence 1: {"loc":"file:src/retry.rs","sel":{"symbol":"MAX_ATTEMPTS"}}
1 mark:
  m1-pdfqery1ae16y8n6r17r  2026-11-21T08:59:34Z
    file:docs/retry.md  b3:prose-v1:0233c89ebf2069292e2778bdc7176396
    file:src/retry.rs  b3:ast-v1:fb3a4a42c90a9093fe98969b8a0018c0
retired: r1-kzkbw96d81jyq607m4sy  2026-11-21T08:59:34Z  replaced by c1-2kfx78rwbb7wyxwst92f
```
