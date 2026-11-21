//! The write commands through the binary: what a new anchor may be, what a
//! write reports that it accepted, and standard-input batches (spec
//! section 15.3).

mod support;

use serde_json::{Value, json};
use support::*;

fn text(v: &Value) -> &str {
    v.as_str().expect("a string")
}

/// Every `{"loc": ...}` object printed inside a message.
fn anchors_in(message: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = message;
    while let Some(start) = rest.find(r#"{"loc":"#) {
        let tail = &rest[start..];
        let mut stream = serde_json::Deserializer::from_str(tail).into_iter::<Value>();
        let value = stream.next().expect("an object").expect("valid JSON");
        let end = stream.byte_offset();
        out.push(value.to_string());
        rest = &tail[end..];
    }
    out
}

/// A symbol that names two declarations is ambiguous. The error lists each
/// candidate with an anchor, and that anchor selects it.
#[test]
fn a_symbol_naming_two_declarations_is_ambiguous_and_each_candidate_can_be_picked() {
    let repo = Repo::new();
    repo.write(
        "dup.py",
        "def max_size():\n    return 10\n\n\ndef other():\n    return 0\n\n\ndef max_size():\n    return 20\n",
    );
    let e = repo.fail(&["claim", "s", "--subject", "file:dup.py#max_size"]);
    assert_eq!(e["code"], "side_ambiguous");
    let message = text(&e["message"]);
    assert!(message.contains("matches 2 units"), "{message}");
    assert!(
        message.contains("function \"max_size\", line 1: "),
        "{message}"
    );
    assert!(
        message.contains("function \"max_size\", line 9: "),
        "{message}"
    );
    assert_eq!(
        e["remediation"],
        "Give a symbol, or a longer quote, so that the anchor matches one unit."
    );
    assert!(repo.records("claims").is_empty());

    let anchors = anchors_in(message);
    assert_eq!(anchors.len(), 2, "{message}");
    let mut lines = Vec::new();
    for anchor in &anchors {
        let v = repo.json(&["claim", "s", "--subject", anchor]);
        let side = &v["claims"][0]["claim"]["sides"][0];
        assert_eq!(side["unit"]["label"], "max_size");
        lines.push(side["unit"]["line"].as_u64().unwrap());
    }
    assert_eq!(lines, [1, 9], "each anchor selects its own declaration");
    assert_eq!(repo.records("claims").len(), 2);
}

/// A quote that occurs in two units is ambiguous in the same way, and the
/// candidates' anchors tell the two apart.
#[test]
fn a_quote_in_two_units_is_ambiguous_with_an_anchor_per_candidate() {
    let repo = Repo::new();
    repo.write(
        "docs/a.md",
        "# One\n\nThe cap is 5.\n\n# Two\n\nThe cap is 5.\n",
    );
    let e = repo.fail(&["claim", "s", "--subject", "file:docs/a.md::The cap is 5."]);
    assert_eq!(e["code"], "side_ambiguous");
    let message = text(&e["message"]);
    assert!(
        message.contains("sentence \"The cap is 5.\", line 3: "),
        "{message}"
    );
    assert!(
        message.contains("sentence \"The cap is 5.\", line 7: "),
        "{message}"
    );
    let anchors = anchors_in(message);
    assert_eq!(anchors.len(), 2);
    let v = repo.json(&["claim", "s", "--subject", &anchors[1]]);
    assert_eq!(v["claims"][0]["claim"]["sides"][0]["unit"]["line"], 7);
}

