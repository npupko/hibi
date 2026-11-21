//! States, reasons, warnings and the shared output objects of spec section
//! 15.2.

use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::git::GitStatus;
use crate::model::{Anchor, ClaimId, Fingerprint, MarkId, Timestamp};
use crate::provider::Unit;
use crate::store::{ORPHAN_RECORD, StoreWarning, UNKNOWN_REPLACEMENT};

/// Unit text shown in `cover` items and for approximate sides of `mark` is
/// cut to this many characters (section 15.3).
pub const TEXT_EXCERPT_CHARS: usize = 240;

/// The sentence `status` carries once in `nextNote` (section 15.2).
pub const NEXT_NOTE: &str = "Re-check each statement before running its next command. When a statement no longer holds, do not run it: fix the text or the code, or retire the claim with hibi retire. A fresh claim is listed only because its anchor drifted: the statement still holds, so look at what the anchor points to now, then run its reanchor command.";

/// Schema helper: a field that is always present and may be `null`.
pub(crate) struct Nullable<T>(std::marker::PhantomData<T>);

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

macro_rules! closed_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $text:literal,)* }) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
            JsonSchema,
        )]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($(#[$vmeta])* $variant,)*
        }

        impl $name {
            /// Every value, in the order of the specification.
            pub const ALL: &'static [$name] = &[$($name::$variant,)*];

            /// The name used in JSON.
            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)*
                }
            }

            /// The value with this JSON name.
            pub fn from_name(s: &str) -> Option<$name> {
                $name::ALL.iter().copied().find(|v| v.as_str() == s)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

closed_enum!(
    /// The state of one side (section 9.2).
    SideState {
        Same => "same",
        Changed => "changed",
        Missing => "missing",
        Ambiguous => "ambiguous",
        Unavailable => "unavailable",
    }
);

closed_enum!(
    /// Why a side is not `same` (section 9.2). A closed list.
    Reason {
        SchemeNotDeclared => "scheme_not_declared",
        ProviderNotRegistered => "provider_not_registered",
        ProviderFailed => "provider_failed",
        ResourceNotFound => "resource_not_found",
        NotText => "not_text",
        TooLarge => "too_large",
        RepoNotFound => "repo_not_found",
        RepoNotAllowed => "repo_not_allowed",
        ProviderError => "provider_error",
        SelectorNotFound => "selector_not_found",
        SeveralUnits => "several_units",
        NoMark => "no_mark",
        FingerprintDiffers => "fingerprint_differs",
        ProfileMismatch => "profile_mismatch",
    }
);

closed_enum!(
    /// The state of a claim (section 9.3).
    ClaimState {
        Fresh => "fresh",
        Stale => "stale",
        Expired => "expired",
        Retired => "retired",
    }
);

closed_enum!(
    /// The closed warning-code list of section 15.1.
    WarningCode {
        OrphanRecord => "orphan_record",
        UnknownReplacement => "unknown_replacement",
        CacheUnavailable => "cache_unavailable",
        GitTooOld => "git_too_old",
        UnknownKind => "unknown_kind",
    }
);

/// What the author can do about a side that cannot be evaluated, by its
/// reason (section 9.2): the cause or the limit, and the way out. `None`
/// for a reason that does not make a side unavailable in a write.
pub fn unavailable_remediation(reason: Reason) -> Option<&'static str> {
    Some(match reason {
        Reason::SchemeNotDeclared
        | Reason::ProviderNotRegistered
        | Reason::ProviderFailed
        | Reason::RepoNotAllowed
        | Reason::RepoNotFound => "hibi providers",
        Reason::TooLarge => {
            "The resource is over the size limit: 8 MiB, or 100,000 units in one resource. Anchor a smaller file; for a claim that exists, retire it; in a scope, add an exclusion for the resource."
        }
        Reason::NotText => {
            "The resource is not UTF-8 text, so it has no units. Anchor a text file; for a claim that exists, retire it; in a scope, add an exclusion for the resource."
        }
        Reason::ProviderError => {
            "The provider gave an answer hibi cannot use. Run hibi providers --check <scheme> to see what it gets wrong; for a claim that exists, retire it if the resource is gone for good."
        }
        _ => return None,
    })
}

/// One entry of `warnings` in the JSON envelope (section 15.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Warning {
    pub code: WarningCode,
    pub message: String,
}

impl Warning {
    pub fn new(code: WarningCode, message: impl Into<String>) -> Self {
        Warning {
            code,
            message: message.into(),
        }
    }

    /// A store warning as a run warning. A store that reports a code
    /// outside section 12.3 breaks the `Store` contract: `internal`.
    pub fn from_store(w: &StoreWarning) -> Result<Warning, Error> {
        let code = match w.code.as_str() {
            ORPHAN_RECORD => WarningCode::OrphanRecord,
            UNKNOWN_REPLACEMENT => WarningCode::UnknownReplacement,
            other => {
                return Err(Error::internal(format!(
                    "the store reported warning code {other:?}, which is not in the closed list"
                )));
            }
        };
        Ok(Warning::new(code, w.message.clone()))
    }

    /// The warnings of a loaded ledger, in the store's order.
    pub fn from_store_all(ws: &[StoreWarning]) -> Result<Vec<Warning>, Error> {
        ws.iter().map(Warning::from_store).collect()
    }

    /// `git_too_old` when the version check found a git older than
    /// `MIN_GIT`; `None` otherwise (section 10.4).
    pub fn git_too_old(status: &GitStatus) -> Option<Warning> {
        status
            .too_old_message()
            .map(|m| Warning::new(WarningCode::GitTooOld, m))
    }
}

