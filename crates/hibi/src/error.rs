//! `Error` and the closed error-code list (spec section 15.4).
//!
//! Every fallible operation of the library returns [`Error`]. Its [`Code`] is
//! one of the closed list of section 15.4; the CLI prints it in the JSON
//! envelope of section 15.1 together with the message and the remediation.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The closed list of error codes of section 15.4.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    /// Unknown command or option, missing argument.
    Usage,
    /// A well-formed argument with an unacceptable value.
    InvalidArgument,
    /// Standard input is not the expected JSON.
    InvalidJson,
    /// A locator, selector or compact anchor does not parse.
    InvalidAnchor,
    /// Section 9.4.
    InvalidDuration,
    /// `--now` does not parse.
    InvalidTime,
    /// Section 10.3.
    InvalidRange,
    /// No `.hibi/hibi.toml` found.
    NotInitialized,
    /// `init` on an existing store.
    AlreadyInitialized,
    /// A command that needs git is run where the root is not a git top level, or git is missing.
    NotAGitRepository,
    /// Section 12.5.
    LegacyStore,
    /// `version` in `hibi.toml` is not 1.
    UnsupportedStoreVersion,
    /// `hibi.toml` does not parse or breaks section 11.1.
    InvalidConfig,
    /// The user config does not parse or breaks section 11.2.
    InvalidUserConfig,
    /// Section 12.3.
    StoreCorrupt,
    /// Section 12.4.
    StoreLocked,
    /// A write would take a shard or the store over its size limit.
    StoreFull,
    /// A file operation failed.
    IoError,
    /// Section 10.4.
    GitFailed,
    /// The installed git is older than `MIN_GIT` (section 10.4).
    GitTooOld,
    /// Section 3.6.
    ClaimNotFound,
    /// Section 3.6.
    AmbiguousId,
    /// A write on a retired claim.
    ClaimRetired,
    /// `reanchor` would produce the same claim.
    NoChange,
    /// A new anchor uses a scheme that is not declared.
    UnknownScheme,
    /// An anchor resolves to no unit.
    SideMissing,
    /// An anchor resolves to several units.
    SideAmbiguous,
    /// An anchor cannot be evaluated.
    SideUnavailable,
    /// A new anchor resolved only approximately.
    SelectorNotExact,
    /// A quote reaches outside one unit.
    QuoteSpansUnits,
    /// A quote lies wholly or partly inside a code comment.
    QuoteInComment,
    /// The quoted text is identical, with identical text around it in its
    /// unit, to another place in the same scope: a stored anchor
    /// could not tell the two apart.
    QuoteNotDistinguishable,
    /// A new anchor has a selector key its provider does not allow.
    UnsupportedSelector,
    /// `cover` or `triage` with no scope declared.
    NoScopes,
    /// `--scope` names no scope.
    UnknownScope,
    /// Section 14.4.
    TooManyCandidates,
    /// Section 14.1.
    JudgeUnavailable,
    /// Section 14.7.
    JudgeFailed,
    /// A bug.
    Internal,
}

impl Code {
    /// Every code, in the order of section 15.4.
    pub const ALL: [Code; 39] = [
        Code::Usage,
        Code::InvalidArgument,
        Code::InvalidJson,
        Code::InvalidAnchor,
        Code::InvalidDuration,
        Code::InvalidTime,
        Code::InvalidRange,
        Code::NotInitialized,
        Code::AlreadyInitialized,
        Code::NotAGitRepository,
        Code::LegacyStore,
        Code::UnsupportedStoreVersion,
        Code::InvalidConfig,
        Code::InvalidUserConfig,
        Code::StoreCorrupt,
        Code::StoreLocked,
        Code::StoreFull,
        Code::IoError,
        Code::GitFailed,
        Code::GitTooOld,
        Code::ClaimNotFound,
        Code::AmbiguousId,
        Code::ClaimRetired,
        Code::NoChange,
        Code::UnknownScheme,
        Code::SideMissing,
        Code::SideAmbiguous,
        Code::SideUnavailable,
        Code::SelectorNotExact,
        Code::QuoteSpansUnits,
        Code::QuoteInComment,
        Code::QuoteNotDistinguishable,
        Code::UnsupportedSelector,
        Code::NoScopes,
        Code::UnknownScope,
        Code::TooManyCandidates,
        Code::JudgeUnavailable,
        Code::JudgeFailed,
        Code::Internal,
    ];

