//! Snapshots of what a person reads: the help of every command, and the
//! `--format human` output of every command in a fixed scenario.
//!
//! Every run passes `--now 2026-11-20T12:00:00Z` and works on fixed file
//! contents, so the content-addressed ids in the snapshots are stable. The
//! temporary directories and the version number are replaced with
//! placeholders.
//!
//! Write the snapshots with
//! `INSTA_UPDATE=always cargo test -p hibi-cli --test snapshots`.

mod support;

use support::*;

const COMMANDS: [&str; 14] = [
    "init",
    "claim",
    "status",
    "mark",
    "reanchor",
    "retire",
    "cover",
    "list",
    "triage",
    "suggest-evidence",
    "providers",
    "schema",
    "completions",
    "version",
];

const GUIDE: &str = "\
# Guide

The cache holds 64 entries. Requests time out after 30 seconds.

- Retries are capped at 3.
";

const OPS: &str = "\
# Operations

Workers are restarted nightly. The queue is drained on shutdown.
";

const CHANGELOG: &str = "\
# Changelog

Version 2 raised the cache size.
";

const CACHE_RS: &str = "\
pub fn limit() -> u32 {
    let max_entries = 64;
    max_entries
}

pub fn timeout() -> u32 {
    30
}
";

const CACHE_SUBJECT: &str = "file:docs/guide.md::The cache holds 64 entries.";
const TIMEOUT_SUBJECT: &str = "file:docs/guide.md::Requests time out after 30 seconds.";
const WORKERS_SUBJECT: &str = "file:docs/ops.md::Workers are restarted nightly.";
const LIMIT_EVIDENCE: &str = "file:src/cache.rs#limit::let max_entries = 64;";
const TIMEOUT_EVIDENCE: &str = "file:src/cache.rs#timeout";

/// A repository with a store and the scenario's files.
fn scenario() -> Repo {
    let repo = Repo::new();
    repo.write("docs/guide.md", GUIDE);
    repo.write("docs/ops.md", OPS);
    repo.write("docs/changelog.md", CHANGELOG);
    repo.write("src/cache.rs", CACHE_RS);
    repo
}

/// Replace what differs from run to run and from release to release: the
/// repository root, the directory of the user config and cache, the
/// temporary directory that holds both, and the version number.
fn redact(repo: &Repo, text: &str) -> String {
    text.replace(repo.root.to_str().unwrap(), "[ROOT]")
        .replace(repo.home.to_str().unwrap(), "[HOME]")
        .replace(repo.root.parent().unwrap().to_str().unwrap(), "[TMP]")
        .replace(env!("CARGO_PKG_VERSION"), "[VERSION]")
}

