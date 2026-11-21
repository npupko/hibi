//! The prose region splitter (spec section 6.3).
//!
//! Pure: no file, clock, environment or process access. The input is the
//! resource text of section 4.2 (no CRLF). Offsets are UTF-8 bytes.

use std::collections::HashMap;

use crate::fingerprint;
use crate::model::Fingerprint;
use crate::text::{is_ws, trim, trim_end, trim_start};

/// The two prose classes of section 6.1. Only `Markdown` skips frontmatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProseClass {
    /// `.md`, `.mdx`
    Markdown,
    /// `.markdown`, `.txt`, `.rst`
    Prose,
}

/// The prose unit kinds of section 6.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProseKind {
    Sentence,
    Heading,
    ListItem,
    TableRow,
    Fenced,
}

impl ProseKind {
    /// The kind name used in units and keys.
    pub fn as_str(self) -> &'static str {
        match self {
            ProseKind::Sentence => "sentence",
            ProseKind::Heading => "heading",
            ProseKind::ListItem => "list-item",
            ProseKind::TableRow => "table-row",
            ProseKind::Fenced => "fenced",
        }
    }
}

/// One region of the splitter: a prose unit without key and fingerprint.
/// `text[start..end]` is the unit's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub kind: ProseKind,
    pub start: usize,
    pub end: usize,
    /// 1-based line of the first character.
    pub line: usize,
    /// 1-based line of the last character.
    pub end_line: usize,
    /// Heading texts above the unit, outermost first (step 6).
    pub heading_path: Vec<String>,
}

/// A prose unit with its `prose-v1` fingerprint and its key (section 6.6,
/// duplicates numbered). Its parent is the `file` unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProseUnit {
    pub region: Region,
    pub fp: Fingerprint,
    pub key: String,
}

/// The prose units of a resource in source order, with fingerprints and
/// keys. The `file` unit is not included.
pub fn units(text: &str, class: ProseClass) -> Vec<ProseUnit> {
    units_from(text, split(text, class))
}

/// The prose units of `regions`, the result of [`split`] on `text`, with
/// fingerprints and keys. Lets a caller check the region count before the
/// fingerprints are computed.
pub fn units_from(text: &str, regions: Vec<Region>) -> Vec<ProseUnit> {
    let fps: Vec<Fingerprint> = regions
        .iter()
        .map(|r| fingerprint::prose_v1(&text[r.start..r.end]))
        .collect();
    let mut keys: Vec<String> = regions
        .iter()
        .zip(&fps)
        .map(|(r, fp)| fingerprint::content_key(r.kind.as_str(), fp))
        .collect();
    fingerprint::number_duplicates(&mut keys);
    regions
        .into_iter()
        .zip(fps)
        .zip(keys)
        .map(|((region, fp), key)| ProseUnit { region, fp, key })
        .collect()
}

/// A line of the text: `[start, end)` without its U+000A.
#[derive(Debug, Clone, Copy)]
struct Line {
    start: usize,
    end: usize,
}

fn lines_of(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            lines.push(Line { start, end: i });
            start = i + 1;
        }
    }
    lines.push(Line {
        start,
        end: text.len(),
    });
    lines
}

/// 1-based line of byte `b`, by the line table.
fn line_of(lines: &[Line], b: usize) -> usize {
    lines.partition_point(|l| l.start <= b)
}

/// `[s, e)` with WS removed at both ends; `None` when nothing is left.
fn trim_span(text: &str, s: usize, e: usize) -> Option<(usize, usize)> {
    let slice = &text[s..e];
    let lead = slice.len() - trim_start(slice).len();
    let t = trim(slice);
    if t.is_empty() {
        None
    } else {
        Some((s + lead, s + lead + t.len()))
    }
}

/// The fence run of a trimmed line, `^(`{3,}|~{3,})`: the marker character
/// and the length of the leading run of it.
fn fence_run(t: &str) -> Option<(char, usize)> {
    ['`', '~'].into_iter().find_map(|m| {
        let n = t.len() - t.trim_start_matches(m).len();
        (n >= 3).then_some((m, n))
    })
}

/// Whether trimmed line `t` closes an open fence of `n` characters `m`: it
/// consists only of `m`, at least `n` of them (CommonMark).
fn closes_fence(t: &str, (m, n): (char, usize)) -> bool {
    t.len() >= n && t.chars().all(|c| c == m)
}

