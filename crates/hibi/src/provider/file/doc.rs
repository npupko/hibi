//! One decoded resource and its units: building them (sections 6.1 to 6.6),
//! the cache form (section 11.3), resolution (section 6.7) and render
//! (section 6.9).
//!
//! Pure: everything here is a function of the resource text and its class.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

use crate::code::{self, Analysis, Class};
use crate::fingerprint::{self, AST_V1};
use crate::locate::{self, Span};
use crate::model::{Fingerprint, MAX_QUOTE_CHARS, Selector};
use crate::prose::{self, ProseClass};
use crate::provider::{Identity, ProviderError, ProviderErrorCode, Range, Refuse, Rendered, Unit};
use crate::text;

/// The most units one resource may have, the `file` unit included. A
/// resource with more is `too_large` (ruling on section 6.1).
pub const MAX_UNITS_PER_RESOURCE: usize = 100_000;

/// Characters of `normalize(text)` in a label (section 6.2).
const LABEL_CHARS: usize = 80;

#[cfg(test)]
thread_local! {
    /// Steps of the unit and comment lookups on this thread. Tests compare
    /// it across input sizes, so that a complexity check does not depend on
    /// machine speed.
    static WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_work(n: usize) {
    WORK.with(|w| w.set(w.get() + n));
}

#[cfg(not(test))]
#[inline(always)]
fn count_work(_: usize) {}

fn too_many_units() -> ProviderError {
    ProviderError::new(
        ProviderErrorCode::TooLarge,
        format!("the resource has more than {MAX_UNITS_PER_RESOURCE} units, the limit"),
    )
}

/// A byte counts as filler at the edge of a quote that runs past its unit:
/// whitespace, `,` or `;` (ruling on section 6.7).
fn is_filler(c: char) -> bool {
    text::is_ws(c) || c == ',' || c == ';'
}
/// Characters of a default quote (section 6.7).
const DEFAULT_QUOTE_CHARS: usize = 120;

/// One unit of a resource. `unit` has no `text`, `approx` false, no
/// `refuse`, and the completed selector built with the default quote.
#[derive(Debug, Clone)]
struct Entry {
    unit: Unit,
    /// The declaration path; `None` for every unit that is not a declaration.
    symbol: Option<String>,
    start: usize,
    end: usize,
}

/// A decoded resource with its units: the `file` unit first, then every
/// other unit in source order.
#[derive(Debug)]
pub(super) struct Doc {
    text: String,
    class: Class,
    /// True when the file has a parse tree: a code class that parsed
    /// without error. A code class without a tree is never used: it is
    /// `unavailable` (section 6.1, [`Doc::check_parses`]).
    tree: bool,
    entries: Vec<Entry>,
    /// Per entry: the index of its parent entry (`None` for the `file`
    /// unit). Units nest, so the parent chain is the containment chain.
    parents: Vec<Option<usize>>,
    /// The declarations of each symbol, in `units` order.
    symbols: HashMap<String, Vec<usize>>,
    /// The normalized text and its map back to the text (section 6.7).
    /// Filled on first use.
    norm: OnceCell<Norm>,
    /// Comment node ranges, when `tree`, sorted and disjoint. Filled on
    /// first use after a cache hit.
    comments: OnceCell<Vec<(usize, usize)>>,
    /// Per entry: heading path, piece and piece name of a prose unit
    /// (section 6.7). Filled on first use.
    prose: OnceCell<ProseIndex>,
}

/// The first `LABEL_CHARS` characters of `normalize(s)`, without
/// normalizing the whole of `s`.
fn label_of(s: &str) -> String {
    let mut out = String::new();
    let mut n = 0;
    let mut pending_space = false;
    for c in s.chars() {
        if n == LABEL_CHARS {
            break;
        }
        if text::is_ws(c) {
            pending_space = n > 0;
        } else {
            if pending_space {
                out.push(' ');
                n += 1;
                pending_space = false;
                if n == LABEL_CHARS {
                    break;
                }
            }
            out.push(c);
            n += 1;
        }
    }
    out
}

/// `endLine` of the `file` unit: the line of the last character.
fn last_line(text: &str) -> usize {
    let last = text.char_indices().last().map_or(0, |(i, _)| i);
    text::line_at(text, last)
}

fn prose_class(class: Class) -> Option<ProseClass> {
    match class {
        Class::Markdown => Some(ProseClass::Markdown),
        Class::Prose => Some(ProseClass::Prose),
        _ => None,
    }
}

/// A unit without its selector, which [`Doc::finish`] adds.
struct Raw {
    key: String,
    kind: String,
    parent: Option<String>,
    symbol: Option<String>,
    start: usize,
    end: usize,
    line: usize,
    end_line: usize,
    fp: Fingerprint,
}

impl Doc {
    /// Build the units of `text`, the resource text of section 4.2, read
    /// with the class of its path.
    pub(super) fn build(text: String, class: Class) -> Result<Doc, ProviderError> {
        let file = |fp: Fingerprint, text: &str| Raw {
            key: "file".into(),
            kind: "file".into(),
            parent: None,
            symbol: None,
            start: 0,
            end: text.len(),
            line: 1,
            end_line: last_line(text),
            fp,
        };
        let mut raws = Vec::new();
        let mut tree = false;
        let comments = OnceCell::new();
        let mut regions: Option<Vec<prose::Region>> = None;
        if let Some(pc) = prose_class(class) {
            // Building stops as soon as the count passes the limit.
            let split = prose::split_at_most(&text, pc, MAX_UNITS_PER_RESOURCE - 1)
                .ok_or_else(too_many_units)?;
            raws.push(file(fingerprint::prose_v1(&text), &text));
            let units = prose::units_from(&text, split);
            let mut kept = Vec::with_capacity(units.len());
            for u in units {
                kept.push(u.region.clone());
                raws.push(Raw {
                    key: u.key,
                    kind: u.region.kind.as_str().into(),
                    parent: Some("file".into()),
                    symbol: None,
                    start: u.region.start,
                    end: u.region.end,
                    line: u.region.line,
                    end_line: u.region.end_line,
                    fp: u.fp,
                });
            }
            regions = Some(kept);
        } else {
            let blocks = if class.is_code() {
                match code::analyze_at_most(&text, class, MAX_UNITS_PER_RESOURCE)
                    .ok_or_else(too_many_units)?
                {
                    Analysis::Code(cf) => {
                        tree = true;
                        let _ = comments.set(cf.comments);
                        for u in cf.units {
                            raws.push(Raw {
                                key: u.key,
                                kind: u.kind.into(),
                                parent: u.parent,
                                symbol: u.symbol,
                                start: u.start,
                                end: u.end,
                                line: u.line,
                                end_line: u.end_line,
                                fp: u.fp,
                            });
                        }
                        None
                    }
                    // No units: `FileProvider` reports the file as
                    // `unavailable` (section 6.1) before any use.
                    Analysis::Unparsed => Some(Vec::new()),
                }
            } else {
                Some(
                    code::text_units_at_most(&text, MAX_UNITS_PER_RESOURCE - 1)
                        .ok_or_else(too_many_units)?,
                )
            };
            if let Some(blocks) = blocks {
                raws.push(file(fingerprint::prose_v1(&text), &text));
                raws.extend(blocks.into_iter().map(|b| Raw {
                    key: b.key,
                    kind: "block".into(),
                    parent: Some("file".into()),
                    symbol: None,
                    start: b.start,
                    end: b.end,
                    line: b.line,
                    end_line: b.end_line,
                    fp: b.fp,
                }));
            }
        }
        Doc::finish(text, class, tree, raws, comments, regions)
    }

    fn finish(
        text: String,
        class: Class,
        tree: bool,
        raws: Vec<Raw>,
        comments: OnceCell<Vec<(usize, usize)>>,
        regions: Option<Vec<prose::Region>>,
    ) -> Result<Doc, ProviderError> {
        let mut doc = Doc {
            text,
            class,
            tree,
            entries: Vec::with_capacity(raws.len()),
            parents: Vec::new(),
            symbols: HashMap::new(),
            norm: OnceCell::new(),
            comments,
            prose: OnceCell::new(),
        };
        for r in raws {
            let label = match &r.symbol {
                Some(s) => s.clone(),
                None => label_of(&doc.text[r.start..r.end]),
            };
            let entry = Entry {
                unit: Unit {
                    key: r.key,
                    kind: r.kind,
                    parent: r.parent,
                    label: Some(label),
                    range: Some(Range {
                        start: r.start,
                        end: r.end,
                        line: r.line,
                        end_line: r.end_line,
                    }),
                    fp: r.fp,
                    sel: Selector::empty(),
                    text: None,
                    approx: false,
                    identity: None,
                    refuse: None,
                },
                symbol: r.symbol,
                start: r.start,
                end: r.end,
            };
            doc.entries.push(entry);
        }
        doc.index();
        if let Some(regions) = regions {
            let index = doc.prose_index_from(&regions);
            let _ = doc.prose.set(index);
        }
        let sels = doc.printed()?;
        for (e, (sel, alone)) in doc.entries.iter_mut().zip(sels) {
            e.unit.sel = sel;
            e.unit.refuse = (!alone).then_some(Refuse::QuoteNotDistinguishable);
        }
        Ok(doc)
    }

    /// Fill `parents` and `symbols` from the entries.
    fn index(&mut self) {
        let mut by_key: HashMap<&str, usize> = HashMap::with_capacity(self.entries.len());
        let mut symbols: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, e) in self.entries.iter().enumerate() {
            by_key.insert(e.unit.key.as_str(), i);
            if let Some(s) = &e.symbol {
                symbols.entry(s.clone()).or_default().push(i);
            }
        }
        self.symbols = symbols;
        self.parents = self
            .entries
            .iter()
            .map(|e| {
                e.unit
                    .parent
                    .as_deref()
                    .and_then(|p| by_key.get(p).copied())
            })
            .collect();
    }

    /// The cache form of section 11.3: the JSON array of the units without
    /// `text`.
    pub(super) fn to_cache(&self) -> String {
        let units: Vec<&Unit> = self.entries.iter().map(|e| &e.unit).collect();
        serde_json::to_string(&units).expect("units serialize")
    }

    /// Rebuild a document from its cache form. `None` when the row is not
    /// exactly what [`Doc::to_cache`] writes for a text like this one: the
    /// caller then ignores it and rewrites it. The checks, each of which
    /// makes the row a miss:
    ///
    /// 1. it is a JSON array with at least one element;
    /// 2. every element is an object with exactly the keys `key`, `kind`,
    ///    `parent`, `label`, `range`, `fp`, `sel`, and `refuse` with the
    ///    value `quote_not_distinguishable` for a unit no anchor selects
    ///    alone;
    /// 3. every element reads as a unit (valid fingerprint and selector);
    /// 4. every kind is one of the file provider's kinds;
    /// 5. the first unit is the `file` unit over `[0, len)` with no parent,
    ///    and no other unit is a `file` unit;
    /// 6. every range lies within the text, on character boundaries;
    /// 7. units after the first are ordered by start, then end descending;
    /// 8. keys are unique, and every parent names an earlier unit;
    /// 9. the profile fits the class: `ast-v1` only for a code class.
    pub(super) fn from_cache(text: String, class: Class, row: &str) -> Option<Doc> {
        const KEYS: [&str; 7] = ["key", "kind", "parent", "label", "range", "fp", "sel"];
        const MARK: &str = "refuse";
        let value: serde_json::Value = serde_json::from_str(row).ok()?;
        let items = value
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= MAX_UNITS_PER_RESOURCE)?;
        let mut entries: Vec<Entry> = Vec::with_capacity(items.len());
        let mut keys: HashSet<String> = HashSet::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let obj = item.as_object()?;
            let marked = obj.contains_key(MARK);
            if obj.len() != KEYS.len() + usize::from(marked)
                || !KEYS.iter().all(|k| obj.contains_key(*k))
            {
                return None;
            }
            let u: Unit = serde_json::from_value(item.clone()).ok()?;
            if !super::KINDS.contains(&u.kind.as_str())
                || u.text.is_some()
                || (marked && u.refuse != Some(Refuse::QuoteNotDistinguishable))
            {
                return None;
            }
            let r = u.range?;
            if r.start > r.end
                || r.end > text.len()
                || !text.is_char_boundary(r.start)
                || !text.is_char_boundary(r.end)
            {
                return None;
            }
            let is_file = u.kind == "file";
            if (i == 0) != is_file
                || (i == 0 && (u.key != "file" || r.start != 0 || r.end != text.len()))
                || (i == 0) != u.parent.is_none()
            {
                return None;
            }
            if i >= 2 {
                let prev = &entries[i - 1];
                if (r.start, std::cmp::Reverse(r.end)) < (prev.start, std::cmp::Reverse(prev.end)) {
                    return None;
                }
            }
            if u.parent.as_ref().is_some_and(|p| !keys.contains(p)) {
                return None;
            }
            if !keys.insert(u.key.clone()) {
                return None;
            }
            entries.push(Entry {
                symbol: u.sel.symbol.clone(),
                start: r.start,
                end: r.end,
                unit: u,
            });
        }
        let profile = entries[0].unit.fp.profile().to_owned();
        if profile == AST_V1 && !class.is_code() {
            return None;
        }
        // A code class has rows only of a parsed file (section 11.3): a row
        // of text blocks for a code class is a miss.
        if class.is_code() && profile != AST_V1 {
            return None;
        }
        let tree = class.is_code();
        let mut doc = Doc {
            text,
            class,
            tree,
            entries,
            parents: Vec::new(),
            symbols: HashMap::new(),
            norm: OnceCell::new(),
            comments: OnceCell::new(),
            prose: OnceCell::new(),
        };
        doc.index();
        Some(doc)
    }

    fn comments(&self) -> &[(usize, usize)] {
        self.comments.get_or_init(|| {
            if !self.tree {
                return Vec::new();
            }
            code::parse(&self.text, self.class)
                .map(|t| code::comment_spans(t.root_node()))
                .unwrap_or_default()
        })
    }

    /// `in_comment(L)` of section 6.7 for a span. The comment ranges are
    /// sorted and disjoint, so only the last comment that starts before the
    /// span's end can intersect it: a binary search.
    fn in_comment(&self, s: Span) -> bool {
        if !self.tree {
            return false;
        }
        count_work(1);
        let comments = self.comments();
        let n = comments.partition_point(|&(cs, _)| cs < s.end);
        n > 0 && s.start < comments[n - 1].1
    }

    /// The start of the first comment that starts in `[from, to)`.
    fn first_comment_in(&self, from: usize, to: usize) -> Option<usize> {
        count_work(1);
        let comments = self.comments();
        let n = comments.partition_point(|&(cs, _)| cs < from);
        comments.get(n).map(|&(cs, _)| cs).filter(|&cs| cs < to)
    }

    /// The default quote span of entry `i` (section 6.7): the first line of
    /// the unit's text, cut before the first comment node that starts on
    /// that line, trailing WS removed, cut to 120 characters, and trailing
    /// WS removed again so that the quote never ends in WS (ruling on
    /// section 6.7).
    fn default_quote_end(&self, i: usize) -> usize {
        let e = &self.entries[i];
        let unit_text = &self.text[e.start..e.end];
        let line_end = e.start + unit_text.find('\n').unwrap_or(unit_text.len());
        let cut = if self.tree {
            self.first_comment_in(e.start, line_end).unwrap_or(line_end)
        } else {
            line_end
        };
        let mut end = e.start + text::trim_end(&self.text[e.start..cut]).len();
        if text::trim(&self.text[e.start..end]).is_empty() {
            // The first line starts with a comment: the rule leaves nothing.
            // Use the whole first line instead (not specified; reported).
            end = e.start + text::trim_end(&self.text[e.start..line_end]).len();
        }
        let q = &self.text[e.start..end];
        let q = match q.char_indices().nth(DEFAULT_QUOTE_CHARS) {
            Some((b, _)) => &q[..b],
            None => q,
        };
        e.start + text::trim_end(q).len()
    }

    /// The units of the resource, optionally of one kind, in order.
    pub(super) fn units(&self, kind: Option<&str>) -> Vec<Unit> {
        self.entries
            .iter()
            .filter(|e| kind.is_none_or(|k| e.unit.kind == k))
            .map(|e| e.unit.clone())
            .collect()
    }

    /// `render` of section 6.9.
    pub(super) fn render(&self, key: &str) -> Result<Rendered, ProviderError> {
        let Some(i) = self.entries.iter().position(|e| e.unit.key == key) else {
            return Err(ProviderError::new(
                ProviderErrorCode::UnknownUnit,
                format!("the resource has no unit with key {key:?}"),
            ));
        };
        let e = &self.entries[i];
        let context = if prose_class(self.class).is_some() {
            self.prose()
                .get(i)
                .map(|p| p.raw_path.clone())
                .unwrap_or_default()
        } else {
            e.unit
                .parent
                .as_deref()
                .and_then(|p| self.entries.iter().find(|x| x.unit.key == p))
                .and_then(|p| p.symbol.clone())
                .into_iter()
                .collect()
        };
        Ok(Rendered {
            text: self.text[e.start..e.end].to_owned(),
            context,
        })
    }

    // -----------------------------------------------------------------------
    // Resolve (section 6.7)
    // -----------------------------------------------------------------------

    /// The parts of `span` outside unit `i` are only filler:
    /// `inside(span, u)` of section 6.7.
    fn within(&self, span: Span, i: usize) -> bool {
        let e = &self.entries[i];
        let before = &self.text[span.start..span.start.max(e.start.min(span.end))];
        let after = &self.text[span.start.max(e.end).min(span.end)..span.end];
        before.chars().all(is_filler) && after.chars().all(is_filler)
    }

    /// `scope(u)`: the range of unit `i` widened over the filler directly
    /// before and after it.
    fn symbol_scope(&self, i: usize) -> (usize, usize) {
        let e = &self.entries[i];
        let before = &self.text[..e.start];
        let from = before.trim_end_matches(is_filler).len();
        let after = &self.text[e.end..];
        let to = e.end + (after.len() - after.trim_start_matches(is_filler).len());
        (from, to)
    }

    /// `core(span)`: `span` without leading and trailing whitespace
    /// (unchanged when it is all whitespace).
    fn trimmed(&self, span: Span) -> Span {
        let t = &self.text[span.start..span.end];
        let lead = t.len() - text::trim_start(t).len();
        let core = text::trim(t);
        if core.is_empty() {
            return span;
        }
        Span {
            start: span.start + lead,
            end: span.start + lead + core.len(),
        }
    }

    /// The innermost unit for a span (section 6.7): among the units other
    /// than `file` that contain the first byte of its core, the smallest
    /// (the earliest of equal ranges); if none, the first unit in source
    /// order that intersects it; if none, the `file` unit.
    ///
    /// Units nest and are sorted by start, then end descending, so the last
    /// unit that starts at or before the byte, or one of its ancestors, is
    /// the smallest one containing it: a binary search and a walk up the
    /// parent chain, not a scan of every unit.
    fn innermost(&self, span: Span) -> usize {
        let s = self.trimmed(span);
        let rest = &self.entries[1..];
        let k = rest.partition_point(|e| e.start <= s.start);
        let mut i = k;
        while i > 0 {
            count_work(1);
            let e = &self.entries[i];
            if e.start <= s.start && s.start < e.end {
                while i > 1
                    && self.entries[i - 1].start == e.start
                    && self.entries[i - 1].end == e.end
                {
                    i -= 1;
                }
                return i;
            }
            match self.parents[i] {
                Some(p) => i = p,
                None => break,
            }
        }
        match rest.get(k) {
            Some(e) if e.start < s.end => k + 1,
            _ => 0,
        }
    }

    /// Error 9 of section 6.7: a selector with a `symbol` on a class without
    /// symbols (markdown, prose, text) is `invalid_selector`, because no
    /// anchor there can name a declaration.
    pub(super) fn check_symbol(&self, loc: &str) -> Result<(), ProviderError> {
        if !self.class.is_code() {
            return Err(ProviderError::new(
                ProviderErrorCode::InvalidSelector,
                format!(
                    "{loc} is a {} file, and this kind of file has no symbols; anchor it with a quote instead",
                    self.class.name()
                ),
            ));
        }
        Ok(())
    }

    /// Error 6 of section 6.7: a file of a code class that does not parse is
    /// `unavailable` for `resolve` with any selector, for `units` and for
    /// `render`, with the line of its first error (section 6.1). Its units
    /// come back unchanged when the syntax error is fixed.
    pub(super) fn check_parses(&self, loc: &str) -> Result<(), ProviderError> {
        if !self.class.is_code() || self.tree {
            return Ok(());
        }
        let line = code::first_error_line(&self.text, self.class);
        let at = line.map_or(String::new(), |l| {
            format!(" (the first error is near line {l})")
        });
        Err(ProviderError::new(
            ProviderErrorCode::Unavailable,
            format!(
                "{loc} does not parse as {}{at}, so its units cannot be read; fix the syntax error",
                self.class.name()
            ),
        ))
    }

    // -- hits ---------------------------------------------------------------

    /// The normalized text with its map back to the resource text, built on
    /// first use.
    fn norm(&self) -> &Norm {
        self.norm.get_or_init(|| Norm::of(&self.text))
    }

    /// The hits of `quote` (section 6.7) whose span lies in `[from, to)` of
    /// the resource text, left to right, without overlap, each with its
    /// unit and whether it is exact.
    fn hits_in(&self, quote: &str, from: usize, to: usize) -> Vec<Hit> {
        let q = text::normalize(quote);
        if q.is_empty() {
            return Vec::new();
        }
        let n = self.norm();
        let nfrom = n.src.partition_point(|&s| (s as usize) < from);
        let nto = n.src.partition_point(|&s| (s as usize) < to);
        n.text[nfrom..nto]
            .match_indices(q.as_str())
            .map(|(b, _)| {
                count_work(1);
                let span = n.span(&self.text, nfrom + b, nfrom + b + q.len());
                let unit = self.innermost(span);
                Hit {
                    span,
                    unit,
                    exact: self.within(span, unit),
                }
            })
            .collect()
    }

    /// Every hit of `quote` in the resource.
    fn hits(&self, quote: &str) -> Vec<Hit> {
        self.hits_in(quote, 0, self.text.len())
    }

    /// `prefix(h, u)`: the last 48 characters of the normalized text of
    /// unit `u` before the span, comments removed.
    fn prefix_window(&self, u: usize, span: Span) -> String {
        let e = &self.entries[u];
        let end = span.start.clamp(e.start, e.end);
        let rev = self
            .code_segments(e.start, end)
            .into_iter()
            .rev()
            .flat_map(|seg| match seg {
                Seg::Text(a, b) => {
                    Box::new(self.text[a..b].chars().rev()) as Box<dyn Iterator<Item = char>>
                }
                Seg::Comment => Box::new(std::iter::once(' ')),
            });
        text::trim(&last_normalized_of(rev, WINDOW_CHARS)).to_owned()
    }

    /// `suffix(h, u)`: the first 48 characters of the normalized text of
    /// unit `u` after the span, comments removed.
    fn suffix_window(&self, u: usize, span: Span) -> String {
        let e = &self.entries[u];
        let start = span.end.clamp(e.start, e.end);
        text::trim(&first_normalized_of(
            self.code_chars(start, e.end),
            WINDOW_CHARS,
        ))
        .to_owned()
    }

    /// `text[from..to]` as code sees it, in order: the stretches of text
    /// between comments, and each comment (section 6.7: in-unit windows
    /// read every comment as one space, as the fingerprint does not see
    /// comments). Without a parse tree, the text alone.
    fn code_segments(&self, from: usize, to: usize) -> Vec<Seg> {
        let comments: &[(usize, usize)] = if self.tree { self.comments() } else { &[] };
        let lo = comments.partition_point(|&(_, ce)| ce <= from);
        let hi = comments.partition_point(|&(cs, _)| cs < to);
        let mut out = Vec::with_capacity(2 * (hi - lo) + 1);
        let mut at = from;
        for &(cs, ce) in &comments[lo..hi] {
            count_work(1);
            if cs > at {
                out.push(Seg::Text(at, cs));
            }
            out.push(Seg::Comment);
            at = ce.min(to).max(at);
        }
        if at < to {
            out.push(Seg::Text(at, to));
        }
        out
    }

    /// The characters of [`Doc::code_segments`], a comment as one space.
    fn code_chars(&self, from: usize, to: usize) -> impl Iterator<Item = char> + '_ {
        self.code_segments(from, to)
            .into_iter()
            .flat_map(move |seg| match seg {
                Seg::Text(a, b) => {
                    Box::new(self.text[a..b].chars()) as Box<dyn Iterator<Item = char>>
                }
                Seg::Comment => Box::new(std::iter::once(' ')),
            })
    }

    /// Some comment intersects `text[from..to]`.
    fn has_comment_in(&self, from: usize, to: usize) -> bool {
        self.tree
            && self
                .code_segments(from, to)
                .iter()
                .any(|s| matches!(s, Seg::Comment))
    }

    /// The span reproduces the stored windows of `sel` in unit `u`: a
    /// stored `prefix` or `suffix` equals, after `normalize`, the window
    /// cut by the same rule; a window the selector lacks is not compared.
    fn reproduces(&self, sel: &Selector, u: usize, span: Span) -> bool {
        sel.prefix
            .as_deref()
            .is_none_or(|p| text::normalize(p) == self.prefix_window(u, span))
            && sel
                .suffix
                .as_deref()
                .is_none_or(|s| text::normalize(s) == self.suffix_window(u, span))
    }

    // -- containers ---------------------------------------------------------

    /// Per entry: the heading keys above the unit, the piece it was cut
    /// from, and whether that piece holds two or more units (`markdown` and
    /// `prose` only). Filled on first use.
    fn prose(&self) -> &[ProseInfo] {
        &self.prose_index().info
    }

    /// [`Doc::prose`] with the lookups of `names`: the section paths that
    /// some unit has, and the multi-unit pieces by heading path and name.
    fn prose_index(&self) -> &ProseIndex {
        self.prose.get_or_init(|| {
            let regions = prose_class(self.class)
                .map(|pc| prose::split(&self.text, pc))
                .unwrap_or_default();
            self.prose_index_from(&regions)
        })
    }

    /// The prose index from the regions of the splitter, which are the
    /// entries after the `file` unit, in order.
    fn prose_index_from(&self, regions: &[prose::Region]) -> ProseIndex {
        {
            let info = self.prose_info(regions);
            let mut sections: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
            let mut pieces: HashMap<(Vec<String>, String), Vec<usize>> = HashMap::new();
            for p in info.iter().skip(1) {
                // A section's units come before the next section of the
                // same path.
                let list = sections.entry(p.path.clone()).or_default();
                if list.last() != Some(&p.section) {
                    list.push(p.section);
                }
                if p.multi {
                    let list = pieces.entry((p.path.clone(), p.name.clone())).or_default();
                    if list.last() != Some(&p.piece) {
                        list.push(p.piece);
                    }
                }
            }
            ProseIndex {
                info,
                sections,
                pieces,
            }
        }
    }

    fn prose_info(&self, regions: &[prose::Region]) -> Vec<ProseInfo> {
        {
            let mut info = vec![ProseInfo::default(); self.entries.len()];
            let mut sizes: HashMap<usize, usize> = HashMap::new();
            for r in regions {
                *sizes.entry(r.piece).or_default() += 1;
            }
            // The text of each multi-unit piece: from its first unit's start
            // to its last unit's end, and its first unit's index.
            let mut spans: HashMap<usize, (usize, usize, usize)> = HashMap::new();
            for (k, r) in regions.iter().enumerate() {
                let s = spans.entry(r.piece).or_insert((r.start, r.end, k + 1));
                s.1 = r.end;
            }
            // The headings of the current path, as entry indexes: the last
            // one starts the section of a unit below it. A heading itself
            // is in the section of the heading above it.
            let mut stack: Vec<usize> = Vec::new();
            for (k, r) in regions.iter().enumerate() {
                let depth = r.heading_path.len();
                stack.truncate(depth);
                let section = if depth == 0 {
                    0
                } else {
                    stack.get(depth - 1).copied().unwrap_or(0)
                };
                if r.kind == prose::ProseKind::Heading {
                    stack.push(k + 1);
                }
                let Some(slot) = info.get_mut(k + 1) else {
                    continue;
                };
                let multi = sizes[&r.piece] > 1;
                // The cut at 120 characters can land on a space, which is
                // removed: `names` compares normalized strings.
                let name = if multi {
                    let (start, end, first) = spans[&r.piece];
                    let first = &regions[first - 1];
                    let name = if first.kind == prose::ProseKind::TableRow {
                        first_normalized(first_cell(&self.text[start..end]), PIECE_NAME_CHARS)
                    } else {
                        first_normalized(&self.text[first.start..first.end], PIECE_NAME_CHARS)
                    };
                    text::trim_end(&name).to_owned()
                } else {
                    String::new()
                };
                *slot = ProseInfo {
                    raw_path: r.heading_path.clone(),
                    path: r.heading_path.iter().map(|h| heading_key(h)).collect(),
                    section,
                    piece: r.piece,
                    multi,
                    name,
                };
            }
            info
        }
    }

    /// `within(h)` of section 6.7 for a hit in unit `u`.
    fn within_of(&self, u: usize) -> Option<Vec<String>> {
        if u == 0 {
            return None;
        }
        if prose_class(self.class).is_some() {
            let p = &self.prose()[u];
            let mut w = p.path.clone();
            if p.multi {
                w.push(p.name.clone());
            }
            return (!w.is_empty()).then_some(w);
        }
        if !self.tree {
            return None;
        }
        // The hit's unit when it is a declaration, else the innermost
        // declaration holding it. The hit is inside its unit (filler
        // allowed), so that declaration holds the hit.
        let mut i = u;
        loop {
            if let Some(s) = &self.entries[i].symbol {
                return Some(vec![s.clone()]);
            }
            i = self.parents[i]?;
        }
    }

    /// `names(w)`: the containers a `within` list names now.
    fn names(&self, w: &[String]) -> Vec<Container> {
        let w: Vec<String> = w.iter().map(|x| text::normalize(x)).collect();
        if prose_class(self.class).is_some() {
            let index = self.prose_index();
            let mut found: Vec<Container> = index
                .sections
                .get(&w)
                .map(|s| s.iter().map(|&x| Container::Section(x)).collect())
                .unwrap_or_default();
            if let Some((name, path)) = w.split_last()
                && let Some(pieces) = index.pieces.get(&(path.to_vec(), name.clone()))
            {
                found.extend(pieces.iter().map(|&p| Container::Piece(p)));
            }
            count_work(found.len());
            return found;
        }
        if !self.tree || w.len() != 1 {
            return Vec::new();
        }
        let found: Vec<Container> = self
            .symbols
            .get(w[0].as_str())
            .map(|d| d.iter().map(|&i| Container::Decl(i)).collect())
            .unwrap_or_default();
        count_work(found.len());
        found
    }

    /// How many containers `names(w)` holds, without listing them.
    /// Two lookups: a unit's anchor is completed for every unit, and a
    /// `within` can name thousands of containers.
    fn names_count(&self, w: &[String]) -> usize {
        if prose_class(self.class).is_some() {
            let w: Vec<String> = w.iter().map(|x| text::normalize(x)).collect();
            let index = self.prose_index();
            let sections = index.sections.get(&w).map_or(0, Vec::len);
            let pieces = w.split_last().map_or(0, |(name, path)| {
                index
                    .pieces
                    .get(&(path.to_vec(), name.clone()))
                    .map_or(0, Vec::len)
            });
            return sections + pieces;
        }
        if !self.tree || w.len() != 1 {
            return 0;
        }
        self.symbols
            .get(text::normalize(&w[0]).as_str())
            .map_or(0, Vec::len)
    }

    /// The containers unit `u` is in: its section and, for a unit of a
    /// multi-unit piece, that piece (prose), or every declaration holding
    /// it, itself included (code).
    fn containers_of(&self, u: usize) -> Vec<Container> {
        if u == 0 {
            return Vec::new();
        }
        if prose_class(self.class).is_some() {
            let p = &self.prose()[u];
            let mut out = vec![Container::Section(p.section)];
            if p.multi {
                out.push(Container::Piece(p.piece));
            }
            return out;
        }
        let mut out = Vec::new();
        let mut a = Some(u);
        while let Some(x) = a {
            if self.entries[x].symbol.is_some() {
                out.push(Container::Decl(x));
            }
            a = self.parents[x];
        }
        out
    }

    /// Unit `u` is in one of the containers of `scope`.
    fn in_scope(&self, scope: &Scope, u: usize) -> bool {
        self.containers_of(u).iter().any(|c| scope.set.contains(c))
    }

    /// The scope a stored `within` gives: the union of the containers it
    /// names, when it names exactly as many as `count` says (one when
    /// `count` is absent); `None` otherwise (section 6.7).
    fn scope_of(&self, within: &[String], count: Option<u64>) -> Option<Scope> {
        let c = self.names(within);
        let want = count.map_or(1, |c| usize::try_from(c).unwrap_or(usize::MAX));
        (c.len() == want).then(|| Scope::of(c))
    }

    // -- completed and printed selectors -------------------------------------

    /// A quote as stored, with the span its windows are cut around: the
    /// resource text of the span; when that is longer than
    /// `MAX_QUOTE_CHARS` characters, its normalized text, when that is not;
    /// otherwise the text cut to `MAX_QUOTE_CHARS` characters with trailing
    /// WS then removed, and the span cut with it. A typed quote is never
    /// cut: the normalized text of its hit is the typed quote normalized,
    /// which has at most `MAX_QUOTE_CHARS` characters. The stored selector
    /// finds its own quote again.
    fn stored_quote(&self, span: Span) -> (String, Span) {
        let q = &self.text[span.start..span.end];
        let Some((b, _)) = q.char_indices().nth(MAX_QUOTE_CHARS) else {
            return (q.to_owned(), span);
        };
        let n = text::normalize(q);
        if n.chars().nth(MAX_QUOTE_CHARS).is_none() {
            return (n, span);
        }
        let cut = Span {
            start: span.start,
            end: span.start + text::trim_end(&q[..b]).len(),
        };
        (self.text[cut.start..cut.end].to_owned(), cut)
    }

    /// The completed selector of a quote-only anchor at an exact hit in
    /// unit `u` (section 6.7): `within` whenever the unit is in a
    /// container, with `count` when it names two or more containers.
    fn quote_selector(&self, u: usize, span: Span) -> Selector {
        let (quote, span) = self.stored_quote(span);
        let within = self.within_of(u);
        let count = within.as_deref().map_or(0, |w| self.names_count(w));
        Selector {
            quote: Some(quote),
            prefix: Some(self.prefix_window(u, span)),
            suffix: Some(self.suffix_window(u, span)),
            within,
            count: (count >= 2).then_some(count as u64),
            ..Selector::default()
        }
    }

    /// The completed selector of a symbol anchor resolved to declaration
    /// `u`, at `hit` when its quote was found in it.
    fn symbol_selector(&self, sel: &Selector, u: usize, hit: Option<Span>) -> Selector {
        let symbol = sel.symbol.clone().expect("a symbol anchor");
        let count = self.symbols.get(symbol.as_str()).map_or(0, Vec::len);
        let windows = hit.filter(|_| sel.quote.is_some());
        Selector {
            quote: sel.quote.clone(),
            prefix: windows.map(|h| self.prefix_window(u, h)),
            suffix: windows.map(|h| self.suffix_window(u, h)),
            symbol: Some(symbol),
            count: (count >= 2).then_some(count as u64),
            ..Selector::default()
        }
    }

    /// The printed anchors of every unit (section 6.7), computed once when
    /// the document is built, each with whether it selects its unit alone:
    /// `{}` for `file`, `{symbol}` for a declaration whose symbol is unique,
    /// `{symbol, count, quote}` for one of several declarations, and a
    /// quote-only selector for every other unit. A unit that no anchor of
    /// these forms selects alone keeps its first-line anchor and is marked
    /// `refuse: quote_not_distinguishable`.
    fn printed(&self) -> Result<Vec<(Selector, bool)>, ProviderError> {
        let mut out = vec![(Selector::empty(), true); self.entries.len()];
        let mut groups: Vec<&Vec<usize>> = self.symbols.values().collect();
        groups.sort_by_key(|g| g[0]);
        for group in groups {
            if group.len() == 1 {
                let i = group[0];
                let sel = Selector {
                    symbol: self.entries[i].symbol.clone(),
                    ..Selector::default()
                };
                out[i] = (sel, true);
            } else {
                for (i, sel, alone) in self.copy_anchors(group) {
                    out[i] = (sel, alone);
                }
            }
        }
        let others: Vec<usize> = (1..self.entries.len())
            .filter(|&i| self.entries[i].symbol.is_none())
            .collect();
        for (i, sel, alone) in self.quote_anchors(&others)? {
            out[i] = (sel, alone);
        }
        Ok(out)
    }

    /// The trimmed, non-empty lines of unit `i`, as spans.
    fn lines_of(&self, i: usize) -> Vec<Span> {
        let e = &self.entries[i];
        let mut out = Vec::new();
        let mut offset = e.start;
        for line in self.text[e.start..e.end].split('\n') {
            let lead = line.len() - text::trim_start(line).len();
            let body = text::trim(line);
            if !body.is_empty() {
                out.push(Span {
                    start: offset + lead,
                    end: offset + lead + body.len(),
                });
            }
            offset += line.len() + 1;
        }
        out
    }

    /// The runs of consecutive lines of unit `i` that a printed anchor
    /// tries after its first choices: for each line in order, the longest
    /// run that starts there and whose normalized text has at most
    /// `MAX_QUOTE_CHARS` characters (the line alone when it is longer).
    fn blocks_of(&self, i: usize) -> Vec<Span> {
        let lines = self.lines_of(i);
        let sizes: Vec<usize> = lines
            .iter()
            .map(|l| text::char_len(&text::normalize(&self.text[l.start..l.end])))
            .collect();
        let mut out = Vec::with_capacity(lines.len());
        for a in 0..lines.len() {
            let mut b = a;
            let mut total = sizes[a];
            while b + 1 < lines.len() && total + 1 + sizes[b + 1] <= MAX_QUOTE_CHARS {
                b += 1;
                total += 1 + sizes[b];
            }
            out.push(Span {
                start: lines[a].start,
                end: lines[b].end,
            });
        }
        out
    }

    /// The printed anchor of one of `n` declarations of a symbol with its
    /// quote at `span` in declaration `i`.
    fn copy_selector(&self, i: usize, span: Span, n: usize) -> Selector {
        let (quote, span) = self.stored_quote(span);
        Selector {
            quote: Some(quote),
            prefix: Some(self.prefix_window(i, span)),
            suffix: Some(self.suffix_window(i, span)),
            symbol: self.entries[i].symbol.clone(),
            count: Some(n as u64),
            ..Selector::default()
        }
    }

    /// The printed anchors of several declarations of one symbol, each with
    /// `count` and with whether it selects its declaration alone. The quote
    /// is the first line of the declaration whose normalized text occurs in
    /// no other declaration's normalized scope text (the first line when
    /// there is none). When that anchor does not select the declaration
    /// alone, the whole declaration text is the quote; then each run of
    /// [`Doc::blocks_of`] in turn. Declarations whose normalized scope
    /// texts are equal are identical twins: no quote selects one alone.
    ///
    /// Which other declarations hold a quote and its windows is looked up
    /// in an index of their 8-byte windows, and the stored-selector check
    /// (A2) runs only on those: for many declarations that differ in a
    /// line, the work is linear in their text, not in its square. A line
    /// shorter than 8 bytes is looked for in every other declaration.
    fn copy_anchors(&self, group: &[usize]) -> Vec<(usize, Selector, bool)> {
        let n = group.len();
        let scopes: Vec<String> = group
            .iter()
            .map(|&i| {
                let (from, to) = self.symbol_scope(i);
                text::normalize(&self.text[from..to])
            })
            .collect();
        let mut same: HashMap<&str, usize> = HashMap::new();
        for s in &scopes {
            *same.entry(s.as_str()).or_default() += 1;
        }
        let index = GramIndex::new(&scopes);
        // How many declarations have each normalized line as a line: a line
        // two have cannot be the line no other has, so the index is asked
        // only about the others.
        let lines: Vec<Vec<(Span, String)>> = group
            .iter()
            .map(|&i| {
                self.lines_of(i)
                    .into_iter()
                    .map(|l| (l, text::normalize(&self.text[l.start..l.end])))
                    .collect()
            })
            .collect();
        let mut line_count: HashMap<&str, usize> = HashMap::new();
        for ls in &lines {
            let mut seen: HashSet<&str> = HashSet::new();
            for (_, l) in ls {
                if seen.insert(l.as_str()) {
                    *line_count.entry(l.as_str()).or_default() += 1;
                }
            }
        }
        // A2 runs only on the declarations whose scope holds the quote and
        // the stored windows: no other one can reproduce them.
        let selects = |k: usize, sel: &Selector| -> bool {
            let q = text::normalize(sel.quote.as_deref().unwrap_or_default());
            let parts: Vec<&str> = [
                q.as_str(),
                sel.prefix.as_deref().unwrap_or_default(),
                sel.suffix.as_deref().unwrap_or_default(),
            ]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect();
            let mut among: Vec<usize> = index
                .holding(&parts, k)
                .into_iter()
                .map(|j| group[j])
                .collect();
            among.push(group[k]);
            matches!(self.a2_reproducing(sel, &among).as_slice(), [(u, _)] if *u == group[k])
        };
        let mut out = Vec::with_capacity(n);
        for (k, &i) in group.iter().enumerate() {
            count_work(1);
            let first = lines[k]
                .first()
                .map(|(s, _)| *s)
                .expect("a declaration has a line");
            if same[scopes[k].as_str()] > 1 {
                out.push((i, self.copy_selector(i, first, n), false));
                continue;
            }
            // A line in a comment is no quote: a stored symbol anchor does
            // not see hits in comments (A2).
            let unique = lines[k].iter().find(|(s, l)| {
                line_count[l.as_str()] == 1 && !self.in_comment(*s) && !index.held_elsewhere(l, k)
            });
            let e = &self.entries[i];
            let choice = unique.map_or(first, |(s, _)| *s);
            let whole = Span {
                start: e.start,
                end: e.end,
            };
            let mut tried: HashSet<(usize, usize)> = HashSet::new();
            let found = [choice, whole]
                .into_iter()
                .chain(std::iter::once(i).flat_map(|i| self.blocks_of(i)))
                .filter(|s| tried.insert((s.start, s.end)))
                .map(|s| self.copy_selector(i, s, n))
                .find(|sel| selects(k, sel));
            match found {
                Some(sel) => out.push((i, sel, true)),
                None => out.push((i, self.copy_selector(i, choice, n), false)),
            }
        }
        out
    }

    /// Per entry: the hit of a quote at the entry's start resolves to it,
    /// because no other unit that starts there is smaller (section 6.7,
    /// innermost unit). Entries that start at one byte are next to each
    /// other, the largest first.
    fn innermost_at_start(&self) -> Vec<bool> {
        let mut out = vec![false; self.entries.len()];
        let mut a = 1;
        while a < self.entries.len() {
            let mut b = a;
            while b + 1 < self.entries.len() && self.entries[b + 1].start == self.entries[a].start {
                b += 1;
            }
            let min = (a..=b).map(|x| self.entries[x].end).min().expect("a run");
            let first = (a..=b)
                .find(|&x| self.entries[x].end == min)
                .expect("a run");
            out[first] = true;
            a = b + 1;
        }
        out
    }

    /// The printed anchors of units without a symbol, each with whether it
    /// selects its unit alone: the first line as the quote when that
    /// selects the unit alone, else the whole unit text, else each run of
    /// [`Doc::blocks_of`] in turn. A unit whose normalized text another
    /// unit of its scope has is an identical twin: nothing selects it alone
    /// there. A unit of a multi-unit piece below a heading is tried with
    /// its own `within` and then with its section's alone.
    ///
    /// A first-line quote starts at its unit's start, so its stored prefix
    /// is empty and only a hit at the start of a unit can reproduce it: the
    /// units of the scope are sorted by the start of their normalized text
    /// and searched by prefix, and whole-unit quotes are counted by
    /// hashing, so these checks are one sort per scope, not a search of the
    /// text per unit. Only a unit these checks do not settle is resolved.
    fn quote_anchors(
        &self,
        units: &[usize],
    ) -> Result<Vec<(usize, Selector, bool)>, ProviderError> {
        let at_start = self.innermost_at_start();
        // The first-line selector of each unit and its scope.
        let mut firsts: Vec<(usize, Selector, ScopeKey)> = Vec::with_capacity(units.len());
        for &i in units {
            let e = &self.entries[i];
            let span = Span {
                start: e.start,
                end: self.default_quote_end(i),
            };
            if text::trim(&self.text[span.start..span.end]).is_empty() {
                return Err(ProviderError::new(
                    ProviderErrorCode::Internal,
                    format!("unit {:?} has an empty first line", e.unit.key),
                ));
            }
            let sel = self.quote_selector(i, span);
            let key = scope_key(sel.within.as_deref());
            firsts.push((i, sel, key));
        }
        // Per scope: the normalized starts of the units a hit at their
        // start resolves to, sorted, equal starts counted once; and their
        // normalized texts, counted. Each unit is added to the scopes it
        // belongs to in one pass.
        let needed: HashSet<ScopeKey> = firsts
            .iter()
            .map(|f| f.2.clone())
            .chain(
                firsts
                    .iter()
                    .filter_map(|f| self.section_within(f.0))
                    .map(|w| scope_key(Some(&w))),
            )
            .collect();
        let mut starts: HashMap<ScopeKey, Vec<String>> = HashMap::new();
        let mut whole: HashMap<ScopeKey, HashMap<String, usize>> = HashMap::new();
        for v in 1..self.entries.len() {
            if !at_start[v] {
                continue;
            }
            let e = &self.entries[v];
            let start = OnceCell::new();
            let full = OnceCell::new();
            for key in self.scope_keys(v) {
                if needed.contains(&key) {
                    count_work(1);
                    let s: &String = start.get_or_init(|| {
                        first_normalized_of(self.code_chars(e.start, e.end), START_CHARS)
                    });
                    starts.entry(key.clone()).or_default().push(s.clone());
                    let t: &String =
                        full.get_or_init(|| text::normalize(&self.text[e.start..e.end]));
                    *whole.entry(key).or_default().entry(t.clone()).or_default() += 1;
                }
            }
        }
        let groups: HashMap<ScopeKey, Vec<(String, usize)>> = starts
            .into_iter()
            .map(|(key, mut list)| {
                list.sort();
                let mut grouped: Vec<(String, usize)> = Vec::new();
                for s in list {
                    match grouped.last_mut() {
                        Some((last, n)) if *last == s => *n += 1,
                        _ => grouped.push((s, 1)),
                    }
                }
                (key, grouped)
            })
            .collect();
        let mut out = Vec::with_capacity(firsts.len());
        for (i, sel, key) in firsts {
            let e = &self.entries[i];
            let unit_text = &self.text[e.start..e.end];
            let whole_span = Span {
                start: e.start,
                end: e.end,
            };
            let cut = text::normalize(unit_text)
                .chars()
                .nth(MAX_QUOTE_CHARS)
                .is_some();
            // The unit's own reading, then, for a unit of a multi-unit
            // piece, the reading by its section alone.
            let mut readings = vec![(sel.clone(), key)];
            if let Some(w) = self.section_within(i) {
                let alt = self.with_within(sel.clone(), w);
                let alt_key = scope_key(alt.within.as_deref());
                readings.push((alt, alt_key));
            }
            let mut unsettled: Vec<Selector> = Vec::new();
            let mut found = None;
            for (first, key) in &readings {
                let full = self.with_within(
                    self.quote_selector(i, whole_span),
                    first.within.clone().unwrap_or_default(),
                );
                match self.by_counting(i, first, &full, key, cut, &at_start[..], &groups, &whole) {
                    Counted::Selects(s) => {
                        found = Some(s);
                        break;
                    }
                    Counted::Twin => {}
                    Counted::Unsettled => unsettled.push(first.clone()),
                }
            }
            // Not settled by counting: resolve, the first line, the whole
            // text, then each run of lines, in each reading.
            if found.is_none() {
                found = unsettled.iter().find_map(|first| {
                    let w = first.within.clone().unwrap_or_default();
                    std::iter::once(first.clone())
                        .chain(
                            std::iter::once(whole_span)
                                .chain(self.blocks_of(i))
                                .map(|b| self.with_within(self.quote_selector(i, b), w.clone())),
                        )
                        .find(|s| self.selects(s, i))
                });
            }
            match found {
                Some(s) => out.push((i, s, true)),
                None => out.push((i, sel, false)),
            }
        }
        Ok(out)
    }

    /// For a unit of a multi-unit piece below a heading: its heading path,
    /// the `within` that names its section without the piece.
    fn section_within(&self, i: usize) -> Option<Vec<String>> {
        prose_class(self.class)?;
        let p = &self.prose()[i];
        (p.multi && !p.path.is_empty()).then(|| p.path.clone())
    }

    /// `sel` with `within` set to `w` (none when `w` is empty) and `count`
    /// to the number of containers `w` names when that is two or more.
    fn with_within(&self, mut sel: Selector, w: Vec<String>) -> Selector {
        if w.is_empty() {
            sel.within = None;
            sel.count = None;
            return sel;
        }
        let n = self.names_count(&w);
        sel.count = (n >= 2).then_some(n as u64);
        sel.within = Some(w);
        sel
    }

    /// What the counts of a scope say about a unit's first-line selector
    /// `first` and whole-text selector `full` (section 6.7, printed
    /// anchors), without resolving either.
    #[allow(clippy::too_many_arguments)]
    fn by_counting(
        &self,
        i: usize,
        first: &Selector,
        full: &Selector,
        key: &ScopeKey,
        cut: bool,
        at_start: &[bool],
        groups: &HashMap<ScopeKey, Vec<(String, usize)>>,
        whole: &HashMap<ScopeKey, HashMap<String, usize>>,
    ) -> Counted {
        if !at_start[i] {
            return Counted::Unsettled;
        }
        let e = &self.entries[i];
        let twins = whole
            .get(key)
            .and_then(|m| m.get(&text::normalize(&self.text[e.start..e.end])))
            .copied()
            .unwrap_or(0);
        if twins >= 2 {
            return Counted::Twin;
        }
        let nq = text::normalize(first.quote.as_deref().unwrap_or_default());
        let suffix = first.suffix.clone().unwrap_or_default();
        let members = groups.get(key).map_or(&[][..], Vec::as_slice);
        let lo = members.partition_point(|(s, _)| s.as_str() < nq.as_str());
        let mut same = 0;
        for (s, n) in &members[lo..] {
            if !s.starts_with(nq.as_str()) {
                break;
            }
            count_work(1);
            let rest = text::trim_start(&s[nq.len()..]);
            let rest: String = rest.chars().take(WINDOW_CHARS).collect();
            if text::trim(&rest) == suffix {
                same += n;
            }
        }
        if same == 1 {
            return Counted::Selects(first.clone());
        }
        // The whole unit text: a quote with empty windows, so only a unit
        // with the same normalized text reproduces it; a unit holding a
        // comment is resolved, since a stored quote does not see a hit
        // that touches a comment.
        if !cut && twins == 1 && !self.has_comment_in(e.start, e.end) {
            return Counted::Selects(full.clone());
        }
        Counted::Unsettled
    }

    /// The scopes unit `v` belongs to: the whole resource, and each union
    /// of containers that a `within` list naming one of its containers
    /// names.
    fn scope_keys(&self, v: usize) -> Vec<ScopeKey> {
        let mut keys = vec![ScopeKey::Whole];
        if prose_class(self.class).is_some() {
            let p = &self.prose()[v];
            if !p.path.is_empty() {
                keys.push(scope_key(Some(&p.path)));
            }
            if p.multi {
                let mut w = p.path.clone();
                w.push(p.name.clone());
                keys.push(scope_key(Some(&w)));
            }
        } else {
            let mut a = Some(v);
            while let Some(x) = a {
                if let Some(s) = &self.entries[x].symbol {
                    keys.push(scope_key(Some(std::slice::from_ref(s))));
                }
                a = self.parents[x];
            }
        }
        keys
    }

    /// The selector resolves, as a stored selector, to unit `i` alone.
    fn selects(&self, sel: &Selector, i: usize) -> bool {
        matches!(self.resolve_stored(sel).as_slice(), [r] if r.unit == i && r.how == How::Resolved)
    }

    // -- resolution -----------------------------------------------------------

    /// `resolve(resource, sel)` of section 6.7. The selector has no `key`.
    pub(super) fn resolve(&self, sel: &Selector) -> Result<Vec<Unit>, ProviderError> {
        if sel.quote.is_none() && sel.symbol.is_none() {
            return Ok(vec![self.out(&Found::resolved(0), Selector::empty())?]);
        }
        let stored = sel.prefix.is_some()
            || sel.suffix.is_some()
            || sel.within.is_some()
            || sel.count.is_some();
        let found = if stored {
            self.resolve_stored(sel)
        } else {
            self.resolve_new(sel)
        };
        let alone = matches!(found.as_slice(), [f] if f.how != How::Candidate);
        let mut units = Vec::with_capacity(found.len());
        for f in &found {
            let u = if alone {
                self.out(f, self.completed(sel, f, stored))?
            } else {
                // A listed unit carries its printed anchor, and is marked
                // when no anchor selects it alone.
                let e = &self.entries[f.unit].unit;
                let mut u = self.out(f, e.sel.clone())?;
                u.refuse = e.refuse;
                u
            };
            units.push(u);
        }
        // The completed selector of a unit returned alone must resolve back
        // to that unit alone (section 6.7). A new anchor whose completed
        // selector does not is refused: the anchored text cannot be told
        // apart from another place in its scope. For a stored selector the
        // stored selector itself is returned: it resolved to the unit alone,
        // and a refresh must not replace it by one that does not.
        if let [u] = units.as_mut_slice()
            && alone
            && u.refuse.is_none()
            && !u.approx
            && u.identity.is_none()
            && !self.selects(&u.sel, found[0].unit)
        {
            if stored {
                u.sel = sel.clone();
            } else {
                u.refuse = Some(Refuse::QuoteNotDistinguishable);
            }
        }
        Ok(units)
    }

    /// The selector to store for a unit returned alone. A stored quote-only
    /// selector keeps its `within` and `count`: a refresh rewrites the quote
    /// and windows only, so it never widens the scope (section 6.7).
    fn completed(&self, sel: &Selector, f: &Found, stored: bool) -> Selector {
        if f.unit == 0 && sel.quote.is_none() {
            return Selector::empty();
        }
        if sel.symbol.is_some() {
            return self.symbol_selector(sel, f.unit, f.span);
        }
        match f.span {
            Some(span) if f.how == How::Resolved && self.within(span, f.unit) => {
                let mut c = self.quote_selector(f.unit, span);
                if stored {
                    c.within = sel.within.clone();
                    c.count = sel.count;
                }
                c
            }
            _ => self.entries[f.unit].unit.sel.clone(),
        }
    }

    /// The unit object for a result.
    fn out(&self, f: &Found, sel: Selector) -> Result<Unit, ProviderError> {
        let e = &self.entries[f.unit];
        let mut u = e.unit.clone();
        u.sel = sel;
        u.text = Some(self.text[e.start..e.end].to_owned());
        u.approx = f.how == How::Candidate;
        u.identity = (f.how == How::BySymbol).then_some(Identity::Symbol);
        u.refuse = f.refuse;
        Ok(u)
    }

    /// The declarations of `symbol`, in `units` order.
    fn declarations(&self, symbol: &str) -> &[usize] {
        self.symbols.get(symbol).map_or(&[], Vec::as_slice)
    }

    /// The hits of `quote` within `scope(u)`: the quote is in the
    /// declaration, also where it runs over several of its members.
    fn hits_in_scope(&self, quote: &str, u: usize) -> Vec<Hit> {
        let (from, to) = self.symbol_scope(u);
        self.hits_in(quote, from, to)
    }

    /// The hits of a stored symbol anchor's quote in `scope(u)` (A1 and
    /// A2): a hit inside a comment is not a hit, so a quote that moved into
    /// a comment counts as gone from the declaration.
    fn stored_hits(&self, quote: &str, u: usize) -> Vec<Hit> {
        self.hits_in_scope(quote, u)
            .into_iter()
            .filter(|h| !self.in_comment(h.span))
            .collect()
    }

    /// A2 on the declarations `among`: each one whose scope holds a hit of
    /// the quote (outside comments) that reproduces the stored windows,
    /// with that hit.
    fn a2_reproducing(&self, sel: &Selector, among: &[usize]) -> Vec<(usize, Span)> {
        let q = sel.quote.as_deref().unwrap_or_default();
        among
            .iter()
            .filter_map(|&u| {
                self.stored_hits(q, u)
                    .into_iter()
                    .find(|h| self.reproduces(sel, u, h.span))
                    .map(|h| (u, h.span))
            })
            .collect()
    }

    /// `resolve_stored` of section 6.7.
    fn resolve_stored(&self, sel: &Selector) -> Vec<Found> {
        if let Some(symbol) = &sel.symbol {
            let s = self.declarations(symbol);
            let c = sel
                .count
                .map_or(1, |c| usize::try_from(c).unwrap_or(usize::MAX));
            if s.len() != c {
                // A3: the number of declarations changed.
                let mut out: Vec<Found> = s.iter().map(|&u| Found::candidate(u)).collect();
                if s.is_empty()
                    && let Some(q) = &sel.quote
                {
                    out.extend(self.hit_units(&self.hits(q)));
                }
                return dedup(out);
            }
            if c == 1 {
                // A1.
                let u = s[0];
                let Some(q) = &sel.quote else {
                    return vec![Found::resolved(u)];
                };
                // The hit that reproduces the stored windows, else the
                // first, so the completed selector is the one the anchor
                // was made with.
                let hits = self.stored_hits(q, u);
                if let Some(h) = hits
                    .iter()
                    .find(|h| self.reproduces(sel, u, h.span))
                    .or(hits.first())
                {
                    return vec![Found::resolved(u).at(h.span)];
                }
                let elsewhere: Vec<Hit> = self
                    .hits(q)
                    .into_iter()
                    .filter(|h| h.exact && !self.in_comment(h.span))
                    .collect();
                if !elsewhere.is_empty() {
                    let mut out = vec![Found::candidate(u)];
                    out.extend(self.hit_units(&elsewhere));
                    return dedup(out);
                }
                return vec![Found {
                    how: How::BySymbol,
                    ..Found::resolved(u)
                }];
            }
            // A2.
            let Some(q) = &sel.quote else {
                return s.iter().map(|&u| Found::candidate(u)).collect();
            };
            let mut d = Vec::new();
            let mut r = Vec::new();
            for &u in s {
                let hits = self.stored_hits(q, u);
                if hits.is_empty() {
                    continue;
                }
                d.push(u);
                if let Some(h) = hits.iter().find(|h| self.reproduces(sel, u, h.span)) {
                    r.push(Found::resolved(u).at(h.span));
                }
            }
            if r.len() == 1 {
                return r;
            }
            let pool = if d.is_empty() { s } else { d.as_slice() };
            return pool.iter().map(|&u| Found::candidate(u)).collect();
        }
        // P.
        let q = sel.quote.as_deref().unwrap_or_default();
        let scope = match &sel.within {
            Some(w) => match self.scope_of(w, sel.count) {
                Some(scope) => Some(scope),
                None => return self.candidates(q, None),
            },
            None => None,
        };
        let hits = self.hits(q);
        let in_scope = |h: &Hit| scope.as_ref().is_none_or(|c| self.in_scope(c, h.unit));
        // As in A1 and A2, a hit inside a comment (wholly or partly) is not
        // a hit: a stored quote that moved into a comment is gone, so it
        // never resolves a side.
        let mut r: Vec<Found> = Vec::new();
        for h in hits
            .iter()
            .filter(|h| h.exact && in_scope(h) && !self.in_comment(h.span))
        {
            if self.reproduces(sel, h.unit, h.span) && !r.iter().any(|f| f.unit == h.unit) {
                r.push(Found::resolved(h.unit).at(h.span));
            }
        }
        match r.len() {
            1 => r,
            0 => self.candidates(q, scope.as_ref()),
            _ => r
                .into_iter()
                .map(|f| Found {
                    how: How::Ambiguous,
                    ..f
                })
                .collect(),
        }
    }

    /// `candidates(scope)`: the units of the exact hits in scope, then of
    /// the hits that cross units, then the similarity guess, each once, all
    /// approximate.
    fn candidates(&self, quote: &str, scope: Option<&Scope>) -> Vec<Found> {
        let hits = self.hits(quote);
        let in_scope = |h: &Hit| scope.is_none_or(|c| self.in_scope(c, h.unit));
        let exact: Vec<Hit> = hits
            .iter()
            .filter(|h| h.exact && in_scope(h))
            .cloned()
            .collect();
        let crossing: Vec<Hit> = hits
            .iter()
            .filter(|h| !h.exact && in_scope(h))
            .cloned()
            .collect();
        let mut out = self.hit_units(&exact);
        out.extend(self.hit_units(&crossing));
        if let Some(u) = self.guess(quote, scope) {
            out.push(Found::candidate(u));
        }
        dedup(out)
    }

    /// The distinct units of `hits`, in their order, as candidates.
    fn hit_units(&self, hits: &[Hit]) -> Vec<Found> {
        dedup(
            hits.iter()
                .map(|h| Found::candidate(h.unit).at(h.span))
                .collect(),
        )
    }

    /// The similarity guess in a scope: the unit of the fuzzy step of
    /// section 7.2 on the scope's text, within the budget of section 7.1.
    /// In a scope of several containers the step runs on each container's
    /// text, and the result nearest the quote (by edit distance, the
    /// earlier container on a tie) is the guess. A result whose unit is
    /// outside the scope is no guess.
    fn guess(&self, quote: &str, scope: Option<&Scope>) -> Option<usize> {
        let Some(scope) = scope else {
            return self.guess_in(quote, 0, self.text.len()).map(|(u, _)| u);
        };
        // The text of a section or piece container: from its first unit's
        // start to its last unit's end.
        let mut hull: HashMap<Container, (usize, usize)> = HashMap::new();
        if scope.list.iter().any(|c| !matches!(c, Container::Decl(_))) {
            for u in 1..self.entries.len() {
                for c in self.containers_of(u) {
                    if scope.set.contains(&c) {
                        let e = &self.entries[u];
                        let h = hull.entry(c).or_insert((e.start, e.end));
                        h.1 = h.1.max(e.end);
                    }
                }
            }
        }
        let mut best: Option<(usize, usize)> = None;
        for c in &scope.list {
            let (from, to) = match c {
                Container::Decl(d) => self.symbol_scope(*d),
                c => match hull.get(c) {
                    Some(&h) => h,
                    None => continue,
                },
            };
            if let Some((u, d)) = self.guess_in(quote, from, to)
                && self.in_scope(scope, u)
                && best.is_none_or(|(_, b)| d < b)
            {
                best = Some((u, d));
            }
        }
        best.map(|(u, _)| u)
    }

    /// The fuzzy step on `text[from..to]`: the unit of its result and the
    /// result's edit distance to the quote, when within the budget.
    fn guess_in(&self, quote: &str, from: usize, to: usize) -> Option<(usize, usize)> {
        let span = locate::fuzzy(&self.text[from..to], quote, "", "", 0)?;
        let span = Span {
            start: from + span.start,
            end: from + span.end.min(to - from),
        };
        if span.start >= span.end {
            return None;
        }
        let a: Vec<char> = text::normalize(&self.text[span.start..span.end])
            .chars()
            .collect();
        let b: Vec<char> = text::normalize(quote).chars().collect();
        let d = if a == b {
            0
        } else {
            text::levenshtein_at_most(&a, &b, locate::error_budget(b.len()))?
        };
        Some((self.innermost(span), d))
    }

    /// `resolve_new` of section 6.7: what the agent typed.
    fn resolve_new(&self, sel: &Selector) -> Vec<Found> {
        if let Some(symbol) = &sel.symbol {
            let s = self.declarations(symbol);
            match s.len() {
                0 => {
                    let q = sel.quote.as_deref().unwrap_or_default();
                    return if q.is_empty() {
                        Vec::new()
                    } else {
                        self.candidates(q, None)
                    };
                }
                1 => {
                    let u = s[0];
                    let Some(q) = &sel.quote else {
                        return vec![Found::resolved(u)];
                    };
                    let hits = self.hits_in_scope(q, u);
                    return match self.first_outside_comments(&hits) {
                        Some(h) => {
                            let mut f = Found::resolved(u).at(h.span);
                            f.refuse = self.in_comment(h.span).then_some(Refuse::QuoteInComment);
                            vec![f]
                        }
                        None => vec![Found {
                            how: How::BySymbol,
                            ..Found::resolved(u)
                        }],
                    };
                }
                _ => {}
            }
            let Some(q) = &sel.quote else {
                return s.iter().map(|&u| Found::ambiguous(u)).collect();
            };
            let mut d: Vec<(usize, Hit)> = Vec::new();
            for &u in s {
                let hits = self.hits_in_scope(q, u);
                if let Some(h) = self.first_outside_comments(&hits) {
                    d.push((u, h));
                }
            }
            if let [(u, h)] = d.as_slice() {
                let mut f = Found::resolved(*u).at(h.span);
                f.refuse = self.in_comment(h.span).then_some(Refuse::QuoteInComment);
                return vec![f];
            }
            let pool: Vec<usize> = if d.is_empty() {
                s.to_vec()
            } else {
                d.iter().map(|(u, _)| *u).collect()
            };
            return pool.into_iter().map(Found::ambiguous).collect();
        }
        let q = sel.quote.as_deref().unwrap_or_default();
        let hits = self.hits(q);
        if hits.is_empty() {
            return self
                .guess(q, None)
                .map(Found::candidate)
                .into_iter()
                .collect();
        }
        let mut units: Vec<usize> = Vec::new();
        for h in &hits {
            if !units.contains(&h.unit) {
                units.push(h.unit);
            }
        }
        if units.len() > 1 {
            return units.into_iter().map(Found::ambiguous).collect();
        }
        let h = self
            .first_outside_comments(&hits)
            .expect("hits is not empty");
        let mut f = Found::resolved(h.unit).at(h.span);
        f.refuse = if self.in_comment(h.span) {
            Some(Refuse::QuoteInComment)
        } else if !h.exact {
            Some(Refuse::QuoteSpansUnits)
        } else {
            None
        };
        vec![f]
    }

    /// The first hit that is not in a comment, or the first hit.
    fn first_outside_comments(&self, hits: &[Hit]) -> Option<Hit> {
        hits.iter()
            .find(|h| !self.in_comment(h.span))
            .or(hits.first())
            .cloned()
    }
}

