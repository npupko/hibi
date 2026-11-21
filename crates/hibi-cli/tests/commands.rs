//! End-to-end behaviour of the record commands: `init`, `claim`, `status`,
//! `mark`, `reanchor`, `retire` and `list` (spec sections 3.6, 5, 6.7, 9
//! and 15; requirements R35 to R37 and R52 to R55).
//!
//! Claim, mark and retirement ids are content-addressed. They depend on the
//! completed selectors, so the tests take every id from command output and
//! compare ids with each other; no id is written out here.

mod support;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use support::*;

/// One day after `NOW`.
const TOMORROW: &str = "2026-11-21T12:00:00Z";

const DOC: &str = "\
# Guide

The cache holds 64 entries. Requests time out after 30 seconds.

The retry count is 3.
";

const CODE: &str = "\
// The limit is checked here.
pub fn limit() -> u32 {
    let max_entries = 64; // max entries note
    max_entries
}

pub fn timeout() -> u32 {
    30
}
";

const CACHE_SUBJECT: &str = "file:docs/a.md::The cache holds 64 entries.";
const TIMEOUT_SUBJECT: &str = "file:docs/a.md::Requests time out after 30 seconds.";
const RETRY_SUBJECT: &str = "file:docs/a.md::The retry count is 3.";
const LIMIT_EVIDENCE: &str = "file:src/a.rs#limit::let max_entries = 64;";

/// A repository with a store, `docs/a.md` and `src/a.rs`.
fn seeded() -> Repo {
    let repo = Repo::new();
    repo.write("docs/a.md", DOC);
    repo.write("src/a.rs", CODE);
    repo
}

/// Run with `--format json --now NOW` and return the envelope of a success.
fn j(repo: &Repo, args: &[&str]) -> Value {
    at(repo, NOW, args)
}

/// Run with `--format json --now <now>` and return the envelope of a
/// success.
fn at(repo: &Repo, now: &str, args: &[&str]) -> Value {
    let mut all = vec!["--now", now];
    all.extend_from_slice(args);
    repo.json(&all)
}

/// Run with `--now NOW`, expect exactly one error, check that the store is
/// byte for byte what it was, and return the error.
fn refused(repo: &Repo, args: &[&str]) -> Value {
    let before = repo.store_files();
    let mut all = vec!["--now", NOW];
    all.extend_from_slice(args);
    let errors = repo.run(&all).errs();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(repo.store_files(), before, "a failed run wrote: {args:?}");
    errors[0].clone()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))
}

/// A full record id of one type: `c1-`, `m1-` or `r1-` and 20 characters.
#[track_caller]
fn id_of(v: &Value, prefix: &str) -> String {
    let id = s(v);
    let body = id
        .strip_prefix(prefix)
        .unwrap_or_else(|| panic!("{id} does not start with {prefix}"));
    assert!(
        body.len() == 20
            && body
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()),
        "{id} is not a full id"
    );
    id.to_owned()
}

/// The ids of a `claims` array, in output order.
fn ids(v: &Value) -> Vec<String> {
    v["claims"]
        .as_array()
        .expect("claims is an array")
        .iter()
        .map(|c| s(&c["id"]).to_owned())
        .collect()
}

/// The claims of a `status` result by id.
fn by_id(v: &Value) -> BTreeMap<String, Value> {
    v["claims"]
        .as_array()
        .expect("claims is an array")
        .iter()
        .map(|c| (s(&c["id"]).to_owned(), c.clone()))
        .collect()
}

/// `[state, reason, has candidates]` of every side of a ClaimView.
fn sides(view: &Value) -> Value {
    Value::Array(
        view["sides"]
            .as_array()
            .expect("sides is an array")
            .iter()
            .map(|side| {
                json!([
                    side["state"],
                    side["reason"],
                    side.get("candidates").is_some()
                ])
            })
            .collect(),
    )
}

