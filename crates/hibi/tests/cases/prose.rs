//! `regions.json` (spec sections 6.3 and 16.1, requirement R5).

use hibi::prose::{self, ProseClass};
use serde_json::{Value, json};

use crate::harness::{Case, cases_dir, expect_eq};

/// The class of `input.path` by section 16.1: `.md` and `.mdx` (compared
/// lower-cased) are `markdown`; anything else, and `null`, is `prose`.
fn class_of(path: Option<&str>) -> ProseClass {
    let ext = path
        .and_then(|p| p.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("md" | "mdx") => ProseClass::Markdown,
        _ => ProseClass::Prose,
    }
}

/// `input: {text, path}` or `{fixture, path}`; `expected`: array of
/// `{kind, text, startByte, endByte, line, endLine?, headingPath}` (the
/// UTF-16 `start` and `end` are reference only). `endLine` is compared when
/// the expected unit has it; the cases extracted from v4 do not.
pub fn regions(case: &Case) -> Result<(), String> {
    let text = match case.input.get("fixture").and_then(Value::as_str) {
        Some(f) => std::fs::read_to_string(cases_dir().join(f)).map_err(|e| format!("{f}: {e}"))?,
        None => case.input["text"]
            .as_str()
            .ok_or("input.text missing")?
            .to_owned(),
    };
    let class = class_of(case.input["path"].as_str());
    let got: Vec<Value> = prose::split(&text, class)
        .iter()
        .map(|r| {
            json!({
                "kind": r.kind.as_str(),
                "text": &text[r.start..r.end],
                "startByte": r.start,
                "endByte": r.end,
                "line": r.line,
                "endLine": r.end_line,
                "headingPath": r.heading_path,
            })
        })
        .collect();
    let want: Vec<Value> = case
        .expected
        .as_array()
        .ok_or("expected is not an array")?
        .iter()
        .map(|u| {
            let mut w = json!({
                "kind": u["kind"],
                "text": u["text"],
                "startByte": u["startByte"],
                "endByte": u["endByte"],
                "line": u["line"],
                "headingPath": u["headingPath"],
            });
            if let Some(end_line) = u.get("endLine") {
                w["endLine"] = end_line.clone();
            }
            w
        })
        .collect();
    if got.len() != want.len() {
        return Err(format!(
            "{} units, want {}:\n got {}\nwant {}",
            got.len(),
            want.len(),
            Value::from(got),
            Value::from(want)
        ));
    }
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        let mut g = g.clone();
        if w.get("endLine").is_none() {
            g.as_object_mut()
                .expect("built as an object")
                .remove("endLine");
        }
        expect_eq(&format!("unit {i}"), &g, w)?;
    }
    Ok(())
}