/// A new anchor with a symbol and a quote: the quote must occur inside
/// that declaration. Text of a doc comment above it does not count.
#[test]
fn a_quote_given_with_a_symbol_must_occur_in_that_declaration() {
    let repo = Repo::new();
    repo.write(
        "src/retry.rs",
        "pub fn delay_ms(attempt: u32) -> u64 {\n    300 * 2u64.pow(attempt)\n}\n\npub fn other() -> u64 {\n    300 * 7\n}\n",
    );
    repo.write(
        "src/retry.ts",
        "/** How many times a download is tried. */\nexport const MAX_ATTEMPTS = 5;\n",
    );
    repo.write("docs/a.md", "Downloads are tried 5 times.\n");
    let subject = "file:docs/a.md::tried 5 times";
    for evidence in [
        "file:src/retry.rs#delay_ms::300 * 7",
        "file:src/retry.ts#MAX_ATTEMPTS::How many times",
    ] {
        let e = repo.fail(&["claim", "s", "--subject", subject, "--evidence", evidence]);
        assert!(
            e["code"] == "selector_not_exact" || e["code"] == "quote_in_comment",
            "{evidence}: {e}"
        );
        assert!(repo.records("claims").is_empty(), "{evidence}");
    }
    let e = repo.fail(&[
        "claim",
        "s",
        "--subject",
        subject,
        "--evidence",
        "file:src/retry.rs#delay_ms::300 * 7",
    ]);
    assert_eq!(e["code"], "selector_not_exact");
    let message = text(&e["message"]);
    assert!(message.contains("\"300 * 7\" does not occur"), "{message}");
    assert!(message.contains("delay_ms"), "{message}");
    assert_eq!(
        e["remediation"],
        "Quote text that occurs in that unit, or leave the quote out to anchor the whole unit."
    );

    // The quote inside the declaration, and the symbol alone, are accepted.
    let id = repo.claim(
        "s",
        subject,
        &["file:src/retry.rs#delay_ms::300 * 2u64.pow(attempt)"],
    );
    repo.claim("t", subject, &["file:src/retry.ts#MAX_ATTEMPTS"]);

    // The same rule for a replaced and an added side of `reanchor`.
    for args in [
        ["--evidence", "1=file:src/retry.rs#delay_ms::300 * 7"],
        ["--add-evidence", "file:src/retry.rs#delay_ms::300 * 7"],
    ] {
        let e = repo.fail(&["reanchor", &id, args[0], args[1]]);
        assert_eq!(e["code"], "selector_not_exact", "{args:?}");
    }
    assert_eq!(repo.records("claims").len(), 2);
}

