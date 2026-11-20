//! Canonical JSON (RFC 8785), id derivation, base32 and id-prefix resolution
//! (spec sections 3.5 and 3.6).
//!
//! Pure: no file, clock, environment or process access.

use std::sync::LazyLock;

use data_encoding::{Encoding, Specification};
use serde::Serialize;

use crate::error::{Code, Error};

/// The id alphabet: Crockford base32 in lower case (section 3.5).
pub const ALPHABET: &str = "0123456789abcdefghjkmnpqrstvwxyz";

/// Number of base32 characters in an id after its type prefix.
pub const ID_CHARS: usize = 20;

/// Minimum number of base32 characters in an id argument (section 3.6).
pub const MIN_PREFIX_CHARS: usize = 4;

static BASE32: LazyLock<Encoding> = LazyLock::new(|| {
    let mut spec = Specification::new();
    spec.symbols.push_str(ALPHABET);
    spec.encoding()
        .expect("the id alphabet is a valid base32 specification")
});

/// The three record types and their id prefixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordKind {
    Claim,
    Mark,
    Retirement,
}

impl RecordKind {
    /// `c1-`, `m1-` or `r1-`.
    pub fn prefix(self) -> &'static str {
        match self {
            RecordKind::Claim => "c1-",
            RecordKind::Mark => "m1-",
            RecordKind::Retirement => "r1-",
        }
    }

    /// The kind whose prefix starts `id`, if any.
    pub fn of(id: &str) -> Option<RecordKind> {
        [RecordKind::Claim, RecordKind::Mark, RecordKind::Retirement]
            .into_iter()
            .find(|k| id.starts_with(k.prefix()))
    }

    /// `claim`, `mark` or `retirement`.
    pub fn noun(self) -> &'static str {
        match self {
            RecordKind::Claim => "claim",
            RecordKind::Mark => "mark",
            RecordKind::Retirement => "retirement",
        }
    }
}

/// RFC 8785 (JCS) serialization of `value`, as UTF-8 bytes.
pub fn to_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, Error> {
    serde_json_canonicalizer::to_vec(value)
        .map_err(|e| Error::internal(format!("canonical JSON serialization failed: {e}")))
}

/// RFC 8785 (JCS) serialization of `value`, as a string.
pub fn to_canonical_string<T: Serialize>(value: &T) -> Result<String, Error> {
    serde_json_canonicalizer::to_string(value)
        .map_err(|e| Error::internal(format!("canonical JSON serialization failed: {e}")))
}

/// The first 100 bits of `digest`, most significant bit of byte 0 first, as
/// 20 characters of [`ALPHABET`] (section 3.5 steps 3 and 4).
pub fn base32_100(digest: &[u8; 32]) -> String {
    // 15 bytes are 120 bits, exactly 24 characters; the first 20 are the
    // first 100 bits.
    let mut s = BASE32.encode(&digest[..15]);
    s.truncate(ID_CHARS);
    s
}

/// The id of a record body: `prefix + base32_100(BLAKE3(JCS(body)))`.
///
/// `body` is the record without its `id` key. If `body` serializes to an
/// object that has an `id` key, that key is removed first, so a whole
/// record may be passed.
pub fn derive_id<T: Serialize + ?Sized>(kind: RecordKind, body: &T) -> Result<String, Error> {
    let mut value = serde_json::to_value(body)
        .map_err(|e| Error::internal(format!("record serialization failed: {e}")))?;
    if let serde_json::Value::Object(map) = &mut value {
        map.remove("id");
    } else {
        return Err(Error::internal("a record body must be a JSON object"));
    }
    let bytes = to_canonical(&value)?;
    let digest = blake3::hash(&bytes);
    Ok(format!(
        "{}{}",
        kind.prefix(),
        base32_100(digest.as_bytes())
    ))
}

fn is_id_char(c: char) -> bool {
    ALPHABET.contains(c)
}

/// True when `id` matches `^(c1|m1|r1)-[0-9a-hjkmnp-tv-z]{20}$`.
pub fn is_full_id(id: &str) -> bool {
    match RecordKind::of(id) {
        Some(kind) => {
            let rest = &id[kind.prefix().len()..];
            rest.len() == ID_CHARS && rest.chars().all(is_id_char)
        }
        None => false,
    }
}

/// True when `id` is a full id of the given kind.
pub fn is_full_id_of(kind: RecordKind, id: &str) -> bool {
    is_full_id(id) && id.starts_with(kind.prefix())
}

