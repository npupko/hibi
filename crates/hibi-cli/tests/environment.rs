//! What the binary does with its environment: the judge key, a missing or
//! old git, how git is invoked, the cache, a legacy store, and the shape of
//! every failure. Spec sections 10.4, 11.3, 12.5, 14.1, 15.1 and 15.4;
//! requirements R22, R23 (V6), R32, R34, R39, R40, R44, R60 and R64.

mod support;

use std::ffi::OsString;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use support::*;

const DOC: &str = "Alpha is one. Beta is two.\n";
const SOURCE: &str = "pub fn first() -> u32 {\n    1\n}\n\npub fn second() -> u32 {\n    2\n}\n";
const DOCS_SCOPE: &str =
    "[[scope]]\nname = \"docs\"\nresource = \"file:docs/**/*.md\"\nkinds = [\"sentence\"]\n";

/// The closed list of section 15.4.
const CODES: &[&str] = &[
    "usage",
    "invalid_argument",
    "invalid_json",
    "invalid_anchor",
    "invalid_duration",
    "invalid_time",
    "invalid_range",
    "not_initialized",
    "already_initialized",
    "not_a_git_repository",
    "legacy_store",
    "unsupported_store_version",
    "invalid_config",
    "invalid_user_config",
    "store_corrupt",
    "store_locked",
    "io_error",
    "git_failed",
    "git_too_old",
    "claim_not_found",
    "ambiguous_id",
    "claim_retired",
    "no_change",
    "unknown_scheme",
    "side_missing",
    "side_ambiguous",
    "side_unavailable",
    "selector_not_exact",
    "quote_spans_units",
    "quote_in_comment",
    "unsupported_selector",
    "no_scopes",
    "unknown_scope",
    "too_many_candidates",
    "judge_unavailable",
    "judge_failed",
    "internal",
];

/// A TCP listener on the loopback interface that stands in for the judge
/// endpoint (`HIBI_JEV_URL`).
///
/// What it proves: after a set of runs, `assert_untouched` shows that no TCP
/// connection to this address reached the listener's queue, so hibi did not
/// contact the endpoint it was configured to use for the judge.
///
/// What it does not prove: that the process opened no socket at all. A
/// connection to any other address (a hard-coded host, a DNS lookup) would
/// not be seen here. Observing that needs a sandbox or a system-call trace
/// around the process, which this test does not have.
struct Endpoint {
    listener: TcpListener,
    url: String,
}

impl Endpoint {
    fn new() -> Endpoint {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        Endpoint { listener, url }
    }

    fn assert_untouched(&self) {
        match self.listener.accept() {
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            other => panic!("a connection was made to the judge endpoint: {other:?}"),
        }
    }

    /// The control: a connection that is made is seen by `accept`.
    fn assert_detects_a_connection(&self) {
        let _client = TcpStream::connect(self.listener.local_addr().unwrap()).unwrap();
        for _ in 0..200 {
            match self.listener.accept() {
                Ok(_) => return,
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("accept failed: {e}"),
            }
        }
        panic!("the listener did not see a connection that was made");
    }
}

/// The real `PATH` with this directory first.
fn path_with(bin: &Bin) -> OsString {
    let mut dirs = vec![bin.dir.clone()];
    dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    std::env::join_paths(dirs).unwrap()
}

fn clear(bin: &Bin) {
    std::fs::remove_file(&bin.log).ok();
}

fn replace(repo: &Repo, rel: &str, from: &str, to: &str) {
    let text = repo.read(rel);
    assert_eq!(text.matches(from).count(), 1, "{rel}: {from:?}");
    repo.write(rel, &text.replace(from, to));
}

fn ids(v: &Value) -> Vec<String> {
    v["claims"]
        .as_array()
        .expect("claims is an array")
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect()
}

fn warning_codes(v: &Value) -> Vec<String> {
    v["warnings"]
        .as_array()
        .expect("warnings is an array")
        .iter()
        .map(|w| {
            assert!(w["message"].is_string(), "{w}");
            w["code"].as_str().unwrap().to_owned()
        })
        .collect()
}

fn counts(v: &Value) -> (u64, u64, u64, u64) {
    (
        v["units"].as_u64().unwrap(),
        v["covered"].as_u64().unwrap(),
        v["excluded"].as_u64().unwrap(),
        v["uncovered"].as_u64().unwrap(),
    )
}

/// A sibling git repository next to `repo` with one document, declared as
/// alias `api` in `hibi.toml` and allowed in the user config (sections 11.1
/// and 11.2). Returns its directory.
fn sibling(repo: &Repo) -> PathBuf {
    let api = repo.root.parent().unwrap().join("api");
    std::fs::create_dir_all(api.join("docs")).unwrap();
    let init = Command::new(real_git())
        .current_dir(&api)
        .args(["init", "-q", "-b", "main", "."])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(init.success());
    std::fs::write(
        api.join("docs/api.md"),
        "The API answers. It never sleeps.\n",
    )
    .unwrap();
    repo.config("[repos]\napi = \"../api\"\n");
    repo.user_config(&format!(
        "[[repo]]\nroot = {:?}\nalias = \"api\"\npath = {:?}\n",
        repo.root.to_str().unwrap(),
        api.to_str().unwrap()
    ));
    api
}

/// A committed repository with scope `docs`, a document, a source file, a
/// claim on files of the root only and a claim with a `repo:` evidence side.
struct WithSibling {
    repo: Repo,
    api: PathBuf,
    local: String,
    remote: String,
}

fn with_sibling() -> WithSibling {
    let repo = Repo::new();
    repo.config(DOCS_SCOPE);
    let api = sibling(&repo);
    repo.write("docs/a.md", DOC);
    repo.write("src/a.rs", SOURCE);
    let local = repo.claim(
        "alpha is one",
        "file:docs/a.md::Alpha is one.",
        &["file:src/a.rs#first"],
    );
    let remote = repo.claim(
        "the API answers",
        "file:docs/a.md::Beta is two.",
        &["repo:api//docs/api.md::The API answers."],
    );
    let view = repo.view(&remote);
    assert_eq!(view["state"], "fresh", "{view}");
    assert_eq!(view["sides"][1]["anchor"]["loc"], "repo:api//docs/api.md");
    assert_eq!(view["sides"][1]["state"], "same");
    assert_eq!(view["sides"][1]["unit"]["label"], "The API answers.");
    repo.commit("base");
    WithSibling {
        repo,
        api,
        local,
        remote,
    }
}

/// A committed repository with scope `docs` and one claim that has no
/// `repo:` side. Returns the claim id.
fn local_only() -> (Repo, String) {
    let repo = Repo::new();
    repo.config(DOCS_SCOPE);
    repo.write("docs/a.md", DOC);
    repo.write("src/a.rs", SOURCE);
    let id = repo.claim(
        "alpha is one",
        "file:docs/a.md::Alpha is one.",
        &["file:src/a.rs#first"],
    );
    repo.commit("base");
    (repo, id)
}

// ---------------------------------------------------------------------------
// The judge key (V6)
// ---------------------------------------------------------------------------

