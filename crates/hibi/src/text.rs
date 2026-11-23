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

/// `Some(levenshtein(a, b))` when that distance is at most `max`, `None`
/// otherwise. Only the diagonal band of width `2 * max + 1` is computed, and
/// the computation stops as soon as every cell of a row exceeds `max`, so
/// the cost is `O(min(len) * max)` instead of `O(len(a) * len(b))`.
pub fn levenshtein_at_most(a: &[char], b: &[char], max: usize) -> Option<usize> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    if long.len() - short.len() > max {
        return None;
    }
    let s = short.len();
    // Cells outside the band hold `over`; no stored value exceeds it.
    let over = max.saturating_add(1);
    let mut prev: Vec<usize> = (0..=s).map(|j| if j <= max { j } else { over }).collect();
    let mut cur = vec![over; s + 1];
    for (i, &lc) in long.iter().enumerate() {
        let i = i + 1;
        let lo = i.saturating_sub(max);
        let hi = (i.saturating_add(max)).min(s);
        // The next row reads this row at `lo - 1 ..= hi + 1` only.
        let mut row_min = over;
        if lo == 0 {
            cur[0] = i;
            row_min = i;
        } else {
            cur[lo - 1] = over;
        }
        for j in lo.max(1)..=hi {
            let substitute = prev[j - 1].saturating_add(usize::from(lc != short[j - 1]));
            let delete = prev[j].saturating_add(1);
            let insert = cur[j - 1].saturating_add(1);
            cur[j] = substitute.min(delete).min(insert).min(over);
            row_min = row_min.min(cur[j]);
        }
        if hi < s {
            cur[hi + 1] = over;
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Some(prev[s]).filter(|&d| d <= max)
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
///
/// Memory: 4 bytes per character, plus 4 bytes per character when the text
/// is not all ASCII. Offsets are `u32`: every text hibi locates in is
/// bounded far below 4 GiB (`MAX_FILE_BYTES`, section 6.1, and the 16 MiB
/// protocol line limit, section 13.1).
#[derive(Debug, Clone)]
pub struct Chars {
    chars: Vec<char>,
    /// Empty when the text is all ASCII (byte offset equals character
    /// index). Otherwise `offsets[i]` is the byte offset of character `i`,
    /// and one extra entry holds the byte length.
    offsets: Vec<u32>,
}

impl Chars {
    /// Panics if the text is 4 GiB or longer.
    pub fn new(text: &str) -> Self {
        let len = u32::try_from(text.len()).expect("a located text is shorter than 4 GiB");
        let mut chars = Vec::with_capacity(text.chars().count());
        chars.extend(text.chars());
        let offsets = if chars.len() == text.len() {
            Vec::new()
        } else {
            let mut offsets = Vec::with_capacity(chars.len() + 1);
            // Each offset is below `len`, which fits in u32.
            offsets.extend(text.char_indices().map(|(b, _)| b as u32));
            offsets.push(len);
            offsets
        };
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
        let i = i.min(self.chars.len());
        if self.offsets.is_empty() {
            i
        } else {
            self.offsets[i] as usize
        }
    }

    /// Character index at or before byte offset `byte`, clamped to the
    /// character count.
    pub fn index(&self, byte: usize) -> usize {
        let n = self.chars.len();
        if self.offsets.is_empty() {
            return byte.min(n);
        }
        if byte >= self.offsets[n] as usize {
            return n;
        }
        // `byte` is below the byte length, so it fits in u32.
        match self.offsets.binary_search(&(byte as u32)) {
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
    fn ascii_offsets() {
        let t = "ab c";
        let c = Chars::new(t);
        for b in 0..=6 {
            assert_eq!(c.index(b), byte_to_char(t, b), "{b}");
        }
        for i in 0..=6 {
            assert_eq!(c.byte(i), char_to_byte(t, i), "{i}");
        }
        assert_eq!(c.string(1, 9), "b c");
        let e = Chars::new("");
        assert_eq!((e.len(), e.index(3), e.byte(2)), (0, 0, 0));
    }

    /// `levenshtein_at_most` agrees with the full computation for every
    /// bound, on pseudo-random strings over small alphabets.
    #[test]
    fn banded_levenshtein_agrees() {
        let mut seed: u64 = 7;
        let mut rnd = |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let alphabet = ['a', 'b', 'c', 'é', '😀'];
        for _ in 0..3000 {
            let k = 1 + rnd(alphabet.len() as u64) as usize;
            let mut word = |len: u64| -> Vec<char> {
                (0..rnd(len))
                    .map(|_| alphabet[rnd(k as u64) as usize])
                    .collect()
            };
            let a = word(14);
            let b = word(14);
            let d = levenshtein_chars(&a, &b);
            for max in 0..=16 {
                let want = (d <= max).then_some(d);
                assert_eq!(levenshtein_at_most(&a, &b, max), want, "{a:?} {b:?} {max}");
            }
            assert_eq!(levenshtein_at_most(&a, &b, usize::MAX), Some(d));
        }
    }

    #[test]
    fn similarity_bounds() {
        assert_eq!(similarity("abc", "xyzw"), 0.0);
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "yabd"), 2);
    }
}
