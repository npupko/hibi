//! Argument handling that every command shares (spec section 15.1): the
//! global `--now`, the `command` of a usage error, and argument
//! combinations that are refused before any value is read.

mod support;

use support::*;

/// A malformed `--now` is `invalid_time` for every command, including the
/// ones that never read the clock.
#[test]
fn a_malformed_now_is_invalid_time_for_every_command() {
    let repo = Repo::new();
    repo.config(
        "[[scope]]\nname = \"docs\"\nresource = \"file:docs/**/*.md\"\nkinds = [\"sentence\"]\n",
    );
    repo.write("docs/a.md", "One sentence.\n");
    let before = repo.files();
    let commands: [&[&str]; 16] = [
        &["init"],
        &[
            "claim",
            "One.",
            "--subject",
            "file:docs/a.md::One sentence.",
        ],
        &["claim", "--stdin"],
        &["status"],
        &["mark", "c1-0000"],
        &["reanchor", "c1-0000"],
        &["retire", "c1-0000"],
        &["cover"],
        &["list"],
        &["triage"],
        &["suggest-evidence", "x", "--in", "file:docs/**"],
        &["status", "--judge-order"],
        &["providers"],
        &["schema"],
        &["completions", "bash"],
        &["version"],
    ];
    for args in commands {
        for bad in ["yesterday", "2026-11-20", "2026-11-20T18:04:11"] {
            let out = repo
                .hibi()
                .env("TYPESAFE_API_KEY", "k")
                .env("HIBI_JEV_URL", "http://127.0.0.1:9")
                .args(["--format", "json", "--now", bad])
                .args(args)
                .run();
            assert_eq!(out.code(), "invalid_time", "{args:?} --now {bad}");
            assert_eq!(out.json()["command"], args[0], "{args:?}");
        }
    }
    assert_eq!(repo.files(), before);

    // A well-formed value is accepted by a command that has no use for it.
    let v = repo.json(&["--now", "2026-11-20T18:04:11.5+02:00", "version"]);
    assert_eq!(v["command"], "version");
}

/// The `command` of a usage error is the subcommand clap matched. A
/// command name that is the value of an option is not the command.
#[test]
fn the_command_of_a_usage_error_is_the_matched_subcommand() {
    let repo = Repo::new();
    let cases: [(&[&str], &str); 7] = [
        (&["--bogus"], "hibi"),
        (&["status", "--bogus"], "status"),
        (&["--root", "status", "--bogus"], "hibi"),
        (&["--root", "status", "list", "--bogus"], "list"),
        (&["--now=mark", "retire"], "retire"),
        (&["nope", "status"], "hibi"),
        (&["--no-cache", "claim", "--stdin", "x"], "claim"),
    ];
    for (args, command) in cases {
        let out = repo.hibi().args(["--format", "json"]).args(args).run();
        assert_eq!(out.code(), "usage", "{args:?}");
        assert_eq!(out.json()["command"], command, "{args:?}");
        let help = if command == "hibi" {
            "hibi --help".to_owned()
        } else {
            format!("hibi {command} --help")
        };
        assert_eq!(out.err()["remediation"], help.as_str(), "{args:?}");
    }
}

/// `--since` with `--state` or with ids is `usage`, reported before the
/// range is interpreted: a bad range does not turn it into
/// `invalid_range`.
#[test]
fn since_with_state_or_ids_is_usage_before_the_range_is_read() {
    let repo = Repo::new();
    repo.commit("base");
    let cases: [&[&str]; 5] = [
        &["status", "--since", "HEAD", "--state", "stale"],
        &["status", "--since", "no-such-rev", "--state", "stale"],
        &["status", "--since", "A..B", "--state", "fresh"],
        &["status", "--since", "A....", "c1-0000"],
        &["status", "c1-0000", "--since", "HEAD..main"],
    ];
    for args in cases {
        assert_eq!(repo.run(args).code(), "usage", "{args:?}");
    }
    // Alone, the same bad ranges are `invalid_range`.
    for range in ["no-such-rev", "A..B", "A...."] {
        assert_eq!(
            repo.run(&["status", "--since", range]).code(),
            "invalid_range",
            "{range}"
        );
    }
    assert_eq!(
        repo.json(&["status", "--since", "HEAD"])["claims"],
        serde_json::json!([])
    );
}

/// A usage error that the library finds after parsing points at the help
/// of the command that was run, not at `hibi --help`.
#[test]
fn a_usage_error_found_after_parsing_names_the_command_help() {
    let repo = Repo::new();
    repo.write("docs/a.md", "One sentence. Another sentence.\n");
    let id = repo.claim(
        "One.",
        "file:docs/a.md::One sentence.",
        &["file:docs/a.md::Another sentence."],
    );
    let cases: [&[&str]; 3] = [
        &[
            "reanchor",
            &id,
            "--drop-evidence",
            "1",
            "--drop-evidence",
            "1",
        ],
        &["reanchor", &id, "--drop-evidence", "7"],
        &["reanchor", &id, "--evidence", "1"],
    ];
    for args in cases {
        let e = repo.fail(args);
        assert_eq!(e["code"], "usage", "{args:?}");
        assert_eq!(e["remediation"], "hibi reanchor --help", "{args:?}");
    }
    let e = repo.fail(&["status", "--since", "HEAD", "--state", "stale"]);
    assert_eq!(e["code"], "usage");
    assert_eq!(e["remediation"], "hibi status --help");
}

