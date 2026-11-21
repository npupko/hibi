//! `freshness.json` (spec sections 9.2 and 9.3) and `coverage.json`
//! (section 10.2): the pure functions `engine::evaluate`,
//! `engine::covered_keys` and `engine::classify`.

use hibi::config::{Exclusion, ResourcePattern};
use hibi::engine::{self, Reason, Resolved, SideState, UnitCoverage};
use hibi::fingerprint::prose_v1;
use hibi::model::{Anchor, Claim, Fingerprint, Locator, Mark, MarkSide, Selector, Timestamp};
use hibi::provider::{NoProvider, ProviderErrorCode, Unit};
use serde_json::{Value, json};

use crate::harness::{Case, expect_eq, str_field};

fn unit(key: &str, kind: &str, parent: Option<&str>, fp: Fingerprint) -> Unit {
    Unit {
        key: key.to_owned(),
        kind: kind.to_owned(),
        parent: parent.map(str::to_owned),
        label: None,
        range: None,
        fp,
        sel: Selector::empty(),
        text: None,
        approx: false,
        refuse: None,
    }
}

fn failed(state: SideState, reason: Reason) -> Resolved {
    Resolved::Failed {
        state,
        reason,
        message: "case".to_owned(),
    }
}

/// One entry of `input.sides`: `{unit: <fp>}`, `{units: 0 | 2}`,
/// `{error: <provider code, or "failed">}`,
/// `{noProvider: "scheme_not_declared" | "provider_not_registered"}`, or
/// `{invalidUnit: true}`.
fn resolution(v: &Value) -> Resolved {
    if let Some(fp) = v.get("unit").and_then(Value::as_str) {
        let fp = Fingerprint::new(fp).unwrap();
        return Resolved::One(Box::new(unit("u", "k", None, fp)));
    }
    if let Some(n) = v.get("units").and_then(Value::as_u64) {
        return match n {
            0 => Resolved::NotFound,
            n => Resolved::Several(hibi::engine::Candidates {
                units: (0..n)
                    .map(|i| unit(&format!("u{i}"), "k", None, prose_v1(&i.to_string())))
                    .collect(),
                anchors: Vec::new(),
            }),
        };
    }
    if let Some(code) = v.get("error").and_then(Value::as_str) {
        let code = match code {
            "failed" => ProviderErrorCode::Failed,
            wire => {
                let c = ProviderErrorCode::from_wire(wire);
                assert_eq!(c.as_str(), wire, "not a provider error code");
                c
            }
        };
        let (state, reason) = engine::side_of_error(code);
        return failed(state, reason);
    }
    if let Some(np) = v.get("noProvider").and_then(Value::as_str) {
        let np = match np {
            "scheme_not_declared" => NoProvider::SchemeNotDeclared,
            "provider_not_registered" => NoProvider::ProviderNotRegistered,
            other => panic!("unknown noProvider {other}"),
        };
        return failed(SideState::Unavailable, engine::reason_of_no_provider(np));
    }
    if v.get("invalidUnit") == Some(&Value::Bool(true)) {
        // What the engine reports when section 2.5 validation fails.
        return failed(SideState::Unavailable, Reason::ProviderError);
    }
    panic!("unknown resolution {v}")
}