/// The keys of an object, sorted.
fn keys(v: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = v
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

// ---------------------------------------------------------------------------
// init and root discovery
// ---------------------------------------------------------------------------

fn the_36_files() -> Vec<String> {
    let mut expected: Vec<String> = [".gitattributes", ".gitignore", ".ignore"]
        .iter()
        .map(|f| format!(".hibi/{f}"))
        .collect();
    expected.insert(0, ".hibi/hibi.toml".to_owned());
    for kind in ["claims", "marks"] {
        for d in "0123456789abcdef".chars() {
            expected.push(format!(".hibi/{kind}/{d}.jsonl"));
        }
    }
    expected
}

#[test]
fn init_creates_exactly_the_36_files_of_the_layout() {
    let repo = Repo::bare();
    repo.write("README.md", "# Project\n");
    repo.write("src/a.rs", CODE);
    let before = repo.files();

    let v = repo.json(&["init"]);
    assert_eq!(v["command"], "init");
    assert_eq!(s(&v["root"]), repo.root.to_str().unwrap());
    let created: Vec<String> = v["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| s(p).to_owned())
        .collect();
    assert_eq!(created.len(), 36);
    assert_eq!(created, the_36_files());

    // Nothing outside `.hibi/` changed, and `.hibi/` holds the created
    // files and no other.
    let after = repo.files();
    let mut expected_paths: Vec<String> = before.keys().cloned().collect();
    expected_paths.extend(the_36_files());
    expected_paths.sort();
    assert_eq!(after.keys().cloned().collect::<Vec<_>>(), expected_paths);
    for (path, content) in &before {
        assert_eq!(&after[path], content, "{path}");
    }

    assert_eq!(
        repo.read(".hibi/.gitattributes"),
        "claims/*.jsonl text eol=lf merge=union linguist-generated\n\
         marks/*.jsonl text eol=lf merge=union linguist-generated\n"
    );
    assert_eq!(repo.read(".hibi/.ignore"), "claims/\nmarks/\n");
    assert_eq!(repo.read(".hibi/.gitignore"), ".lock\n*.tmp-*\n");
    let config = repo.read(".hibi/hibi.toml");
    assert!(
        config.lines().any(|l| l == "version = 1"),
        "hibi.toml: {config}"
    );
    for (path, content) in repo.store_files() {
        if path.ends_with(".jsonl") {
            assert!(content.is_empty(), "{path} is not empty");
        }
    }
}

#[test]
fn init_from_a_subdirectory_uses_the_git_top_level() {
    let repo = Repo::bare();
    repo.write("deep/er/file.txt", "x\n");
    let v = repo
        .hibi()
        .cwd(&repo.root.join("deep/er"))
        .args(["--format", "json", "init"])
        .run()
        .ok();
    assert_eq!(s(&v["root"]), repo.root.to_str().unwrap());
    assert_eq!(v["created"].as_array().unwrap().len(), 36);
    assert!(repo.root.join(".hibi/hibi.toml").is_file());
    assert!(!repo.root.join("deep/er/.hibi").exists());
    assert!(!repo.root.join("deep/.hibi").exists());
}

#[test]
fn init_twice_is_already_initialized_and_writes_nothing() {
    let repo = Repo::new();
    let before = repo.files();
    let e = repo.fail(&["init"]);
    assert_eq!(e["code"], "already_initialized");
    assert_eq!(repo.files(), before);
}

#[test]
fn init_outside_a_git_repository_is_not_a_git_repository() {
    let repo = Repo::bare();
    let outside = repo.home.join("plain");
    std::fs::create_dir_all(&outside).unwrap();
    let out = repo
        .hibi()
        .cwd(&outside)
        .args(["--format", "json", "init"])
        .run();
    assert_eq!(out.code(), "not_a_git_repository");
    assert!(!outside.join(".hibi").exists());
}

#[test]
fn the_root_is_found_from_a_subdirectory_and_given_by_root() {
    let repo = seeded();
    let id = repo.claim("The cache holds 64 entries.", CACHE_SUBJECT, &[]);

    let from_sub = repo
        .hibi()
        .cwd(&repo.root.join("docs"))
        .args(["--format", "json", "status", "--state", "fresh"])
        .run()
        .ok();
    assert_eq!(ids(&from_sub), [id.as_str()]);
    assert_eq!(from_sub["summary"]["claims"], 1);

    // From a directory that is in no repository, with `--root`.
    let from_outside = repo
        .hibi()
        .cwd(&repo.home)
        .args(["--format", "json", "--root"])
        .args([&repo.root])
        .args(["status", "--state", "fresh"])
        .run()
        .ok();
    assert_eq!(ids(&from_outside), [id]);
}

#[test]
fn a_missing_store_is_not_initialized_with_the_init_remediation() {
    let repo = Repo::bare();
    let e = repo.fail(&["status"]);
    assert_eq!(e["code"], "not_initialized");
    assert_eq!(e["remediation"], "hibi init");

    // The same from a subdirectory, from outside any repository, and with
    // `--root` naming a directory without a store.
    repo.write("docs/a.md", DOC);
    let sub = repo
        .hibi()
        .cwd(&repo.root.join("docs"))
        .args(["--format", "json", "list"])
        .run()
        .err();
    assert_eq!(sub["code"], "not_initialized");
    assert_eq!(sub["remediation"], "hibi init");
    let outside = repo
        .hibi()
        .cwd(&repo.home)
        .args(["--format", "json", "status"])
        .run()
        .err();
    assert_eq!(outside["code"], "not_initialized");
    assert_eq!(outside["remediation"], "hibi init");
    let rooted = repo
        .hibi()
        .cwd(&repo.home)
        .args(["--format", "json", "--root"])
        .args([&repo.root])
        .args(["status"])
        .run()
        .err();
    assert_eq!(rooted["code"], "not_initialized");
    assert_eq!(rooted["remediation"], "hibi init");
}

// ---------------------------------------------------------------------------
// claim
// ---------------------------------------------------------------------------

#[test]
fn claim_reports_a_fresh_claim_view_with_completed_selectors() {
    let repo = seeded();
    let v = j(
        &repo,
        &[
            "claim",
            "The cache holds 64 entries.",
            "--subject",
            CACHE_SUBJECT,
            "--evidence",
            LIMIT_EVIDENCE,
        ],
    );
    assert_eq!(v["command"], "claim");
    assert_eq!(v["warnings"], json!([]));
    assert_eq!(v["claims"].as_array().unwrap().len(), 1);
    let entry = &v["claims"][0];
    assert_eq!(keys(entry), ["accepted", "claim", "created", "id", "mark"]);
    assert_eq!(entry["accepted"], Value::Null);
    assert!(v["nextNote"].is_string());
    let id = id_of(&entry["id"], "c1-");
    assert_eq!(entry["created"], true);
    let mark = id_of(&entry["mark"], "m1-");

    // The ClaimView, whole. The stored anchors are the completed selectors
    // (quote, prefix, suffix, pos, and the symbol of a declaration), not
    // the compact text that was typed. In prose the context is the text
    // around the quote in the file. For a declaration whose symbol is
    // unique it is cut at the unit's scope: the unit and the whitespace,
    // `,` and `;` directly around it. `pos` is a byte offset in the file
    // in both.
    assert_eq!(
        entry["claim"],
        json!({
            "id": id,
            "statement": "The cache holds 64 entries.",
            "recheck": null,
            "note": null,
            "state": "fresh",
            "mark": mark,
            "markedAt": NOW,
            "expiresAt": null,
            "sides": [
                {
                    "index": 0,
                    "role": "subject",
                    "anchor": {"loc": "file:docs/a.md", "sel": {
                        "quote": "The cache holds 64 entries.",
                        "prefix": "# Guide\n\n",
                        "suffix": " Requests time out after 30 seconds.\n\nThe retry ",
                        "pos": 9
                    }},
                    "state": "same",
                    "reason": null,
                    "unit": {"key": "sentence:34da78fbbacf", "kind": "sentence",
                        "label": "The cache holds 64 entries.", "line": 3, "endLine": 3},
                    "fp": "b3:prose-v1:34da78fbbacfbf227381ac83d86394ff",
                    "markedFp": "b3:prose-v1:34da78fbbacfbf227381ac83d86394ff",
                    "remediation": null,
                    "detail": null
                },
                {
                    "index": 1,
                    "role": "evidence",
                    "anchor": {"loc": "file:src/a.rs", "sel": {
                        "quote": "let max_entries = 64;",
                        "prefix": "\npub fn limit() -> u32 {\n    ",
                        "suffix": " // max entries note\n    max_entries\n}\n\n",
                        "pos": 58,
                        "symbol": "limit"
                    }},
                    "state": "same",
                    "reason": null,
                    "unit": {"key": "function:limit", "kind": "function",
                        "label": "limit", "line": 2, "endLine": 5},
                    "fp": "b3:ast-v1:d4a43aaddbd75fa038a7b141187f69d5",
                    "markedFp": "b3:ast-v1:d4a43aaddbd75fa038a7b141187f69d5",
                    "remediation": null,
                    "detail": null
                }
            ],
            "next": null
        })
    );

    // The stored claim holds the same completed anchors.
    let claims = repo.records("claims");
    assert_eq!(claims.len(), 1);
    assert_eq!(
        claims[0],
        json!({
            "id": id,
            "statement": "The cache holds 64 entries.",
            "subject": entry["claim"]["sides"][0]["anchor"],
            "evidence": [entry["claim"]["sides"][1]["anchor"]],
            "recheck": null,
            "note": null
        })
    );
    let marks = repo.records("marks");
    assert_eq!(
        marks,
        [json!({
            "id": mark,
            "claim": id,
            "at": NOW,
            "by": null,
            "sides": [
                {"loc": "file:docs/a.md", "fp": "b3:prose-v1:34da78fbbacfbf227381ac83d86394ff"},
                {"loc": "file:src/a.rs", "fp": "b3:ast-v1:d4a43aaddbd75fa038a7b141187f69d5"}
            ],
            "note": null
        })]
    );
}

/// A symbol alone, a quote alone in code, and a whole file complete to the
/// selectors of section 6.7.
#[test]
fn claim_completes_a_symbol_a_bare_quote_and_a_whole_file() {
    let repo = seeded();
    let v = j(
        &repo,
        &[
            "claim",
            "Timeouts",
            "--subject",
            "file:docs/a.md",
            "--evidence",
            "file:src/a.rs#timeout",
            "--evidence",
            "file:src/a.rs::max_entries = 64",
        ],
    );
    let view = &v["claims"][0]["claim"];
    let id = id_of(&v["claims"][0]["id"], "c1-");
    assert_eq!(view["id"], id.as_str());
    assert_eq!(
        view["sides"][0]["anchor"],
        json!({"loc": "file:docs/a.md", "sel": {}})
    );
    assert_eq!(view["sides"][0]["unit"]["kind"], "file");
    // A symbol without a quote gets the default quote: the first line of
    // the declaration.
    assert_eq!(
        view["sides"][1]["anchor"],
        json!({"loc": "file:src/a.rs", "sel": {
            "quote": "pub fn timeout() -> u32 {",
            "prefix": "\n\n",
            "suffix": "\n    30\n}\n",
            "pos": 119,
            "symbol": "timeout"
        }})
    );
    // A quote without a symbol gets the symbol of the unit it lies in.
    assert_eq!(
        view["sides"][2]["anchor"],
        json!({"loc": "file:src/a.rs", "sel": {
            "quote": "max_entries = 64",
            "prefix": "\npub fn limit() -> u32 {\n    let ",
            "suffix": "; // max entries note\n    max_entries\n}\n\n",
            "pos": 62,
            "symbol": "limit"
        }})
    );
    assert_eq!(view["sides"][2]["index"], 2);
    assert_eq!(view["sides"][2]["role"], "evidence");
    assert_eq!(view["sides"][2]["unit"]["key"], "function:limit");
}

/// R53: the same input anchors on the same file contents give the same
/// claim id on every run and in every repository. A second run writes no
/// second claim and one more mark.
#[test]
fn r53_claiming_again_gives_the_same_id_and_a_second_mark() {
    let repo = seeded();
    let args = [
        "claim",
        "The cache holds 64 entries.",
        "--subject",
        CACHE_SUBJECT,
        "--evidence",
        LIMIT_EVIDENCE,
    ];
    let first = j(&repo, &args);
    let id = id_of(&first["claims"][0]["id"], "c1-");
    assert_eq!(first["claims"][0]["created"], true);
    let first_mark = id_of(&first["claims"][0]["mark"], "m1-");

    // One hour later: the same claim, not created, with a new mark.
    let second = at(&repo, "2026-11-20T13:00:00Z", &args);
    assert_eq!(second["claims"][0]["id"], id.as_str());
    assert_eq!(second["claims"][0]["created"], false);
    let second_mark = id_of(&second["claims"][0]["mark"], "m1-");
    assert_ne!(second_mark, first_mark);
    assert_eq!(second["claims"][0]["claim"]["state"], "fresh");
    assert_eq!(
        second["claims"][0]["claim"]["markedAt"],
        "2026-11-20T13:00:00Z"
    );
    assert_eq!(repo.records("claims").len(), 1);
    assert_eq!(repo.records("marks").len(), 2);

    // At the same instant the mark is the same record too: nothing new.
    let third = j(&repo, &args);
    assert_eq!(third["claims"][0]["id"], id.as_str());
    assert_eq!(third["claims"][0]["created"], false);
    assert_eq!(third["claims"][0]["mark"], first_mark.as_str());
    assert_eq!(repo.records("claims").len(), 1);
    assert_eq!(repo.records("marks").len(), 2);

    // Another repository with the same file contents.
    let other = seeded();
    let again = j(&other, &args);
    assert_eq!(again["claims"][0]["id"], id.as_str());
    assert_eq!(again["claims"][0]["created"], true);
    assert_eq!(again["claims"][0]["mark"], first_mark.as_str());
    assert_eq!(other.records("claims"), repo.records("claims"));

    // The typed form does not matter, only what it resolves to: the bare
    // quote completes to the same selector as symbol plus quote.
    let bare = j(
        &other,
        &[
            "claim",
            "The cache holds 64 entries.",
            "--subject",
            CACHE_SUBJECT,
            "--evidence",
            "file:src/a.rs::let max_entries = 64;",
        ],
    );
    assert_eq!(bare["claims"][0]["id"], id.as_str());
    assert_eq!(bare["claims"][0]["created"], false);
}

#[test]
fn claim_write_checks_fail_with_their_codes_and_write_nothing() {
    let repo = Repo::new();
    repo.write(
        "docs/a.md",
        "# Guide\n\nThe cache holds 64 entries. Requests time out after 30 seconds.\n\n\
         The retry count is 3.\n\n## Other\n\nThe retry count is 3.\n\nTrailing paragraph here.\n",
    );
    repo.write("src/a.rs", CODE);

    let e = refused(
        &repo,
        &["claim", "s", "--subject", CACHE_SUBJECT, "--recheck", "5d"],
    );
    assert_eq!(e["code"], "invalid_duration");

    let e = refused(
        &repo,
        &[
            "claim",
            "s",
            "--subject",
            "file:docs/a.md::nonexistent words here",
        ],
    );
    assert_eq!(e["code"], "side_missing");
    assert!(e.get("index").is_none(), "{e}");

    // The evidence side is named in the message.
    let e = refused(
        &repo,
        &[
            "claim",
            "s",
            "--subject",
            CACHE_SUBJECT,
            "--evidence",
            "file:src/a.rs#no_such_function",
        ],
    );
    assert_eq!(e["code"], "side_missing");
    assert!(s(&e["message"]).contains("evidence 1"), "{e}");

    // The file does not exist.
    let e = refused(&repo, &["claim", "s", "--subject", "file:docs/none.md"]);
    assert_eq!(e["code"], "side_missing");

    // The same sentence in two units, and no symbol to choose by.
    let e = refused(&repo, &["claim", "s", "--subject", RETRY_SUBJECT]);
    assert_eq!(e["code"], "side_ambiguous");
    let message = s(&e["message"]);
    assert!(message.contains("line 5"), "{message}");
    assert!(message.contains("line 9"), "{message}");

    // Close to a sentence, but not its text.
    let e = refused(
        &repo,
        &[
            "claim",
            "s",
            "--subject",
            "file:docs/a.md::The cache holds 65 entrees.",
        ],
    );
    assert_eq!(e["code"], "selector_not_exact");
    assert!(
        s(&e["message"]).contains("The cache holds 64 entries."),
        "{e}"
    );

    // From the end of one sentence into the next.
    let e = refused(
        &repo,
        &[
            "claim",
            "s",
            "--subject",
            "file:docs/a.md::64 entries. Requests time",
        ],
    );
    assert_eq!(e["code"], "quote_spans_units");

    assert_eq!(repo.records("claims"), Vec::<Value>::new());
    assert_eq!(repo.records("marks"), Vec::<Value>::new());
}

#[test]
fn by_and_note_are_stored_trimmed_and_empty_is_null() {
    let repo = seeded();

    // `--note` belongs to the claim, `--by` to the mark.
    let v = j(
        &repo,
        &[
            "claim",
            "  The cache holds 64 entries.  ",
            "--subject",
            CACHE_SUBJECT,
            "--note",
            "  see the limit function \n",
            "--by",
            "\t nick  ",
        ],
    );
    let noted = s(&v["claims"][0]["id"]).to_owned();
    assert_eq!(
        v["claims"][0]["claim"]["statement"],
        "The cache holds 64 entries."
    );
    assert_eq!(v["claims"][0]["claim"]["note"], "see the limit function");
    let claims = repo.records("claims");
    assert_eq!(claims[0]["note"], "see the limit function");
    assert_eq!(claims[0]["statement"], "The cache holds 64 entries.");
    let marks = repo.records("marks");
    assert_eq!(marks[0]["by"], "nick");
    assert_eq!(marks[0]["note"], Value::Null);

    // An empty note and an empty `--by` are stored as null. With a null
    // note the claim is the plain claim, a different id.
    let v = j(
        &repo,
        &[
            "claim",
            "The cache holds 64 entries.",
            "--subject",
            CACHE_SUBJECT,
            "--note",
            "   ",
            "--by",
            "",
        ],
    );
    let plain = s(&v["claims"][0]["id"]).to_owned();
    assert_ne!(plain, noted);
    assert_eq!(v["claims"][0]["claim"]["note"], Value::Null);
    let listed = j(&repo, &["list", &plain]);
    assert_eq!(listed["claim"]["note"], Value::Null);
    assert_eq!(listed["marks"][0]["by"], Value::Null);

    let mark_of = |v: &Value| -> Value {
        let id = s(&v["marks"][0]["mark"]).to_owned();
        repo.records("marks")
            .into_iter()
            .find(|m| m["id"] == id.as_str())
            .expect("the mark is stored")
    };

    // HIBI_BY is the default for `--by`, trimmed.
    let v = repo
        .hibi()
        .env("HIBI_BY", "  env-agent ")
        .args(["--format", "json", "--now", "2026-11-20T12:00:01Z"])
        .args(["mark", &plain])
        .run()
        .ok();
    let m = mark_of(&v);
    assert_eq!(m["by"], "env-agent");
    assert_eq!(m["note"], Value::Null);

    // `--by` wins over HIBI_BY; `mark --note` is stored trimmed on the mark.
    let v = repo
        .hibi()
        .env("HIBI_BY", "env-agent")
        .args(["--format", "json", "--now", "2026-11-20T12:00:02Z"])
        .args(["mark", &plain, "--by", " flag-agent ", "--note", " ran it "])
        .run()
        .ok();
    let m = mark_of(&v);
    assert_eq!(m["by"], "flag-agent");
    assert_eq!(m["note"], "ran it");

    // An empty HIBI_BY and an empty `--note` are null.
    let v = repo
        .hibi()
        .env("HIBI_BY", "   ")
        .args(["--format", "json", "--now", "2026-11-20T12:00:03Z"])
        .args(["mark", &plain, "--note", " "])
        .run()
        .ok();
    let m = mark_of(&v);
    assert_eq!(m["by"], Value::Null);
    assert_eq!(m["note"], Value::Null);

    // `retire` takes both too.
    let v = repo
        .hibi()
        .env("HIBI_BY", "env-agent")
        .args(["--format", "json", "--now", NOW])
        .args(["retire", &noted, "--note", "  gone  "])
        .run()
        .ok();
    let listed = j(&repo, &["list", &noted]);
    assert_eq!(
        listed["retirements"][0]["id"],
        v["retired"][0]["retirement"]
    );
    assert_eq!(listed["retirements"][0]["by"], "env-agent");
    assert_eq!(listed["retirements"][0]["note"], "gone");
}

// ---------------------------------------------------------------------------
// R52: quotes in comments
// ---------------------------------------------------------------------------

/// R52: `claim` with a quote in, or overlapping, a code comment fails with
/// `quote_in_comment` and writes nothing.
#[test]
fn r52_claim_with_a_quote_in_a_comment_is_refused() {
    let repo = seeded();
    for (label, anchor) in [
        (
            "a comment above a declaration",
            "file:src/a.rs::limit is checked",
        ),
        (
            "a trailing comment inside the named function",
            "file:src/a.rs#limit::max entries note",
        ),
        (
            "code running into a comment",
            "file:src/a.rs#limit::64; // max",
        ),
    ] {
        // As the subject.
        let e = refused(&repo, &["claim", "s", "--subject", anchor]);
        assert_eq!(e["code"], "quote_in_comment", "{label}");
        assert_eq!(
            e["remediation"], "Anchor the code the comment describes.",
            "{label}"
        );
        // As evidence.
        let e = refused(
            &repo,
            &[
                "claim",
                "s",
                "--subject",
                CACHE_SUBJECT,
                "--evidence",
                anchor,
            ],
        );
        assert_eq!(e["code"], "quote_in_comment", "{label}");
    }
    assert_eq!(repo.records("claims"), Vec::<Value>::new());
    assert_eq!(repo.records("marks"), Vec::<Value>::new());
}

/// R52: `reanchor` refuses a comment quote for a replaced side and for an
/// added side, and writes nothing.
#[test]
fn r52_reanchor_with_a_quote_in_a_comment_is_refused() {
    let repo = seeded();
    let id = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );

    let e = refused(
        &repo,
        &[
            "reanchor",
            &id,
            "--evidence",
            "1=file:src/a.rs#limit::max entries note",
        ],
    );
    assert_eq!(e["code"], "quote_in_comment");

    let e = refused(
        &repo,
        &[
            "reanchor",
            &id,
            "--add-evidence",
            "file:src/a.rs::limit is checked",
        ],
    );
    assert_eq!(e["code"], "quote_in_comment");

    let e = refused(
        &repo,
        &[
            "reanchor",
            &id,
            "--subject",
            "file:src/a.rs#limit::64; // max",
        ],
    );
    assert_eq!(e["code"], "quote_in_comment");

    // The claim is untouched and still fresh.
    assert_eq!(repo.state(&id), "fresh");
    assert_eq!(repo.records("claims").len(), 1);
    assert_eq!(repo.records("marks").len(), 1);
}

// ---------------------------------------------------------------------------
// R35: status and `next`
// ---------------------------------------------------------------------------

const PADDED_DOC_HEAD: &str = "\
# Guide

The cache holds 64 entries. Requests time out after 30 seconds.

";

const RETRY_BLOCK: &str = "\
Padding sentence number one that is quite long indeed, really.

The retry count is 3.

Padding sentence number two that is quite long indeed, really.

";

const CACHE_CODE: &str = "\
pub struct Cache;

impl Cache {
    pub fn get(&self) -> u32 {
        7
    }
}
";

/// R35: every stale or expired claim in `status` output has the `next` of
/// the table in section 15.2. One claim per row that the file provider can
/// reach.
#[test]
fn r35_every_stale_or_expired_claim_has_the_next_of_the_table() {
    let repo = Repo::new();
    repo.write(
        "docs/a.md",
        &format!(
            "{PADDED_DOC_HEAD}{RETRY_BLOCK}Workers are restarted nightly.\n\n\
             The queue is drained on shutdown.\n"
        ),
    );
    repo.write("docs/b.md", "Stable text.\n");
    repo.write("src/a.rs", CODE);
    repo.write("src/c.rs", CACHE_CODE);

    let stable = "file:docs/b.md";
    let queue = "file:docs/a.md::The queue is drained on shutdown.";
    let changed = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);
    let expired = {
        let v = j(
            &repo,
            &[
                "claim",
                "Timeout",
                "--subject",
                TIMEOUT_SUBJECT,
                "--evidence",
                stable,
                "--recheck",
                "P1D",
            ],
        );
        assert_eq!(v["claims"][0]["claim"]["expiresAt"], TOMORROW);
        s(&v["claims"][0]["id"]).to_owned()
    };
    let evidence_gone = repo.claim(
        "Workers",
        "file:docs/a.md::Workers are restarted nightly.",
        &[stable, queue],
    );
    let subject_gone = repo.claim("Queue", queue, &[stable]);
    let both_gone = repo.claim("Queue twice", queue, &[queue]);
    let ambiguous = repo.claim("Retries", RETRY_SUBJECT, &[stable]);
    let fresh_approx = repo.claim("Get", "file:docs/b.md", &["file:src/c.rs#Cache.get::7"]);
    let stale_approx = repo.claim(
        "Timeout value",
        TIMEOUT_SUBJECT,
        &["file:src/a.rs#timeout::30"],
    );
    // Eight different claims.
    let mut created = vec![
        changed.clone(),
        expired.clone(),
        evidence_gone.clone(),
        subject_gone.clone(),
        both_gone.clone(),
        ambiguous.clone(),
        fresh_approx.clone(),
        stale_approx.clone(),
    ];
    created.sort();
    created.dedup();
    assert_eq!(created.len(), 8);

    // Everything is fresh and nothing is listed.
    let v = j(&repo, &["status"]);
    assert_eq!(v["claims"], json!([]));
    assert_eq!(
        v["summary"],
        json!({"claims": 8, "fresh": 8, "stale": 0, "expired": 0, "retired": 0})
    );
    assert_eq!(v["nextNote"], hibi::engine::NEXT_NOTE);
    assert_eq!(v["since"], Value::Null);
    assert_eq!(v["now"], NOW);

    // The edits: the limit changes, the queue sentence is deleted, the
    // retry block appears twice, `Cache` becomes `Store`, `timeout`
    // becomes `time_out`.
    repo.write(
        "docs/a.md",
        &format!("{PADDED_DOC_HEAD}{RETRY_BLOCK}{RETRY_BLOCK}Workers are restarted nightly.\n"),
    );
    repo.write(
        "src/a.rs",
        &CODE
            .replace("max_entries = 64", "max_entries = 128")
            .replace("fn timeout", "fn time_out"),
    );
    repo.write("src/c.rs", &CACHE_CODE.replace("Cache", "Store"));

    let v = at(
        &repo,
        TOMORROW,
        &[
            "status", "--state", "stale", "--state", "expired", "--state", "fresh",
        ],
    );
    assert_eq!(v["now"], TOMORROW);
    assert_eq!(
        v["summary"],
        json!({"claims": 8, "fresh": 0, "stale": 7, "expired": 1, "retired": 0})
    );
    // The default selection is the same eight: every one has a `next`.
    assert_eq!(at(&repo, TOMORROW, &["status"])["claims"], v["claims"]);
    assert_eq!(v["nextNote"], hibi::engine::NEXT_NOTE);
    let mut sorted = ids(&v);
    assert_eq!(sorted.len(), 8);
    let listed_order = sorted.clone();
    sorted.sort();
    assert_eq!(listed_order, sorted, "claims are ordered by id");
    assert_eq!(sorted, created, "the eight claims that were created");
    let claims = by_id(&v);
    let same = json!(["same", null, false]);
    let missing = json!(["missing", "selector_not_found", false]);

    // A side changed: mark.
    let c = &claims[&changed];
    assert_eq!(c["state"], "stale");
    assert_eq!(
        sides(c),
        json!([same, ["changed", "fingerprint_differs", false]])
    );
    assert_eq!(s(&c["next"]), format!("hibi mark {changed}"));
    assert_eq!(c["expiresAt"], Value::Null);

    // Only time ran out: mark. The boundary is inclusive.
    let c = &claims[&expired];
    assert_eq!(c["state"], "expired");
    assert_eq!(sides(c), json!([same, same]));
    assert_eq!(c["markedAt"], NOW);
    assert_eq!(c["expiresAt"], TOMORROW);
    assert_eq!(s(&c["next"]), format!("hibi mark {expired}"));

    // The quoted evidence text was deleted: reanchor that evidence.
    let c = &claims[&evidence_gone];
    assert_eq!(c["state"], "stale");
    assert_eq!(sides(c), json!([same, same, missing]));
    assert_eq!(c["sides"][2]["unit"], Value::Null);
    assert_eq!(c["sides"][2]["fp"], Value::Null);
    assert!(c["sides"][2]["markedFp"].is_string());
    assert_eq!(
        s(&c["next"]),
        format!("hibi reanchor {evidence_gone} --evidence 2=<anchor>")
    );

    // The quoted subject text was deleted: reanchor the subject.
    let c = &claims[&subject_gone];
    assert_eq!(c["state"], "stale");
    assert_eq!(sides(c), json!([missing, same]));
    assert_eq!(
        s(&c["next"]),
        format!("hibi reanchor {subject_gone} --subject <anchor>")
    );

    // Both sides are gone: one command names both.
    let c = &claims[&both_gone];
    assert_eq!(c["state"], "stale");
    assert_eq!(sides(c), json!([missing, missing]));
    assert_eq!(
        s(&c["next"]),
        format!("hibi reanchor {both_gone} --subject <anchor> --evidence 1=<anchor>")
    );

    // The quote now matches two units equally well.
    let c = &claims[&ambiguous];
    assert_eq!(c["state"], "stale");
    assert_eq!(
        sides(c),
        json!([["ambiguous", "several_units", true], same])
    );
    let candidates = c["sides"][0]["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0]["line"], 7);
    assert_eq!(candidates[1]["line"], 13);
    assert_eq!(candidates[0]["label"], "The retry count is 3.");
    assert_eq!(c["sides"][0]["candidatesTotal"], 2);
    // Each candidate carries an anchor that selects it alone.
    assert_ne!(candidates[0]["anchor"], candidates[1]["anchor"]);
    for candidate in candidates {
        assert_eq!(candidate["sameFingerprint"], true);
        assert!(s(&candidate["anchor"]).starts_with('{'), "{candidate}");
    }
    assert_eq!(
        s(&c["next"]),
        format!("hibi reanchor {ambiguous} --subject <anchor>")
    );
    // `candidates` is present only on a side that has some.
    assert!(c["sides"][1].get("candidates").is_none());

    // The container was renamed: the method is found only by similarity.
    // That is a guess, so the side is missing and the method is a
    // candidate; its content is what was marked.
    let c = &claims[&fresh_approx];
    assert_eq!(c["state"], "stale");
    assert_eq!(sides(c), json!([same, ["missing", "not_exact", true]]));
    assert_eq!(c["sides"][1]["unit"], Value::Null);
    assert_eq!(
        c["sides"][1]["candidates"],
        json!([{
            "key": "method:Store.get", "kind": "method", "label": "Store.get",
            "line": c["sides"][1]["candidates"][0]["line"],
            "endLine": c["sides"][1]["candidates"][0]["endLine"],
            "text": c["sides"][1]["candidates"][0]["text"],
            "anchor": "file:src/c.rs#Store.get",
            "sameFingerprint": true
        }])
    );
    assert_eq!(
        s(&c["next"]),
        format!("hibi reanchor {fresh_approx} --evidence 1=<anchor>")
    );

    // The function itself was renamed, and its body differs from the mark
    // by the name: a candidate with another fingerprint.
    let c = &claims[&stale_approx];
    assert_eq!(c["state"], "stale");
    assert_eq!(sides(c), json!([same, ["missing", "not_exact", true]]));
    let candidate = &c["sides"][1]["candidates"][0];
    assert_eq!(candidate["key"], "function:time_out");
    assert_eq!(candidate["anchor"], "file:src/a.rs#time_out");
    assert_eq!(candidate["sameFingerprint"], false);
    assert_eq!(
        s(&c["next"]),
        format!("hibi reanchor {stale_approx} --evidence 1=<anchor>")
    );
    // No `next` holds a candidate: the anchor is the caller's to choose.
    for c in claims.values() {
        assert!(!s(&c["next"]).contains("Store.get"), "{c}");
        assert!(!s(&c["next"]).contains("time_out"), "{c}");
    }

    // The default selection is the stale and the expired claims, and
    // every one of them has a `next`.
    let v = at(&repo, TOMORROW, &["status"]);
    let default = by_id(&v);
    assert_eq!(default.len(), 8);
    for (id, c) in &default {
        assert_eq!(c["next"], claims[id]["next"], "{id}");
        assert!(matches!(s(&c["state"]), "stale" | "expired"), "{id}");
        let next = s(&c["next"]);
        assert!(
            next.starts_with(&format!("hibi mark {id}"))
                || next.starts_with(&format!("hibi reanchor {id} --")),
            "{id}: {next}"
        );
    }
}

/// A fresh claim and a retired claim have no `next`.
#[test]
fn fresh_and_retired_claims_have_no_next() {
    let repo = seeded();
    let fresh = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);
    let retired = repo.claim("Retries", RETRY_SUBJECT, &[]);
    j(&repo, &["retire", &retired]);

    let v = j(&repo, &["status", &fresh, &retired]);
    let claims = by_id(&v);
    assert_eq!(claims[&fresh]["state"], "fresh");
    assert_eq!(claims[&fresh]["next"], Value::Null);
    assert_eq!(claims[&retired]["state"], "retired");
    assert_eq!(claims[&retired]["next"], Value::Null);
    assert_eq!(claims[&retired]["sides"], json!([]));
    assert_eq!(claims[&retired]["expiresAt"], Value::Null);
}