/// An anchor argument may be a JSON object: a path with `#` in it cannot
/// be written in compact form.
#[test]
fn a_json_anchor_argument_reaches_a_path_with_a_hash() {
    let repo = Repo::new();
    repo.write("docs/c#notes.md", "Timeouts are 30 seconds.\n");
    repo.write("docs/a.md", "Timeouts are 30 seconds.\n");
    let hash = r#"{"loc":"file:docs/c#notes.md","sel":{"quote":"30 seconds"}}"#;
    let v = repo.json(&["claim", "s", "--subject", hash]);
    let id = text(&v["claims"][0]["id"]).to_owned();
    assert_eq!(
        v["claims"][0]["claim"]["sides"][0]["anchor"]["loc"],
        "file:docs/c#notes.md"
    );
    // `reanchor` accepts it too, also in the `n=` form.
    let v = repo.json(&["reanchor", &id, "--add-evidence", hash]);
    let new = text(&v["new"]).to_owned();
    let other = r#"1={"loc":"file:docs/a.md","sel":{"quote":"30 seconds"}}"#;
    let v = repo.json(&["reanchor", &new, "--evidence", other]);
    assert_eq!(v["claim"]["sides"][1]["anchor"]["loc"], "file:docs/a.md");
    let e = repo.fail(&["claim", "s", "--subject", r#"{"loc":"file:docs/a.md"}"#]);
    assert_eq!(e["code"], "invalid_anchor");
    assert!(text(&e["remediation"]).contains("JSON object"));
}

/// A write that marks a claim says what state the claim was in and which
/// sides had changed, in JSON and in text, with the note.
#[test]
fn a_write_reports_what_it_accepted() {
    let repo = Repo::new();
    repo.write("docs/a.md", "Downloads are tried 5 times.\n");
    repo.write("src/a.rs", "pub const MAX: u32 = 5;\n");
    let args = [
        "claim",
        "Downloads are tried 5 times.",
        "--subject",
        "file:docs/a.md::tried 5 times",
        "--evidence",
        "file:src/a.rs#MAX",
    ];
    let at = |now: &'static str, rest: &[&'static str]| [&["--now", now][..], rest].concat();
    let first = repo.json(&at("2026-11-20T12:00:00Z", &args));
    let id = text(&first["claims"][0]["id"]).to_owned();
    assert_eq!(first["claims"][0]["accepted"], Value::Null);
    assert!(text(&first["nextNote"]).contains("retire the claim"));

    repo.write("src/a.rs", "pub const MAX: u32 = 7;\n");
    let stale = json!({"was": "stale", "sides": [
        {"index": 1, "role": "evidence", "loc": "file:src/a.rs", "state": "changed",
         "reason": "fingerprint_differs"}]});

    // The identical claim again: marked, and it says the claim was stale.
    let again = repo.json(&at("2026-11-20T12:01:00Z", &args));
    assert_eq!(again["claims"][0]["created"], false);
    assert_eq!(again["claims"][0]["accepted"], stale);
    assert_eq!(repo.state(&id), "fresh");

    repo.write("src/a.rs", "pub const MAX: u32 = 8;\n");
    let human = repo
        .hibi()
        .args(["--format", "human", "--now", "2026-11-20T12:02:00Z"])
        .args(args)
        .run();
    assert_eq!(human.status, 0, "{human:?}");
    assert!(
        human.stdout.contains(
            "  was stale: evidence 1 file:src/a.rs changed (fingerprint_differs); now marked"
        ),
        "{}",
        human.stdout
    );

    repo.write("src/a.rs", "pub const MAX: u32 = 9;\n");
    let marked = repo.json(&["--now", "2026-11-20T12:03:00Z", "mark", &id]);
    assert_eq!(marked["marks"][0]["accepted"], stale);
    assert!(marked["nextNote"].is_string());

    repo.write("src/a.rs", "pub const MAX: u32 = 10;\n");
    let human = repo.human(&["--now", "2026-11-20T12:04:00Z", "mark", &id]);
    assert!(
        human
            .stdout
            .contains("  was stale: evidence 1 file:src/a.rs changed"),
        "{}",
        human.stdout
    );

    // `reanchor` with a new statement also marks the changed evidence.
    repo.write("src/a.rs", "pub const MAX: u32 = 11;\n");
    let out = repo.json(&[
        "--now",
        "2026-11-20T12:05:00Z",
        "reanchor",
        &id,
        "--statement",
        "Downloads are tried.",
    ]);
    assert_eq!(out["accepted"], stale);
    assert_eq!(out["merged"], false);
    assert!(out["nextNote"].is_string());
    assert_eq!(out["claim"]["state"], "fresh");
}

/// `reanchor` onto an existing claim says it merged; a retired claim cannot
/// replace another; a write on a retired claim names its replacement.
#[test]
fn a_merge_is_reported_and_a_retired_claim_points_to_its_replacement() {
    let repo = Repo::new();
    repo.write("docs/a.md", "One sentence here. Another sentence here.\n");
    let a = repo.claim("A", "file:docs/a.md::One sentence here.", &[]);
    let b = repo.claim("B", "file:docs/a.md::One sentence here.", &[]);
    let c = repo.claim("C", "file:docs/a.md::Another sentence here.", &[]);
    let out = repo.json(&["reanchor", &a, "--statement", "B"]);
    assert_eq!(out["new"], b.as_str());
    assert_eq!(out["merged"], true);
    assert_eq!(repo.records("claims").len(), 3);

    let human = repo.human(&["mark", &a]);
    assert_eq!(human.status, 1);
    assert!(
        human.stderr.contains(&format!("replaced by {b}")),
        "{}",
        human.stderr
    );
    let e = repo.fail(&["mark", &a]);
    assert_eq!(e["code"], "claim_retired");
    assert_eq!(
        e["remediation"],
        format!("The claim was replaced by {b}: run hibi status {b}").as_str()
    );

    let e = repo.fail(&["retire", &c, "--replaced-by", &a]);
    assert_eq!(e["code"], "invalid_argument");
    assert!(text(&e["message"]).contains("is retired"));
    assert_eq!(repo.state(&c), "fresh");

    let e = repo.fail(&["reanchor", &b]);
    assert_eq!(e["code"], "no_change");
    assert!(text(&e["remediation"]).contains(&format!("hibi mark {b}")));
    let e = repo.fail(&["mark", "c1-!"]);
    assert_eq!(e["code"], "invalid_argument");
    assert!(text(&e["remediation"]).contains("hibi list"));
}

/// Spec 15.3: one run of a standard-input batch reports every failing
/// element, the malformed ones and the others, in index order, and writes
/// nothing.
#[test]
fn a_stdin_batch_reports_every_failing_element() {
    let repo = Repo::new();
    repo.write("docs/x.md", "A sentence with y in it.\n");
    let before = repo.store_files();
    let batch = r#"[
        {"statement":"a","subject":"file:nope.md::x"},
        {"statement":"b","subject":"file:docs/x.md::y","bogus":1},
        {"statement":"c","subject":"file:docs/x.md::y","recheck":"P1X"},
        {"statement":"d","subject":"file:docs/x.md::y","recheck":"P0D"},
        {"statement":"e","subject":"file:docs/x.md::y"}
    ]"#;
    let out = repo
        .hibi()
        .args(["--format", "json", "claim", "--stdin"])
        .stdin(batch)
        .run();
    let errors = out.errs();
    let got: Vec<(u64, &str)> = errors
        .iter()
        .map(|e| (e["index"].as_u64().unwrap(), text(&e["code"])))
        .collect();
    assert_eq!(
        got,
        [
            (0, "side_missing"),
            (1, "invalid_json"),
            (2, "invalid_duration"),
            (3, "invalid_duration"),
        ]
    );
    assert!(text(&errors[3]["message"]).contains("longer than zero"));
    assert_eq!(repo.store_files(), before);

    let id = repo.claim("e", "file:docs/x.md::y", &[]);
    let before = repo.store_files();
    let batch = format!(r#"["c1-zzzz", {{"id":"{id}"}}, "{id}", 7]"#);
    let out = repo
        .hibi()
        .args(["--format", "json", "mark", "--stdin"])
        .stdin(&batch)
        .run();
    let errors = out.errs();
    let got: Vec<(u64, &str)> = errors
        .iter()
        .map(|e| (e["index"].as_u64().unwrap(), text(&e["code"])))
        .collect();
    assert_eq!(
        got,
        [
            (0, "claim_not_found"),
            (1, "invalid_json"),
            (3, "invalid_json")
        ]
    );
    assert_eq!(repo.store_files(), before);
}

/// A recheck period of zero and a repeated evidence anchor are refused.
#[test]
fn a_zero_recheck_and_a_repeated_evidence_anchor_are_refused() {
    let repo = Repo::new();
    repo.write("docs/a.md", "Downloads are tried 5 times.\n");
    repo.write("src/a.rs", "pub const MAX: u32 = 5;\n");
    let subject = "file:docs/a.md::tried 5 times";
    for zero in ["P0D", "PT0S"] {
        let e = repo.fail(&["claim", "s", "--subject", subject, "--recheck", zero]);
        assert_eq!(e["code"], "invalid_duration", "{zero}");
    }
    let e = repo.fail(&[
        "claim",
        "s",
        "--subject",
        subject,
        "--evidence",
        "file:src/a.rs#MAX",
        "--evidence",
        "file:src/a.rs::pub const MAX: u32 = 5;",
    ]);
    assert_eq!(e["code"], "invalid_argument");
    assert!(text(&e["message"]).contains("evidence 1 and evidence 2 are the same anchor"));
    assert!(repo.records("claims").is_empty());
    let id = repo.claim("s", subject, &["file:src/a.rs#MAX"]);
    let e = repo.fail(&["reanchor", &id, "--recheck", "P0D"]);
    assert_eq!(e["code"], "invalid_duration");
}

/// A completed selector is a fixed point: `reanchor` without arguments on
/// unchanged files produces the same claim, for an anchor given by symbol,
/// by quote, and by symbol with a quote that occurs several times in the
/// declaration and in comments around it.
#[test]
fn reanchor_on_unchanged_files_is_no_change_for_every_anchor_form() {
    let repo = Repo::new();
    repo.write(
        "m.rs",
        "fn g() {\n    run(alpha); // run(alpha);\n}\n\n// run(alpha);\nfn f() {\n    run(alpha);\n    // run(alpha);\n    /* run(alpha); */\n    run(alpha); // run(alpha);\n}\n",
    );
    repo.write("docs/a.md", "The function f runs alpha twice.\n");
    let subject = "file:docs/a.md::runs alpha twice";
    for evidence in ["file:m.rs#f", "file:m.rs#f::(a", "file:m.rs#g"] {
        let v = repo.json(&[
            "claim",
            evidence,
            "--subject",
            subject,
            "--evidence",
            evidence,
        ]);
        let id = text(&v["claims"][0]["id"]).to_owned();
        let stored = v["claims"][0]["claim"]["sides"][1]["anchor"].clone();
        for round in 0..2 {
            let e = repo.fail(&["reanchor", &id]);
            assert_eq!(e["code"], "no_change", "{evidence}, round {round}: {e}");
        }
        // The stored anchor, given back as it is, selects the same unit
        // and completes to itself.
        let again = repo.json(&[
            "claim",
            evidence,
            "--subject",
            subject,
            "--evidence",
            &stored.to_string(),
        ]);
        assert_eq!(again["claims"][0]["id"], id.as_str(), "{evidence}");
        assert_eq!(again["claims"][0]["created"], false, "{evidence}");
        assert_eq!(repo.state(&id), "fresh");
    }
    // A quote anchor on the document, and the whole file.
    for anchor in [subject, "file:docs/a.md"] {
        let v = repo.json(&["claim", "about the document", "--subject", anchor]);
        let id = text(&v["claims"][0]["id"]).to_owned();
        let e = repo.fail(&["reanchor", &id]);
        assert_eq!(e["code"], "no_change", "{anchor}: {e}");
    }
    assert_eq!(repo.records("claims").len(), 5);
}

/// A stored claim names a file that git now ignores: the side reads as
/// missing, and no command prints what the file holds now.
#[test]
fn the_content_of_an_ignored_file_is_never_printed() {
    let repo = Repo::new();
    repo.config("[[scope]]\nname = \"all\"\nresource = \"file:**/*.md\"\nkinds = [\"sentence\"]\n");
    repo.write("notes.md", "The launch code is set.\n");
    repo.write("docs/a.md", "A public sentence.\n");
    repo.commit("base");
    let id = repo.claim(
        "The launch code is set.",
        "file:notes.md::The launch code is set.",
        &[],
    );

    repo.git(&["rm", "-q", "--cached", "notes.md"]);
    repo.write(".gitignore", "notes.md\n");
    repo.write("notes.md", "The launch code is tangerine.\n");

    let view = repo.view(&id);
    assert_eq!(view["state"], "stale");
    assert_eq!(view["sides"][0]["state"], "missing");
    assert_eq!(view["sides"][0]["reason"], "resource_not_found");
    assert_eq!(view["sides"][0]["unit"], Value::Null);

    let runs: [&[&str]; 7] = [
        &["status"],
        &["status", "--detail"],
        &["status", "--detail", &id],
        &["cover", "--limit", "0"],
        &["list"],
        &["list", &id],
        &["mark", &id],
    ];
    for args in runs {
        for format in ["json", "human"] {
            let out = repo.hibi().args(["--format", format]).args(args).run();
            assert!(
                !out.stdout.contains("tangerine") && !out.stderr.contains("tangerine"),
                "{args:?} in {format} printed the ignored file: {out:?}"
            );
        }
    }
    // A new claim on the ignored file is refused.
    let e = repo.fail(&["claim", "s", "--subject", "file:notes.md::tangerine"]);
    assert_eq!(e["code"], "side_missing");
    assert!(!e.to_string().contains("launch code is tangerine"), "{e}");
    // The listed file is still read.
    let cover = repo.json(&["cover"]);
    assert_eq!(cover["scopes"][0]["items"][0]["text"], "A public sentence.");
}

/// A file over the size limit, and a file that is not text, make a side
/// unavailable. The error, and the text output of `status` for a stored
/// claim, say what the limit or cause is and what the author can do.
#[test]
fn a_too_large_or_binary_file_says_the_limit_and_the_way_out() {
    let repo = Repo::new();
    repo.write("docs/a.md", "Small enough.\n");
    repo.write("big.md", "A sentence.\n");
    repo.write("data.md", "Text for now.\n");
    let big = repo.claim("s", "file:docs/a.md::Small enough.", &["file:big.md"]);
    let bin = repo.claim("t", "file:docs/a.md::Small enough.", &["file:data.md"]);

    // 8 MiB and one byte; and bytes that are not UTF-8.
    let mut large = "A sentence.\n".repeat(700_000);
    large.truncate(8 * 1024 * 1024 + 1);
    repo.write("big.md", &large);
    std::fs::write(repo.root.join("data.md"), [0xff, 0xfe, 0x00, 0x80]).unwrap();

    let limit = "The resource is over the size limit: 8 MiB, or 100,000 units in one resource.";
    let cause = "The resource is not UTF-8 text, so it has no units.";
    for (id, file, reason, says) in [
        (&big, "file:big.md", "too_large", limit),
        (&bin, "file:data.md", "not_text", cause),
    ] {
        let e = repo.fail(&["claim", "new", "--subject", file]);
        assert_eq!(e["code"], "side_unavailable", "{file}");
        assert!(text(&e["message"]).contains(reason), "{e}");
        let fix = text(&e["remediation"]);
        assert!(fix.starts_with(says), "{fix}");
        assert!(
            fix.contains("retire it") && fix.contains("add an exclusion"),
            "{fix}"
        );

        let e = repo.fail(&["mark", id]);
        assert_eq!(e["code"], "side_unavailable", "{file}");
        assert!(text(&e["remediation"]).starts_with(says), "{e}");

        let view = repo.view(id);
        assert_eq!(view["sides"][1]["state"], "unavailable");
        assert_eq!(view["sides"][1]["reason"], reason);
        assert_eq!(view["next"], Value::Null);
        // The JSON carries the sentence the text output prints, on the
        // side that cannot be evaluated and on no other.
        assert!(text(&view["sides"][1]["remediation"]).starts_with(says));
        assert_eq!(view["sides"][0]["remediation"], Value::Null);
        let human = repo.human(&["status", id]);
        assert!(human.stdout.contains(says), "{}", human.stdout);
    }
}
