//! The quote locate cascade, the fuzzy step and Bitap (spec section 7), and
//! `quote_selector` (section 16.1).
//!
//! Pure: no file, clock, environment or process access. The algorithms work
//! on character indices; every offset passed in or returned is a UTF-8 byte
//! offset on a character boundary.

use crate::model::MAX_QUOTE_CHARS;
use crate::text::{self, Chars};

/// Characters of stored context on each side of a quote.
pub const CONTEXT: usize = 48;
/// Exact hits of a quote at least this long are never broken by position.
pub const AMBIGUOUS_MIN: usize = 8;
/// The longest Bitap pattern.
pub const MAX_BITS: usize = 32;
/// Bitap match threshold.
pub const THRESHOLD: f64 = 0.4;
/// Bitap match distance.
pub const DISTANCE: f64 = 100_000.0;
/// Error budget ratio.
pub const ERROR_RATIO: f64 = 0.4;
/// Error budget cap.
pub const ERROR_CAP: usize = 256;

/// A byte range, `start` inclusive, `end` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// How a quote was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum How {
    Exact,
    Fuzzy,
}

/// The result of [`locate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Located {
    /// One place.
    Found { how: How, span: Span },
    /// Several exact hits tie. `span` is the first of them (the earliest);
    /// `ties` are all tied hits in text order, `span` included.
    Ambiguous { span: Span, ties: Vec<Span> },
    /// Not found.
    Missing,
}

impl Located {
    /// The range carried by a found or ambiguous result.
    pub fn span(&self) -> Option<Span> {
        match self {
            Located::Found { span, .. } | Located::Ambiguous { span, .. } => Some(*span),
            Located::Missing => None,
        }
    }

    /// `exact` for an ambiguous result, the found `how` otherwise.
    pub fn how(&self) -> Option<How> {
        match self {
            Located::Found { how, .. } => Some(*how),
            Located::Ambiguous { .. } => Some(How::Exact),
            Located::Missing => None,
        }
    }
}

/// `error_budget(n) = min(ERROR_CAP, floor(n * ERROR_RATIO))`.
pub fn error_budget(n: usize) -> usize {
    ((n as f64 * ERROR_RATIO).floor() as usize).min(ERROR_CAP)
}

/// `within_budget(located, quote)`: the normalized strings are equal, or
/// their Levenshtein distance is within the budget of the normalized quote.
/// The distance is computed only up to the budget (at most [`ERROR_CAP`]),
/// so the cost is linear in the length of the shorter string.
pub fn within_budget(located: &str, quote: &str) -> bool {
    let a: Vec<char> = text::normalize(located).chars().collect();
    let b: Vec<char> = text::normalize(quote).chars().collect();
    a == b || text::levenshtein_at_most(&a, &b, error_budget(b.len())).is_some()
}

fn similarity_chars(a: &[char], b: &str) -> f64 {
    let a: String = a.iter().collect();
    text::similarity(&a, b)
}

/// `locate(text, sel)` of section 7.1. `pos` is a byte offset hint; it is
/// converted to the index of the character at or before it.
pub fn locate(text: &str, quote: &str, prefix: &str, suffix: &str, pos: usize) -> Located {
    let t = Chars::new(text);
    let qlen = text::char_len(quote);
    let pos = t.index(pos);
    if quote.is_empty() {
        return Located::Missing;
    }

    // 1. Exact, non-overlapping hits, left to right (character indices).
    let hits: Vec<usize> = text.match_indices(quote).map(|(b, _)| t.index(b)).collect();
    let span = |i: usize| Span {
        start: t.byte(i),
        end: t.byte(i + qlen),
    };

    match hits.len() {
        0 => {
            // 4. Fuzzy.
            match fuzzy_chars(&t, quote, prefix, suffix, pos) {
                Some((s, e)) if within_budget(&t.string(s, e), quote) => Located::Found {
                    how: How::Fuzzy,
                    span: Span {
                        start: t.byte(s),
                        end: t.byte(e.max(s)),
                    },
                },
                _ => Located::Missing,
            }
        }
        1 => Located::Found {
            how: How::Exact,
            span: span(hits[0]),
        },
        _ => {
            // 3. Score by context, order by score descending then position.
            let mut scored: Vec<(f64, usize)> = hits
                .iter()
                .map(|&i| {
                    let before = t.slice(i.saturating_sub(CONTEXT), i);
                    let after = t.slice(i + qlen, i + qlen + CONTEXT);
                    (
                        similarity_chars(before, prefix) + similarity_chars(after, suffix),
                        i,
                    )
                })
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            let top = scored[0].0;
            if top > scored[1].0 {
                return Located::Found {
                    how: How::Exact,
                    span: span(scored[0].1),
                };
            }
            let mut tied: Vec<usize> = scored.iter().filter(|s| s.0 == top).map(|s| s.1).collect();
            tied.sort_unstable();
            if qlen < AMBIGUOUS_MIN {
                let best = tied
                    .iter()
                    .copied()
                    .min_by_key(|&i| (i.abs_diff(pos), i))
                    .expect("at least two tied hits");
                return Located::Found {
                    how: How::Exact,
                    span: span(best),
                };
            }
            Located::Ambiguous {
                span: span(tied[0]),
                ties: tied.into_iter().map(span).collect(),
            }
        }
    }
}