#[test]
fn status_selects_by_state_by_locator_and_by_id() {
    let repo = seeded();
    repo.write("docs/b.md", "Stable text.\n");
    let fresh = repo.claim("Retries", RETRY_SUBJECT, &["file:docs/b.md"]);
    let stale = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);
    let retired = repo.claim("Timeout", TIMEOUT_SUBJECT, &["file:src/a.rs#timeout"]);
    assert!(fresh != stale && stale != retired && fresh != retired);
    j(&repo, &["retire", &retired]);
    repo.write("src/a.rs", &CODE.replace("= 64;", "= 128;"));

    let summary = json!({"claims": 3, "fresh": 1, "stale": 1, "expired": 0, "retired": 1});

    // Default: stale and expired only.
    let v = j(&repo, &["status"]);
    assert_eq!(ids(&v), [stale.as_str()]);
    assert_eq!(v["summary"], summary);

    // --state, once and repeated.
    let v = j(&repo, &["status", "--state", "fresh"]);
    assert_eq!(ids(&v), [fresh.as_str()]);
    assert_eq!(v["summary"], summary);
    let v = j(&repo, &["status", "--state", "retired"]);
    assert_eq!(ids(&v), [retired.as_str()]);
    assert_eq!(v["claims"][0]["state"], "retired");
    assert_eq!(v["summary"], summary);
    let v = j(&repo, &["status", "--state", "expired"]);
    assert_eq!(v["claims"], json!([]));
    assert_eq!(v["summary"], summary);
    let v = j(
        &repo,
        &[
            "status", "--state", "fresh", "--state", "stale", "--state", "retired",
        ],
    );
    let mut all = vec![fresh.clone(), stale.clone(), retired.clone()];
    all.sort();
    assert_eq!(ids(&v), all);

    // --loc is a glob over the whole locator string of any side.
    let any = ["--state", "fresh", "--state", "stale"];
    let with_loc = |globs: &[&str]| -> Vec<String> {
        let mut args = vec!["status"];
        args.extend_from_slice(&any);
        for g in globs {
            args.push("--loc");
            args.push(g);
        }
        let v = j(&repo, &args);
        assert_eq!(v["summary"], summary, "{globs:?}");
        ids(&v)
    };
    let mut live = vec![fresh.clone(), stale.clone()];
    live.sort();
    assert_eq!(with_loc(&["file:docs/a.md"]), live);
    assert_eq!(with_loc(&["file:src/*.rs"]), [stale.as_str()]);
    assert_eq!(with_loc(&["file:docs/b.*"]), [fresh.as_str()]);
    assert_eq!(with_loc(&["file:**/*.rs"]), [stale.as_str()]);
    assert_eq!(with_loc(&["file:docs/b.md", "file:src/a.rs"]), live);
    // Without the scheme the glob does not match the locator.
    assert_eq!(with_loc(&["src/*.rs"]), Vec::<String>::new());
    assert_eq!(with_loc(&["docs/a.md"]), Vec::<String>::new());
    assert_eq!(with_loc(&["file:src/b.rs"]), Vec::<String>::new());
    // The default states still apply with --loc.
    let v = j(&repo, &["status", "--loc", "file:docs/*.md"]);
    assert_eq!(ids(&v), [stale.as_str()]);

    // Ids select exactly those claims, in any state, ordered by id.
    let v = j(&repo, &["status", &retired, &fresh]);
    let mut two = vec![fresh.clone(), retired.clone()];
    two.sort();
    assert_eq!(ids(&v), two);
    assert_eq!(v["summary"], summary);
    let claims = by_id(&v);
    assert_eq!(claims[&fresh]["state"], "fresh");
    assert_eq!(claims[&retired]["state"], "retired");

    // A unique prefix of at least four characters after `c1-`.
    let v = j(&repo, &["status", &stale[..7]]);
    assert_eq!(ids(&v), [stale.as_str()]);
    let v = j(&repo, &["status", &retired[..12]]);
    assert_eq!(ids(&v), [retired.as_str()]);

    // Too short, malformed, wrong type prefix: invalid_argument.
    for bad in [
        &stale[..6],
        "nope",
        "c1-",
        "C1-ABCD",
        "c1-abcl",
        "c1-ab cd",
        &format!("{stale}0"),
    ] {
        let e = refused(&repo, &["status", bad]);
        assert_eq!(e["code"], "invalid_argument", "{bad}");
    }
    let e = refused(&repo, &["status", &stale.replace("c1-", "m1-")]);
    assert_eq!(e["code"], "invalid_argument");
    // Well-formed and unknown: claim_not_found.
    let e = refused(&repo, &["status", "c1-zzzz"]);
    assert_eq!(e["code"], "claim_not_found");
    let e = refused(&repo, &["status", &stale, "c1-zzzzzzzzzzzzzzzzzzzz"]);
    assert_eq!(e["code"], "claim_not_found");

    // --detail adds the current unit text to every resolved side.
    let v = j(&repo, &["status", &stale]);
    for side in v["claims"][0]["sides"].as_array().unwrap() {
        assert!(side.get("text").is_none(), "{side}");
    }
    let v = j(&repo, &["status", &stale, "--detail"]);
    let c = &v["claims"][0];
    assert_eq!(c["sides"][0]["text"], "The cache holds 64 entries.");
    assert_eq!(
        c["sides"][1]["text"],
        "pub fn limit() -> u32 {\n    let max_entries = 128; // max entries note\n    max_entries\n}"
    );
    assert_eq!(c["sides"][1]["state"], "changed");
    let v = j(&repo, &["status", "--detail"]);
    assert_eq!(ids(&v), [stale.as_str()]);
    assert_eq!(
        v["claims"][0]["sides"][0]["text"],
        "The cache holds 64 entries."
    );
}

/// Two claims whose ids share a prefix: the prefix is `ambiguous_id`, and
/// one character more names one claim.
#[test]
fn an_id_prefix_that_matches_two_claims_is_ambiguous_id() {
    let repo = seeded();
    // Two statements whose claim ids share their first characters.
    let a = repo.claim("statement 267", CACHE_SUBJECT, &[]);
    let b = repo.claim("statement 525", CACHE_SUBJECT, &[]);
    assert_ne!(a, b);
    let shared = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    assert!(shared >= 7, "{a} and {b} share a valid prefix");

    for command in ["status", "list", "mark", "retire", "reanchor"] {
        let e = refused(&repo, &[command, &a[..shared]]);
        assert_eq!(e["code"], "ambiguous_id", "{command}");
        let message = s(&e["message"]);
        assert!(message.contains(&a) && message.contains(&b), "{message}");
    }
    assert_eq!(ids(&j(&repo, &["status", &a[..=shared]])), [a.as_str()]);
    assert_eq!(ids(&j(&repo, &["status", &b[..=shared]])), [b.as_str()]);

    // A retired claim is still a candidate.
    j(&repo, &["retire", &a]);
    let e = refused(&repo, &["status", &a[..shared]]);
    assert_eq!(e["code"], "ambiguous_id");
}

// ---------------------------------------------------------------------------
// R36: reanchor
// ---------------------------------------------------------------------------