/// The remediation of `side_ambiguous` and `unsupported_selector` names
/// the selector keys the provider of that anchor accepts.
#[test]
fn selector_advice_comes_from_the_provider() {
    let repo = Repo::new();
    repo.config("[schemes.ref]\n");
    repo.user_config(&format!(
        "[providers.ref]\ncommand = [{:?}, \"--fixture\", {:?}]\n",
        reference_provider().to_string_lossy(),
        notes_fixture().to_string_lossy()
    ));
    // The same words in two different sentences.
    repo.write(
        "docs/a.md",
        "# Caps\n\nThe upload cap is 5.\n\nThe download cap is 5.\n",
    );

    // The file provider accepts a quote and a symbol, but the units of a
    // markdown file have no symbol: only the quote is advised.
    let e = repo.fail(&["claim", "S", "--subject", "file:docs/a.md::cap is 5."]);
    assert_eq!(e["code"], "side_ambiguous");
    assert_eq!(
        e["remediation"],
        "Give a longer quote, so that the anchor matches one unit."
    );
    // The same sentence twice with the same text around it: no quote tells
    // the two apart, and the advice does not say that one would.
    let block = "Padding sentence number one that is quite long indeed, really.\n\nThe cap is 5.\n\nPadding sentence number two that is quite long indeed, really.\n\n";
    repo.write("docs/twins.md", &block.repeat(2));
    let e = repo.fail(&[
        "claim",
        "S",
        "--subject",
        "file:docs/twins.md::The cap is 5.",
    ]);
    assert_eq!(e["code"], "side_ambiguous");
    assert!(
        e["message"]
            .as_str()
            .unwrap()
            .contains("matches 2 units that cannot be told apart: "),
        "{e}"
    );
    assert_eq!(
        e["remediation"],
        // Only what works for a claim, by what the units are: sentences
        // of a markdown file have no enclosing declaration, so the whole
        // file is the unit that contains them.
        "Units that are identical where they stand cannot be anchored one by one. Anchor the whole file instead: file:docs/twins.md"
    );
    // Twins inside a declaration: the enclosing declaration is named, and
    // anchoring it works.
    repo.write(
        "src/t.py",
        "class K:\n    def v(self):\n        return 1\n\n    def v(self):\n        return 1\n",
    );
    let e = repo.fail(&["claim", "S", "--subject", "file:src/t.py#K.v"]);
    assert_eq!(e["code"], "side_ambiguous", "{e}");
    assert_eq!(
        e["remediation"],
        "Units that are identical where they stand cannot be anchored one by one. Anchor the unit that contains them: the enclosing declaration file:src/t.py#K or the whole file file:src/t.py",
        "{e}"
    );
    let v = repo.json(&["claim", "S", "--subject", "file:src/t.py#K"]);
    assert_eq!(v["claims"][0]["created"], true);
    // The units of a code file have symbols.
    repo.write(
        "src/a.py",
        "class P:\n    def v(self):\n        return 1\n\n\nclass Q:\n    def v(self):\n        return 1\n",
    );
    let e = repo.fail(&["claim", "S", "--subject", "file:src/a.py::return 1"]);
    assert_eq!(e["code"], "side_ambiguous");
    assert_eq!(
        e["remediation"],
        "Give a symbol, or a longer quote, so that the anchor matches one unit."
    );
    // An anchor found only by similarity: the advice names what this
    // provider accepts and is useful here.
    let e = repo.fail(&["claim", "S", "--subject", "file:docs/a.md::The cap is 6."]);
    assert_eq!(e["code"], "selector_not_exact");
    assert_eq!(
        e["remediation"],
        "Give a quote that occurs exactly in the unit you mean; the candidates in the message have an anchor each."
    );
    let stdin = r#"[{"statement":"S","subject":{"loc":"file:docs/a.md","sel":{"key":"k"}}}]"#;
    let e = repo
        .hibi()
        .args(["--format", "json", "claim", "--stdin"])
        .stdin(stdin)
        .run()
        .err();
    assert_eq!(e["code"], "unsupported_selector");
    assert_eq!(
        e["remediation"],
        "Use only the selector keys this provider accepts: quote, symbol."
    );

    // The reference provider accepts a key and a quote, and no symbol.
    let e = repo.fail(&["claim", "S", "--subject", "ref:notes"]);
    assert_eq!(e["code"], "side_ambiguous");
    assert_eq!(
        e["remediation"],
        "Give a longer quote, or the unit's key, so that the anchor matches one unit."
    );
    let stdin = r#"[{"statement":"S","subject":{"loc":"ref:notes","sel":{"symbol":"x"}}}]"#;
    let e = repo
        .hibi()
        .args(["--format", "json", "claim", "--stdin"])
        .stdin(stdin)
        .run()
        .err();
    assert_eq!(e["code"], "unsupported_selector");
    assert_eq!(
        e["remediation"],
        "Use only the selector keys this provider accepts: key, quote."
    );
}