/// How a unit of an answer was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum How {
    /// The unit the selector was made for.
    Resolved,
    /// Resolved by its symbol alone; the stored quote is gone (A1).
    BySymbol,
    /// One of several units the selector selects equally.
    Ambiguous,
    /// A candidate, `approx: true`.
    Candidate,
}

/// One unit of an answer, with the hit that found it when there is one.
#[derive(Debug, Clone, Copy)]
struct Found {
    unit: usize,
    how: How,
    span: Option<Span>,
    refuse: Option<Refuse>,
}

impl Found {
    fn resolved(unit: usize) -> Self {
        Found {
            unit,
            how: How::Resolved,
            span: None,
            refuse: None,
        }
    }

    fn candidate(unit: usize) -> Self {
        Found {
            how: How::Candidate,
            ..Found::resolved(unit)
        }
    }

    fn ambiguous(unit: usize) -> Self {
        Found {
            how: How::Ambiguous,
            ..Found::resolved(unit)
        }
    }

    fn at(self, span: Span) -> Self {
        Found {
            span: Some(span),
            ..self
        }
    }
}

/// The units of `found` with each unit kept once, at its first place.
fn dedup(found: Vec<Found>) -> Vec<Found> {
    let mut seen: HashSet<usize> = HashSet::new();
    found.into_iter().filter(|f| seen.insert(f.unit)).collect()
}