/// R22 (V6): with `TYPESAFE_API_KEY` unset (and with it empty), every command
/// that is not a judge command works, and none of them connects to the judge
/// endpoint. See [`Endpoint`] for what the connection check does and does
/// not prove.
#[test]
fn r22_every_other_command_works_without_a_key_and_contacts_no_judge() {
    let endpoint = Endpoint::new();

    for key in [None, Some("")] {
        let repo = Repo::bare();
        let run = |args: &[&str]| {
            let mut cmd = repo
                .hibi()
                .env("HIBI_JEV_URL", &endpoint.url)
                .args(["--format", "json", "--now", NOW])
                .args(args);
            if let Some(key) = key {
                cmd = cmd.env("TYPESAFE_API_KEY", key);
            }
            cmd.run()
        };

        let v = run(&["init"]).ok();
        assert_eq!(v["command"], "init");
        assert_eq!(v["root"], repo.root.to_str().unwrap());
        assert_eq!(v["created"].as_array().unwrap().len(), 36);
        assert!(repo.root.join(".hibi/hibi.toml").is_file());

        repo.config(DOCS_SCOPE);
        repo.write("docs/a.md", DOC);
        repo.write("src/a.rs", SOURCE);

        let v = run(&[
            "claim",
            "alpha is one",
            "--subject",
            "file:docs/a.md::Alpha is one.",
            "--evidence",
            "file:src/a.rs#first",
        ])
        .ok();
        assert_eq!(v["claims"][0]["created"], true);
        assert_eq!(v["claims"][0]["claim"]["state"], "fresh");
        let id = v["claims"][0]["id"].as_str().unwrap().to_owned();
        repo.commit("base");

        let v = run(&["status"]).ok();
        assert_eq!(ids(&v), Vec::<String>::new());
        assert_eq!(
            v["summary"],
            json!({"claims": 1, "fresh": 1, "drifted": 0, "stale": 0, "expired": 0, "retired": 0})
        );

        replace(&repo, "src/a.rs", "    1\n", "    11\n");
        let v = run(&["status"]).ok();
        assert_eq!(ids(&v), vec![id.clone()]);
        assert_eq!(v["claims"][0]["state"], "stale");
        assert_eq!(v["claims"][0]["sides"][1]["state"], "changed");
        let v = run(&["status", "--since", "HEAD"]).ok();
        assert_eq!(ids(&v), vec![id.clone()]);

        let v = run(&["mark", &id]).ok();
        assert_eq!(v["marks"][0]["claim"], id.as_str());
        assert_eq!(v["marks"][0]["claimView"]["state"], "fresh");

        let v = run(&["cover"]).ok();
        assert_eq!(counts(&v["summary"]), (2, 1, 0, 1), "{v}");
        assert_eq!(v["scopes"][0]["items"][0]["text"], "Beta is two.");
        let v = run(&["cover", "--since", "HEAD"]).ok();
        assert_eq!(counts(&v["summary"]), (0, 0, 0, 0), "{v}");

        let v = run(&["reanchor", &id, "--statement", "alpha equals one"]).ok();
        assert_eq!(v["old"], id.as_str());
        let new = v["new"].as_str().unwrap().to_owned();
        assert_ne!(new, id);
        assert_eq!(v["claim"]["state"], "fresh");
        assert_eq!(v["claim"]["statement"], "alpha equals one");

        let v = run(&["list"]).ok();
        assert_eq!(ids(&v), vec![new.clone()]);
        let v = run(&["list", "--retired"]).ok();
        assert_eq!(v["claims"].as_array().unwrap().len(), 2);

        let v = run(&["retire", &new]).ok();
        assert_eq!(
            v["retired"][0],
            json!({"claim": new, "retirement": v["retired"][0]["retirement"], "alreadyRetired": false})
        );
        assert!(
            v["retired"][0]["retirement"]
                .as_str()
                .unwrap()
                .starts_with("r1-")
        );
        let v = run(&["status"]).ok();
        assert_eq!(
            v["summary"],
            json!({"claims": 2, "fresh": 0, "drifted": 0, "stale": 0, "expired": 0, "retired": 2})
        );

        let v = run(&["providers"]).ok();
        assert_eq!(v["builtin"]["schemes"], json!(["file", "repo"]));
        let v = run(&["schema"]).ok();
        assert!(
            v["schemas"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n == "claim")
        );
        let out = run(&["schema", "claim"]);
        assert_eq!(out.status, 0, "{out:?}");
        assert_eq!(out.stderr, "");
        let schema: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        let out = run(&["completions", "bash"]);
        assert_eq!(out.status, 0, "{out:?}");
        assert!(out.stdout.contains("hibi"));
        let v = run(&["version"]).ok();
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(v["judgeModel"], "jev-1.14.0");

        // The same commands in human format.
        for args in [
            vec!["status"],
            vec!["cover"],
            vec!["list"],
            vec!["providers"],
            vec!["version"],
        ] {
            let mut cmd = repo
                .hibi()
                .env("HIBI_JEV_URL", &endpoint.url)
                .args(["--format", "human"])
                .args(&args);
            if let Some(key) = key {
                cmd = cmd.env("TYPESAFE_API_KEY", key);
            }
            let out = cmd.run();
            assert_eq!(out.status, 0, "{args:?}: {out:?}");
            assert_eq!(out.stderr, "", "{args:?}");
        }
    }

    endpoint.assert_untouched();
    endpoint.assert_detects_a_connection();
}

/// R23 (V6): with the key unset or empty, `triage`, `suggest-evidence` and
/// `status --judge-order` exit 1 with `judge_unavailable`, before any other
/// work: no file under `.hibi/` changes, the judge endpoint is not
/// contacted, and the same error comes without git and without a scope.
#[test]
fn r23_judge_commands_without_a_key_are_judge_unavailable() {
    let endpoint = Endpoint::new();
    let (repo, id) = local_only();
    replace(&repo, "src/a.rs", "    1\n", "    11\n");
    assert_eq!(repo.state(&id), "stale");
    let no_git = Bin::new(&repo.home, "empty");

    let invocations: Vec<Vec<&str>> = vec![
        vec!["triage"],
        vec!["triage", "--scope", "docs", "--limit", "5"],
        vec!["triage", "--since", "HEAD"],
        vec!["suggest-evidence", "alpha is one", "--in", "file:src/**"],
        vec![
            "suggest-evidence",
            "alpha is one",
            "--in",
            "file:src/**",
            "--kind",
            "function",
            "--limit",
            "3",
        ],
        vec!["status", "--judge-order"],
        vec!["status", "--judge-order", "--state", "stale"],
    ];
    let before_store = repo.store_files();
    let before = repo.files();

    for key in [None, Some("")] {
        for path in [None, Some(no_git.only())] {
            for args in &invocations {
                let build = |format: &str| {
                    let mut cmd = repo
                        .hibi()
                        .env("HIBI_JEV_URL", &endpoint.url)
                        .args(["--format", format])
                        .args(args);
                    if let Some(key) = key {
                        cmd = cmd.env("TYPESAFE_API_KEY", key);
                    }
                    if let Some(path) = &path {
                        cmd = cmd.env("PATH", path);
                    }
                    cmd
                };
                let out = build("json").run();
                let e = out.err();
                assert_eq!(e["code"], "judge_unavailable", "{args:?}: {e}");
                assert_eq!(out.errs().len(), 1);
                assert_eq!(out.json()["command"], args[0]);

                let out = build("human").run();
                assert_eq!(out.status, 1, "{args:?}: {out:?}");
                assert_eq!(out.stdout, "", "{args:?}");
                assert!(out.stderr.contains("judge_unavailable"), "{out:?}");
            }
        }
    }
    assert_eq!(repo.store_files(), before_store);
    assert_eq!(repo.files(), before);

    // Without a scope `triage` still fails on the key first.
    let bare = Repo::new();
    let before = bare.files();
    assert_eq!(bare.run(&["triage"]).code(), "judge_unavailable");
    assert_eq!(bare.files(), before);

    // The flag asks for the judge; plain `status` works.
    let v = repo.json(&["status"]);
    assert_eq!(ids(&v), vec![id]);

    endpoint.assert_untouched();
    endpoint.assert_detects_a_connection();
}

// ---------------------------------------------------------------------------
// git absent (R39, R64)
// ---------------------------------------------------------------------------

