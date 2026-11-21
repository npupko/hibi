//! The `Provider` trait, the provider protocol types and the `Providers`
//! registry (spec sections 2.5 and 13).
//!
//! `ProviderInfo`, `Unit`, `Rendered` and `ProviderError` are the Rust forms
//! of the protocol objects of section 13.3 and serialize to exactly those
//! shapes. The wire envelope and request types are in [`protocol`].

#[cfg(feature = "command")]
pub mod command;
#[cfg(feature = "file")]
pub mod file;
pub mod protocol;

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::{Anchor, Fingerprint, Locator, Selector, SelectorKey};

pub use protocol::*;

/// A component that resolves locators to units, text and fingerprints
/// (section 2.5). `FileProvider` and `CommandProvider` implement it, and the
/// engine uses nothing else from them.
pub trait Provider {
    /// What the provider is and accepts. A provider that starts lazily
    /// (`CommandProvider`) starts here on first use; a start or `hello`
    /// failure is the error `failed`, which the engine maps to
    /// `provider_failed` (section 9.2).
    fn info(&mut self) -> Result<&ProviderInfo, ProviderError>;
    /// One result per anchor, in order.
    fn resolve(&mut self, anchors: &[Anchor]) -> Vec<Result<Vec<Unit>, ProviderError>>;
    fn units(&mut self, loc: &Locator, kind: Option<&str>) -> Result<Vec<Unit>, ProviderError>;
    fn render(&mut self, loc: &Locator, unit: &str) -> Result<Rendered, ProviderError>;
}

// ---------------------------------------------------------------------------
// hello result
// ---------------------------------------------------------------------------

/// A name and a version, as in `hello` (`client`, `provider`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NameVersion {
    pub name: String,
    pub version: String,
}

/// Optional provider methods. A missing key is `false`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    #[serde(default)]
    pub units: bool,
    #[serde(default)]
    pub render: bool,
    #[serde(default)]
    pub batch: bool,
}

/// The `hello` result (section 13.3): what a provider is and accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderInfo {
    /// The protocol version chosen from the client's list.
    pub protocol: u32,
    pub provider: NameVersion,
    /// Schemes this process serves.
    pub schemes: Vec<String>,
    /// Selector keys it accepts, from `quote`, `symbol`, `key`. `quote`
    /// includes `prefix`, `suffix` and `pos`. `{}` is always accepted. See
    /// [`Selector::disallowed_key`].
    pub selectors: Vec<SelectorKey>,
    /// Unit kinds it can return. Informational.
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
}

impl ProviderInfo {
    /// True when `sel` uses only keys this provider allows (section 2.5).
    pub fn allows(&self, sel: &Selector) -> bool {
        sel.is_allowed_by(&self.selectors)
    }
}

// ---------------------------------------------------------------------------
// Unit, Range, Refuse, Rendered
// ---------------------------------------------------------------------------

/// A unit's place in a text resource (section 4.2): UTF-8 byte offsets,
/// `start` inclusive, `end` exclusive, 1-based lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub end_line: usize,
}

/// Why a resolution must not be used to create a claim or replace a side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Refuse {
    QuoteSpansUnits,
    QuoteInComment,
    /// The anchored text is identical, with identical in-unit windows, to
    /// another place in the same container (section 6.7).
    QuoteNotDistinguishable,
}

impl Refuse {
    pub fn as_str(self) -> &'static str {
        match self {
            Refuse::QuoteSpansUnits => "quote_spans_units",
            Refuse::QuoteInComment => "quote_in_comment",
            Refuse::QuoteNotDistinguishable => "quote_not_distinguishable",
        }
    }
}

/// How a unit returned without `approx` was resolved, when not by its full
/// selector (section 6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Identity {
    /// By its symbol alone: the selector's stored quote is no longer in the
    /// resource.
    Symbol,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The unit object of section 13.3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Unit {
    /// Unique within the resource. Stable while the unit is the same unit.
    pub key: String,
    pub kind: String,
    /// Key of the enclosing unit. Absent on the wire means `null`.
    #[serde(default)]
    pub parent: Option<String>,
    /// One line for people.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Omitted when the resource is not text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    pub fp: Fingerprint,
    /// A selector that, sent to `resolve` with the same `loc`, returns
    /// exactly this unit now.
    pub sel: Selector,
    /// The unit's content. In `resolve` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// In `resolve`: the unit is a candidate, not the unit the selector was
    /// made for.
    #[serde(default, skip_serializing_if = "is_false")]
    pub approx: bool,
    /// In `resolve`: `symbol` when the unit is resolved by its symbol alone
    /// because the selector's stored quote is no longer in the resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity>,
    /// In `resolve`: must not be used to create a claim or replace a side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refuse: Option<Refuse>,
}

