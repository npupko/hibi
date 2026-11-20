//! `normalize.json`, `levenshtein.json`, `similarity.json` (spec section 4.3).

use hibi::text;

use crate::harness::{Case, expect_eq, str_field};

/// `input: {text}`, `expected`: string.
pub fn normalize(case: &Case) -> Result<(), String> {
    let want = case.expected.as_str().expect("expected is a string");
    expect_eq(
        "normalize",
        text::normalize(str_field(&case.input, "text")).as_str(),
        want,
    )
}

/// `input: {a, b}`, `expected`: number.
pub fn levenshtein(case: &Case) -> Result<(), String> {
    let want = case.expected.as_u64().expect("expected is an integer");
    let got = text::levenshtein(str_field(&case.input, "a"), str_field(&case.input, "b"));
    expect_eq("levenshtein", got as u64, want)
}

/// `input: {a, b}`, `expected`: binary64, compared exactly.
pub fn similarity(case: &Case) -> Result<(), String> {
    let want = case.expected.as_f64().expect("expected is a number");
    let got = text::similarity(str_field(&case.input, "a"), str_field(&case.input, "b"));
    expect_eq("similarity", got.to_bits(), want.to_bits())
        .map_err(|_| format!("similarity: got {got}, want {want}"))
}