/// R39: with no `git` on `PATH`, `claim`, `status`, `mark`, `list`,
/// `reanchor` and `retire` work for claims with no `repo:` side, with no
/// warning. `init`, `cover` and `--since` need git and fail with
/// `not_a_git_repository`.
#[test]
fn r39_commands_that_do_not_need_git_work_without_git() {
    let repo = Repo::new();
    repo.config(DOCS_SCOPE);
    repo.write("docs/a.md", DOC);
    repo.write("src/a.rs", SOURCE);
    repo.commit("base");

    let empty = Bin::new(&repo.home, "empty");
    assert!(std::fs::read_dir(&empty.dir).unwrap().next().is_none());
    let run = |args: &[&str]| {
        repo.hibi()
            .env("PATH", empty.only())
            .args(["--format", "json", "--now", NOW])
            .args(args)
            .run()
    };
    let no_warnings = |v: &Value| assert_eq!(v["warnings"], json!([]), "{v}");

    let v = run(&[
        "claim",
        "alpha is one",
        "--subject",
        "file:docs/a.md::Alpha is one.",
        "--evidence",
        "file:src/a.rs#first",
    ])
    .ok();
    no_warnings(&v);
    assert_eq!(v["claims"][0]["created"], true);
    let claim = &v["claims"][0]["claim"];
    assert_eq!(claim["state"], "fresh");
    assert_eq!(claim["markedAt"], NOW);
    assert_eq!(claim["sides"][0]["unit"]["label"], "Alpha is one.");
    assert_eq!(claim["sides"][1]["unit"]["key"], "function:first");
    let id = v["claims"][0]["id"].as_str().unwrap().to_owned();
    let first_mark = v["claims"][0]["mark"].as_str().unwrap().to_owned();

    let v = run(&["status"]).ok();
    no_warnings(&v);
    assert_eq!(ids(&v), Vec::<String>::new());
    assert_eq!(
        v["summary"],
        json!({"claims": 1, "fresh": 1, "drifted": 0, "stale": 0, "expired": 0, "retired": 0})
    );

    replace(&repo, "src/a.rs", "    1\n", "    11\n");
    let v = run(&["status"]).ok();
    no_warnings(&v);
    assert_eq!(ids(&v), vec![id.clone()]);
    let stale = &v["claims"][0];
    assert_eq!(stale["state"], "stale");
    assert_eq!(stale["sides"][0]["state"], "same");
    assert_eq!(stale["sides"][1]["state"], "changed");
    assert_eq!(stale["sides"][1]["reason"], "fingerprint_differs");
    assert_eq!(stale["next"], format!("hibi mark {id}"));
    let v = run(&["status", "--detail", &id]).ok();
    assert_eq!(
        v["claims"][0]["sides"][1]["text"],
        "pub fn first() -> u32 {\n    11\n}"
    );

    let v = run(&["mark", &id, "--note", "checked"]).ok();
    no_warnings(&v);
    assert_eq!(v["marks"].as_array().unwrap().len(), 1);
    assert_eq!(v["marks"][0]["claim"], id.as_str());
    let second_mark = v["marks"][0]["mark"].as_str().unwrap().to_owned();
    assert_ne!(second_mark, first_mark);
    assert_eq!(v["marks"][0]["claimView"]["state"], "fresh");
    assert_eq!(v["marks"][0]["claimView"]["mark"], second_mark.as_str());

    let v = run(&["list"]).ok();
    no_warnings(&v);
    assert_eq!(
        v["claims"],
        json!([{"id": id, "statement": "alpha is one", "subject": "file:docs/a.md",
                "evidence": ["file:src/a.rs"], "recheck": null, "note": null,
                "marks": 2, "lastMarkedAt": NOW, "retired": false}])
    );
    let v = run(&["list", &id]).ok();
    assert_eq!(v["claim"]["id"], id.as_str());
    assert_eq!(v["marks"].as_array().unwrap().len(), 2);
    assert_eq!(v["retirements"], json!([]));

    let v = run(&[
        "reanchor",
        &id,
        "--subject",
        "file:docs/a.md::Beta is two.",
        "--statement",
        "beta is two",
    ])
    .ok();
    no_warnings(&v);
    assert_eq!(v["old"], id.as_str());
    let new = v["new"].as_str().unwrap().to_owned();
    assert_ne!(new, id);
    assert!(v["mark"].as_str().unwrap().starts_with("m1-"));
    assert!(v["retirement"].as_str().unwrap().starts_with("r1-"));
    assert_eq!(v["claim"]["id"], new.as_str());
    assert_eq!(v["claim"]["state"], "fresh");
    assert_eq!(v["claim"]["sides"][0]["unit"]["label"], "Beta is two.");

    let v = run(&["status", &id, &new]).ok();
    let states: Vec<(String, String)> = v["claims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["id"].as_str().unwrap().to_owned(),
                c["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mut expected = vec![
        (id.clone(), "retired".to_owned()),
        (new.clone(), "fresh".to_owned()),
    ];
    expected.sort();
    assert_eq!(states, expected);

    let v = run(&["retire", &new, "--note", "gone"]).ok();
    no_warnings(&v);
    assert_eq!(v["retired"].as_array().unwrap().len(), 1);
    assert_eq!(v["retired"][0]["claim"], new.as_str());
    assert_eq!(v["retired"][0]["alreadyRetired"], false);
    let v = run(&["retire", &new]).ok();
    assert_eq!(
        v["retired"],
        json!([{"claim": new, "retirement": null, "alreadyRetired": true}])
    );
    let v = run(&["status"]).ok();
    assert_eq!(
        v["summary"],
        json!({"claims": 2, "fresh": 0, "drifted": 0, "stale": 0, "expired": 0, "retired": 2})
    );
    run(&["providers"]).ok();
    run(&["version"]).ok();

    // The commands that need git.
    let before = repo.files();
    for args in [
        vec!["cover"],
        vec!["cover", "--scope", "docs"],
        vec!["cover", "--since", "HEAD"],
        vec!["status", "--since", "HEAD"],
        vec!["init"],
    ] {
        let out = run(&args);
        assert_eq!(out.code(), "not_a_git_repository", "{args:?}: {out:?}");
    }
    assert_eq!(repo.files(), before);

    let fresh = Repo::bare();
    let out = fresh
        .hibi()
        .env("PATH", empty.only())
        .args(["--format", "json", "init"])
        .run();
    assert_eq!(out.code(), "not_a_git_repository", "{out:?}");
    assert!(!fresh.root.join(".hibi").exists());
}

/// Section 10.4, first column: with git absent a `repo:` side is
/// `unavailable` with `repo_not_found`, and the run carries no warning.
#[test]
fn r39_a_repo_side_without_git_is_repo_not_found() {
    let s = with_sibling();
    let empty = Bin::new(&s.repo.home, "empty");
    let out = s
        .repo
        .hibi()
        .env("PATH", empty.only())
        .args(["--format", "json", "status"])
        .run();
    let v = out.ok();
    assert_eq!(v["warnings"], json!([]));
    assert_eq!(ids(&v), vec![s.remote.clone()]);
    let claim = &v["claims"][0];
    assert_eq!(claim["state"], "stale");
    assert_eq!(claim["sides"][0]["state"], "same");
    assert_eq!(claim["sides"][1]["state"], "unavailable");
    assert_eq!(claim["sides"][1]["reason"], "repo_not_found");
    assert_eq!(claim["next"], Value::Null);
    assert_eq!(
        v["summary"],
        json!({"claims": 2, "fresh": 1, "drifted": 0, "stale": 1, "expired": 0, "retired": 0})
    );
}

/// R64 and section 14.1: with a key set and git absent, the judge commands
/// fail with `not_a_git_repository` before any network contact.
#[test]
fn r64_judge_commands_with_a_key_and_no_git_are_not_a_git_repository() {
    let endpoint = Endpoint::new();
    let (repo, _id) = local_only();
    let empty = Bin::new(&repo.home, "empty");
    let before = repo.files();

    for args in [
        vec!["triage"],
        vec!["suggest-evidence", "alpha is one", "--in", "file:src/**"],
        vec!["status", "--judge-order"],
    ] {
        for url in [endpoint.url.as_str(), "http://127.0.0.1:9"] {
            let out = repo
                .hibi()
                .env("PATH", empty.only())
                .env("TYPESAFE_API_KEY", "test-key-not-real")
                .env("HIBI_JEV_URL", url)
                .args(["--format", "json"])
                .args(&args)
                .run();
            let e = out.err();
            assert_eq!(e["code"], "not_a_git_repository", "{args:?}: {e}");
            assert_eq!(out.json()["command"], args[0]);
        }
    }
    assert_eq!(repo.files(), before);
    endpoint.assert_untouched();
    endpoint.assert_detects_a_connection();
}

// ---------------------------------------------------------------------------
// git too old (R60)
// ---------------------------------------------------------------------------

fn assert_names_both_versions(e: &Value) {
    assert_eq!(e["code"], "git_too_old", "{e}");
    let message = e["message"].as_str().unwrap();
    assert!(message.contains("2.30.0"), "{message}");
    assert!(message.contains("2.45.0"), "{message}");
}

/// R60: with a git older than 2.45.0 as the only `git` on `PATH`, the
/// commands that need git fail with `git_too_old` naming both versions, and
/// nothing but `git --version` is run.
#[test]
fn r60_commands_that_need_git_fail_with_git_too_old() {
    let endpoint = Endpoint::new();
    let (repo, id) = local_only();
    let old = Bin::new(&repo.home, "old-git");
    old.fake_git("git version 2.30.0");
    let before = repo.files();

    for path in [old.only(), path_with(&old)] {
        for args in [
            vec!["cover"],
            vec!["cover", "--scope", "docs"],
            vec!["cover", "--since", "HEAD"],
            vec!["status", "--since", "HEAD"],
            vec!["status", "--since", "main..."],
            vec!["init"],
        ] {
            clear(&old);
            let out = repo
                .hibi()
                .env("PATH", &path)
                .args(["--format", "json"])
                .args(&args)
                .run();
            assert_names_both_versions(&out.err());
            assert_eq!(out.errs().len(), 1);
            assert_eq!(old.calls(), ["git --version"], "{args:?}");
        }

        // The judge commands, with a key set: the same, before any network
        // contact.
        for args in [
            vec!["triage"],
            vec!["suggest-evidence", "alpha is one", "--in", "file:src/**"],
            vec!["status", "--judge-order"],
        ] {
            for url in [endpoint.url.as_str(), "http://127.0.0.1:9"] {
                clear(&old);
                let out = repo
                    .hibi()
                    .env("PATH", &path)
                    .env("TYPESAFE_API_KEY", "test-key-not-real")
                    .env("HIBI_JEV_URL", url)
                    .args(["--format", "json"])
                    .args(&args)
                    .run();
                assert_names_both_versions(&out.err());
                assert_eq!(old.calls(), ["git --version"], "{args:?}");
            }
        }

        // `init` in a repository that has no store yet.
        let fresh = Repo::bare();
        clear(&old);
        let out = fresh
            .hibi()
            .env("PATH", &path)
            .args(["--format", "json", "init"])
            .run();
        assert_names_both_versions(&out.err());
        assert!(!fresh.root.join(".hibi").exists());
        assert_eq!(old.calls(), ["git --version"]);

        // Human format: exit 1, the error on standard error.
        clear(&old);
        let out = repo
            .hibi()
            .env("PATH", &path)
            .args(["--format", "human", "cover"])
            .run();
        assert_eq!(out.status, 1, "{out:?}");
        assert_eq!(out.stdout, "");
        assert!(out.stderr.contains("git_too_old"), "{out:?}");
        assert!(out.stderr.contains("2.30.0") && out.stderr.contains("2.45.0"));
        assert_eq!(old.calls(), ["git --version"]);
    }
    assert_eq!(repo.files(), before);
    assert_eq!(repo.state(&id), "fresh");
    endpoint.assert_untouched();
}

/// R60, second row: in every other command a `repo:` side is `unavailable`
/// with `repo_not_found` and the run carries one warning `git_too_old`; a run
/// with no `repo:` side has no warning. Only `git --version` is run.
#[test]
fn r60_a_repo_side_with_an_old_git_is_repo_not_found_with_a_warning() {
    let s = with_sibling();
    let repo = &s.repo;
    let old = Bin::new(&repo.home, "old-git");
    old.fake_git("git version 2.30.0");
    let run = |repo: &Repo, args: &[&str]| {
        repo.hibi()
            .env("PATH", old.only())
            .args(["--format", "json", "--now", NOW])
            .args(args)
            .run()
    };

    clear(&old);
    let v = run(repo, &["status"]).ok();
    assert_eq!(warning_codes(&v), ["git_too_old"], "{v}");
    let message = v["warnings"][0]["message"].as_str().unwrap();
    assert!(
        message.contains("2.30.0") && message.contains("2.45.0"),
        "{message}"
    );
    assert_eq!(ids(&v), vec![s.remote.clone()]);
    assert_eq!(
        v["summary"],
        json!({"claims": 2, "fresh": 1, "drifted": 0, "stale": 1, "expired": 0, "retired": 0})
    );
    let claim = &v["claims"][0];
    assert_eq!(claim["state"], "stale");
    assert_eq!(claim["sides"][0]["state"], "same");
    let side = &claim["sides"][1];
    assert_eq!(side["anchor"]["loc"], "repo:api//docs/api.md");
    assert_eq!(side["state"], "unavailable");
    assert_eq!(side["reason"], "repo_not_found");
    assert_eq!(side["unit"], Value::Null);
    assert_eq!(claim["next"], Value::Null);
    assert_eq!(old.calls(), ["git --version"]);

    // Human format: the warning goes to standard error.
    clear(&old);
    let out = repo
        .hibi()
        .env("PATH", old.only())
        .args(["--format", "human", "status"])
        .run();
    assert_eq!(out.status, 0, "{out:?}");
    assert!(out.stderr.contains("git_too_old"), "{out:?}");
    assert!(out.stdout.contains("repo_not_found"), "{out:?}");
    assert_eq!(old.calls(), ["git --version"]);

    // Writes on the `repo:` side fail; nothing is written.
    let before = repo.store_files();
    clear(&old);
    let out = run(repo, &["mark", &s.remote]);
    let e = out.err();
    assert_eq!(e["code"], "side_unavailable", "{e}");
    assert!(
        e["message"].as_str().unwrap().contains("repo_not_found"),
        "{e}"
    );
    assert_eq!(old.calls(), ["git --version"]);
    clear(&old);
    let out = run(
        repo,
        &[
            "claim",
            "it never sleeps",
            "--subject",
            "repo:api//docs/api.md::It never sleeps.",
        ],
    );
    assert_eq!(out.code(), "side_unavailable", "{out:?}");
    assert_eq!(old.calls(), ["git --version"]);
    assert_eq!(repo.store_files(), before);

    // Commands that resolve no `repo:` side: no warning, and at most the
    // version probe.
    clear(&old);
    let v = run(repo, &["list"]).ok();
    assert_eq!(v["warnings"], json!([]));
    assert_eq!(ids(&v).len(), 2);
    let v = run(repo, &["retire", &s.remote]).ok();
    assert_eq!(v["warnings"], json!([]));
    let v = run(repo, &["status"]).ok();
    assert_eq!(
        v["warnings"],
        json!([]),
        "a retired claim's sides are not resolved"
    );
    assert_eq!(ids(&v), Vec::<String>::new());
    assert!(
        old.calls().iter().all(|c| c == "git --version"),
        "{:?}",
        old.calls()
    );

    // A store with no `repo:` side at all.
    let (plain, id) = local_only();
    clear(&old);
    let v = run(&plain, &["status", "--state", "fresh"]).ok();
    assert_eq!(v["warnings"], json!([]));
    assert_eq!(ids(&v), vec![id.clone()]);
    assert_eq!(v["claims"][0]["state"], "fresh");
    let v = run(&plain, &["mark", &id]).ok();
    assert_eq!(v["warnings"], json!([]));
    let v = run(
        &plain,
        &[
            "claim",
            "beta is two",
            "--subject",
            "file:docs/a.md::Beta is two.",
        ],
    )
    .ok();
    assert_eq!(v["warnings"], json!([]));
    assert_eq!(v["claims"][0]["claim"]["state"], "fresh");
    assert!(
        old.calls().iter().all(|c| c == "git --version"),
        "{:?}",
        old.calls()
    );
}

// ---------------------------------------------------------------------------
// How git is invoked (R44)
// ---------------------------------------------------------------------------

/// The git subcommand of one logged call and the directory given with `-C`.
fn parse_git_call(line: &str) -> (String, String) {
    let words: Vec<&str> = line.split(' ').collect();
    assert_eq!(words[0], "git", "{line}");
    let mut dir = String::new();
    let mut i = 1;
    while i < words.len() {
        match words[i] {
            "--no-pager" => i += 1,
            "-C" => {
                dir = words[i + 1].to_owned();
                i += 2;
            }
            "-c" => i += 2,
            other => return (other.to_owned(), dir),
        }
    }
    panic!("no subcommand in {line}");
}

/// R44 and section 10.4: every git process has `--no-pager`,
/// `-c safe.bareRepository=explicit` and `-c core.fsmonitor=false` (and the
/// other fixed options) in its arguments, except the one `git --version`
/// probe, which is the first call of each run. Only the commands of the
/// table in section 10.4 are run, and in a sibling repository only
/// `rev-parse --show-toplevel` and the file list.
#[test]
fn r44_every_git_call_carries_the_fixed_options() {
    let s = with_sibling();
    let repo = &s.repo;
    replace(repo, "src/a.rs", "    1\n", "    11\n");
    repo.write("docs/new.md", "A new sentence.\n");
    let bin = Bin::new(&repo.home, "rec");
    bin.recording_git();
    let path = path_with(&bin);
    let root = repo.root.to_str().unwrap().to_owned();
    let api = s.api.to_str().unwrap().to_owned();

    // (repository, arguments, whether the run must call git)
    let fresh = Repo::bare();
    let runs: Vec<(&Repo, Vec<&str>, bool)> = vec![
        (&fresh, vec!["init"], true),
        (
            repo,
            vec![
                "claim",
                "a new sentence",
                "--subject",
                "file:docs/new.md::A new sentence.",
            ],
            false,
        ),
        (repo, vec!["status"], true),
        (repo, vec!["status", "--since", "HEAD"], true),
        (repo, vec!["status", "--since", "main..."], true),
        (repo, vec!["cover"], true),
        (repo, vec!["cover", "--since", "HEAD"], true),
        (repo, vec!["cover", "--since", "main...HEAD"], true),
        (repo, vec!["mark", &s.remote], true),
        (repo, vec!["providers"], true),
        (repo, vec!["list"], false),
    ];

    let mut subcommands = std::collections::BTreeSet::new();
    let mut in_sibling = 0;
    for (target, args, needs_git) in &runs {
        clear(&bin);
        let out = target
            .hibi()
            .env("PATH", &path)
            .args(["--format", "json", "--now", NOW])
            .args(args)
            .run();
        let v = out.ok();
        assert_eq!(v["warnings"], json!([]), "{args:?}");
        let calls = bin.calls();
        if *needs_git {
            assert!(calls.len() >= 2, "{args:?}: {calls:?}");
        }
        if calls.is_empty() {
            continue;
        }
        assert_eq!(calls[0], "git --version", "{args:?}: {calls:?}");
        let target_root = target.root.to_str().unwrap();
        for call in &calls[1..] {
            assert_ne!(call, "git --version", "{args:?}: probed twice: {calls:?}");
            assert!(call.starts_with("git --no-pager -C "), "{args:?}: {call}");
            for option in [
                "-c core.quotePath=false",
                "-c safe.bareRepository=explicit",
                "-c core.fsmonitor=false",
                "-c core.untrackedCache=false",
            ] {
                assert!(
                    call.contains(&format!(" {option} ")),
                    "{args:?}: {option} is missing in {call}"
                );
            }
            let (sub, dir) = parse_git_call(call);
            assert!(
                [
                    "rev-parse",
                    "merge-base",
                    "ls-files",
                    "diff",
                    "ls-tree",
                    "cat-file"
                ]
                .contains(&sub.as_str()),
                "{args:?}: {call}"
            );
            if dir == api {
                in_sibling += 1;
                assert!(
                    call.ends_with(" rev-parse --show-toplevel")
                        || call.contains(" ls-files -z --cached --others --exclude-standard"),
                    "{args:?}: not allowed in a sibling repository: {call}"
                );
            } else {
                assert_eq!(dir, target_root, "{args:?}: {call}");
            }
            subcommands.insert(sub);
        }
    }
    assert_ne!(root, api);
    assert!(in_sibling >= 2, "the sibling repository was read");
    // The runs above exercised every row of the table.
    assert_eq!(
        subcommands.into_iter().collect::<Vec<_>>(),
        [
            "cat-file",
            "diff",
            "ls-files",
            "ls-tree",
            "merge-base",
            "rev-parse"
        ]
    );

    // The result with the recording git equals the result with the real one.
    let v = repo.json(&["status", "--since", "HEAD"]);
    assert_eq!(ids(&v), vec![s.local.clone()]);
}

// ---------------------------------------------------------------------------
// The cache (R40)
// ---------------------------------------------------------------------------

fn without_warnings(stdout: &str) -> Value {
    let mut v: Value = serde_json::from_str(stdout).unwrap();
    v.as_object_mut().unwrap().remove("warnings");
    v
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// R40 and section 11.3: deleting the cache database changes no output of a
/// command that does not call the judge; `--no-cache` gives the same output
/// and creates no cache file; a cache that cannot be opened gives the same
/// results plus the warning `cache_unavailable`.
#[test]
fn r40_the_cache_changes_no_output() {
    let repo = Repo::new();
    repo.config(DOCS_SCOPE);
    repo.config(
        "[[scope]]\nname = \"code\"\nresource = \"file:src/**/*.rs\"\nkinds = [\"function\"]\n",
    );
    repo.write("docs/a.md", DOC);
    repo.write(
        "docs/b.md",
        "# Title\n\nGamma is three.\n\n- Delta is four.\n",
    );
    repo.write("src/a.rs", SOURCE);
    let stale = repo.claim(
        "alpha is one",
        "file:docs/a.md::Alpha is one.",
        &["file:src/a.rs#first"],
    );
    let fresh = repo.claim("second is two", "file:src/a.rs#second", &[]);
    repo.commit("base");
    replace(&repo, "src/a.rs", "    1\n", "    11\n");
    repo.write("docs/c.md", "Epsilon is five.\n");

    let commands: Vec<Vec<&str>> = vec![
        vec!["status"],
        vec!["status", "--detail", &stale, &fresh],
        vec![
            "status",
            "--state",
            "fresh",
            "--state",
            "stale",
            "--loc",
            "file:src/**",
        ],
        vec!["status", "--since", "HEAD"],
        vec!["cover"],
        vec!["cover", "--scope", "code", "--limit", "1"],
        vec!["cover", "--since", "HEAD"],
        vec!["list"],
        vec!["list", &stale],
        vec!["providers"],
    ];
    // Whether the command reads units, and so would use the cache.
    let reads_units = |args: &[&str]| args[0] == "status" || args[0] == "cover";

    let run_all = |extra: &[&str], cache_dir: Option<&Path>| -> Vec<Out> {
        let mut outs = Vec::new();
        for format in ["json", "human"] {
            for args in &commands {
                let mut cmd = repo
                    .hibi()
                    .args(["--format", format, "--now", NOW])
                    .args(extra)
                    .args(args);
                if let Some(dir) = cache_dir {
                    cmd = cmd.env("HIBI_CACHE_DIR", dir);
                }
                let out = cmd.run();
                assert_eq!(out.status, 0, "{format} {args:?}: {out:?}");
                outs.push(out);
            }
        }
        outs
    };
    let stdouts = |outs: &[Out]| -> Vec<String> { outs.iter().map(|o| o.stdout.clone()).collect() };

    // The claims above already used the cache. Start from none.
    let database = repo.cache_dir().join("cache.sqlite3");
    assert!(
        database.is_file(),
        "the cache is at $HIBI_CACHE_DIR/cache.sqlite3"
    );
    std::fs::remove_dir_all(repo.cache_dir()).unwrap();

    let cold = run_all(&[], None);
    for out in &cold {
        assert_eq!(out.stderr, "", "{out:?}");
    }
    // The baseline is what the scenario says, not just self-consistent.
    let status: Value = serde_json::from_str(&cold[0].stdout).unwrap();
    assert_eq!(ids(&status), vec![stale.clone()]);
    assert_eq!(status["now"], NOW);
    let cover: Value = serde_json::from_str(&cold[4].stdout).unwrap();
    assert_eq!(counts(&cover["summary"]), (6, 3, 0, 3), "{cover}");
    assert!(database.is_file(), "a run wrote the cache");

    // Warm.
    let warm = run_all(&[], None);
    assert_eq!(stdouts(&warm), stdouts(&cold));

    // Deleted, then run again.
    std::fs::remove_dir_all(repo.cache_dir()).unwrap();
    assert!(!database.exists());
    let again = run_all(&[], None);
    assert_eq!(stdouts(&again), stdouts(&cold));
    for out in &again {
        assert_eq!(out.stderr, "", "{out:?}");
    }
    assert!(database.is_file());

    // --no-cache with no cache present: same output, no file created.
    std::fs::remove_dir_all(repo.cache_dir()).unwrap();
    let uncached = run_all(&["--no-cache"], None);
    assert_eq!(stdouts(&uncached), stdouts(&cold));
    for out in &uncached {
        assert_eq!(out.stderr, "", "{out:?}");
    }
    assert_eq!(files_under(&repo.cache_dir()), Vec::<PathBuf>::new());

    // --no-cache with a cache present: same output, the file is not written.
    run_all(&[], None);
    let bytes = std::fs::read(&database).unwrap();
    let modified = std::fs::metadata(&database).unwrap().modified().unwrap();
    let uncached = run_all(&["--no-cache"], None);
    assert_eq!(stdouts(&uncached), stdouts(&cold));
    assert_eq!(std::fs::read(&database).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&database).unwrap().modified().unwrap(),
        modified
    );

    // A cache directory that cannot exist: its parent is a regular file.
    let blocker = repo.home.join("not-a-directory");
    std::fs::write(&blocker, "x").unwrap();
    let bad_dir = blocker.join("cache");
    let broken = run_all(&[], Some(&bad_dir));
    let half = commands.len();
    for (n, args) in commands.iter().enumerate() {
        // JSON: the same result; the warning where the run would have used
        // the cache.
        let out = &broken[n];
        assert_eq!(out.stderr, "", "{args:?}");
        assert_eq!(
            without_warnings(&out.stdout),
            without_warnings(&cold[n].stdout),
            "{args:?}"
        );
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        if reads_units(args) {
            assert_eq!(warning_codes(&v), ["cache_unavailable"], "{args:?}: {v}");
        } else {
            let codes = warning_codes(&v);
            assert!(
                codes.is_empty() || codes == ["cache_unavailable"],
                "{args:?}: {v}"
            );
        }
        // Human: the same standard output; the warning on standard error.
        let out = &broken[half + n];
        assert_eq!(out.stdout, cold[half + n].stdout, "{args:?}");
        if reads_units(args) {
            assert!(
                out.stderr.contains("cache_unavailable"),
                "{args:?}: {out:?}"
            );
        }
    }
    assert_eq!(std::fs::read_to_string(&blocker).unwrap(), "x");

    // With --no-cache the run would not have used the cache: no warning.
    let broken_uncached = run_all(&["--no-cache"], Some(&bad_dir));
    assert_eq!(stdouts(&broken_uncached), stdouts(&cold));
    for out in &broken_uncached {
        assert_eq!(out.stderr, "", "{out:?}");
    }
}

// ---------------------------------------------------------------------------
// Legacy store (R32)
// ---------------------------------------------------------------------------

const LEGACY_MESSAGE: &str =
    "this repository has a hibi 0.x store in .claims/, which this version does not read";
const LEGACY_REMEDIATION: &str = "remove or move .claims/, or use hibi 0.7";

fn assert_legacy(out: &Out, what: &str) {
    let errors = out.errs();
    assert_eq!(errors.len(), 1, "{what}: {out:?}");
    let e = &errors[0];
    assert_eq!(e["code"], "legacy_store", "{what}: {e}");
    assert_eq!(e["message"], LEGACY_MESSAGE, "{what}");
    assert_eq!(e["remediation"], LEGACY_REMEDIATION, "{what}");
}

/// Every command that section 12.5 refuses, with a claim id where one is
/// needed.
fn refused_commands(id: &str) -> Vec<Vec<String>> {
    let list: Vec<Vec<&str>> = vec![
        vec!["init"],
        vec![
            "claim",
            "beta is two",
            "--subject",
            "file:docs/a.md::Beta is two.",
        ],
        vec!["status"],
        vec!["status", id],
        vec!["status", "--detail"],
        vec!["status", "--since", "HEAD"],
        vec!["mark", id],
        vec!["reanchor", id, "--statement", "other"],
        vec!["retire", id],
        vec!["cover"],
        vec!["cover", "--since", "HEAD"],
        vec!["list"],
        vec!["list", id],
        vec!["providers"],
        vec!["providers", "--check", "ref"],
    ];
    list.into_iter()
        .map(|args| args.into_iter().map(str::to_owned).collect())
        .collect()
}

fn judge_commands() -> Vec<Vec<&'static str>> {
    vec![
        vec!["triage"],
        vec!["suggest-evidence", "alpha is one", "--in", "file:src/**"],
        vec!["status", "--judge-order"],
    ]
}

