//! Which unit a stored anchor names (spec sections 6.7 and 9.2), end to end.
//!
//! Each test is a way in which a claim could come to rest on a unit the
//! caller never chose: a copy of the text elsewhere, a sibling with the same
//! sentence, a swapped name. In every one the claim must stop resolving
//! instead: the side is `missing` with candidates, no printed `next` holds
//! a candidate, and running the printed commands as they are printed never
//! makes the claim fresh.

mod support;

use serde_json::{Value, json};
use support::*;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))
}

/// The first JSON anchor in a message.
fn json_anchor_in(message: &str) -> String {
    let start = message.find("{\"loc\"").expect("a JSON anchor");
    let end = message[start..].find("}}").expect("the end of the anchor") + start + 2;
    message[start..end].to_owned()
}

/// `hibi claim` on an anchor that names two units: the error lists an
/// anchor for each. The claim is then made with the first of them.
fn claim_the_first_of_two(repo: &Repo, statement: &str, ambiguous: &str) -> String {
    let e = repo.fail(&["--now", NOW, "claim", statement, "--subject", ambiguous]);
    assert_eq!(e["code"], "side_ambiguous", "{e}");
    let message = s(&e["message"]);
    assert!(message.contains("matches 2 units"), "{message}");
    assert!(
        !message.contains("identical to another candidate"),
        "the two units have their own anchors: {message}"
    );
    let first = json_anchor_in(message);
    let v = repo.json(&["--now", NOW, "claim", statement, "--subject", &first]);
    assert_eq!(v["claims"][0]["claim"]["state"], "fresh");
    s(&v["claims"][0]["id"]).to_owned()
}

/// The claim does not resolve, and nothing that hibi prints leads to a
/// fresh claim without a choice by the caller:
///
/// - the claim is `stale` and side `side` is `missing`, without a unit;
/// - `next` is the reanchor template and holds no candidate;
/// - `mark` is refused;
/// - the printed `next`, run as it is printed, fails and writes nothing.
///
/// Returns the side.
fn assert_never_fresh_without_a_choice(repo: &Repo, id: &str, side: usize) -> Value {
    let view = repo.view(id);
    assert_eq!(view["state"], "stale", "{view}");
    let the_side = view["sides"][side].clone();
    assert_eq!(the_side["state"], "missing", "{view}");
    assert_eq!(the_side["unit"], Value::Null, "{view}");
    assert_eq!(the_side["fp"], Value::Null, "{view}");

    let option = if side == 0 {
        "--subject <anchor>".to_owned()
    } else {
        format!("--evidence {side}=<anchor>")
    };
    let next = s(&view["next"]).to_owned();
    assert_eq!(next, format!("hibi reanchor {id} {option}"));
    for candidate in the_side["candidates"].as_array().into_iter().flatten() {
        for key in ["anchor", "key", "label"] {
            // (A label can be one letter, such as the function `a`.)
            if let Some(text) = candidate[key].as_str().filter(|t| t.len() >= 6) {
                assert!(!next.contains(text), "{next} holds the candidate's {key}");
            }
        }
    }

    let before = repo.store_files();
    let e = repo.fail(&["--now", NOW, "mark", id]);
    assert_eq!(e["code"], "side_missing", "{e}");
    assert_eq!(s(&e["remediation"]), next);
    // The printed command, word for word.
    let words: Vec<&str> = next.split(' ').skip(1).collect();
    let out = repo.run(&words);
    assert_eq!(out.status, 1, "{out:?}");
    assert_eq!(repo.store_files(), before);
    assert_eq!(repo.state(id), "stale");
    the_side
}

