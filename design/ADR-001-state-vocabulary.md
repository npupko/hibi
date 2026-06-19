# ADR-001 — Ubiquitous language for the computed state model

Status: confirmed
Date: 2026-06-19
Deciders: project owner (decision delegated to research synthesis)
Supersedes: the original three-enum state model in PRD §10 (pre-revision)

## Context

The engine computes, per claim, on every `check`, a set of "states" that drive banners and exit
codes. The first-cut model had **three flat enums named by three unrelated metaphors**:

- code-side: `fresh / moved / stale / ghost / expired`
- doc-side: `doc-fresh / doc-moved / doc-edited / doc-ambiguous / doc-orphaned`
- behavioral: `behavior-unscoped / behavior-risk / behavior-failed / behavior-verified`

This is not ubiquitous language. Three tells:

1. **The same concept appeared twice under different words.** code `ghost` ≈ doc `doc-orphaned`
   ("the anchored target is gone"); code `stale` ≈ doc `doc-edited` ("the anchored content changed").
2. **Mixed metaphors.** A food/freshness metaphor (`fresh`/`stale`), a spooky one (`ghost`), and a
   literal one (`doc-edited`), for parallel ideas.
3. **A leaf state collided with an orthogonal axis.** `fresh` (freshness) overlapped conceptually with
   `expired` (TTL/time), which is a *different* question.

No backwards-compatibility constraint applies (greenfield), so a full revamp was on the table. The
decision was grounded in prior-art web research rather than taste (two parallel research sub-agents;
findings recorded in PRD §18-C).

## Decision

Collapse the three ad-hoc enums into **two axes that answer two different questions**, each given an
established, borrowed vocabulary, plus orthogonal flags.

### Axis 1 — Anchor resolution ("can I find the span, and is it unchanged?")
A *localization* fact. **One vocabulary, applied per side** (reported `doc:…` / `code:…`):

| State | Meaning | Borrowed from |
|---|---|---|
| `unchanged` | found, identical | git status (`unmodified`) |
| `moved` | found, relocated (same content) | git (`moved`/`renamed`) |
| `changed` | found, content differs | literal; antonym of `unchanged` |
| `ambiguous` | matches in >1 place | W3C Web Annotation (multiple-match) |
| `orphaned` | span deleted / unresolvable | hypothes.is ("orphaned" annotation) |

### Axis 2 — Behavioral belief ("do we still believe the documented behavior holds?")
An *epistemic / verification* fact. Absent on non-behavioral claims (display `n/a`, never stored):

| State | Meaning | Borrowed from |
|---|---|---|
| `unverified` | behavioral, untested, nothing changed (resting) | SMT/Frama-C/Nagios "unknown" family |
| `at-risk` | reachable evidence changed; belief no longer justified — re-verify | JTMS "support withdrawn" |
| `supported` | a linked verifier passed | FEVER `SUPPORTED` |
| `refuted` | a linked verifier failed (only state that may gate) | FEVER `REFUTED` |

### Orthogonal (not states)
`expired` (TTL flag); `enforcement` (`suggested`/`enforced`/`retired`); `authoredTrust`
(`verified`/`inferred`/`assumed`); `documentLifecycle`. The colloquial **drift** / **stale** is the
human roll-up for "any claim needing attention" — a banner-headline word, never a machine state.

A verdict reads: `doc:unchanged · code:changed · behavior:at-risk`.

## Why these specific words

- **`changed` over `stale` for the leaf state.** Makes `unchanged`/`changed` a clean antonym pair and
  frees "drift/stale" to be the human roll-up instead of overloading a leaf state. (`drifted` was
  rejected as a leaf state: it names the *whole phenomenon*, so using it for one outcome is a category
  error.)
- **`orphaned` promoted to both sides.** It was already the doc-side word and is hypothes.is's term of
  art for "an anchor that can no longer attach" — applying it to the code side too fixes the asymmetry.
- **`supported`/`refuted` not `verified`/`failed`.** Our unit is a *claim*; FEVER's claim-verification
  labels fit exactly, and `verified` is already taken by authored-trust (avoids a real collision).
- **`unverified` vs `at-risk` kept distinct.** "Never had proof" (resting) ≠ "basis just shifted"
  (change-gated alarm). Merging them into one `unknown` would lose hibi's most valuable signal.

## Consequences

- Doc and code sides now share one resolver/grader code path and one enum — less surface, perfectly
  parallel output. The code-side gains `ambiguous` (previously implicit).
- Two enums become two clearly-separated domain types (`AnchorState`, `BehaviorState`) instead of one
  conflated set; `expired` becomes a boolean flag.
- All consumers that matched the old strings must move to the new ones (no compat shim — greenfield).
- PRD §4, §6, §9, §10, §13, §14 (D5), §17.1, §17.3, §17.6, §18 updated to the new vocabulary.

## Alternatives considered

- **Keep docs-rot familiarity** (`fresh / moved / stale / ambiguous / missing`): higher recognition,
  but keeps the freshness metaphor and re-overloads "stale".
- **Drift-native** (`current / moved / drifted / ambiguous / lost`): evocative, but `drifted` as a leaf
  state is the category error noted above; `lost` is vaguer than `orphaned`.
- **Single-token behavioral** (`unsupported` instead of `at-risk`): avoids the hyphen but blurs
  "support withdrawn" with "never supported". Held as the fallback if serialization dislikes hyphens.

## Fitness function

- **Parallelism invariant (CI lint):** the doc-side and code-side state enums must be the *same* type;
  a test asserts `AnchorState` has exactly `{unchanged, moved, changed, ambiguous, orphaned}` and that
  no state name carries a `doc-`/`code-`/`behavior-` prefix (the side is a separate field, not baked
  into the word).
- **No-collision invariant:** a test asserts the behavioral enum shares no member with `authoredTrust`
  and that `drift`/`stale` appear in no machine enum (only in human-facing banner copy).
- **Gating invariant:** a test asserts only `refuted` (behavioral) and the `changed`/`orphaned`/
  `ambiguous`/`expired` anchor outcomes can produce exit code 2; `at-risk`/`moved` never do.