/// The checks of R32 on one repository whose root holds `.claims/`.
fn assert_everything_is_refused(repo: &Repo, id: &str, endpoint: &Endpoint) {
    let before = repo.files();
    assert!(before.keys().any(|k| k.starts_with(".claims/")));

    for args in refused_commands(id) {
        let what = format!("{args:?}");
        let out = repo.hibi().args(["--format", "json"]).args(&args).run();
        assert_legacy(&out, &what);
        assert_eq!(out.json()["command"], args[0].as_str(), "{what}");

        let out = repo.hibi().args(["--format", "human"]).args(&args).run();
        assert_eq!(out.status, 1, "{what}: {out:?}");
        assert_eq!(out.stdout, "", "{what}");
        assert!(out.stderr.contains("legacy_store"), "{what}: {out:?}");
        assert!(out.stderr.contains(LEGACY_MESSAGE), "{what}: {out:?}");
        assert!(out.stderr.contains(LEGACY_REMEDIATION), "{what}: {out:?}");
    }

    // The standard-input forms.
    for (command, input) in [
        (
            "claim",
            json!([{"statement": "x", "subject": "file:docs/a.md::Beta is two."}]),
        ),
        ("mark", json!([id])),
    ] {
        let out = repo
            .hibi()
            .args(["--format", "json", command, "--stdin"])
            .stdin(&input.to_string())
            .run();
        assert_legacy(&out, command);
    }

    // The judge commands with a key set. (With the key unset, section 14.1
    // and section 12.5 name different errors; see the next test.)
    for args in judge_commands() {
        let out = repo
            .hibi()
            .env("TYPESAFE_API_KEY", "test-key-not-real")
            .env("HIBI_JEV_URL", &endpoint.url)
            .args(["--format", "json"])
            .args(&args)
            .run();
        assert_legacy(&out, &format!("{args:?}"));
    }

    // From a subdirectory and with --root.
    let sub = repo.root.join("docs");
    std::fs::create_dir_all(&sub).unwrap();
    let before_sub = repo.files();
    let out = repo
        .hibi()
        .cwd(&sub)
        .args(["--format", "json", "status"])
        .run();
    assert_legacy(&out, "status in a subdirectory");
    let out = repo
        .hibi()
        .cwd(&repo.home)
        .args([
            "--format",
            "json",
            "--root",
            repo.root.to_str().unwrap(),
            "status",
        ])
        .run();
    assert_legacy(&out, "status --root");
    assert_eq!(repo.files(), before_sub);

    // version, schema and completions are not refused.
    let v = repo.json(&["version"]);
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    let v = repo.json(&["schema"]);
    assert!(!v["schemas"].as_array().unwrap().is_empty());
    let out = repo.run(&["schema", "claim"]);
    assert_eq!(out.status, 0, "{out:?}");
    let out = repo.hibi().args(["completions", "zsh"]).run();
    assert_eq!(out.status, 0, "{out:?}");
    assert!(out.stdout.contains("hibi"));

    assert_eq!(repo.files(), before_sub);
    endpoint.assert_untouched();
}

