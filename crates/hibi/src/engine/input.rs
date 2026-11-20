//! Inputs of the write commands and the standard-input documents of
//! `claim --stdin` and `mark --stdin` (spec section 15.3).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Code, Error};
use crate::model::{Anchor, parse_compact};

/// An anchor as a caller gives it: compact syntax (section 5.5) or the
/// JSON object of section 5.3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum AnchorInput {
    Compact(String),
    Anchor(Anchor),
}

impl AnchorInput {
    /// The anchor. Errors are `invalid_anchor`.
    pub fn parse(&self) -> Result<Anchor, Error> {
        match self {
            AnchorInput::Compact(s) => parse_compact(s),
            AnchorInput::Anchor(a) => {
                a.sel.validate()?;
                Ok(a.clone())
            }
        }
    }

    /// The input as one command-line argument: the compact text, or the
    /// anchor as JSON.
    pub fn to_argument(&self) -> String {
        match self {
            AnchorInput::Compact(s) => s.clone(),
            AnchorInput::Anchor(a) => serde_json::to_string(a).unwrap_or_default(),
        }
    }
}

impl From<&str> for AnchorInput {
    fn from(s: &str) -> Self {
        AnchorInput::Compact(s.to_owned())
    }
}

impl From<Anchor> for AnchorInput {
    fn from(a: Anchor) -> Self {
        AnchorInput::Anchor(a)
    }
}

/// One claim to create: the arguments of `hibi claim`, or one element of
/// its standard-input array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimInput {
    pub statement: String,
    pub subject: AnchorInput,
    #[serde(default)]
    pub evidence: Vec<AnchorInput>,
    #[serde(default)]
    pub recheck: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub by: Option<String>,
}

/// One claim to mark: an argument of `hibi mark`, or one element of its
/// standard-input array (a claim id string, or this object).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkInput {
    /// A claim id or id prefix (section 3.6).
    pub claim: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub by: Option<String>,
}

impl MarkInput {
    pub fn new(claim: impl Into<String>) -> Self {
        MarkInput {
            claim: claim.into(),
            note: None,
            by: None,
        }
    }
}

/// The arguments of `hibi reanchor`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReanchorInput {
    /// A claim id or id prefix.
    pub claim: String,
    pub statement: Option<String>,
    pub subject: Option<AnchorInput>,
    /// `--evidence <n>=<anchor>`: replace evidence number `n` (1-based).
    pub evidence: Vec<(usize, AnchorInput)>,
    /// `--add-evidence <anchor>`.
    pub add_evidence: Vec<AnchorInput>,
    /// `--drop-evidence <n>` (1-based, numbers of the old list).
    pub drop_evidence: Vec<usize>,
    /// `--recheck <duration|none>` as given; `none` sets `null`.
    pub recheck: Option<String>,
    /// `--note <text|none>` as given; `none` sets `null`.
    pub note: Option<String>,
    pub by: Option<String>,
}

/// The arguments of `hibi retire`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetireInput {
    /// Claim ids or id prefixes.
    pub claims: Vec<String>,
    pub replaced_by: Option<String>,
    pub note: Option<String>,
    pub by: Option<String>,
}

fn invalid_json(message: impl Into<String>) -> Error {
    Error::new(Code::InvalidJson, message)
}

/// The elements of a standard-input array. Not JSON, not an array, or an
/// empty array is `invalid_json`.
fn elements(text: &str) -> Result<Vec<Value>, Vec<Error>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| vec![invalid_json(format!("standard input is not JSON: {e}"))])?;
    match value {
        Value::Array(items) if !items.is_empty() => Ok(items),
        Value::Array(_) => Err(vec![invalid_json("standard input is an empty array")]),
        _ => Err(vec![invalid_json("standard input is not a JSON array")]),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimWire {
    statement: String,
    subject: Value,
    #[serde(default)]
    evidence: Option<Vec<Value>>,
    #[serde(default)]
    recheck: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    by: Option<String>,
}

fn anchor_input(what: &str, value: Value) -> Result<AnchorInput, Error> {
    match value {
        Value::String(s) => Ok(AnchorInput::Compact(s)),
        v @ Value::Object(_) => serde_json::from_value::<Anchor>(v)
            .map(AnchorInput::Anchor)
            .map_err(|e| Error::invalid_anchor(format!("{what}: {e}"))),
        _ => Err(Error::invalid_anchor(format!(
            "{what}: an anchor is a compact string or an object with loc and sel"
        ))),
    }
}

/// Parse the standard input of `claim --stdin`: a non-empty JSON array of
/// [`ClaimInput`] elements. Every bad element is reported with its `index`:
/// `invalid_json` for a wrong shape or an unknown key, `invalid_anchor` for
/// an anchor object that does not parse.
pub fn parse_claim_stdin(text: &str) -> Result<Vec<ClaimInput>, Vec<Error>> {
    let mut inputs = Vec::new();
    let mut errors = Vec::new();
    for (index, element) in elements(text)?.into_iter().enumerate() {
        let parsed = serde_json::from_value::<ClaimWire>(element)
            .map_err(|e| invalid_json(format!("element {index}: {e}")))
            .and_then(|w| {
                let subject = anchor_input("subject", w.subject)?;
                let evidence = w
                    .evidence
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| anchor_input(&format!("evidence {}", i + 1), v))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ClaimInput {
                    statement: w.statement,
                    subject,
                    evidence,
                    recheck: w.recheck,
                    note: w.note,
                    by: w.by,
                })
            });
        match parsed {
            Ok(input) => inputs.push(input),
            Err(e) => errors.push(e.with_index(index)),
        }
    }
    if errors.is_empty() {
        Ok(inputs)
    } else {
        Err(errors)
    }
}

/// Parse the standard input of `mark --stdin`: a non-empty JSON array whose
/// elements are a claim id string or a [`MarkInput`] object. Every bad
/// element is reported as `invalid_json` with its `index`.
pub fn parse_mark_stdin(text: &str) -> Result<Vec<MarkInput>, Vec<Error>> {
    let mut inputs = Vec::new();
    let mut errors = Vec::new();
    for (index, element) in elements(text)?.into_iter().enumerate() {
        let parsed = match element {
            Value::String(claim) => Ok(MarkInput::new(claim)),
            v => serde_json::from_value::<MarkInput>(v)
                .map_err(|e| invalid_json(format!("element {index}: {e}"))),
        };
        match parsed {
            Ok(input) => inputs.push(input),
            Err(e) => errors.push(e.with_index(index)),
        }
    }
    if errors.is_empty() {
        Ok(inputs)
    } else {
        Err(errors)
    }
}
