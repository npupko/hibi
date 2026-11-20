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
use serde_json::{Map, Value};

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
///
/// The schema is generated from the type that is read or written, and then
/// completed with what the envelope contract of section 15.1 states and the
/// generator cannot derive: a title that names the document, `ok` as a
/// constant, the closed top level of an envelope (a struct with a flattened
/// field is generated open), and at least one entry in `errors`.
pub fn schema(name: &str) -> Option<Schema> {
    let mut schema = generated(name)?;
    let object = schema.ensure_object();
    if name == "error" {
        close_envelope(object, false);
        if let Some(Value::Object(errors)) = object
            .get_mut("properties")
            .and_then(|p| p.get_mut("errors"))
        {
            errors.insert("minItems".to_owned(), 1.into());
        }
    } else if name.ends_with("-output") {
        close_envelope(object, true);
    }
    if name == "providers-output" {
        // `check` is absent without `--check` and never `null`; the
        // generator describes an optional field as its type or `null`.
        if let Some(check) = object
            .get_mut("properties")
            .and_then(|p| p.get_mut("check"))
            && let Some(present) = check
                .get("anyOf")
                .and_then(Value::as_array)
                .and_then(|forms| forms.iter().find(|f| f.get("$ref").is_some()))
                .cloned()
        {
            *check = present;
        }
    }
    object.insert("title".to_owned(), title(name).into());
    Some(schema)
}

/// The title of a schema: what the document is, in the words of the
/// command line, not the name of the type it was generated from.
fn title(name: &str) -> String {
    match name {
        "config" => "hibi.toml".to_owned(),
        "user-config" => "hibi user config".to_owned(),
        "error" => "hibi error output".to_owned(),
        "claim-input" => "hibi claim --stdin input".to_owned(),
        "mark-input" => "hibi mark --stdin input".to_owned(),
        "claim" | "mark" | "retirement" => format!("hibi {name} record"),
        "provider-request" => "hibi provider request".to_owned(),
        "provider-response" => "hibi provider response".to_owned(),
        "conformance-case" => "hibi provider conformance case".to_owned(),
        other => match other.strip_suffix("-output") {
            Some(command) => format!("hibi {command} output"),
            None => format!("hibi {other}"),
        },
    }
}

/// Make the schema of an envelope say what section 15.1 states: `ok` is
/// the constant `ok`, and no key other than the declared ones appears.
///
/// An output with several shapes (`list`) is generated as the envelope keys
/// beside an `anyOf` of the shapes. Each shape is completed here with the
/// envelope keys and closed, so that every entry of `anyOf` is one whole
/// result with its own required keys.
fn close_envelope(object: &mut Map<String, Value>, ok: bool) {
    if let Some(Value::Object(ok_schema)) =
        object.get_mut("properties").and_then(|p| p.get_mut("ok"))
    {
        ok_schema.insert("const".to_owned(), ok.into());
    }
    let Some(Value::Array(shapes)) = object.get_mut("anyOf") else {
        object.insert("additionalProperties".to_owned(), false.into());
        return;
    };
    let mut shapes = std::mem::take(shapes);
    let properties = object.remove("properties");
    let required = object.remove("required");
    for shape in shapes.iter_mut().filter_map(Value::as_object_mut) {
        let mut all = match &properties {
            Some(Value::Object(envelope)) => envelope.clone(),
            _ => Map::new(),
        };
        if let Some(Value::Object(own)) = shape.remove("properties") {
            all.extend(own);
        }
        let mut names = match &required {
            Some(Value::Array(envelope)) => envelope.clone(),
            _ => Vec::new(),
        };
        if let Some(Value::Array(own)) = shape.remove("required") {
            names.extend(own);
        }
        shape.insert("properties".to_owned(), all.into());
        shape.insert("required".to_owned(), names.into());
        shape.insert("additionalProperties".to_owned(), false.into());
    }
    object.insert("anyOf".to_owned(), shapes.into());
}

