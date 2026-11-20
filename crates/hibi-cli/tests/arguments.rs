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