/// R32: a root that holds `.hibi/` and a directory `.claims/`.
#[test]
fn r32_a_store_next_to_a_legacy_store_is_refused() {
    let endpoint = Endpoint::new();
    let (repo, id) = local_only();
    repo.write(".claims/claims.jsonl", "{\"id\":\"old\"}\n");
    repo.commit("legacy");
    assert_everything_is_refused(&repo, &id, &endpoint);

    // Removing `.claims/` is the remediation: the store works again.
    std::fs::remove_dir_all(repo.root.join(".claims")).unwrap();
    assert_eq!(repo.state(&id), "fresh");

    // An empty `.claims/` directory is refused as well; a regular file of
    // that name is not a legacy store.
    std::fs::create_dir(repo.root.join(".claims")).unwrap();
    assert_legacy(&repo.run(&["status"]), "an empty .claims/");
    std::fs::remove_dir(repo.root.join(".claims")).unwrap();
    repo.write(".claims", "a file\n");
    assert_eq!(repo.state(&id), "fresh");
}

/// R32: a git repository that holds only `.claims/` and no `.hibi/`. `init`
/// does not create a store next to it.
#[test]
fn r32_a_legacy_store_alone_is_refused_and_init_creates_nothing() {
    let endpoint = Endpoint::new();
    let repo = Repo::bare();
    repo.write(".claims/claims.jsonl", "{\"id\":\"old\"}\n");
    repo.write("docs/a.md", DOC);
    repo.commit("legacy");
    assert_everything_is_refused(&repo, "c1-0000", &endpoint);
    assert!(!repo.root.join(".hibi").exists());

    // Not yet committed, in a repository with no commit.
    let repo = Repo::bare();
    std::fs::create_dir(repo.root.join(".claims")).unwrap();
    assert_legacy(&repo.run(&["init"]), "init");
    assert_legacy(&repo.run(&["status"]), "status");
    assert!(!repo.root.join(".hibi").exists());
}

