//! Whitespace set, normalization, Levenshtein, similarity and offset
//! conversion (spec sections 4.1 to 4.3).
//!
//! Pure: no file, clock, environment or process access. "Character" means
//! one Unicode scalar value; every length and distance here counts
//! characters, and every offset that crosses this module's boundary is a
//! UTF-8 byte offset.

/// WS: the Unicode `White_Space` property (section 4.1). U+FEFF is not WS.
pub fn is_ws(c: char) -> bool {
    c.is_whitespace()
}

/// Remove leading and trailing WS.
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_ws)
}

/// Remove trailing WS.
pub fn trim_end(s: &str) -> &str {
    s.trim_end_matches(is_ws)
}

/// Remove leading WS.
pub fn trim_start(s: &str) -> &str {
    s.trim_start_matches(is_ws)
}

/// `normalize(s)`: replace every maximal run of WS with one U+0020, then
/// trim. No Unicode normalization.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if is_ws(c) {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(c);
        }
    }
    out
}

/// Edit distance over characters with unit cost for insertion, deletion and
/// substitution.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    levenshtein_chars(&a, &b)
}

/// [`levenshtein`] over character slices.
pub fn levenshtein_chars(a: &[char], b: &[char]) -> usize {
    // Keep the shorter sequence in the row.
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    if short.is_empty() {
        return long.len();
    }
    let mut row: Vec<usize> = (0..=short.len()).collect();
    for (i, &lc) in long.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &sc) in short.iter().enumerate() {
            let above = row[j + 1];
            let cost = usize::from(lc != sc);
            row[j + 1] = (diagonal + cost).min(above + 1).min(row[j] + 1);
            diagonal = above;
        }
    }
    row[short.len()]
}

/// `similarity(a, b)` of section 4.3: 1 when the normalized strings are
/// equal (also when both are empty), else
/// `max(0, 1 - levenshtein(na, nb) / max(len(na), len(nb)))` in binary64.
pub fn similarity(a: &str, b: &str) -> f64 {
    let na = normalize(a);
    let nb = normalize(b);
    if na == nb {
        return 1.0;
    }
    let ca: Vec<char> = na.chars().collect();
    let cb: Vec<char> = nb.chars().collect();
    let m = ca.len().max(cb.len());
    let d = levenshtein_chars(&ca, &cb);
    (1.0 - d as f64 / m as f64).max(0.0)
}

/// Number of characters in `s`.
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// The index of the character at or before byte offset `byte`. A byte past
/// the end gives the character count.
pub fn byte_to_char(text: &str, byte: usize) -> usize {
    if byte >= text.len() {
        return text.chars().count();
    }
    let mut b = byte;
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    text[..b].chars().count()
}

/// The byte offset of character index `index`. An index past the end gives
/// the byte length.
pub fn char_to_byte(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(b, _)| b)
}

/// The largest character boundary at or before `byte`, clamped to the text.
pub fn floor_boundary(text: &str, byte: usize) -> usize {
    let mut b = byte.min(text.len());
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    b
}

/// The smallest character boundary at or after `byte`, clamped to the text.
pub fn ceil_boundary(text: &str, byte: usize) -> usize {
    let mut b = byte.min(text.len());
    while !text.is_char_boundary(b) {
        b += 1;
    }
    b
}

/// The 1-based line of byte offset `byte`: one plus the number of U+000A
/// before it (section 4.2).
pub fn line_at(text: &str, byte: usize) -> usize {
    1 + text.as_bytes()[..byte.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

/// A text as characters with the byte offset of each, for algorithms that
/// work on character indices and report byte offsets.
#[derive(Debug, Clone)]
pub struct Chars {
    chars: Vec<char>,
    /// `offsets[i]` is the byte offset of character `i`; one extra entry
    /// holds the byte length.
    offsets: Vec<usize>,
}

impl Chars {
    pub fn new(text: &str) -> Self {
        let mut chars = Vec::with_capacity(text.len());
        let mut offsets = Vec::with_capacity(text.len() + 1);
        for (b, c) in text.char_indices() {
            chars.push(c);
            offsets.push(b);
        }
        offsets.push(text.len());
        Chars { chars, offsets }
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn as_slice(&self) -> &[char] {
        &self.chars
    }

    /// Byte offset of character index `i`, clamped to the byte length.
    pub fn byte(&self, i: usize) -> usize {
        self.offsets[i.min(self.chars.len())]
    }

    /// Character index at or before byte offset `byte`, clamped to the
    /// character count.
    pub fn index(&self, byte: usize) -> usize {
        match self.offsets.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    }

    /// `t[a .. b]`: the characters from `a` inclusive to `b` exclusive, cut
    /// to the bounds of the text; empty when `a >= b` after cutting.
    pub fn slice(&self, a: usize, b: usize) -> &[char] {
        let n = self.chars.len();
        let (a, b) = (a.min(n), b.min(n));
        if a >= b { &[] } else { &self.chars[a..b] }
    }

    /// [`Chars::slice`] as a string.
    pub fn string(&self, a: usize, b: usize) -> String {
        self.slice(a, b).iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets() {
        let t = "aé😀b";
        assert_eq!(byte_to_char(t, 0), 0);
        assert_eq!(byte_to_char(t, 2), 1);
        assert_eq!(byte_to_char(t, 3), 2);
        assert_eq!(byte_to_char(t, 5), 2);
        assert_eq!(byte_to_char(t, 7), 3);
        assert_eq!(byte_to_char(t, 99), 4);
        assert_eq!(char_to_byte(t, 3), 7);
        assert_eq!(char_to_byte(t, 4), 8);
        assert_eq!(char_to_byte(t, 9), 8);
        let c = Chars::new(t);
        for b in 0..=10 {
            assert_eq!(c.index(b), byte_to_char(t, b), "{b}");
        }
        assert_eq!(c.byte(2), 3);
        assert_eq!(c.string(1, 3), "é😀");
        assert_eq!(c.string(3, 1), "");
        assert_eq!(floor_boundary(t, 5), 3);
        assert_eq!(ceil_boundary(t, 5), 7);
        assert_eq!(line_at("a\nb\nc", 4), 3);
    }

    #[test]
    fn similarity_bounds() {
        assert_eq!(similarity("abc", "xyzw"), 0.0);
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "yabd"), 2);
    }
}