/// A hit of a quote (section 6.7): its span, its unit and whether it lies
/// inside that unit.
#[derive(Debug, Clone, Copy)]
struct Hit {
    span: Span,
    unit: usize,
    exact: bool,
}

/// A container of section 6.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Container {
    /// The units of one section: directly below one heading (its entry
    /// index), or above the first heading (0).
    Section(usize),
    /// The units of one multi-unit piece, by its number.
    Piece(usize),
    /// A declaration unit and the units within its range.
    Decl(usize),
}

/// The key of the scope a printed anchor with this `within` is resolved
/// in.
fn scope_key(within: Option<&[String]>) -> ScopeKey {
    match within {
        Some(w) => ScopeKey::Within(w.iter().map(|x| text::normalize(x)).collect()),
        None => ScopeKey::Whole,
    }
}

/// Bytes in a window of [`GramIndex`].
const GRAM: usize = 8;

/// Which of several texts hold some strings: each 8-byte window of the
/// texts, with the texts that have it. Strings are looked for only in the
/// texts that have the rarest window of any of them; when none is 8 bytes
/// long, in every text.
struct GramIndex<'a> {
    texts: &'a [String],
    grams: HashMap<[u8; GRAM], Vec<u32>>,
}

impl<'a> GramIndex<'a> {
    fn new(texts: &'a [String]) -> Self {
        let mut grams: HashMap<[u8; GRAM], Vec<u32>> = HashMap::new();
        for (t, s) in texts.iter().enumerate() {
            let t = t as u32;
            for w in s.as_bytes().windows(GRAM) {
                count_work(1);
                let key: [u8; GRAM] = w.try_into().expect("a window of GRAM bytes");
                let list = grams.entry(key).or_default();
                if list.last() != Some(&t) {
                    list.push(t);
                }
            }
        }
        GramIndex { texts, grams }
    }

