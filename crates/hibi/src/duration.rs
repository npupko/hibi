//! The duration grammar and arithmetic (spec section 9.4).
//!
//! Pure: no file, clock, environment or process access. Instants are
//! `jiff::civil::DateTime` values read as UTC, so that every instant up to
//! 9999-12-31T23:59:59Z is representable; a fraction of a second is kept.

use jiff::SignedDuration;
use jiff::civil::{Date, DateTime};

use crate::error::{Code, Error};

/// A parsed duration: `P[nY][nM][nW][nD][T[nH][nM][nS]]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Duration {
    pub years: u64,
    pub months: u64,
    pub weeks: u64,
    pub days: u64,
    pub hours: u64,
    pub minutes: u64,
    pub seconds: u64,
}

fn invalid(s: &str, why: &str) -> Error {
    Error::new(
        Code::InvalidDuration,
        format!("{s:?} is not a duration: {why}"),
    )
}

/// Parse a duration by the grammar of section 9.4. Case-sensitive, integers
/// only, components in order, each at most once, at least one component,
/// and `T` requires at least one time component. Errors are
/// `invalid_duration`, also for a number that does not fit in 64 bits.
pub fn parse(s: &str) -> Result<Duration, Error> {
    let Some(mut rest) = s.strip_prefix('P') else {
        return Err(invalid(s, "it must start with P"));
    };
    let mut d = Duration::default();
    let mut any = false;

    // Date part: designators Y, M, W, D in this order.
    let date_order = *b"YMWD";
    let mut next_date = 0;
    while !rest.is_empty() && !rest.starts_with('T') {
        let (n, designator, tail) = component(s, rest)?;
        let Some(idx) = date_order[next_date..]
            .iter()
            .position(|&c| c == designator)
        else {
            return Err(invalid(
                s,
                "date components must be Y, M, W, D, in that order, each at most once",
            ));
        };
        let idx = next_date + idx;
        match designator {
            b'Y' => d.years = n,
            b'M' => d.months = n,
            b'W' => d.weeks = n,
            _ => d.days = n,
        }
        next_date = idx + 1;
        any = true;
        rest = tail;
    }

    if let Some(mut time) = rest.strip_prefix('T') {
        let time_order = *b"HMS";
        let mut next_time = 0;
        let mut any_time = false;
        while !time.is_empty() {
            let (n, designator, tail) = component(s, time)?;
            let Some(idx) = time_order[next_time..]
                .iter()
                .position(|&c| c == designator)
            else {
                return Err(invalid(
                    s,
                    "time components must be H, M, S, in that order, each at most once",
                ));
            };
            let idx = next_time + idx;
            match designator {
                b'H' => d.hours = n,
                b'M' => d.minutes = n,
                _ => d.seconds = n,
            }
            next_time = idx + 1;
            any_time = true;
            time = tail;
        }
        if !any_time {
            return Err(invalid(s, "T must be followed by at least one of H, M, S"));
        }
        any = true;
    }

    if !any {
        return Err(invalid(s, "it has no component"));
    }
    Ok(d)
}

/// One `n` followed by a designator letter. Returns the number, the
/// designator and the rest.
fn component<'a>(whole: &str, s: &'a str) -> Result<(u64, u8, &'a str), Error> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return Err(invalid(
            whole,
            "every component is a whole number followed by a letter",
        ));
    }
    let n: u64 = s[..digits]
        .parse()
        .map_err(|_| invalid(whole, "a number is too large"))?;
    let Some(&designator) = s.as_bytes().get(digits) else {
        return Err(invalid(whole, "a number has no designator letter"));
    };
    if !matches!(designator, b'Y' | b'M' | b'W' | b'D' | b'H' | b'S') {
        return Err(invalid(whole, "unknown designator"));
    }
    Ok((n, designator, &s[digits + 1..]))
}

/// The latest instant `add` may produce: 9999-12-31T23:59:59Z.
pub fn max_instant() -> DateTime {
    jiff::civil::date(9999, 12, 31).at(23, 59, 59, 0)
}

