//! Asking questions: the cache of section 14.6, packing into requests by
//! the limits of section 14.2, and reading the answers.
//!
//! The unit of work is an item: one or two questions about one thing, asked
//! over the state of its group. A group is a set of items that share one
//! state; requests never mix groups. Items are packed in input order; the
//! questions of one item always go in the same request.
//!
//! Size accounting: a request holds at most
//! [`super::MAX_QUESTIONS_PER_REQUEST`] questions and at most
//! [`super::MAX_QUESTION_BYTES_PER_REQUEST`] bytes of serialized
//! `questions` (the exact byte length of the JSON object, names, quotes,
//! colons, commas and braces included). Bytes stand in for Jev's token
//! limits (64k per request, 32k per state plus one question): a token of a
//! byte-level tokenizer covers at least one byte. An item whose questions
//! alone exceed the byte limit is sent alone in a request of its own; if
//! Jev refuses it, the command fails with `judge_failed`.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::cache::{Cache, JudgeEntry};
use crate::canon;
use crate::error::{Code, Error};
use crate::model::Timestamp;

use super::client::send_all;
use super::{Judge, MAX_QUESTION_BYTES_PER_REQUEST, MAX_QUESTIONS_PER_REQUEST, MODEL};

/// One question of an item: the name suffix (empty for a lone question,
/// else `q<i>.<suffix>`) and the question object.
pub(super) struct Question {
    pub suffix: &'static str,
    pub body: Value,
}

/// Items that share one state.
pub(super) struct Group {
    pub state: Value,
    pub items: Vec<Vec<Question>>,
}

/// An answer, as hibi reads it: the Noul probability, or the chosen option
/// with its probability. `confidence` is never read.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Noul(f64),
    Choice { choice: String, p: f64 },
}

impl Answer {
    /// The Noul probability, or `None` for a Choice.
    pub fn noul(&self) -> Option<f64> {
        match self {
            Answer::Noul(p) => Some(*p),
            Answer::Choice { .. } => None,
        }
    }
}