    /// The texts that may hold every string of `parts`: those with the
    /// rarest window of any part, or every text.
    fn pool(&self, parts: &[&str]) -> Vec<usize> {
        let mut best: Option<&[u32]> = None;
        for part in parts {
            for w in part.as_bytes().windows(GRAM) {
                count_work(1);
                let key: [u8; GRAM] = w.try_into().expect("a window of GRAM bytes");
                let list = self.grams.get(&key).map_or(&[][..], Vec::as_slice);
                if best.is_none_or(|b| list.len() < b.len()) {
                    best = Some(list);
                }
            }
        }
        match best {
            Some(list) => list.iter().map(|&t| t as usize).collect(),
            None => (0..self.texts.len()).collect(),
        }
    }

    /// The texts other than `k` that hold every string of `parts`.
    fn holding(&self, parts: &[&str], k: usize) -> Vec<usize> {
        self.pool(parts)
            .into_iter()
            .filter(|&t| {
                count_work(1);
                t != k && parts.iter().all(|p| self.texts[t].contains(p))
            })
            .collect()
    }

    /// A text other than `k` holds `needle`.
    fn held_elsewhere(&self, needle: &str, k: usize) -> bool {
        self.pool(&[needle]).into_iter().any(|t| {
            count_work(1);
            t != k && self.texts[t].contains(needle)
        })
    }
}