/// Sections 12.5 and 14.1 both apply to a judge command run without a key in
/// a root that holds `.claims/`. The spec does not order them; either error
/// is a refusal that changes no file.
#[test]
fn r32_a_judge_command_without_a_key_is_refused_in_a_legacy_root() {
    let (repo, _id) = local_only();
    repo.write(".claims/claims.jsonl", "{}\n");
    let before = repo.files();
    for args in judge_commands() {
        let out = repo.run(&args);
        let code = out.code();
        assert!(
            code == "legacy_store" || code == "judge_unavailable",
            "{args:?}: {out:?}"
        );
    }
    assert_eq!(repo.files(), before);
}

// ---------------------------------------------------------------------------
// Failures (R34)
// ---------------------------------------------------------------------------

struct Failing {
    what: &'static str,
    args: Vec<String>,
    stdin: Option<String>,
    /// Run without `--format`, to test a bad `--format` value itself.
    raw: bool,
    code: &'static str,
}

fn failing(what: &'static str, args: &[&str], code: &'static str) -> Failing {
    Failing {
        what,
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        stdin: None,
        raw: false,
        code,
    }
}

fn failing_stdin(what: &'static str, args: &[&str], stdin: &str, code: &'static str) -> Failing {
    Failing {
        stdin: Some(stdin.to_owned()),
        ..failing(what, args, code)
    }
}

