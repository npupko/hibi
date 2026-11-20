//! One decoded resource and its units: building them (sections 6.1 to 6.6),
//! the cache form (section 11.3), resolution (section 6.7) and render
//! (section 6.9).
//!
//! Pure: everything here is a function of the resource text and its class.

use std::cell::OnceCell;

use crate::code::{self, Analysis, Class};
use crate::fingerprint::{self, AST_V1};
use crate::locate::{self, How, Located, Span};
use crate::model::{Fingerprint, Selector};
use crate::prose::{self, ProseClass};
use crate::provider::{ProviderError, ProviderErrorCode, Range, Refuse, Rendered, Unit};
use crate::text;

/// Characters of `normalize(text)` in a label (section 6.2).
const LABEL_CHARS: usize = 80;
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
    /// Comment node ranges, when `tree`. Filled on first use after a cache
    /// hit.
    comments: OnceCell<Vec<(usize, usize)>>,
    /// The heading path of every entry (empty for the `file` unit and for
    /// classes without prose units). Filled on first use after a cache hit.
    headings: OnceCell<Vec<Vec<String>>>,
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
    pub(super) fn build(text: String, class: Class) -> Doc {
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
            raws.push(file(fingerprint::prose_v1(&text), &text));
            let units = prose::units(&text, pc);
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
                match code::analyze(&text, class) {
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
                Some(code::text_units(&text))
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
    ) -> Doc {
        let mut doc = Doc {
            text,
            class,
            tree,
            entries: Vec::with_capacity(raws.len()),
            comments,
            headings,
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
                    refuse: None,
                },
                symbol: r.symbol,
                start: r.start,
                end: r.end,
            };
            doc.entries.push(entry);
        }
        for i in 0..doc.entries.len() {
            let sel = doc.completed(i, None);
            doc.entries[i].unit.sel = sel;
        }
        doc
    }

    /// The cache form of section 11.3: the JSON array of the units without
    /// `text`.
    pub(super) fn to_cache(&self) -> String {
        let units: Vec<&Unit> = self.entries.iter().map(|e| &e.unit).collect();
        serde_json::to_string(&units).expect("units serialize")
    }

    /// Rebuild a document from its cache form. `None` when the row does not
    /// describe this text (it is then ignored and rewritten).
    pub(super) fn from_cache(text: String, class: Class, row: &str) -> Option<Doc> {
        let units: Vec<Unit> = serde_json::from_str(row).ok()?;
        let first = units.first()?;
        if first.key != "file" || first.range?.end != text.len() {
            return None;
        }
        let tree = class.is_code() && first.fp.profile() == AST_V1;
        let mut entries = Vec::with_capacity(units.len());
        for u in units {
            let r = u.range?;
            if r.start > r.end
                || r.end > text.len()
                || !text.is_char_boundary(r.start)
                || !text.is_char_boundary(r.end)
            {
                return None;
            }
            entries.push(Entry {
                symbol: u.sel.symbol.clone(),
                start: r.start,
                end: r.end,
                unit: u,
            });
        }
        Some(Doc {
            text,
            class,
            tree,
            entries,
            comments: OnceCell::new(),
            headings: OnceCell::new(),
        })
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

    /// `in_comment(L)` of section 6.7 for a span.
    fn in_comment(&self, s: Span) -> bool {
        self.tree
            && self
                .comments()
                .iter()
                .any(|&(cs, ce)| cs < s.end && s.start < ce)
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
            self.comments()
                .iter()
                .map(|&(s, _)| s)
                .find(|&s| e.start <= s && s < line_end)
                .unwrap_or(line_end)
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
    fn completed(&self, i: usize, kept: Option<Span>) -> Selector {
        let e = &self.entries[i];
        if e.unit.kind == "file" {
            return Selector::empty();
        }
        let (s, end) = match kept {
            Some(span) => (span.start, span.end),
            None => (e.start, self.default_quote_end(i)),
        };
        let mut sel = match locate::quote_selector(&self.text, s, end) {
            Ok(q) => Selector {
                quote: Some(q.quote),
                prefix: Some(q.prefix),
                suffix: Some(q.suffix),
                pos: Some(q.pos as u64),
                ..Selector::default()
            },
            Err(_) => Selector::default(),
        };
        sel.symbol = e.symbol.clone();
        sel
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

    /// `inside(L, u)`.
    fn inside(&self, l: &Located, i: usize) -> bool {
        let e = &self.entries[i];
        matches!(l, Located::Found { how: How::Exact, span } if e.start <= span.start && span.end <= e.end)
    }

    /// The innermost unit for a span.
    fn innermost(&self, s: Span) -> usize {
        let others = || self.entries.iter().enumerate().skip(1);
        others()
            .filter(|(_, e)| e.start <= s.start && s.start < e.end)
            .min_by_key(|(i, e)| (e.end - e.start, *i))
            .or_else(|| others().find(|(_, e)| e.start < s.end && s.start < e.end))
            .map_or(0, |(i, _)| i)
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
        match locate::locate(
            &self.text[from..to],
            quote,
            sel.prefix.as_deref().unwrap_or_default(),
            sel.suffix.as_deref().unwrap_or_default(),
            pos,
        ) {
            Located::Found { how, span } => Located::Found {
                how,
                span: shift(span),
            },
            Located::Ambiguous { span, ties } => Located::Ambiguous {
                span: shift(span),
                ties: ties.into_iter().map(shift).collect(),
            },
            Located::Missing => Located::Missing,
        }
    }

    /// One resolved unit with its text, `approx`, `refuse` and completed
    /// selector. `l` is the final locate result, `None` without a quote.
    fn out(&self, i: usize, l: Option<&Located>, approx: bool, refuse: Option<Refuse>) -> Unit {
        let e = &self.entries[i];
        let kept = match l {
            Some(l) if refuse.is_none() && self.inside(l, i) => l.span(),
            _ => None,
        };
        let mut u = e.unit.clone();
        u.sel = self.completed(i, kept);
        u.text = Some(self.text[e.start..e.end].to_owned());
        u.approx = approx;
        u.refuse = refuse;
        u
    }

    /// `resolve(resource, sel)` of section 6.7. The selector has no `key`.
    pub(super) fn resolve(&self, sel: &Selector) -> Vec<Unit> {
        let has_quote = sel.quote.is_some();
        if !has_quote && sel.symbol.is_none() {
            return vec![self.out(0, None, false, None)];
        }
        let mut missed = false;
        if let Some(symbol) = &sel.symbol {
            let s: Vec<usize> = (0..self.entries.len())
                .filter(|&i| self.entries[i].symbol.as_deref() == Some(symbol.as_str()))
                .collect();
            if s.len() == 1 {
                let i = s[0];
                if !has_quote {
                    return vec![self.out(i, None, false, None)];
                }
                let e = &self.entries[i];
                let mut l = self.locate_in(e.start, e.end, sel);
                if let Located::Ambiguous { ties, .. } = &l {
                    l = self.pick(ties);
                }
                let refuse = match &l {
                    Located::Found { span, .. } if self.in_comment(*span) => {
                        Some(Refuse::QuoteInComment)
                    }
                    _ => None,
                };
                return vec![self.out(i, Some(&l), false, refuse)];
            }
            if s.len() > 1 {
                if has_quote {
                    let l = self.locate_in(0, self.text.len(), sel);
                    if let Located::Found { how, span } = l {
                        let c: Vec<usize> = s
                            .iter()
                            .copied()
                            .filter(|&i| {
                                let e = &self.entries[i];
                                e.start <= span.start && span.start < e.end
                            })
                            .collect();
                        if c.len() == 1 {
                            let refuse = self.in_comment(span).then_some(Refuse::QuoteInComment);
                            return vec![self.out(c[0], Some(&l), how == How::Fuzzy, refuse)];
                        }
                    }
                    return s
                        .iter()
                        .map(|&i| self.out(i, Some(&l), false, None))
                        .collect();
                }
                return s.iter().map(|&i| self.out(i, None, false, None)).collect();
            }
            missed = true;
        }
        if has_quote {
            let mut l = self.locate_in(0, self.text.len(), sel);
            match &l {
                Located::Missing => return Vec::new(),
                Located::Ambiguous { ties, .. } => {
                    let mut u: Vec<usize> = Vec::new();
                    for t in ties {
                        let i = self.innermost(*t);
                        if !u.contains(&i) {
                            u.push(i);
                        }
                    }
                    if u.len() > 1 {
                        return u
                            .into_iter()
                            .map(|i| self.out(i, Some(&l), false, None))
                            .collect();
                    }
                    l = self.pick(ties);
                }
                Located::Found { .. } => {}
            }
            let Located::Found { how, span } = l else {
                unreachable!("missing and ambiguous results returned above")
            };
            let i = self.innermost(span);
            let refuse = if self.in_comment(span) {
                Some(Refuse::QuoteInComment)
            } else if how == How::Exact && !self.inside(&l, i) {
                Some(Refuse::QuoteSpansUnits)
            } else {
                None
            };
            return vec![self.out(i, Some(&l), how == How::Fuzzy || missed, refuse)];
        }
        Vec::new()
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

    #[test]
    fn cache_round_trip_gives_the_same_answers() {
        let t = "// lead\nfn a() { 1 } // c\nstruct S;\nimpl S { fn m(&self) {} }\n".to_owned();
        let fresh = Doc::build(t.clone(), Class::Rust);
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