/// The scope a stored `within` gives: the containers it names, in order,
/// and as a set.
#[derive(Debug)]
struct Scope {
    list: Vec<Container>,
    set: HashSet<Container>,
}

impl Scope {
    fn of(list: Vec<Container>) -> Self {
        let set = list.iter().copied().collect();
        Scope { list, set }
    }
}

/// A stretch of `text[from..to]` as code sees it.
#[derive(Debug, Clone, Copy)]
enum Seg {
    /// Text between comments, as a byte range.
    Text(usize, usize),
    /// One comment, read as one space.
    Comment,
}

/// What counting says about a printed anchor (section 6.7).
enum Counted {
    /// This selector selects the unit alone.
    Selects(Selector),
    /// Another unit of the scope has the same text: nothing selects it
    /// alone in this scope.
    Twin,
    /// Counting cannot tell: the selectors are resolved.
    Unsettled,
}

/// The scope a printed quote-only selector is resolved in: the whole
/// resource, or the union of the containers a `within` list names (keyed
/// by the list, normalized).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ScopeKey {
    Whole,
    Within(Vec<String>),
}

/// The prose units' information with the lookups of `names`.
#[derive(Debug)]
struct ProseIndex {
    info: Vec<ProseInfo>,
    /// The sections that some unit is in, by heading path, in source order.
    sections: HashMap<Vec<String>, Vec<usize>>,
    pieces: HashMap<(Vec<String>, String), Vec<usize>>,
}

