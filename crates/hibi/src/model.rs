//! Records, locators, selectors, anchors, fingerprints and ids (spec sections
//! 3 and 5).
//!
//! Every type here deserializes strictly: unknown keys are an error, a
//! nullable record field must be present, and a value that breaks a rule of
//! sections 3 or 5 does not deserialize. A store reader maps any such error
//! to `store_corrupt`; a standard-input reader maps it to `invalid_json`.
//! The validation errors themselves carry `invalid_anchor` (locators,
//! selectors, compact anchors) or `invalid_argument` (everything else).

use std::borrow::Borrow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canon::{self, RecordKind};
use crate::duration;
use crate::error::{Code, Error};
use crate::text;

/// The largest `pos` value: 2^53 - 1 (section 3.1).
pub const MAX_POS: u64 = (1 << 53) - 1;

/// The most characters a stored `prefix` or `suffix` holds (section 5.2).
pub const MAX_CONTEXT_CHARS: usize = 48;

/// The most characters a selector `quote` holds (section 5.2). It bounds the
/// cost of locating a quote (section 7).
pub const MAX_QUOTE_CHARS: usize = 2048;

/// The stored form of a free-text field (`note`, `by`): trimmed (WS,
/// section 4.1), and `null` when the result is empty.
fn normalize_free_text(value: Option<String>) -> Option<String> {
    value
        .map(|v| text::trim(&v).to_owned())
        .filter(|v| !v.is_empty())
}

/// A stored free-text field must already be in the form
/// [`normalize_free_text`] gives.
fn check_free_text(record: &str, name: &str, value: &Option<String>) -> Result<(), Error> {
    match value {
        Some(v) if v.is_empty() || text::trim(v) != v => Err(Error::invalid_argument(format!(
            "a {record} {name} is stored trimmed and never empty (an empty one is null)"
        ))),
        _ => Ok(()),
    }
}

fn is_control(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}')
}

// ---------------------------------------------------------------------------
// String newtypes
// ---------------------------------------------------------------------------

