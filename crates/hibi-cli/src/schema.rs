//! `hibi schema <name>` (spec section 15.3): JSON Schemas generated with
//! schemars from the types that read and write the data. The files under
//! `schemas/` are this module's output (spec section 2.7).

use hibi::engine::{
    ClaimInput, ClaimOutput, CoverOutput, Envelope, ErrorEnvelope, ListOutput, MarkInput,
    MarkOutput, ReanchorOutput, RetireOutput, StatusOutput,
};
use hibi::model::{Anchor, Claim, Mark, Retirement, Selector};
use hibi::provider::command::ConformanceCase;
use hibi::provider::{Request, Response, Unit};
use schemars::{JsonSchema, Schema, schema_for};

use crate::output::{InitOutput, ProvidersReply, SchemaOutput, VersionOutput};

/// Standard input of `claim --stdin`: a non-empty array of claims.
#[derive(JsonSchema)]
#[schemars(rename = "ClaimStdin")]
#[allow(dead_code)]
struct ClaimStdin(#[schemars(length(min = 1))] Vec<ClaimInput>);

/// One element of the standard input of `mark --stdin`: a claim id, or an
/// object with the id and an optional note and actor.
#[derive(JsonSchema)]
#[schemars(untagged, rename = "MarkStdinElement")]
#[allow(dead_code)]
enum MarkElement {
    Id(String),
    Mark(MarkInput),
}

/// Standard input of `mark --stdin`: a non-empty array.
#[derive(JsonSchema)]
#[schemars(rename = "MarkStdin")]
#[allow(dead_code)]
struct MarkStdin(#[schemars(length(min = 1))] Vec<MarkElement>);

/// Every schema name, in the order of section 15.3.
pub const NAMES: [&str; 25] = [
    "claim",
    "mark",
    "retirement",
    "anchor",
    "selector",
    "config",
    "user-config",
    "claim-input",
    "mark-input",
    "error",
    "unit",
    "provider-request",
    "provider-response",
    "conformance-case",
    "init-output",
    "claim-output",
    "status-output",
    "mark-output",
    "reanchor-output",
    "retire-output",
    "cover-output",
    "list-output",
    "providers-output",
    "schema-output",
    "version-output",
];

/// The schema with this name, or `None` for an unknown name.
pub fn schema(name: &str) -> Option<Schema> {
    Some(match name {
        "claim" => schema_for!(Claim),
        "mark" => schema_for!(Mark),
        "retirement" => schema_for!(Retirement),
        "anchor" => schema_for!(Anchor),
        "selector" => schema_for!(Selector),
        "config" => hibi::config::repo_config_schema(),
        "user-config" => hibi::config::user_config_schema(),
        "claim-input" => schema_for!(ClaimStdin),
        "mark-input" => schema_for!(MarkStdin),
        "error" => schema_for!(ErrorEnvelope),
        "unit" => schema_for!(Unit),
        "provider-request" => schema_for!(Request),
        "provider-response" => schema_for!(Response),
        "conformance-case" => schema_for!(ConformanceCase),
        "init-output" => schema_for!(Envelope<InitOutput>),
        "claim-output" => schema_for!(Envelope<ClaimOutput>),
        "status-output" => schema_for!(Envelope<StatusOutput>),
        "mark-output" => schema_for!(Envelope<MarkOutput>),
        "reanchor-output" => schema_for!(Envelope<ReanchorOutput>),
        "retire-output" => schema_for!(Envelope<RetireOutput>),
        "cover-output" => schema_for!(Envelope<CoverOutput>),
        "list-output" => schema_for!(Envelope<ListOutput>),
        "providers-output" => schema_for!(Envelope<ProvidersReply>),
        "schema-output" => schema_for!(Envelope<SchemaOutput>),
        "version-output" => schema_for!(Envelope<VersionOutput>),
        _ => return None,
    })
}

/// The schema as `hibi schema <name>` prints it and as
/// `schemas/<name>.schema.json` holds it: indented JSON and a final newline.
pub fn text(name: &str) -> Option<String> {
    let schema = schema(name)?;
    let mut text = serde_json::to_string_pretty(&schema).ok()?;
    text.push('\n');
    Some(text)
}

/// The file name of a schema under `schemas/`.
pub fn file_name(name: &str) -> String {
    format!("{name}.schema.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_has_a_draft_2020_12_schema() {
        for name in NAMES {
            let schema = schema(name).unwrap_or_else(|| panic!("{name} has no schema"));
            let value = serde_json::to_value(&schema).unwrap();
            assert_eq!(
                value["$schema"], "https://json-schema.org/draft/2020-12/schema",
                "{name}"
            );
        }
        assert!(schema("nope").is_none());
    }

    #[test]
    fn names_are_unique() {
        let mut names = NAMES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), NAMES.len());
    }

    #[test]
    fn every_command_with_json_output_has_an_output_schema() {
        for command in crate::cli::command_names() {
            let name = format!("{command}-output");
            assert_eq!(
                NAMES.contains(&name.as_str()),
                !["completions", "triage", "suggest-evidence"].contains(&command.as_str()),
                "{name}"
            );
        }
    }

    #[test]
    fn the_stdin_documents_are_non_empty_arrays() {
        for name in ["claim-input", "mark-input"] {
            let v = serde_json::to_value(schema(name).unwrap()).unwrap();
            assert_eq!(v["type"], "array", "{name}");
            assert_eq!(v["minItems"], 1, "{name}");
        }
    }
}