/// A section is copied as a template for the next one, and the original is
/// then edited. The claim on the original's sentence does not move to the
/// copy: the copy is a candidate, first because it has the marked content.
#[test]
fn a_copied_section_template_does_not_take_the_claim() {
    let repo = Repo::new();
    repo.write("t.md", "# Doc\n\n## Alpha\n\nThe limit is 5.\n");
    let id = repo.claim("Alpha's limit is 5.", "file:t.md::The limit is 5.", &[]);
    assert_eq!(
        repo.view(&id)["sides"][0]["anchor"]["sel"]["within"],
        json!(["Doc", "Alpha"])
    );
    // The copy alone changes nothing.
    repo.write(
        "t.md",
        "# Doc\n\n## Alpha\n\nThe limit is 5.\n\n## Beta\n\nThe limit is 5.\n",
    );
    assert_eq!(repo.state(&id), "fresh");
    // The original is edited: the same sentence now stands only in the copy.
    repo.write(
        "t.md",
        "# Doc\n\n## Alpha\n\nThe limit is 6.\n\n## Beta\n\nThe limit is 5.\n",
    );
    let side = assert_never_fresh_without_a_choice(&repo, &id, 0);
    assert_eq!(side["reason"], "not_exact");
    let first = &side["candidates"][0];
    assert_eq!(first["sameFingerprint"], true);
    assert_eq!(first["line"], 9, "the copy under Beta: {side}");
    assert!(s(&first["anchor"]).contains("\"Beta\""), "{first}");
    // The edited original is offered as well, and it is the caller who
    // says which of them the statement is about.
    let original = side["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["label"] == "The limit is 6.")
        .unwrap_or_else(|| panic!("{side}"));
    let v = repo.json(&[
        "--now",
        NOW,
        "reanchor",
        &id,
        "--subject",
        s(&original["anchor"]),
    ]);
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(v["claim"]["sides"][0]["unit"]["label"], "The limit is 6.");
}

/// An option reference: the same sentence under one heading per option.
/// A bare quote is ambiguous, each printed anchor names its heading, and a
/// claim on one option does not move to the other when its own sentence is
/// edited.
#[test]
fn the_same_sentence_under_two_option_headings_stays_with_its_heading() {
    let repo = Repo::new();
    let doc = "# Opts\n\n## verbose\n\nIt defaults to off.\n\n## quiet\n\nIt defaults to off.\n";
    repo.write("o.md", doc);
    let id = claim_the_first_of_two(&repo, "verbose is off", "file:o.md::It defaults to off.");
    let view = repo.view(&id);
    assert_eq!(
        view["sides"][0]["anchor"]["sel"]["within"],
        json!(["Opts", "verbose"])
    );
    assert_eq!(view["sides"][0]["unit"]["line"], 5);

    repo.write(
        "o.md",
        &doc.replacen("It defaults to off.", "It defaults to on.", 1),
    );
    let side = assert_never_fresh_without_a_choice(&repo, &id, 0);
    // The sentence under `quiet` has the marked content and is only offered.
    assert_eq!(side["candidates"][0]["sameFingerprint"], true);
    assert_eq!(side["candidates"][0]["line"], 9);
}

/// The same reference written as list items. Each item is its own
/// container, named by how it begins.
#[test]
fn the_same_sentence_in_two_list_items_stays_with_its_item() {
    let repo = Repo::new();
    let doc = "# Opts\n\n- `--verbose`: prints more. It defaults to off.\n- `--quiet`: prints less. It defaults to off.\n";
    repo.write("l.md", doc);
    let id = claim_the_first_of_two(&repo, "verbose is off", "file:l.md::It defaults to off.");
    let view = repo.view(&id);
    assert_eq!(view["sides"][0]["unit"]["line"], 3);
    assert_eq!(
        view["sides"][0]["anchor"]["sel"]["within"],
        json!(["Opts", "- `--verbose`: prints more."])
    );

    repo.write(
        "l.md",
        &doc.replacen("It defaults to off.", "It defaults to on.", 1),
    );
    let side = assert_never_fresh_without_a_choice(&repo, &id, 0);
    assert_eq!(side["candidates"][0]["sameFingerprint"], true);
    assert_eq!(side["candidates"][0]["line"], 4);
}

/// "It defaults to 30 s." closes two paragraphs under one heading. Each
/// paragraph is a container named by its first sentence.
#[test]
fn the_same_sentence_in_two_paragraphs_stays_with_its_paragraph() {
    let repo = Repo::new();
    let doc = "# T\n\nThe timeout bounds a request. It defaults to 30 s.\n\nThe retry delay grows. It defaults to 30 s.\n";
    repo.write("p.md", doc);
    let id = claim_the_first_of_two(
        &repo,
        "the timeout is 30 s",
        "file:p.md::It defaults to 30 s.",
    );
    assert_eq!(
        repo.view(&id)["sides"][0]["anchor"]["sel"]["within"],
        json!(["T", "The timeout bounds a request."])
    );
    // The other paragraph changes: nothing to do.
    repo.write(
        "p.md",
        &doc.replace(
            "The retry delay grows. It defaults to 30 s.",
            "The retry delay grows. It defaults to 2 s.",
        ),
    );
    assert_eq!(repo.state(&id), "fresh");
    // The claimed paragraph changes its value: the claim does not move to
    // the other paragraph, which still says 30 s.
    repo.write(
        "p.md",
        &doc.replacen("It defaults to 30 s.", "It defaults to 45 s.", 1),
    );
    let side = assert_never_fresh_without_a_choice(&repo, &id, 0);
    assert_eq!(side["candidates"][0]["sameFingerprint"], true);
    assert_eq!(side["candidates"][0]["line"], 5);
}

/// A section is deleted, and a sibling section holds an identical sentence.
/// The claim does not move to the sibling.
#[test]
fn a_deleted_section_does_not_hand_its_claim_to_a_sibling() {
    let repo = Repo::new();
    repo.write(
        "d.md",
        "# S\n\n## A\n\nThe cap is 5.\n\n## B\n\nThe cap is 5.\n",
    );
    let id = claim_the_first_of_two(&repo, "A's cap is 5", "file:d.md::The cap is 5.");
    repo.write("d.md", "# S\n\n## B\n\nThe cap is 5.\n");
    let side = assert_never_fresh_without_a_choice(&repo, &id, 0);
    assert_eq!(side["reason"], "not_exact");
    let candidates = side["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1, "{side}");
    assert_eq!(candidates[0]["sameFingerprint"], true);
    assert!(s(&candidates[0]["anchor"]).contains("\"B\""), "{side}");
}

/// "30 s" leaves the sentence it was quoted in and appears in another one.
#[test]
fn a_short_quote_that_reappears_in_another_sentence_is_a_candidate() {
    let repo = Repo::new();
    repo.write(
        "doc.md",
        "# Doc\n\nThe timeout is 30 s by default. Retries back off slowly.\n",
    );
    let id = repo.claim("The timeout is 30 s", "file:doc.md::30 s", &[]);
    repo.write(
        "doc.md",
        "# Doc\n\nThe timeout is 45 s by default. Retries back off slowly, up to 30 s.\n",
    );
    let side = assert_never_fresh_without_a_choice(&repo, &id, 0);
    assert_eq!(
        side["detail"],
        "the stored anchor no longer identifies one unit"
    );
    assert_eq!(
        side["candidates"][0]["label"],
        "Retries back off slowly, up to 30 s."
    );
    assert_eq!(side["candidates"][0]["sameFingerprint"], false);
}

/// Two functions swap their names. A quote-only anchor is bound to the
/// declaration it stood in, by name: the text now stands in the other
/// function, and the claim does not follow it there.
#[test]
fn a_rename_swap_does_not_move_a_quote_only_anchor() {
    let repo = Repo::new();
    repo.write(
        "s.py",
        "def a():\n    return 1\n\n\ndef b():\n    return 2\n",
    );
    let v = repo.json(&[
        "--now",
        NOW,
        "claim",
        "a returns 1",
        "--subject",
        "file:s.py::return 1",
    ]);
    let id = s(&v["claims"][0]["id"]).to_owned();
    assert_eq!(
        v["claims"][0]["claim"]["sides"][0]["anchor"]["sel"]["within"],
        json!(["a"])
    );
    // The claim was told that this anchor tracks the text in its place.
    assert_eq!(
        v["claims"][0]["notes"],
        json!([
            "the subject (file:s.py) tracks the quoted text in its place. To follow the declaration a instead, anchor file:s.py#a::return 1"
        ])
    );
    repo.write(
        "s.py",
        "def b():\n    return 1\n\n\ndef a():\n    return 2\n",
    );
    let side = assert_never_fresh_without_a_choice(&repo, &id, 0);
    for candidate in side["candidates"].as_array().unwrap() {
        assert_eq!(candidate["sameFingerprint"], false, "{candidate}");
    }
}

/// A typed `#run::return value + 1`. The function is pasted under another
/// name and the original is edited: the name says `run`, the quoted text
/// says the copy. Nothing resolves, and the side says that the two point
/// to different units.
#[test]
fn a_pasted_copy_does_not_take_a_name_and_line_anchor() {
    let repo = Repo::new();
    repo.write("r.py", "def run(value):\n    return value + 1\n");
    let id = repo.claim("run adds one", "file:r.py#run::return value + 1", &[]);
    repo.write(
        "r.py",
        "def run(value):\n    return value + 1\n\n\ndef run_copy(value):\n    return value + 1\n",
    );
    assert_eq!(repo.state(&id), "fresh", "the copy alone changes nothing");
    repo.write(
        "r.py",
        "def run(value):\n    return value + 2\n\n\ndef run_copy(value):\n    return value + 1\n",
    );
    let side = assert_never_fresh_without_a_choice(&repo, &id, 0);
    assert_eq!(side["reason"], "not_exact");
    assert_eq!(
        side["detail"],
        "the name and the quoted text now point to different units: the first candidate has the name, the others hold the quoted text"
    );
    let candidates = side["candidates"].as_array().unwrap();
    assert_eq!(candidates[0]["anchor"], "file:r.py#run");
    assert_eq!(candidates[1]["anchor"], "file:r.py#run_copy");
    let human = repo.human(&["--now", NOW, "status"]);
    assert!(
        human
            .stdout
            .contains("    the name and the quoted text now point to different units"),
        "{}",
        human.stdout
    );
    // The caller decides: the statement is about `run`, which now adds two.
    let v = repo.json(&[
        "--now",
        NOW,
        "reanchor",
        &id,
        "--subject",
        "file:r.py#run::return value + 2",
        "--statement",
        "run adds two",
    ]);
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(v["claim"]["sides"][0]["unit"]["label"], "run");
}

/// A heading is renamed. The claims of its section become candidates; the
/// sentence with the marked content is the first of them.
#[test]
fn a_renamed_heading_makes_its_claims_candidates() {
    let repo = Repo::new();
    repo.write("h.md", "# G\n\n## Limits\n\nThe cap is 9.\n");
    let id = repo.claim("The cap is 9.", "file:h.md::The cap is 9.", &[]);
    repo.write("h.md", "# G\n\n## Caps\n\nThe cap is 9.\n");
    let side = assert_never_fresh_without_a_choice(&repo, &id, 0);
    assert_eq!(side["reason"], "not_exact");
    let first = &side["candidates"][0];
    assert_eq!(first["sameFingerprint"], true);
    assert_eq!(first["label"], "The cap is 9.");
    let v = repo.json(&[
        "--now",
        NOW,
        "reanchor",
        &id,
        "--subject",
        s(&first["anchor"]),
    ]);
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(
        v["claim"]["sides"][0]["anchor"]["sel"]["within"],
        json!(["G", "Caps"])
    );
}

/// A function is renamed. A claim on its name finds nothing; a claim on a
/// line of it, bound to the declaration by name, is offered the renamed
/// function as a candidate. Neither follows the rename by itself.
#[test]
fn a_renamed_function_makes_its_claims_missing() {
    let repo = Repo::new();
    repo.write("f.py", "def limit():\n    return 9\n");
    let by_name = repo.claim("limit returns 9", "file:f.py#limit", &[]);
    let by_line = repo.claim("the limit is 9", "file:f.py::return 9", &[]);
    let by_both = repo.claim("limit returns nine", "file:f.py#limit::return 9", &[]);
    repo.write("f.py", "def cap():\n    return 9\n");

    let side = assert_never_fresh_without_a_choice(&repo, &by_name, 0);
    assert_eq!(side["reason"], "selector_not_found");
    for id in [&by_line, &by_both] {
        let side = assert_never_fresh_without_a_choice(&repo, id, 0);
        assert_eq!(side["reason"], "not_exact", "{side}");
        assert_eq!(side["candidates"][0]["anchor"], "file:f.py#cap", "{side}");
    }
}

/// A printed anchor is given back as it was printed: with its `within`,
/// `prefix` and `suffix`, or its `symbol` and `count`, as an argument, as a
/// string in a standard-input document and as an object there.
#[test]
fn a_printed_anchor_is_accepted_with_its_stored_keys() {
    let repo = Repo::new();
    repo.write(
        "o.md",
        "# Opts\n\n## verbose\n\nIt defaults to off.\n\n## quiet\n\nIt defaults to off.\n",
    );
    repo.write(
        "prop.py",
        "class P:\n    @property\n    def v(self):\n        return self._v\n\n    @v.setter\n    def v(self, x):\n        self._v = x\n",
    );
    let e = repo.fail(&["claim", "s", "--subject", "file:o.md::It defaults to off."]);
    let prose = json_anchor_in(s(&e["message"]));
    assert!(
        prose.contains("\"within\":[\"Opts\",\"verbose\"]"),
        "{prose}"
    );
    let e = repo.fail(&["claim", "s", "--subject", "file:prop.py#P.v"]);
    assert_eq!(e["code"], "side_ambiguous", "{e}");
    let code = json_anchor_in(s(&e["message"]));
    assert!(code.contains("\"count\":2"), "{code}");

    let as_argument = repo.json(&["claim", "s", "--subject", &prose, "--evidence", &code]);
    let id = as_argument["claims"][0]["id"].clone();
    assert_eq!(as_argument["claims"][0]["created"], true);
    let sides = &as_argument["claims"][0]["claim"]["sides"];
    assert_eq!(sides[0]["unit"]["line"], 5);
    assert_eq!(sides[1]["anchor"]["sel"]["symbol"], "P.v");
    assert_eq!(sides[1]["anchor"]["sel"]["count"], 2);

    let as_object: Value = serde_json::from_str(&prose).unwrap();
    for batch in [
        json!([{"statement": "s", "subject": prose, "evidence": [code]}]),
        json!([{"statement": "s", "subject": as_object, "evidence": [code]}]),
    ] {
        let v = repo
            .hibi()
            .args(["--format", "json", "claim", "--stdin"])
            .stdin(&batch.to_string())
            .run()
            .ok();
        assert_eq!(v["claims"][0]["id"], id);
        assert_eq!(v["claims"][0]["created"], false);
    }
}
