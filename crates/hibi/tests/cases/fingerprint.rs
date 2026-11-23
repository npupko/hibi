//! `fingerprint.json` (spec sections 8.3, 8.4 and 16.3).

use hibi::fingerprint;
use serde_json::Value;

use crate::harness::{Case, expect_eq, str_field};

/// `input: {profile: "prose-v1", text}` or
/// `input: {profile: "ast-v1", class, stream}`; `expected: {fingerprint}`.
pub fn fingerprint(case: &Case) -> Result<(), String> {
    let got = match str_field(&case.input, "profile") {
        "prose-v1" => fingerprint::prose_v1(str_field(&case.input, "text")),
        "ast-v1" => {
            let stream: Vec<&str> = case.input["stream"]
                .as_array()
                .ok_or("stream is not an array")?
                .iter()
                .map(|t| t.as_str().ok_or("stream token is not a string"))
                .collect::<Result<_, _>>()?;
            fingerprint::ast_v1(str_field(&case.input, "class"), &stream)
        }
        other => return Err(format!("unknown profile {other}")),
    };
    expect_eq(
        "fingerprint",
        got.as_str(),
        case.expected
            .get("fingerprint")
            .and_then(Value::as_str)
            .ok_or("expected.fingerprint missing")?,
    )
}