/// What section 6.7 needs about a prose unit.
#[derive(Debug, Clone, Default)]
struct ProseInfo {
    /// The heading texts above the unit, as written (for `render`).
    raw_path: Vec<String>,
    /// Their heading keys.
    path: Vec<String>,
    /// The section the unit is in: the entry index of the heading that
    /// starts it, or 0 above the first heading. Two sections whose heading
    /// paths read the same are two sections.
    section: usize,
    /// The piece the unit was cut from.
    piece: usize,
    /// The piece holds two or more units.
    multi: bool,
    /// The piece's name, when `multi`.
    name: String,
}

/// `normalize(text)` with, for each of its bytes, the byte of the resource
/// text its character came from.
#[derive(Debug)]
struct Norm {
    text: String,
    src: Vec<u32>,
}

impl Norm {
    fn of(text: &str) -> Self {
        let mut out = String::with_capacity(text.len());
        let mut src: Vec<u32> = Vec::with_capacity(text.len());
        let mut pending: Option<usize> = None;
        for (b, c) in text.char_indices() {
            if text::is_ws(c) {
                if !out.is_empty() && pending.is_none() {
                    pending = Some(b);
                }
                continue;
            }
            if let Some(p) = pending.take() {
                out.push(' ');
                src.push(p as u32);
            }
            out.push(c);
            src.extend(std::iter::repeat_n(b as u32, c.len_utf8()));
        }
        Norm { text: out, src }
    }