#[cfg(feature = "cache")]
impl From<&crate::cache::Unavailable> for Warning {
    fn from(u: &crate::cache::Unavailable) -> Self {
        Warning::new(WarningCode::CacheUnavailable, u.message.clone())
    }
}

/// `subject` for side 0, `evidence` for the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Subject,
    Evidence,
}

impl Role {
    pub fn of(index: usize) -> Role {
        if index == 0 {
            Role::Subject
        } else {
            Role::Evidence
        }
    }
}

/// UnitRef of section 15.2. `label` is `null` when the provider gave none;
/// `line` and `endLine` are `null` without a range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnitRef {
    pub key: String,
    pub kind: String,
    #[schemars(with = "Nullable<String>")]
    pub label: Option<String>,
    #[schemars(with = "Nullable<usize>")]
    pub line: Option<usize>,
    #[schemars(with = "Nullable<usize>")]
    pub end_line: Option<usize>,
}

impl From<&Unit> for UnitRef {
    fn from(u: &Unit) -> Self {
        UnitRef {
            key: u.key.clone(),
            kind: u.kind.clone(),
            label: u.label.clone(),
            line: u.range.map(|r| r.line),
            end_line: u.range.map(|r| r.end_line),
        }
    }
}

/// Side of section 15.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Side {
    /// 0 for the subject.
    pub index: usize,
    pub role: Role,
    /// The anchor as stored.
    pub anchor: Anchor,
    pub state: SideState,
    #[schemars(with = "Nullable<Reason>")]
    pub reason: Option<Reason>,
    pub approx: bool,
    #[schemars(with = "Nullable<UnitRef>")]
    pub unit: Option<UnitRef>,
    /// The current fingerprint.
    #[schemars(with = "Nullable<Fingerprint>")]
    pub fp: Option<Fingerprint>,
    /// The fingerprint in the reference mark.
    #[schemars(with = "Nullable<Fingerprint>")]
    pub marked_fp: Option<Fingerprint>,
    /// For a side that cannot be evaluated: what the limit or the cause is
    /// and what the author can do ([`unavailable_remediation`]). `null` for
    /// every other side; a side that changed, is missing or is ambiguous
    /// is answered by the claim's `next`.
    #[schemars(with = "Nullable<String>")]
    pub remediation: Option<String>,
    /// Present only when `state` is `ambiguous`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<UnitRef>>,
    /// The current unit text. Present only with `--detail`, and for an
    /// approximate side in the output of `mark` (cut to 240 characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// ClaimView of section 15.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimView {
    pub id: ClaimId,
    pub statement: String,
    #[schemars(with = "Nullable<String>")]
    pub recheck: Option<String>,
    #[schemars(with = "Nullable<String>")]
    pub note: Option<String>,
    pub state: ClaimState,
    /// Id of the reference mark (section 9.3).
    #[schemars(with = "Nullable<MarkId>")]
    pub mark: Option<MarkId>,
    #[schemars(with = "Nullable<Timestamp>")]
    pub marked_at: Option<Timestamp>,
    /// `add(markedAt, recheck)` when the claim is `fresh` or `expired` and
    /// `recheck` is set.
    #[schemars(with = "Nullable<Timestamp>")]
    pub expires_at: Option<Timestamp>,
    /// Empty for a retired claim.
    pub sides: Vec<Side>,
    /// One command line, or `null` (section 15.2).
    #[schemars(with = "Nullable<String>")]
    pub next: Option<String>,
}

/// One side that was not `same` when a write accepted its claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptedSide {
    /// 0 for the subject.
    pub index: usize,
    pub role: Role,
    pub loc: crate::model::Locator,
    /// The side's state before the write.
    pub state: SideState,
    #[schemars(with = "Nullable<Reason>")]
    pub reason: Option<Reason>,
}

/// What a write that produces a mark took over without being told: the
/// state the claim was in before the write, and the sides that were not
/// `same` then. The write made the claim fresh; this is the record that it
/// was not. `sides` is empty when the claim was `fresh`, and when it was
/// `expired` (only time had run out).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Accepted {
    /// The claim's state before the write.
    pub was: ClaimState,
    pub sides: Vec<AcceptedSide>,
}

/// The success envelope of section 15.1: `{"ok": true, "command": ...}`
/// followed by the keys of the command's output (which include
/// `warnings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Envelope<T> {
    /// Always `true`.
    pub ok: bool,
    pub command: String,
    #[serde(flatten)]
    pub output: T,
}

impl<T> Envelope<T> {
    pub fn new(command: &str, output: T) -> Self {
        Envelope {
            ok: true,
            command: command.to_owned(),
            output,
        }
    }
}

/// The failure envelope of section 15.1: `{"ok": false, "command",
/// "errors"}`. `errors` has at least one entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ErrorEnvelope {
    /// Always `false`.
    pub ok: bool,
    pub command: String,
    pub errors: Vec<Error>,
}

impl ErrorEnvelope {
    pub fn new(command: &str, errors: Vec<Error>) -> Self {
        ErrorEnvelope {
            ok: false,
            command: command.to_owned(),
            errors,
        }
    }
}

/// The first `max` characters of `text`.
pub fn excerpt(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => text[..i].to_owned(),
        None => text.to_owned(),
    }
}

/// Quote one argument for a POSIX shell. An argument of only
/// `[A-Za-z0-9_@%+=:,./-]` is left as it is; anything else is put in single
/// quotes, with each `'` written as `'\''`.
pub fn sh_quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if safe {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}
