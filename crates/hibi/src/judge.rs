//! Jev requests, the judge cache, and the advisory commands `triage`,
//! `suggest-evidence` and `status --judge-order` (spec section 14).
//!
//! The judge is advisory. No answer is written to the store, changes a
//! claim's state, or changes which claims or units a command reports. Every
//! entry point needs a [`Judge`], which only exists when `TYPESAFE_API_KEY`
//! is set and non-empty ([`Judge::new`] fails with `judge_unavailable`
//! otherwise), and then checks git (section 10.4) before any network
//! contact. A failed request fails the whole command with `judge_failed`;
//! it is never read as a "no".
//!
//! | Command | Function | Output |
//! |---|---|---|
//! | `triage` | [`triage`] | [`TriageOutput`] |
//! | `suggest-evidence` | [`suggest_evidence`] | [`SuggestOutput`] |
//! | `status --judge-order` | [`judge_order`] | [`crate::engine::StatusOutput`] |
//!
//! Question wording lives in [`questions`]; the transport policy of section
//! 14.7 in [`Policy`]; the cache of section 14.6 in [`ask`].

mod ask;
mod client;
mod gate;
mod order;
pub mod questions;
mod suggest;
mod triage;

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::cache::Cache;
use crate::config::{RepoConfig, UserConfig};
use crate::error::{Code, Error};
use crate::git::{Git, GitStatus};
use crate::model::Timestamp;

pub use ask::{Answer, cache_key, read_answer};
pub use order::judge_order;
pub use suggest::{
    DEFAULT_KINDS, DEFAULT_SUGGEST_LIMIT, MAX_CANDIDATES, SuggestCandidate, SuggestOutput,
    SuggestQuery, suggest_evidence,
};
pub use triage::{
    Band, DEFAULT_TRIAGE_LIMIT, EvidenceLabel, PROSE_KINDS, TriageItem, TriageOutput, TriageQuery,
    triage,
};

/// The pinned model id (section 14.2, R24). An alias is never sent.
pub const MODEL: &str = "jev-1.14.0";

/// The endpoint of section 14.2.
pub const DEFAULT_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

/// The environment variable that holds the key (section 14.1).
pub const KEY_VAR: &str = "TYPESAFE_API_KEY";

/// The environment variable that replaces the endpoint, for tests
/// (section 14.2). The library never reads it: the caller passes its value
/// to [`Judge::new`].
pub const ENDPOINT_VAR: &str = "HIBI_JEV_URL";

/// At most this many questions in one request (section 14.2).
pub const MAX_QUESTIONS_PER_REQUEST: usize = 100;

/// At most this many bytes of serialized `questions` in one request
/// (section 14.2).
pub const MAX_QUESTION_BYTES_PER_REQUEST: usize = 60_000;

/// Item text placed in a question is cut to this many characters
/// (section 14.2).
pub const MAX_ITEM_CHARS: usize = 4_000;

/// The transport policy of section 14.7. [`Policy::default`] is the
/// specified one; tests replace `sleep` (and may shorten `timeout`) so that
/// backoff can be observed without waiting.
#[derive(Clone)]
pub struct Policy {
    /// Timeout of one attempt.
    pub timeout: Duration,
    /// Attempts per request, the first included.
    pub attempts: u32,
    /// Backoff before the first retry; it doubles before each later one.
    pub backoff_initial: Duration,
    /// Cap on every backoff, including one taken from `Retry-After`.
    pub backoff_max: Duration,
    /// Requests in flight at once.
    pub concurrency: usize,
    /// How a backoff is waited out.
    pub sleep: Arc<dyn Fn(Duration) + Send + Sync>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            timeout: Duration::from_secs(10),
            attempts: 3,
            backoff_initial: Duration::from_millis(500),
            backoff_max: Duration::from_secs(5),
            concurrency: 6,
            sleep: Arc::new(std::thread::sleep),
        }
    }
}

impl fmt::Debug for Policy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Policy")
            .field("timeout", &self.timeout)
            .field("attempts", &self.attempts)
            .field("backoff_initial", &self.backoff_initial)
            .field("backoff_max", &self.backoff_max)
            .field("concurrency", &self.concurrency)
            .finish_non_exhaustive()
    }
}

impl Policy {
    /// The backoff before retry `n` (1 for the first retry):
    /// `min(backoff_max, backoff_initial * 2^(n-1))`, or `retry_after`
    /// capped at `backoff_max` when the response carried one.
    pub fn backoff(&self, n: u32, retry_after: Option<Duration>) -> Duration {
        match retry_after {
            Some(d) => d.min(self.backoff_max),
            None => {
                let factor = 1u32.checked_shl(n.saturating_sub(1)).unwrap_or(u32::MAX);
                self.backoff_initial
                    .checked_mul(factor)
                    .unwrap_or(self.backoff_max)
                    .min(self.backoff_max)
            }
        }
    }
}

/// A configured Jev client: the key, the endpoint and the transport policy.
/// Holding one means the key was present (section 14.1).
#[derive(Clone)]
pub struct Judge {
    key: String,
    endpoint: String,
    policy: Policy,
}