/// `add(instant, duration)` of section 9.4, in UTC:
///
/// 1. `months = 12 * Y + M`; if positive, move the calendar date forward by
///    that many months keeping the day of month, using the last day of a
///    shorter target month. The time of day is unchanged.
/// 2. Add `604800 * W + 86400 * D + 3600 * H + 60 * M + S` seconds.
///
/// Overflow, or a result past 9999-12-31T23:59:59Z, is `invalid_duration`.
/// The fraction of a second of `instant` is kept.
pub fn add(instant: DateTime, d: &Duration) -> Result<DateTime, Error> {
    let overflow = || {
        Error::new(
            Code::InvalidDuration,
            format!("adding the duration to {instant}Z goes past 9999-12-31T23:59:59Z"),
        )
    };
    let months = d
        .years
        .checked_mul(12)
        .and_then(|m| m.checked_add(d.months))
        .ok_or_else(overflow)?;
    let mut dt = instant;
    if months > 0 {
        let index = i64::from(dt.year()) * 12 + i64::from(dt.month()) - 1;
        let target = i64::try_from(months)
            .ok()
            .and_then(|m| index.checked_add(m))
            .ok_or_else(overflow)?;
        let year = i16::try_from(target.div_euclid(12)).map_err(|_| overflow())?;
        let month = i8::try_from(target.rem_euclid(12) + 1).map_err(|_| overflow())?;
        let first = Date::new(year, month, 1).map_err(|_| overflow())?;
        let day = dt.day().min(first.days_in_month());
        let date = Date::new(year, month, day).map_err(|_| overflow())?;
        dt = date.to_datetime(dt.time());
    }
    let seconds = u128::from(d.weeks) * 604_800
        + u128::from(d.days) * 86_400
        + u128::from(d.hours) * 3_600
        + u128::from(d.minutes) * 60
        + u128::from(d.seconds);
    let seconds = i64::try_from(seconds).map_err(|_| overflow())?;
    if seconds > 0 {
        dt = dt
            .checked_add(SignedDuration::from_secs(seconds))
            .map_err(|_| overflow())?;
    }
    if dt > max_instant() {
        return Err(overflow());
    }
    Ok(dt)
}

/// The expiry comparison of section 9.3: `now >= add(from, duration)`.
pub fn is_expired(from: DateTime, d: &Duration, now: DateTime) -> Result<bool, Error> {
    Ok(now >= add(from, d)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A UTC instant written with `Z`. Parsed as a civil date-time because
    /// `jiff::Timestamp` cannot hold the last day of year 9999 in UTC.
    fn dt(s: &str) -> DateTime {
        s.strip_suffix('Z').unwrap().parse().unwrap()
    }

    /// The message names the instant as the outputs write it, with `Z`.
    #[test]
    fn the_overflow_message_writes_the_instant_with_z() {
        let e = add(dt("9999-06-01T00:00:00Z"), &parse("P1Y").unwrap()).unwrap_err();
        assert_eq!(e.code, Code::InvalidDuration);
        assert_eq!(
            e.message,
            "adding the duration to 9999-06-01T00:00:00Z goes past 9999-12-31T23:59:59Z"
        );
    }

    #[test]
    fn grammar() {
        for ok in [
            "P1Y",
            "PT1S",
            "P0D",
            "P1Y2M3W4DT5H6M7S",
            "P1W1D",
            "P007D",
            "P18446744073709551615D",
        ] {
            parse(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for bad in [
            "",
            "P",
            "PT",
            "P1",
            "P1DT",
            "PT1D",
            "P1H",
            "P18446744073709551616D",
            "P1Y1Y",
            "P1D1M",
            "P1MT1H1H",
            "P+1D",
            "P1d",
            "P１D",
        ] {
            assert_eq!(parse(bad).unwrap_err().code, Code::InvalidDuration, "{bad}");
        }
    }

    #[test]
    fn range_limits() {
        let p = |s: &str| parse(s).unwrap();
        assert_eq!(
            add(dt("9999-12-30T23:59:59Z"), &p("P1D")).unwrap(),
            max_instant()
        );
        assert_eq!(
            add(dt("9999-12-31T23:59:59Z"), &p("PT1S"))
                .unwrap_err()
                .code,
            Code::InvalidDuration
        );
        assert_eq!(
            add(dt("2026-01-01T00:00:00Z"), &p("P8000Y"))
                .unwrap_err()
                .code,
            Code::InvalidDuration
        );
        assert!(add(dt("2026-01-01T00:00:00Z"), &p("P18446744073709551615D")).is_err());
        assert!(add(dt("2026-01-01T00:00:00Z"), &p("P18446744073709551615Y")).is_err());
        assert!(add(dt("2026-01-01T00:00:00Z"), &p("P1537228672809129301M")).is_err());
        let big = jiff::civil::date(9999, 12, 31).at(23, 59, 59, 0);
        assert_eq!(add(big, &p("P0D")).unwrap(), big);
        let past = jiff::civil::date(9999, 12, 31).at(23, 59, 59, 500_000_000);
        assert!(add(past, &p("P0D")).is_err());
    }
}