/// R36: `reanchor --statement` writes a new claim, a mark and a retirement
/// of the old claim with `replacedBy`; the old claim then reports
/// `retired`.
#[test]
fn r36_reanchor_with_a_new_statement_replaces_the_claim() {
    let repo = seeded();
    let old = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );

    let v = at(
        &repo,
        "2026-11-20T13:00:00Z",
        &[
            "reanchor",
            &old,
            "--statement",
            "The cache holds sixty-four entries.",
            "--by",
            "nick",
        ],
    );
    assert_eq!(v["command"], "reanchor");
    assert_eq!(
        keys(&v),
        [
            "accepted",
            "claim",
            "command",
            "mark",
            "merged",
            "new",
            "nextNote",
            "ok",
            "old",
            "retirement",
            "warnings"
        ]
    );
    assert_eq!(v["old"], old.as_str());
    let new = id_of(&v["new"], "c1-");
    assert_ne!(new, old);
    id_of(&v["mark"], "m1-");
    let retirement = id_of(&v["retirement"], "r1-");
    assert_eq!(v["claim"]["id"], new.as_str());
    assert_eq!(
        v["claim"]["statement"],
        "The cache holds sixty-four entries."
    );
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(v["claim"]["mark"], v["mark"]);
    assert_eq!(v["claim"]["markedAt"], "2026-11-20T13:00:00Z");
    assert_eq!(v["claim"]["next"], Value::Null);
    assert_eq!(
        sides(&v["claim"]),
        json!([["same", null, false], ["same", null, false]])
    );

    // The old claim: its record is kept, with one retirement naming the
    // new claim.
    let listed_old = j(&repo, &["list", &old]);
    assert_eq!(listed_old["claim"]["id"], old.as_str());
    assert_eq!(
        listed_old["retirements"],
        json!([{
            "id": retirement,
            "retires": old,
            "replacedBy": new,
            "at": "2026-11-20T13:00:00Z",
            "by": "nick",
            "note": null
        }])
    );
    assert_eq!(listed_old["marks"].as_array().unwrap().len(), 1);

    // The new claim: same anchors, the new statement, one mark.
    let listed_new = j(&repo, &["list", &new]);
    assert_eq!(
        listed_new["claim"],
        json!({
            "id": new,
            "statement": "The cache holds sixty-four entries.",
            "subject": listed_old["claim"]["subject"],
            "evidence": listed_old["claim"]["evidence"],
            "recheck": null,
            "note": null
        })
    );
    assert_eq!(listed_new["marks"].as_array().unwrap().len(), 1);
    assert_eq!(listed_new["marks"][0]["id"], v["mark"]);
    assert_eq!(listed_new["marks"][0]["claim"], new.as_str());
    assert_eq!(listed_new["marks"][0]["at"], "2026-11-20T13:00:00Z");
    assert_eq!(listed_new["marks"][0]["by"], "nick");
    assert_eq!(listed_new["retirements"], json!([]));

    // Exactly one claim, one mark and one retirement were appended.
    assert_eq!(repo.records("claims").len(), 2);
    assert_eq!(repo.records("marks").len(), 3);

    // The old claim reports retired, the new one fresh.
    let old_view = repo.view(&old);
    assert_eq!(old_view["state"], "retired");
    assert_eq!(old_view["sides"], json!([]));
    assert_eq!(old_view["next"], Value::Null);
    assert_eq!(repo.state(&new), "fresh");
    let v = j(&repo, &["status"]);
    assert_eq!(
        v["summary"],
        json!({"claims": 2, "fresh": 1, "stale": 0, "expired": 0, "retired": 1})
    );

    // A retired claim cannot be re-anchored again.
    let e = refused(&repo, &["reanchor", &old, "--statement", "Again"]);
    assert_eq!(e["code"], "claim_retired");
}

/// R36 with a new subject, on a claim whose subject went missing: the
/// command that `status` gave as `next`, with the anchor filled in.
#[test]
fn r36_reanchor_with_a_new_subject_replaces_the_claim() {
    let repo = seeded();
    let old = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    repo.write(
        "docs/a.md",
        "# Guide\n\nUp to 64 entries are cached. Requests time out after 30 seconds.\n",
    );
    let view = repo.view(&old);
    assert_eq!(view["state"], "stale");
    assert_eq!(
        sides(&view),
        json!([
            ["missing", "selector_not_found", false],
            ["same", null, false]
        ])
    );
    assert_eq!(
        s(&view["next"]),
        format!("hibi reanchor {old} --subject <anchor>")
    );

    let v = j(
        &repo,
        &[
            "reanchor",
            &old,
            "--subject",
            "file:docs/a.md::Up to 64 entries are cached.",
        ],
    );
    assert_eq!(v["old"], old.as_str());
    let new = id_of(&v["new"], "c1-");
    assert_ne!(new, old);
    id_of(&v["mark"], "m1-");
    id_of(&v["retirement"], "r1-");
    assert_eq!(v["claim"]["id"], new.as_str());
    assert_eq!(v["claim"]["mark"], v["mark"]);
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(v["claim"]["statement"], "The cache holds 64 entries.");
    assert_eq!(
        v["claim"]["sides"][0]["anchor"],
        json!({"loc": "file:docs/a.md", "sel": {
            "quote": "Up to 64 entries are cached.",
            "prefix": "# Guide\n\n",
            "suffix": " Requests time out after 30 seconds.\n",
            "pos": 9
        }})
    );
    assert_eq!(v["claim"]["sides"][1]["anchor"]["sel"]["symbol"], "limit");

    let listed_old = j(&repo, &["list", &old]);
    assert_eq!(listed_old["retirements"][0]["id"], v["retirement"]);
    assert_eq!(listed_old["retirements"][0]["retires"], old.as_str());
    assert_eq!(listed_old["retirements"][0]["replacedBy"], new.as_str());
    let listed_new = j(&repo, &["list", &new]);
    assert_eq!(
        listed_new["claim"]["subject"],
        v["claim"]["sides"][0]["anchor"]
    );
    assert_eq!(listed_new["marks"][0]["id"], v["mark"]);
    assert_eq!(listed_new["retirements"], json!([]));
    assert_eq!(repo.state(&old), "retired");
    assert_eq!(repo.state(&new), "fresh");
    assert_eq!(j(&repo, &["status"])["claims"], json!([]));
}

#[test]
fn reanchor_without_a_change_on_an_exact_claim_is_no_change() {
    let repo = seeded();
    let id = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    let e = refused(&repo, &["reanchor", &id]);
    assert_eq!(e["code"], "no_change");
    // Options that restate what the claim already holds change nothing.
    let e = refused(
        &repo,
        &[
            "reanchor",
            &id,
            "--statement",
            "The cache holds 64 entries.",
            "--subject",
            CACHE_SUBJECT,
            "--evidence",
            &format!("1={LIMIT_EVIDENCE}"),
            "--recheck",
            "none",
            "--note",
            "none",
        ],
    );
    assert_eq!(e["code"], "no_change");
    assert_eq!(repo.state(&id), "fresh");
}

/// `--evidence <n>=<anchor>`, `--add-evidence`, `--drop-evidence <n>`,
/// `--recheck`, `--note`, and the two `none` values.
#[test]
fn reanchor_edits_the_evidence_list_the_recheck_period_and_the_note() {
    let repo = seeded();
    let first = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE, "file:src/a.rs#timeout"],
    );
    let evidence = |id: &str| -> Vec<(String, Value)> {
        let listed = j(&repo, &["list", id]);
        listed["claim"]["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (
                    s(&a["loc"]).to_owned(),
                    a["sel"].get("symbol").unwrap_or(&a["sel"]["quote"]).clone(),
                )
            })
            .collect()
    };
    let rs = |symbol: &str| ("file:src/a.rs".to_owned(), json!(symbol));
    let md = |quote: &str| ("file:docs/a.md".to_owned(), json!(quote));
    assert_eq!(evidence(&first), [rs("limit"), rs("timeout")]);

    // Replace evidence 2.
    let v = j(
        &repo,
        &[
            "reanchor",
            &first,
            "--evidence",
            &format!("2={RETRY_SUBJECT}"),
        ],
    );
    assert_eq!(v["old"], first.as_str());
    let second = id_of(&v["new"], "c1-");
    assert_eq!(
        evidence(&second),
        [rs("limit"), md("The retry count is 3.")]
    );
    assert_eq!(v["claim"]["sides"][2]["unit"]["kind"], "sentence");

    // Add two at the end.
    let v = j(
        &repo,
        &[
            "reanchor",
            &second,
            "--add-evidence",
            "file:src/a.rs#timeout",
            "--add-evidence",
            TIMEOUT_SUBJECT,
        ],
    );
    assert_eq!(v["old"], second.as_str());
    let third = id_of(&v["new"], "c1-");
    assert_eq!(
        evidence(&third),
        [
            rs("limit"),
            md("The retry count is 3."),
            rs("timeout"),
            md("Requests time out after 30 seconds.")
        ]
    );
    assert_eq!(v["claim"]["sides"].as_array().unwrap().len(), 5);

    // Drop 1 and 3, replace 4, add one: the numbers refer to the old list.
    let v = j(
        &repo,
        &[
            "reanchor",
            &third,
            "--drop-evidence",
            "3",
            "--drop-evidence",
            "1",
            "--evidence",
            "4=file:src/a.rs#timeout::30",
            "--add-evidence",
            "file:src/a.rs#limit",
        ],
    );
    assert_eq!(v["old"], third.as_str());
    let fourth = id_of(&v["new"], "c1-");
    assert_eq!(
        evidence(&fourth),
        [md("The retry count is 3."), rs("timeout"), rs("limit")]
    );
    let roles: Vec<(u64, &str)> = v["claim"]["sides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x["index"].as_u64().unwrap(), s(&x["role"])))
        .collect();
    assert_eq!(
        roles,
        [
            (0, "subject"),
            (1, "evidence"),
            (2, "evidence"),
            (3, "evidence")
        ]
    );

    // Set a recheck period and a note.
    let v = j(
        &repo,
        &[
            "reanchor",
            &fourth,
            "--recheck",
            "P30D",
            "--note",
            "  run the cache test  ",
        ],
    );
    assert_eq!(v["old"], fourth.as_str());
    let fifth = id_of(&v["new"], "c1-");
    assert_eq!(v["claim"]["recheck"], "P30D");
    assert_eq!(v["claim"]["note"], "run the cache test");
    assert_eq!(v["claim"]["expiresAt"], "2026-12-20T12:00:00Z");
    let e = refused(&repo, &["reanchor", &fifth, "--recheck", "30 days"]);
    assert_eq!(e["code"], "invalid_duration");

    // `none` removes each; removing both gives the fourth claim's body
    // again, which is retired, so remove them one at a time.
    let v = j(&repo, &["reanchor", &fifth, "--recheck", "none"]);
    assert_eq!(v["old"], fifth.as_str());
    let sixth = id_of(&v["new"], "c1-");
    assert_eq!(v["claim"]["recheck"], Value::Null);
    assert_eq!(v["claim"]["note"], "run the cache test");
    assert_eq!(v["claim"]["expiresAt"], Value::Null);
    let listed = j(&repo, &["list", &sixth]);
    assert_eq!(listed["claim"]["recheck"], Value::Null);

    let v = j(
        &repo,
        &["reanchor", &sixth, "--note", "none", "--recheck", "P1Y"],
    );
    assert_eq!(v["old"], sixth.as_str());
    let seventh = id_of(&v["new"], "c1-");
    assert_eq!(v["claim"]["note"], Value::Null);
    assert_eq!(v["claim"]["recheck"], "P1Y");
    let listed = j(&repo, &["list", &seventh]);
    assert_eq!(listed["claim"]["note"], Value::Null);

    // Drop every evidence: the list may be empty.
    let v = j(
        &repo,
        &[
            "reanchor",
            &seventh,
            "--drop-evidence",
            "1",
            "--drop-evidence",
            "2",
            "--drop-evidence",
            "3",
        ],
    );
    assert_eq!(v["old"], seventh.as_str());
    let eighth = id_of(&v["new"], "c1-");
    assert_eq!(evidence(&eighth), []);

    // Every step made a claim of its own.
    let mut chain = vec![
        first,
        second,
        third,
        fourth,
        fifth,
        sixth,
        seventh,
        eighth.clone(),
    ];
    chain.sort();
    chain.dedup();
    assert_eq!(chain.len(), 8);
    assert_eq!(ids(&j(&repo, &["list", "--retired"])), chain);

    // Only the last claim is live.
    let v = j(&repo, &["status"]);
    assert_eq!(
        v["summary"],
        json!({"claims": 8, "fresh": 1, "stale": 0, "expired": 0, "retired": 7})
    );
    assert_eq!(ids(&j(&repo, &["list"])), [eighth]);
}

#[test]
fn reanchor_evidence_numbers_named_twice_or_out_of_range_are_usage() {
    let repo = seeded();
    let id = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE, "file:src/a.rs#timeout"],
    );
    let other = format!("1={RETRY_SUBJECT}");
    let third = format!("3={RETRY_SUBJECT}");
    let zero = format!("0={RETRY_SUBJECT}");
    let cases: [&[&str]; 9] = [
        // The same n twice, in every combination.
        &[
            "--evidence",
            &other,
            "--evidence",
            "1=file:src/a.rs#timeout",
        ],
        &["--evidence", &other, "--drop-evidence", "1"],
        &["--drop-evidence", "1", "--evidence", &other],
        &["--drop-evidence", "2", "--drop-evidence", "2"],
        // An n that does not exist.
        &["--evidence", &third],
        &["--evidence", &zero],
        &["--drop-evidence", "3"],
        &["--drop-evidence", "0"],
        // Not `n=anchor` at all.
        &["--evidence", RETRY_SUBJECT],
    ];
    for extra in cases {
        let mut args = vec!["reanchor", id.as_str()];
        args.extend_from_slice(extra);
        let e = refused(&repo, &args);
        assert_eq!(e["code"], "usage", "{extra:?}: {e}");
    }
    assert_eq!(repo.state(&id), "fresh");
}

/// A replaced or added side is subject to the write checks of `claim`.
#[test]
fn reanchor_applies_the_write_checks_to_replaced_and_added_sides() {
    let repo = seeded();
    let id = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    let e = refused(
        &repo,
        &[
            "reanchor",
            &id,
            "--subject",
            "file:docs/a.md::The cache holds 65 entrees.",
        ],
    );
    assert_eq!(e["code"], "selector_not_exact");
    let e = refused(
        &repo,
        &["reanchor", &id, "--evidence", "1=file:src/a.rs#nothing"],
    );
    assert_eq!(e["code"], "side_missing");
    let e = refused(
        &repo,
        &[
            "reanchor",
            &id,
            "--add-evidence",
            "file:docs/a.md::64 entries. Requests time",
        ],
    );
    assert_eq!(e["code"], "quote_spans_units");
    assert_eq!(repo.state(&id), "fresh");
}

// ---------------------------------------------------------------------------
// R37 and R55: similarity proposes, it never resolves
// ---------------------------------------------------------------------------

