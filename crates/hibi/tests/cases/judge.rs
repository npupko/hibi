//! `cases/judge/` (spec section 16.3): request and response pairs for the
//! three question shapes of sections 14.3 to 14.5. The cases are synthetic,
//! written from the spec and the vendor's documented answer schemas; they
//! were not recorded from the live API.
//!
//! Each case checks that the request hibi builds equals `expected.request`
//! byte for byte as JSON, that every answer is read as `expected.answers`
//! says (`null`: unusable, which fails the request), and the values derived
//! from them.

use hibi::judge::{self, Answer, Band, EvidenceLabel, questions as q};
use serde_json::{Map, Value, json};

use crate::harness::Case;

fn request(state: Value, questions: Map<String, Value>) -> Value {
    json!({"model": judge::MODEL, "state": state, "questions": questions})
}

fn answer_json(a: &Option<Answer>) -> Value {
    match a {
        None => Value::Null,
        Some(Answer::Noul(p)) => json!({"noul": p}),
        Some(Answer::Choice { choice, p }) => json!({"choice": choice, "p": p}),
    }
}

/// Check the request, then read each answer of the response.
fn check_exchange(case: &Case, built: Value) -> Result<Map<String, Value>, String> {
    if built != case.expected["request"] {
        return Err(format!(
            "request\n  built:    {built}\n  expected: {}",
            case.expected["request"]
        ));
    }
    let response = &case.input["response"];
    let mut read = Map::new();
    for (name, question) in built["questions"].as_object().unwrap() {
        let raw = response["answers"]
            .get(name)
            .cloned()
            .unwrap_or(Value::Null);
        read.insert(
            name.clone(),
            answer_json(&judge::read_answer(question, &raw)),
        );
    }
    if Value::Object(read.clone()) != case.expected["answers"] {
        return Err(format!(
            "answers\n  read:     {}\n  expected: {}",
            Value::Object(read),
            case.expected["answers"]
        ));
    }
    Ok(read)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

pub fn triage(case: &Case) -> Result<(), String> {
    let input = &case.input;
    let units = input["units"].as_array().unwrap();
    let mut qs = Map::new();
    for (i, u) in units.iter().enumerate() {
        let headings = strings(&u["headings"]);
        let text = u["text"].as_str().unwrap();
        qs.insert(
            format!("q{i}.checkable"),
            q::triage_checkable(&headings, text),
        );
        qs.insert(
            format!("q{i}.evidence"),
            q::triage_evidence(&headings, text),
        );
    }
    let built = request(
        q::triage_state(input["document"].as_str().unwrap()),
        qs.clone(),
    );
    let read = check_exchange(case, built)?;
    let results = if read.values().any(Value::is_null) {
        Value::Null
    } else {
        let mut out = Vec::new();
        for i in 0..units.len() {
            let raw = |s: &str| &case.input["response"]["answers"][format!("q{i}.{s}")];
            let p = judge::read_answer(&qs[&format!("q{i}.checkable")], raw("checkable"))
                .and_then(|a| a.noul())
                .ok_or("no checkable")?;
            let e = judge::read_answer(&qs[&format!("q{i}.evidence")], raw("evidence"))
                .ok_or("no evidence")?;
            out.push(json!({
                "checkable": p,
                "band": serde_json::to_value(Band::of(p)).unwrap(),
                "evidence": serde_json::to_value(EvidenceLabel::of(&e)).unwrap(),
            }));
        }
        Value::Array(out)
    };
    if results != case.expected["results"] {
        return Err(format!(
            "results {results}, expected {}",
            case.expected["results"]
        ));
    }
    Ok(())
}

pub fn suggest_evidence(case: &Case) -> Result<(), String> {
    let input = &case.input;
    let mut qs = Map::new();
    for (i, c) in input["candidates"].as_array().unwrap().iter().enumerate() {
        qs.insert(
            format!("q{i}"),
            q::suggest_question(c["where"].as_str().unwrap(), c["code"].as_str().unwrap()),
        );
    }
    let built = request(q::suggest_state(input["statement"].as_str().unwrap()), qs);
    let read = check_exchange(case, built)?;
    let (results, none_fits) = if read.values().any(Value::is_null) {
        (Value::Null, Value::Null)
    } else {
        let ps: Vec<f64> = (0..read.len())
            .map(|i| read[&format!("q{i}")]["noul"].as_f64().unwrap())
            .collect();
        let results = ps
            .iter()
            .map(|p| json!({"p": p, "fits": *p >= q::FITS_AT}))
            .collect();
        (
            Value::Array(results),
            json!(!ps.iter().any(|p| *p >= q::FITS_AT)),
        )
    };
    if results != case.expected["results"] || none_fits != case.expected["noneFits"] {
        return Err(format!(
            "results {results} noneFits {none_fits}, expected {} {}",
            case.expected["results"], case.expected["noneFits"]
        ));
    }
    Ok(())
}

pub fn order(case: &Case) -> Result<(), String> {
    let input = &case.input;
    let mut qs = Map::new();
    qs.insert("q0".into(), q::order_question(&strings(&input["content"])));
    let built = request(q::order_state(input["statement"].as_str().unwrap()), qs);
    check_exchange(case, built).map(|_| ())
}