/// `fuzzy(text, quote, prefix, suffix, pos)` of section 7.2, with `pos` a
/// byte offset. Returns a byte range, not yet checked against the budget.
pub fn fuzzy(text: &str, quote: &str, prefix: &str, suffix: &str, pos: usize) -> Option<Span> {
    let t = Chars::new(text);
    let pos = t.index(pos);
    fuzzy_chars(&t, quote, prefix, suffix, pos).map(|(s, e)| Span {
        start: t.byte(s),
        end: t.byte(e.max(s)),
    })
}

/// The fuzzy step on character indices. The returned range may reach past
/// the text; callers cut it to the text.
fn fuzzy_chars(
    t: &Chars,
    quote: &str,
    prefix: &str,
    suffix: &str,
    pos: usize,
) -> Option<(usize, usize)> {
    let n = t.len();
    if n == 0 {
        return None;
    }
    let q: Vec<char> = quote.chars().collect();
    let bias = pos.min(n - 1);
    let better_end = |at: usize, a: usize, b: usize| -> usize {
        if a == b {
            return a;
        }
        if similarity_chars(t.slice(at, b), quote) > similarity_chars(t.slice(at, a), quote) {
            b
        } else {
            a
        }
    };
    let collapsed_end = |at: usize| -> usize {
        let target = text::char_len(&text::normalize(quote));
        let limit = n.min(at + 2 * q.len() + 16);
        let mut count = 0;
        let mut prev_ws = true;
        for i in at..limit {
            let ws = text::is_ws(t.as_slice()[i]);
            if !ws || !prev_ws {
                count += 1;
            }
            prev_ws = ws;
            if !ws && count >= target {
                return i + 1;
            }
        }
        n.min(at + q.len())
    };

    if q.len() <= MAX_BITS {
        if let Some(at) = bitap_main(t.as_slice(), &q, bias) {
            let naive = (at + q.len()).min(n);
            return Some((at, better_end(at, naive, collapsed_end(at))));
        }
    } else if let Some(at) = bitap_main(t.as_slice(), &q[..MAX_BITS], bias) {
        let naive = (at + q.len()).min(n);
        let mut end = better_end(at, naive, collapsed_end(at));
        let s: Vec<char> = suffix.chars().take(MAX_BITS).collect();
        if !s.is_empty()
            && let Some(sat) = bitap_main(t.as_slice(), &s, end)
        {
            end = better_end(at, end, sat);
        }
        return Some((at, at.max(end)));
    }
    let p: Vec<char> = prefix.chars().collect();
    if !p.is_empty() {
        let p = &p[p.len().saturating_sub(MAX_BITS)..];
        if let Some(pat) = bitap_main(t.as_slice(), p, bias) {
            let start = pat + p.len();
            return Some((start, (start + q.len()).min(n)));
        }
    }
    None
}

/// The error of [`match_main`] for a pattern longer than [`MAX_BITS`]
/// characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Bitap pattern longer than {MAX_BITS} characters")]
pub struct PatternTooLong;

