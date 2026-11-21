//! The fingerprint profiles `prose-v1` and `ast-v1` and the fingerprint
//! string format (spec section 8).
//!
//! Always compiled, also without feature `file`: an external provider such as
//! the reference provider uses `prose-v1`. The digest functions take strings
//! (the unit text, or the class and the semantic stream that `code` builds),
//! so this module needs no tree-sitter. The string type is
//! [`crate::model::Fingerprint`].

use unicode_normalization::UnicodeNormalization;

use crate::model::Fingerprint;
use crate::text;

/// Profile name of section 8.3.
pub const PROSE_V1: &str = "prose-v1";
/// Profile name of section 8.4.
pub const AST_V1: &str = "ast-v1";
/// Digest bytes the file provider emits: 16 bytes, 32 hex digits (section 8.1).
pub const DIGEST_BYTES: usize = 16;
/// The separator between semantic-stream tokens in `ast-v1` (section 8.4).
pub const STREAM_SEPARATOR: char = '\u{1f}';

fn fingerprint(profile: &str, input: &[u8]) -> Fingerprint {
    let digest = blake3::hash(input);
    Fingerprint::from_digest(profile, &digest.as_bytes()[..DIGEST_BYTES])
        .expect("a profile constant and 16 digest bytes form a valid fingerprint")
}

/// `prose-v1` (section 8.3): NFC, then `normalize`, then
/// `BLAKE3("hibi:prose-v1\n" + result)`, first 16 bytes.
pub fn prose_v1(text: &str) -> Fingerprint {
    let nfc: String = text.nfc().collect();
    let normalized = text::normalize(&nfc);
    let mut input = Vec::with_capacity(14 + normalized.len());
    input.extend_from_slice(b"hibi:prose-v1\n");
    input.extend_from_slice(normalized.as_bytes());
    fingerprint(PROSE_V1, &input)
}

/// `ast-v1` (section 8.4): `BLAKE3("hibi:ast-v1\n" + class + "\n" +
/// join(stream, U+001F))`, first 16 bytes. `class` is the resource class
/// name of section 6.1 (`typescript`, `tsx`, `python`, `rust`, `go`,
/// `java`); `stream` is the semantic stream that `code::semantic_stream`
/// builds.
pub fn ast_v1<S: AsRef<str>>(class: &str, stream: &[S]) -> Fingerprint {
    ast_v1_tokens(class, stream.iter().map(AsRef::as_ref))
}

/// [`ast_v1`] over tokens given one at a time. The hash is fed as they
/// come, so the joined stream is never held in memory; the digest is the
/// same.
pub fn ast_v1_tokens<'a>(class: &str, stream: impl IntoIterator<Item = &'a str>) -> Fingerprint {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"hibi:ast-v1\n");
    hasher.update(class.as_bytes());
    hasher.update(b"\n");
    let mut sep = [0u8; 4];
    let sep = STREAM_SEPARATOR.encode_utf8(&mut sep).as_bytes();
    for (i, token) in stream.into_iter().enumerate() {
        if i > 0 {
            hasher.update(sep);
        }
        hasher.update(token.as_bytes());
    }
    Fingerprint::from_digest(AST_V1, &hasher.finalize().as_bytes()[..DIGEST_BYTES])
        .expect("a profile constant and 16 digest bytes form a valid fingerprint")
}

/// The outcome of comparing a recorded fingerprint with a current one
/// (section 8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    /// Equal byte for byte.
    Same,
    /// Same profile, different strings.
    Changed,
    /// Different profiles: never compared (reason `profile_mismatch`).
    ProfileMismatch,
}

/// Compare two fingerprints by section 8.2.
pub fn compare(recorded: &Fingerprint, current: &Fingerprint) -> Comparison {
    if recorded.profile() != current.profile() {
        Comparison::ProfileMismatch
    } else if recorded == current {
        Comparison::Same
    } else {
        Comparison::Changed
    }
}

/// The content key of section 6.6 before duplicate numbering:
/// `<kind>:<h>`, where `h` is the first 12 hex digits of the digest.
pub fn content_key(kind: &str, fp: &Fingerprint) -> String {
    format!("{kind}:{}", &fp.hex()[..12])
}

/// Number duplicate keys by section 6.6: given keys in source order, the
/// first occurrence keeps its key and the n-th (n >= 2) gets `~n`.
pub fn number_duplicates(keys: &mut [String]) {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for key in keys.iter_mut() {
        let n = seen.entry(key.clone()).or_insert(0);
        *n += 1;
        if *n >= 2 {
            key.push_str(&format!("~{n}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_v1_ignores_whitespace_and_applies_nfc() {
        let a = prose_v1("Pro costs\n  $29/month.");
        assert_eq!(a, prose_v1("  Pro costs $29/month. "));
        assert_ne!(a, prose_v1("Pro costs $30/month."));
        assert_eq!(prose_v1("e\u{301}"), prose_v1("\u{e9}"));
        assert_eq!(a.profile(), PROSE_V1);
        assert_eq!(a.hex().len(), 32);
    }

    #[test]
    fn ast_v1_depends_on_class_and_token_boundaries() {
        let s = ["number:5", "=5"];
        assert_ne!(ast_v1("typescript", &s), ast_v1("tsx", &s));
        assert_ne!(ast_v1("go", &["ab", "c"]), ast_v1("go", &["a", "bc"]));
        assert_eq!(ast_v1("go", &s).profile(), AST_V1);
    }

    #[test]
    fn comparison_rules() {
        let p = prose_v1("a");
        let q = prose_v1("b");
        let a = ast_v1("go", &["a"]);
        assert_eq!(compare(&p, &p.clone()), Comparison::Same);
        assert_eq!(compare(&p, &q), Comparison::Changed);
        assert_eq!(compare(&p, &a), Comparison::ProfileMismatch);
    }

    #[test]
    fn duplicate_keys_are_numbered_in_order() {
        let mut keys: Vec<String> = ["a", "b", "a", "a", "b"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        number_duplicates(&mut keys);
        assert_eq!(keys, ["a", "b", "a~2", "a~3", "b~2"]);
    }
}