/// The `render` result: the unit as text for a person or a judge, and its
/// context from outermost to innermost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Rendered {
    pub text: String,
    pub context: Vec<String>,
}

/// Check the units of one answer by the rules of section 2.5 that are not
/// already enforced by deserialization: every `sel` uses only keys `info`
/// allows, and keys are unique within the answer. (`fp` and `sel` validity
/// are enforced when a [`Unit`] is deserialized or built.) Returns a message
/// for the `provider_error` of the affected side or resource.
pub fn check_units(info: &ProviderInfo, units: &[Unit]) -> Result<(), String> {
    let mut keys = HashSet::new();
    for u in units {
        if let Some(k) = u.sel.disallowed_key(&info.selectors) {
            return Err(format!(
                "unit {:?} has a selector with key {k}, which the provider does not allow",
                u.key
            ));
        }
        if !keys.insert(u.key.as_str()) {
            return Err(format!("two units have the key {:?}", u.key));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ProviderError (section 13.4)
// ---------------------------------------------------------------------------

/// The closed provider error-code list of section 13.4, plus `failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderErrorCode {
    InvalidRequest,
    UnknownMethod,
    UnsupportedProtocol,
    InvalidLocator,
    InvalidSelector,
    NotFound,
    NotText,
    TooLarge,
    RepoNotFound,
    RepoNotAllowed,
    UnknownUnit,
    Unavailable,
    Internal,
    /// Produced only by `CommandProvider`, never on the wire: the process
    /// could not be started, timed out, broke the protocol or was killed.
    Failed,
}

impl ProviderErrorCode {
    /// The codes that may appear on the wire, in the order of section 13.4.
    pub const WIRE: [ProviderErrorCode; 13] = [
        ProviderErrorCode::InvalidRequest,
        ProviderErrorCode::UnknownMethod,
        ProviderErrorCode::UnsupportedProtocol,
        ProviderErrorCode::InvalidLocator,
        ProviderErrorCode::InvalidSelector,
        ProviderErrorCode::NotFound,
        ProviderErrorCode::NotText,
        ProviderErrorCode::TooLarge,
        ProviderErrorCode::RepoNotFound,
        ProviderErrorCode::RepoNotAllowed,
        ProviderErrorCode::UnknownUnit,
        ProviderErrorCode::Unavailable,
        ProviderErrorCode::Internal,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderErrorCode::InvalidRequest => "invalid_request",
            ProviderErrorCode::UnknownMethod => "unknown_method",
            ProviderErrorCode::UnsupportedProtocol => "unsupported_protocol",
            ProviderErrorCode::InvalidLocator => "invalid_locator",
            ProviderErrorCode::InvalidSelector => "invalid_selector",
            ProviderErrorCode::NotFound => "not_found",
            ProviderErrorCode::NotText => "not_text",
            ProviderErrorCode::TooLarge => "too_large",
            ProviderErrorCode::RepoNotFound => "repo_not_found",
            ProviderErrorCode::RepoNotAllowed => "repo_not_allowed",
            ProviderErrorCode::UnknownUnit => "unknown_unit",
            ProviderErrorCode::Unavailable => "unavailable",
            ProviderErrorCode::Internal => "internal",
            ProviderErrorCode::Failed => "failed",
        }
    }

    /// Read a wire code. An unknown code, and `failed` (which is not a wire
    /// code), are `internal` (section 13.4).
    pub fn from_wire(s: &str) -> ProviderErrorCode {
        ProviderErrorCode::WIRE
            .into_iter()
            .find(|c| c.as_str() == s)
            .unwrap_or(ProviderErrorCode::Internal)
    }
}

impl fmt::Display for ProviderErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ProviderErrorCode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProviderErrorCode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(ProviderErrorCode::from_wire(&s))
    }
}