/// `match_main` of diff-match-patch (section 7.3) on characters. `loc` and
/// the result are byte offsets; `None` is no match (-1).
pub fn match_main(text: &str, pattern: &str, loc: usize) -> Result<Option<usize>, PatternTooLong> {
    let p: Vec<char> = pattern.chars().collect();
    if p.len() > MAX_BITS {
        return Err(PatternTooLong);
    }
    let t = Chars::new(text);
    Ok(bitap_main(t.as_slice(), &p, t.index(loc)).map(|i| t.byte(i)))
}

/// `match_main` on character indices. The pattern is at most `MAX_BITS`
/// characters.
fn bitap_main(text: &[char], pattern: &[char], loc: usize) -> Option<usize> {
    debug_assert!(pattern.len() <= MAX_BITS);
    let loc = loc.min(text.len());
    if text == pattern {
        return Some(0);
    }
    if text.is_empty() {
        return None;
    }
    let end = (loc + pattern.len()).min(text.len());
    if &text[loc..end] == pattern {
        return Some(loc);
    }
    bitap(text, pattern, loc)
}

fn find_from(text: &[char], pattern: &[char], from: usize) -> Option<usize> {
    if pattern.len() > text.len() {
        return None;
    }
    (from.min(text.len())..=text.len() - pattern.len())
        .find(|&i| &text[i..i + pattern.len()] == pattern)
}

fn rfind_at_or_before(text: &[char], pattern: &[char], at: usize) -> Option<usize> {
    if pattern.len() > text.len() {
        return None;
    }
    let last = at.min(text.len() - pattern.len());
    (0..=last)
        .rev()
        .find(|&i| &text[i..i + pattern.len()] == pattern)
}

fn bitap(text: &[char], pattern: &[char], loc: usize) -> Option<usize> {
    let m = pattern.len();
    if m == 0 {
        return None;
    }
    let mask = |c: char| -> u32 {
        pattern
            .iter()
            .enumerate()
            .filter(|(_, p)| **p == c)
            .fold(0u32, |acc, (i, _)| acc | (1u32 << (m - i - 1)))
    };
    let score =
        |e: usize, x: usize| -> f64 { e as f64 / m as f64 + loc.abs_diff(x) as f64 / DISTANCE };

    let mut threshold = THRESHOLD;
    if let Some(b) = find_from(text, pattern, loc) {
        threshold = score(0, b).min(threshold);
        if let Some(b) = rfind_at_or_before(text, pattern, loc + m) {
            threshold = score(0, b).min(threshold);
        }
    }

    let match_bit = 1u32 << (m - 1);
    let mut best: Option<usize> = None;
    let mut bin_max = m + text.len();
    let mut last_rd: Vec<u32> = Vec::new();
    let get = |v: &[u32], i: usize| v.get(i).copied().unwrap_or(0);

    for d in 0..m {
        let mut bin_min = 0;
        let mut bin_mid = bin_max;
        while bin_min < bin_mid {
            if score(d, loc + bin_mid) <= threshold {
                bin_min = bin_mid;
            } else {
                bin_max = bin_mid;
            }
            bin_mid = (bin_max - bin_min) / 2 + bin_min;
        }
        bin_max = bin_mid;
        let mut start = (loc + 1).saturating_sub(bin_mid).max(1);
        let finish = (loc + bin_mid).min(text.len()) + m;
        let mut rd = vec![0u32; finish + 2];
        rd[finish + 1] = (1u32 << d).wrapping_sub(1);
        let mut j = finish;
        while j >= start {
            let cm = if j - 1 < text.len() {
                mask(text[j - 1])
            } else {
                0
            };
            rd[j] = if d == 0 {
                ((rd[j + 1] << 1) | 1) & cm
            } else {
                (((rd[j + 1] << 1) | 1) & cm)
                    | (((get(&last_rd, j + 1) | get(&last_rd, j)) << 1) | 1)
                    | get(&last_rd, j + 1)
            };
            if rd[j] & match_bit != 0 {
                let s = score(d, j - 1);
                if s <= threshold {
                    threshold = s;
                    best = Some(j - 1);
                    if j - 1 > loc {
                        start = (2 * loc).saturating_sub(j - 1).max(1);
                    } else {
                        break;
                    }
                }
            }
            j -= 1;
        }
        if score(d + 1, loc) > threshold {
            break;
        }
        last_rd = rd;
    }
    best
}