/// Section 15.3: the twin sentence names a declaration only when one unit
/// of the resource holds every listed twin and its own anchor selects it
/// alone. The name is never read off the symbol: `Id.fmt` is a method of an
/// `impl` block, and `#Id` is the struct, which holds no method. Nothing
/// follows a locator in the sentence, so each can be copied as it stands.
#[test]
fn the_twin_sentence_names_a_declaration_only_when_one_holds_the_twins() {
    const WHOLE: &str = "Units that are identical where they stand cannot be anchored one by one. Anchor the whole file instead: ";
    let repo = Repo::new();
    repo.write("docs/a.md", "# A\n\nText.\n");
    // Rust: two `impl` blocks for one struct, each with the same `fmt`.
    let impls = "impl fmt::Display for Id {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        write!(f, \"id-{}\", self.0)\n    }\n}\n\nimpl fmt::Debug for Id {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        write!(f, \"id-{}\", self.0)\n    }\n}\n";
    repo.write(
        "src/id.rs",
        &format!("use std::fmt;\n\npub struct Id(pub u64);\n\n{impls}"),
    );
    // The same, with the struct declared in another file.
    repo.write("src/other.rs", "pub struct Key(pub u64);\n");
    repo.write(
        "src/fmt.rs",
        &format!("use std::fmt;\nuse crate::other::Id;\n\n{impls}"),
    );
    // TypeScript: a merged interface, the same member in both parts.
    repo.write(
        "src/p.ts",
        "export interface Props {\n  onClose(): void;\n}\n\nexport interface Props {\n  onClose(): void;\n}\n",
    );
    // Go: two methods of one type; the type does not hold them.
    repo.write(
        "src/g.go",
        "package g\n\ntype S struct{}\n\nfunc (s S) M() int {\n\treturn 1\n}\n\nfunc (s S) M() int {\n\treturn 1\n}\n",
    );
    // TypeScript: two members of one class. The class holds both.
    repo.write(
        "src/n.ts",
        "export class Box {\n  get size(): number {\n    return 1;\n  }\n  get size(): number {\n    return 1;\n  }\n}\n",
    );
    for (anchor, file) in [
        ("file:src/id.rs#Id.fmt", "file:src/id.rs"),
        ("file:src/fmt.rs#Id.fmt", "file:src/fmt.rs"),
        ("file:src/p.ts#Props.onClose", "file:src/p.ts"),
        ("file:src/g.go#S.M", "file:src/g.go"),
    ] {
        let e = repo.fail(&["claim", "S", "--subject", anchor]);
        assert_eq!(e["code"], "side_ambiguous", "{anchor}: {e}");
        assert_eq!(
            e["remediation"],
            format!("{WHOLE}{file}").as_str(),
            "{anchor}"
        );
        let message = e["message"].as_str().unwrap();
        assert!(!message.contains("enclosing declaration"), "{message}");
        assert!(message.ends_with(&format!("{WHOLE}{file}")), "{message}");
    }
    // What the old sentence named for the Rust file is the struct, which
    // stays fresh when an `impl` changes: it is not offered.
    let v = repo.json(&["claim", "S", "--subject", "file:src/id.rs#Id"]);
    assert_eq!(
        v["claims"][0]["claim"]["sides"][0]["unit"]["kind"],
        "struct"
    );
    // The whole file, as the sentence says, does hold the twins.
    let v = repo.json(&["claim", "whole", "--subject", "file:src/id.rs"]);
    let whole = v["claims"][0]["claim"]["id"].as_str().unwrap().to_owned();
    repo.write(
        "src/id.rs",
        &format!(
            "use std::fmt;\n\npub struct Id(pub u64);\n\n{}",
            impls.replacen("id-{}", "Id({})", 1)
        ),
    );
    assert_eq!(repo.state(&whole), "stale");

    // One class holds both twins: it is named, and anchoring it works.
    let e = repo.fail(&["claim", "S", "--subject", "file:src/n.ts#Box.size"]);
    assert_eq!(
        e["remediation"],
        "Units that are identical where they stand cannot be anchored one by one. Anchor the unit that contains them: the enclosing declaration file:src/n.ts#Box or the whole file file:src/n.ts",
        "{e}"
    );
    let v = repo.json(&["claim", "box", "--subject", "file:src/n.ts#Box"]);
    assert_eq!(v["claims"][0]["claim"]["sides"][0]["unit"]["kind"], "class");
}