/// A function moves to another file. Nothing in the old file is taken for
/// it: the side is missing, `mark` and a `reanchor` that keeps the side
/// are refused, and only an anchor the caller chooses moves the claim. A
/// later change at the new place makes the claim stale.
#[test]
fn a_function_moved_to_another_file_is_missing_until_the_caller_anchors_it() {
    let repo = seeded();
    repo.write("notes.md", "# Notes\n\nThe default threshold is 50.\n");
    repo.write(
        "report.py",
        "def threshold():\n    return 50\n\n\ndef other():\n    return 1\n",
    );
    let id = repo.claim(
        "The default threshold is 50.",
        "file:notes.md::The default threshold is 50.",
        &["file:report.py#threshold"],
    );
    repo.write("report.py", "def other():\n    return 1\n");
    repo.write("config.py", "def threshold():\n    return 50\n");

    let v = j(&repo, &["status"]);
    assert_eq!(ids(&v), [id.as_str()]);
    assert_eq!(
        v["summary"],
        json!({"claims": 1, "fresh": 0, "stale": 1, "expired": 0, "retired": 0})
    );
    let view = &v["claims"][0];
    assert_eq!(view["state"], "stale");
    let side = &view["sides"][1];
    assert_eq!(side["state"], "missing");
    assert!(
        matches!(s(&side["reason"]), "selector_not_found" | "not_exact"),
        "{side}"
    );
    assert_eq!(side["unit"], Value::Null);
    assert_eq!(side["fp"], Value::Null);
    assert!(side.get("approx").is_none(), "{side}");
    let next = format!("hibi reanchor {id} --evidence 1=<anchor>");
    assert_eq!(s(&view["next"]), next);

    // `mark` does not accept another unit of the old file in its place.
    let e = refused(&repo, &["mark", &id]);
    assert_eq!(e["code"], "side_missing");
    assert_eq!(s(&e["remediation"]), next);
    // Nor does a `reanchor` that keeps the side.
    let e = refused(
        &repo,
        &["reanchor", &id, "--statement", "The threshold is 50."],
    );
    assert_eq!(e["code"], "side_missing");
    assert_eq!(s(&e["remediation"]), next);

    // The caller names the new place.
    let v = at(
        &repo,
        "2026-11-20T13:00:00Z",
        &["reanchor", &id, "--evidence", "1=file:config.py#threshold"],
    );
    let new = id_of(&v["new"], "c1-");
    assert_eq!(v["merged"], false);
    assert_eq!(v["accepted"]["was"], "stale");
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(v["claim"]["next"], Value::Null);
    assert_eq!(v["claim"]["sides"][1]["anchor"]["loc"], "file:config.py");
    assert_eq!(repo.state(&id), "retired");
    assert_eq!(j(&repo, &["status"])["claims"], json!([]));

    // The function changes at its new place: the claim says so.
    repo.write("config.py", "def threshold():\n    return 60\n");
    let view = repo.view(&new);
    assert_eq!(view["state"], "stale");
    assert_eq!(
        sides(&view),
        json!([
            ["same", null, false],
            ["changed", "fingerprint_differs", false]
        ])
    );
    assert_eq!(s(&view["next"]), format!("hibi mark {new}"));
}

/// R37: a renamed function whose claim has a quote is found only by
/// similarity. It is a candidate under a missing side, `mark` is refused,
/// and `reanchor` with the candidate's anchor stores the new symbol.
#[test]
fn r37_a_renamed_function_is_a_candidate_and_never_the_side() {
    let repo = seeded();
    let id = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &["file:src/a.rs#limit::let max_entries = 64;"],
    );
    repo.write("src/a.rs", &CODE.replace("fn limit", "fn max_cache_size"));

    let view = repo.view(&id);
    assert_eq!(view["state"], "stale");
    assert_eq!(
        sides(&view),
        json!([["same", null, false], ["missing", "not_exact", true]])
    );
    let side = &view["sides"][1];
    assert_eq!(side["unit"], Value::Null);
    assert_eq!(side["fp"], Value::Null);
    // The stored anchor still names the old symbol.
    assert_eq!(side["anchor"]["sel"]["symbol"], "limit");
    assert_eq!(side["candidatesTotal"], 1);
    let candidate = &side["candidates"][0];
    assert_eq!(
        keys(candidate),
        [
            "anchor",
            "endLine",
            "key",
            "kind",
            "label",
            "line",
            "sameFingerprint",
            "text"
        ]
    );
    assert_eq!(candidate["key"], "function:max_cache_size");
    assert_eq!(candidate["kind"], "function");
    assert_eq!(candidate["label"], "max_cache_size");
    assert_eq!(candidate["anchor"], "file:src/a.rs#max_cache_size");
    assert!(s(&candidate["text"]).contains("let max_entries = 64;"));
    let next = format!("hibi reanchor {id} --evidence 1=<anchor>");
    assert_eq!(s(&view["next"]), next);

    let e = refused(&repo, &["mark", &id]);
    assert_eq!(e["code"], "side_missing");
    assert!(s(&e["message"]).contains("not_exact"), "{e}");
    assert!(
        s(&e["message"]).contains("file:src/a.rs#max_cache_size"),
        "{e}"
    );
    assert_eq!(s(&e["remediation"]), next);
    let e = refused(&repo, &["reanchor", &id]);
    assert_eq!(e["code"], "side_missing");
    assert_eq!(s(&e["remediation"]), next);

    // A new claim cannot be made on a guess either.
    let e = refused(
        &repo,
        &[
            "claim",
            "Another",
            "--subject",
            "file:src/a.rs#limit::let max_entries = 64;",
        ],
    );
    assert_eq!(e["code"], "selector_not_exact");

    // The text output lists the candidate, its anchor and what it is.
    let human = repo.human(&["--now", NOW, "status"]);
    for part in [
        "missing (not_exact)",
        "1 candidate; each is a guess, read it before you anchor to it:",
        "- function \"max_cache_size\", line ",
        "anchor: file:src/a.rs#max_cache_size",
    ] {
        assert!(human.stdout.contains(part), "{part}: {}", human.stdout);
    }
    assert!(!human.stdout.contains("approximately"), "{}", human.stdout);

    // The caller read the candidate and chose it.
    let anchor = format!("1={}", s(&candidate["anchor"]));
    let v = at(
        &repo,
        "2026-11-20T14:00:00Z",
        &["reanchor", &id, "--evidence", &anchor],
    );
    assert_eq!(v["old"], id.as_str());
    let new = id_of(&v["new"], "c1-");
    assert_ne!(new, id);
    assert_eq!(
        v["accepted"]["sides"],
        json!([{"index": 1, "role": "evidence", "loc": "file:src/a.rs",
            "state": "missing", "reason": "not_exact"}])
    );
    assert_eq!(
        v["claim"]["sides"][1]["anchor"]["sel"]["symbol"],
        "max_cache_size"
    );
    assert_eq!(
        sides(&v["claim"]),
        json!([["same", null, false], ["same", null, false]])
    );
    assert_eq!(v["claim"]["next"], Value::Null);
    assert_eq!(repo.state(&id), "retired");
    assert_eq!(repo.view(&new)["next"], Value::Null);
}

/// R55: a reworded sentence is not the sentence that was marked. The new
/// wording is a candidate with its text and a JSON anchor; `mark` is
/// refused and writes nothing; `reanchor` with that anchor works.
#[test]
fn r55_a_reworded_sentence_is_a_candidate_and_mark_is_refused() {
    let repo = seeded();
    let id = repo.claim("Retries", RETRY_SUBJECT, &[LIMIT_EVIDENCE]);
    repo.write(
        "docs/a.md",
        &DOC.replace("retry count is 3", "retry count is 4"),
    );

    let v = j(&repo, &["status"]);
    assert_eq!(ids(&v), [id.as_str()]);
    assert!(v["summary"].get("drifted").is_none());
    assert!(!s(&v["nextNote"]).contains("drifted"));
    assert!(s(&v["nextNote"]).contains("A candidate listed under a side is a guess"));
    let view = &v["claims"][0];
    assert_eq!(view["state"], "stale");
    assert_eq!(
        sides(view),
        json!([["missing", "not_exact", true], ["same", null, false]])
    );
    let candidate = &view["sides"][0]["candidates"][0];
    assert_eq!(candidate["kind"], "sentence");
    assert_eq!(candidate["label"], "The retry count is 4.");
    assert_eq!(candidate["text"], "The retry count is 4.");
    assert_eq!(candidate["line"], 5);
    assert_eq!(candidate["sameFingerprint"], false);
    let anchor = s(&candidate["anchor"]).to_owned();
    let parsed: Value = serde_json::from_str(&anchor).expect("a JSON anchor");
    assert_eq!(parsed["loc"], "file:docs/a.md");
    assert_eq!(parsed["sel"]["quote"], "The retry count is 4.");
    let next = format!("hibi reanchor {id} --subject <anchor>");
    assert_eq!(s(&view["next"]), next);

    let e = refused(&repo, &["mark", &id]);
    assert_eq!(e["code"], "side_missing");
    assert_eq!(s(&e["remediation"]), next);
    assert_eq!(repo.records("marks").len(), 1);
    // The cause: found by similarity only.
    assert_eq!(
        view["sides"][0]["detail"],
        "the anchor matches no unit exactly"
    );
    assert!(
        s(&e["message"]).contains("(not_exact): the anchor matches no unit exactly; "),
        "{e}"
    );
    let human = repo.human(&["--now", NOW, "status"]);
    assert!(
        human
            .stdout
            .contains("missing (not_exact)\n    the anchor matches no unit exactly\n"),
        "{}",
        human.stdout
    );

    let v = at(
        &repo,
        "2026-11-20T14:00:00Z",
        &["reanchor", &id, "--subject", &anchor],
    );
    let new = id_of(&v["new"], "c1-");
    assert_ne!(new, id);
    assert_eq!(
        v["claim"]["sides"][0]["anchor"]["sel"]["quote"],
        "The retry count is 4."
    );
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(v["claim"]["next"], Value::Null);
    assert_eq!(j(&repo, &["status"])["claims"], json!([]));
}

/// The text of a candidate is cut to 240 characters.
#[test]
fn the_text_of_a_candidate_is_cut_to_240_characters() {
    let repo = seeded();
    let long: String = (0..60).map(|n| format!("word{n} ")).collect();
    let sentence = format!("The list is {}and it ends here.", long);
    assert!(sentence.chars().count() > 240);
    repo.write("docs/long.md", &format!("# Long\n\n{sentence}\n"));
    let id = repo.claim("Long", &format!("file:docs/long.md::{sentence}"), &[]);
    repo.write(
        "docs/long.md",
        &format!("# Long\n\n{}\n", sentence.replace("word7 ", "word-seven ")),
    );
    let view = repo.view(&id);
    assert_eq!(sides(&view), json!([["missing", "not_exact", true]]));
    let text = s(&view["sides"][0]["candidates"][0]["text"]);
    assert_eq!(text.chars().count(), 240);
    assert!(text.starts_with("The list is word0 word1 "));
}

/// The first JSON anchor in an error message.
fn json_anchor_in(message: &str) -> String {
    let start = message.find("{\"loc\"").expect("a JSON anchor");
    let end = message[start..].find("}}").expect("the end of the anchor") + start + 2;
    message[start..end].to_owned()
}

/// A candidate is printed as `#symbol` only when that symbol is unique in
/// the whole resource; otherwise as its completed anchor in JSON, which
/// selects it alone. Python property pairs, a Rust inherent method beside
/// a trait method, and TypeScript get and set forms share a symbol.
#[test]
fn a_candidate_anchor_selects_one_unit_of_the_whole_resource() {
    let repo = seeded();
    repo.write(
        "prop.py",
        "class P:\n    @property\n    def v(self):\n        return self._v\n\n    @v.setter\n    def v(self, x):\n        self._v = x\n\n\nclass Q:\n    def v(self):\n        return self._v\n",
    );
    repo.write(
        "a.rs",
        "struct A;\ntrait T { fn run(&self) -> u8; }\nimpl A {\n    fn run(&self) -> u8 {\n        1\n    }\n}\nimpl T for A {\n    fn run(&self) -> u8 {\n        1\n    }\n}\n",
    );
    repo.write(
        "c.ts",
        "class C {\n  private x = 1;\n  get v() {\n    return this.x;\n  }\n  set v(n: number) {\n    this.x = n;\n  }\n}\nclass D {\n  v() {\n    return this.x;\n  }\n}\n",
    );
    // (anchor, the symbol that is unique, the symbol that is not)
    for (anchor, unique, shared) in [
        (
            "file:prop.py::return self._v",
            "file:prop.py#Q.v",
            "file:prop.py#P.v",
        ),
        (
            "file:c.ts::return this.x;",
            "file:c.ts#D.v",
            "file:c.ts#C.v",
        ),
        (
            "file:a.rs::fn run(&self) -> u8",
            "file:a.rs#T.run",
            "file:a.rs#A.run",
        ),
    ] {
        let e = refused(&repo, &["claim", "S", "--subject", anchor]);
        assert_eq!(e["code"], "side_ambiguous", "{anchor}: {e}");
        let message = s(&e["message"]);
        assert!(message.contains(&format!(": {unique}")), "{message}");
        // The shared symbol alone would be ambiguous again: it is never
        // offered.
        assert!(!message.contains(shared), "{message}");
        assert!(refused(&repo, &["claim", "S", "--subject", shared])["code"] == "side_ambiguous");

        // Each offered anchor makes a claim on one unit.
        let chosen = json_anchor_in(message);
        let v = j(&repo, &["claim", anchor, "--subject", &chosen]);
        assert_eq!(v["claims"][0]["claim"]["state"], "fresh", "{chosen}");
        let v = j(&repo, &["claim", anchor, "--subject", unique]);
        assert_eq!(v["claims"][0]["created"], true, "{unique}");
    }
}

/// An anchor that matches very many units: twenty candidates and the
/// count of all of them. (The engine tests cover a stored side.)
#[test]
fn at_most_twenty_candidates_are_shown_with_the_total() {
    let repo = seeded();
    let many: String = (0..25).map(|_| "The item is ready.\n\n").collect();
    repo.write("docs/list.md", &format!("# List\n\n{many}"));

    let e = refused(
        &repo,
        &[
            "claim",
            "Again",
            "--subject",
            "file:docs/list.md::The item is ready.",
        ],
    );
    assert_eq!(e["code"], "side_ambiguous");
    let message = s(&e["message"]);
    assert!(message.contains("matches 25 units"), "{message}");
    assert_eq!(message.matches("{\"loc\"").count(), 20, "{message}");
    assert!(message.ends_with("; and 5 more"), "{message}");
}

/// `reanchor` onto a unit that a live claim with the same statement
/// already covers merges into that claim, also when the stored context of
/// that claim is out of date and the ids would differ.
#[test]
fn reanchor_onto_the_same_claim_under_another_id_merges() {
    let repo = seeded();
    let first = repo.claim("The limit.", "file:src/a.rs#limit", &[]);
    let second = repo.claim("The limit.", "file:src/a.rs#timeout", &[]);
    assert_ne!(first, second);
    // Text before the function changes: the stored context of the first
    // claim is now out of date, and its unit is the same.
    repo.write("src/a.rs", &format!("// A new first line.\n\n{CODE}"));
    assert_eq!(repo.state(&first), "fresh");

    let v = at(
        &repo,
        "2026-11-20T13:00:00Z",
        &["reanchor", &second, "--subject", "file:src/a.rs#limit"],
    );
    assert_eq!(v["merged"], true, "{v}");
    assert_eq!(v["new"], first.as_str());
    assert_eq!(v["claim"]["id"], first.as_str());
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(repo.records("claims").len(), 2, "no third claim");
    assert_eq!(repo.state(&second), "retired");
    let v = j(&repo, &["status"]);
    assert_eq!(
        v["summary"],
        json!({"claims": 2, "fresh": 1, "stale": 0, "expired": 0, "retired": 1})
    );
}

