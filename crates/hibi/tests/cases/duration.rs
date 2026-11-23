//! `duration.json` and `recheck-boundary.json` (spec sections 9.3 and 9.4).
//!
//! Instants in these files carry milliseconds and sometimes an offset
//! (cases/README.md, open point 1). They are read as instants and compared
//! as instants; `duration::add` keeps full precision.

use hibi::duration;
use jiff::civil::DateTime;
use serde_json::Value;

use crate::harness::{Case, expect_eq, str_field};

/// An RFC 3339 instant with any offset, as a UTC civil date-time.
fn instant(s: &str) -> DateTime {
    let ts: jiff::Timestamp = s.parse().unwrap_or_else(|e| panic!("{s}: {e}"));
    jiff::tz::Offset::UTC.to_datetime(ts)
}

/// `input: {start, duration}`, `expected: {instant}` or `{error: "invalid_duration"}`.
pub fn duration(case: &Case) -> Result<(), String> {
    let start = instant(str_field(&case.input, "start"));
    let text = str_field(&case.input, "duration");
    let got = duration::parse(text).and_then(|d| duration::add(start, &d));
    match (&case.expected.get("instant"), got) {
        (Some(Value::String(want)), Ok(got)) => expect_eq("instant", got, instant(want)),
        (None, Err(e)) => expect_eq("error", e.code.as_str(), str_field(&case.expected, "error")),
        (_, got) => Err(format!("got {got:?}, want {}", case.expected)),
    }
}

/// `input: {from, duration, now}`, `expected: {expired}`.
pub fn recheck_boundary(case: &Case) -> Result<(), String> {
    let from = instant(str_field(&case.input, "from"));
    let now = instant(str_field(&case.input, "now"));
    let d = duration::parse(str_field(&case.input, "duration")).map_err(|e| e.to_string())?;
    let expired = duration::is_expired(from, &d, now).map_err(|e| e.to_string())?;
    expect_eq(
        "expired",
        expired,
        case.expected["expired"]
            .as_bool()
            .expect("expected.expired is a boolean"),
    )
}