macro_rules! string_newtype {
    ($(#[$meta:meta])* $name:ident, $check:path, $schema_name:literal, $schema:tt) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap.
            pub fn new(s: impl Into<String>) -> Result<Self, Error> {
                let s = s.into();
                $check(&s)?;
                Ok($name(s))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = Error;
            fn from_str(s: &str) -> Result<Self, Error> {
                $name::new(s)
            }
        }

        impl TryFrom<String> for $name {
            type Error = Error;
            fn try_from(s: String) -> Result<Self, Error> {
                $name::new(s)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = Error;
            fn try_from(s: &str) -> Result<Self, Error> {
                $name::new(s)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                $name::new(s).map_err(|e| serde::de::Error::custom(e.message))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                $schema_name.into()
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!($schema)
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Locator (section 5.1)
// ---------------------------------------------------------------------------

/// The parts of a locator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocatorParts<'a> {
    /// `file:<path>`
    File { path: &'a str },
    /// `repo:<alias>//<path>`
    Repo { alias: &'a str, path: &'a str },
    /// `<scheme>:<opaque>`
    External { scheme: &'a str, opaque: &'a str },
}

fn is_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

fn is_alias(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z' | '0'..='9'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

/// `path = segment *( "/" segment )`; a segment is one or more characters
/// other than `/` and controls, and is not `.` or `..`.
pub fn is_locator_path(path: &str) -> bool {
    path.split('/')
        .all(|seg| !seg.is_empty() && seg != "." && seg != ".." && !seg.chars().any(is_control))
}

fn split_locator(s: &str) -> Result<LocatorParts<'_>, Error> {
    let bad = |why: &str| Error::invalid_anchor(format!("{s:?} is not a locator: {why}"));
    let Some((scheme, rest)) = s.split_once(':') else {
        return Err(bad("it has no scheme"));
    };
    match scheme {
        "file" => {
            if !is_locator_path(rest) {
                return Err(bad(
                    "the path must be segments separated by /, none empty, . or .., without control characters",
                ));
            }
            Ok(LocatorParts::File { path: rest })
        }
        "repo" => {
            let Some((alias, path)) = rest.split_once("//") else {
                return Err(bad("a repo locator is repo:<alias>//<path>"));
            };
            if !is_alias(alias) {
                return Err(bad("the alias must match [a-z0-9][a-z0-9-]*"));
            }
            if !is_locator_path(path) {
                return Err(bad(
                    "the path must be segments separated by /, none empty, . or .., without control characters",
                ));
            }
            Ok(LocatorParts::Repo { alias, path })
        }
        _ => {
            if !is_scheme(scheme) {
                return Err(bad("the scheme must match [a-z][a-z0-9-]*"));
            }
            if rest.is_empty() || rest.chars().any(is_control) {
                return Err(bad(
                    "the part after the scheme must be non-empty and without control characters",
                ));
            }
            Ok(LocatorParts::External {
                scheme,
                opaque: rest,
            })
        }
    }
}

fn check_locator(s: &str) -> Result<(), Error> {
    split_locator(s).map(|_| ())
}

string_newtype!(
    /// A locator (section 5.1). Compared byte for byte.
    Locator,
    check_locator,
    "Locator",
    {
        "type": "string",
        "description": "file:<path>, repo:<alias>//<path>, or <scheme>:<opaque> (spec section 5.1)",
        "pattern": "^[a-z][a-z0-9-]*:[^\\u0000-\\u001f\\u007f]+$"
    }
);

impl Locator {
    /// The scheme: the text before the first `:`.
    pub fn scheme(&self) -> &str {
        self.0.split_once(':').map_or("", |(s, _)| s)
    }

    /// The parts of the locator.
    pub fn parts(&self) -> LocatorParts<'_> {
        split_locator(&self.0).expect("a Locator is valid by construction")
    }

    /// True for the schemes the file provider serves, `file` and `repo`.
    pub fn is_file_or_repo(&self) -> bool {
        matches!(self.scheme(), "file" | "repo")
    }
}

// ---------------------------------------------------------------------------
// Selector (section 5.2), Anchor (5.3)
// ---------------------------------------------------------------------------

/// A selector (section 5.2). Every key is optional; an absent key is omitted
/// when serialized, and `null` is not accepted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(description = "A selector (spec section 5.2). No key may be null.")]
pub struct Selector {
    /// Text that occurs in the unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(min = 1))]
    pub quote: Option<String>,
    /// Up to 48 characters that preceded the quote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub prefix: Option<String>,
    /// Up to 48 characters that followed the quote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub suffix: Option<String>,
    /// Byte offset of the quote when it was recorded. A hint only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64", range(max = 9_007_199_254_740_991u64))]
    pub pos: Option<u64>,
    /// Declaration path in a code resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(min = 1))]
    pub symbol: Option<String>,
    /// A unit key, for providers that address units by key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(min = 1))]
    pub key: Option<String>,
}

/// Deserialize a present, non-null value into `Some`. Used with
/// `#[serde(default)]` so that an absent key is `None` and `null` is an error.
fn present<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectorWire {
    #[serde(default, deserialize_with = "present")]
    quote: Option<String>,
    #[serde(default, deserialize_with = "present")]
    prefix: Option<String>,
    #[serde(default, deserialize_with = "present")]
    suffix: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pos: Option<u64>,
    #[serde(default, deserialize_with = "present")]
    symbol: Option<String>,
    #[serde(default, deserialize_with = "present")]
    key: Option<String>,
}

impl<'de> Deserialize<'de> for Selector {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = SelectorWire::deserialize(d)?;
        Selector::try_from(wire).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<SelectorWire> for Selector {
    type Error = String;
    fn try_from(w: SelectorWire) -> Result<Self, String> {
        let sel = Selector {
            quote: w.quote,
            prefix: w.prefix,
            suffix: w.suffix,
            pos: w.pos,
            symbol: w.symbol,
            key: w.key,
        };
        sel.validate().map_err(|e| e.message)?;
        Ok(sel)
    }
}

/// Selector keys a provider can accept (section 13.3, `hello.selectors`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum SelectorKey {
    /// `quote`, with `prefix`, `suffix` and `pos`.
    Quote,
    Symbol,
    Key,
}

impl Selector {
    /// The empty selector `{}`.
    pub fn empty() -> Self {
        Selector::default()
    }

    /// True for `{}`.
    pub fn is_empty(&self) -> bool {
        *self == Selector::default()
    }

    /// A selector with only a quote.
    pub fn quote(quote: impl Into<String>) -> Self {
        Selector {
            quote: Some(quote.into()),
            ..Selector::default()
        }
    }

    /// The rules of section 5.2. Errors are `invalid_anchor`.
    pub fn validate(&self) -> Result<(), Error> {
        let bad = |why: String| Err(Error::invalid_anchor(format!("invalid selector: {why}")));
        if self.quote.is_none() {
            for (name, present) in [
                ("prefix", self.prefix.is_some()),
                ("suffix", self.suffix.is_some()),
                ("pos", self.pos.is_some()),
            ] {
                if present {
                    return bad(format!("{name} is allowed only together with quote"));
                }
            }
        }
        if let Some(q) = &self.quote {
            if text::trim(q).is_empty() {
                return bad("the quote is empty after trimming".into());
            }
            if q.chars().nth(MAX_QUOTE_CHARS).is_some() {
                return bad(format!(
                    "the quote is longer than {MAX_QUOTE_CHARS} characters"
                ));
            }
        }
        for (name, value) in [("prefix", &self.prefix), ("suffix", &self.suffix)] {
            if let Some(v) = value
                && v.chars().count() > MAX_CONTEXT_CHARS
            {
                return bad(format!(
                    "{name} is longer than {MAX_CONTEXT_CHARS} characters"
                ));
            }
        }
        if let Some(pos) = self.pos
            && pos > MAX_POS
        {
            return bad(format!("pos is larger than {MAX_POS}"));
        }
        for (name, value) in [("symbol", &self.symbol), ("key", &self.key)] {
            if let Some(v) = value
                && v.is_empty()
            {
                return bad(format!("{name} is empty"));
            }
        }
        Ok(())
    }

    /// The provider-facing keys this selector uses: `quote` (also for
    /// `prefix`, `suffix`, `pos`), `symbol`, `key`.
    pub fn keys(&self) -> Vec<SelectorKey> {
        let mut keys = Vec::new();
        if self.quote.is_some() {
            keys.push(SelectorKey::Quote);
        }
        if self.symbol.is_some() {
            keys.push(SelectorKey::Symbol);
        }
        if self.key.is_some() {
            keys.push(SelectorKey::Key);
        }
        keys
    }

    /// The allowed-keys rule of section 2.5, used both for a selector a
    /// provider returns and before a selector is sent to a provider:
    /// `allowed` is the provider's `ProviderInfo.selectors`; listing `quote`
    /// also allows `prefix`, `suffix` and `pos`; `{}` is always allowed.
    ///
    /// Returns the first key of this selector that `allowed` does not cover,
    /// or `None` when every key is allowed. The selector must already be
    /// valid by section 5.2 (so `prefix`, `suffix` and `pos` come with a
    /// quote).
    pub fn disallowed_key(&self, allowed: &[SelectorKey]) -> Option<SelectorKey> {
        self.keys().into_iter().find(|k| !allowed.contains(k))
    }

    /// True when [`Selector::disallowed_key`] finds nothing.
    pub fn is_allowed_by(&self, allowed: &[SelectorKey]) -> bool {
        self.disallowed_key(allowed).is_none()
    }
}

impl SelectorKey {
    /// `quote`, `symbol` or `key`.
    pub fn as_str(self) -> &'static str {
        match self {
            SelectorKey::Quote => "quote",
            SelectorKey::Symbol => "symbol",
            SelectorKey::Key => "key",
        }
    }
}

impl fmt::Display for SelectorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An anchor: a locator plus a selector (section 5.3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub loc: Locator,
    pub sel: Selector,
}

impl Anchor {
    pub fn new(loc: Locator, sel: Selector) -> Self {
        Anchor { loc, sel }
    }
}

/// Parse an anchor in the compact syntax of section 5.5 (CLI arguments).
///
/// For `file:` and `repo:` locators: split at the first `::`, the right part
/// is the quote, verbatim; split the left part at the first `#`, the right
/// part is the symbol. For any other scheme the whole argument is the
/// locator and the selector is `{}`. Errors are `invalid_anchor`.
pub fn parse_compact(arg: &str) -> Result<Anchor, Error> {
    let scheme = arg.split_once(':').map_or("", |(s, _)| s);
    if scheme != "file" && scheme != "repo" {
        let loc = Locator::new(arg)?;
        return Ok(Anchor::new(loc, Selector::empty()));
    }
    let (left, quote) = match arg.split_once("::") {
        Some((l, q)) => (l, Some(q)),
        None => (arg, None),
    };
    let (loc, symbol) = match left.split_once('#') {
        Some((l, s)) => (l, Some(s)),
        None => (left, None),
    };
    let loc = Locator::new(loc)?;
    let sel = Selector {
        quote: quote.map(str::to_owned),
        symbol: symbol.map(str::to_owned),
        ..Selector::default()
    };
    sel.validate()?;
    Ok(Anchor::new(loc, sel))
}

// ---------------------------------------------------------------------------
// Fingerprint (section 8.1)
// ---------------------------------------------------------------------------

fn check_fingerprint(s: &str) -> Result<(), Error> {
    let bad = || {
        Error::invalid_argument(format!(
            "{s:?} is not a fingerprint: expected b3:<profile>:<32 to 64 lower-case hex digits>"
        ))
    };
    let rest = s.strip_prefix("b3:").ok_or_else(bad)?;
    let (profile, hex) = rest.split_once(':').ok_or_else(bad)?;
    if !is_scheme(profile) {
        return Err(bad());
    }
    if !(32..=64).contains(&hex.len())
        || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(bad());
    }
    Ok(())
}

string_newtype!(
    /// A fingerprint string `b3:<profile>:<hex>` (section 8.1). Two
    /// fingerprints are equal when the strings are equal byte for byte.
    Fingerprint,
    check_fingerprint,
    "Fingerprint",
    {
        "type": "string",
        "pattern": "^b3:[a-z][a-z0-9-]*:[0-9a-f]{32,64}$"
    }
);

impl Fingerprint {
    /// The profile name, such as `prose-v1`.
    pub fn profile(&self) -> &str {
        self.0[3..].split_once(':').map_or("", |(p, _)| p)
    }

    /// The hex digest digits.
    pub fn hex(&self) -> &str {
        self.0[3..].split_once(':').map_or("", |(_, h)| h)
    }

    /// Build from a profile and digest bytes; the hex part is the
    /// lower-case hex of `digest` (16 to 32 bytes).
    pub fn from_digest(profile: &str, digest: &[u8]) -> Result<Self, Error> {
        let mut s = format!("b3:{profile}:");
        for b in digest {
            s.push_str(&format!("{b:02x}"));
        }
        Fingerprint::new(s)
    }
}

// ---------------------------------------------------------------------------
// Ids (section 3.5)
// ---------------------------------------------------------------------------

fn check_claim_id(s: &str) -> Result<(), Error> {
    check_full_id(RecordKind::Claim, s)
}
fn check_mark_id(s: &str) -> Result<(), Error> {
    check_full_id(RecordKind::Mark, s)
}
fn check_retirement_id(s: &str) -> Result<(), Error> {
    check_full_id(RecordKind::Retirement, s)
}
fn check_full_id(kind: RecordKind, s: &str) -> Result<(), Error> {
    if canon::is_full_id_of(kind, s) {
        Ok(())
    } else {
        Err(Error::invalid_argument(format!(
            "{s:?} is not a full {} id ({}followed by 20 characters of {})",
            kind.noun(),
            kind.prefix(),
            canon::ALPHABET
        )))
    }
}

string_newtype!(
    /// A full claim id, `c1-` and 20 base32 characters.
    ClaimId,
    check_claim_id,
    "ClaimId",
    { "type": "string", "pattern": "^c1-[0-9a-hjkmnp-tv-z]{20}$" }
);
string_newtype!(
    /// A full mark id, `m1-` and 20 base32 characters.
    MarkId,
    check_mark_id,
    "MarkId",
    { "type": "string", "pattern": "^m1-[0-9a-hjkmnp-tv-z]{20}$" }
);
string_newtype!(
    /// A full retirement id, `r1-` and 20 base32 characters.
    RetirementId,
    check_retirement_id,
    "RetirementId",
    { "type": "string", "pattern": "^r1-[0-9a-hjkmnp-tv-z]{20}$" }
);

// ---------------------------------------------------------------------------
// Timestamp (section 3.1)
// ---------------------------------------------------------------------------

/// A stored timestamp: RFC 3339 in UTC with second precision and the `Z`
/// suffix, such as `2026-11-20T18:04:11Z`.
///
/// Holds a civil date and time read as UTC, so that every instant up to
/// 9999-12-31T23:59:59Z is representable (section 9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp(jiff::civil::DateTime);

impl Timestamp {
    /// Parse the strict stored form. Errors are `invalid_argument`.
    pub fn parse(s: &str) -> Result<Self, Error> {
        let bad = || {
            Error::invalid_argument(format!(
                "{s:?} is not a timestamp: expected YYYY-MM-DDTHH:MM:SSZ in UTC"
            ))
        };
        let b = s.as_bytes();
        if b.len() != 20 {
            return Err(bad());
        }
        let digits = |r: std::ops::Range<usize>| -> Option<i32> {
            let mut n = 0i32;
            for &c in &b[r] {
                if !c.is_ascii_digit() {
                    return None;
                }
                n = n * 10 + i32::from(c - b'0');
            }
            Some(n)
        };
        if b[4] != b'-'
            || b[7] != b'-'
            || b[10] != b'T'
            || b[13] != b':'
            || b[16] != b':'
            || b[19] != b'Z'
        {
            return Err(bad());
        }
        let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(sec)) = (
            digits(0..4),
            digits(5..7),
            digits(8..10),
            digits(11..13),
            digits(14..16),
            digits(17..19),
        ) else {
            return Err(bad());
        };
        let fit = |n: i32| i8::try_from(n).map_err(|_| bad());
        let dt = jiff::civil::DateTime::new(
            i16::try_from(y).map_err(|_| bad())?,
            fit(mo)?,
            fit(d)?,
            fit(h)?,
            fit(mi)?,
            fit(sec)?,
            0,
        )
        .map_err(|_| bad())?;
        Ok(Timestamp(dt))
    }

    /// A UTC civil date-time as a stored timestamp. The fraction of a second
    /// is truncated. Errors (`invalid_argument`) for a year before 0000.
    pub fn from_datetime(dt: jiff::civil::DateTime) -> Result<Self, Error> {
        if dt.year() < 0 {
            return Err(Error::invalid_argument(format!(
                "{dt} is before year 0000 and cannot be stored"
            )));
        }
        let whole = dt
            .with()
            .subsec_nanosecond(0)
            .build()
            .map_err(|e| Error::internal(e.to_string()))?;
        Ok(Timestamp(whole))
    }

    /// An instant as a stored timestamp, in UTC, with the fraction truncated.
    pub fn from_instant(ts: jiff::Timestamp) -> Result<Self, Error> {
        Timestamp::from_datetime(jiff::tz::Offset::UTC.to_datetime(ts))
    }

    /// The UTC civil date and time.
    pub fn datetime(self) -> jiff::civil::DateTime {
        self.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = self.0;
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            d.year(),
            d.month(),
            d.day(),
            d.hour(),
            d.minute(),
            d.second()
        )
    }
}

impl FromStr for Timestamp {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Error> {
        Timestamp::parse(s)
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Timestamp::parse(&s).map_err(|e| serde::de::Error::custom(e.message))
    }
}

impl JsonSchema for Timestamp {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Timestamp".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "description": "RFC 3339 in UTC with second precision and the Z suffix",
            "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"
        })
    }
}