fn quoted(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_=/.:#,".contains(c))
    {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// The command line, the exit status and both streams of one run, as one
/// text.
fn render(repo: &Repo, args: &[&str], out: &Out) -> String {
    let line: Vec<String> = args.iter().map(|a| quoted(a)).collect();
    let stream = |text: &str| {
        if text.is_empty() {
            "(empty)\n".to_owned()
        } else {
            redact(repo, text)
        }
    };
    format!(
        "$ hibi {}\nexit status: {}\n---- standard output ----\n{}---- standard error ----\n{}",
        redact(repo, &line.join(" ")),
        out.status,
        stream(&out.stdout),
        stream(&out.stderr)
    )
}

/// One run with `--format human --now NOW`, rendered.
fn shot(repo: &Repo, args: &[&str]) -> String {
    let out = repo
        .hibi()
        .args(["--format", "human", "--now", NOW])
        .args(args)
        .run();
    render(repo, args, &out)
}

/// Several runs, one after the other, in one text.
fn shots(repo: &Repo, runs: &[&[&str]]) -> String {
    runs.iter()
        .map(|args| shot(repo, args))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The id of a claim made with the fixed `--now`.
fn claim(repo: &Repo, statement: &str, subject: &str, evidence: &[&str]) -> String {
    repo.claim(statement, subject, evidence)
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

fn help(repo: &Repo, args: &[&str]) -> String {
    // Help needs no root: run it outside the repository.
    let out = repo.hibi().cwd(&repo.home).args(args).run();
    assert_eq!(out.status, 0, "{args:?}: {out:?}");
    assert_eq!(out.stderr, "", "{args:?}");
    assert!(!out.stdout.is_empty(), "{args:?}");
    render(repo, args, &out)
}

#[test]
fn help_of_the_program() {
    let repo = Repo::bare();
    insta::assert_snapshot!("help", help(&repo, &["--help"]));
    insta::assert_snapshot!("help_short", help(&repo, &["-h"]));
}

#[test]
fn help_of_every_command() {
    let repo = Repo::bare();
    for command in COMMANDS {
        insta::assert_snapshot!(
            format!("help_{}", command.replace('-', "_")),
            help(&repo, &[command, "--help"])
        );
    }
}

/// The list above is the list of commands the binary has.
#[test]
fn the_help_snapshots_cover_every_command() {
    let mut actual: Vec<String> = hibi_cli::cli::command()
        .get_subcommands()
        .map(|c| c.get_name().to_owned())
        .filter(|name| name != "help")
        .collect();
    actual.sort();
    let mut expected: Vec<String> = COMMANDS.iter().map(|c| (*c).to_owned()).collect();
    expected.sort();
    assert_eq!(actual, expected);
}

// ---------------------------------------------------------------------------
// Human output
// ---------------------------------------------------------------------------

#[test]
fn human_init() {
    let repo = Repo::bare();
    insta::assert_snapshot!("human_init", shots(&repo, &[&["init"], &["init"]]));
}

#[test]
fn human_claim() {
    let repo = scenario();
    let created: &[&str] = &[
        "claim",
        "The cache holds 64 entries.",
        "--subject",
        CACHE_SUBJECT,
        "--evidence",
        LIMIT_EVIDENCE,
        "--evidence",
        TIMEOUT_EVIDENCE,
    ];
    let no_evidence: &[&str] = &[
        "claim",
        "Workers restart every night.",
        "--subject",
        WORKERS_SUBJECT,
        "--recheck",
        "P90D",
        "--note",
        "see the deploy schedule",
    ];
    // The first run creates the claim, the second marks it again.
    insta::assert_snapshot!(
        "human_claim",
        shots(&repo, &[created, created, no_evidence])
    );
}

#[test]
fn human_status() {
    let repo = scenario();
    let cache = claim(
        &repo,
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    claim(
        &repo,
        "Workers restart every night.",
        WORKERS_SUBJECT,
        &[TIMEOUT_EVIDENCE],
    );
    insta::assert_snapshot!("human_status_nothing_stale", shot(&repo, &["status"]));
    insta::assert_snapshot!(
        "human_status_fresh_claim_by_id",
        shot(&repo, &["status", &cache])
    );

    // The limit changes: one stale claim with a changed side.
    repo.write(
        "src/cache.rs",
        &CACHE_RS.replace("max_entries = 64", "max_entries = 128"),
    );
    insta::assert_snapshot!("human_status_one_stale", shot(&repo, &["status"]));
    insta::assert_snapshot!("human_status_detail", shot(&repo, &["status", "--detail"]));

    // The workers sentence is deleted: a second stale claim, with a
    // missing side.
    repo.write(
        "docs/ops.md",
        "# Operations\n\nThe queue is drained on shutdown.\n",
    );
    insta::assert_snapshot!("human_status_missing_side", shot(&repo, &["status"]));
    insta::assert_snapshot!(
        "human_status_every_state",
        shot(
            &repo,
            &[
                "status", "--state", "fresh", "--state", "stale", "--state", "expired", "--state",
                "retired"
            ]
        )
    );
}

#[test]
fn human_mark() {
    let repo = scenario();
    let cache = claim(
        &repo,
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    let workers = claim(&repo, "Workers restart every night.", WORKERS_SUBJECT, &[]);
    repo.write(
        "src/cache.rs",
        &CACHE_RS.replace("max_entries = 64", "max_entries = 128"),
    );
    insta::assert_snapshot!(
        "human_mark",
        shot(
            &repo,
            &[
                "mark",
                &cache,
                "--by",
                "nick",
                "--note",
                "read the function"
            ]
        )
    );

    // The sentence is reworded slightly: the mark is accepted and the
    // output shows what was marked.
    repo.write(
        "docs/ops.md",
        &OPS.replace("restarted nightly", "restarted every night"),
    );
    insta::assert_snapshot!(
        "human_mark_approx",
        shots(
            &repo,
            &[&["status"], &["mark", &workers], &["status", &workers]]
        )
    );
}

#[test]
fn human_reanchor() {
    let repo = scenario();
    let cache = claim(
        &repo,
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    insta::assert_snapshot!(
        "human_reanchor",
        shots(
            &repo,
            &[
                &["reanchor", &cache],
                &[
                    "reanchor",
                    &cache,
                    "--statement",
                    "The cache holds at most 64 entries.",
                    "--add-evidence",
                    TIMEOUT_EVIDENCE,
                    "--recheck",
                    "P30D"
                ],
                &["reanchor", &cache, "--note", "again"],
                &["status", &cache],
            ]
        )
    );
}

#[test]
fn human_retire() {
    let repo = scenario();
    let cache = claim(
        &repo,
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    let workers = claim(&repo, "Workers restart every night.", WORKERS_SUBJECT, &[]);
    let timeout = claim(
        &repo,
        "Requests time out after 30 seconds.",
        TIMEOUT_SUBJECT,
        &[TIMEOUT_EVIDENCE],
    );
    insta::assert_snapshot!(
        "human_retire",
        shots(
            &repo,
            &[
                &["retire", &cache, "--note", "section removed"],
                &["retire", &cache, &workers, "--replaced-by", &timeout],
                &["status"],
            ]
        )
    );
}

#[test]
fn human_cover() {
    let repo = scenario();
    repo.config(
        "[[scope]]\n\
         name = \"docs\"\n\
         resource = \"file:docs/**/*.md\"\n\
         kinds = [\"sentence\", \"list-item\"]\n\
         exclude = [\n  \
           { resource = \"file:docs/changelog.md\", reason = \"release notes, not claims\" },\n\
         ]\n\n\
         [[scope]]\n\
         name = \"code\"\n\
         resource = \"file:src/**/*.rs\"\n\
         kinds = [\"function\"]\n",
    );
    claim(
        &repo,
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE],
    );
    insta::assert_snapshot!("human_cover", shot(&repo, &["cover"]));
    insta::assert_snapshot!(
        "human_cover_limit_1",
        shot(&repo, &["cover", "--limit", "1"])
    );
    insta::assert_snapshot!(
        "human_cover_one_scope",
        shot(&repo, &["cover", "--scope", "code"])
    );
}

#[test]
fn human_list() {
    let repo = scenario();
    let cache = claim(
        &repo,
        "The cache holds 64 entries.",
        CACHE_SUBJECT,
        &[LIMIT_EVIDENCE, TIMEOUT_EVIDENCE],
    );
    let workers = claim(&repo, "Workers restart every night.", WORKERS_SUBJECT, &[]);
    repo.json(&[
        "--now",
        "2026-11-20T13:00:00Z",
        "mark",
        &cache,
        "--by",
        "nick",
        "--note",
        "read the function",
    ]);
    repo.json(&[
        "--now",
        "2026-11-20T14:00:00Z",
        "retire",
        &workers,
        "--note",
        "section removed",
    ]);
    insta::assert_snapshot!(
        "human_list",
        shots(
            &repo,
            &[
                &["list"],
                &["list", "--retired"],
                &["list", "--text", "no such statement"],
            ]
        )
    );
    insta::assert_snapshot!(
        "human_list_one_claim",
        shots(&repo, &[&["list", &cache], &["list", &workers]])
    );
}

#[test]
fn human_providers() {
    let repo = scenario();
    insta::assert_snapshot!("human_providers_default", shot(&repo, &["providers"]));
    repo.config("[repos]\napi = \"../api\"\n\n[schemes.x]\ndescription = \"Facts served by x\"\n");
    insta::assert_snapshot!("human_providers", shot(&repo, &["providers"]));
}

#[test]
fn human_schema_and_version() {
    let repo = Repo::bare();
    insta::assert_snapshot!("human_schema", shot(&repo, &["schema"]));
    insta::assert_snapshot!("human_version", shot(&repo, &["version"]));
}

#[test]
fn human_errors() {
    // A usage error.
    let repo = scenario();
    insta::assert_snapshot!(
        "human_error_usage",
        shots(
            &repo,
            &[
                &["status", "--no-such-option"],
                &["claim", "A statement without a subject."],
                &["frobnicate"],
            ]
        )
    );

    // No store.
    let bare = Repo::bare();
    insta::assert_snapshot!("human_error_not_initialized", shot(&bare, &["status"]));

    // A standard-input batch whose second and third elements fail: each
    // error names its index, and nothing is written.
    let batch = serde_json::json!([
        {"statement": "The cache holds 64 entries.", "subject": CACHE_SUBJECT},
        {"statement": "Workers restart at noon.", "subject": "file:docs/ops.md::Workers are restarted at noon sharp."},
        {"statement": "Requests time out.", "subject": TIMEOUT_SUBJECT, "recheck": "90 days"}
    ])
    .to_string();
    let before = repo.store_files();
    let args = ["claim", "--stdin"];
    let out = repo
        .hibi()
        .args(["--format", "human", "--now", NOW])
        .args(args)
        .stdin(&batch)
        .run();
    assert_eq!(repo.store_files(), before);
    insta::assert_snapshot!(
        "human_error_stdin_batch",
        format!("stdin: {batch}\n{}", render(&repo, &args, &out))
    );
}