/// The schema as schemars generates it from the type.
fn generated(name: &str) -> Option<Schema> {
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

    /// Section 15.1: `ok` is a constant, an envelope has no key besides
    /// the declared ones, and `errors` is never empty.
    #[test]
    fn the_envelope_schemas_are_closed_and_ok_is_a_constant() {
        for name in NAMES {
            let v = serde_json::to_value(schema(name).unwrap()).unwrap();
            let ok = if name == "error" {
                false
            } else if name.ends_with("-output") {
                true
            } else {
                continue;
            };
            // One closed shape, or one per entry of `anyOf`.
            let shapes = match v.get("anyOf").and_then(Value::as_array) {
                Some(shapes) => {
                    assert!(v.get("properties").is_none(), "{name}");
                    shapes.clone()
                }
                None => vec![v.clone()],
            };
            for shape in shapes {
                assert_eq!(shape["type"], "object", "{name}");
                assert_eq!(shape["additionalProperties"], false, "{name}");
                assert_eq!(shape["properties"]["ok"]["const"], ok, "{name}");
                assert!(shape["properties"]["command"].is_object(), "{name}");
                let required = shape["required"].as_array().unwrap();
                for key in ["ok", "command", if ok { "warnings" } else { "errors" }] {
                    assert!(required.contains(&key.into()), "{name}: {key}");
                }
                // Every required key is declared.
                for key in required {
                    assert!(
                        shape["properties"].get(key.as_str().unwrap()).is_some(),
                        "{name}: {key}"
                    );
                }
            }
        }
        let error = serde_json::to_value(schema("error").unwrap()).unwrap();
        assert_eq!(error["properties"]["errors"]["minItems"], 1);
        assert_eq!(
            error["$defs"]["Error"]["required"],
            serde_json::json!(["code", "message", "remediation"])
        );
        assert_eq!(
            error["$defs"]["Error"]["properties"]["remediation"]["type"],
            serde_json::json!(["string", "null"])
        );
    }

    /// `list` has two results; each entry of `anyOf` is one of them, whole.
    #[test]
    fn the_list_schema_states_the_required_keys_of_each_shape() {
        let v = serde_json::to_value(schema("list-output").unwrap()).unwrap();
        let required: Vec<&Value> = v["anyOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|shape| &shape["required"])
            .collect();
        assert_eq!(
            required,
            [
                &serde_json::json!(["ok", "command", "claims", "warnings"]),
                &serde_json::json!(["ok", "command", "claim", "marks", "retirements", "warnings"]),
            ]
        );
    }

    /// `check` of `providers` is an object or absent, and its `hello` is
    /// described by the `hello` result of the protocol.
    #[test]
    fn the_providers_schema_describes_check_and_its_hello() {
        let v = serde_json::to_value(schema("providers-output").unwrap()).unwrap();
        assert_eq!(
            v["properties"]["check"],
            serde_json::json!({"$ref": "#/$defs/ProviderCheck"})
        );
        assert!(
            !v["required"]
                .as_array()
                .unwrap()
                .contains(&Value::from("check"))
        );
        let check = &v["$defs"]["ProviderCheck"];
        assert!(
            check["required"]
                .as_array()
                .unwrap()
                .contains(&Value::from("hello"))
        );
        assert_eq!(
            check["properties"]["hello"]["anyOf"][0],
            serde_json::json!({"$ref": "#/$defs/ProviderInfo"})
        );
        assert_eq!(check["properties"]["hello"]["anyOf"][1]["type"], "null");
        for key in ["protocol", "provider", "schemes", "selectors", "kinds"] {
            assert!(
                v["$defs"]["ProviderInfo"]["properties"][key].is_object(),
                "{key}"
            );
        }
    }

    /// A title names the document, not the type it was generated from.
    #[test]
    fn titles_name_the_document() {
        let title = |name: &str| {
            serde_json::to_value(schema(name).unwrap()).unwrap()["title"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(title("config"), "hibi.toml");
        assert_eq!(title("user-config"), "hibi user config");
        assert_eq!(title("error"), "hibi error output");
        assert_eq!(title("status-output"), "hibi status output");
        assert_eq!(title("claim-input"), "hibi claim --stdin input");
        assert_eq!(title("claim"), "hibi claim record");
        assert_eq!(title("anchor"), "hibi anchor");
        let mut titles: Vec<String> = NAMES.iter().map(|n| title(n)).collect();
        assert!(titles.iter().all(|t| t.starts_with("hibi")), "{titles:?}");
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), NAMES.len(), "titles are unique");
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