// ---------------------------------------------------------------------------
// Records (sections 3.2 to 3.4)
// ---------------------------------------------------------------------------

/// Deserialize a nullable record field that must be present (section 3.1:
/// optional record fields are always present and hold `null` when unset).
fn nullable<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}

/// Schema of a nullable field that is required: `T` or `null`.
struct Nullable<T>(std::marker::PhantomData<T>);

impl<T: JsonSchema> JsonSchema for Nullable<T> {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("Nullable_{}", T::schema_name()).into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        <Option<T>>::json_schema(generator)
    }
}

/// A claim (section 3.2). Immutable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub id: ClaimId,
    /// Non-empty, stored trimmed.
    pub statement: String,
    pub subject: Anchor,
    /// May be empty. Order is significant.
    pub evidence: Vec<Anchor>,
    /// A duration in the grammar of section 9.4.
    #[schemars(with = "Nullable<String>")]
    pub recheck: Option<String>,
    /// Never empty: an empty note is stored as `null`.
    #[schemars(with = "Nullable<String>")]
    pub note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimWire {
    id: ClaimId,
    statement: String,
    subject: Anchor,
    evidence: Vec<Anchor>,
    #[serde(deserialize_with = "nullable")]
    recheck: Option<String>,
    #[serde(deserialize_with = "nullable")]
    note: Option<String>,
}

