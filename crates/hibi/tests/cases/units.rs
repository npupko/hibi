//! `units-code.json` (spec sections 6.1, 6.4 to 6.6 and 16.3).

use hibi::code::{self, Analysis, Class};
use serde_json::{Value, json};

use crate::harness::{Case, cases_dir, expect_eq, str_field};

/// The `file` unit of section 6.2 as a case object.
fn file_unit(text: &str) -> Value {
    let last = text.char_indices().last().map_or(0, |(i, _)| i);
    json!({
        "kind": "file", "symbol": null, "key": "file", "parent": null,
        "start": 0, "end": text.len(), "line": 1,
        "endLine": hibi::text::line_at(text, last),
    })
}

/// A content key `<kind>:<12 hex>` with an optional `~n` matches the
/// expected `<kind>:*` with the same suffix: the 12 hex digits come from a
/// digest that the case author could not compute by hand.
fn key_matches(got: &str, want: &str) -> bool {
    let Some((kind, rest)) = want.split_once(":*") else {
        return got == want;
    };
    let Some(after) = got.strip_prefix(kind).and_then(|g| g.strip_prefix(':')) else {
        return false;
    };
    after.len() >= 12
        && after[..12]
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && &after[12..] == rest
}

/// Section 6.6 for content keys, which the case data writes as `<kind>:*`:
/// every `statement` and `block` key is `<kind>:` plus the first 12 hex
/// digits of that unit's own fingerprint, and the n-th unit (n >= 2) with
/// that base key in source order gets `~n`.
fn check_content_keys(units: &[(&str, &str, &hibi::model::Fingerprint)]) -> Result<(), String> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (kind, key, fp) in units {
        if !matches!(*kind, "statement" | "block") {
            continue;
        }
        let base = format!("{kind}:{}", &fp.hex()[..12]);
        let n = seen.entry(base.clone()).or_insert(0);
        *n += 1;
        let want = if *n == 1 { base } else { format!("{base}~{n}") };
        if *key != want {
            return Err(format!(
                "content key {key:?}, want {want:?} from the unit's fingerprint"
            ));
        }
    }
    Ok(())
}

/// `input: {fixture, path}`; `expected: {class, units}`, every unit in
/// source order as `{kind, symbol, key, parent, start, end, line, endLine}`.
pub fn units_code(case: &Case) -> Result<(), String> {
    let fixture = str_field(&case.input, "fixture");
    let text = std::fs::read_to_string(cases_dir().join(fixture))
        .map_err(|e| format!("{fixture}: {e}"))?;
    let class = Class::from_path(str_field(&case.input, "path"));
    match code::analyze(&text, class) {
        Analysis::Code(f) => check_content_keys(
            &f.units
                .iter()
                .map(|u| (u.kind, u.key.as_str(), &u.fp))
                .collect::<Vec<_>>(),
        )?,
        Analysis::Text(blocks) => check_content_keys(
            &blocks
                .iter()
                .map(|u| ("block", u.key.as_str(), &u.fp))
                .collect::<Vec<_>>(),
        )?,
    }
    let (class_name, mut got) = match code::analyze(&text, class) {
        Analysis::Code(f) => (
            f.class.name(),
            f.units
                .iter()
                .map(|u| {
                    json!({
                        "kind": u.kind, "symbol": u.symbol, "key": u.key, "parent": u.parent,
                        "start": u.start, "end": u.end, "line": u.line, "endLine": u.end_line,
                    })
                })
                .collect::<Vec<_>>(),
        ),
        Analysis::Text(blocks) => {
            let mut v = vec![file_unit(&text)];
            v.extend(blocks.iter().map(|u| {
                json!({
                    "kind": "block", "symbol": null, "key": u.key, "parent": "file",
                    "start": u.start, "end": u.end, "line": u.line, "endLine": u.end_line,
                })
            }));
            ("text", v)
        }
    };
    expect_eq("class", class_name, str_field(&case.expected, "class"))?;
    let want = case.expected["units"]
        .as_array()
        .ok_or("expected.units is not an array")?;
    // Replace a matching content key by the expected pattern, so that the
    // comparison below reports every other difference.
    for (g, w) in got.iter_mut().zip(want) {
        if let (Some(gk), Some(wk)) = (g["key"].as_str(), w["key"].as_str())
            && wk.contains(":*")
            && key_matches(gk, wk)
        {
            g["key"] = Value::from(wk);
        }
    }
    let mut diffs = Vec::new();
    for i in 0..got.len().max(want.len()) {
        let (g, w) = (got.get(i), want.get(i));
        if g != w {
            diffs.push(format!(
                "unit {i}:\n   got {}\n  want {}",
                g.map_or("-".into(), Value::to_string),
                w.map_or("-".into(), Value::to_string)
            ));
        }
    }
    if diffs.is_empty() {
        Ok(())
    } else {
        Err(diffs.join("\n"))
    }
}
