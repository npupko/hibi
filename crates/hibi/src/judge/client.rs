//! The transport of section 14.7: plain HTTPS POST with `ureq`, at most
//! `concurrency` requests in flight, a timeout per attempt, retries with
//! backoff on 408, 429, 5xx, connection errors and timeouts.

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use ureq::Agent;

use crate::error::{Code, Error};

use super::Judge;

/// The largest response body read, in bytes. A Jev answer for 100
/// questions is a few kilobytes.
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

/// The most characters of an error response body quoted in a message.
const MAX_DETAIL_CHARS: usize = 300;

/// What one attempt gave.
enum Attempt {
    /// A 2xx status, with the body.
    Done(String),
    /// Worth another attempt: the reason, and the `Retry-After` delay.
    Retry(String, Option<Duration>),
    /// Not retried.
    Fatal(String),
}

fn failed(message: impl Into<String>) -> Error {
    Error::new(Code::JudgeFailed, message)
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

/// Seconds in a `Retry-After` header: a non-negative decimal number. An
/// HTTP date is not read (the library does not read the clock); the
/// computed backoff is used instead.
fn retry_after(value: &str) -> Option<Duration> {
    let secs: f64 = value.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs.min(3600.0)))
}

fn detail(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return String::new();
    }
    let cut = crate::engine::excerpt(body, MAX_DETAIL_CHARS);
    let more = if cut.len() < body.len() { "…" } else { "" };
    format!(": {cut}{more}")
}

fn attempt(agent: &Agent, judge: &Judge, body: &[u8]) -> Attempt {
    let sent = agent
        .post(&judge.endpoint)
        .header("Authorization", &format!("Bearer {}", judge.key))
        .header("Content-Type", "application/json")
        .send(body);
    let mut response = match sent {
        Ok(r) => r,
        Err(e) => {
            let retry = matches!(
                e,
                ureq::Error::Timeout(_)
                    | ureq::Error::Io(_)
                    | ureq::Error::ConnectionFailed
                    | ureq::Error::HostNotFound
                    | ureq::Error::Protocol(_)
            );
            let message = format!("the request to {} failed: {e}", judge.endpoint);
            return if retry {
                Attempt::Retry(message, None)
            } else {
                Attempt::Fatal(message)
            };
        }
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
    if (200..300).contains(&status) {
        return match read {
            Ok(_) => Attempt::Done(text),
            Err(e) => Attempt::Retry(format!("reading the response failed: {e}"), None),
        };
    }
    let message = format!("Jev answered with status {status}{}", detail(&text));
    if status == 408 || status == 429 || (500..600).contains(&status) {
        Attempt::Retry(message, wait)
    } else {
        Attempt::Fatal(message)
    }
}

/// One request with its retries. On success, the response body.
fn post(agent: &Agent, judge: &Judge, body: &[u8]) -> Result<String, String> {
    let policy = &judge.policy;
    let attempts = policy.attempts.max(1);
    let mut n = 1;
    loop {
        match attempt(agent, judge, body) {
            Attempt::Done(text) => return Ok(text),
            Attempt::Fatal(message) => return Err(message),
            Attempt::Retry(message, wait) => {
                if n >= attempts {
                    return Err(format!("{message} (after {attempts} attempts)"));
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
    let agent = agent(judge);
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel();
    let workers = judge.policy.concurrency.max(1).min(total);
    std::thread::scope(|s| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (agent, next, stop) = (&agent, &next, &stop);
            s.spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= total {
                        break;
                    }
                    let result = post(agent, judge, &bodies[i])
                        .map_err(|m| failed(format!("request {} of {total}: {m}", i + 1)))
                        .and_then(|text| parse(i, text));
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
}