/// A code file that does not parse cannot be searched for a symbol: the
/// stored side and a new symbol anchor report `resource_unavailable` with
/// the line of the first error, nothing is written, a quote-only anchor on
/// the same file still resolves, and once the syntax is fixed the stored
/// anchor resolves again.
#[test]
fn a_code_file_that_does_not_parse_is_resource_unavailable_for_a_symbol() {
    let repo = seeded();
    let good = "def threshold():\n    return 50\n\n\ndef other():\n    return 1\n";
    let broken = good.replace("def other():", "def other(:");
    repo.write("report.py", good);
    let id = repo.claim("Threshold", CACHE_SUBJECT, &["file:report.py#threshold"]);
    let stored = repo.store_files();

    repo.write("report.py", &broken);
    let view = repo.view(&id);
    assert_eq!(view["state"], "stale");
    assert_eq!(view["next"], Value::Null);
    let side = &view["sides"][1];
    assert_eq!(side["state"], "unavailable");
    assert_eq!(side["reason"], "resource_unavailable");
    assert_eq!(side["unit"], Value::Null);
    let said = s(&side["detail"]);
    assert!(said.contains("file:report.py does not parse"), "{said}");
    assert!(said.contains("line 5"), "{said}");
    // The provider says the cause and the fix; hibi adds when to run the
    // command again, once.
    assert_eq!(
        s(&side["remediation"]),
        "file:report.py does not parse as python (the first error is on line 5), so its declarations cannot be found; fix the syntax error. Run the command again once that is fixed."
    );

    // The stored anchor: `mark` is refused with the same remediation.
    let e = refused(&repo, &["mark", &id]);
    assert_eq!(e["code"], "side_unavailable");
    assert!(s(&e["message"]).contains("line 5"), "{e}");
    assert_eq!(e["remediation"], side["remediation"]);
    // A new symbol anchor on the file.
    let e = refused(
        &repo,
        &["claim", "New", "--subject", "file:report.py#threshold"],
    );
    assert_eq!(e["code"], "side_unavailable");
    assert!(s(&e["message"]).contains("resource_unavailable"), "{e}");
    assert!(s(&e["message"]).contains("line 5"), "{e}");
    assert_eq!(e["remediation"], side["remediation"]);
    let human = repo.human(&["--now", NOW, "status"]);
    assert!(
        human
            .stdout
            .contains("unavailable (resource_unavailable)\n")
            && human.stdout.contains("line 5"),
        "{}",
        human.stdout
    );

    // The syntax is fixed: the stored anchor resolves again, and nothing
    // was written in between.
    repo.write("report.py", good);
    assert_eq!(repo.store_files(), stored);
    let view = repo.view(&id);
    assert_eq!(view["state"], "fresh");
    assert_eq!(view["sides"][1]["detail"], Value::Null);
    assert_eq!(repo.store_files(), stored);

    // A quote-only anchor does not need the declarations: it resolves on
    // the file that does not parse.
    repo.write("report.py", &broken);
    let v = j(
        &repo,
        &["claim", "Quote", "--subject", "file:report.py::return 50"],
    );
    assert_eq!(v["claims"][0]["claim"]["state"], "fresh");
    assert_eq!(v["claims"][0]["claim"]["sides"][0]["reason"], Value::Null);
}

/// A symbol on a file that has no symbols is the caller's anchor being
/// wrong: `invalid_anchor` with what the file provider said, and no advice
/// to check the provider.
#[test]
fn a_symbol_on_a_markdown_file_is_invalid_anchor() {
    let repo = seeded();
    let e = refused(&repo, &["claim", "s", "--subject", "file:docs/a.md#Guide"]);
    assert_eq!(e["code"], "invalid_anchor");
    let message = s(&e["message"]);
    assert!(
        message.starts_with("the subject (file:docs/a.md): "),
        "{message}"
    );
    assert!(message.contains("no symbols"), "{message}");
    assert!(!message.contains("provider_error"), "{message}");
    assert!(!message.contains("invalid_selector"), "{message}");
    assert!(s(&e["remediation"]).starts_with("Change the anchor"), "{e}");
    assert!(!s(&e["remediation"]).contains("hibi providers"), "{e}");
}

/// Section 9.3: a claim that is not fresh is reported against the mark
/// that matches the most sides now. A second mark in the same second as
/// the first does not make the side it covers look changed.
#[test]
fn a_stale_claim_is_reported_against_the_mark_that_matches_the_most_sides() {
    let repo = seeded();
    let id = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);
    repo.write(
        "src/a.rs",
        &CODE.replace("max_entries = 64", "max_entries = 128"),
    );
    // The second mark, in the same second as the first.
    let v = j(&repo, &["mark", &id]);
    let second = id_of(&v["marks"][0]["mark"], "m1-");
    assert_eq!(repo.records("marks").len(), 2);
    assert_eq!(repo.state(&id), "fresh");

    // The subject sentence is deleted. The evidence is as the second mark
    // has it, whichever of the two ids is greater.
    repo.write("docs/a.md", "# Guide\n\nThe retry count is 3.\n");
    let view = repo.view(&id);
    assert_eq!(view["state"], "stale");
    assert_eq!(view["mark"], second.as_str());
    assert_eq!(
        sides(&view),
        json!([
            ["missing", "selector_not_found", false],
            ["same", null, false]
        ])
    );
    assert_eq!(view["sides"][1]["fp"], view["sides"][1]["markedFp"]);

    // The other way round: the evidence goes back to what the first mark
    // has, and the first mark is the reference.
    repo.write("src/a.rs", CODE);
    let view = repo.view(&id);
    assert_ne!(view["mark"], second.as_str());
    assert_eq!(
        sides(&view),
        json!([
            ["missing", "selector_not_found", false],
            ["same", null, false]
        ])
    );
}

/// When git cannot list the files, a file side is `resource_unavailable`
/// and its remediation says once when to run the command again.
#[test]
fn an_unreadable_git_file_list_is_resource_unavailable_without_repetition() {
    let repo = seeded();
    repo.commit("base");
    let id = repo.claim("Cache size", CACHE_SUBJECT, &[]);
    let index = repo.root.join(".git/index");
    let saved = std::fs::read(&index).unwrap();
    std::fs::write(&index, b"not an index").unwrap();

    let view = repo.view(&id);
    let side = &view["sides"][0];
    assert_eq!(side["state"], "unavailable", "{view}");
    assert_eq!(side["reason"], "resource_unavailable", "{view}");
    assert_eq!(view["next"], Value::Null);
    let fix = s(&side["remediation"]);
    assert!(
        fix.starts_with(s(&side["detail"]).trim_end_matches('.')),
        "{fix}"
    );
    assert!(
        fix.ends_with(". Run the command again once that is fixed."),
        "{fix}"
    );
    assert_eq!(fix.to_lowercase().matches("again").count(), 1, "{fix}");

    std::fs::write(&index, saved).unwrap();
    assert_eq!(repo.state(&id), "fresh");
}

/// A class is renamed. The method with the marked content is the first
/// candidate, before a method of another class that only looks similar.
#[test]
fn a_method_of_a_renamed_class_is_offered_first_by_its_content() {
    let repo = seeded();
    let before = "class Alpha:\n    def run(self):\n        return 1\n\n\nclass Beta:\n    def run(self):\n        return 2\n";
    repo.write("m.py", before);
    let id = repo.claim("Alpha runs", CACHE_SUBJECT, &["file:m.py#Alpha.run"]);
    repo.write("m.py", &before.replace("Alpha", "Gamma"));

    let view = repo.view(&id);
    let side = &view["sides"][1];
    assert_eq!(side["state"], "missing");
    let candidates = side["candidates"].as_array().unwrap();
    assert_eq!(candidates[0]["key"], "method:Gamma.run");
    assert_eq!(candidates[0]["anchor"], "file:m.py#Gamma.run");
    assert_eq!(candidates[0]["sameFingerprint"], true);
    assert!(s(&candidates[0]["text"]).contains("return 1"));
    for c in &candidates[1..] {
        assert_eq!(c["sameFingerprint"], false, "{c}");
        assert_ne!(c["key"], "method:Gamma.run");
    }
    assert_eq!(side["candidatesTotal"], candidates.len());
    assert_eq!(
        s(&view["next"]),
        format!("hibi reanchor {id} --evidence 1=<anchor>")
    );
    // The error of `mark` lists the same candidates in the same order.
    let e = refused(&repo, &["mark", &id]);
    assert_eq!(e["code"], "side_missing");
    let message = s(&e["message"]);
    let anchors: Vec<&str> = candidates.iter().map(|c| s(&c["anchor"])).collect();
    let mut at = 0;
    for anchor in anchors {
        let found = message[at..]
            .find(anchor)
            .unwrap_or_else(|| panic!("{anchor}: {message}"));
        at += found + anchor.len();
    }
}

/// Text from a file reaches a terminal without control characters, except
/// newline and tab. JSON keeps them, written as escapes.
#[test]
fn human_output_has_no_control_characters_from_file_text() {
    let repo = seeded();
    repo.write(
        "docs/esc.md",
        "# Esc\n\nThe \u{1b}[31mred\u{1b}[0m word \u{7}rings.\n",
    );
    let id = repo.claim("Red", "file:docs/esc.md::word", &[]);
    repo.write(
        "docs/esc.md",
        "# Esc\n\nThe \u{1b}[31mred\u{1b}[0m word \u{7}rings twice.\n",
    );
    let human = repo.human(&["--now", NOW, "status", "--detail"]);
    assert!(human.stdout.contains(&id), "{}", human.stdout);
    assert!(
        human.stdout.contains("[31mred[0m word rings twice."),
        "{}",
        human.stdout
    );
    for text in [&human.stdout, &human.stderr] {
        assert!(
            !text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t'),
            "{text:?}"
        );
    }
    // An error message that quotes the file.
    let failed = repo.human(&[
        "--now",
        NOW,
        "claim",
        "x",
        "--subject",
        "file:docs/esc.md::rings once",
    ]);
    assert_eq!(failed.status, 1);
    assert!(failed.stderr.contains("rings twice"), "{}", failed.stderr);
    assert!(!failed.stderr.contains('\u{1b}') && !failed.stderr.contains('\u{7}'));
    // JSON: the characters are there, as escapes.
    let raw = repo.run(&["--now", NOW, "status", "--detail"]).stdout;
    assert!(raw.contains("\\u001b[31mred"), "{raw}");
    assert!(!raw.contains('\u{1b}'));
}

/// Section 9.2: a quote that is found exactly in another sentence does not
/// make that sentence the claim's unit. The side is missing with the
/// sentence as a candidate, `mark` is refused, the caller anchors the
/// claim where the statement lives, and a later change there is reported.
#[test]
fn an_exact_quote_in_another_sentence_is_a_candidate_and_never_the_side() {
    let repo = seeded();
    repo.write(
        "doc.md",
        "# Doc\n\nThe timeout is 30 s by default. Retries back off slowly.\n",
    );
    let id = repo.claim("The timeout is 30 s", "file:doc.md::30 s", &[]);
    repo.write(
        "doc.md",
        "# Doc\n\nThe timeout is 45 s by default. Retries back off slowly, up to 30 s.\n",
    );

    let view = repo.view(&id);
    assert_eq!(view["state"], "stale");
    assert_eq!(sides(&view), json!([["missing", "not_exact", true]]));
    let side = &view["sides"][0];
    assert_eq!(side["unit"], Value::Null);
    assert_eq!(side["candidatesTotal"], 1);
    let candidate = &side["candidates"][0];
    assert_eq!(candidate["label"], "Retries back off slowly, up to 30 s.");
    assert_eq!(candidate["sameFingerprint"], false);
    let next = format!("hibi reanchor {id} --subject <anchor>");
    assert_eq!(s(&view["next"]), next);

    let e = refused(&repo, &["mark", &id]);
    assert_eq!(e["code"], "side_missing");
    assert!(s(&e["message"]).contains("up to 30 s"), "{e}");
    assert_eq!(s(&e["remediation"]), next);
    assert_eq!(repo.records("marks").len(), 1);
    // The words say what happened: the quote was found, elsewhere.
    let cause = "the quoted text was found, but not where it was stored, and the unit there is not the one that was marked";
    assert_eq!(side["detail"], cause);
    assert!(s(&e["message"]).contains(cause), "{e}");
    assert!(!s(&e["message"]).contains("matches no unit exactly"), "{e}");
    let human = repo.human(&["--now", NOW, "status"]);
    assert!(
        human
            .stdout
            .contains(&format!("missing (not_exact)\n    {cause}\n")),
        "{}",
        human.stdout
    );

    // The statement is about the timeout sentence.
    let v = at(
        &repo,
        "2026-11-20T13:00:00Z",
        &[
            "reanchor",
            &id,
            "--subject",
            "file:doc.md::The timeout is 45 s",
        ],
    );
    let new = id_of(&v["new"], "c1-");
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(
        v["claim"]["sides"][0]["unit"]["label"],
        "The timeout is 45 s by default."
    );
    assert_eq!(j(&repo, &["status"])["claims"], json!([]));

    // The timeout sentence changes again: the claim is stale.
    repo.write(
        "doc.md",
        "# Doc\n\nThe timeout is 60 s by default. Retries back off slowly, up to 30 s.\n",
    );
    let v = j(&repo, &["status"]);
    assert_eq!(ids(&v), [new.as_str()]);
    assert_eq!(v["claims"][0]["state"], "stale");
    assert_eq!(v["claims"][0]["sides"][0]["state"], "missing");
}

/// The sentence before a quoted sentence is edited: the quoted sentence is
/// what was marked, so the claim stays fresh and there is nothing to do.
#[test]
fn an_edit_next_to_a_quoted_sentence_leaves_the_claim_fresh() {
    let repo = seeded();
    repo.write("n.md", "# N\n\nFirst sentence here. The cap is five.\n");
    let id = repo.claim("cap", "file:n.md::The cap is five.", &[]);
    let stored = repo.store_files();
    repo.write(
        "n.md",
        "# N\n\nFirst sentence was edited, twice. The cap is five.\n",
    );

    let view = repo.view(&id);
    assert_eq!(view["state"], "fresh");
    assert_eq!(view["next"], Value::Null);
    assert_eq!(sides(&view), json!([["same", null, false]]));
    assert_eq!(view["sides"][0]["detail"], Value::Null);
    assert_eq!(j(&repo, &["status"])["claims"], json!([]));
    assert_eq!(repo.store_files(), stored);
}