    /// The span of the resource text that normalized bytes `[from, to)`
    /// came from: from the first matched character to the end of the last.
    fn span(&self, text: &str, from: usize, to: usize) -> Span {
        let start = self.src[from] as usize;
        let last = self.src[to - 1] as usize;
        let len = text[last..].chars().next().map_or(0, char::len_utf8);
        Span {
            start,
            end: last + len,
        }
    }
}

/// Characters of a stored `prefix` or `suffix` window.
const WINDOW_CHARS: usize = 48;

/// Characters of a piece's name.
const PIECE_NAME_CHARS: usize = 120;

/// Characters of a unit's normalized start compared when printed anchors
/// are checked: a first-line quote (at most 120) and its suffix window.
const START_CHARS: usize = DEFAULT_QUOTE_CHARS + 1 + WINDOW_CHARS + 1;

/// The first `n` characters of `normalize(s)`.
fn first_normalized(s: &str, n: usize) -> String {
    first_normalized_of(s.chars(), n)
}

/// The first `n` characters of `normalize` of a character sequence.
fn first_normalized_of(chars: impl Iterator<Item = char>, n: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut pending = false;
    for c in chars {
        if count == n {
            break;
        }
        if text::is_ws(c) {
            pending = count > 0;
            continue;
        }
        if pending {
            out.push(' ');
            count += 1;
            pending = false;
            if count == n {
                break;
            }
        }
        out.push(c);
        count += 1;
    }
    out
}

/// The last `n` characters of `normalize` of a character sequence given
/// last character first.
fn last_normalized_of(rev_chars: impl Iterator<Item = char>, n: usize) -> String {
    let mut rev: Vec<char> = Vec::new();
    let mut pending = false;
    for c in rev_chars {
        if rev.len() == n {
            break;
        }
        if text::is_ws(c) {
            pending = !rev.is_empty();
            continue;
        }
        if pending {
            rev.push(' ');
            pending = false;
            if rev.len() == n {
                break;
            }
        }
        rev.push(c);
    }
    rev.into_iter().rev().collect()
}

/// The heading key of a heading text (section 6.7): links and images
/// replaced by their text, `` ` ``, `*` and `~` removed, `_` removed unless
/// it stands between two letters or digits, normalized.
fn heading_key(h: &str) -> String {
    // Links and images become their text first, so that markup inside a
    // link's text is removed like any other.
    let chars: Vec<char> = h.chars().collect();
    let mut plain: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let image = c == '!' && chars.get(i + 1) == Some(&'[');
        if c == '[' || image {
            let open = if image { i + 1 } else { i };
            if let Some(close) = (open + 1..chars.len()).find(|&k| chars[k] == ']') {
                let target = match chars.get(close + 1) {
                    Some('(') => (close + 2..chars.len()).find(|&k| chars[k] == ')'),
                    Some('[') => (close + 2..chars.len()).find(|&k| chars[k] == ']'),
                    _ => None,
                };
                if let Some(end) = target {
                    plain.extend(&chars[open + 1..close]);
                    i = end + 1;
                    continue;
                }
            }
        }
        plain.push(c);
        i += 1;
    }
    let mut out = String::with_capacity(plain.len());
    let mut i = 0;
    while i < plain.len() {
        let c = plain[i];
        // A run of `_` between two letters or digits is kept: an intraword
        // underscore cannot open or close emphasis (CommonMark 0.31.2,
        // section 6.2, rules 2 and 4), so `max_retries` keeps it.
        if c == '_' {
            let end = (i..plain.len())
                .find(|&k| plain[k] != '_')
                .unwrap_or(plain.len());
            let intraword = i > 0
                && plain[i - 1].is_alphanumeric()
                && plain.get(end).is_some_and(|n| n.is_alphanumeric());
            if intraword {
                out.extend(&plain[i..end]);
            }
            i = end;
            continue;
        }
        if !matches!(c, '`' | '*' | '~') {
            out.push(c);
        }
        i += 1;
    }
    text::normalize(&out)
}