/// Two claims in one store whose ids share their first four characters after
/// `c1-`, found by claiming many statements on one sentence in one batch.
/// Ids are derived from content, so the outcome is the same on every run.
fn claims_with_a_shared_prefix(repo: &Repo) -> (String, Vec<String>) {
    let input: Vec<Value> = (0..3000)
        .map(|n| json!({"statement": format!("statement {n}"), "subject": "file:docs/many.md::Many claims."}))
        .collect();
    let v = repo
        .hibi()
        .args(["--format", "json", "--now", NOW, "claim", "--stdin"])
        .stdin(&Value::Array(input).to_string())
        .run()
        .ok();
    let mut all: Vec<String> = v["claims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect();
    all.sort();
    let pair = all
        .windows(2)
        .find(|w| w[0][..7] == w[1][..7])
        .expect("3000 ids over 2^20 prefixes hold a shared prefix");
    let prefix = pair[0][..7].to_owned();
    let with_prefix: Vec<String> = all
        .iter()
        .filter(|i| i.starts_with(&prefix))
        .cloned()
        .collect();
    (prefix, with_prefix)
}

/// R34: every failing run exits exactly 1. In `json` format it prints one
/// object with `ok: false` and a code from section 15.4, with standard error
/// empty; in `human` format standard output is empty and the error is on
/// standard error.
#[test]
fn r34_every_failure_exits_1_with_one_error_object() {
    let repo = Repo::new();
    repo.config(DOCS_SCOPE);
    repo.write("docs/a.md", DOC);
    repo.write(
        "src/a.rs",
        "// A comment about first.\npub fn first() -> u32 {\n    1\n}\n",
    );
    let live = repo.claim("alpha is one", "file:docs/a.md::Alpha is one.", &[]);
    let retired = repo.claim("beta is two", "file:docs/a.md::Beta is two.", &[]);
    repo.json(&["retire", &retired]);
    repo.commit("base");

    let uninitialised = Repo::bare();

    let mut table = vec![
        // Usage errors.
        failing("no command", &[], "usage"),
        failing("unknown command", &["frobnicate"], "usage"),
        failing("unknown option", &["status", "--frobnicate"], "usage"),
        failing(
            "unknown global option",
            &["--frobnicate", "status"],
            "usage",
        ),
        failing("missing argument", &["mark"], "usage"),
        failing("missing option value", &["status", "--state"], "usage"),
        failing("missing statement and subject", &["claim"], "usage"),
        failing(
            "claim without --subject",
            &["claim", "a statement"],
            "usage",
        ),
        failing("reanchor without an id", &["reanchor"], "usage"),
        failing("retire without an id", &["retire"], "usage"),
        failing(
            "suggest-evidence without --in",
            &["suggest-evidence", "x"],
            "usage",
        ),
        failing("completions without a shell", &["completions"], "usage"),
        failing(
            "completions for an unknown shell",
            &["completions", "tcsh"],
            "usage",
        ),
        failing("an unexpected argument", &["version", "extra"], "usage"),
        failing(
            "--since with --state",
            &["status", "--since", "HEAD", "--state", "stale"],
            "usage",
        ),
        failing(
            "--since with an id",
            &["status", "--since", "HEAD", &live],
            "usage",
        ),
        failing(
            "reanchor naming an evidence number that does not exist",
            &["reanchor", &live, "--drop-evidence", "3"],
            "usage",
        ),
        failing_stdin(
            "claim --stdin with a statement",
            &["claim", "a statement", "--stdin"],
            "[]",
            "usage",
        ),
        failing_stdin(
            "claim --stdin with --subject",
            &["claim", "--stdin", "--subject", "file:docs/a.md"],
            "[]",
            "usage",
        ),
        Failing {
            raw: true,
            ..failing(
                "bad --format value",
                &["--format", "xml", "status"],
                "usage",
            )
        },
        Failing {
            raw: true,
            ..failing("--format without a value", &["status", "--format"], "usage")
        },
        // Arguments with unacceptable values.
        failing("malformed id", &["status", "nope"], "invalid_argument"),
        failing("id too short", &["status", "c1-abc"], "invalid_argument"),
        failing("id in upper case", &["mark", "C1-ABCD"], "invalid_argument"),
        failing(
            "id with a letter outside the alphabet",
            &["retire", "c1-iiii"],
            "invalid_argument",
        ),
        failing("unknown schema", &["schema", "nope"], "invalid_argument"),
        failing(
            "--replaced-by one of the retired",
            &["retire", &live, "--replaced-by", &live],
            "invalid_argument",
        ),
        failing(
            "unknown claim id",
            &["status", "c1-zzzz"],
            "claim_not_found",
        ),
        failing(
            "unknown claim id in mark",
            &["mark", "c1-zzzzzzzz"],
            "claim_not_found",
        ),
        failing(
            "unknown claim id in list",
            &["list", "c1-zzzz"],
            "claim_not_found",
        ),
        failing(
            "bad --now",
            &["--now", "yesterday", "status"],
            "invalid_time",
        ),
        failing(
            "bad --since",
            &["status", "--since", "main..other"],
            "invalid_range",
        ),
        failing(
            "bad --since in cover",
            &["cover", "--since", "nope"],
            "invalid_range",
        ),
        failing(
            "unknown scope",
            &["cover", "--scope", "nope"],
            "unknown_scope",
        ),
        failing("init twice", &["init"], "already_initialized"),
        failing(
            "an anchor that does not parse",
            &["claim", "x", "--subject", "no-scheme"],
            "invalid_anchor",
        ),
        failing(
            "a bad duration",
            &[
                "claim",
                "x",
                "--subject",
                "file:docs/a.md::Alpha is one.",
                "--recheck",
                "soon",
            ],
            "invalid_duration",
        ),
        failing(
            "an undeclared scheme",
            &["claim", "x", "--subject", "zzz:thing"],
            "unknown_scheme",
        ),
        failing(
            "a resource that does not exist",
            &["claim", "x", "--subject", "file:docs/absent.md::Nothing."],
            "side_missing",
        ),
        failing(
            "a quote that is not there",
            &[
                "claim",
                "x",
                "--subject",
                "file:docs/a.md::Omega is last and nowhere.",
            ],
            "side_missing",
        ),
        failing(
            "a quote in a comment",
            &[
                "claim",
                "x",
                "--subject",
                "file:src/a.rs::A comment about first.",
            ],
            "quote_in_comment",
        ),
        failing(
            "a quote across two sentences",
            &["claim", "x", "--subject", "file:docs/a.md::one. Beta"],
            "quote_spans_units",
        ),
        failing(
            "mark on a retired claim",
            &["mark", &retired],
            "claim_retired",
        ),
        failing(
            "reanchor on a retired claim",
            &["reanchor", &retired, "--statement", "other"],
            "claim_retired",
        ),
        failing(
            "reanchor that changes nothing",
            &["reanchor", &live],
            "no_change",
        ),
        failing_stdin(
            "claim --stdin that is not JSON",
            &["claim", "--stdin"],
            "not json",
            "invalid_json",
        ),
        failing_stdin(
            "claim --stdin with an empty array",
            &["claim", "--stdin"],
            "[]",
            "invalid_json",
        ),
        failing_stdin(
            "claim --stdin with an object",
            &["claim", "--stdin"],
            "{}",
            "invalid_json",
        ),
        failing_stdin(
            "mark --stdin that is not JSON",
            &["mark", "--stdin"],
            "[",
            "invalid_json",
        ),
        failing_stdin(
            "claim --stdin with a key selector on a file",
            &["claim", "--stdin"],
            r#"[{"statement":"x","subject":{"loc":"file:docs/a.md","sel":{"key":"k"}}}]"#,
            "unsupported_selector",
        ),
        failing("judge without a key", &["triage"], "judge_unavailable"),
    ];
    let judge_order = failing(
        "status --judge-order without a key",
        &["status", "--judge-order"],
        "judge_unavailable",
    );
    table.push(judge_order);

    let run = |target: &Repo, case: &Failing, format: Option<&str>| -> Out {
        let mut cmd = target.hibi();
        if let Some(format) = format {
            cmd = cmd.args(["--format", format]);
        }
        cmd = cmd.args(&case.args);
        if let Some(text) = &case.stdin {
            cmd = cmd.stdin(text);
        }
        cmd.run()
    };

    let before = repo.files();
    for case in &table {
        let what = case.what;
        // JSON. A raw case has no --format of its own: standard output is
        // not a terminal, so the default is json.
        let out = run(&repo, case, if case.raw { None } else { Some("json") });
        assert_eq!(out.status, 1, "{what}: {out:?}");
        assert_eq!(out.stderr, "", "{what}");
        let v = out.json();
        assert_eq!(v["ok"], false, "{what}: {v}");
        assert!(v["command"].is_string(), "{what}: {v}");
        assert!(
            v.get("warnings").is_none() || v["warnings"].is_array(),
            "{what}"
        );
        let errors = out.errs();
        for e in &errors {
            let code = e["code"].as_str().unwrap();
            assert!(
                CODES.contains(&code),
                "{what}: {code} is not in the closed list"
            );
            assert!(
                e["remediation"].is_string() || e["remediation"].is_null(),
                "{what}"
            );
            assert!(!e["message"].as_str().unwrap().is_empty(), "{what}");
        }
        assert_eq!(errors[0]["code"], case.code, "{what}: {}", errors[0]);

        // Human: nothing on standard output, the error on standard error.
        if !case.raw {
            let out = run(&repo, case, Some("human"));
            assert_eq!(out.status, 1, "{what}: {out:?}");
            assert_eq!(out.stdout, "", "{what}");
            assert!(out.stderr.contains(case.code), "{what}: {out:?}");
        }
    }
    assert_eq!(repo.files(), before, "no failing run changed a file");

    // Not initialised: every command that needs a root.
    let before = uninitialised.files();
    for args in [
        vec!["status"],
        vec!["list"],
        vec!["cover"],
        vec!["providers"],
        vec!["mark", "c1-zzzz"],
        vec!["retire", "c1-zzzz"],
        vec!["reanchor", "c1-zzzz"],
        vec!["claim", "x", "--subject", "file:docs/a.md"],
    ] {
        let out = uninitialised.run(&args);
        assert_eq!(out.code(), "not_initialized", "{args:?}: {out:?}");
        assert_eq!(out.err()["remediation"], "hibi init");
        let out = uninitialised.human(&args);
        assert_eq!(out.status, 1, "{args:?}: {out:?}");
        assert_eq!(out.stdout, "");
        assert!(out.stderr.contains("not_initialized"), "{out:?}");
    }
    assert_eq!(uninitialised.files(), before);

    // `init` outside a git work tree.
    let outside = tempfile::tempdir().unwrap();
    let out = uninitialised
        .hibi()
        .cwd(outside.path())
        .args(["--format", "json", "init"])
        .run();
    assert_eq!(out.code(), "not_a_git_repository", "{out:?}");
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());

    // A config that does not parse, and a store version that is not 1.
    let config = repo.read(".hibi/hibi.toml");
    repo.write(".hibi/hibi.toml", "version = 1\nunknown_key = true\n");
    assert_eq!(repo.run(&["status"]).code(), "invalid_config");
    repo.write(".hibi/hibi.toml", "version = 2\n");
    assert_eq!(repo.run(&["status"]).code(), "unsupported_store_version");
    repo.write(".hibi/hibi.toml", &config);
    repo.user_config("unknown_key = true\n");
    assert_eq!(repo.run(&["status"]).code(), "invalid_user_config");
    std::fs::remove_file(repo.user_config_path()).unwrap();
    assert_eq!(repo.state(&live), "fresh");
}