/// A quote on part of a sentence, and the sentence is edited elsewhere:
/// the edited sentence is a candidate, not the side.
#[test]
fn a_quoted_phrase_in_an_edited_sentence_is_a_candidate() {
    let repo = seeded();
    repo.write("s.md", "# S\n\nThe cap is five attempts per request.\n");
    let id = repo.claim("cap", "file:s.md::The cap is five", &[]);
    repo.write("s.md", "# S\n\nThe cap is five attempts per call.\n");

    let view = repo.view(&id);
    assert_eq!(view["state"], "stale");
    assert_eq!(sides(&view), json!([["missing", "not_exact", true]]));
    let candidate = &view["sides"][0]["candidates"][0];
    assert_eq!(candidate["text"], "The cap is five attempts per call.");
    assert_eq!(candidate["sameFingerprint"], false);
    assert_eq!(refused(&repo, &["mark", &id])["code"], "side_missing");

    // The caller read it and takes it.
    let anchor = s(&candidate["anchor"]).to_owned();
    let v = at(
        &repo,
        "2026-11-20T13:00:00Z",
        &["reanchor", &id, "--subject", &anchor],
    );
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(v["accepted"]["sides"][0]["reason"], "not_exact");
}

/// R55: editing the quoted line of a declaration anchored by symbol does
/// not lose the side; the symbol identifies the unit exactly.
#[test]
fn r55_editing_the_quoted_line_under_a_symbol_keeps_the_side() {
    let repo = seeded();
    let id = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    repo.write(
        "src/a.rs",
        &CODE.replace("max_entries = 64", "max_entries = 128"),
    );

    let view = repo.view(&id);
    assert_eq!(view["state"], "stale");
    assert_eq!(
        sides(&view),
        json!([
            ["same", null, false],
            ["changed", "fingerprint_differs", false]
        ])
    );
    assert_eq!(view["sides"][1]["unit"]["key"], "function:limit");
    assert_eq!(s(&view["next"]), format!("hibi mark {id}"));
    // After the mark the claim is fresh and there is nothing more to do.
    let v = at(&repo, "2026-11-20T13:00:00Z", &["mark", &id]);
    let marked = &v["marks"][0]["claimView"];
    assert_eq!(marked["state"], "fresh");
    assert_eq!(
        sides(marked),
        json!([["same", null, false], ["same", null, false]])
    );
    assert!(marked["sides"][1].get("text").is_none());
    assert_eq!(marked["next"], Value::Null);
    assert_eq!(repo.view(&id)["next"], Value::Null);

    // The quoted line is deleted altogether: still the same unit, by
    // symbol.
    repo.write(
        "src/a.rs",
        "pub fn limit() -> u32 {\n    256\n}\n\npub fn timeout() -> u32 {\n    30\n}\n",
    );
    let view = repo.view(&id);
    assert_eq!(
        sides(&view),
        json!([
            ["same", null, false],
            ["changed", "fingerprint_differs", false]
        ])
    );
    assert_eq!(s(&view["next"]), format!("hibi mark {id}"));
    // The symbol names the unit; the side says that the stored quote is
    // no longer in it, in JSON and in text.
    assert_eq!(
        view["sides"][1]["detail"],
        "the stored quote is no longer in this unit"
    );
    let human = repo.human(&["--now", NOW, "status"]);
    assert!(
        human.stdout.contains(
            "    the stored quote is no longer in this unit
"
        ),
        "{}",
        human.stdout
    );
    // After the mark the side is the same and says nothing.
    let v = at(&repo, "2026-11-20T14:00:00Z", &["mark", &id]);
    assert_eq!(
        v["marks"][0]["claimView"]["sides"][1]["detail"],
        Value::Null
    );
    assert_eq!(repo.view(&id)["sides"][1]["detail"], Value::Null);
}

/// Reverting the content to what an older mark covers makes the claim
/// fresh again (section 9.3), and a comment-only edit never makes a code
/// side stale.
#[test]
fn any_matching_mark_is_enough_and_comments_do_not_count() {
    let repo = seeded();
    let id = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    repo.write(
        "src/a.rs",
        &CODE.replace("// max entries note", "// reworded note"),
    );
    assert_eq!(repo.state(&id), "fresh");

    repo.write("src/a.rs", &CODE.replace("= 64;", "= 128;"));
    assert_eq!(repo.state(&id), "stale");
    let v = at(&repo, "2026-11-20T13:00:00Z", &["mark", &id]);
    let later = s(&v["marks"][0]["mark"]).to_owned();
    assert_eq!(repo.view(&id)["mark"], later.as_str());

    repo.write("src/a.rs", CODE);
    let view = repo.view(&id);
    assert_eq!(view["state"], "fresh");
    assert_eq!(view["markedAt"], NOW);
    assert_ne!(view["mark"], later.as_str());
}

// ---------------------------------------------------------------------------
// R54: reanchor back to a retired claim
// ---------------------------------------------------------------------------

/// R54: a `reanchor` whose result equals a retired claim fails with
/// `claim_retired` and writes nothing.
#[test]
fn r54_reanchor_back_to_a_retired_claim_is_refused() {
    let repo = seeded();
    let a = repo.claim(
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    let v = j(
        &repo,
        &[
            "reanchor",
            &a,
            "--statement",
            "The cache holds sixty-four entries.",
        ],
    );
    let b = id_of(&v["new"], "c1-");
    assert_ne!(b, a);
    assert_eq!(repo.state(&a), "retired");

    // Re-anchoring B back to A's statement would produce A again.
    let e = refused(
        &repo,
        &["reanchor", &b, "--statement", "The cache holds 64 entries."],
    );
    assert_eq!(e["code"], "claim_retired");
    assert_eq!(
        s(&e["remediation"]),
        format!("hibi reanchor {b} --statement 'The cache holds 64 entries.' --note <text>")
    );
    assert_eq!(repo.state(&a), "retired");
    assert_eq!(repo.state(&b), "fresh");
    assert_eq!(repo.records("claims").len(), 2);
    assert_eq!(repo.records("marks").len(), 3);

    // `claim` with A's body is refused the same way.
    let e = refused(
        &repo,
        &[
            "claim",
            "The cache holds 64 entries.",
            "--subject",
            CACHE_SUBJECT,
            "--evidence",
            LIMIT_EVIDENCE,
        ],
    );
    assert_eq!(e["code"], "claim_retired");

    // The remediation works: a note makes a different claim.
    let v = j(
        &repo,
        &[
            "reanchor",
            &b,
            "--statement",
            "The cache holds 64 entries.",
            "--note",
            "restored",
        ],
    );
    assert_eq!(v["old"], b.as_str());
    let c = id_of(&v["new"], "c1-");
    assert_ne!(c, a);
    assert_ne!(c, b);
    assert_eq!(v["claim"]["id"], c.as_str());
    assert_eq!(v["claim"]["statement"], "The cache holds 64 entries.");
    assert_eq!(v["claim"]["note"], "restored");
    assert_eq!(repo.state(&b), "retired");
}

// ---------------------------------------------------------------------------
// mark
// ---------------------------------------------------------------------------

#[test]
fn mark_clears_a_stale_claim_and_keeps_its_id() {
    let repo = seeded();
    let cache = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);
    let timeout = repo.claim("Timeout", TIMEOUT_SUBJECT, &["file:src/a.rs#timeout"]);
    repo.write(
        "src/a.rs",
        &CODE.replace("= 64;", "= 128;").replace("    30", "    45"),
    );
    assert_eq!(repo.state(&cache), "stale");
    assert_eq!(repo.state(&timeout), "stale");

    let v = at(
        &repo,
        "2026-11-20T13:00:00Z",
        &[
            "mark",
            &cache,
            &timeout[..8],
            "--by",
            "nick",
            "--note",
            "read both",
        ],
    );
    let marks = v["marks"].as_array().unwrap();
    assert_eq!(marks.len(), 2);
    assert_eq!(marks[0]["claim"], cache.as_str());
    assert_eq!(marks[1]["claim"], timeout.as_str());
    let cache_mark = id_of(&marks[0]["mark"], "m1-");
    let timeout_mark = id_of(&marks[1]["mark"], "m1-");
    assert_ne!(cache_mark, timeout_mark);
    for m in marks {
        assert_eq!(m["claimView"]["state"], "fresh");
        assert_eq!(m["claimView"]["mark"], m["mark"]);
        assert_eq!(m["claimView"]["markedAt"], "2026-11-20T13:00:00Z");
        assert_eq!(m["claimView"]["next"], Value::Null);
        assert_eq!(
            sides(&m["claimView"]),
            json!([["same", null, false], ["same", null, false]])
        );
    }
    let listed = j(&repo, &["list", &cache]);
    assert_eq!(listed["marks"].as_array().unwrap().len(), 2);
    assert_eq!(
        listed["marks"][1],
        json!({
            "id": cache_mark,
            "claim": cache,
            "at": "2026-11-20T13:00:00Z",
            "by": "nick",
            "sides": [
                {"loc": "file:docs/a.md", "fp": "b3:prose-v1:34da78fbbacfbf227381ac83d86394ff"},
                {"loc": "file:src/a.rs", "fp": marks[0]["claimView"]["sides"][1]["fp"]}
            ],
            "note": "read both"
        })
    );
    assert_ne!(listed["marks"][0]["id"], cache_mark.as_str());
    let listed = j(&repo, &["list", &timeout]);
    assert_eq!(listed["marks"].as_array().unwrap().len(), 2);
    assert_eq!(listed["marks"][1]["id"], timeout_mark.as_str());
    assert_eq!(j(&repo, &["status"])["claims"], json!([]));
}

#[test]
fn mark_on_a_retired_claim_is_claim_retired() {
    let repo = seeded();
    let id = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);
    let live = repo.claim("Timeout", TIMEOUT_SUBJECT, &[]);
    j(&repo, &["retire", &id]);
    let e = refused(&repo, &["mark", &id]);
    assert_eq!(e["code"], "claim_retired");
    // All or nothing: the live claim named with it gets no mark either.
    let e = refused(&repo, &["mark", &live, &id]);
    assert_eq!(e["code"], "claim_retired");
    assert_eq!(
        j(&repo, &["list", &live])["marks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn mark_with_a_missing_side_is_side_missing() {
    let repo = seeded();
    let id = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);

    // The evidence file is deleted.
    std::fs::remove_file(repo.root.join("src/a.rs")).unwrap();
    let view = repo.view(&id);
    assert_eq!(view["state"], "stale");
    assert_eq!(
        sides(&view),
        json!([
            ["same", null, false],
            ["missing", "resource_not_found", false]
        ])
    );
    assert_eq!(
        s(&view["next"]),
        format!("hibi reanchor {id} --evidence 1=<anchor>")
    );
    let e = refused(&repo, &["mark", &id]);
    assert_eq!(e["code"], "side_missing");
    assert!(s(&e["message"]).contains("file:src/a.rs"), "{e}");

    // The file is back without the function, and without the quote.
    repo.write("src/a.rs", "pub fn timeout() -> u32 {\n    30\n}\n");
    assert_eq!(
        sides(&repo.view(&id)),
        json!([
            ["same", null, false],
            ["missing", "selector_not_found", false]
        ])
    );
    let e = refused(&repo, &["mark", &id]);
    assert_eq!(e["code"], "side_missing");

    // Unknown and malformed ids.
    let e = refused(&repo, &["mark", "c1-zzzz"]);
    assert_eq!(e["code"], "claim_not_found");
    let e = refused(&repo, &["mark", "c1-zz"]);
    assert_eq!(e["code"], "invalid_argument");
}

#[test]
fn mark_with_an_ambiguous_side_is_side_ambiguous() {
    let repo = Repo::new();
    repo.write("docs/a.md", &format!("{PADDED_DOC_HEAD}{RETRY_BLOCK}"));
    let id = repo.claim("Retries", RETRY_SUBJECT, &[]);
    repo.write(
        "docs/a.md",
        &format!("{PADDED_DOC_HEAD}{RETRY_BLOCK}{RETRY_BLOCK}"),
    );
    let e = refused(&repo, &["mark", &id]);
    assert_eq!(e["code"], "side_ambiguous");
    // A plain reanchor cannot resolve it either.
    let e = refused(&repo, &["reanchor", &id]);
    assert_eq!(e["code"], "side_ambiguous");
}

// ---------------------------------------------------------------------------
// retire
// ---------------------------------------------------------------------------

