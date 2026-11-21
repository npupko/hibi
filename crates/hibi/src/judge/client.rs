//! The transport of section 14.7: plain HTTPS POST with `ureq`, at most
//! `concurrency` requests in flight, a timeout per attempt, retries with
//! backoff on 408, 429, 5xx, connection errors and timeouts.
//!
//! The key. No error that leaves this module holds the key or the
//! `Authorization` header value: the body of a refused request is passed
//! through [`redact`] before it is quoted, and every error is passed
//! through it once more by [`seal`] before it is returned, so neither a
//! transport error string nor text quoted from a usable response can hold
//! it. A usable response is parsed as the peer wrote it: only its typed
//! answer fields are read, and only those are cached (`Answer::to_cached`),
//! so a key a proxy echoed back reaches neither the output nor the cache.
//!
//! Local errors. The header value and the URL are built once by
//! [`prepare`], before any attempt. A key or URL the HTTP library cannot
//! send fails there: nothing is sent, nothing is retried, nothing is
//! waited for.

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use ureq::Agent;
use ureq::http::{HeaderValue, Uri};

use crate::error::{Code, Error};

use super::{ENDPOINT_VAR, Judge, KEY_VAR};

/// The largest response body read, in bytes. A Jev answer for 100
/// questions is a few kilobytes.
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

/// The most characters of an error response body quoted in a message.
const MAX_DETAIL_CHARS: usize = 300;

/// What stands where the key or the `Authorization` header value stood.
const REDACTED: &str = "[redacted]";

/// The scheme of the `Authorization` header value.
const SCHEME: &str = "Bearer";

/// Where a bug in hibi is reported.
const ISSUES: &str = "https://github.com/npupko/hibi/issues";

/// Why a request failed, as far as the remediation depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cause {
    /// 401 or 403: the service refused the key.
    Auth,
    /// The key cannot be sent in an HTTP header.
    Key,
    /// Another 4xx status that is not retried: the service rejected what
    /// hibi sent.
    Rejected(u16),
    /// A request the HTTP library refused for a reason that is not the key
    /// or the URL.
    Bug,
    /// A redirect, a status outside 200 to 599, an unusable URL, or an
    /// answer that is not the service's: the endpoint is not Jev.
    Endpoint,
    /// 408, 429, 5xx, a connection error, a timeout, or a cause not known:
    /// the default remediation of `judge_failed`.
    Later,
}

impl Cause {
    /// The remediation, or `None` for the default one of `judge_failed`.
    fn remediation(self) -> Option<String> {
        match self {
            Cause::Auth => Some(format!("Check {KEY_VAR}: the service refused it.")),
            Cause::Key => Some(format!(
                "Check {KEY_VAR}: set it to the key as the service issued it, on one line."
            )),
            Cause::Rejected(status) => Some(format!(
                "This is a bug in hibi: the service rejected the request. Report it at {ISSUES} with the status {status}."
            )),
            Cause::Bug => Some(format!(
                "This is a bug in hibi. Report it at {ISSUES} with this message."
            )),
            Cause::Endpoint => Some(endpoint_remediation()),
            Cause::Later => None,
        }
    }
}

fn endpoint_remediation() -> String {
    format!("Check {ENDPOINT_VAR}: the endpoint did not answer as the Jev service does.")
}

/// A failed request: the message and the cause.
struct Failure {
    message: String,
    cause: Cause,
}

/// What one attempt gave.
enum Attempt {
    /// A 2xx status, with the body.
    Done(String),
    /// Worth another attempt: the reason, and the `Retry-After` delay.
    Retry(String, Option<Duration>),
    /// Not retried.
    Fatal(Failure),
}

/// `text` without the key: every occurrence of the key (as it is, trimmed,
/// and as a JSON string writes it) is replaced with [`REDACTED`], and a
/// `Bearer` scheme in front of a replaced key is removed with it, so that
/// the whole `Authorization` header value is gone.
fn redact(text: &str, key: &str) -> String {
    let mut secrets = vec![key.to_owned(), key.trim().to_owned()];
    if let Ok(quoted) = serde_json::to_string(key) {
        secrets.push(quoted[1..quoted.len() - 1].to_owned());
    }
    secrets.retain(|s| !s.is_empty());
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    secrets.dedup();
    if !secrets.iter().any(|s| text.contains(s.as_str())) {
        return text.to_owned();
    }
    let mut replaced = text.to_owned();
    for secret in &secrets {
        replaced = replaced.replace(secret.as_str(), REDACTED);
    }
    let mut out = String::with_capacity(replaced.len());
    let mut rest = replaced.as_str();
    while let Some(at) = rest.find(REDACTED) {
        out.push_str(without_scheme(&rest[..at]));
        out.push_str(REDACTED);
        rest = &rest[at + REDACTED.len()..];
    }
    out.push_str(rest);
    out
}