impl<'de> Deserialize<'de> for Claim {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = ClaimWire::deserialize(d)?;
        Claim::try_from(wire).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<ClaimWire> for Claim {
    type Error = String;
    fn try_from(w: ClaimWire) -> Result<Self, String> {
        let c = Claim {
            id: w.id,
            statement: w.statement,
            subject: w.subject,
            evidence: w.evidence,
            recheck: w.recheck,
            note: w.note,
        };
        c.validate().map_err(|e| e.message)?;
        Ok(c)
    }
}

/// The body of a claim: the record without `id`.
#[derive(Serialize)]
struct ClaimBody<'a> {
    statement: &'a str,
    subject: &'a Anchor,
    evidence: &'a [Anchor],
    recheck: &'a Option<String>,
    note: &'a Option<String>,
}

impl Claim {
    /// Build a claim and derive its id. The statement and the note are
    /// trimmed (WS, section 4.1); an empty note becomes `null`. The result
    /// must then pass the same rules as a stored claim ([`Claim::validate`]):
    /// an empty statement is `invalid_argument`, a bad selector
    /// `invalid_anchor`, a bad `recheck` `invalid_duration`.
    pub fn new(
        statement: &str,
        subject: Anchor,
        evidence: Vec<Anchor>,
        recheck: Option<String>,
        note: Option<String>,
    ) -> Result<Claim, Error> {
        let mut c = Claim {
            id: ClaimId(String::new()),
            statement: text::trim(statement).to_owned(),
            subject,
            evidence,
            recheck,
            note: normalize_free_text(note),
        };
        c.validate_body()?;
        c.id = c.derive_id()?;
        Ok(c)
    }

    /// The id derived from the body (section 3.5).
    pub fn derive_id(&self) -> Result<ClaimId, Error> {
        let body = ClaimBody {
            statement: &self.statement,
            subject: &self.subject,
            evidence: &self.evidence,
            recheck: &self.recheck,
            note: &self.note,
        };
        Ok(ClaimId(canon::derive_id(RecordKind::Claim, &body)?))
    }

    /// The rules of section 3.2, the selector rules of section 5.2, and the
    /// id. Errors are `invalid_argument`, `invalid_anchor` or
    /// `invalid_duration`. Deserialization runs this.
    pub fn validate(&self) -> Result<(), Error> {
        self.validate_body()?;
        check_derived(self.id.as_str(), self.derive_id()?.as_str())
    }

    /// Every rule except the id. [`Claim::new`] runs this.
    fn validate_body(&self) -> Result<(), Error> {
        if self.statement.is_empty() {
            return Err(Error::invalid_argument("the statement is empty"));
        }
        if text::trim(&self.statement) != self.statement {
            return Err(Error::invalid_argument(
                "a claim statement is stored trimmed",
            ));
        }
        for a in self.sides() {
            a.sel.validate()?;
        }
        if let Some(r) = &self.recheck {
            duration::parse(r)?;
        }
        check_free_text("claim", "note", &self.note)
    }

    /// The anchors of every side: entry 0 is the subject, entry `i` is
    /// `evidence[i-1]`.
    pub fn sides(&self) -> impl Iterator<Item = &Anchor> {
        std::iter::once(&self.subject).chain(&self.evidence)
    }

    /// `1 + evidence.len()`.
    pub fn side_count(&self) -> usize {
        1 + self.evidence.len()
    }

    /// The canonical JSON of the whole record (section 3.5), without the
    /// trailing newline of a shard line.
    pub fn canonical(&self) -> Result<Vec<u8>, Error> {
        canon::to_canonical(self)
    }
}

fn check_derived(stored: &str, derived: &str) -> Result<(), Error> {
    if stored == derived {
        Ok(())
    } else {
        Err(Error::invalid_argument(format!(
            "the id {stored} is not the id derived from the record body ({derived})"
        )))
    }
}

/// One side of a mark: the side's locator and its fingerprint at check time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkSide {
    pub loc: Locator,
    pub fp: Fingerprint,
}

/// A mark (section 3.3). Append-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mark {
    pub id: MarkId,
    /// Full id of the claim.
    pub claim: ClaimId,
    /// When the check happened.
    pub at: Timestamp,
    /// Free text, kept for audit, never interpreted.
    #[schemars(with = "Nullable<String>")]
    pub by: Option<String>,
    /// One entry per side, index-aligned with the claim's sides.
    pub sides: Vec<MarkSide>,
    #[schemars(with = "Nullable<String>")]
    pub note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkWire {
    id: MarkId,
    claim: ClaimId,
    at: Timestamp,
    #[serde(deserialize_with = "nullable")]
    by: Option<String>,
    sides: Vec<MarkSide>,
    #[serde(deserialize_with = "nullable")]
    note: Option<String>,
}

impl<'de> Deserialize<'de> for Mark {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = MarkWire::deserialize(d)?;
        Mark::try_from(wire).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<MarkWire> for Mark {
    type Error = String;
    fn try_from(w: MarkWire) -> Result<Self, String> {
        let m = Mark {
            id: w.id,
            claim: w.claim,
            at: w.at,
            by: w.by,
            sides: w.sides,
            note: w.note,
        };
        m.validate().map_err(|e| e.message)?;
        Ok(m)
    }
}

#[derive(Serialize)]
struct MarkBody<'a> {
    claim: &'a ClaimId,
    at: &'a Timestamp,
    by: &'a Option<String>,
    sides: &'a [MarkSide],
    note: &'a Option<String>,
}

