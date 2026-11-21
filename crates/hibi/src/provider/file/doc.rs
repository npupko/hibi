//! One decoded resource and its units: building them (sections 6.1 to 6.6),
//! the cache form (section 11.3), resolution (section 6.7) and render
//! (section 6.9).
//!
//! Pure: everything here is a function of the resource text and its class.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

use crate::code::{self, Analysis, Class};
use crate::fingerprint::{self, AST_V1};
use crate::locate::{self, How, Located, Span};
use crate::model::{Fingerprint, MAX_QUOTE_CHARS, Selector};
use crate::prose::{self, ProseClass};
use crate::provider::{ProviderError, ProviderErrorCode, Range, Refuse, Rendered, Unit};
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
    /// without error.
    tree: bool,
    entries: Vec<Entry>,
    /// Per entry: the index of its parent entry (`None` for the `file`
    /// unit). Units nest, so the parent chain is the containment chain.
    parents: Vec<Option<usize>>,
    /// The declarations of each symbol, in `units` order.
    symbols: HashMap<String, Vec<usize>>,
    /// Per entry: true for a twin, a declaration whose completed selector
    /// with the default quote and scope context another declaration of the
    /// same symbol reproduces (ruling on section 6.7). Its context is cut
    /// from the whole text. Filled on first use, once per document.
    twins: OnceCell<Vec<bool>>,
    /// Comment node ranges, when `tree`, sorted and disjoint. Filled on
    /// first use after a cache hit.
    comments: OnceCell<Vec<(usize, usize)>>,
    /// The heading path of every entry (empty for the `file` unit and for
    /// classes without prose units). Filled on first use after a cache hit.
    headings: OnceCell<Vec<Vec<String>>>,
    /// The line of the first parse error, for a code class without a
    /// tree. Filled on first use.
    error_line: OnceCell<Option<usize>>,
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
        let headings = OnceCell::new();
        if let Some(pc) = prose_class(class) {
            // Building stops as soon as the count passes the limit.
            let regions = prose::split_at_most(&text, pc, MAX_UNITS_PER_RESOURCE - 1)
                .ok_or_else(too_many_units)?;
            raws.push(file(fingerprint::prose_v1(&text), &text));
            let units = prose::units_from(&text, regions);
            let mut paths = vec![Vec::new()];
            for u in units {
                paths.push(u.region.heading_path.clone());
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
            let _ = headings.set(paths);
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
                    Analysis::Text(blocks) => Some(blocks),
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
        Doc::finish(text, class, tree, raws, comments, headings)
    }

    fn finish(
        text: String,
        class: Class,
        tree: bool,
        raws: Vec<Raw>,
        comments: OnceCell<Vec<(usize, usize)>>,
        headings: OnceCell<Vec<Vec<String>>>,
    ) -> Result<Doc, ProviderError> {
        let mut doc = Doc {
            text,
            class,
            tree,
            entries: Vec::with_capacity(raws.len()),
            parents: Vec::new(),
            symbols: HashMap::new(),
            twins: OnceCell::new(),
            comments,
            headings,
            error_line: OnceCell::new(),
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
                    context_changed: false,
                    refuse: None,
                },
                symbol: r.symbol,
                start: r.start,
                end: r.end,
            };
            doc.entries.push(entry);
        }
        doc.index();
        for i in 0..doc.entries.len() {
            let sel = doc.completed(i, None)?;
            doc.entries[i].unit.sel = sel;
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
    ///    `parent`, `label`, `range`, `fp`, `sel`;
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
        let value: serde_json::Value = serde_json::from_str(row).ok()?;
        let items = value
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= MAX_UNITS_PER_RESOURCE)?;
        let mut entries: Vec<Entry> = Vec::with_capacity(items.len());
        let mut keys: HashSet<String> = HashSet::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let obj = item.as_object()?;
            if obj.len() != KEYS.len() || !KEYS.iter().all(|k| obj.contains_key(*k)) {
                return None;
            }
            let u: Unit = serde_json::from_value(item.clone()).ok()?;
            if !super::KINDS.contains(&u.kind.as_str()) || u.text.is_some() {
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
        let tree = class.is_code() && profile == AST_V1;
        let mut doc = Doc {
            text,
            class,
            tree,
            entries,
            parents: Vec::new(),
            symbols: HashMap::new(),
            twins: OnceCell::new(),
            comments: OnceCell::new(),
            headings: OnceCell::new(),
            error_line: OnceCell::new(),
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

    fn headings(&self) -> &[Vec<String>] {
        self.headings.get_or_init(|| {
            let mut paths = vec![Vec::new(); self.entries.len()];
            if let Some(pc) = prose_class(self.class) {
                for (i, r) in prose::split(&self.text, pc).into_iter().enumerate() {
                    if let Some(p) = paths.get_mut(i + 1) {
                        *p = r.heading_path;
                    }
                }
            }
            paths
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

    /// The completed selector of entry `i` (section 6.7). `kept` is the span
    /// `[L.start, L.end)` when the input quote is kept; otherwise the default
    /// quote is used.
    ///
    /// The span is cut to its first `MAX_QUOTE_CHARS` characters, then
    /// trailing WS is removed, so the quote is never dropped for its length
    /// (ruling on section 6.7). A kept span is an exact hit of the input
    /// quote, which is at most `MAX_QUOTE_CHARS` characters, and a default
    /// quote is at most 120, so the cut is a guard. The span is never empty
    /// after trimming: a kept span is the input quote, which is non-empty
    /// after trimming (section 5.2), and a default quote starts at the
    /// unit's first character, which is not WS. An empty span would be a
    /// bug, so it is the error `internal`, not a selector without a quote.
    fn completed(&self, i: usize, kept: Option<Span>) -> Result<Selector, ProviderError> {
        let e = &self.entries[i];
        if e.unit.kind == "file" {
            return Ok(Selector::empty());
        }
        let default_end = |i| self.default_quote_end(i);
        let (s, end, is_default) = match kept {
            Some(span) => (span.start, span.end, false),
            None => (e.start, default_end(i), true),
        };
        let span = &self.text[s..end];
        let end = match span.char_indices().nth(MAX_QUOTE_CHARS) {
            Some((b, _)) => s + text::trim_end(&span[..b]).len(),
            None => end,
        };
        // The context comes from the text that resolving this selector will
        // compare it with: a declaration's scope, whether its symbol is
        // unique or not (both symbol branches locate there), so a stored
        // context is compared with text of the same kind when a symbol
        // later names several declarations. A twin, whose scope selector
        // another declaration of its symbol reproduces, and every unit that
        // is not a declaration, take it from the whole resource, which can
        // still tell them apart. So the completed selector is a fixed point
        // (rulings on section 6.7).
        let scope_context = e.symbol.is_some()
            && if is_default || (s, end) == (e.start, default_end(i)) {
                !self.twins()[i]
            } else {
                !self.kept_twin(i, Span { start: s, end })
            };
        let (from, to) = if scope_context {
            self.symbol_scope(i)
        } else {
            (0, self.text.len())
        };
        debug_assert!(from <= s && end <= to, "a kept span lies in its scope");
        let q = locate::quote_selector(&self.text[from..to], s - from, end - from).map_err(|err| {
            ProviderError::new(
                ProviderErrorCode::Internal,
                format!(
                    "the completed selector of unit {:?} has no quote span ({err}) at {s}..{end}",
                    e.unit.key
                ),
            )
        })?;
        Ok(Selector {
            quote: Some(q.quote),
            prefix: Some(q.prefix),
            suffix: Some(q.suffix),
            pos: Some((from + q.pos) as u64),
            symbol: e.symbol.clone(),
            key: None,
        })
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
            self.headings().get(i).cloned().unwrap_or_default()
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

    /// `inside(L, u)`: `L` is found and exact, and the parts of its span
    /// outside the unit consist only of whitespace, `,` and `;` (ruling on
    /// section 6.7: copying a member line with its indentation or its
    /// separator does not span another unit).
    fn inside(&self, l: &Located, i: usize) -> bool {
        matches!(l, Located::Found { how: How::Exact, span } if self.within(*span, i))
    }

    /// The parts of `span` outside unit `i` are only filler.
    fn within(&self, span: Span, i: usize) -> bool {
        let e = &self.entries[i];
        let before = &self.text[span.start..span.start.max(e.start.min(span.end))];
        let after = &self.text[span.start.max(e.end).min(span.end)..span.end];
        before.chars().all(is_filler) && after.chars().all(is_filler)
    }

    /// The text the one-declaration symbol path searches for unit `i`: its
    /// range widened over the filler (whitespace, `,`, `;`) directly
    /// before and after it, so that a quote copied with its indentation or
    /// separator is found there, and kept, exactly as in the other paths.
    fn symbol_scope(&self, i: usize) -> (usize, usize) {
        let e = &self.entries[i];
        let before = &self.text[..e.start];
        let from = before.trim_end_matches(is_filler).len();
        let after = &self.text[e.end..];
        let to = e.end + (after.len() - after.trim_start_matches(is_filler).len());
        (from, to)
    }

    /// `span` without leading and trailing whitespace (unchanged when it is
    /// all whitespace).
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

    /// The innermost unit for a span, from the span with leading and
    /// trailing whitespace ignored (ruling on section 6.7): among the units
    /// other than `file` that contain its first byte, the smallest (the
    /// earliest of equal ranges); if none, the first unit in source order
    /// that intersects it; if none, the `file` unit.
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

    /// The first unit of `candidates` (indices into the entries) that
    /// contains the first byte of `span` with whitespace ignored.
    fn containing(&self, candidates: &[usize], span: Span) -> Vec<usize> {
        let s = self.trimmed(span);
        candidates
            .iter()
            .copied()
            .filter(|&i| {
                let e = &self.entries[i];
                e.start <= s.start && s.start < e.end
            })
            .collect()
    }

    /// `pick(L)` for an ambiguous result whose tied hits lie in one unit.
    fn pick(&self, ties: &[Span]) -> Located {
        let span = ties
            .iter()
            .copied()
            .find(|&t| !self.in_comment(t))
            .unwrap_or(ties[0]);
        Located::Found {
            how: How::Exact,
            span,
        }
    }

    fn locate_in(&self, from: usize, to: usize, sel: &Selector) -> Located {
        let quote = sel.quote.as_deref().unwrap_or_default();
        let pos = sel
            .pos
            .map_or(0, |p| usize::try_from(p).unwrap_or(usize::MAX))
            .saturating_sub(from);
        let shift = |s: Span| Span {
            start: s.start + from,
            end: s.end + from,
        };
        // A new anchor carries none of `prefix`, `suffix` and `pos`: nothing
        // its author wrote can choose between exact hits, so context scoring
        // and the position tie-break of section 7.1 step 3 are not used.
        // Every exact hit is a candidate, whatever the quote's length
        // (ruling on section 6.7). A selector with any of the three is
        // stored and keeps the full cascade.
        if sel.prefix.is_none() && sel.suffix.is_none() && sel.pos.is_none() {
            let hits: Vec<Span> = self.text[from..to]
                .match_indices(quote)
                .map(|(b, _)| {
                    shift(Span {
                        start: b,
                        end: b + quote.len(),
                    })
                })
                .collect();
            if hits.len() >= 2 {
                return Located::Ambiguous {
                    span: hits[0],
                    ties: hits,
                };
            }
        }
        match locate::locate(
            &self.text[from..to],
            quote,
            sel.prefix.as_deref().unwrap_or_default(),
            sel.suffix.as_deref().unwrap_or_default(),
            pos,
        ) {
            // A located span equal to the quote after `normalize` counts as
            // exact, whatever step of section 7 found it (ruling on section
            // 6.7): a reflow that moves a line break inside the quote is
            // not an uncertain anchor. Only a difference beyond whitespace
            // leaves the result fuzzy.
            Located::Found { how, span } => {
                let span = shift(span);
                let how = if how == How::Fuzzy
                    && text::normalize(&self.text[span.start..span.end]) == text::normalize(quote)
                {
                    How::Exact
                } else {
                    how
                };
                Located::Found { how, span }
            }
            Located::Ambiguous { span, ties } => Located::Ambiguous {
                span: shift(span),
                ties: ties.into_iter().map(shift).collect(),
            },
            Located::Missing => Located::Missing,
        }
    }

    /// One resolved unit with its text, `approx`, `refuse` and completed
    /// selector. `l` is the final locate result, `None` without a quote.
    fn out(
        &self,
        i: usize,
        l: Option<&Located>,
        approx: bool,
        refuse: Option<Refuse>,
        context_changed: bool,
    ) -> Result<Unit, ProviderError> {
        let e = &self.entries[i];
        let kept = match l {
            Some(l) if refuse.is_none() && self.inside(l, i) => l.span(),
            _ => None,
        };
        let mut u = e.unit.clone();
        u.sel = self.completed(i, kept)?;
        u.text = Some(self.text[e.start..e.end].to_owned());
        u.approx = approx;
        u.refuse = refuse;
        u.context_changed = context_changed;
        Ok(u)
    }

    /// The window pairs through which a hit at `span` in unit `i` is
    /// compared with a stored context: the windows `quote_selector` cuts
    /// from `scope(i)` for a declaration (when the span lies in it), and
    /// the ones it cuts from the whole text (section 6.7).
    fn hit_windows(&self, i: usize, span: Span) -> [Option<(&str, &str)>; 2] {
        let scope = self.entries[i].symbol.as_ref().and_then(|_| {
            let (from, to) = self.symbol_scope(i);
            (from <= span.start && span.end <= to)
                .then(|| windows(&self.text[from..to], span.start - from, span.end - from))
        });
        [scope, Some(windows(&self.text, span.start, span.end))]
    }

    /// How the hit at `span` in unit `i` reproduces the stored context:
    /// [`Match::Whole`] through its whole-text windows, else
    /// [`Match::Scope`] through its scope windows (a declaration only),
    /// else [`Match::None`]. A whole-text match ranks above a scope match:
    /// a twin's context is cut from the whole text, and another copy can
    /// reproduce it only through its scope windows, for example when the
    /// twin starts the file and the other copy's scope starts with blank
    /// lines (ruling on section 6.7).
    fn reproduction(&self, i: usize, span: Span, c: &Context<'_>) -> Match {
        if c.reproduced_by(windows(&self.text, span.start, span.end)) {
            return Match::Whole;
        }
        if self.entries[i].symbol.is_some() {
            let (from, to) = self.symbol_scope(i);
            if from <= span.start
                && span.end <= to
                && c.reproduced_by(windows(
                    &self.text[from..to],
                    span.start - from,
                    span.end - from,
                ))
            {
                return Match::Scope;
            }
        }
        Match::None
    }

    /// The context score of the hit at `span` in unit `i`: the higher of
    /// its window pairs.
    fn hit_score(&self, i: usize, span: Span, c: &Context<'_>) -> f64 {
        self.hit_windows(i, span)
            .into_iter()
            .flatten()
            .map(|w| c.score(w))
            .fold(f64::MIN, f64::max)
    }

    /// The twins of the document (see `twins`). A declaration's default
    /// selector with scope context is reproduced by another declaration of
    /// its symbol when that one's default quote is the same and its scope
    /// or whole-text windows equal the first one's scope windows after
    /// `normalize`: keyed and counted once per document, linear in the
    /// declarations of duplicated symbols.
    fn twins(&self) -> &[bool] {
        self.twins.get_or_init(|| {
            type Key<'a> = (&'a str, String, String);
            let mut twin = vec![false; self.entries.len()];
            for group in self.symbols.values().filter(|g| g.len() > 1) {
                let mut scope_keys: Vec<(usize, Key<'_>)> = Vec::with_capacity(group.len());
                let mut by_scope: HashMap<Key<'_>, usize> = HashMap::new();
                let mut by_whole: HashMap<Key<'_>, Vec<usize>> = HashMap::new();
                for &i in group {
                    let span = Span {
                        start: self.entries[i].start,
                        end: self.default_quote_end(i),
                    };
                    let quote = &self.text[span.start..span.end];
                    let key =
                        |(p, s): (&str, &str)| (quote, text::normalize(p), text::normalize(s));
                    let [scope, whole] = self.hit_windows(i, span);
                    let k = key(scope.expect("a default quote lies in its scope"));
                    *by_scope.entry(k.clone()).or_default() += 1;
                    by_whole
                        .entry(key(whole.expect("whole-text windows always exist")))
                        .or_default()
                        .push(i);
                    scope_keys.push((i, k));
                }
                for (i, k) in scope_keys {
                    twin[i] = by_scope[&k] > 1
                        || by_whole
                            .get(&k)
                            .is_some_and(|js| js.iter().any(|&j| j != i));
                }
            }
            twin
        })
    }

    /// A kept quote span of declaration `i` whose scope selector another
    /// declaration of the same symbol reproduces at a hit of the quote.
    fn kept_twin(&self, i: usize, span: Span) -> bool {
        let Some(group) = self.entries[i]
            .symbol
            .as_ref()
            .and_then(|s| self.symbols.get(s))
            .filter(|g| g.len() > 1)
        else {
            return false;
        };
        let (from, to) = self.symbol_scope(i);
        let (prefix, suffix) = windows(&self.text[from..to], span.start - from, span.end - from);
        let c = Context {
            prefix: Some(prefix),
            suffix: Some(suffix),
        };
        let quote = &self.text[span.start..span.end];
        group.iter().filter(|&&j| j != i).any(|&j| {
            let (f, t) = self.symbol_scope(j);
            self.text[f..t].match_indices(quote).any(|(b, _)| {
                count_work(1);
                let hit = Span {
                    start: f + b,
                    end: f + b + quote.len(),
                };
                self.reproduction(j, hit, &c) != Match::None
            })
        })
    }

    /// The errors of a selector with a `symbol` on a resource that has no
    /// declarations to search (ruling on section 6.7). A class without
    /// symbols (markdown, prose, text) is `invalid_selector`: no anchor
    /// there can name a declaration. A file of a code class that does not
    /// parse is `unavailable`: its declarations come back when the syntax
    /// error is fixed, so a stored symbol anchor is neither resolved
    /// against text blocks nor reported as a broken provider.
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
        if self.tree {
            return Ok(());
        }
        let line = self
            .error_line
            .get_or_init(|| code::first_error_line(&self.text, self.class));
        let at = line.map_or(String::new(), |l| {
            format!(" (the first error is on line {l})")
        });
        Err(ProviderError::new(
            ProviderErrorCode::Unavailable,
            format!(
                "{loc} does not parse as {}{at}, so its declarations cannot be found; fix the syntax error",
                self.class.name()
            ),
        ))
    }

    /// The branch for several declarations of one symbol, when the quote
    /// has an exact hit in the scope of at least one of them (ruling on
    /// section 6.7). The quote is located in each declaration's own scope,
    /// and a stored context is compared through the windows its completed
    /// selector could have been cut from.
    ///
    /// - One declaration with a hit: that one.
    /// - A new anchor with hits in several: all of them (ambiguous).
    /// - A stored anchor with hits in several: the one whose windows
    ///   reproduce the stored `prefix` and `suffix`, when only one does;
    ///   all of them (ambiguous) when several do. When none does, every one
    ///   of them is a candidate (`approx`), the best context score first and
    ///   equal scores in source order: similarity proposes and orders, it
    ///   never resolves, and one guess alone could be the wrong copy.
    ///
    /// Whether a hit reproduces the context is an equality test without
    /// allocation; scores are computed only when no declaration reproduces
    /// it, so a stored anchor of one of many copies costs one pass over
    /// their scopes.
    ///
    /// `None` when no scope holds an exact hit: the fuzzy step over the
    /// whole text follows.
    fn several(&self, s: &[usize], sel: &Selector) -> Result<Option<Vec<Unit>>, ProviderError> {
        struct Found {
            unit: usize,
            reproduced: Option<(Match, Span)>,
        }
        let quote = sel.quote.as_deref().unwrap_or_default();
        if quote.is_empty() {
            return Ok(None);
        }
        let ctx = Context::of(sel);
        let pos = sel.pos.map(|p| usize::try_from(p).unwrap_or(usize::MAX));
        // The exact hits in the scope of declaration `u`.
        let hits_of = |u: usize| {
            let (from, to) = self.symbol_scope(u);
            self.text[from..to].match_indices(quote).map(move |(b, _)| {
                count_work(1);
                Span {
                    start: from + b,
                    end: from + b + quote.len(),
                }
            })
        };
        // One pass over each scope. Of the hits that reproduce the stored
        // context, the best match, then the one outside comments, then the
        // one nearest the stored position, then the earliest: within one
        // declaration the position may choose, as in the one-declaration
        // path. Hits are kept only when a score is needed (below).
        // Best match first, then outside comments, nearest `pos`, earliest.
        type Rank = (std::cmp::Reverse<Match>, bool, usize, usize);
        let mut d: Vec<Found> = Vec::new();
        for &u in s {
            let mut any = false;
            let mut reproduced: Option<(Rank, Span)> = None;
            for h in hits_of(u) {
                any = true;
                let Some(c) = &ctx else { continue };
                let level = self.reproduction(u, h, c);
                if level == Match::None {
                    continue;
                }
                let key = (
                    std::cmp::Reverse(level),
                    self.in_comment(h),
                    pos.map_or(0, |p| p.abs_diff(h.start)),
                    h.start,
                );
                if reproduced.is_none_or(|(k, _)| key < k) {
                    reproduced = Some((key, h));
                }
            }
            if any {
                d.push(Found {
                    unit: u,
                    reproduced: reproduced.map(|((level, ..), h)| (level.0, h)),
                });
            }
        }
        let at =
            |u: usize, span: Span, approx: bool, changed: bool| -> Result<Unit, ProviderError> {
                let l = Located::Found {
                    how: How::Exact,
                    span,
                };
                // A hit inside a scope can leave the unit only over filler, so
                // `quote_spans_units` cannot occur here.
                let refuse = self.in_comment(span).then_some(Refuse::QuoteInComment);
                self.out(u, Some(&l), approx, refuse, changed)
            };
        let all = |d: &[Found]| -> Result<Vec<Unit>, ProviderError> {
            d.iter()
                .map(|f| self.out(f.unit, None, false, None, false))
                .collect()
        };
        // The hit of a declaration chosen by score when it does not
        // reproduce the stored context: the highest score, then outside
        // comments, then the earliest.
        let by_score = |f: &Found, c: &Context<'_>| -> (Span, f64) {
            let mut best: Option<(Span, f64, bool)> = None;
            for h in hits_of(f.unit) {
                let score = self.hit_score(f.unit, h, c);
                let outside = !self.in_comment(h);
                let better =
                    best.is_none_or(|(_, bs, bo)| score > bs || (score == bs && outside && !bo));
                if better {
                    best = Some((h, score, outside));
                }
            }
            let (h, score, _) = best.expect("a declaration of D has a hit");
            (h, score)
        };
        let Some(c) = ctx.as_ref() else {
            return match d.len() {
                0 => Ok(None),
                1 => {
                    let hits: Vec<Span> = hits_of(d[0].unit).collect();
                    at(d[0].unit, pick_span(self, &hits), false, false).map(|u| Some(vec![u]))
                }
                _ => all(&d).map(Some),
            };
        };
        if d.len() == 1 {
            let f = &d[0];
            let u = match f.reproduced {
                Some((_, h)) => at(f.unit, h, false, false)?,
                None => at(f.unit, by_score(f, c).0, false, true)?,
            };
            return Ok(Some(vec![u]));
        }
        if d.is_empty() {
            return Ok(None);
        }
        // R: the declarations whose best hit reproduces the stored context
        // with the best match found (whole text, else scope).
        let best = d.iter().filter_map(|f| f.reproduced.map(|(m, _)| m)).max();
        let r: Vec<&Found> = d
            .iter()
            .filter(|f| best.is_some() && f.reproduced.map(|(m, _)| m) == best)
            .collect();
        match r.len() {
            1 => {
                let (_, h) = r[0].reproduced.expect("filtered on reproduced");
                return at(r[0].unit, h, false, false).map(|u| Some(vec![u]));
            }
            0 => {}
            _ => return all(&d).map(Some),
        }
        // No declaration reproduces the stored context: every one with a
        // hit is a candidate, the likeliest first. `d` is in source order
        // and the sort is stable, so equal scores keep it.
        let mut ranked: Vec<(usize, Span, f64)> = d
            .iter()
            .map(|f| {
                let (h, score) = by_score(f, c);
                (f.unit, h, score)
            })
            .collect();
        ranked.sort_by(|a, b| b.2.total_cmp(&a.2));
        ranked
            .into_iter()
            .map(|(u, h, _)| at(u, h, true, true))
            .collect::<Result<_, _>>()
            .map(Some)
    }

    /// A stored selector whose symbol names no declaration, when its quote
    /// has two or more exact hits (ruling on section 6.7). Each hit is
    /// compared with the stored context through the windows of its
    /// innermost unit, the same windows that unit's completed selector is
    /// cut from, instead of through whole-text windows only: a declaration
    /// stored context was cut from its scope. Every unit found is a
    /// candidate. The hits that reproduce the context, or else the hits
    /// with the top score, give the units: one, or several (ambiguous).
    /// Position does not choose. `None` with fewer than two hits.
    fn missed_stored(
        &self,
        sel: &Selector,
        c: &Context<'_>,
    ) -> Result<Option<Vec<Unit>>, ProviderError> {
        let quote = sel.quote.as_deref().unwrap_or_default();
        if quote.is_empty() {
            return Ok(None);
        }
        let hits: Vec<Span> = self
            .text
            .match_indices(quote)
            .map(|(b, _)| Span {
                start: b,
                end: b + quote.len(),
            })
            .collect();
        if hits.len() < 2 {
            return Ok(None);
        }
        let scored: Vec<(usize, Span, Match, f64)> = hits
            .iter()
            .map(|&h| {
                count_work(1);
                let u = self.innermost(h);
                (u, h, self.reproduction(u, h, c), self.hit_score(u, h, c))
            })
            .collect();
        let best = scored.iter().map(|x| x.2).max().unwrap_or(Match::None);
        let top = scored
            .iter()
            .filter(|x| x.2 == best)
            .map(|x| x.3)
            .fold(f64::MIN, f64::max);
        let mut seen: HashSet<usize> = HashSet::new();
        let mut units: Vec<(usize, Span, bool)> = Vec::new();
        for &(u, h, level, score) in &scored {
            let wins = if best == Match::None {
                score == top
            } else {
                level == best
            };
            if wins && seen.insert(u) {
                units.push((u, h, level != Match::None));
            }
        }
        units
            .into_iter()
            .map(|(u, h, reproduces)| {
                let l = Located::Found {
                    how: How::Exact,
                    span: h,
                };
                let refuse = if self.in_comment(h) {
                    Some(Refuse::QuoteInComment)
                } else if !self.within(h, u) {
                    Some(Refuse::QuoteSpansUnits)
                } else {
                    None
                };
                self.out(u, Some(&l), true, refuse, !reproduces)
            })
            .collect::<Result<_, _>>()
            .map(Some)
    }

    /// `resolve(resource, sel)` of section 6.7. The selector has no `key`.
    pub(super) fn resolve(&self, sel: &Selector) -> Result<Vec<Unit>, ProviderError> {
        let has_quote = sel.quote.is_some();
        // The stored context, `None` for a new anchor.
        let ctx = Context::of(sel);
        if !has_quote && sel.symbol.is_none() {
            return Ok(vec![self.out(0, None, false, None, false)?]);
        }
        let mut missed = false;
        if let Some(symbol) = &sel.symbol {
            let s: &[usize] = self.symbols.get(symbol.as_str()).map_or(&[], Vec::as_slice);
            if s.len() == 1 {
                let i = s[0];
                if !has_quote {
                    return Ok(vec![self.out(i, None, false, None, false)?]);
                }
                let (from, to) = self.symbol_scope(i);
                let mut l = self.locate_in(from, to, sel);
                if let Located::Ambiguous { ties, .. } = &l {
                    l = self.pick(ties);
                }
                let refuse = match &l {
                    Located::Found { span, .. } if self.in_comment(*span) => {
                        Some(Refuse::QuoteInComment)
                    }
                    _ => None,
                };
                // The symbol identifies the unit; a stored quote that is no
                // longer in it is reported, not acted on.
                let changed = ctx.is_some() && matches!(l, Located::Missing);
                return Ok(vec![self.out(i, Some(&l), false, refuse, changed)?]);
            }
            if s.len() > 1 {
                if has_quote {
                    if let Some(units) = self.several(s, sel)? {
                        return Ok(units);
                    }
                    let l = self.locate_in(0, self.text.len(), sel);
                    if let Located::Found { how, span } = l {
                        let c = self.containing(s, span);
                        if c.len() == 1 {
                            return Ok(vec![self.located(
                                c[0],
                                &l,
                                how,
                                span,
                                false,
                                ctx.as_ref(),
                            )?]);
                        }
                    }
                    return s
                        .iter()
                        .map(|&i| self.out(i, Some(&l), false, None, false))
                        .collect::<Result<_, _>>();
                }
                return s
                    .iter()
                    .map(|&i| self.out(i, None, false, None, false))
                    .collect::<Result<_, _>>();
            }
            missed = true;
        }
        if has_quote {
            if missed
                && let Some(c) = &ctx
                && let Some(units) = self.missed_stored(sel, c)?
            {
                return Ok(units);
            }
            let mut l = self.locate_in(0, self.text.len(), sel);
            match &l {
                Located::Missing => return Ok(Vec::new()),
                Located::Ambiguous { ties, .. } => {
                    let mut u: Vec<usize> = Vec::new();
                    let mut seen: HashSet<usize> = HashSet::new();
                    for t in ties {
                        let i = self.innermost(*t);
                        if seen.insert(i) {
                            u.push(i);
                        }
                    }
                    // With a stored symbol that names no declaration, every
                    // unit found through the quote is only a candidate.
                    if u.len() > 1 {
                        return u
                            .into_iter()
                            .map(|i| self.out(i, Some(&l), missed, None, false))
                            .collect::<Result<_, _>>();
                    }
                    l = self.pick(ties);
                }
                Located::Found { .. } => {}
            }
            let Located::Found { how, span } = l else {
                unreachable!("missing and ambiguous results returned above")
            };
            let i = self.innermost(span);
            return Ok(vec![self.located(
                i,
                &l,
                how,
                span,
                missed,
                ctx.as_ref(),
            )?]);
        }
        Ok(Vec::new())
    }

    /// Unit `i` for a quote located once at `span` in the whole text, with
    /// its refusals, `approx` and `contextChanged` (section 6.7). A stored
    /// selector whose exact span is not inside `i` is `approx`: the
    /// structure around the quote changed. `contextChanged` is set for a
    /// stored selector located exactly whose context the unit's windows do
    /// not reproduce.
    fn located(
        &self,
        i: usize,
        l: &Located,
        how: How,
        span: Span,
        missed: bool,
        ctx: Option<&Context<'_>>,
    ) -> Result<Unit, ProviderError> {
        let crosses = how == How::Exact && !self.within(span, i);
        let refuse = if self.in_comment(span) {
            Some(Refuse::QuoteInComment)
        } else if crosses {
            Some(Refuse::QuoteSpansUnits)
        } else {
            None
        };
        let approx = how == How::Fuzzy || missed || (ctx.is_some() && crosses);
        let changed =
            how == How::Exact && ctx.is_some_and(|c| self.reproduction(i, span, c) == Match::None);
        self.out(i, Some(l), approx, refuse, changed)
    }
}

/// The hit `pick` chooses among exact hits in one unit: the earliest that is
/// not in a comment, or the earliest.
fn pick_span(doc: &Doc, hits: &[Span]) -> Span {
    hits.iter()
        .copied()
        .find(|&h| !doc.in_comment(h))
        .unwrap_or(hits[0])
}

/// How a hit reproduces a stored context, in increasing rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Match {
    None,
    Scope,
    Whole,
}

/// The stored context of a selector: its `prefix` and `suffix`.
struct Context<'s> {
    prefix: Option<&'s str>,
    suffix: Option<&'s str>,
}

impl<'s> Context<'s> {
    /// `None` for a new anchor, which has none of `prefix`, `suffix` and
    /// `pos` (section 6.7).
    fn of(sel: &'s Selector) -> Option<Self> {
        (sel.prefix.is_some() || sel.suffix.is_some() || sel.pos.is_some()).then_some(Context {
            prefix: sel.prefix.as_deref(),
            suffix: sel.suffix.as_deref(),
        })
    }

    /// The windows equal the stored context after `normalize`. A `prefix`
    /// or `suffix` the selector does not carry is not compared.
    fn reproduced_by(&self, (prefix, suffix): (&str, &str)) -> bool {
        self.prefix.is_none_or(|p| same_normalized(prefix, p))
            && self.suffix.is_none_or(|s| same_normalized(suffix, s))
    }

    /// The score of section 7.1 step 3 for one pair of windows. A missing
    /// `prefix` or `suffix` counts as the empty string.
    fn score(&self, (prefix, suffix): (&str, &str)) -> f64 {
        text::similarity(prefix, self.prefix.unwrap_or_default())
            + text::similarity(suffix, self.suffix.unwrap_or_default())
    }
}

/// The context windows `quote_selector` (section 7.4) cuts around
/// `[start, end)` of `ctx`: up to `CONTEXT` characters before and after,
/// borrowed rather than copied.
fn windows(ctx: &str, start: usize, end: usize) -> (&str, &str) {
    let before = &ctx[..start];
    let from = before
        .char_indices()
        .rev()
        .nth(locate::CONTEXT - 1)
        .map_or(0, |(b, _)| b);
    let after = &ctx[end..];
    let to = after
        .char_indices()
        .nth(locate::CONTEXT)
        .map_or(after.len(), |(b, _)| b);
    (&before[from..], &after[..to])
}

/// `normalize(a) == normalize(b)` without building either string.
fn same_normalized(a: &str, b: &str) -> bool {
    // Equal bytes are the common case (an unchanged anchor, identical
    // copies) and need no character walk.
    a == b || Normalized::new(a).eq(Normalized::new(b))
}

/// The characters of `normalize(s)` (section 4.1): every run of WS becomes
/// one space, and leading and trailing WS go.
struct Normalized<'a> {
    chars: std::str::Chars<'a>,
    started: bool,
    pending_space: bool,
    held: Option<char>,
}