/// `before` without a trailing `Bearer` and the white space after it
/// (ASCII case-insensitive), or `before` when it does not end so.
fn without_scheme(before: &str) -> &str {
    let trimmed = before.trim_end_matches([' ', '\t']);
    if trimmed.len() == before.len() || trimmed.len() < SCHEME.len() {
        return before;
    }
    let at = trimmed.len() - SCHEME.len();
    if trimmed.is_char_boundary(at) && trimmed[at..].eq_ignore_ascii_case(SCHEME) {
        &trimmed[..at]
    } else {
        before
    }
}

/// The last step of every error of this module: no key in the message or
/// the remediation.
fn seal(judge: &Judge, mut e: Error) -> Error {
    e.message = redact(&e.message, &judge.key);
    e.remediation = e.remediation.map(|r| redact(&r, &judge.key));
    e
}

/// The `judge_failed` error of a failure, with the remediation of its
/// cause.
fn failed(judge: &Judge, failure: Failure) -> Error {
    let e = Error::new(Code::JudgeFailed, failure.message);
    let e = match failure.cause.remediation() {
        Some(r) => e.with_remediation(r),
        None => e,
    };
    seal(judge, e)
}

/// The error of a response body that was read and is not usable (section
/// 14.2): the endpoint did not answer as Jev does.
fn unusable(judge: &Judge, e: Error) -> Error {
    let e = if e.code == Code::JudgeFailed {
        e.with_remediation(endpoint_remediation())
    } else {
        e
    };
    seal(judge, e)
}

fn agent(judge: &Judge) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(judge.policy.timeout))
        .http_status_as_error(false)
        // A redirect is a non-retried status: the key is never sent to a
        // second host.
        .max_redirects(0)
        .max_redirects_will_error(false)
        .user_agent(concat!("hibi/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// What every attempt sends besides the body.
struct Prepared {
    uri: Uri,
    authorization: HeaderValue,
}

/// Build the URL and the `Authorization` header value with the types of
/// the HTTP library. A failure here is local: nothing was sent, and
/// another attempt would fail the same way. The message says what is wrong
/// with the key and never holds it.
fn prepare(judge: &Judge) -> Result<Prepared, Failure> {
    let cannot = |why: &str| Failure {
        message: format!("the value of {KEY_VAR} cannot be sent in an HTTP header: {why}"),
        cause: Cause::Key,
    };
    // `from_str` refuses control characters other than a tab; `to_str`
    // refuses what is left that is not printable ASCII, which the library
    // would refuse when it writes the request.
    let mut authorization = HeaderValue::from_str(&format!("{SCHEME} {}", judge.key))
        .map_err(|_| cannot("it contains a line break or another control character"))?;
    if authorization.to_str().is_err() {
        return Err(cannot(
            "it contains a character that is not printable ASCII",
        ));
    }
    authorization.set_sensitive(true);
    let uri = judge.endpoint.parse::<Uri>().map_err(|e| Failure {
        message: format!("{ENDPOINT_VAR} {:?} is not a URL: {e}", judge.endpoint),
        cause: Cause::Endpoint,
    })?;
    Ok(Prepared { uri, authorization })
}

/// Seconds in a `Retry-After` header: a non-negative decimal number. An
/// HTTP date is not read (the library does not read the clock); the
/// computed backoff is used instead.
fn retry_after(value: &str) -> Option<Duration> {
    let secs: f64 = value.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs.min(3600.0)))
}

/// The start of an error response body, for a message. `body` is already
/// redacted, so the cut cannot leave a part of the key.
fn detail(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return String::new();
    }
    let cut = crate::engine::excerpt(body, MAX_DETAIL_CHARS);
    let more = if cut.len() < body.len() { "…" } else { "" };
    format!(": {cut}{more}")
}

/// What a status outside 200 to 299 means (section 14.7): `None` when it
/// is retried, else the cause.
fn status_cause(status: u16) -> Option<Cause> {
    match status {
        408 | 429 | 500..=599 => None,
        401 | 403 => Some(Cause::Auth),
        400..=499 => Some(Cause::Rejected(status)),
        _ => Some(Cause::Endpoint),
    }
}

