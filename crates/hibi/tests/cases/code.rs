//! `ast-node.json` and `ast-equivalence.json` (spec sections 8.4 and 16.1).

use hibi::code::{self, Class};
use hibi::fingerprint;
use serde_json::{Value, json};

use crate::harness::{Case, cases_dir, expect_eq, str_field};

fn read_fixture(path: &str) -> Result<String, String> {
    std::fs::read_to_string(cases_dir().join(path)).map_err(|e| format!("{path}: {e}"))
}

fn class_of(input: &Value) -> Result<Class, String> {
    let name = str_field(input, "language");
    Class::from_name(name).ok_or_else(|| format!("unknown language {name}"))
}

fn byte_field(input: &Value, key: &str) -> Result<usize, String> {
    input[key]
        .as_u64()
        .map(|n| n as usize)
        .ok_or_else(|| format!("missing {key}"))
}

/// Snap the span of an `ast-node.json` input and return the node's kind,
/// byte range and stream, plus the class.
fn snap(input: &Value) -> Result<(Class, Value, Vec<String>), String> {
    let text = read_fixture(str_field(input, "fixture"))?;
    let class = class_of(input)?;
    let tree = code::parse(&text, class).ok_or("no grammar")?;
    let node = code::smallest_named_node(
        tree.root_node(),
        &text,
        byte_field(input, "startByte")?,
        byte_field(input, "endByte")?,
    );
    let stream = code::semantic_stream(node, &text);
    let shape = json!({
        "nodeType": node.kind(),
        "startByte": node.start_byte(),
        "endByte": node.end_byte(),
    });
    Ok((class, shape, stream))
}

/// `input: {fixture, language, quote, startByte, endByte}`; `expected:
/// {nodeType, startByte, endByte, semanticStream}`.
pub fn ast_node(case: &Case) -> Result<(), String> {
    let (_, shape, stream) = snap(&case.input)?;
    let e = &case.expected;
    expect_eq(
        "node",
        shape,
        json!({
            "nodeType": e["nodeType"],
            "startByte": e["startByte"],
            "endByte": e["endByte"],
        }),
    )?;
    expect_eq(
        "semanticStream",
        Value::from(stream),
        e["semanticStream"].clone(),
    )
}

/// `input: {a, b}`, each an `ast-node.json` input; `expected:
/// {semanticHashEqual}`: equality of the `ast-v1` digests.
pub fn ast_equivalence(case: &Case) -> Result<(), String> {
    let (ca, _, sa) = snap(&case.input["a"])?;
    let (cb, _, sb) = snap(&case.input["b"])?;
    let equal = fingerprint::ast_v1(ca.name(), &sa) == fingerprint::ast_v1(cb.name(), &sb);
    expect_eq(
        "semanticHashEqual",
        Value::from(equal),
        case.expected["semanticHashEqual"].clone(),
    )
}