impl Mark {
    /// Build a mark and derive its id. `by` and `note` are trimmed (WS,
    /// section 4.1) and become `null` when empty. The result must then pass
    /// the same rules as a stored mark ([`Mark::validate`]).
    pub fn new(
        claim: ClaimId,
        at: Timestamp,
        by: Option<String>,
        sides: Vec<MarkSide>,
        note: Option<String>,
    ) -> Result<Mark, Error> {
        let mut m = Mark {
            id: MarkId(String::new()),
            claim,
            at,
            by: normalize_free_text(by),
            sides,
            note: normalize_free_text(note),
        };
        m.validate_body()?;
        m.id = m.derive_id()?;
        Ok(m)
    }

    /// The id derived from the body (section 3.5).
    pub fn derive_id(&self) -> Result<MarkId, Error> {
        let body = MarkBody {
            claim: &self.claim,
            at: &self.at,
            by: &self.by,
            sides: &self.sides,
            note: &self.note,
        };
        Ok(MarkId(canon::derive_id(RecordKind::Mark, &body)?))
    }

    /// The rules of section 3.3 that need no claim, and the id.
    /// Deserialization runs this.
    pub fn validate(&self) -> Result<(), Error> {
        self.validate_body()?;
        check_derived(self.id.as_str(), self.derive_id()?.as_str())
    }

    /// Every rule except the id. [`Mark::new`] runs this.
    fn validate_body(&self) -> Result<(), Error> {
        if self.sides.is_empty() {
            return Err(Error::invalid_argument("a mark has at least one side"));
        }
        check_free_text("mark", "by", &self.by)?;
        check_free_text("mark", "note", &self.note)
    }

    /// The rules of section 3.3 that need the claim: the claim id, the side
    /// count and every side's locator. A violation is `store_corrupt` when
    /// read from the store.
    pub fn check_against(&self, claim: &Claim) -> Result<(), Error> {
        if self.claim != claim.id {
            return Err(Error::invalid_argument(format!(
                "mark {} names claim {}, not {}",
                self.id, self.claim, claim.id
            )));
        }
        if self.sides.len() != claim.side_count() {
            return Err(Error::invalid_argument(format!(
                "mark {} has {} sides; claim {} has {}",
                self.id,
                self.sides.len(),
                claim.id,
                claim.side_count()
            )));
        }
        for (i, (side, anchor)) in self.sides.iter().zip(claim.sides()).enumerate() {
            if side.loc != anchor.loc {
                return Err(Error::invalid_argument(format!(
                    "side {i} of mark {} has locator {}; claim {} has {}",
                    self.id, side.loc, claim.id, anchor.loc
                )));
            }
        }
        Ok(())
    }

    pub fn canonical(&self) -> Result<Vec<u8>, Error> {
        canon::to_canonical(self)
    }
}

/// A retirement (section 3.4). Permanent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Retirement {
    pub id: RetirementId,
    /// Full id of the retired claim.
    pub retires: ClaimId,
    /// Full id of another claim.
    #[schemars(with = "Nullable<ClaimId>")]
    pub replaced_by: Option<ClaimId>,
    pub at: Timestamp,
    #[schemars(with = "Nullable<String>")]
    pub by: Option<String>,
    #[schemars(with = "Nullable<String>")]
    pub note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RetirementWire {
    id: RetirementId,
    retires: ClaimId,
    #[serde(deserialize_with = "nullable")]
    replaced_by: Option<ClaimId>,
    at: Timestamp,
    #[serde(deserialize_with = "nullable")]
    by: Option<String>,
    #[serde(deserialize_with = "nullable")]
    note: Option<String>,
}

