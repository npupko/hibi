//! Every state, question, option and threshold the judge sends or applies
//! (sections 14.3 to 14.5), in one place so they can be audited together.
//!
//! The functions build the exact JSON of the spec. Item text passed in must
//! already be cut to [`super::MAX_ITEM_CHARS`] characters.

use serde_json::{Value, json};

/// `triage`: a Noul at or above this is band `likely` (decision 34).
pub const LIKELY_AT: f64 = 0.7;
/// `triage`: a Noul at or below this is band `unlikely` (decision 34).
pub const UNLIKELY_AT: f64 = 0.3;
/// `triage`: the chosen evidence option needs at least this probability,
/// else the label is `unsure` (decision 34).
pub const EVIDENCE_AT: f64 = 0.6;
/// `suggest-evidence`: a candidate with at least this probability fits.
pub const FITS_AT: f64 = 0.7;

/// The `ask` of the triage Noul.
pub const TRIAGE_CHECKABLE_ASK: &str = "The text states a fact about how the software behaves, is configured or is built, and that fact could be confirmed or refuted by reading source code or configuration.";
/// The `ask` of the triage Choice.
pub const TRIAGE_EVIDENCE_ASK: &str = "Where the fact in the text would be confirmed.";
/// The options of the triage Choice, in the spec's order.
pub const TRIAGE_EVIDENCE_OPTIONS: [(&str, &str); 5] = [
    ("code", "In program source code"),
    ("config", "In a configuration or build file"),
    ("doc", "In another document"),
    ("external", "Outside this repository"),
    ("none", "The text states no checkable fact"),
];
/// The `ask` of the suggest-evidence Noul.
pub const SUGGEST_ASK: &str = "This code determines whether the statement is true. Reading it would confirm or refute the statement.";
/// The `ask` of the judge-order Noul.
pub const ORDER_ASK: &str = "The statement is still true given this content.";

/// Question-name suffixes of the two triage questions.
pub const CHECKABLE: &str = "checkable";
pub const EVIDENCE: &str = "evidence";

/// `{"task":"hibi-triage-v1","document":<locator>}`
pub fn triage_state(document: &str) -> Value {
    json!({"task": "hibi-triage-v1", "document": document})
}

/// `q<i>.checkable` of section 14.3.
pub fn triage_checkable(headings: &[String], text: &str) -> Value {
    json!({
        "type": "noul",
        "instructions": {"ask": TRIAGE_CHECKABLE_ASK, "headings": headings, "text": text},
    })
}

/// `q<i>.evidence` of section 14.3.
pub fn triage_evidence(headings: &[String], text: &str) -> Value {
    let criteria: serde_json::Map<String, Value> = TRIAGE_EVIDENCE_OPTIONS
        .iter()
        .map(|(k, v)| ((*k).to_owned(), Value::from(*v)))
        .collect();
    json!({
        "type": "choice",
        "instructions": {"ask": TRIAGE_EVIDENCE_ASK, "headings": headings, "text": text},
        "criteria": criteria,
    })
}

/// `{"task":"hibi-suggest-evidence-v1","statement":<statement>}`
pub fn suggest_state(statement: &str) -> Value {
    json!({"task": "hibi-suggest-evidence-v1", "statement": statement})
}

/// `q<i>` of section 14.4. `place` is `<locator> <label>`.
pub fn suggest_question(place: &str, code: &str) -> Value {
    json!({
        "type": "noul",
        "instructions": {"ask": SUGGEST_ASK, "where": place, "code": code},
    })
}

/// `{"task":"hibi-order-v1","statement":<statement>}`
pub fn order_state(statement: &str) -> Value {
    json!({"task": "hibi-order-v1", "statement": statement})
}

/// `q0` of section 14.5: the current text of each side.
pub fn order_question(content: &[String]) -> Value {
    json!({
        "type": "noul",
        "instructions": {"ask": ORDER_ASK, "content": content},
    })
}