/// The key of section 14.6: the 64 hex digits of
/// `BLAKE3(JCS({"model": model, "state": state, "question": question}))`.
pub fn cache_key(model: &str, state: &Value, question: &Value) -> Result<String, Error> {
    let mut m = Map::new();
    m.insert("model".into(), Value::from(model));
    m.insert("state".into(), state.clone());
    m.insert("question".into(), question.clone());
    let bytes = canon::to_canonical(&Value::Object(m))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn probability(v: Option<&Value>) -> Option<f64> {
    let p = v?.as_f64()?;
    (p.is_finite() && (0.0..=1.0).contains(&p)).then_some(p)
}

/// Read an answer of the type `question` asks for (section 14.2): `noul`,
/// or `choice` with `probabilities[choice]`. `None` when the answer lacks
/// the type, the field or a probability in [0, 1], or names an option the
/// question does not have; a request with such an answer fails.
pub fn read_answer(question: &Value, answer: &Value) -> Option<Answer> {
    let kind = question.get("type")?.as_str()?;
    if answer.get("type")?.as_str()? != kind {
        return None;
    }
    match kind {
        "noul" => probability(answer.get("noul")).map(Answer::Noul),
        "choice" => {
            let choice = answer.get("choice")?.as_str()?;
            question.get("criteria")?.get(choice)?;
            let p = probability(answer.get("probabilities")?.get(choice))?;
            Some(Answer::Choice {
                choice: choice.to_owned(),
                p,
            })
        }
        _ => None,
    }
}

/// Where a question sits: group, item, question.
type Slot = (usize, usize, usize);

/// One request: its body and, per question name, its slot.
struct Request {
    body: Vec<u8>,
    names: BTreeMap<String, Slot>,
}

fn name(position: usize, suffix: &str) -> String {
    if suffix.is_empty() {
        format!("q{position}")
    } else {
        format!("q{position}.{suffix}")
    }
}

fn json_len(v: &impl serde::Serialize) -> Result<usize, Error> {
    serde_json::to_vec(v)
        .map(|b| b.len())
        .map_err(|e| Error::internal(format!("a judge question did not serialize: {e}")))
}

fn body(state: &Value, questions: Map<String, Value>) -> Result<Vec<u8>, Error> {
    let mut m = Map::new();
    m.insert("model".into(), Value::from(MODEL));
    m.insert("state".into(), state.clone());
    m.insert("questions".into(), Value::Object(questions));
    serde_json::to_vec(&Value::Object(m))
        .map_err(|e| Error::internal(format!("a judge request did not serialize: {e}")))
}

/// Pack the pending questions of one group into requests.
fn pack(
    g: usize,
    group: &Group,
    pending: &[(usize, Vec<usize>)],
    out: &mut Vec<Request>,
) -> Result<(), Error> {
    let mut questions = Map::new();
    let mut names = BTreeMap::new();
    // Serialized length of `questions` so far: `{}` plus entries and commas.
    let mut bytes = 2usize;
    let mut position = 0usize;
    for (item, qs) in pending {
        let mut entries = Vec::with_capacity(qs.len());
        let entries_len = |position: usize, entries: &mut Vec<(String, Slot)>| {
            entries.clear();
            let mut len = 0usize;
            for &q in qs {
                let n = name(position, group.items[*item][q].suffix);
                len += json_len(&n)? + 1 + json_len(&group.items[*item][q].body)? + 1;
                entries.push((n, (g, *item, q)));
            }
            Ok::<usize, Error>(len)
        };
        let mut len = entries_len(position, &mut entries)?;
        let full = !names.is_empty()
            && (names.len() + qs.len() > MAX_QUESTIONS_PER_REQUEST
                || bytes + len - 1 > MAX_QUESTION_BYTES_PER_REQUEST);
        if full {
            out.push(Request {
                body: body(&group.state, std::mem::take(&mut questions))?,
                names: std::mem::take(&mut names),
            });
            bytes = 2;
            position = 0;
            len = entries_len(position, &mut entries)?;
        }
        for (n, slot) in entries {
            questions.insert(n.clone(), group.items[slot.1][slot.2].body.clone());
            names.insert(n, slot);
        }
        bytes += len;
        position += 1;
    }
    if !names.is_empty() {
        out.push(Request {
            body: body(&group.state, questions)?,
            names,
        });
    }
    Ok(())
}

/// Read a response body: every question of the request must have an answer
/// of its type, and the answering model must be [`MODEL`].
fn read_response(
    groups: &[Group],
    request: &Request,
    index: usize,
    total: usize,
    text: &str,
) -> Result<Vec<(Slot, Value, Answer)>, Error> {
    let bad = |why: String| {
        Error::new(
            Code::JudgeFailed,
            format!(
                "request {} of {total}: Jev's answer is unusable: {why}",
                index + 1
            ),
        )
    };
    let v: Value = serde_json::from_str(text).map_err(|e| bad(format!("it is not JSON ({e})")))?;
    match v.get("model").and_then(Value::as_str) {
        Some(MODEL) => {}
        Some(other) => return Err(bad(format!("it came from model {other:?}, not {MODEL}"))),
        None => return Err(bad("it names no model".into())),
    }
    let answers = v
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(|| bad("it has no answers object".into()))?;
    let mut out = Vec::with_capacity(request.names.len());
    for (n, &(g, i, q)) in &request.names {
        let question = &groups[g].items[i][q].body;
        let raw = answers
            .get(n)
            .ok_or_else(|| bad(format!("question {n} has no answer")))?;
        let answer = read_answer(question, raw).ok_or_else(|| {
            bad(format!(
                "the answer to {n} is not a valid {} answer",
                question["type"]
            ))
        })?;
        out.push(((g, i, q), raw.clone(), answer));
    }
    Ok(out)
}

/// Ask every question of `groups` and return the answers, per group, per
/// item, per question, in input order.
///
/// A question with a usable cached answer is not sent (R25). Each request
/// that succeeds has its answers stored in the cache, also when another
/// request fails; a failed request stores nothing. Any failed request fails
/// the whole call with `judge_failed` and no answers (R26). A cache that
/// cannot be read or written behaves as an empty cache.
pub(super) fn ask(
    judge: &Judge,
    cache: Option<&Cache>,
    now: Timestamp,
    groups: &[Group],
) -> Result<Vec<Vec<Vec<Answer>>>, Error> {
    let mut answers: Vec<Vec<Vec<Option<Answer>>>> = groups
        .iter()
        .map(|g| g.items.iter().map(|qs| vec![None; qs.len()]).collect())
        .collect();
    let mut keys: Vec<Vec<Vec<String>>> = Vec::with_capacity(groups.len());
    let mut requests = Vec::new();
    for (g, group) in groups.iter().enumerate() {
        let mut group_keys = Vec::with_capacity(group.items.len());
        let mut pending: Vec<(usize, Vec<usize>)> = Vec::new();
        for (i, qs) in group.items.iter().enumerate() {
            let mut item_keys = Vec::with_capacity(qs.len());
            let mut missing = Vec::new();
            for (q, question) in qs.iter().enumerate() {
                let key = cache_key(MODEL, &group.state, &question.body)?;
                let hit = cache
                    .and_then(|c| c.judge(&key).ok().flatten())
                    .filter(|e| e.model == MODEL)
                    .and_then(|e| serde_json::from_str::<Value>(&e.answer).ok())
                    .and_then(|a| read_answer(&question.body, &a));
                match hit {
                    Some(a) => answers[g][i][q] = Some(a),
                    None => missing.push(q),
                }
                item_keys.push(key);
            }
            if !missing.is_empty() {
                pending.push((i, missing));
            }
            group_keys.push(item_keys);
        }
        keys.push(group_keys);
        pack(g, group, &pending, &mut requests)?;
    }

    let bodies: Vec<Vec<u8>> = requests.iter().map(|r| r.body.clone()).collect();
    let total = requests.len();
    let parse =
        |index: usize, text: String| read_response(groups, &requests[index], index, total, &text);
    let results = send_all(judge, &bodies, &parse);

    let mut first_error = None;
    let at = now.to_string();
    for result in results.into_iter().flatten() {
        match result {
            Ok(read) => {
                for ((g, i, q), raw, answer) in read {
                    if let Some(c) = cache {
                        let entry = JudgeEntry {
                            model: MODEL.to_owned(),
                            answer: raw.to_string(),
                            at: at.clone(),
                        };
                        let _ = c.put_judge(&keys[g][i][q], &entry);
                    }
                    answers[g][i][q] = Some(answer);
                }
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }
    answers
        .into_iter()
        .map(|items| {
            items
                .into_iter()
                .map(|qs| qs.into_iter().collect::<Option<Vec<Answer>>>())
                .collect::<Option<Vec<_>>>()
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| Error::internal("a judge question was neither cached nor answered"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_is_blake3_of_jcs() {
        let k = cache_key(MODEL, &json!({"b": 1, "a": "x"}), &json!({"type": "noul"})).unwrap();
        let expected = blake3::hash(
            br#"{"model":"jev-1.14.0","question":{"type":"noul"},"state":{"a":"x","b":1}}"#,
        );
        assert_eq!(k, expected.to_hex().to_string());
        assert_eq!(k.len(), 64);
    }

    #[test]
    fn answers_are_read_by_type() {
        let noul = json!({"type": "noul"});
        let choice = json!({"type": "choice", "criteria": {"a": "A", "b": "B"}});
        assert_eq!(
            read_answer(&noul, &json!({"type": "noul", "noul": 0.25})),
            Some(Answer::Noul(0.25))
        );
        assert_eq!(
            read_answer(&noul, &json!({"type": "noul", "noul": 1.5})),
            None
        );
        assert_eq!(read_answer(&noul, &json!({"type": "noul"})), None);
        assert_eq!(
            read_answer(&noul, &json!({"type": "choice", "choice": "a"})),
            None
        );
        assert_eq!(
            read_answer(
                &choice,
                &json!({"type": "choice", "choice": "b", "confidence": 0.1,
                        "probabilities": {"a": 0.2, "b": 0.8}})
            ),
            Some(Answer::Choice {
                choice: "b".into(),
                p: 0.8
            })
        );
        // An option the question does not have, or no probability for it.
        assert_eq!(
            read_answer(
                &choice,
                &json!({"type": "choice", "choice": "z", "probabilities": {"z": 1.0}})
            ),
            None
        );
        assert_eq!(
            read_answer(
                &choice,
                &json!({"type": "choice", "choice": "a", "probabilities": {"b": 1.0}})
            ),
            None
        );
    }

    fn group(n: usize, text_len: usize, per_item: usize) -> Group {
        Group {
            state: json!({"task": "t"}),
            items: (0..n)
                .map(|_| {
                    (0..per_item)
                        .map(|k| Question {
                            suffix: if per_item == 1 {
                                ""
                            } else if k == 0 {
                                "a"
                            } else {
                                "b"
                            },
                            body: json!({"type": "noul", "instructions": "x".repeat(text_len)}),
                        })
                        .collect()
                })
                .collect(),
        }
    }

    fn packed(g: &Group) -> Vec<Request> {
        let pending: Vec<(usize, Vec<usize>)> = g
            .items
            .iter()
            .enumerate()
            .map(|(i, qs)| (i, (0..qs.len()).collect()))
            .collect();
        let mut out = Vec::new();
        pack(0, g, &pending, &mut out).unwrap();
        out
    }

    fn questions_len(r: &Request) -> usize {
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        serde_json::to_vec(&v["questions"]).unwrap().len()
    }

    #[test]
    fn at_most_100_questions_per_request() {
        let rs = packed(&group(250, 1, 1));
        let counts: Vec<usize> = rs.iter().map(|r| r.names.len()).collect();
        assert_eq!(counts, [100, 100, 50]);
        assert!(rs[1].names.contains_key("q0") && rs[1].names.contains_key("q99"));
        // Two questions per item: 50 items per request, never split.
        let rs = packed(&group(51, 1, 2));
        let counts: Vec<usize> = rs.iter().map(|r| r.names.len()).collect();
        assert_eq!(counts, [100, 2]);
        assert!(rs[1].names.contains_key("q0.a") && rs[1].names.contains_key("q0.b"));
    }

    #[test]
    fn byte_limit_is_exact() {
        // Each question body is {"type":"noul","instructions":"x..."}.
        let rs = packed(&group(40, 4000, 1));
        assert!(rs.len() > 1);
        for r in &rs {
            assert!(questions_len(r) <= MAX_QUESTION_BYTES_PER_REQUEST);
        }
        // The first request is full: one more question would not fit.
        let one = questions_len(&rs[0]) / rs[0].names.len();
        assert!(questions_len(&rs[0]) + one > MAX_QUESTION_BYTES_PER_REQUEST);
        let total: usize = rs.iter().map(|r| r.names.len()).sum();
        assert_eq!(total, 40);
    }

    #[test]
    fn an_oversized_item_goes_alone() {
        let mut g = group(3, 10, 1);
        g.items[1][0].body = json!({"type": "noul", "instructions": "y".repeat(70_000)});
        let rs = packed(&g);
        let counts: Vec<usize> = rs.iter().map(|r| r.names.len()).collect();
        assert_eq!(counts, [1, 1, 1]);
        assert!(questions_len(&rs[1]) > MAX_QUESTION_BYTES_PER_REQUEST);
    }
}