impl JsonSchema for ProviderErrorCode {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ProviderErrorCode".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let names: Vec<&str> = ProviderErrorCode::WIRE.iter().map(|c| c.as_str()).collect();
        schemars::json_schema!({
            "type": "string",
            "description": "Closed list of section 13.4. A reader treats an unknown code as internal.",
            "enum": names
        })
    }
}

/// A provider error: `{"code", "message"}` (section 13.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ProviderError {
    pub code: ProviderErrorCode,
    pub message: String,
}

impl ProviderError {
    pub fn new(code: ProviderErrorCode, message: impl Into<String>) -> Self {
        ProviderError {
            code,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        ProviderError::new(ProviderErrorCode::NotFound, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        ProviderError::new(ProviderErrorCode::InvalidRequest, message)
    }

    pub fn failed(message: impl Into<String>) -> Self {
        ProviderError::new(ProviderErrorCode::Failed, message)
    }
}

// ---------------------------------------------------------------------------
// Providers registry
// ---------------------------------------------------------------------------

/// Why no provider can serve a locator (section 9.2, engine rows). A
/// provider that cannot start or fails `hello` is not this: it answers every
/// request with the error `failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoProvider {
    /// The scheme is not `file`, not `repo`, and not listed in `hibi.toml`.
    SchemeNotDeclared,
    /// The scheme is listed but the user config has no command for it.
    ProviderNotRegistered,
}

impl NoProvider {
    /// The side reason of section 9.2.
    pub fn reason(self) -> &'static str {
        match self {
            NoProvider::SchemeNotDeclared => "scheme_not_declared",
            NoProvider::ProviderNotRegistered => "provider_not_registered",
        }
    }
}

enum Slot {
    Ready(usize),
    Unregistered,
}

/// Maps a scheme to a boxed [`Provider`] (section 2.5). One provider may
/// serve several schemes (the file provider serves `file` and `repo`).
#[derive(Default)]
pub struct Providers {
    providers: Vec<Box<dyn Provider>>,
    schemes: BTreeMap<String, Slot>,
}

impl Providers {
    pub fn new() -> Self {
        Providers::default()
    }

    /// Register `provider` for each of `schemes`. A scheme registered or
    /// declared earlier is replaced.
    pub fn register<S: AsRef<str>>(&mut self, schemes: &[S], provider: Box<dyn Provider>) {
        let index = self.providers.len();
        self.providers.push(provider);
        for s in schemes {
            self.schemes
                .insert(s.as_ref().to_owned(), Slot::Ready(index));
        }
    }

    /// Record that `scheme` is declared in `hibi.toml` but has no command in
    /// the user config. Does nothing if the scheme already has a provider.
    pub fn declare_unregistered(&mut self, scheme: &str) {
        self.schemes
            .entry(scheme.to_owned())
            .or_insert(Slot::Unregistered);
    }

    /// The provider for a locator's scheme, or why there is none.
    pub fn provider_for(&mut self, loc: &Locator) -> Result<&mut dyn Provider, NoProvider> {
        self.provider_for_scheme(loc.scheme())
    }

    /// The provider for a scheme, or why there is none.
    pub fn provider_for_scheme(&mut self, scheme: &str) -> Result<&mut dyn Provider, NoProvider> {
        match self.schemes.get(scheme) {
            Some(Slot::Ready(i)) => Ok(self.providers[*i].as_mut()),
            Some(Slot::Unregistered) => Err(NoProvider::ProviderNotRegistered),
            None => Err(NoProvider::SchemeNotDeclared),
        }
    }

    /// True when the scheme has a registered provider.
    pub fn is_registered(&self, scheme: &str) -> bool {
        matches!(self.schemes.get(scheme), Some(Slot::Ready(_)))
    }

