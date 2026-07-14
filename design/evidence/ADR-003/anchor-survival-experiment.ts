/**
 * Empirical experiment: how often do hibi-style doc-side text anchors
 * (text-quote exact + 48-char prefix/suffix + text-position) fail or ambiguate
 * across real doc-rewrite commits in the hibi repo, and would a markdown
 * heading-path selector (heading trail + block index) have rescued /
 * disambiguated / misled?
 *
 * Read-only against the repo; imports hibi's own Bitap localizer for a
 * faithful cross-check of the real cascade.
 *
 * Run from the repo root: bun design/evidence/ADR-003/anchor-survival-experiment.ts
 * (writes results.json next to this script; see anchor-survival-experiment.md for the report)
 */

import { localizeTextQuote } from "../../../src/algo/localize.ts";

const REPO = new URL("../../..", import.meta.url).pathname.replace(/\/$/, "");
const CONTEXT = 48; // hibi TEXT_QUOTE_CONTEXT
const FUZZY_THRESHOLD = 0.75; // task spec: "found" bar for fuzzy reattach
const RESCUE_SIM = 0.5; // same-sentence-reworded bar for heading-path rescue
const FALSE_ATTACH_SIM = 0.3; // below this, heading+block-index attaches to unrelated text
const MAX_SENTENCES_PER_FILE = 80;

// ---------- commit pairs -------------------------------------------------

const PAIRS: { sha: string; label: string }[] = [
  { sha: "befcaf5", label: "AI-writing purge (befcaf5) — reword sweep, 14 mdx" },
  { sha: "a488da7", label: "ADR-002 realignment (a488da7) — structural, 11 mdx + PRD" },
  { sha: "ec8d0a7", label: "PRD re-ground (ec8d0a7) — docs/PRD.md heavy rewrite" },
  { sha: "86af1ae", label: "PRD final-state restate (86af1ae) — docs/PRD.md" },
  { sha: "cb9b092", label: "README popular-library rewrite (cb9b092)" },
  { sha: "b09c7a9", label: "SKILL.md scenario-led restructure (b09c7a9)" },
];

function git(args: string[]): string {
  const p = Bun.spawnSync(["git", ...args], { cwd: REPO });
  if (p.exitCode !== 0) return "";
  return p.stdout.toString();
}

function changedDocFiles(sha: string): string[] {
  const out = git(["diff-tree", "--no-commit-id", "--name-only", "-r", sha]);
  return out
    .split("\n")
    .filter((f) => /\.(md|mdx)$/.test(f))
    .filter((f) => !/researches\.local/.test(f));
}

function fileAt(sha: string, path: string): string | null {
  const p = Bun.spawnSync(["git", "show", `${sha}:${path}`], { cwd: REPO });
  if (p.exitCode !== 0) return null;
  return p.stdout.toString();
}

// ---------- markdown parsing ---------------------------------------------

interface Heading { level: number; text: string; lineStart: number; contentStart: number }
interface Sentence { text: string; start: number; line: number }
interface Block { start: number; end: number; text: string }
interface Section { trail: string; start: number; end: number } // content span

interface ParsedDoc {
  text: string;
  headings: Heading[];
  sentences: Sentence[]; // candidate claim sentences with absolute offsets
  sections: Section[]; // includes root section trail ""
}