/// The stored form of a quote taken from a span (section 6.7, completed
/// selector): the quote, up to [`CONTEXT`] characters before and after it,
/// and its start byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteSelector {
    pub quote: String,
    pub prefix: String,
    pub suffix: String,
    pub pos: usize,
}

/// The errors of [`quote_selector`]. Its only caller, the file provider,
/// builds the span from a unit it has just located, and reports either
/// error as the provider error `internal` (section 6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SpanError {
    /// The span is empty after trimming WS.
    #[error("empty span")]
    EmptySpan,
    /// The span is longer than [`MAX_QUOTE_CHARS`] characters, the longest
    /// quote a selector may hold (section 5.2).
    #[error("span longer than {MAX_QUOTE_CHARS} characters")]
    TooLong,
}

/// `quote_selector(text, start, end)` of section 16.1: clamp the byte range
/// to the text (start moves down and end up to a character boundary, end is
/// at least start), and return the quote with its context and position. The
/// quote is the span as it is, untrimmed (section 5.2).
pub fn quote_selector(text: &str, start: usize, end: usize) -> Result<QuoteSelector, SpanError> {
    let start = text::floor_boundary(text, start);
    let end = text::ceil_boundary(text, end.max(start));
    let quote = &text[start..end];
    if text::trim(quote).is_empty() {
        return Err(SpanError::EmptySpan);
    }
    if quote.chars().nth(MAX_QUOTE_CHARS).is_some() {
        return Err(SpanError::TooLong);
    }
    let before = &text[..start];
    let prefix_start = before
        .char_indices()
        .rev()
        .nth(CONTEXT - 1)
        .map_or(0, |(b, _)| b);
    let suffix: String = text[end..].chars().take(CONTEXT).collect();
    Ok(QuoteSelector {
        quote: quote.to_owned(),
        prefix: before[prefix_start..].to_owned(),
        suffix,
        pos: start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget() {
        assert_eq!(error_budget(10), 4);
        assert_eq!(error_budget(usize::MAX), ERROR_CAP);
    }

    /// Long inputs: the budget check is banded, so 20,000 characters with
    /// the cap of 256 errors stays cheap, and the answer is exact at the cap.
    #[test]
    fn within_budget_long_inputs() {
        let quote = "abcdefghij".repeat(2000);
        let with_errors = |k: usize| -> String {
            quote
                .chars()
                .enumerate()
                .map(|(i, c)| if i % 10 == 0 && i / 10 < k { 'X' } else { c })
                .collect()
        };
        assert!(within_budget(&with_errors(ERROR_CAP), &quote));
        assert!(!within_budget(&with_errors(ERROR_CAP + 1), &quote));
        assert!(!within_budget(&"x".repeat(40_000), &quote));
    }

    #[test]
    fn quote_selector_boundaries() {
        let t = "é TARGET é";
        let q = quote_selector(t, 1, 9).unwrap();
        assert_eq!(q.quote, "é TARGET");
        assert_eq!(q.pos, 0);
        assert_eq!(quote_selector(t, 99, 120), Err(SpanError::EmptySpan));
    }

    #[test]
    fn quote_selector_length_limit() {
        let t = format!("{}x", "é".repeat(MAX_QUOTE_CHARS));
        let q = quote_selector(&t, 0, 2 * MAX_QUOTE_CHARS).unwrap();
        assert_eq!(q.quote.chars().count(), MAX_QUOTE_CHARS);
        assert_eq!(q.suffix, "x");
        assert_eq!(quote_selector(&t, 0, t.len()), Err(SpanError::TooLong));
        // The stored form of a span within the limit is a valid selector.
        let sel = crate::model::Selector {
            quote: Some(q.quote),
            prefix: Some(q.prefix),
            suffix: Some(q.suffix),
            ..Default::default()
        };
        sel.validate().unwrap();
    }

    #[test]
    fn locate_bytes_after_multibyte() {
        let t = "é😀 abc";
        assert_eq!(
            locate(t, "abc", "", "", 0),
            Located::Found {
                how: How::Exact,
                span: Span { start: 7, end: 10 }
            }
        );
        assert_eq!(match_main(t, "abd", 0).unwrap(), Some(7));
    }
}