/// `^<\/?[A-Za-z][\w.:-]*(?:\s[^<>]*)?\/?>$`
fn is_tag_line(t: &str) -> bool {
    let Some(rest) = t.strip_prefix('<') else {
        return false;
    };
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let mut chars = rest.char_indices();
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    let name_end = chars
        .find(|&(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-')))
        .map_or(rest.len(), |(i, _)| i);
    let tail = &rest[name_end..];
    if tail == ">" || tail == "/>" {
        return true;
    }
    match tail.chars().next() {
        // `\s[^<>]*` then `\/?>`: the optional `/` is also matched by `[^<>]*`.
        Some(c) if is_ws(c) => tail.ends_with('>') && !tail[..tail.len() - 1].contains(['<', '>']),
        _ => false,
    }
}

/// `^(?:-(?:[ \t]*-){2,}|\*(?:[ \t]*\*){2,}|_(?:[ \t]*_){2,})$` on a
/// trimmed line: three or more of one of `-`, `*`, `_` with only spaces or
/// tabs between them.
fn is_thematic_break(t: &str) -> bool {
    ['-', '*', '_'].into_iter().any(|m| {
        t.chars().all(|c| c == m || matches!(c, ' ' | '\t'))
            && t.chars().filter(|&c| c == m).count() >= 3
    })
}

/// Count of leading `#` and the rest, for `#{1,6}`.
fn hashes(s: &str) -> Option<(usize, &str)> {
    let n = s.len() - s.trim_start_matches('#').len();
    (1..=6).contains(&n).then(|| (n, &s[n..]))
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `^(#{1,6})\s+(.*)$` on a line with leading WS removed: the level and the
/// heading's own text (the captured text, trimmed, without a closing `#` run
/// that is preceded by WS or is the whole text, trimmed again).
fn heading(s: &str) -> Option<(usize, String)> {
    let (level, rest) = hashes(s)?;
    if !rest.starts_with(is_ws) {
        return None;
    }
    // `\s+` takes the whole WS run, so `(.*)` must hold no line terminator.
    let content = trim_start(rest);
    if content.contains(is_line_terminator) {
        return None;
    }
    let content = trim(content);
    let before_run = content.trim_end_matches('#');
    let own = if before_run.is_empty() || before_run.ends_with(is_ws) {
        trim(before_run)
    } else {
        content
    };
    Some((level, own.to_owned()))
}

/// `^(?:[-*+]\s|\d+[.)]\s)` on a line with leading WS removed; returns the
/// byte length of the marker without its WS.
fn list_marker(s: &str) -> Option<usize> {
    let after_ws = |n: usize| s[n..].starts_with(is_ws).then_some(n);
    if s.starts_with(['-', '*', '+']) {
        return after_ws(1);
    }
    let digits = s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0 && s[digits..].starts_with(['.', ')']) {
        return after_ws(digits + 1);
    }
    None
}

/// `^(?:#{1,6}\s|[-*+]\s|\d+[.)]\s|>\s?|\|)` on a line with leading WS
/// removed.
fn starts_piece(s: &str) -> bool {
    hashes(s).is_some_and(|(_, rest)| rest.starts_with(is_ws))
        || list_marker(s).is_some()
        || s.starts_with('>')
        || s.starts_with('|')
}

/// `^\|?(?:\s*:?-+:?\s*\|)+\s*(?::?-+:?\s*)?$` on a trimmed line.
fn is_table_separator(t: &str) -> bool {
    // `\s*:?-+:?\s*`
    fn cell(p: &str) -> bool {
        let p = trim(p);
        let p = p.strip_prefix(':').unwrap_or(p);
        let p = p.strip_suffix(':').unwrap_or(p);
        !p.is_empty() && p.chars().all(|c| c == '-')
    }
    let s = t.strip_prefix('|').unwrap_or(t);
    let parts: Vec<&str> = s.split('|').collect();
    let (last, groups) = parts.split_last().expect("split yields one part");
    !groups.is_empty() && groups.iter().all(|p| cell(p)) && (trim(last).is_empty() || cell(last))
}

/// Inline code spans in `p`, as byte ranges: each run of n backticks pairs
/// with the next later run of exactly n backticks; a run with no partner is
/// literal.
fn code_spans(p: &str) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let bytes = p.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let s = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            runs.push((s, i));
        } else {
            i += 1;
        }
    }
    // next_same[r]: the next later run with the length of run r, from one
    // backward pass, so that pairing stays linear.
    let mut next_same = vec![None; runs.len()];
    let mut last_of_len: HashMap<usize, usize> = HashMap::new();
    for (r, &(s, e)) in runs.iter().enumerate().rev() {
        count_work(1);
        next_same[r] = last_of_len.insert(e - s, r);
    }
    let mut spans = Vec::new();
    let mut r = 0;
    while r < runs.len() {
        count_work(1);
        match next_same[r] {
            Some(q) => {
                spans.push((runs[r].0, runs[q].1));
                r = q + 1;
            }
            None => r += 1,
        }
    }
    spans
}