    /// Every scheme the registry knows, registered or declared, in order.
    pub fn schemes(&self) -> impl Iterator<Item = &str> {
        self.schemes.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(ProviderInfo);
    impl Provider for Fake {
        fn info(&mut self) -> Result<&ProviderInfo, ProviderError> {
            Ok(&self.0)
        }
        fn resolve(&mut self, anchors: &[Anchor]) -> Vec<Result<Vec<Unit>, ProviderError>> {
            anchors.iter().map(|_| Ok(vec![])).collect()
        }
        fn units(&mut self, _: &Locator, _: Option<&str>) -> Result<Vec<Unit>, ProviderError> {
            Ok(vec![])
        }
        fn render(&mut self, _: &Locator, _: &str) -> Result<Rendered, ProviderError> {
            Err(ProviderError::new(ProviderErrorCode::UnknownUnit, "x"))
        }
    }

    fn info(name: &str) -> ProviderInfo {
        ProviderInfo {
            protocol: 1,
            provider: NameVersion {
                name: name.into(),
                version: "0".into(),
            },
            schemes: vec![],
            selectors: vec![SelectorKey::Quote, SelectorKey::Symbol],
            kinds: vec![],
            capabilities: Capabilities::default(),
        }
    }

    #[test]
    fn registry() {
        let mut p = Providers::new();
        p.register(&["file", "repo"], Box::new(Fake(info("file"))));
        p.declare_unregistered("archctl");
        p.declare_unregistered("file");
        let loc = |s: &str| Locator::new(s).unwrap();
        assert_eq!(
            p.provider_for(&loc("repo:a//b"))
                .unwrap()
                .info()
                .unwrap()
                .provider
                .name,
            "file"
        );
        assert_eq!(
            p.provider_for(&loc("file:b"))
                .unwrap()
                .info()
                .unwrap()
                .provider
                .name,
            "file"
        );
        assert_eq!(
            p.provider_for(&loc("archctl:m")).err(),
            Some(NoProvider::ProviderNotRegistered)
        );
        assert_eq!(
            p.provider_for(&loc("other:m")).err(),
            Some(NoProvider::SchemeNotDeclared)
        );
        assert_eq!(p.schemes().collect::<Vec<_>>(), ["archctl", "file", "repo"]);
    }

    #[test]
    fn error_codes_on_the_wire() {
        let e: ProviderError =
            serde_json::from_str(r#"{"code":"not_found","message":"m"}"#).unwrap();
        assert_eq!(e.code, ProviderErrorCode::NotFound);
        let e: ProviderError =
            serde_json::from_str(r#"{"code":"weird","message":"m","extra":1}"#).unwrap();
        assert_eq!(e.code, ProviderErrorCode::Internal);
        let e: ProviderError = serde_json::from_str(r#"{"code":"failed","message":"m"}"#).unwrap();
        assert_eq!(e.code, ProviderErrorCode::Internal);
        for c in ProviderErrorCode::WIRE {
            assert_eq!(ProviderErrorCode::from_wire(c.as_str()), c);
        }
    }

    #[test]
    fn unit_shape_and_checks() {
        let json = r#"{"key":"n1","kind":"note","fp":"b3:prose-v1:0123456789abcdef0123456789abcdef","sel":{"key":"n1"},"future":true}"#;
        let u: Unit = serde_json::from_str(json).unwrap();
        assert_eq!(u.parent, None);
        assert!(!u.approx);
        assert_eq!(
            serde_json::to_value(&u).unwrap(),
            serde_json::json!({"key":"n1","kind":"note","parent":null,"fp":"b3:prose-v1:0123456789abcdef0123456789abcdef","sel":{"key":"n1"}})
        );
        // A closed selector: an unknown key inside `sel` is an error.
        let bad = json.replace(r#"{"key":"n1"}"#, r#"{"key":"n1","x":1}"#);
        assert!(serde_json::from_str::<Unit>(&bad).is_err());
        let bad = json.replace("b3:prose-v1:", "b3:prose-v1:z");
        assert!(serde_json::from_str::<Unit>(&bad).is_err());

        let mut i = info("x");
        assert!(check_units(&i, std::slice::from_ref(&u)).is_err());
        i.selectors.push(SelectorKey::Key);
        assert!(check_units(&i, std::slice::from_ref(&u)).is_ok());
        assert!(check_units(&i, &[u.clone(), u]).is_err());
    }

    #[test]
    fn hello_result_defaults() {
        let r: ProviderInfo = serde_json::from_str(
            r#"{"protocol":1,"provider":{"name":"p","version":"1"},"schemes":["ref"],"selectors":["key","quote"],"capabilities":{"units":true}}"#,
        )
        .unwrap();
        assert!(r.capabilities.units && !r.capabilities.render && !r.capabilities.batch);
        assert!(r.kinds.is_empty());
        assert!(serde_json::from_str::<ProviderInfo>(
            r#"{"protocol":1,"provider":{"name":"p","version":"1"},"schemes":["ref"],"selectors":["pos"]}"#
        )
        .is_err());
    }
}