/// The first cell of a table row: the text between its first `|` and the
/// next `|`.
fn first_cell(row: &str) -> &str {
    let Some(open) = row.find('|') else {
        return row;
    };
    let rest = &row[open + 1..];
    &rest[..rest.find('|').unwrap_or(rest.len())]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Section 6.7: heading keys drop markup, and keep an underscore that
    /// stands inside a word (CommonMark: it is not emphasis there).
    #[test]
    fn heading_keys_keep_intraword_underscores() {
        for (heading, key) in [
            ("`max_retries`", "max_retries"),
            ("maxretries", "maxretries"),
            ("max__retries", "max__retries"),
            ("_Emphasis_ here", "Emphasis here"),
            ("**Bold** and `code`", "Bold and code"),
            ("trailing_ space", "trailing space"),
            ("[The `x_y` link](https://e.example)", "The x_y link"),
            ("~~old~~ new", "old new"),
        ] {
            assert_eq!(heading_key(heading), key, "{heading:?}");
        }
    }

    #[test]
    fn label_is_a_prefix_of_normalize() {
        for s in [
            "",
            "  a  b ",
            &"x ".repeat(100),
            &" y".repeat(100),
            "a\n\n\tb",
        ] {
            let full: String = text::normalize(s).chars().take(LABEL_CHARS).collect();
            assert_eq!(label_of(s), full, "{s:?}");
        }
    }

    /// The result of `f` and the lookup work counted while it ran.
    fn with_work<T>(f: impl FnOnce() -> T) -> (T, usize) {
        WORK.with(|w| w.set(0));
        let out = f();
        (out, WORK.with(|w| w.get()))
    }

    /// Doubling the input must at most about double the work: linear is
    /// near 2, quadratic near 4.
    fn assert_linear(small: usize, large: usize, what: &str) {
        assert!(
            large <= 3 * small,
            "{what}: work grew from {small} to {large} when the input doubled"
        );
    }

    /// Limit 7: an ambiguous quote with many tied hits in many units, and
    /// many tied hits in one unit with many comments, resolve in linear
    /// work (innermost and in_comment are lookups, not scans).
    #[test]
    fn many_tied_hits_resolve_in_linear_work() {
        let across = |n: usize| {
            let t = "Same text here.\n\n".repeat(n);
            let doc = Doc::build(t, Class::Markdown).unwrap();
            let (r, work) = with_work(|| doc.resolve(&Selector::quote("Same text")).unwrap());
            assert_eq!(r.len(), n);
            work
        };
        assert_linear(across(2000), across(4000), "ties across units");
        let within = |n: usize| {
            // n hits in comments before the one in code, so `pick` checks
            // every one of them.
            let body = format!("{}    x();\n", "    // x();\n".repeat(n));
            let t = format!("fn f() {{\n{body}}}\n");
            let doc = Doc::build(t, Class::Rust).unwrap();
            let (r, work) = with_work(|| doc.resolve(&Selector::quote("x();")).unwrap());
            assert_eq!(r.len(), 1);
            work
        };
        assert_linear(within(2000), within(4000), "ties within one unit");
    }

    /// The branch for several declarations of one symbol locates the quote
    /// in each declaration's scope once: linear in the declarations and
    /// their hits, for new and stored anchors.
    #[test]
    fn many_declarations_of_one_symbol_resolve_in_linear_work() {
        let run = |n: usize, stored: bool| {
            let t = format!(
                "class P:\n{}",
                "    def v(self):\n        return 0\n\n".repeat(n)
            );
            let doc = Doc::build(t, Class::Python).unwrap();
            let sel = if stored {
                doc.entries[2].unit.sel.clone()
            } else {
                let mut s = Selector::quote("def v(self):");
                s.symbol = Some("P.v".into());
                s
            };
            let (r, work) = with_work(|| doc.resolve(&sel).unwrap());
            // A new anchor gives every copy (ambiguous); the printed anchor
            // of an identical twin cannot tell the copies apart, so every
            // copy is a candidate (section 6.7, A2).
            assert_eq!(r.len(), n);
            assert_eq!(r.iter().all(|u| u.approx), stored);
            work
        };
        for stored in [false, true] {
            assert_linear(
                run(1000, stored),
                run(2000, stored),
                "declarations of one symbol",
            );
        }
    }

    /// The printed anchors of many same-named declarations that differ in
    /// one line cost linear work: which other declarations hold a line is
    /// an index lookup, not a search of every other declaration.
    #[test]
    fn printed_anchors_of_many_copies_cost_linear_work() {
        let build = |n: usize| {
            let t: String = (0..n)
                .map(|i| format!("fn f() {{\n    let k = 2;\n    k + {i}\n}}\n\n"))
                .collect();
            with_work(|| Doc::build(t, Class::Rust).unwrap())
        };
        let (small_doc, small) = build(1000);
        let (_, large) = build(2000);
        assert_linear(small, large, "printed anchors of copies");
        // Each anchor names its declaration alone.
        assert!(small_doc.units(None).iter().all(|u| u.refuse.is_none()));
    }

    /// The printed anchors of near-identical units cost linear work in
    /// every shape where one `within` names many containers or units are
    /// twins: pieces with one name, sections with one path, identical
    /// paragraphs, same-named and same-bodied functions, top-level
    /// statements (a `within` is counted by lookups, not by listing what
    /// it names).
    #[test]
    fn printed_anchors_of_near_identical_units_cost_linear_work() {
        type Shape = (&'static str, Class, fn(usize) -> String);
        let shapes: [Shape; 7] = [
            ("pieces with one name", Class::Markdown, |i| {
                format!(
                    "Same start here. The value is {i} and the rest of this sentence is the same.\n\n"
                )
            }),
            ("sections with one path", Class::Markdown, |i| {
                format!("## Item\n\nText number {i} here.\n\n")
            }),
            ("identical paragraphs", Class::Markdown, |_| {
                "Same start here. Same end here.\n\n".to_owned()
            }),
            ("same-named functions", Class::Typescript, |i| {
                format!("export function f(a: number): number {{\n  return a + {i};\n}}\n\n")
            }),
            ("same-bodied functions", Class::Typescript, |i| {
                format!(
                    "function f{i}(a: number): number {{\n  const k = a * 2;\n  return k;\n}}\n\n"
                )
            }),
            ("top-level statements", Class::Typescript, |i| {
                format!("setup({i});\n")
            }),
            ("identical top-level statements", Class::Typescript, |_| {
                "setup(1);\n".to_owned()
            }),
        ];
        for (what, class, unit) in shapes {
            let build = |n: usize| {
                let head = if class == Class::Markdown {
                    "# Doc\n\n"
                } else {
                    ""
                };
                let t: String = std::iter::once(head.to_owned())
                    .chain((0..n).map(unit))
                    .collect();
                with_work(|| Doc::build(t, class).unwrap()).1
            };
            assert_linear(build(1000), build(2000), what);
        }
    }

    /// Limit 8: default quotes for many units with comments cost linear
    /// work (the comment lookup per unit is a binary search).
    #[test]
    fn default_quotes_cost_linear_work() {
        let run = |n: usize| {
            let t = "function f(){} // c\n".repeat(n);
            let (doc, work) = with_work(|| Doc::build(t, Class::Typescript).unwrap());
            assert_eq!(doc.entries.len(), n + 1);
            work
        };
        assert_linear(run(2000), run(4000), "default quotes");
    }

    /// Limit 9: a resource with more than MAX_UNITS_PER_RESOURCE units is
    /// too_large; exactly the limit is accepted.
    #[test]
    fn unit_count_cap() {
        let at = "S.\n\n".repeat(MAX_UNITS_PER_RESOURCE - 1);
        assert_eq!(
            Doc::build(at, Class::Markdown).unwrap().entries.len(),
            MAX_UNITS_PER_RESOURCE
        );
        let over = "S.\n\n".repeat(MAX_UNITS_PER_RESOURCE);
        let err = Doc::build(over, Class::Markdown).unwrap_err();
        assert_eq!(err.code, ProviderErrorCode::TooLarge);
        assert!(
            err.message.contains("more than 100000 units"),
            "{}",
            err.message
        );
        let blocks = "b\n\n".repeat(MAX_UNITS_PER_RESOURCE);
        assert_eq!(
            Doc::build(blocks, Class::Text).unwrap_err().code,
            ProviderErrorCode::TooLarge
        );
        assert_eq!(MAX_UNITS_PER_RESOURCE, 100_000);
    }

    /// Limit 11: every condition of `from_cache` makes a row a miss.
    #[test]
    fn cache_rows_are_validated_strictly() {
        let t = "// lead\nfn a() { 1 }\nstruct S { x: u8 }\n".to_owned();
        let good = Doc::build(t.clone(), Class::Rust).unwrap().to_cache();
        assert!(Doc::from_cache(t.clone(), Class::Rust, &good).is_some());
        // A unit that no anchor selects alone keeps its mark through the
        // cache.
        let twins = "fn init() {}\nfn init() {}\n".to_owned();
        let row = Doc::build(twins.clone(), Class::Rust).unwrap().to_cache();
        let back = Doc::from_cache(twins, Class::Rust, &row).expect("a valid row");
        let marks: Vec<Option<Refuse>> = back.units(None).iter().map(|u| u.refuse).collect();
        assert_eq!(
            marks,
            [
                None,
                Some(Refuse::QuoteNotDistinguishable),
                Some(Refuse::QuoteNotDistinguishable)
            ]
        );
        let rows: Vec<serde_json::Value> = serde_json::from_str(&good).unwrap();
        let edit = |f: &dyn Fn(&mut Vec<serde_json::Value>)| {
            let mut r = rows.clone();
            f(&mut r);
            serde_json::to_string(&r).unwrap()
        };
        let bad: Vec<(&str, String)> = vec![
            ("not JSON", "[{".to_owned()),
            ("not an array", "{}".to_owned()),
            ("empty array", "[]".to_owned()),
            (
                "an extra key",
                edit(&|r| {
                    r[1]["extra"] = 1.into();
                }),
            ),
            (
                "text, which a row never has",
                edit(&|r| {
                    r[1]["text"] = "x".into();
                }),
            ),
            (
                "a refuse other than quote_not_distinguishable",
                edit(&|r| {
                    r[1]["refuse"] = "quote_in_comment".into();
                }),
            ),
            (
                "a refuse quote_spans_units",
                edit(&|r| {
                    r[1]["refuse"] = "quote_spans_units".into();
                }),
            ),
            (
                "approx, which a row never has",
                edit(&|r| {
                    r[1]["approx"] = true.into();
                }),
            ),
            (
                "a missing key",
                edit(&|r| {
                    r[1].as_object_mut().unwrap().remove("label");
                }),
            ),
            (
                "an invalid fingerprint",
                edit(&|r| {
                    r[1]["fp"] = "b3:ast-v1:zz".into();
                }),
            ),
            (
                "an unknown kind",
                edit(&|r| {
                    r[1]["kind"] = "widget".into();
                }),
            ),
            (
                "a first unit that is not the file",
                edit(&|r| {
                    r.swap(0, 1);
                }),
            ),
            (
                "a file range that is not the text",
                edit(&|r| {
                    r[0]["range"]["end"] = 3.into();
                }),
            ),
            (
                "a range past the text",
                edit(&|r| {
                    r[1]["range"]["end"] = 999.into();
                }),
            ),
            ("a range off a character boundary", {
                let t2 = "é".to_owned();
                let mut r: Vec<serde_json::Value> =
                    serde_json::from_str(&Doc::build(t2, Class::Text).unwrap().to_cache()).unwrap();
                r[1]["range"]["end"] = 1.into();
                serde_json::to_string(&r).unwrap()
            }),
            (
                "units out of order",
                edit(&|r| {
                    let n = r.len();
                    r.swap(1, n - 1);
                }),
            ),
            (
                "a duplicate key",
                edit(&|r| {
                    r[2]["key"] = r[1]["key"].clone();
                }),
            ),
            (
                "a parent that names no earlier unit",
                edit(&|r| {
                    r[1]["parent"] = "nope".into();
                }),
            ),
            (
                "an ast-v1 file unit for a class without a grammar",
                good.clone(),
            ),
        ];
        for (what, row) in &bad {
            let class = if *what == "an ast-v1 file unit for a class without a grammar"
                || *what == "a range off a character boundary"
            {
                Class::Text
            } else {
                Class::Rust
            };
            let text = if *what == "a range off a character boundary" {
                "é".to_owned()
            } else {
                t.clone()
            };
            assert!(
                Doc::from_cache(text, class, row).is_none(),
                "accepted a row with {what}"
            );
        }
    }

    /// Pin: the default-quote cut at a comment that starts exactly at the
    /// unit's start (unreachable from a parse, so the unit is made by hand)
    /// takes the whole first line.
    #[test]
    fn default_quote_with_a_comment_at_the_unit_start() {
        let t = "/* c */ x = 1\n".to_owned();
        let mut doc = Doc::build(t.clone(), Class::Text).unwrap();
        doc.tree = true;
        let _ = doc.comments.set(vec![(0, 7)]);
        assert_eq!(doc.entries[1].start, 0);
        assert_eq!(&t[..doc.default_quote_end(1)], "/* c */ x = 1");
        // A comment later on the line cuts there.
        let mut doc = Doc::build("x = 1 /* c */\n".to_owned(), Class::Text).unwrap();
        doc.tree = true;
        let _ = doc.comments.set(vec![(6, 13)]);
        assert_eq!(doc.default_quote_end(1), 5);
    }

    /// Ruling on section 6.7: a completed quote is cut to MAX_QUOTE_CHARS
    /// characters and trimmed, never dropped; an empty span is `internal`.
    #[test]
    fn completed_quote_is_cut_to_the_limit_and_never_dropped() {
        // One block unit: 2,047 'a', a space, then 10 'b'.
        let t = format!("{} {}\n", "a".repeat(MAX_QUOTE_CHARS - 1), "b".repeat(10));
        let doc = Doc::build(t.clone(), Class::Text).unwrap();
        let i = 1;
        assert_eq!(doc.entries[i].unit.kind, "block");
        let sel = doc.quote_selector(
            i,
            Span {
                start: 0,
                end: t.len() - 1,
            },
        );
        // The 2,048th character is the space: cut there, then trimmed. The
        // windows are cut around the stored quote, inside the unit.
        let quote = sel.quote.clone().unwrap();
        assert_eq!(quote, "a".repeat(MAX_QUOTE_CHARS - 1));
        assert_eq!(sel.prefix.as_deref(), Some(""));
        assert_eq!(sel.suffix.as_deref(), Some("bbbbbbbbbb"));
        assert!(
            doc.selects(&sel, i),
            "a cut quote's selector finds its unit"
        );
        // A span of exactly MAX_QUOTE_CHARS characters is kept whole.
        let t = format!("{}\n", "c".repeat(MAX_QUOTE_CHARS));
        let doc = Doc::build(t.clone(), Class::Text).unwrap();
        let sel = doc.quote_selector(
            1,
            Span {
                start: 0,
                end: MAX_QUOTE_CHARS,
            },
        );
        assert_eq!(sel.quote.unwrap().len(), MAX_QUOTE_CHARS);
        assert_eq!(sel.suffix.as_deref(), Some(""));
    }

    #[test]
    fn cache_round_trip_gives_the_same_answers() {
        let t = "// lead\nfn a() { 1 } // c\nstruct S;\nimpl S { fn m(&self) {} }\n".to_owned();
        let fresh = Doc::build(t.clone(), Class::Rust).unwrap();
        let warm = Doc::from_cache(t, Class::Rust, &fresh.to_cache()).unwrap();
        assert_eq!(fresh.units(None), warm.units(None));
        for q in ["c", "fn a", "fn m", "1 }"] {
            let sel = Selector::quote(q);
            assert_eq!(fresh.resolve(&sel), warm.resolve(&sel), "{q}");
        }
        assert_eq!(
            fresh.render("method:S.m").unwrap(),
            warm.render("method:S.m").unwrap()
        );
    }
}
