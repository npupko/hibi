//! `hibi schema <name>` (spec section 15.3): JSON Schemas generated with
//! schemars from the types that read and write the data. The files under
//! `schemas/` are this module's output (spec section 2.7).

use hibi::engine::{
    ClaimInput, ClaimOutput, CoverOutput, Envelope, ErrorEnvelope, ListOutput, MarkInput,
    MarkOutput, ReanchorOutput, RetireOutput, StatusOutput,
};
use hibi::judge::{SuggestOutput, TriageOutput};
use hibi::model::{Anchor, CLAIM_ID_ARGUMENT_PATTERN, Claim, Mark, Retirement, Selector};
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
    Id(#[schemars(extend("pattern" = CLAIM_ID_ARGUMENT_PATTERN))] String),
    Mark(MarkInput),
}

/// Standard input of `mark --stdin`: a non-empty array.
#[derive(JsonSchema)]
#[schemars(rename = "MarkStdin")]
#[allow(dead_code)]
struct MarkStdin(#[schemars(length(min = 1))] Vec<MarkElement>);

/// Every schema name, in the order of section 15.3.
pub const NAMES: [&str; 27] = [
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
    "triage-output",
    "suggest-evidence-output",
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
        "triage-output" => schema_for!(Envelope<TriageOutput>),
        "suggest-evidence-output" => schema_for!(Envelope<SuggestOutput>),
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
                command != "completions",
                "{name}"
            );
        }
    }

    /// The limits that validation enforces are in the schemas, with the
    /// values of the model's constants.
    #[test]
    fn the_schemas_carry_the_limits_that_validation_enforces() {
        use hibi::model::{
            CLAIM_ID_ARGUMENT_PATTERN, DURATION_PATTERN, MAX_CONTEXT_CHARS, MAX_POS,
            MAX_QUOTE_CHARS,
        };
        let get = |name: &str, pointer: &str| -> serde_json::Value {
            serde_json::to_value(schema(name).unwrap())
                .unwrap()
                .pointer(pointer)
                .unwrap_or_else(|| panic!("{name} has no {pointer}"))
                .clone()
        };
        // The selector, alone and wherever it is embedded.
        for (name, at) in [
            ("selector", ""),
            ("anchor", "/$defs/Selector"),
            ("claim", "/$defs/Selector"),
            ("claim-input", "/$defs/Selector"),
            ("unit", "/$defs/Selector"),
            ("status-output", "/$defs/Selector"),
            ("cover-output", "/$defs/Selector"),
        ] {
            let p = |rest: &str| get(name, &format!("{at}/properties/{rest}"));
            assert_eq!(p("quote/minLength"), 1, "{name}");
            assert_eq!(p("quote/maxLength"), MAX_QUOTE_CHARS, "{name}");
            assert_eq!(p("prefix/maxLength"), MAX_CONTEXT_CHARS, "{name}");
            assert_eq!(p("suffix/maxLength"), MAX_CONTEXT_CHARS, "{name}");
            assert_eq!(p("pos/maximum"), MAX_POS, "{name}");
            assert_eq!(p("symbol/minLength"), 1, "{name}");
            assert_eq!(p("key/minLength"), 1, "{name}");
            assert_eq!(
                get(name, &format!("{at}/dependentRequired")),
                serde_json::json!({"prefix": ["quote"], "suffix": ["quote"], "pos": ["quote"]}),
                "{name}"
            );
            assert_eq!(get(name, &format!("{at}/additionalProperties")), false);
        }
        // Stored records.
        assert_eq!(get("claim", "/properties/statement/minLength"), 1);
        assert_eq!(get("claim", "/properties/note/minLength"), 1);
        assert_eq!(
            get("claim", "/properties/recheck/pattern"),
            DURATION_PATTERN
        );
        assert_eq!(
            get("claim", "/$defs/ClaimId/pattern"),
            "^c1-[0-9a-hjkmnp-tv-z]{20}$"
        );
        for record in ["mark", "retirement"] {
            assert_eq!(get(record, "/properties/by/minLength"), 1, "{record}");
            assert_eq!(get(record, "/properties/note/minLength"), 1, "{record}");
            assert!(get(record, "/$defs/Timestamp/pattern").is_string());
        }
        assert_eq!(get("mark", "/properties/sides/minItems"), 1);
        assert_eq!(
            get("mark", "/$defs/MarkId/pattern"),
            "^m1-[0-9a-hjkmnp-tv-z]{20}$"
        );
        assert!(get("mark", "/$defs/Fingerprint/pattern").is_string());
        assert_eq!(
            get("mark", "/$defs/Locator/pattern"),
            hibi::model::LOCATOR_PATTERN
        );
        assert_eq!(
            get("config", "/properties/version/const"),
            hibi::build_info::STORE_VERSION
        );
        assert_eq!(
            get("retirement", "/$defs/RetirementId/pattern"),
            "^r1-[0-9a-hjkmnp-tv-z]{20}$"
        );
        // Standard-input documents.
        let input = "/$defs/ClaimInput/properties";
        assert_eq!(
            get("claim-input", &format!("{input}/statement/minLength")),
            1
        );
        assert_eq!(
            get("claim-input", &format!("{input}/recheck/pattern")),
            DURATION_PATTERN
        );
        assert_eq!(
            get("mark-input", "/$defs/MarkInput/properties/claim/pattern"),
            CLAIM_ID_ARGUMENT_PATTERN
        );
        assert_eq!(
            get("mark-input", "/$defs/MarkStdinElement/anyOf/0/pattern"),
            CLAIM_ID_ARGUMENT_PATTERN
        );
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