    /// The wire name, as in section 15.4.
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Usage => "usage",
            Code::InvalidArgument => "invalid_argument",
            Code::InvalidJson => "invalid_json",
            Code::InvalidAnchor => "invalid_anchor",
            Code::InvalidDuration => "invalid_duration",
            Code::InvalidTime => "invalid_time",
            Code::InvalidRange => "invalid_range",
            Code::NotInitialized => "not_initialized",
            Code::AlreadyInitialized => "already_initialized",
            Code::NotAGitRepository => "not_a_git_repository",
            Code::LegacyStore => "legacy_store",
            Code::UnsupportedStoreVersion => "unsupported_store_version",
            Code::InvalidConfig => "invalid_config",
            Code::InvalidUserConfig => "invalid_user_config",
            Code::StoreCorrupt => "store_corrupt",
            Code::StoreLocked => "store_locked",
            Code::StoreFull => "store_full",
            Code::IoError => "io_error",
            Code::GitFailed => "git_failed",
            Code::GitTooOld => "git_too_old",
            Code::ClaimNotFound => "claim_not_found",
            Code::AmbiguousId => "ambiguous_id",
            Code::ClaimRetired => "claim_retired",
            Code::NoChange => "no_change",
            Code::UnknownScheme => "unknown_scheme",
            Code::SideMissing => "side_missing",
            Code::SideAmbiguous => "side_ambiguous",
            Code::SideUnavailable => "side_unavailable",
            Code::SelectorNotExact => "selector_not_exact",
            Code::QuoteSpansUnits => "quote_spans_units",
            Code::QuoteInComment => "quote_in_comment",
            Code::QuoteNotDistinguishable => "quote_not_distinguishable",
            Code::UnsupportedSelector => "unsupported_selector",
            Code::NoScopes => "no_scopes",
            Code::UnknownScope => "unknown_scope",
            Code::TooManyCandidates => "too_many_candidates",
            Code::JudgeUnavailable => "judge_unavailable",
            Code::JudgeFailed => "judge_failed",
            Code::Internal => "internal",
        }
    }

    /// The default remediation for the code: one sentence or one command
    /// that fixes the cause, or `None` when no general fix exists. A caller
    /// that knows a more specific fix (for example `hibi reanchor <id>`)
    /// replaces it with [`Error::with_remediation`].
    pub fn remediation(self) -> Option<&'static str> {
        Some(match self {
            Code::Usage => "hibi --help",
            Code::InvalidArgument => return None,
            Code::InvalidJson => {
                "Run `hibi schema` to list the input schemas, and check standard input against the one for this command."
            }
            Code::InvalidAnchor => {
                "Write the anchor as <locator>, <locator>#<symbol>, <locator>::<quote> or <locator>#<symbol>::<quote>, or as a JSON object {\"loc\": \"...\", \"sel\": {...}} when the path contains # or ::."
            }
            Code::InvalidDuration => {
                "Write the duration as P[nY][nM][nW][nD][T[nH][nM][nS]] with whole numbers, for example P90D."
            }
            Code::InvalidTime => "Write the time in RFC 3339, for example 2026-11-20T18:04:11Z.",
            Code::InvalidRange => "Use A, A.., A..HEAD, A... or A...HEAD, where A names a commit.",
            Code::NotInitialized => "hibi init",
            Code::AlreadyInitialized => {
                "The store is already there. Run hibi status to see its claims."
            }
            Code::NotAGitRepository => {
                "Run the command inside a git work tree, with git installed."
            }
            Code::LegacyStore => "remove or move .claims/, or use hibi 0.7",
            Code::UnsupportedStoreVersion => {
                "Use the hibi version that wrote this store, or set version = 1 in .hibi/hibi.toml."
            }
            Code::InvalidConfig => "Fix .hibi/hibi.toml as the message says.",
            Code::InvalidUserConfig => "Fix the user config file as the message says.",
            Code::StoreCorrupt => {
                "Look at the named file first: git diff -- <file> shows what differs from the last commit. Restoring it with git checkout -- <file> also discards every record in it that is not committed yet, so keep those lines if there are any. Replace any symbolic link with the real file or directory."
            }
            Code::StoreLocked => {
                "Wait for the other hibi process to finish, then run the command again."
            }
            Code::StoreFull => {
                "Nothing was written. Retired claims stay in the store by design, so it does not shrink: the store has to be split into several, or a later hibi version has to raise the limits."
            }
            Code::IoError => return None,
            Code::GitFailed => return None,
            Code::GitTooOld => "Install git 2.45.0 or later.",
            Code::ClaimNotFound => "hibi list",
            Code::AmbiguousId => "Give more characters of the id.",
            Code::ClaimRetired => {
                "To record the same statement and anchors again, give a different --note."
            }
            Code::NoChange => return None,
            Code::UnknownScheme => "Declare the scheme under [schemes] in .hibi/hibi.toml.",
            Code::SideMissing => "Point the anchor at text that exists in the resource.",
            Code::SideAmbiguous => {
                "Add a symbol, or a longer quote, so that the anchor matches one unit."
            }
            Code::SideUnavailable => "hibi providers",
            Code::SelectorNotExact => "Give a quote that occurs exactly in the unit you mean.",
            Code::QuoteSpansUnits => "Quote text from one unit only.",
            Code::QuoteInComment => "Anchor the code the comment describes.",
            Code::QuoteNotDistinguishable => {
                "Quote a longer passage that occurs once in its place."
            }
            Code::UnsupportedSelector => {
                "Use only the selector keys the provider lists; hibi providers shows them."
            }
            Code::NoScopes => "Declare a [[scope]] in .hibi/hibi.toml.",
            Code::UnknownScope => "Use a scope name declared in .hibi/hibi.toml.",
            Code::TooManyCandidates => "Narrow --in or --kind.",
            Code::JudgeUnavailable => {
                "Set TYPESAFE_API_KEY, or run hibi status without --judge-order."
            }
            Code::JudgeFailed => "Run the command again later.",
            Code::Internal => return None,
        })
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A hibi error: a code of section 15.4, a message, a remediation, and for
/// standard-input batches the index of the offending element.
///
/// Serializes to the error object of the JSON envelope (section 15.1):
/// `{"code", "message", "remediation", "index"?}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("{code}: {message}")]
#[serde(deny_unknown_fields)]
#[schemars(transform = remediation_is_required)]
pub struct Error {
    pub code: Code,
    pub message: String,
    pub remediation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}