/// R34 and section 3.6: a prefix that more than one claim id starts with is
/// `ambiguous_id`, in every command that takes an id, and the message lists
/// the candidates.
#[test]
fn r34_an_ambiguous_id_prefix_is_ambiguous_id() {
    let repo = Repo::new();
    repo.write("docs/many.md", "Many claims.\n");
    let (prefix, with_prefix) = claims_with_a_shared_prefix(&repo);
    assert!(with_prefix.len() >= 2);
    assert_eq!(prefix.len(), 7);
    let before = repo.files();

    for args in [
        vec!["status", prefix.as_str()],
        vec!["mark", prefix.as_str()],
        vec!["list", prefix.as_str()],
        vec!["retire", prefix.as_str()],
        vec!["reanchor", prefix.as_str(), "--statement", "other"],
    ] {
        let out = repo.run(&args);
        assert_eq!(out.status, 1, "{args:?}");
        let errors = out.errs();
        assert_eq!(errors.len(), 1, "{args:?}");
        let e = &errors[0];
        assert_eq!(e["code"], "ambiguous_id", "{args:?}: {e}");
        for id in &with_prefix {
            assert!(e["message"].as_str().unwrap().contains(id.as_str()), "{e}");
        }
        let out = repo.human(&args);
        assert_eq!(out.status, 1, "{args:?}: {out:?}");
        assert_eq!(out.stdout, "");
        assert!(out.stderr.contains("ambiguous_id"), "{out:?}");
    }
    assert_eq!(repo.files(), before);

    // A longer prefix of one of them resolves; so does the full id.
    assert_eq!(
        repo.view(&with_prefix[0][..12])["id"],
        with_prefix[0].as_str()
    );
    assert_eq!(repo.view(&with_prefix[1])["id"], with_prefix[1].as_str());
}

/// Section 15.1: `--help` and `-h` print help and exit 0, for the program
/// and for every command.
#[test]
fn r34_help_exits_0() {
    let repo = Repo::bare();
    let commands = [
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
    let mut invocations: Vec<Vec<&str>> = vec![vec!["--help"], vec!["-h"]];
    for c in commands {
        invocations.push(vec![c, "--help"]);
        invocations.push(vec![c, "-h"]);
    }
    for args in &invocations {
        for format in [None, Some("json"), Some("human")] {
            let mut cmd = repo.hibi().cwd(&repo.home);
            if let Some(format) = format {
                cmd = cmd.args(["--format", format]);
            }
            let out = cmd.args(args).run();
            assert_eq!(out.status, 0, "{args:?}: {out:?}");
            assert_eq!(out.stderr, "", "{args:?}");
            assert!(out.stdout.contains("Usage:"), "{args:?}: {}", out.stdout);
            assert!(out.stdout.contains("hibi"), "{args:?}");
        }
    }
    // The program help names every command.
    let out = repo.hibi().cwd(&repo.home).args(["--help"]).run();
    for c in commands {
        assert!(out.stdout.contains(c), "{c}: {}", out.stdout);
    }
}