#[test]
fn retire_takes_several_ids_and_reports_already_retired() {
    let repo = seeded();
    let cache = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);
    let timeout = repo.claim("Timeout", TIMEOUT_SUBJECT, &[]);
    let retries = repo.claim("Retries", RETRY_SUBJECT, &[]);
    assert!(cache != timeout && timeout != retries && cache != retries);

    let v = j(
        &repo,
        &[
            "retire",
            &cache,
            &timeout,
            "--note",
            "section removed",
            "--by",
            "nick",
        ],
    );
    assert_eq!(v["command"], "retire");
    let cache_retirement = id_of(&v["retired"][0]["retirement"], "r1-");
    let timeout_retirement = id_of(&v["retired"][1]["retirement"], "r1-");
    assert_ne!(cache_retirement, timeout_retirement);
    assert_eq!(
        v["retired"],
        json!([
            {"claim": cache, "retirement": cache_retirement, "alreadyRetired": false},
            {"claim": timeout, "retirement": timeout_retirement, "alreadyRetired": false}
        ])
    );
    assert_eq!(
        j(&repo, &["list", &cache])["retirements"],
        json!([{
            "id": cache_retirement,
            "retires": cache,
            "replacedBy": null,
            "at": NOW,
            "by": "nick",
            "note": "section removed"
        }])
    );
    assert_eq!(repo.state(&cache), "retired");
    assert_eq!(repo.state(&timeout), "retired");
    assert_eq!(repo.state(&retries), "fresh");

    // A second run: the retired ones are reported and get no second
    // retirement; the live one named with them is retired.
    let before = repo.records("marks").len();
    let v = at(
        &repo,
        "2026-11-20T13:00:00Z",
        &["retire", &cache, &retries, &timeout],
    );
    let retries_retirement = id_of(&v["retired"][1]["retirement"], "r1-");
    assert_ne!(retries_retirement, cache_retirement);
    assert_ne!(retries_retirement, timeout_retirement);
    assert_eq!(
        v["retired"],
        json!([
            {"claim": cache, "retirement": null, "alreadyRetired": true},
            {"claim": retries, "retirement": retries_retirement, "alreadyRetired": false},
            {"claim": timeout, "retirement": null, "alreadyRetired": true}
        ])
    );
    assert_eq!(
        j(&repo, &["list", &retries])["retirements"][0]["id"],
        retries_retirement.as_str()
    );
    assert_eq!(repo.records("marks").len(), before + 1);
    assert_eq!(
        j(&repo, &["list", &cache])["retirements"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Everything retired already: nothing is written.
    let files = repo.store_files();
    let v = at(&repo, "2026-11-20T14:00:00Z", &["retire", &cache]);
    assert_eq!(
        v["retired"],
        json!([{"claim": cache, "retirement": null, "alreadyRetired": true}])
    );
    assert_eq!(repo.store_files(), files);

    let v = j(&repo, &["status"]);
    assert_eq!(
        v["summary"],
        json!({"claims": 3, "fresh": 0, "stale": 0, "expired": 0, "retired": 3})
    );
}

#[test]
fn retire_replaced_by_must_name_another_existing_claim() {
    let repo = seeded();
    let cache = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);
    let timeout = repo.claim("Timeout", TIMEOUT_SUBJECT, &[]);
    let retries = repo.claim("Retries", RETRY_SUBJECT, &[]);

    // No such claim.
    let e = refused(&repo, &["retire", &cache, "--replaced-by", "c1-zzzz"]);
    assert_eq!(e["code"], "invalid_argument");
    // One of the claims being retired.
    let e = refused(&repo, &["retire", &cache, "--replaced-by", &cache]);
    assert_eq!(e["code"], "invalid_argument");
    assert_eq!(
        e["remediation"],
        "Name a claim that stays live as the replacement, or leave --replaced-by out."
    );
    let e = refused(
        &repo,
        &["retire", &cache, &timeout, "--replaced-by", &timeout[..9]],
    );
    assert_eq!(e["code"], "invalid_argument");
    // Not an id.
    let e = refused(&repo, &["retire", &cache, "--replaced-by", "the new one"]);
    assert_eq!(e["code"], "invalid_argument");
    assert_eq!(repo.state(&cache), "fresh");

    // Another existing claim, by prefix: stored as the full id.
    let v = j(
        &repo,
        &["retire", &cache, &timeout, "--replaced-by", &retries[..9]],
    );
    let retired = v["retired"].as_array().unwrap();
    assert_eq!(retired.len(), 2);
    for (entry, id) in retired.iter().zip([&cache, &timeout]) {
        assert_eq!(entry["claim"], id.as_str());
        assert_eq!(entry["alreadyRetired"], false);
        let listed = j(&repo, &["list", id]);
        assert_eq!(listed["retirements"][0]["id"], entry["retirement"]);
        assert_eq!(listed["retirements"][0]["replacedBy"], retries.as_str());
    }
    assert_eq!(repo.state(&retries), "fresh");

    // Unknown claim to retire.
    let e = refused(&repo, &["retire", "c1-zzzz"]);
    assert_eq!(e["code"], "claim_not_found");
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[test]
fn list_filters_by_retirement_locator_and_text() {
    let repo = seeded();
    repo.write("docs/b.md", "Stable text.\n");
    let cache = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);
    let timeout = {
        let v = j(
            &repo,
            &[
                "claim",
                "Timeout of requests",
                "--subject",
                TIMEOUT_SUBJECT,
                "--evidence",
                "file:src/a.rs#timeout",
                "--evidence",
                "file:docs/b.md",
                "--recheck",
                "P90D",
                "--note",
                "see the timeout function",
            ],
        );
        s(&v["claims"][0]["id"]).to_owned()
    };
    let stable = repo.claim("stable cache text", "file:docs/b.md", &[]);
    assert!(cache != timeout && timeout != stable && cache != stable);
    at(&repo, "2026-11-20T13:00:00Z", &["mark", &timeout]);
    at(&repo, "2026-11-20T14:00:00Z", &["retire", &stable]);

    // Default: live claims, ordered by id, with the summary fields.
    let v = j(&repo, &["list"]);
    assert_eq!(v["command"], "list");
    let mut live = vec![cache.clone(), timeout.clone()];
    live.sort();
    assert_eq!(ids(&v), live);
    let rows = by_id(&v);
    assert_eq!(
        rows[&timeout],
        json!({
            "id": timeout,
            "statement": "Timeout of requests",
            "subject": "file:docs/a.md",
            "evidence": ["file:src/a.rs", "file:docs/b.md"],
            "recheck": "P90D",
            "note": "see the timeout function",
            "marks": 2,
            "lastMarkedAt": "2026-11-20T13:00:00Z",
            "retired": false
        })
    );
    assert_eq!(
        rows[&cache],
        json!({
            "id": cache,
            "statement": "Cache size",
            "subject": "file:docs/a.md",
            "evidence": ["file:src/a.rs"],
            "recheck": null,
            "note": null,
            "marks": 1,
            "lastMarkedAt": NOW,
            "retired": false
        })
    );

    // --retired adds the retired claims.
    let v = j(&repo, &["list", "--retired"]);
    let mut all = vec![cache.clone(), timeout.clone(), stable.clone()];
    all.sort();
    assert_eq!(ids(&v), all);
    let rows = by_id(&v);
    assert_eq!(rows[&stable]["retired"], true);
    assert_eq!(rows[&stable]["evidence"], json!([]));
    assert_eq!(rows[&cache]["retired"], false);

    let list = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["list"];
        all.extend_from_slice(args);
        ids(&j(&repo, &all))
    };
    // --loc: any side, the whole locator string.
    assert_eq!(list(&["--loc", "file:docs/b.md"]), [timeout.as_str()]);
    let mut with_b = vec![timeout.clone(), stable.clone()];
    with_b.sort();
    assert_eq!(list(&["--loc", "file:docs/b.md", "--retired"]), with_b);
    assert_eq!(list(&["--loc", "file:src/**"]), live);
    assert_eq!(list(&["--loc", "file:docs/*.md"]), live);
    assert_eq!(list(&["--loc", "docs/*.md"]), Vec::<String>::new());
    assert_eq!(
        list(&["--loc", "file:nothing/*", "--loc", "file:docs/b.*"]),
        [timeout.as_str()]
    );

    // --text: a case-sensitive substring of the statement.
    assert_eq!(list(&["--text", "Cache"]), [cache.as_str()]);
    assert_eq!(list(&["--text", "cache"]), Vec::<String>::new());
    assert_eq!(list(&["--text", "cache", "--retired"]), [stable.as_str()]);
    assert_eq!(list(&["--text", "Timeout of"]), [timeout.as_str()]);
    assert_eq!(list(&["--text", "timeout"]), Vec::<String>::new());
    assert_eq!(list(&["--text", "e"]), live);
    // Both filters apply together.
    assert_eq!(
        list(&["--text", "Timeout", "--loc", "file:src/a.rs"]),
        [timeout.as_str()]
    );
    assert_eq!(
        list(&["--text", "Cache", "--loc", "file:docs/b.md"]),
        Vec::<String>::new()
    );
}

#[test]
fn list_with_an_id_gives_the_record_and_its_history() {
    let repo = seeded();
    let id = repo.claim("Cache size", CACHE_SUBJECT, &[LIMIT_EVIDENCE]);
    // Marks are written out of time order; the output orders them by `at`.
    at(
        &repo,
        "2026-11-20T15:00:00Z",
        &["mark", &id, "--by", "third"],
    );
    at(
        &repo,
        "2026-11-20T13:00:00Z",
        &["mark", &id, "--by", "second"],
    );
    at(
        &repo,
        "2026-11-19T09:00:00Z",
        &["mark", &id, "--by", "first"],
    );

    let v = j(&repo, &["list", &id[..10]]);
    assert_eq!(v["command"], "list");
    assert_eq!(
        keys(&v),
        ["claim", "command", "marks", "ok", "retirements", "warnings"]
    );
    assert_eq!(v["claim"], repo.records("claims")[0]);
    assert_eq!(v["claim"]["id"], id.as_str());
    let marks = v["marks"].as_array().unwrap();
    let order: Vec<(&str, &str)> = marks
        .iter()
        .map(|m| (s(&m["at"]), m["by"].as_str().unwrap_or("-")))
        .collect();
    assert_eq!(
        order,
        [
            ("2026-11-19T09:00:00Z", "first"),
            ("2026-11-20T12:00:00Z", "-"),
            ("2026-11-20T13:00:00Z", "second"),
            ("2026-11-20T15:00:00Z", "third")
        ]
    );
    for m in marks {
        assert_eq!(keys(m), ["at", "by", "claim", "id", "note", "sides"]);
        assert_eq!(m["claim"], id.as_str());
        assert_eq!(m["sides"].as_array().unwrap().len(), 2);
    }
    assert_eq!(v["retirements"], json!([]));

    // A retired claim is still found by id, with its retirement.
    let r = at(&repo, "2026-11-20T16:00:00Z", &["retire", &id]);
    let v = j(&repo, &["list", &id]);
    assert_eq!(v["marks"].as_array().unwrap().len(), 4);
    assert_eq!(
        v["retirements"],
        json!([{
            "id": r["retired"][0]["retirement"],
            "retires": id,
            "replacedBy": null,
            "at": "2026-11-20T16:00:00Z",
            "by": null,
            "note": null
        }])
    );
    assert_eq!(j(&repo, &["list"])["claims"], json!([]));

    let e = refused(&repo, &["list", "c1-zzzz"]);
    assert_eq!(e["code"], "claim_not_found");
    let e = refused(&repo, &["list", "zzzz"]);
    assert_eq!(e["code"], "invalid_argument");
}

// ---------------------------------------------------------------------------
// Members of a struct and of an enum
// ---------------------------------------------------------------------------

const MEMBERS: &str = "\
pub struct Config {
    pub timeout_ms: u64,
    pub retries: u32,
    pub name: String,
}

pub enum Mode {
    Fast,
    Slow(u8),
    Off,
}
";

/// A claim quoted on one field line of a struct, or on one variant line of
/// an enum, goes stale only when that member changes. A change to another
/// field or variant of the same struct or enum leaves it fresh.
#[test]
fn a_member_claim_goes_stale_only_when_that_member_changes() {
    let repo = seeded();
    repo.write("src/m.rs", MEMBERS);
    let field = repo.claim(
        "Requests time out after 30 seconds.",
        TIMEOUT_SUBJECT,
        &["file:src/m.rs::pub timeout_ms: u64"],
    );
    let variant = repo.claim(
        "The slow mode carries a level.",
        "file:docs/a.md",
        &["file:src/m.rs::Slow(u8)"],
    );
    // `[claim state, subject state, whether the member side is "same"]`.
    let states = |repo: &Repo| -> (Value, Value) {
        let f = repo.view(&field);
        let v = repo.view(&variant);
        (
            json!([
                f["state"],
                f["sides"][0]["state"],
                f["sides"][1]["state"] == "same"
            ]),
            json!([
                v["state"],
                v["sides"][0]["state"],
                v["sides"][1]["state"] == "same"
            ]),
        )
    };
    let fresh = json!(["fresh", "same", true]);
    let stale = json!(["stale", "same", false]);
    // The quotes resolved to the members, not to their containers.
    assert_eq!(repo.view(&field)["sides"][1]["unit"]["kind"], "field");
    assert_eq!(
        repo.view(&field)["sides"][1]["unit"]["label"],
        "Config.timeout_ms"
    );
    assert_eq!(repo.view(&variant)["sides"][1]["unit"]["kind"], "variant");
    assert_eq!(
        repo.view(&variant)["sides"][1]["unit"]["label"],
        "Mode.Slow"
    );
    assert_eq!(states(&repo), (fresh.clone(), fresh.clone()));

    // Another field of the same struct changes its type; another variant
    // of the same enum is renamed. Both claims stay fresh.
    repo.write(
        "src/m.rs",
        &MEMBERS
            .replace("pub retries: u32", "pub retries: u8")
            .replace("    Off,", "    Stopped,"),
    );
    assert_eq!(states(&repo), (fresh.clone(), fresh.clone()));
    assert_eq!(j(&repo, &["status"])["claims"], json!([]));

    // A field is replaced by another and a variant is added. Both claims
    // stay fresh.
    repo.write(
        "src/m.rs",
        &MEMBERS
            .replace("    pub name: String,\n", "    pub verbose: bool,\n")
            .replace("    Fast,\n", "    Fast,\n    Faster,\n"),
    );
    assert_eq!(states(&repo), (fresh.clone(), fresh.clone()));
    assert_eq!(j(&repo, &["status"])["claims"], json!([]));

    // The claimed field changes its type: only the field claim is stale.
    repo.write(
        "src/m.rs",
        &MEMBERS.replace("pub timeout_ms: u64", "pub timeout_ms: u32"),
    );
    assert_eq!(states(&repo), (stale.clone(), fresh.clone()));
    assert_eq!(ids(&j(&repo, &["status"])), [field.as_str()]);

    // The claimed variant changes its payload: only the variant claim is
    // stale.
    repo.write("src/m.rs", &MEMBERS.replace("Slow(u8)", "Slow(u16)"));
    assert_eq!(states(&repo), (fresh.clone(), stale.clone()));
    assert_eq!(ids(&j(&repo, &["status"])), [variant.as_str()]);

    // Back to the claimed content: both fresh again.
    repo.write("src/m.rs", MEMBERS);
    assert_eq!(states(&repo), (fresh.clone(), fresh));
}

/// The identical `claim` command after an edit above the quoted sentence
/// marks the existing claim and creates no second one. The claim id covers
/// the stored context and position of the quote, which the edit changed.
#[test]
fn claiming_again_after_an_edit_near_the_quote_creates_no_duplicate() {
    let repo = Repo::new();
    repo.write(
        "docs/a.md",
        "# Limits\n\nAn introduction line.\n\nRequests time out after 30 seconds.\n",
    );
    repo.write("src/a.rs", "pub const TIMEOUT_SECONDS: u32 = 30;\n");
    let args = [
        "claim",
        "Requests time out after 30 seconds.",
        "--subject",
        "file:docs/a.md::time out after 30 seconds",
        "--evidence",
        "file:src/a.rs#TIMEOUT_SECONDS",
    ];
    let at = |now: &'static str| [&["--now", now][..], &args[..]].concat();
    let first = repo.json(&at("2026-11-20T12:00:00Z"));
    let id = first["claims"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(first["claims"][0]["created"], true);

    // Delete the line above the sentence.
    repo.write(
        "docs/a.md",
        "# Limits\n\nRequests time out after 30 seconds.\n",
    );
    let again = repo.json(&at("2026-11-20T12:05:00Z"));
    assert_eq!(again["claims"][0]["id"], id.as_str());
    assert_eq!(again["claims"][0]["created"], false);
    assert_eq!(again["claims"][0]["claim"]["state"], "fresh");
    assert_ne!(again["claims"][0]["mark"], first["claims"][0]["mark"]);

    let listed = repo.json(&["list"]);
    assert_eq!(listed["claims"].as_array().unwrap().len(), 1);
    assert_eq!(listed["claims"][0]["id"], id.as_str());
    assert_eq!(listed["claims"][0]["marks"], 2);
    assert_eq!(repo.records("claims").len(), 1);
    let summary = repo.json(&["status"])["summary"].clone();
    assert_eq!(
        (summary["claims"].as_u64(), summary["fresh"].as_u64()),
        (Some(1), Some(1))
    );

    // A different statement on the same anchors is another claim.
    let mut other = args;
    other[1] = "Requests time out.";
    let v = repo.json(&other);
    assert_eq!(v["claims"][0]["created"], true);
    assert_ne!(v["claims"][0]["id"], id.as_str());

    // Only a live claim is matched. After the retirement the command
    // makes a new claim: its completed context differs from the retired
    // claim's, so it is not the retired claim again either.
    repo.json(&["retire", &id]);
    let v = repo.json(&args);
    assert_eq!(v["claims"][0]["created"], true);
    assert_ne!(v["claims"][0]["id"], id.as_str());
}