/// Schema of [`Error`]: `remediation` is always written, as a string or as
/// `null` (section 15.1). The generator leaves an optional field out of
/// `required`.
fn remediation_is_required(schema: &mut schemars::Schema) {
    if let Some(serde_json::Value::Array(required)) = schema.get_mut("required") {
        required.push("remediation".into());
    }
}

/// Text appended to the message of an `internal` error.
const REPORT: &str =
    "this is a bug in hibi; please report it at https://github.com/npupko/hibi/issues";

impl Error {
    /// An error with the code's default remediation.
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        let mut message = message.into();
        if code == Code::Internal {
            message = format!("{message} ({REPORT})");
        }
        Error {
            code,
            message,
            remediation: code.remediation().map(str::to_owned),
            index: None,
        }
    }

    /// Replace the remediation.
    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }

    /// Remove the remediation.
    pub fn without_remediation(mut self) -> Self {
        self.remediation = None;
        self
    }

    /// Set the index of the offending standard-input element.
    pub fn with_index(mut self, index: usize) -> Self {
        self.index = Some(index);
        self
    }

    /// The same error under another code, keeping the message. The
    /// remediation becomes the new code's default.
    pub fn recode(self, code: Code) -> Self {
        Error::new(code, self.message).with_index_opt(self.index)
    }

    fn with_index_opt(mut self, index: Option<usize>) -> Self {
        self.index = index;
        self
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Error::new(Code::Usage, message)
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Error::new(Code::InvalidArgument, message)
    }

    pub fn invalid_anchor(message: impl Into<String>) -> Self {
        Error::new(Code::InvalidAnchor, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Error::new(Code::Internal, message)
    }

    /// An `io_error` naming the path and the operating-system error.
    pub fn io(path: impl fmt::Display, err: &std::io::Error) -> Self {
        Error::new(Code::IoError, format!("{path}: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_serialize_to_their_names() {
        for code in Code::ALL {
            let json = serde_json::to_value(code).unwrap();
            assert_eq!(json, serde_json::Value::String(code.as_str().to_owned()));
            let back: Code = serde_json::from_value(json).unwrap();
            assert_eq!(back, code);
        }
    }

    #[test]
    fn codes_are_unique() {
        let mut names: Vec<_> = Code::ALL.iter().map(|c| c.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Code::ALL.len());
    }

    #[test]
    fn envelope_shape() {
        let e = Error::new(Code::NotInitialized, "no .hibi/hibi.toml found").with_index(2);
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"code": "not_initialized", "message": "no .hibi/hibi.toml found", "remediation": "hibi init", "index": 2})
        );
        let e = Error::new(Code::IoError, "x");
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"code": "io_error", "message": "x", "remediation": null})
        );
    }

    #[test]
    fn legacy_store_remediation_is_the_spec_text() {
        assert_eq!(
            Code::LegacyStore.remediation(),
            Some("remove or move .claims/, or use hibi 0.7")
        );
    }
}
