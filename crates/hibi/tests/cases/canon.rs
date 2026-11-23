//! `canon.json` and `ids.json` (spec sections 3.5 and 16.3, requirement R3).

use hibi::canon::{self, RecordKind};
use hibi::model::{Claim, Mark, Retirement};
use serde_json::Value;

use crate::harness::{Case, expect_eq, str_field};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `input: {json, record}`, `expected: {canonical, utf8}`. The JSON text is
/// parsed and serialized by RFC 8785. When `record` names a record type the
/// text also deserializes as that record and the record's own canonical
/// form equals the expected bytes.
pub fn canon(case: &Case) -> Result<(), String> {
    let json = str_field(&case.input, "json");
    let want = str_field(&case.expected, "canonical");
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("input does not parse: {e}"))?;
    let got = canon::to_canonical(&value).map_err(|e| e.to_string())?;
    expect_eq(
        "utf8",
        hex(&got),
        str_field(&case.expected, "utf8").to_owned(),
    )?;
    expect_eq(
        "canonical",
        String::from_utf8(got).unwrap(),
        want.to_owned(),
    )?;

    let typed = match case.input.get("record").and_then(Value::as_str) {
        None => return Ok(()),
        Some("claim") => serde_json::from_str::<Claim>(json).map(|r| r.canonical()),
        Some("mark") => serde_json::from_str::<Mark>(json).map(|r| r.canonical()),
        Some("retirement") => serde_json::from_str::<Retirement>(json).map(|r| r.canonical()),
        Some(other) => return Err(format!("unknown record type {other}")),
    };
    let bytes = typed
        .map_err(|e| format!("does not deserialize as a record: {e}"))?
        .map_err(|e| e.to_string())?;
    expect_eq(
        "record canonical",
        String::from_utf8(bytes).unwrap(),
        want.to_owned(),
    )
}

/// `input: {kind, body}`, `expected: {canonical, blake3, id}`.
pub fn ids(case: &Case) -> Result<(), String> {
    let body = &case.input["body"];
    let kind = match str_field(&case.input, "kind") {
        "claim" => RecordKind::Claim,
        "mark" => RecordKind::Mark,
        "retirement" => RecordKind::Retirement,
        other => return Err(format!("unknown kind {other}")),
    };
    let bytes = canon::to_canonical(body).map_err(|e| e.to_string())?;
    expect_eq(
        "canonical",
        String::from_utf8(bytes.clone()).unwrap(),
        str_field(&case.expected, "canonical").to_owned(),
    )?;
    expect_eq(
        "blake3",
        blake3::hash(&bytes).to_hex().to_string(),
        str_field(&case.expected, "blake3").to_owned(),
    )?;
    let want_id = str_field(&case.expected, "id");
    expect_eq(
        "id",
        canon::derive_id(kind, body)
            .map_err(|e| e.to_string())?
            .as_str(),
        want_id,
    )?;

    // The whole record deserializes (which checks the id against the body)
    // and its typed id derivation agrees.
    let mut record = body.clone();
    record["id"] = Value::from(want_id);
    let typed_id =
        match kind {
            RecordKind::Claim => serde_json::from_value::<Claim>(record)
                .map(|r| r.derive_id().map(|i| i.into_string())),
            RecordKind::Mark => serde_json::from_value::<Mark>(record)
                .map(|r| r.derive_id().map(|i| i.into_string())),
            RecordKind::Retirement => serde_json::from_value::<Retirement>(record)
                .map(|r| r.derive_id().map(|i| i.into_string())),
        }
        .map_err(|e| format!("the record does not deserialize: {e}"))?
        .map_err(|e| e.to_string())?;
    expect_eq("typed id", typed_id.as_str(), want_id)
}