/// `input: {recheck, retired, now, marks: [{at, sides: [fp]}], sides}`,
/// `expected: {state, mark, expiresAt, sides: [{state, reason}]}`. `mark`
/// is the index of the reference mark in `input.marks`, `null`, or
/// `{greaterIdOf: [indexes]}` for a tie on `at`.
pub fn freshness(case: &Case) -> Result<(), String> {
    let input = &case.input;
    let marks_in = input["marks"].as_array().unwrap();
    let sides_in = input["sides"].as_array().unwrap();
    let retired = input["retired"].as_bool().unwrap();
    // A retired claim's sides are not resolved; its side count comes from
    // its marks.
    let count = if retired {
        marks_in
            .first()
            .map_or(1, |m| m["sides"].as_array().unwrap().len())
    } else {
        sides_in.len()
    };
    let loc = |i: usize| Locator::new(format!("t:s{i}")).unwrap();
    let mut anchors: Vec<Anchor> = (0..count)
        .map(|i| Anchor::new(loc(i), Selector::empty()))
        .collect();
    let subject = anchors.remove(0);
    let recheck = input["recheck"].as_str().map(str::to_owned);
    let claim = Claim::new("case", subject, anchors, recheck, None).map_err(|e| e.to_string())?;
    let marks: Vec<Mark> = marks_in
        .iter()
        .map(|m| {
            let sides = m["sides"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, fp)| MarkSide {
                    loc: loc(i),
                    fp: Fingerprint::new(fp.as_str().unwrap()).unwrap(),
                })
                .collect();
            let at = Timestamp::parse(str_field(m, "at")).unwrap();
            Mark::new(claim.id.clone(), at, None, sides, None).unwrap()
        })
        .collect();
    let mark_refs: Vec<&Mark> = marks.iter().collect();
    let resolved: Vec<Resolved> = sides_in.iter().map(resolution).collect();
    let now = Timestamp::parse(str_field(input, "now")).unwrap();

    let eval =
        engine::evaluate(&claim, &mark_refs, retired, &resolved, now).map_err(|e| e.to_string())?;

    let expected = &case.expected;
    expect_eq("state", eval.state.as_str(), str_field(expected, "state"))?;
    let want_mark = match &expected["mark"] {
        Value::Null => None,
        Value::Number(n) => Some(&marks[n.as_u64().unwrap() as usize]),
        tie => tie["greaterIdOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| &marks[i.as_u64().unwrap() as usize])
            .max_by(|a, b| a.id.cmp(&b.id)),
    };
    expect_eq(
        "reference mark",
        eval.reference.map(|m| &m.id),
        want_mark.map(|m| &m.id),
    )?;
    expect_eq(
        "expiresAt",
        json!(eval.expires_at),
        expected["expiresAt"].clone(),
    )?;
    let sides: Vec<Value> = eval
        .sides
        .iter()
        .map(|(state, reason)| json!({"state": state, "reason": reason}))
        .collect();
    expect_eq("sides", Value::Array(sides), expected["sides"].clone())
}

/// `input: {loc, units: [{key, kind, parent}], kinds, resolved: [key],
/// exclude: [{resource, unit}]}`, `expected: {covered, excluded, uncovered,
/// matched}`: the scope units by status in unit order, and the match count
/// of each exclusion.
pub fn coverage(case: &Case) -> Result<(), String> {
    let input = &case.input;
    let loc = Locator::new(str_field(input, "loc")).unwrap();
    let units: Vec<Unit> = input["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            let key = str_field(u, "key");
            unit(
                key,
                str_field(u, "kind"),
                u["parent"].as_str(),
                prose_v1(key),
            )
        })
        .collect();
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_owned())
            .collect()
    };
    let kinds = strings(&input["kinds"]);
    let resolved = strings(&input["resolved"]);
    let exclusions: Vec<Exclusion> = input["exclude"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| Exclusion {
            resource: ResourcePattern::parse(str_field(x, "resource")).unwrap(),
            unit: x["unit"].as_str().map(str::to_owned),
            reason: "case".to_owned(),
        })
        .collect();

    let covered_set = engine::covered_keys(&units, resolved.iter().map(String::as_str));
    let (mut covered, mut excluded, mut uncovered) = (Vec::new(), Vec::new(), Vec::new());
    let mut matched = vec![0u64; exclusions.len()];
    for u in units.iter().filter(|u| kinds.contains(&u.kind)) {
        match engine::classify(&loc, u, &covered_set, &exclusions) {
            UnitCoverage::Covered => covered.push(u.key.clone()),
            UnitCoverage::Excluded(i) => {
                excluded.push(u.key.clone());
                matched[i] += 1;
            }
            UnitCoverage::Uncovered => uncovered.push(u.key.clone()),
        }
    }
    let expected = &case.expected;
    expect_eq("covered", json!(covered), expected["covered"].clone())?;
    expect_eq("excluded", json!(excluded), expected["excluded"].clone())?;
    expect_eq("uncovered", json!(uncovered), expected["uncovered"].clone())?;
    expect_eq("matched", json!(matched), expected["matched"].clone())
}