/// Check an id argument (section 3.6 steps 1 and 2): it must match
/// `^(c1|m1|r1)-[0-9a-hjkmnp-tv-z]{4,20}$` and carry the expected type
/// prefix. Both failures are `invalid_argument`.
pub fn check_id_argument(arg: &str, expected: RecordKind) -> Result<(), Error> {
    let well_formed = match RecordKind::of(arg) {
        Some(kind) => {
            let rest = &arg[kind.prefix().len()..];
            (MIN_PREFIX_CHARS..=ID_CHARS).contains(&rest.len()) && rest.chars().all(is_id_char)
        }
        None => false,
    };
    if !well_formed {
        return Err(Error::invalid_argument(format!(
            "{arg:?} is not a record id: an id is c1-, m1- or r1- followed by {MIN_PREFIX_CHARS} to {ID_CHARS} characters of {ALPHABET}"
        )));
    }
    if !arg.starts_with(expected.prefix()) {
        return Err(Error::invalid_argument(format!(
            "{arg:?} is not a {} id: a {} id starts with {}",
            expected.noun(),
            expected.noun(),
            expected.prefix()
        )));
    }
    Ok(())
}

/// Resolve an id argument against the full ids of the records of the
/// expected type (section 3.6). Retired claims must be among `ids`.
///
/// Errors: `invalid_argument` (steps 1 and 2), `claim_not_found` when no id
/// starts with `arg`, `ambiguous_id` when several do; its message lists them.
pub fn resolve_prefix<'a, I>(arg: &str, expected: RecordKind, ids: I) -> Result<&'a str, Error>
where
    I: IntoIterator<Item = &'a str>,
{
    check_id_argument(arg, expected)?;
    let mut found: Vec<&'a str> = ids.into_iter().filter(|id| id.starts_with(arg)).collect();
    found.sort_unstable();
    found.dedup();
    match found.as_slice() {
        [one] => Ok(one),
        [] => Err(Error::new(
            Code::ClaimNotFound,
            format!("no {} id starts with {arg}", expected.noun()),
        )),
        many => Err(Error::new(
            Code::AmbiguousId,
            format!(
                "{arg} matches {} {} ids: {}",
                many.len(),
                expected.noun(),
                many.join(", ")
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_of_known_bytes() {
        // 0b00000_00001_00010_00011 ... : the bytes 00 44 32 14 c7 42 54 b6 35 cf
        // 84 65 3a 56 d7 encode the values 0, 1, 2, ..., 23.
        let mut digest = [0u8; 32];
        let bytes = [
            0x00, 0x44, 0x32, 0x14, 0xc7, 0x42, 0x54, 0xb6, 0x35, 0xcf, 0x84, 0x65, 0x3a, 0x56,
            0xd7,
        ];
        digest[..15].copy_from_slice(&bytes);
        assert_eq!(base32_100(&digest), "0123456789abcdefghjk");
        let all = [0xffu8; 32];
        assert_eq!(base32_100(&all), "zzzzzzzzzzzzzzzzzzzz");
    }

    #[test]
    fn id_shapes() {
        assert!(is_full_id("c1-0123456789abcdefghjk"));
        assert!(!is_full_id("c1-0123456789abcdefghji"));
        assert!(!is_full_id("x1-0123456789abcdefghjk"));
        assert!(!is_full_id("c1-0123456789abcdefghj"));
        assert!(is_full_id_of(RecordKind::Mark, "m1-zzzzzzzzzzzzzzzzzzzz"));
        assert!(!is_full_id_of(RecordKind::Claim, "m1-zzzzzzzzzzzzzzzzzzzz"));
    }

    #[test]
    fn prefix_resolution() {
        let ids = [
            "c1-aaaa0000000000000000",
            "c1-aaab0000000000000000",
            "c1-bbbb0000000000000000",
        ];
        let r = |arg: &str| resolve_prefix(arg, RecordKind::Claim, ids.iter().copied());
        assert_eq!(r("c1-bbbb").unwrap(), "c1-bbbb0000000000000000");
        assert_eq!(r("c1-aaab").unwrap(), "c1-aaab0000000000000000");
        assert_eq!(r("c1-aaa").unwrap_err().code, Code::InvalidArgument);
        assert_eq!(r("c1-aaaa0").unwrap(), "c1-aaaa0000000000000000");
        assert_eq!(r("c1-cccc").unwrap_err().code, Code::ClaimNotFound);
        let ids2 = ["c1-aaaa0000000000000000", "c1-aaaa1000000000000000"];
        let e = resolve_prefix("c1-aaaa", RecordKind::Claim, ids2.iter().copied()).unwrap_err();
        assert_eq!(e.code, Code::AmbiguousId);
        assert!(e.message.contains("c1-aaaa0000000000000000"));
        assert!(e.message.contains("c1-aaaa1000000000000000"));
        assert_eq!(r("m1-aaaa").unwrap_err().code, Code::InvalidArgument);
        assert_eq!(r("c1-AAAA").unwrap_err().code, Code::InvalidArgument);
        assert_eq!(r("c1-aaai").unwrap_err().code, Code::InvalidArgument);
        assert_eq!(
            r("c1-aaaa00000000000000000").unwrap_err().code,
            Code::InvalidArgument
        );
    }
}