function cleanHeadingText(s: string): string {
  return s.replace(/#+\s*$/, "").replace(/[*_`]/g, "").trim();
}

function parseDoc(text: string): ParsedDoc {
  const lines = text.split("\n");
  const headings: Heading[] = [];
  const sentences: Sentence[] = [];
  let offset = 0;
  let inFence = false;
  let inFrontmatter = false;
  let frontmatterDone = false;

  const lineOffsets: number[] = [];
  for (const line of lines) {
    lineOffsets.push(offset);
    offset += line.length + 1;
  }

  for (let li = 0; li < lines.length; li++) {
    const line = lines[li]!;
    const lo = lineOffsets[li]!;

    if (li === 0 && /^---\s*$/.test(line)) { inFrontmatter = true; continue; }
    if (inFrontmatter) {
      if (/^---\s*$/.test(line)) { inFrontmatter = false; frontmatterDone = true; }
      continue;
    }
    if (/^\s*(```|~~~)/.test(line)) { inFence = !inFence; continue; }
    if (inFence) continue;

    const hm = line.match(/^(#{1,6})\s+(.*)$/);
    if (hm) {
      headings.push({
        level: hm[1]!.length,
        text: cleanHeadingText(hm[2]!),
        lineStart: lo,
        contentStart: lo + line.length + 1,
      });
      continue;
    }

    if (!line.trim()) continue;
    if (/^\s*\|/.test(line)) continue; // table row
    if (/^\s*<\/?[A-Za-z!]/.test(line)) continue; // JSX / HTML tag line
    if (/^\s*(import|export)\s/.test(line)) continue;
    if (/^\s*!\[/.test(line)) continue; // image
    if (/^\s*\[[^\]]*\]:/.test(line)) continue; // link def
    if (/^\s*[-*_]{3,}\s*$/.test(line)) continue; // hr
    if (/^\s*\{/.test(line)) continue; // MDX expression

    // strip leading blockquote / bullet / ordered-list marker (keep offsets exact)
    const mm = line.match(/^(\s*(?:>\s*)*(?:[-*+]\s+|\d+[.)]\s+)?)/);
    const markerLen = mm ? mm[1]!.length : 0;
    const content = line.slice(markerLen);
    if (content.trim().length < 8) continue;

    // split into sentence spans on ". " / "! " / "? " boundaries
    const spans: { s: number; e: number }[] = [];
    let last = 0;
    const boundary = /[.!?](?=\s)/g;
    let bm: RegExpExecArray | null;
    while ((bm = boundary.exec(content)) !== null) {
      spans.push({ s: last, e: bm.index + 1 });
      last = bm.index + 1;
      while (last < content.length && /\s/.test(content[last]!)) last++;
    }
    if (last < content.length) spans.push({ s: last, e: content.length });

    for (const sp of spans) {
      const raw = content.slice(sp.s, sp.e).trim();
      if (raw.length < 8) continue;
      if (!/[a-zA-Z]{3}/.test(raw)) continue; // must contain a word
      if (/^https?:\/\/\S+$/.test(raw)) continue;
      // absolute start: find raw within the span (trim shifted it)
      const rel = content.indexOf(raw, sp.s);
      if (rel === -1) continue;
      sentences.push({ text: raw, start: lo + markerLen + rel, line: li });
    }
  }

  // build sections (content spans per heading trail; plus root)
  const sections: Section[] = [];
  const rootEnd = headings.length > 0 ? headings[0]!.lineStart : text.length;
  sections.push({ trail: "", start: frontmatterDone ? 0 : 0, end: rootEnd });
  for (let i = 0; i < headings.length; i++) {
    const h = headings[i]!;
    // trail = nearest ancestors by level
    const trailParts: string[] = [];
    const stack: Heading[] = [];
    for (let j = 0; j <= i; j++) {
      const hj = headings[j]!;
      while (stack.length && stack[stack.length - 1]!.level >= hj.level) stack.pop();
      stack.push(hj);
    }
    for (const s of stack) trailParts.push(s.text);
    // end = next heading with level <= h.level, else EOF
    let end = text.length;
    for (let j = i + 1; j < headings.length; j++) {
      if (headings[j]!.level <= h.level) { end = headings[j]!.lineStart; break; }
    }
    sections.push({ trail: trailParts.join(" > "), start: h.contentStart, end });
  }

  return { text, headings, sentences, sections };
}

/** innermost section containing offset */
function sectionAt(doc: ParsedDoc, off: number): Section {
  let best: Section = doc.sections[0]!;
  let bestSize = Infinity;
  for (const s of doc.sections) {
    if (off >= s.start && off < s.end && s.end - s.start < bestSize) {
      best = s;
      bestSize = s.end - s.start;
    }
  }
  return best;
}

/** blank-line-separated blocks within [start,end) of text */
function blocksIn(text: string, start: number, end: number): Block[] {
  const slice = text.slice(start, end);
  const blocks: Block[] = [];
  const re = /(?:^|\n)((?:[^\n]*\S[^\n]*\n?)+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(slice)) !== null) {
    const bs = start + m.index + (m[0]!.startsWith("\n") ? 1 : 0);
    const btext = m[1]!;
    blocks.push({ start: bs, end: bs + btext.length, text: btext });
  }
  return blocks;
}

// ---------- similarity ----------------------------------------------------

function normalize(s: string): string {
  return s.toLowerCase().replace(/[*_`]/g, "").replace(/\s+/g, " ").trim();
}

function bigrams(s: string): Map<string, number> {
  const m = new Map<string, number>();
  for (let i = 0; i < s.length - 1; i++) {
    const b = s.slice(i, i + 2);
    m.set(b, (m.get(b) ?? 0) + 1);
  }
  return m;
}

function diceFromMaps(a: Map<string, number>, an: number, b: Map<string, number>, bn: number): number {
  if (an === 0 || bn === 0) return 0;
  let inter = 0;
  const [small, large] = a.size <= b.size ? [a, b] : [b, a];
  for (const [k, v] of small) {
    const w = large.get(k);
    if (w) inter += Math.min(v, w);
  }
  return (2 * inter) / (an + bn);
}

interface SimEntry { grams: Map<string, number>; n: number }
function simEntry(s: string): SimEntry {
  const n = normalize(s);
  const g = bigrams(n);
  let total = 0;
  for (const v of g.values()) total += v;
  return { grams: g, n: total };
}
function dice(a: SimEntry, b: SimEntry): number {
  return diceFromMaps(a.grams, a.n, b.grams, b.n);
}

// ---------- exact-occurrence helpers ---------------------------------------

function occurrences(hay: string, needle: string): number[] {
  const out: number[] = [];
  let i = 0;
  while ((i = hay.indexOf(needle, i)) !== -1) {
    out.push(i);
    i += 1; // allow overlaps; harmless here
    if (out.length > 50) break;
  }
  return out;
}

// ---------- per-sentence evaluation ----------------------------------------

type Outcome = "UNIQUE" | "AMBIGUOUS_CTX_RESOLVED" | "AMBIGUOUS" | "FUZZY_FOUND" | "NOT_FOUND";

interface Result {
  pair: string;
  file: string;
  sentence: string;
  outcome: Outcome;
  exactCount: number;
  hibiLocalized: boolean; // hibi's real Bitap cascade found a region
  hibiSim: number; // similarity of hibi's region to the quote
  bestGlobalSim: number;
  bestGlobalFragment: string;
  bestGlobalOff: number;
  oldTrail: string;
  trailSurvived: boolean;
  trailAmbiguousInNew: boolean;
  movedSection: boolean | null; // null = unknown (no true location)
  headingRescue: "block-index" | "relaxed-fuzzy" | null;
  headingDisambiguated: boolean;
  headingMislead: boolean; // true loc outside surviving old-trail section
  falseAttachRisk: boolean; // NOT_FOUND + trail survived + same-index block is unrelated
  blockIndexSim: number | null;
}

function evaluatePair(sha: string, label: string, results: Result[], headingStats: any[]) {
  const files = changedDocFiles(sha);
  for (const path of files) {
    const oldText = fileAt(`${sha}^`, path);
    const newText = fileAt(sha, path);
    if (oldText == null || newText == null) continue; // added or deleted file
    if (oldText === newText) continue;

    const oldDoc = parseDoc(oldText);
    const newDoc = parseDoc(newText);

    // heading survival for this file
    const newHeadingTexts = new Set(newDoc.headings.map((h) => h.text));
    const newTrails = new Map<string, Section[]>();
    for (const s of newDoc.sections) {
      if (!newTrails.has(s.trail)) newTrails.set(s.trail, []);
      newTrails.get(s.trail)!.push(s);
    }
    const oldTrailsSet = new Set(oldDoc.sections.slice(1).map((s) => s.trail));
    let headingsSurvived = 0;
    for (const h of oldDoc.headings) if (newHeadingTexts.has(h.text)) headingsSurvived++;
    let trailsSurvived = 0;
    for (const t of oldTrailsSet) if (newTrails.has(t)) trailsSurvived++;
    headingStats.push({
      pair: label, file: path,
      oldHeadings: oldDoc.headings.length,
      headingsSurvived,
      oldTrails: oldTrailsSet.size,
      trailsSurvived,
    });

    // sample sentences
    let sents = oldDoc.sentences;
    if (sents.length > MAX_SENTENCES_PER_FILE) {
      const step = sents.length / MAX_SENTENCES_PER_FILE;
      const sampled: Sentence[] = [];
      for (let i = 0; i < MAX_SENTENCES_PER_FILE; i++) sampled.push(sents[Math.floor(i * step)]!);
      sents = sampled;
    }

    // precompute new-sentence sim entries (global)
    const newSimEntries = newDoc.sentences.map((s) => ({ s, e: simEntry(s.text) }));

    for (const sent of sents) {
      const quote = sent.text;
      const start = sent.start;
      const end = start + quote.length;
      const prefix = oldText.slice(Math.max(0, start - CONTEXT), start);
      const suffix = oldText.slice(end, end + CONTEXT);
      const quoteEntry = simEntry(quote);

      // (i) exact matches in new
      const occ = occurrences(newText, quote);
      let outcome: Outcome;
      let trueLoc: number | null = null;

      let ctxResolved = false;
      if (occ.length === 1) {
        outcome = "UNIQUE";
        trueLoc = occ[0]!;
      } else if (occ.length > 1) {
        // context disambiguation with stored 48-char prefix/suffix
        const preE = simEntry(prefix);
        const sufE = simEntry(suffix);
        const scored = occ.map((o) => {
          const p = simEntry(newText.slice(Math.max(0, o - CONTEXT), o));
          const su = simEntry(newText.slice(o + quote.length, o + quote.length + CONTEXT));
          return { o, score: dice(preE, p) + dice(sufE, su) };
        });
        scored.sort((a, b) => b.score - a.score);
        if (scored.length > 1 && scored[0]!.score - scored[1]!.score > 0.05) {
          ctxResolved = true;
          outcome = "AMBIGUOUS_CTX_RESOLVED";
          trueLoc = scored[0]!.o;
        } else {
          outcome = "AMBIGUOUS";
        }
      } else {
        // (ii) fuzzy: best-similar sentence anywhere in new (sliding approximation)
        outcome = "NOT_FOUND"; // provisional
      }

      // hibi's real Bitap cascade (cross-check, and used for fuzzy)
      let hibiLocalized = false;
      let hibiSim = 0;
      const region = localizeTextQuote(newText, { type: "text-quote", exact: quote, prefix, suffix } as any, start);
      if (region) {
        hibiLocalized = true;
        hibiSim = dice(quoteEntry, simEntry(newText.slice(region.start, region.end)));
      }

      // best global fragment
      let bestGlobalSim = 0;
      let bestGlobalFragment = "";
      let bestGlobalOff = -1;
      for (const ns of newSimEntries) {
        const d = dice(quoteEntry, ns.e);
        if (d > bestGlobalSim) {
          bestGlobalSim = d;
          bestGlobalFragment = ns.s.text;
          bestGlobalOff = ns.s.start;
        }
      }

      if (occ.length === 0) {
        if (bestGlobalSim >= FUZZY_THRESHOLD) {
          outcome = "FUZZY_FOUND";
          trueLoc = bestGlobalOff;
        } else if (hibiLocalized && hibiSim >= FUZZY_THRESHOLD) {
          outcome = "FUZZY_FOUND";
          trueLoc = region!.start;
        } else {
          outcome = "NOT_FOUND";
        }
      }

      // ---------- heading-path evaluation ----------
      const oldSection = sectionAt(oldDoc, start);
      const oldTrail = oldSection.trail;
      const candidates = newTrails.get(oldTrail) ?? [];
      const trailSurvived = candidates.length > 0;
      const trailAmbiguousInNew = candidates.length > 1;
      const newSection = trailSurvived ? candidates[0]! : null;

      // block index of sentence within old section
      const oldBlocks = blocksIn(oldText, oldSection.start, oldSection.end);
      let blockIdx = -1;
      for (let i = 0; i < oldBlocks.length; i++) {
        if (start >= oldBlocks[i]!.start && start < oldBlocks[i]!.end) { blockIdx = i; break; }
      }

      let movedSection: boolean | null = null;
      let headingMislead = false;
      if (trueLoc !== null) {
        const newSec = sectionAt(newDoc, trueLoc);
        movedSection = newSec.trail !== oldTrail;
        // mislead: sentence found elsewhere, but old trail still exists in new →
        // a heading-path-scoped search points at the wrong section
        if (movedSection && trailSurvived) headingMislead = true;
      }

      // heading rescue / disambiguation / false-attach
      let headingRescue: Result["headingRescue"] = null;
      let headingDisambiguated = false;
      let falseAttachRisk = false;
      let blockIndexSim: number | null = null;

      if (trailSurvived && newSection) {
        // sentences of new confined to section
        const sectionSents = newSimEntries.filter(
          (ns) => ns.s.start >= newSection.start && ns.s.start < newSection.end,
        );

        if (outcome === "AMBIGUOUS" || outcome === "AMBIGUOUS_CTX_RESOLVED") {
          const inSection = occ.filter((o) => o >= newSection.start && o < newSection.end);
          if (outcome === "AMBIGUOUS" && inSection.length === 1) headingDisambiguated = true;
        }

        if (outcome === "NOT_FOUND") {
          // (a) block-index attach: same-index block in the new section
          const newBlocks = blocksIn(newText, newSection.start, newSection.end);
          if (blockIdx >= 0 && blockIdx < newBlocks.length) {
            const nb = newBlocks[blockIdx]!;
            // best sentence inside that block
            let best = 0;
            for (const ns of sectionSents) {
              if (ns.s.start >= nb.start && ns.s.start < nb.end) {
                const d = dice(quoteEntry, ns.e);
                if (d > best) best = d;
              }
            }
            if (best === 0) best = dice(quoteEntry, simEntry(nb.text));
            blockIndexSim = best;
            if (best >= RESCUE_SIM) headingRescue = "block-index";
            else if (best < FALSE_ATTACH_SIM) falseAttachRisk = true;
          } else if (blockIdx >= 0) {
            falseAttachRisk = true; // block index out of range in new section
          }
          // (b) relaxed fuzzy within section: scope makes a lower bar safe
          if (!headingRescue) {
            let bestSec = 0;
            for (const ns of sectionSents) {
              const d = dice(quoteEntry, ns.e);
              if (d > bestSec) bestSec = d;
            }
            if (bestSec >= RESCUE_SIM) headingRescue = "relaxed-fuzzy";
          }
        }
      }

      results.push({
        pair: label, file: path, sentence: quote, outcome,
        exactCount: occ.length, hibiLocalized, hibiSim,
        bestGlobalSim, bestGlobalFragment, bestGlobalOff,
        oldTrail, trailSurvived, trailAmbiguousInNew,
        movedSection, headingRescue, headingDisambiguated, headingMislead,
        falseAttachRisk, blockIndexSim,
      });
    }
  }
}

// ---------- run -------------------------------------------------------------

const results: Result[] = [];
const headingStats: any[] = [];
for (const p of PAIRS) evaluatePair(p.sha, p.label, results, headingStats);

// ---------- aggregate --------------------------------------------------------

function pct(n: number, d: number): string {
  return d === 0 ? "-" : ((100 * n) / d).toFixed(1) + "%";
}

function agg(rows: Result[]) {
  const N = rows.length;
  const c = (f: (r: Result) => boolean) => rows.filter(f).length;
  const unique = c((r) => r.outcome === "UNIQUE");
  const ambigBefore = c((r) => r.exactCount > 1);
  const ambigResolved = c((r) => r.outcome === "AMBIGUOUS_CTX_RESOLVED");
  const ambigAfter = c((r) => r.outcome === "AMBIGUOUS");
  const fuzzy = c((r) => r.outcome === "FUZZY_FOUND");
  const notFound = c((r) => r.outcome === "NOT_FOUND");
  const hibiFound = c((r) => r.hibiLocalized);
  const trailSurv = c((r) => r.trailSurvived);
  const trailSurvNonRoot = c((r) => r.oldTrail !== "" && r.trailSurvived);
  const nonRoot = c((r) => r.oldTrail !== "");
  const rescued = c((r) => r.headingRescue !== null);
  const rescuedBlock = c((r) => r.headingRescue === "block-index");
  const disamb = c((r) => r.headingDisambiguated);
  const mislead = c((r) => r.headingMislead);
  const falseAttach = c((r) => r.falseAttachRisk);
  const moved = c((r) => r.movedSection === true);
  return { N, unique, ambigBefore, ambigResolved, ambigAfter, fuzzy, notFound, hibiFound, trailSurv, trailSurvNonRoot, nonRoot, rescued, rescuedBlock, disamb, mislead, falseAttach, moved };
}

function fmtRow(name: string, a: ReturnType<typeof agg>): string {
  return [
    name.padEnd(46),
    String(a.N).padStart(5),
    pct(a.unique, a.N).padStart(8),
    pct(a.ambigBefore, a.N).padStart(8),
    pct(a.ambigAfter, a.N).padStart(8),
    pct(a.fuzzy, a.N).padStart(8),
    pct(a.notFound, a.N).padStart(8),
    pct(a.hibiFound, a.N).padStart(8),
    pct(a.trailSurvNonRoot, a.nonRoot).padStart(8),
    pct(a.rescued, a.notFound).padStart(9),
    pct(a.disamb, a.ambigAfter).padStart(9),
    pct(a.mislead, a.N).padStart(8),
    pct(a.falseAttach, a.notFound).padStart(9),
  ].join(" ");
}

const header = [
  "pair".padEnd(46), "N".padStart(5),
  "uniq".padStart(8), "ambig<".padStart(8), "ambig>".padStart(8),
  "fuzzy".padStart(8), "notfnd".padStart(8), "hibiFnd".padStart(8),
  "trail✓".padStart(8), "rescue/NF".padStart(9), "disamb/A".padStart(9),
  "mislead".padStart(8), "falseAt/NF".padStart(9),
].join(" ");

console.log("=== PER-PAIR BREAKDOWN ===");
console.log(header);
for (const p of PAIRS) {
  const rows = results.filter((r) => r.pair === p.label);
  if (rows.length) console.log(fmtRow(p.label.slice(0, 46), agg(rows)));
}
console.log("-".repeat(header.length));
console.log(fmtRow("ALL PAIRS", agg(results)));

const A = agg(results);
console.log("\n=== AGGREGATE DETAIL (counts over N=" + A.N + " sentences) ===");
console.log(`exact-unique:                 ${A.unique} (${pct(A.unique, A.N)})`);
console.log(`ambiguous-without-context:    ${A.ambigBefore} (${pct(A.ambigBefore, A.N)})  [exact quote matched >1 place]`);
console.log(`  resolved by 48c context:    ${A.ambigResolved} (${pct(A.ambigResolved, A.ambigBefore)} of ambiguous)`);
console.log(`ambiguous-after-context:      ${A.ambigAfter} (${pct(A.ambigAfter, A.N)})`);
console.log(`fuzzy-rescued (sim>=0.75):    ${A.fuzzy} (${pct(A.fuzzy, A.N)})`);
console.log(`not-found:                    ${A.notFound} (${pct(A.notFound, A.N)})`);
console.log(`hibi real Bitap localized:    ${A.hibiFound} (${pct(A.hibiFound, A.N)}) [cross-check: hibi's own cascade]`);
console.log(`heading-trail survived:       ${A.trailSurvNonRoot}/${A.nonRoot} non-root sentences (${pct(A.trailSurvNonRoot, A.nonRoot)})`);
console.log(`sentences that moved section: ${A.moved} (of ${A.N - results.filter(r=>r.movedSection===null).length} with known new location)`);
console.log(`heading-path rescued:         ${A.rescued} of ${A.notFound} NOT_FOUND (${pct(A.rescued, A.notFound)}) [${A.rescuedBlock} via block-index]`);
console.log(`heading-path disambiguated:   ${A.disamb} of ${A.ambigAfter} still-ambiguous (${pct(A.disamb, A.ambigAfter)})`);
console.log(`heading-path would MISLEAD:   ${A.mislead} (${pct(A.mislead, A.N)}) [found elsewhere, old trail still exists]`);
console.log(`false-attach risk:            ${A.falseAttach} of ${A.notFound} NOT_FOUND (${pct(A.falseAttach, A.notFound)}) [block-index points at unrelated text]`);

// heading survival aggregate
let oh = 0, hs = 0, ot = 0, ts = 0;
for (const h of headingStats) { oh += h.oldHeadings; hs += h.headingsSurvived; ot += h.oldTrails; ts += h.trailsSurvived; }
console.log(`\n=== HEADING STABILITY ===`);
console.log(`old headings across all pairs: ${oh}; exact text survived: ${hs} (${pct(hs, oh)})`);
console.log(`old heading trails: ${ot}; full trail survived: ${ts} (${pct(ts, ot)})`);
console.log(`\nper-pair heading survival:`);
const byPair = new Map<string, { oh: number; hs: number; ot: number; ts: number }>();
for (const h of headingStats) {
  const e = byPair.get(h.pair) ?? { oh: 0, hs: 0, ot: 0, ts: 0 };
  e.oh += h.oldHeadings; e.hs += h.headingsSurvived; e.ot += h.oldTrails; e.ts += h.trailsSurvived;
  byPair.set(h.pair, e);
}
for (const [pair, e] of byPair) {
  console.log(`  ${pair.slice(0, 50).padEnd(52)} headings ${e.hs}/${e.oh} (${pct(e.hs, e.oh)})  trails ${e.ts}/${e.ot} (${pct(e.ts, e.ot)})`);
}

// ---------- NOT_FOUND human-judgment sample -----------------------------------
console.log(`\n=== NOT_FOUND SAMPLE (~15, spread across pairs; judge: deleted vs reworded) ===`);
const nf = results.filter((r) => r.outcome === "NOT_FOUND");
// spread: sort by pair then take stride, prefer diverse similarity levels
const perPairNF = new Map<string, Result[]>();
for (const r of nf) {
  if (!perPairNF.has(r.pair)) perPairNF.set(r.pair, []);
  perPairNF.get(r.pair)!.push(r);
}
const sample: Result[] = [];
const quota = Math.max(2, Math.ceil(15 / Math.max(1, perPairNF.size)));
for (const [, rows] of perPairNF) {
  rows.sort((a, b) => b.bestGlobalSim - a.bestGlobalSim);
  const step = Math.max(1, Math.floor(rows.length / quota));
  for (let i = 0; i < rows.length && sample.length < 15 && i / step < quota; i += step) sample.push(rows[i]!);
}
let si = 1;
for (const r of sample.slice(0, 15)) {
  console.log(`\n[${si++}] pair: ${r.pair}`);
  console.log(`    file: ${r.file}   trail: ${r.oldTrail || "(root)"}   trailSurvived: ${r.trailSurvived}   rescue: ${r.headingRescue ?? "no"}`);
  console.log(`    OLD: ${r.sentence}`);
  console.log(`    CLOSEST NEW (sim ${r.bestGlobalSim.toFixed(2)}): ${r.bestGlobalFragment || "(none)"}`);
}

// dump full results for further inspection
const RESULTS_PATH = new URL("results.json", import.meta.url).pathname;
await Bun.write(RESULTS_PATH, JSON.stringify({ results, headingStats }, null, 1));
console.log(`\n(full per-sentence results: ${RESULTS_PATH} — gitignored, regenerate anytime)`);