impl<'a> Normalized<'a> {
    fn new(s: &'a str) -> Self {
        Normalized {
            chars: s.chars(),
            started: false,
            pending_space: false,
            held: None,
        }
    }
}

impl Iterator for Normalized<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        if let Some(c) = self.held.take() {
            return Some(c);
        }
        loop {
            let c = self.chars.next()?;
            if text::is_ws(c) {
                self.pending_space = self.started;
                continue;
            }
            self.started = true;
            if self.pending_space {
                self.pending_space = false;
                self.held = Some(c);
                return Some(' ');
            }
            return Some(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            // A new anchor gives every copy; the stored anchor of a twin
            // has whole-text context and resolves to its own copy.
            assert_eq!(r.len(), if stored { 1 } else { n });
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
        let sel = doc
            .completed(
                i,
                Some(Span {
                    start: 0,
                    end: t.len() - 1,
                }),
            )
            .unwrap();
        // The 2,048th character is the space: cut there, then trimmed.
        let quote = sel.quote.unwrap();
        assert_eq!(quote, "a".repeat(MAX_QUOTE_CHARS - 1));
        assert_eq!(sel.pos, Some(0));
        assert_eq!(sel.prefix.as_deref(), Some(""));
        assert_eq!(sel.suffix.unwrap(), format!(" {}\n", "b".repeat(10)));
        // A span of exactly MAX_QUOTE_CHARS characters is kept whole.
        let t = format!("{}\n", "c".repeat(MAX_QUOTE_CHARS));
        let doc = Doc::build(t.clone(), Class::Text).unwrap();
        let sel = doc
            .completed(
                1,
                Some(Span {
                    start: 0,
                    end: MAX_QUOTE_CHARS,
                }),
            )
            .unwrap();
        assert_eq!(sel.quote.unwrap().len(), MAX_QUOTE_CHARS);
        // An empty span is a bug: internal, not a selector without a quote.
        let t = "x  y\n".to_owned();
        let doc = Doc::build(t, Class::Text).unwrap();
        let err = doc
            .completed(1, Some(Span { start: 1, end: 3 }))
            .unwrap_err();
        assert_eq!(err.code, ProviderErrorCode::Internal);
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