impl fmt::Debug for Judge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Judge")
            .field("key", &"<redacted>")
            .field("endpoint", &self.endpoint)
            .field("policy", &self.policy)
            .finish()
    }
}

impl Judge {
    /// A client for `key` (the value of `TYPESAFE_API_KEY`) and `endpoint`
    /// (the value of `HIBI_JEV_URL`; `None` or empty means
    /// [`DEFAULT_ENDPOINT`]), with the default [`Policy`].
    ///
    /// An unset or empty key is `judge_unavailable` (section 14.1). An
    /// endpoint that is not `https://...`, or `http://` to a loopback host
    /// (`127.0.0.1`, `[::1]`, `localhost`), is `invalid_argument`, so the key
    /// never travels in clear text over a network. Nothing is contacted.
    pub fn new(key: Option<&str>, endpoint: Option<&str>) -> Result<Judge, Error> {
        let key = match key {
            Some(k) if !k.is_empty() => k.to_owned(),
            _ => {
                return Err(Error::new(
                    Code::JudgeUnavailable,
                    format!("{KEY_VAR} is not set; the judge commands need it"),
                ));
            }
        };
        let endpoint = match endpoint {
            Some(e) if !e.is_empty() => e.to_owned(),
            _ => DEFAULT_ENDPOINT.to_owned(),
        };
        check_endpoint(&endpoint)?;
        Ok(Judge {
            key,
            endpoint,
            policy: Policy::default(),
        })
    }

    /// The same client with another transport policy.
    pub fn with_policy(mut self, policy: Policy) -> Judge {
        self.policy = policy;
        self
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }
}

fn check_endpoint(endpoint: &str) -> Result<(), Error> {
    let bad = || {
        Error::invalid_argument(format!(
            "{ENDPOINT_VAR} {endpoint:?} must be an https:// URL, or http:// to 127.0.0.1, [::1] or localhost"
        ))
    };
    if endpoint.chars().any(|c| c.is_control() || c == ' ') {
        return Err(bad());
    }
    if let Some(rest) = endpoint.strip_prefix("https://") {
        return if rest.is_empty() { Err(bad()) } else { Ok(()) };
    }
    let Some(rest) = endpoint.strip_prefix("http://") else {
        return Err(bad());
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err(bad());
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if matches!(host, "127.0.0.1" | "::1" | "localhost") {
        Ok(())
    } else {
        Err(bad())
    }
}

/// What the judge commands read besides the store and the providers.
pub struct JudgeEnv<'a> {
    /// The root: the directory that contains `.hibi/`.
    pub root: &'a Path,
    pub config: &'a RepoConfig,
    pub user: &'a UserConfig,
    /// The once-per-run git check. The judge commands require git
    /// (section 10.4).
    pub git: &'a GitStatus,
    /// The cache database, or `None` (no cache, `--no-cache`, or it could
    /// not be opened): then every question is asked.
    pub cache: Option<&'a Cache>,
    /// `now`: the `at` of cache rows, and `now` of `status`.
    pub now: Timestamp,
}

impl JudgeEnv<'_> {
    /// Section 14.1 after the key: git must be usable and the root must be
    /// a git top level, or the command fails with `not_a_git_repository`
    /// or `git_too_old` before any network contact.
    fn require_git(&self) -> Result<&Git, Error> {
        let git = self.git.require()?;
        git.check_root(self.root)?;
        Ok(git)
    }
}

/// Cut `text` to its first [`MAX_ITEM_CHARS`] characters (section 14.2).
fn cut(text: &str) -> String {
    crate::engine::excerpt(text, MAX_ITEM_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_must_be_present() {
        for key in [None, Some("")] {
            let e = Judge::new(key, None).unwrap_err();
            assert_eq!(e.code, Code::JudgeUnavailable);
        }
        let j = Judge::new(Some("k"), None).unwrap();
        assert_eq!(j.endpoint(), DEFAULT_ENDPOINT);
        assert!(!format!("{j:?}").contains("\"k\""));
    }

    #[test]
    fn endpoints() {
        for ok in [
            "https://api.typesafe.ai/v1/systemone",
            "http://127.0.0.1:4000/v1/systemone",
            "http://localhost/x",
            "http://[::1]:9/x",
        ] {
            assert!(Judge::new(Some("k"), Some(ok)).is_ok(), "{ok}");
        }
        for bad in [
            "http://example.com/x",
            "ftp://x",
            "http://127.0.0.1.evil.com/",
            "http://u@127.0.0.1:1@evil.com/",
            "https://",
            "https://a b",
        ] {
            let e = Judge::new(Some("k"), Some(bad)).unwrap_err();
            assert_eq!(e.code, Code::InvalidArgument, "{bad}");
        }
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        let p = Policy::default();
        let ms = |n| p.backoff(n, None).as_millis();
        assert_eq!(
            [ms(1), ms(2), ms(3), ms(4), ms(5)],
            [500, 1000, 2000, 4000, 5000]
        );
        assert_eq!(ms(64), 5000);
        assert_eq!(
            p.backoff(1, Some(Duration::from_secs(2))),
            Duration::from_secs(2)
        );
        assert_eq!(
            p.backoff(1, Some(Duration::from_secs(60))),
            Duration::from_secs(5)
        );
    }
}