/// An error of the HTTP library for a request that got no response.
fn transport_error(judge: &Judge, e: &ureq::Error) -> Attempt {
    let message = format!("the request to {} failed: {e}", judge.endpoint);
    let cause = match e {
        // Connection errors and timeouts (section 14.7). `Protocol` is
        // here because, with the header and the URL built by `prepare`,
        // what is left of it is a peer that does not speak HTTP.
        ureq::Error::Timeout(_)
        | ureq::Error::Io(_)
        | ureq::Error::ConnectionFailed
        | ureq::Error::HostNotFound
        | ureq::Error::Protocol(_) => return Attempt::Retry(message, None),
        // Raised before anything is sent.
        ureq::Error::BadUri(_) | ureq::Error::RequireHttpsOnly(_) => Cause::Endpoint,
        ureq::Error::Http(_) | ureq::Error::BodyExceedsLimit(_) => Cause::Bug,
        // The peer is not the service.
        ureq::Error::Tls(_)
        | ureq::Error::TlsRequired
        | ureq::Error::RedirectFailed
        | ureq::Error::TooManyRedirects
        | ureq::Error::LargeResponseHeader(_, _) => Cause::Endpoint,
        _ => Cause::Later,
    };
    Attempt::Fatal(Failure { message, cause })
}

fn attempt(agent: &Agent, judge: &Judge, prepared: &Prepared, body: &[u8]) -> Attempt {
    let sent = agent
        .post(prepared.uri.clone())
        .header("Authorization", prepared.authorization.clone())
        .header("Content-Type", "application/json")
        .send(body);
    let mut response = match sent {
        Ok(r) => r,
        Err(e) => return transport_error(judge, &e),
    };
    let status = response.status().as_u16();
    let wait = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(retry_after);
    let mut text = String::new();
    let read = response
        .body_mut()
        .as_reader()
        .take(MAX_RESPONSE_BYTES)
        .read_to_string(&mut text);
    // A usable body is parsed as the peer wrote it; only typed fields are
    // read from it, and any text taken from it for an error is redacted
    // when the error leaves this module (`seal`).
    if (200..300).contains(&status) {
        return match read {
            Ok(_) => Attempt::Done(text),
            Err(e) => Attempt::Retry(format!("reading the response failed: {e}"), None),
        };
    }
    // Redacted before the cut, so that the cut cannot leave a part of the
    // key.
    let message = format!(
        "Jev answered with status {status}{}",
        detail(&redact(&text, &judge.key))
    );
    match status_cause(status) {
        None => Attempt::Retry(message, wait),
        Some(cause) => Attempt::Fatal(Failure { message, cause }),
    }
}

/// One request with its retries. On success, the response body as the peer
/// wrote it.
fn post(agent: &Agent, judge: &Judge, prepared: &Prepared, body: &[u8]) -> Result<String, Failure> {
    let policy = &judge.policy;
    let attempts = policy.attempts.max(1);
    let mut n = 1;
    loop {
        match attempt(agent, judge, prepared, body) {
            Attempt::Done(text) => return Ok(text),
            Attempt::Fatal(failure) => return Err(failure),
            Attempt::Retry(message, wait) => {
                if n >= attempts {
                    return Err(Failure {
                        message: format!("{message} (after {attempts} attempts)"),
                        cause: Cause::Later,
                    });
                }
                (policy.sleep)(policy.backoff(n, wait));
                n += 1;
            }
        }
    }
}