impl<'de> Deserialize<'de> for Retirement {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = RetirementWire::deserialize(d)?;
        Retirement::try_from(wire).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<RetirementWire> for Retirement {
    type Error = String;
    fn try_from(w: RetirementWire) -> Result<Self, String> {
        let r = Retirement {
            id: w.id,
            retires: w.retires,
            replaced_by: w.replaced_by,
            at: w.at,
            by: w.by,
            note: w.note,
        };
        r.validate().map_err(|e| e.message)?;
        Ok(r)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RetirementBody<'a> {
    retires: &'a ClaimId,
    replaced_by: &'a Option<ClaimId>,
    at: &'a Timestamp,
    by: &'a Option<String>,
    note: &'a Option<String>,
}

impl Retirement {
    /// Build a retirement and derive its id. `by` and `note` are trimmed
    /// (WS, section 4.1) and become `null` when empty. The result must then
    /// pass the same rules as a stored retirement ([`Retirement::validate`]).
    pub fn new(
        retires: ClaimId,
        replaced_by: Option<ClaimId>,
        at: Timestamp,
        by: Option<String>,
        note: Option<String>,
    ) -> Result<Retirement, Error> {
        let mut r = Retirement {
            id: RetirementId(String::new()),
            retires,
            replaced_by,
            at,
            by: normalize_free_text(by),
            note: normalize_free_text(note),
        };
        r.validate_body()?;
        r.id = r.derive_id()?;
        Ok(r)
    }

    /// The id derived from the body (section 3.5).
    pub fn derive_id(&self) -> Result<RetirementId, Error> {
        let body = RetirementBody {
            retires: &self.retires,
            replaced_by: &self.replaced_by,
            at: &self.at,
            by: &self.by,
            note: &self.note,
        };
        Ok(RetirementId(canon::derive_id(
            RecordKind::Retirement,
            &body,
        )?))
    }

    /// The rules of section 3.4 and the id. Deserialization runs this.
    pub fn validate(&self) -> Result<(), Error> {
        self.validate_body()?;
        check_derived(self.id.as_str(), self.derive_id()?.as_str())
    }

    /// Every rule except the id. [`Retirement::new`] runs this.
    fn validate_body(&self) -> Result<(), Error> {
        if self.replaced_by.as_ref() == Some(&self.retires) {
            return Err(Error::invalid_argument(format!(
                "claim {} cannot be replaced by itself",
                self.retires
            )));
        }
        check_free_text("retirement", "by", &self.by)?;
        check_free_text("retirement", "note", &self.note)
    }

    pub fn canonical(&self) -> Result<Vec<u8>, Error> {
        canon::to_canonical(self)
    }
}

/// Map a model validation or deserialization failure to the code of the
/// context it was read in: `store_corrupt` for the store, `invalid_json` for
/// standard input.
pub fn recode(err: Error, code: Code) -> Error {
    err.recode(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(s: &str) -> Locator {
        Locator::new(s).unwrap()
    }

    #[test]
    fn locator_grammar() {
        for ok in [
            "file:README.md",
            "file:docs/a b/c.md",
            "file:.hidden/x",
            "file:a/...",
            "repo:api//src/main.rs",
            "repo:0-x//a",
            "archctl:model",
            "x-1:anything goes: here//",
        ] {
            assert!(Locator::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "README.md",
            "file:",
            "file:/abs",
            "file:a//b",
            "file:a/",
            "file:./a",
            "file:a/../b",
            "file:a\tb",
            "repo:api/src",
            "repo:Api//src",
            "repo:-a//src",
            "repo:api//",
            "Ext:x",
            "1x:y",
            "ext:",
            "ext:a\u{7f}",
        ] {
            let e = Locator::new(bad).unwrap_err();
            assert_eq!(e.code, Code::InvalidAnchor, "{bad}");
        }
        assert_eq!(
            loc("repo:api//a/b").parts(),
            LocatorParts::Repo {
                alias: "api",
                path: "a/b"
            }
        );
        assert_eq!(loc("ext:x:y").scheme(), "ext");
    }

    #[test]
    fn selector_rules() {
        let ok = |j: &str| serde_json::from_str::<Selector>(j).unwrap();
        let bad = |j: &str| serde_json::from_str::<Selector>(j).unwrap_err();
        assert!(ok("{}").is_empty());
        ok(r#"{"quote":" x ","prefix":"","suffix":"s","pos":3,"symbol":"A.b"}"#);
        ok(r#"{"key":"k"}"#);
        bad(r#"{"quote":null}"#);
        bad(r#"{"quote":"   "}"#);
        bad(r#"{"quote":""}"#);
        bad(r#"{"prefix":"a"}"#);
        bad(r#"{"pos":1}"#);
        bad(r#"{"symbol":""}"#);
        bad(r#"{"key":""}"#);
        bad(r#"{"other":"x"}"#);
        bad(r#"{"quote":"x","pos":-1}"#);
        bad(r#"{"quote":"x","pos":1.5}"#);
        bad(r#"{"quote":"x","pos":9007199254740992}"#);
        ok(r#"{"quote":"x","pos":9007199254740991}"#);
        let p48: String = "é".repeat(48);
        ok(&format!(r#"{{"quote":"x","prefix":"{p48}"}}"#));
        bad(&format!(r#"{{"quote":"x","prefix":"{p48}a"}}"#));
        let s = Selector {
            quote: Some("q".into()),
            pos: Some(4),
            ..Selector::default()
        };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"quote":"q","pos":4}"#
        );
    }

    #[test]
    fn allowed_keys() {
        use SelectorKey::*;
        let sel = |j: &str| serde_json::from_str::<Selector>(j).unwrap();
        assert!(sel("{}").is_allowed_by(&[]));
        assert_eq!(
            sel(r#"{"quote":"q","prefix":"p","pos":1}"#).disallowed_key(&[Quote]),
            None
        );
        assert_eq!(
            sel(r#"{"quote":"q","symbol":"s"}"#).disallowed_key(&[Quote]),
            Some(Symbol)
        );
        assert_eq!(
            sel(r#"{"quote":"q","symbol":"s"}"#).disallowed_key(&[Symbol]),
            Some(Quote)
        );
        assert_eq!(
            sel(r#"{"key":"k"}"#).disallowed_key(&[Quote, Symbol]),
            Some(Key)
        );
        assert!(sel(r#"{"key":"k"}"#).is_allowed_by(&[Key, Quote]));
    }

    #[test]
    fn compact_anchor_syntax() {
        let a = parse_compact("file:src/a.rs#Cache.get::fn get(").unwrap();
        assert_eq!(a.loc, "file:src/a.rs");
        assert_eq!(a.sel.symbol.as_deref(), Some("Cache.get"));
        assert_eq!(a.sel.quote.as_deref(), Some("fn get("));

        let a = parse_compact("file:README.md:: spaced quote # with hash::and colons ").unwrap();
        assert_eq!(a.loc, "file:README.md");
        assert_eq!(a.sel.symbol, None);
        assert_eq!(
            a.sel.quote.as_deref(),
            Some(" spaced quote # with hash::and colons ")
        );

        let a = parse_compact("repo:api//src/lib.rs#run").unwrap();
        assert_eq!(a.loc, "repo:api//src/lib.rs");
        assert_eq!(a.sel.symbol.as_deref(), Some("run"));

        let a = parse_compact("file:docs/a.md").unwrap();
        assert!(a.sel.is_empty());

        let a = parse_compact("archctl:model#x::y").unwrap();
        assert_eq!(a.loc, "archctl:model#x::y");
        assert!(a.sel.is_empty());

        for bad in [
            "file:a.md::",
            "file:a.md::  ",
            "file:a.md#",
            "file:#x",
            "nope",
            "file:a/../b#x",
        ] {
            assert_eq!(
                parse_compact(bad).unwrap_err().code,
                Code::InvalidAnchor,
                "{bad}"
            );
        }
    }

    #[test]
    fn fingerprint_format() {
        let fp = Fingerprint::new("b3:prose-v1:0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(fp.profile(), "prose-v1");
        assert_eq!(fp.hex(), "0123456789abcdef0123456789abcdef");
        assert!(Fingerprint::new(format!("b3:x:{}", "a".repeat(64))).is_ok());
        for bad in [
            "b3:prose-v1:0123456789abcdef0123456789abcde",
            "b3:prose-v1:0123456789ABCDEF0123456789abcdef",
            "b3:Prose:0123456789abcdef0123456789abcdef",
            "b2:prose-v1:0123456789abcdef0123456789abcdef",
            "b3::0123456789abcdef0123456789abcdef",
        ] {
            assert!(Fingerprint::new(bad).is_err(), "{bad}");
        }
        assert!(Fingerprint::new(format!("b3:x:{}", "a".repeat(65))).is_err());
        let fp = Fingerprint::from_digest("ast-v1", &[0xab; 16]).unwrap();
        assert_eq!(fp.as_str(), format!("b3:ast-v1:{}", "ab".repeat(16)));
    }

    #[test]
    fn timestamps() {
        let t = Timestamp::parse("2026-11-20T18:04:11Z").unwrap();
        assert_eq!(t.to_string(), "2026-11-20T18:04:11Z");
        assert!(Timestamp::parse("9999-12-31T23:59:59Z").is_ok());
        for bad in [
            "2026-11-20T18:04:11.000Z",
            "2026-11-20T18:04:11+00:00",
            "2026-11-20 18:04:11Z",
            "2026-02-30T00:00:00Z",
            "2026-11-20T24:00:00Z",
            "2026-11-20T23:59:60Z",
            "2026-11-20T18:04:11z",
            "+2026-11-20T18:04:1Z",
        ] {
            assert!(Timestamp::parse(bad).is_err(), "{bad}");
        }
        let now: jiff::Timestamp = "2026-11-20T18:04:11.9+02:00".parse().unwrap();
        assert_eq!(
            Timestamp::from_instant(now).unwrap().to_string(),
            "2026-11-20T16:04:11Z"
        );
    }

    fn sample_claim() -> Claim {
        Claim::new(
            "  Retries are capped at 5. ",
            parse_compact("file:docs/retry.md::Retries are capped at 5.").unwrap(),
            vec![parse_compact("file:src/retry.rs#MAX_ATTEMPTS").unwrap()],
            Some("P90D".into()),
            Some(String::new()),
        )
        .unwrap()
    }

    #[test]
    fn claim_round_trip() {
        let c = sample_claim();
        assert_eq!(c.statement, "Retries are capped at 5.");
        assert_eq!(c.note, None);
        let bytes = c.canonical().unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.starts_with(
            r#"{"evidence":[{"loc":"file:src/retry.rs","sel":{"symbol":"MAX_ATTEMPTS"}}],"id":"c1-"#
        ));
        assert!(text.contains(r#""note":null"#));
        let back: Claim = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, c);

        // A missing nullable key, an unknown key, and a wrong id are errors.
        let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        v.as_object_mut().unwrap().remove("note");
        assert!(serde_json::from_value::<Claim>(v.clone()).is_err());
        v["note"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<Claim>(v.clone()).is_ok());
        v["extra"] = 1.into();
        assert!(serde_json::from_value::<Claim>(v.clone()).is_err());
        v.as_object_mut().unwrap().remove("extra");
        v["statement"] = "Retries are capped at 6.".into();
        assert!(serde_json::from_value::<Claim>(v.clone()).is_err());
        v["statement"] = " x".into();
        assert!(serde_json::from_value::<Claim>(v).is_err());

        assert_eq!(
            Claim::new(" \n", c.subject.clone(), vec![], None, None)
                .unwrap_err()
                .code,
            Code::InvalidArgument
        );
        assert_eq!(
            Claim::new("x", c.subject.clone(), vec![], Some("P".into()), None)
                .unwrap_err()
                .code,
            Code::InvalidDuration
        );
    }

    #[test]
    fn mark_and_retirement() {
        let c = sample_claim();
        let at = Timestamp::parse("2026-11-20T18:04:11Z").unwrap();
        let fp = Fingerprint::new("b3:prose-v1:0123456789abcdef0123456789abcdef").unwrap();
        let sides: Vec<MarkSide> = c
            .sides()
            .map(|a| MarkSide {
                loc: a.loc.clone(),
                fp: fp.clone(),
            })
            .collect();
        let m = Mark::new(c.id.clone(), at, Some(String::new()), sides.clone(), None).unwrap();
        assert_eq!(m.by, None);
        m.check_against(&c).unwrap();
        let back: Mark = serde_json::from_slice(&m.canonical().unwrap()).unwrap();
        assert_eq!(back, m);
        let short = Mark::new(c.id.clone(), at, None, sides[..1].to_vec(), None).unwrap();
        assert!(short.check_against(&c).is_err());

        let r = Retirement::new(c.id.clone(), None, at, Some("n".into()), None).unwrap();
        let text = String::from_utf8(r.canonical().unwrap()).unwrap();
        assert!(text.contains(r#""replacedBy":null"#), "{text}");
        let back: Retirement = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r);
        assert!(r.id.as_str().starts_with("r1-"));
    }

    #[test]
    fn constructors_reject_what_readers_reject() {
        let c = sample_claim();
        let at = Timestamp::parse("2026-11-20T18:04:11Z").unwrap();
        let quoteless = Anchor::new(
            loc("file:a.md"),
            Selector {
                prefix: Some("p".into()),
                ..Selector::default()
            },
        );
        let e = Claim::new("x", quoteless, vec![], None, None).unwrap_err();
        assert_eq!(e.code, Code::InvalidAnchor);
        let long = Anchor::new(
            loc("file:a.md"),
            Selector::quote("é".repeat(MAX_QUOTE_CHARS + 1)),
        );
        let e = Claim::new("x", c.subject.clone(), vec![long], None, None).unwrap_err();
        assert_eq!(e.code, Code::InvalidAnchor);
        let at_limit = Anchor::new(
            loc("file:a.md"),
            Selector::quote("é".repeat(MAX_QUOTE_CHARS)),
        );
        Claim::new("x", at_limit, vec![], None, None).unwrap();

        let e = Mark::new(c.id.clone(), at, None, vec![], None).unwrap_err();
        assert_eq!(e.code, Code::InvalidArgument);

        let e = Retirement::new(c.id.clone(), Some(c.id.clone()), at, None, None).unwrap_err();
        assert_eq!(e.code, Code::InvalidArgument);
        let r = Retirement::new(c.id.clone(), None, at, None, None).unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&r.canonical().unwrap()).unwrap();
        v["replacedBy"] = c.id.as_str().into();
        // Re-derive the id so that only the replacedBy rule fails.
        let mut body = v.clone();
        body.as_object_mut().unwrap().remove("id");
        v["id"] = canon::derive_id(RecordKind::Retirement, &body)
            .unwrap()
            .into();
        let e = serde_json::from_value::<Retirement>(v).unwrap_err();
        assert!(e.to_string().contains("replaced by itself"), "{e}");
    }

    #[test]
    fn free_text_is_trimmed_and_checked_on_read() {
        let c = sample_claim();
        let at = Timestamp::parse("2026-11-20T18:04:11Z").unwrap();
        let ws = "\u{a0}\u{2028} \t";
        let claim = Claim::new(
            "x",
            c.subject.clone(),
            vec![],
            None,
            Some(format!("{ws}n{ws}")),
        )
        .unwrap();
        assert_eq!(claim.note.as_deref(), Some("n"));
        let claim = Claim::new("x", c.subject.clone(), vec![], None, Some(ws.into())).unwrap();
        assert_eq!(claim.note, None);
        let sides = vec![MarkSide {
            loc: c.subject.loc.clone(),
            fp: Fingerprint::new("b3:prose-v1:0123456789abcdef0123456789abcdef").unwrap(),
        }];
        let m = Mark::new(c.id.clone(), at, Some(" a ".into()), sides, Some(ws.into())).unwrap();
        assert_eq!((m.by.as_deref(), m.note.as_deref()), (Some("a"), None));
        let r = Retirement::new(
            c.id.clone(),
            None,
            at,
            Some(ws.into()),
            Some("\u{feff}".into()),
        )
        .unwrap();
        // U+FEFF is not WS: it stays.
        assert_eq!(
            (r.by.as_deref(), r.note.as_deref()),
            (None, Some("\u{feff}"))
        );

        // A stored value that is not in the normalized form is invalid on
        // read, even with an id derived from it.
        let reread = |kind: RecordKind, mut v: serde_json::Value, key: &str, bad: &str| {
            v[key] = bad.into();
            let mut body = v.clone();
            body.as_object_mut().unwrap().remove("id");
            v["id"] = canon::derive_id(kind, &body).unwrap().into();
            match kind {
                RecordKind::Claim => serde_json::from_value::<Claim>(v).map(|_| ()),
                RecordKind::Mark => serde_json::from_value::<Mark>(v).map(|_| ()),
                RecordKind::Retirement => serde_json::from_value::<Retirement>(v).map(|_| ()),
            }
        };
        let json = |b: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&b).unwrap();
        for bad in ["", " n", "n\u{a0}", "\u{2028}"] {
            assert!(
                reread(
                    RecordKind::Claim,
                    json(claim.canonical().unwrap()),
                    "note",
                    bad
                )
                .is_err()
            );
            assert!(reread(RecordKind::Mark, json(m.canonical().unwrap()), "by", bad).is_err());
            assert!(reread(RecordKind::Mark, json(m.canonical().unwrap()), "note", bad).is_err());
            assert!(
                reread(
                    RecordKind::Retirement,
                    json(r.canonical().unwrap()),
                    "by",
                    bad
                )
                .is_err()
            );
            assert!(
                reread(
                    RecordKind::Retirement,
                    json(r.canonical().unwrap()),
                    "note",
                    bad
                )
                .is_err()
            );
        }
        assert!(reread(RecordKind::Mark, json(m.canonical().unwrap()), "by", "n").is_ok());
    }

    /// Property: whatever a constructor accepts reads back from its
    /// canonical bytes as an equal record with the same bytes. The inputs
    /// mix valid and invalid values of every field.
    #[test]
    fn constructed_records_round_trip() {
        let mut seed: u64 = 1;
        let mut pick = |n: usize| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as usize) % n
        };
        let texts: Vec<Option<String>> = vec![
            None,
            Some(String::new()),
            Some(" ".into()),
            Some("\u{a0}x\u{2028}".into()),
            Some("x".into()),
            Some("\u{feff}".into()),
            Some("e\u{301} y".into()),
        ];
        let selectors: Vec<Selector> = [
            "{}",
            r#"{"quote":"q"}"#,
            r#"{"quote":" q ","prefix":"","suffix":"s","pos":3}"#,
            r#"{"symbol":"A.b","quote":"x"}"#,
            r#"{"key":"k"}"#,
        ]
        .iter()
        .map(|j| serde_json::from_str(j).unwrap())
        .chain([
            Selector {
                prefix: Some("p".into()),
                ..Selector::default()
            },
            Selector {
                pos: Some(1),
                ..Selector::default()
            },
            Selector::quote(" \n"),
            Selector::quote("q".repeat(MAX_QUOTE_CHARS + 1)),
            Selector {
                quote: Some("q".into()),
                pos: Some(MAX_POS + 1),
                ..Selector::default()
            },
            Selector {
                quote: Some("q".into()),
                suffix: Some("s".repeat(MAX_CONTEXT_CHARS + 1)),
                ..Selector::default()
            },
            Selector {
                symbol: Some(String::new()),
                ..Selector::default()
            },
        ])
        .collect();
        let locs = ["file:a.md", "repo:x//b.rs", "ext:o"].map(loc);
        let statements = ["x", "  y\u{2028}", " ", ""];
        let rechecks = [None, Some("P90D"), Some("P"), Some("PT0S")];
        let claim_ids: Vec<ClaimId> = (0..3)
            .map(|i| {
                Claim::new(
                    &format!("c{i}"),
                    Anchor::new(loc("file:a"), Selector::empty()),
                    vec![],
                    None,
                    None,
                )
                .unwrap()
                .id
            })
            .collect();
        let fp = Fingerprint::new("b3:prose-v1:0123456789abcdef0123456789abcdef").unwrap();
        let at = Timestamp::parse("2026-11-20T18:04:11Z").unwrap();
        let (mut ok, mut err) = (0, 0);

        for _ in 0..4000 {
            let anchor = |pick: &mut dyn FnMut(usize) -> usize| {
                Anchor::new(
                    locs[pick(locs.len())].clone(),
                    selectors[pick(selectors.len())].clone(),
                )
            };
            let subject = anchor(&mut pick);
            let evidence: Vec<Anchor> = (0..pick(3)).map(|_| anchor(&mut pick)).collect();
            let claim = Claim::new(
                statements[pick(statements.len())],
                subject,
                evidence,
                rechecks[pick(rechecks.len())].map(str::to_owned),
                texts[pick(texts.len())].clone(),
            );
            let sides: Vec<MarkSide> = (0..pick(3))
                .map(|_| MarkSide {
                    loc: locs[pick(locs.len())].clone(),
                    fp: fp.clone(),
                })
                .collect();
            let mark = Mark::new(
                claim_ids[pick(3)].clone(),
                at,
                texts[pick(texts.len())].clone(),
                sides,
                texts[pick(texts.len())].clone(),
            );
            let replaced = [None, Some(claim_ids[pick(3)].clone())][pick(2)].clone();
            let retirement = Retirement::new(
                claim_ids[pick(3)].clone(),
                replaced,
                at,
                texts[pick(texts.len())].clone(),
                texts[pick(texts.len())].clone(),
            );
            match claim {
                Ok(r) => {
                    let bytes = r.canonical().unwrap();
                    let back: Claim =
                        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{r:?}: {e}"));
                    assert_eq!((&back, back.canonical().unwrap()), (&r, bytes));
                    ok += 1;
                }
                Err(_) => err += 1,
            }
            match mark {
                Ok(r) => {
                    let bytes = r.canonical().unwrap();
                    let back: Mark =
                        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{r:?}: {e}"));
                    assert_eq!((&back, back.canonical().unwrap()), (&r, bytes));
                    ok += 1;
                }
                Err(_) => err += 1,
            }
            match retirement {
                Ok(r) => {
                    let bytes = r.canonical().unwrap();
                    let back: Retirement =
                        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{r:?}: {e}"));
                    assert_eq!((&back, back.canonical().unwrap()), (&r, bytes));
                    ok += 1;
                }
                Err(_) => err += 1,
            }
        }
        // The inputs exercise both outcomes.
        assert!(ok > 1000 && err > 1000, "ok {ok}, err {err}");
    }
}
