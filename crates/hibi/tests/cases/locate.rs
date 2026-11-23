//! `error-budget.json`, `within-budget.json`, `bitap.json`,
//! `fuzzy-locate.json`, `locate.json` and `quote-selector.json` (spec
//! sections 6.7, 7 and 16.1). Where a case gives an offset twice, the
//! `*Byte` value is used (section 16).

use hibi::locate::{self, How, Located, PatternTooLong};
use serde_json::Value;

use crate::harness::{Case, expect_eq, str_field};

fn usize_field(v: &Value, key: &str) -> usize {
    v.get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("missing integer field {key:?} in {v}")) as usize
}

/// `input: {quoteLength}`, `expected`: number.
pub fn error_budget(case: &Case) -> Result<(), String> {
    let n = usize_field(&case.input, "quoteLength");
    expect_eq(
        "budget",
        locate::error_budget(n) as u64,
        case.expected.as_u64().unwrap(),
    )
}

/// `input: {located, exact}`, `expected`: boolean.
pub fn within_budget(case: &Case) -> Result<(), String> {
    let got = locate::within_budget(
        str_field(&case.input, "located"),
        str_field(&case.input, "exact"),
    );
    expect_eq("within", got, case.expected.as_bool().unwrap())
}

/// `input: {text, pattern, locByte}`, `expected: {indexByte}` (-1 for no
/// match) or `{error: "PatternTooLong"}`.
pub fn bitap(case: &Case) -> Result<(), String> {
    let got = locate::match_main(
        str_field(&case.input, "text"),
        str_field(&case.input, "pattern"),
        usize_field(&case.input, "locByte"),
    );
    match (case.expected.get("error"), got) {
        (Some(e), Err(PatternTooLong)) => expect_eq("error", e.as_str().unwrap(), "PatternTooLong"),
        (None, Ok(got)) => {
            let want = case.expected["indexByte"].as_i64().unwrap();
            expect_eq("indexByte", got.map_or(-1, |i| i as i64), want)
        }
        (_, got) => Err(format!("got {got:?}, want {}", case.expected)),
    }
}

/// `input: {text, quote: {exact, prefix, suffix}, biasByte}`,
/// `expected: {found, startByte, endByte}`.
pub fn fuzzy(case: &Case) -> Result<(), String> {
    let q = &case.input["quote"];
    let got = locate::fuzzy(
        str_field(&case.input, "text"),
        str_field(q, "exact"),
        str_field(q, "prefix"),
        str_field(q, "suffix"),
        usize_field(&case.input, "biasByte"),
    );
    let want = if case.expected["found"].as_bool().unwrap() {
        Some((
            usize_field(&case.expected, "startByte"),
            usize_field(&case.expected, "endByte"),
        ))
    } else {
        None
    };
    expect_eq("range", got.map(|s| (s.start, s.end)), want)
}

/// `input: {text, quote: {exact, prefix, suffix}, position?: {startByte}}`,
/// `expected: {status, how, startByte, endByte}` or `{status: "missing"}`.
pub fn locate(case: &Case) -> Result<(), String> {
    let q = &case.input["quote"];
    let pos = case
        .input
        .get("position")
        .map_or(0, |p| usize_field(p, "startByte"));
    let got = locate::locate(
        str_field(&case.input, "text"),
        str_field(q, "exact"),
        str_field(q, "prefix"),
        str_field(q, "suffix"),
        pos,
    );
    let status = match &got {
        Located::Found { .. } => "found",
        Located::Ambiguous { .. } => "ambiguous",
        Located::Missing => "missing",
    };
    expect_eq("status", status, str_field(&case.expected, "status"))?;
    if let Some(span) = got.span() {
        let how = match got.how() {
            Some(How::Exact) => "exact",
            Some(How::Fuzzy) => "fuzzy",
            None => unreachable!(),
        };
        expect_eq("how", how, str_field(&case.expected, "how"))?;
        expect_eq(
            "range",
            (span.start, span.end),
            (
                usize_field(&case.expected, "startByte"),
                usize_field(&case.expected, "endByte"),
            ),
        )?;
    }
    Ok(())
}

/// `input: {text, startByte, endByte}`, `expected: {exact, prefix, suffix,
/// position: {startByte, endByte}}` or `{error: "empty span"}`.
pub fn quote_selector(case: &Case) -> Result<(), String> {
    let got = locate::quote_selector(
        str_field(&case.input, "text"),
        usize_field(&case.input, "startByte"),
        usize_field(&case.input, "endByte"),
    );
    match (case.expected.get("error"), got) {
        (Some(e), Err(err)) => expect_eq("error", err.to_string().as_str(), e.as_str().unwrap()),
        (None, Ok(q)) => {
            let p = &case.expected["position"];
            expect_eq(
                "quote",
                q.quote.as_str(),
                str_field(&case.expected, "exact"),
            )?;
            expect_eq(
                "prefix",
                q.prefix.as_str(),
                str_field(&case.expected, "prefix"),
            )?;
            expect_eq(
                "suffix",
                q.suffix.as_str(),
                str_field(&case.expected, "suffix"),
            )?;
            expect_eq("pos", q.pos, usize_field(p, "startByte"))?;
            expect_eq("end", q.pos + q.quote.len(), usize_field(p, "endByte"))
        }
        (_, got) => Err(format!("got {got:?}, want {}", case.expected)),
    }
}