/// Send every body, at most `policy.concurrency` at once, and pass each
/// successful response body to `parse`. Returns one entry per body: `None`
/// for a body that was not sent because an earlier one had failed, else
/// the parse result or the `judge_failed` error. After the first failure no
/// further request is started; requests in flight finish.
///
/// The body given to `parse` and every error returned are without the key
/// (see [`redact`]). A key or URL that cannot be sent fails the first body
/// and sends nothing.
pub(super) fn send_all<T: Send>(
    judge: &Judge,
    bodies: &[Vec<u8>],
    parse: &(dyn Fn(usize, String) -> Result<T, Error> + Sync),
) -> Vec<Option<Result<T, Error>>> {
    let total = bodies.len();
    let mut out: Vec<Option<Result<T, Error>>> = (0..total).map(|_| None).collect();
    if total == 0 {
        return out;
    }
    let prepared = match prepare(judge) {
        Ok(p) => p,
        Err(failure) => {
            out[0] = Some(Err(failed(judge, failure)));
            return out;
        }
    };
    let agent = agent(judge);
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel();
    let workers = judge.policy.concurrency.max(1).min(total);
    std::thread::scope(|s| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (agent, prepared, next, stop) = (&agent, &prepared, &next, &stop);
            s.spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= total {
                        break;
                    }
                    let result = match post(agent, judge, prepared, &bodies[i]) {
                        Ok(text) => parse(i, text).map_err(|e| unusable(judge, e)),
                        Err(mut failure) => {
                            failure.message =
                                format!("request {} of {total}: {}", i + 1, failure.message);
                            Err(failed(judge, failure))
                        }
                    };
                    if result.is_err() {
                        stop.store(true, Ordering::SeqCst);
                    }
                    if tx.send((i, result)).is_err() {
                        break;
                    }
                }
            });
        }
    });
    drop(tx);
    for (i, result) in rx {
        out[i] = Some(result);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "sk-test-9f3k2m7q4x8tbh2d5wqa";

    #[test]
    fn retry_after_seconds() {
        assert_eq!(retry_after("2"), Some(Duration::from_secs(2)));
        assert_eq!(retry_after(" 0.25 "), Some(Duration::from_millis(250)));
        assert_eq!(retry_after("-1"), None);
        assert_eq!(retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
    }

    #[test]
    fn detail_is_cut() {
        assert_eq!(detail("  "), "");
        assert_eq!(detail("bad"), ": bad");
        let long = "x".repeat(400);
        assert!(detail(&long).ends_with('…'));
    }

    #[test]
    fn redact_removes_the_key_and_the_header_value() {
        let r = |text: &str| redact(text, KEY);
        assert_eq!(r("nothing here"), "nothing here");
        assert_eq!(r(&format!("key={KEY};")), "key=[redacted];");
        assert_eq!(
            r(&format!("authorization: Bearer {KEY}\r\nx: {KEY}")),
            "authorization: [redacted]\r\nx: [redacted]"
        );
        assert_eq!(
            r(&format!(r#"{{"authorization":"bearer   {KEY}"}}"#)),
            r#"{"authorization":"[redacted]"}"#
        );
        // A scheme that is not in front of the key stays.
        assert_eq!(
            r(&format!("Bearer tokens: {KEY}")),
            "Bearer tokens: [redacted]"
        );
        assert_eq!(r(&format!("é{KEY}é")), "é[redacted]é");
    }

    #[test]
    fn redact_knows_the_trimmed_and_the_json_written_key() {
        // A server reads the header value without the white space around it.
        assert_eq!(redact("Bearer abc123", " abc123 "), "[redacted]");
        // A JSON body writes `"` and `\` of the key with a backslash.
        let key = r#"ab"c\d"#;
        let body = serde_json::json!({ "authorization": format!("Bearer {key}") }).to_string();
        assert_eq!(redact(&body, key), r#"{"authorization":"[redacted]"}"#);
        assert_eq!(redact(key, key), "[redacted]");
    }

    #[test]
    fn statuses_by_cause() {
        for retried in [408, 429, 500, 502, 529, 599] {
            assert_eq!(status_cause(retried), None, "{retried}");
        }
        assert_eq!(status_cause(401), Some(Cause::Auth));
        assert_eq!(status_cause(403), Some(Cause::Auth));
        for status in [400, 404, 405, 413, 415, 422, 499] {
            assert_eq!(status_cause(status), Some(Cause::Rejected(status)));
        }
        for status in [100, 199, 300, 301, 307, 308, 399, 600, 999] {
            assert_eq!(status_cause(status), Some(Cause::Endpoint), "{status}");
        }
    }

    #[test]
    fn a_key_that_cannot_be_a_header_value_fails_before_any_attempt() {
        let failure = |key: &str| {
            let judge = Judge::new(Some(key), None).unwrap();
            prepare(&judge).err().expect("refused")
        };
        for key in ["a\nb", "a\rb", "a\0b", "a\u{7f}b"] {
            let f = failure(key);
            assert_eq!(f.cause, Cause::Key);
            assert!(f.message.contains("control character"), "{}", f.message);
        }
        for key in ["clé", "ключ", "a\u{a0}b"] {
            let f = failure(key);
            assert_eq!(f.cause, Cause::Key);
            assert!(f.message.contains("not printable ASCII"), "{}", f.message);
            assert!(!f.message.contains(key));
        }
        let judge = Judge::new(Some(KEY), None).unwrap();
        let p = prepare(&judge).ok().expect("accepted");
        assert!(p.authorization.is_sensitive());
        assert_eq!(p.authorization.to_str().unwrap(), format!("Bearer {KEY}"));
    }

    #[test]
    fn local_errors_of_the_library_are_not_retried() {
        let judge = Judge::new(Some(KEY), None).unwrap();
        let fatal = |e: ureq::Error| match transport_error(&judge, &e) {
            Attempt::Fatal(f) => Some(f.cause),
            _ => None,
        };
        assert_eq!(
            fatal(ureq::Error::BadUri("x is missing host".into())),
            Some(Cause::Endpoint)
        );
        let http = ureq::http::Request::builder()
            .header("a b", "c")
            .body(())
            .unwrap_err();
        assert_eq!(fatal(ureq::Error::Http(http)), Some(Cause::Bug));
        assert_eq!(fatal(ureq::Error::TooManyRedirects), Some(Cause::Endpoint));
        assert_eq!(fatal(ureq::Error::ConnectionFailed), None);
        assert_eq!(fatal(ureq::Error::HostNotFound), None);
        let io = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
        assert_eq!(fatal(ureq::Error::Io(io)), None);
    }
}
