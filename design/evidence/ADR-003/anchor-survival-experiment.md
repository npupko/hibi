# Anchor-survival experiment — evidence for ADR-003 D22/D23

Date: 2026-07-08. Grounds: D22 (structural-path selector retracted), D23 (record-time quote
guard replaces context expansion).

Method: replay 6 heavy doc-rewrite commit pairs from this repo's own history (33 changed file
pairs; N = 2,501 old-version sentences). For each sentence, simulate hibi's doc anchor exactly
(quote + 48-char prefix/suffix + position) and re-attach in the new version using hibi's actual
`localizeTextQuote` (Bitap, `src/algo/localize.ts`), cross-checked with a bigram-Dice
approximation (found-threshold 0.75). For each sentence also compute the markdown heading trail
(h1>h2>h3 + block index) in the old version and test whether a heading-path selector would have
rescued, disambiguated, or misled. Reproduce with `bun run
design/evidence/ADR-003/anchor-survival-experiment.ts` (read-only `git show`; writes
`results.json` next to itself).

Commit pairs: `befcaf5` (AI-writing reword sweep, 14 mdx), `a488da7` (ADR-002 structural
realignment, 11 mdx + PRD), `ec8d0a7`, `86af1ae` (two heavy PRD rewrites), `cb9b092` (README
full rewrite), `b09c7a9` (SKILL.md scenario-led restructure).

## Aggregate (N = 2,501)

| metric | value |
|---|---|
| exact-unique | **81.3%** (2,034) |
| ambiguous-without-context | 1.4% (34) |
| resolved by 48-char prefix/suffix | 34/34 = **100%** |
| ambiguous-after-context | **0.0%** |
| fuzzy-rescued (sim ≥ 0.75) | 11.0% (276) |
| not-found (orphan) | **6.3%** (157) |
| heading-trail survived (non-root sentences) | 92.1% |
| heading survival (exact text, 354 old headings) | 92.4%; full trails 90.7% |
| sentences that moved section | 119; old trail still exists for 22 → **18.5% of movers = mislead** |
| heading-path rescued | 34/157 orphans = **21.7%** = 1.4% of all sentences |
| heading-path disambiguated | **0 cases even possible** (context already resolved all 34) |
| heading-path would mislead | 22 = 0.9% of all sentences |
| block-index false-attach risk | 35/157 orphans = **22.3%** (same-index block similarity < 0.3) |

## Per-pair

| pair | N | uniq | ambig | fuzzy | not-found | trail✓ | rescue/NF | mislead | falseAttach/NF |
|---|---|---|---|---|---|---|---|---|---|
| AI-writing purge (befcaf5) | 984 | 78.4% | 0.8% | 18.8% | 2.0% | 93.5% | 30.0% | 0.3% | 45.0% |
| ADR-002 realignment (a488da7) | 1155 | 93.0% | 2.0% | 3.2% | 1.8% | 97.8% | 47.6% | 1.2% | 19.0% |
| PRD re-ground (ec8d0a7) | 80 | 75.0% | 3.8% | 16.3% | 5.0% | 97.5% | 50.0% | 0.0% | 25.0% |
| PRD final-state (86af1ae) | 80 | 85.0% | 0.0% | 10.0% | 5.0% | 90.0% | 25.0% | 2.5% | 50.0% |
| README rewrite (cb9b092) | 42 | 2.4% | 0.0% | 14.3% | **83.3%** | **0.0%** | 0.0% | 0.0% | 0.0% |
| SKILL.md restructure (b09c7a9) | 160 | 37.5% | 0.0% | 16.9% | **45.6%** | 70.0% | 20.5% | 1.9% | 26.0% |

Heading stability per pair: befcaf5 93% headings / 92% trails; a488da7 98%/98%; ec8d0a7
93%/93%; 86af1ae 91%/86%; **cb9b092 25% / 0%**; **b09c7a9 61%/61%**.

## Reading

1. The text cascade alone re-attaches 92.3% (81.3 exact + 1.4 context-resolved + 11.0 fuzzy);
   the 48-char context already resolves **all** exact-match ambiguity, and every ambiguous case
   was a degenerate short fragment (e.g. `` `refuted`. ``) → the useful record-time guard is a
   quote length/uniqueness check (D23), not wider stored context.
2. Heading-path adds ~zero disambiguation, marginal rescue (1.4% of sentences) roughly cancelled
   by misleads + false-attaches, and its trails are *least* stable in exactly the heavy
   restructures where quote matching needs help (README: 100% trail death) → D22 retraction.
3. A human-judged orphan sample split ~60/40 reworded-with-semantic-change vs truly deleted —
   a large share of would-be "rescues" are cases where flagging is the correct product behavior.