#[cfg(test)]
thread_local! {
    /// Steps of the code-span pairing and of the code-span check, on this
    /// thread. Tests compare it across input sizes, so that a complexity
    /// check does not depend on machine speed.
    static WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_work(n: usize) {
    WORK.with(|w| w.set(w.get() + n));
}

#[cfg(not(test))]
#[inline(always)]
fn count_work(_: usize) {}

/// Cut positions (relative to `p`) for a non-fenced, non-heading piece:
/// after each `[.!?]["')\]]?` followed by a WS character, with the
/// exceptions of step 5.
fn sentence_cuts(p: &str, kind: ProseKind) -> Vec<usize> {
    let spans = code_spans(p);
    let marker_end = if kind == ProseKind::ListItem {
        let rest = trim_start(p);
        let lead = p.len() - rest.len();
        list_marker(rest).map_or(0, |m| {
            let after = &rest[m..];
            lead + m + (after.len() - trim_start(after).len())
        })
    } else {
        0
    };
    let mut cuts = Vec::new();
    // The first span that ends after the candidate: the spans are sorted
    // and disjoint, and candidates come in source order.
    let mut span = 0;
    for (i, c) in p.char_indices() {
        if !matches!(c, '.' | '!' | '?') {
            continue;
        }
        let mut cut = i + c.len_utf8();
        let mut next = p[cut..].chars();
        let cut = match (next.next(), next.next()) {
            (Some(w), _) if is_ws(w) => cut,
            (Some(q @ ('"' | '\'' | ')' | ']')), Some(w)) if is_ws(w) => {
                cut += q.len_utf8();
                cut
            }
            _ => continue,
        };
        while spans.get(span).is_some_and(|&(_, e)| e <= i) {
            count_work(1);
            span += 1;
        }
        count_work(1);
        if spans.get(span).is_some_and(|&(s, _)| s <= i) {
            continue;
        }
        if i < marker_end {
            continue;
        }
        if kind == ProseKind::TableRow && p[cut..].chars().all(|c| is_ws(c) || c == '|') {
            continue;
        }
        cuts.push(cut);
    }
    cuts
}

/// Split `text` into prose regions by section 6.3, in source order.
pub fn split(text: &str, class: ProseClass) -> Vec<Region> {
    let lines = lines_of(text);
    let line_str = |l: &Line| &text[l.start..l.end];

    // Step 1: frontmatter, `---` or `+++`. A leading U+FEFF is set aside for
    // recognition only; it stays in the text.
    let mut first = 0;
    let line1 = line_str(&lines[0]);
    let delim = trim_end(line1.strip_prefix('\u{feff}').unwrap_or(line1));
    if class == ProseClass::Markdown
        && (delim == "---" || delim == "+++")
        && let Some(close) = (1..lines.len()).find(|&i| trim_end(line_str(&lines[i])) == delim)
    {
        first = close + 1;
    }

    // Step 2: blocks as (first line, last line, fenced).
    let mut blocks: Vec<(usize, usize, bool)> = Vec::new();
    let mut open: Option<(char, usize)> = None;
    let mut cur: Option<(usize, usize, bool)> = None;
    for (li, line) in lines.iter().enumerate().skip(first) {
        let t = trim(line_str(line));
        let marker = fence_run(t);
        match open {
            None => open = marker,
            Some(o) if closes_fence(t, o) => open = None,
            Some(_) => {}
        }
        let boundary = open.is_none() && (t.is_empty() || is_tag_line(t) || is_thematic_break(t));
        if boundary {
            blocks.extend(cur.take());
        } else {
            let b = cur.get_or_insert((li, li, false));
            b.1 = li;
            b.2 |= marker.is_some();
        }
    }
    blocks.extend(cur.take());

    // Steps 3 to 6.
    let mut out = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    let path = |stack: &Vec<(usize, String)>| stack.iter().map(|(_, h)| h.clone()).collect();
    let mut emit = |kind: ProseKind, s: usize, e: usize, heading_path: Vec<String>| {
        if let Some((s, e)) = trim_span(text, s, e) {
            out.push(Region {
                kind,
                start: s,
                end: e,
                line: line_of(&lines, s),
                end_line: line_of(&lines, e - 1),
                heading_path,
            });
        }
    };
    for (bf, bl, fenced) in blocks {
        let Some((bs, be)) = trim_span(text, lines[bf].start, lines[bl].end) else {
            continue;
        };
        if fenced {
            emit(ProseKind::Fenced, bs, be, path(&stack));
            continue;
        }
        // Pieces as line ranges.
        let mut pieces: Vec<(usize, usize)> = Vec::new();
        let mut prev_heading = false;
        for (li, line) in lines.iter().enumerate().take(bl + 1).skip(bf) {
            let content = trim_start(line_str(line));
            if li == bf || prev_heading || starts_piece(content) {
                pieces.push((li, li));
            } else if let Some(p) = pieces.last_mut() {
                p.1 = li;
            }
            prev_heading = heading(content).is_some();
        }
        for (pf, pl) in pieces {
            let ps = if pf == bf { bs } else { lines[pf].start };
            let pe = if pl == bl { be } else { lines[pl].end };
            let first_line = trim_start(&text[ps..lines[pf].end]);
            if is_table_separator(trim(first_line)) {
                continue;
            }
            let kind = if heading(first_line).is_some() {
                ProseKind::Heading
            } else if list_marker(first_line).is_some() {
                ProseKind::ListItem
            } else if first_line.starts_with('|') {
                ProseKind::TableRow
            } else {
                ProseKind::Sentence
            };
            if kind == ProseKind::Heading {
                let (level, own) = heading(first_line).expect("checked above");
                while stack.last().is_some_and(|(l, _)| *l >= level) {
                    stack.pop();
                }
                emit(kind, ps, pe, path(&stack));
                stack.push((level, own));
                continue;
            }
            let p = &text[ps..pe];
            let mut from = 0;
            for cut in sentence_cuts(p, kind) {
                emit(kind, ps + from, ps + cut, path(&stack));
                from = cut;
            }
            emit(kind, ps + from, pe, path(&stack));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The result of `f` and the work counted while it ran.
    fn with_work<T>(f: impl FnOnce() -> T) -> (T, usize) {
        WORK.with(|w| w.set(0));
        let out = f();
        (out, WORK.with(|w| w.get()))
    }

    /// Doubling the input must at most about double the work: a linear
    /// algorithm gives a ratio near 2, a quadratic one near 4. The bound 3
    /// separates them without depending on machine speed or load.
    fn assert_linear(small: usize, large: usize, what: &str) {
        assert!(
            large <= 3 * small,
            "{what}: work grew from {small} to {large} when the input doubled"
        );
    }

    #[test]
    fn many_code_spans_each_followed_by_a_sentence_end_split_in_linear_time() {
        let run = |n: usize| {
            let text = "`a`. ".repeat(n);
            let (regions, work) = with_work(|| split(&text, ProseClass::Prose));
            assert_eq!(regions.len(), n);
            assert!(
                regions
                    .iter()
                    .all(|r| &text[r.start..r.end] == "`a`." && r.kind == ProseKind::Sentence)
            );
            work
        };
        assert_linear(run(10_000), run(20_000), "code-span check");
    }

    #[test]
    fn many_unpaired_backtick_runs_pair_in_linear_time() {
        // n runs of 2..=n+1 backticks have no partner (each used to scan every
        // later run), followed by n paired single backticks.
        let run = |n: usize| {
            let mut text = String::new();
            for len in 2..=n + 1 {
                text.push_str(&"`".repeat(len));
                text.push(' ');
            }
            text.push_str(&"`a`. ".repeat(n));
            let (spans, work) = with_work(|| code_spans(&text));
            assert_eq!(spans.len(), n);
            work
        };
        assert_linear(run(500), run(1000), "code-span pairing");
    }
}
