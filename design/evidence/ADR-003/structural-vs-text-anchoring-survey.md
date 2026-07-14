# Structural vs text anchoring — literature & production survey (evidence for ADR-003 D22)

Date: 2026-07-08. Question: for anchoring sentences in plain-text/markdown documents that get
edited, does adding a structural selector (heading path / block path / DOM path) to a
text-quote+context+position bundle materially improve correct re-attachment, versus the text
selectors alone plus optional inline IDs?

**Answer: no.** No study anywhere quantifies a marginal re-attachment gain from adding a
structural selector to a quote+context bundle in text documents. Every quantitative and
production data point favors content-based matching as the workhorse, shows structural anchors
decaying under exactly the edits markdown undergoes, and shows production systems solving
durability with assigned IDs — not heading-path matching.

## Strongest evidence FOR structure

- **Phelps & Wilensky, "Robust Intra-document Locations" (WWW9, 2000)** — the origin of the
  ID → tree-walk → context cascade. 742/754 annotations automatically repositioned. Caveats: a
  small self-run test, no ablation isolating the tree walk's contribution over context alone —
  and the paper itself notes "introducing a new previous sibling, or removing one, can
  invalidate the walk" (a markdown block-index is exactly a sibling count).
  http://www.ra.ethz.ch/CDstore/www9/312/312.html
- **W3C Web Annotation** notes quote selection "can be ambiguous when the same selected text and
  context match more than once" — but widened context achieves the same disambiguation.
  https://www.w3.org/TR/annotation-model/#selectors
- **Hypothes.is keeps a RangeSelector (XPath)** — explicitly as the fast path for *unchanged*
  documents, verified against the quote, never as the robustness mechanism.
  https://web.hypothes.is/blog/fuzzy-anchoring/

## Strongest evidence AGAINST

- **Brush, Bargeron, Gupta & Cadiz (CHI 2001 / MSR-TR-2000-95)** — the only user-grounded
  quantitative study of anchor repositioning under real edits. Pure text matching found **100%**
  of annotations whose anchor text moved but did not change. User ratings correlated **+0.72**
  with modification amount for orphans: the more the text changed, the more users *preferred
  orphaning over rescue*. "Participants appeared to pay little attention to the text surrounding
  their annotations." https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/tr-2000-95.pdf
- **Brush & Bargeron (MSR-TR-2001-107)** — the follow-up deliberately rejects structure: "it
  ignores any specific internal document structure… Keyword Anchoring requires no cooperation
  from the document." It beat simple text search (median 6.5 vs 5.0–5.5, p=.02). On
  Annotea/XPointer: positions "can be orphaned or incorrectly positioned if the document
  changes." https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/tr-2001-107.pdf
- **Hypothes.is orphan study (arXiv:1512.06195)** — 20,953 annotations; ~22% detached; only
  ~12% of detached rescuable from web archives; attachment was determined by text search alone —
  structure contributed nothing to rescue. https://arxiv.org/abs/1512.06195
- **Reiss, "Tracking source locations" (ICSE 2008)** and follow-ups: across real edit histories
  "relatively simple techniques can be very effective"; the winning tracker is Levenshtein
  content + context lines — no structure.
- **Heading anchors rot in production**: Obsidian heading links break on rename (standing forum
  bugs); Wikipedia section links break on rename — permanent database report + repair bots
  (cewbot, Dexbot, FrescoBot), sanctioned fix is Template:Anchor, i.e. an inline stable ID;
  GitBook documents that heading edits break anchors.
  https://en.wikipedia.org/wiki/Wikipedia:Broken_section_links

## Production convergence

| System | Durable anchor | Heading-path matching? |
|---|---|---|
| Notion | per-block UUIDv4 | No |
| Obsidian | author-inserted `^block-id` | heading links exist and are the known-broken feature |
| Wikipedia | Template:Anchor (manual ID) + repair bots | section links = known rot source |
| GitBook | auto heading anchors | documented to break on edit |
| Swimm | content signals (tokens, line markers, histogram) + human reselect | No |
| Hypothes.is | quote+context fuzzy (Bitap) | XPath = unchanged-doc fast path only |

Pattern: **content matching for automatic re-attachment, assigned IDs for durability, explicit
orphaning below confidence** — which is hibi's shipped doc-side design.

## Verdict (as adopted in D22/D23)

Text-quote + context + position + optional inline ID dominates. The only defensible residual
role for a heading trail is display metadata or a candidate-ranking hint — never a selector able
to relocate a claim that quote+context could not. Both roles were declined in D22 (schema/scope
cost for a display nicety). The one real problem structure would solve — repeated-text
ambiguity — is deterministically prevented at record time instead (D23), matching W3C's own
"store enough context" guidance and the source report's fallback recommendation (inline IDs, not
more structure).
