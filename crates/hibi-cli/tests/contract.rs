//! The order of the checks of a judge command (spec section 14.1), the
//! output schemas against real output (sections 15.1 and 15.3), a panic in
//! JSON format (section 15.1), scope kinds (sections 10.1 and 11.1) and the
//! largest `--now`.

mod support;

use std::path::Path;

use serde_json::{Value, json};
use support::*;

// ---------------------------------------------------------------------------
// Judge commands: the key, then git, then everything else
// ---------------------------------------------------------------------------

const JUDGE_COMMANDS: [&[&str]; 3] = [
    &["triage"],
    &["suggest-evidence", "x", "--in", "file:docs/**"],
    &["status", "--judge-order"],
];

/// A judge command with the key set and an endpoint nothing listens on.
fn judge(repo: &Repo, args: &[&str]) -> Cmd {
    repo.hibi()
        .env("TYPESAFE_API_KEY", "k")
        .env("HIBI_JEV_URL", "http://127.0.0.1:9")
        .args(["--format", "json"])
        .args(args)
}

/// A store whose `hibi.toml` and user config do not parse.
fn broken_configuration() -> Repo {
    let repo = Repo::new();
    std::fs::write(repo.root.join(".hibi/hibi.toml"), "this is not = [toml\n").unwrap();
    repo.user_config("neither [ is this\n");
    repo
}

/// Where the root is not a git repository, a judge command says so before
/// it reads `hibi.toml` or the user config.
#[test]
fn a_judge_command_outside_git_is_not_a_git_repository_before_the_configuration_is_read() {
    let repo = broken_configuration();
    std::fs::remove_dir_all(repo.root.join(".git")).unwrap();
    for args in JUDGE_COMMANDS {
        let out = judge(&repo, args).run();
        assert_eq!(out.code(), "not_a_git_repository", "{args:?}: {out:?}");
        assert_eq!(out.json()["command"], args[0]);
        // The key still comes first.
        let out = repo.run(args);
        assert_eq!(out.code(), "judge_unavailable", "{args:?}");
    }
    // A command that does not ask the judge reads the configuration.
    assert_eq!(repo.run(&["status"]).code(), "invalid_config");
}

/// Without git on `PATH` the answer is the same.
#[test]
fn a_judge_command_without_git_is_not_a_git_repository_before_the_configuration_is_read() {
    let repo = broken_configuration();
    let empty = Bin::new(&repo.home, "empty");
    for args in JUDGE_COMMANDS {
        let out = judge(&repo, args).env("PATH", empty.only()).run();
        assert_eq!(out.code(), "not_a_git_repository", "{args:?}: {out:?}");
    }
}

/// With a git older than the minimum first on `PATH`, a judge command is
/// `git_too_old` before it reads `hibi.toml` or the user config.
#[test]
fn a_judge_command_with_an_old_git_is_git_too_old_before_the_configuration_is_read() {
    let repo = broken_configuration();
    let old = Bin::new(&repo.home, "old-git");
    old.fake_git("git version 2.30.0");
    for args in JUDGE_COMMANDS {
        let out = judge(&repo, args).env("PATH", old.only()).run();
        assert_eq!(out.code(), "git_too_old", "{args:?}: {out:?}");
    }
}

/// After the git checks the configuration is read as before, and a legacy
/// store is still found during root discovery, before git is asked.
#[test]
fn a_judge_command_in_git_still_reads_the_configuration_and_refuses_a_legacy_store() {
    let repo = broken_configuration();
    for args in JUDGE_COMMANDS {
        assert_eq!(
            judge(&repo, args).run().code(),
            "invalid_config",
            "{args:?}"
        );
    }
    let repo = Repo::new();
    repo.user_config("neither [ is this\n");
    for args in JUDGE_COMMANDS {
        assert_eq!(
            judge(&repo, args).run().code(),
            "invalid_user_config",
            "{args:?}"
        );
    }
    let legacy = Repo::new();
    std::fs::remove_dir_all(legacy.root.join(".git")).unwrap();
    std::fs::create_dir(legacy.root.join(".claims")).unwrap();
    for args in JUDGE_COMMANDS {
        assert_eq!(
            judge(&legacy, args).run().code(),
            "legacy_store",
            "{args:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The output schemas describe what the commands print
// ---------------------------------------------------------------------------

fn schema_of(repo: &Repo, name: &str) -> Value {
    let out = repo.hibi().args(["schema", name]).run();
    assert_eq!(out.status, 0, "schema {name}: {out:?}");
    serde_json::from_str(&out.stdout).expect("a schema is JSON")
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Why `value` does not have the shape of the closed object schema
/// `shape`: an undeclared key, a missing required key, or a `const` that
/// differs. This is the part of JSON Schema validation that the envelope
/// contract of section 15.1 needs; nested values are not followed.
fn mismatch(shape: &Value, value: &Value) -> Option<String> {
    if shape["type"] != "object" {
        return Some("the shape is not an object schema".to_owned());
    }
    if shape["additionalProperties"] != false {
        return Some("the shape does not say additionalProperties: false".to_owned());
    }
    let declared = keys(&shape["properties"]);
    for key in keys(value) {
        if !declared.contains(&key) {
            return Some(format!("key {key:?} is not declared"));
        }
        if let Some(constant) = shape["properties"][&key].get("const")
            && *constant != value[&key]
        {
            return Some(format!("{key} is {}, not {constant}", value[&key]));
        }
    }
    strings(&shape["required"])
        .into_iter()
        .find(|key| value.get(key).is_none())
        .map(|key| format!("required key {key:?} is absent"))
}

/// The closed shapes of an envelope schema: the schema itself, or each
/// entry of its `anyOf` when the command has several result shapes.
fn shapes(schema: &Value) -> Vec<Value> {
    match schema.get("anyOf").and_then(Value::as_array) {
        Some(all) => all.clone(),
        None => vec![schema.clone()],
    }
}

/// Assert that `output` has exactly one of the shapes of schema `name`,
/// and return the position of that shape.
fn assert_described(repo: &Repo, name: &str, output: &Value) -> usize {
    let schema = schema_of(repo, name);
    let shapes = shapes(&schema);
    let results: Vec<Option<String>> = shapes.iter().map(|s| mismatch(s, output)).collect();
    let matching: Vec<usize> = (0..shapes.len())
        .filter(|i| results[*i].is_none())
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "schema {name} does not describe {output}: {results:?}"
    );
    let shape = &shapes[matching[0]];
    let ok = &shape["properties"]["ok"];
    assert_eq!(ok["const"], output["ok"], "{name}: ok is a const");
    assert!(
        strings(&shape["required"]).contains(&"ok".to_owned())
            && strings(&shape["required"]).contains(&"command".to_owned()),
        "{name}: ok and command are required"
    );
    matching[0]
}

fn register_reference(repo: &Repo) {
    repo.config("[schemes.ref]\n");
    repo.user_config(&format!(
        "[providers.ref]\ncommand = [{:?}, \"--fixture\", {:?}]\n",
        reference_provider().to_string_lossy(),
        notes_fixture().to_string_lossy()
    ));
}

/// The real JSON output of every command that needs no judge has the shape
/// its `<command>-output` schema states: every top-level key is declared,
/// every required key is present, the object is closed and `ok` is the
/// constant `true`.
#[test]
fn every_output_schema_describes_the_real_output_of_its_command() {
    let bare = Repo::bare();
    let init = bare.json(&["init"]);
    assert_described(&bare, "init-output", &init);

    let repo = Repo::new();
    register_reference(&repo);
    repo.config(
        "[[scope]]\nname = \"docs\"\nresource = \"file:docs/**/*.md\"\nkinds = [\"sentence\"]\n",
    );
    repo.write("docs/a.md", "One sentence. Another sentence.\n");
    repo.commit("base");

    let claim = repo.json(&[
        "--now",
        NOW,
        "claim",
        "One.",
        "--subject",
        "file:docs/a.md::One sentence.",
    ]);
    assert_described(&repo, "claim-output", &claim);
    let id = claim["claims"][0]["id"].as_str().unwrap().to_owned();

    for args in [vec!["status"], vec!["status", &id, "--detail"]] {
        assert_described(&repo, "status-output", &repo.json(&args));
    }
    assert_described(&repo, "mark-output", &repo.json(&["mark", &id]));
    assert_described(&repo, "cover-output", &repo.json(&["cover"]));

    // `list` has two shapes: the claims, and one claim with its history.
    let claims = assert_described(&repo, "list-output", &repo.json(&["list"]));
    let record = assert_described(&repo, "list-output", &repo.json(&["list", &id]));
    assert_ne!(claims, record, "the two shapes of list are told apart");
    let list = schema_of(&repo, "list-output");
    let mut required: Vec<Vec<String>> = shapes(&list)
        .iter()
        .map(|s| {
            let mut r = strings(&s["required"]);
            r.sort();
            r
        })
        .collect();
    required.sort();
    assert_eq!(
        required,
        [
            vec!["claim", "command", "marks", "ok", "retirements", "warnings"],
            vec!["claims", "command", "ok", "warnings"],
        ]
    );

    let reanchor = repo.json(&["reanchor", &id, "--statement", "One, reworded."]);
    assert_described(&repo, "reanchor-output", &reanchor);
    let new = reanchor["new"].as_str().unwrap().to_owned();
    assert_described(&repo, "retire-output", &repo.json(&["retire", &new]));

    let providers = repo.json(&["providers"]);
    assert!(providers.get("check").is_none());
    assert_described(&repo, "providers-output", &providers);
    let checked = repo.json(&["providers", "--check", "ref"]);
    assert_described(&repo, "providers-output", &checked);
    // `check.hello` is described: every key the provider sent is declared.
    let schema = schema_of(&repo, "providers-output");
    let hello = &schema["$defs"]["ProviderCheck"]["properties"]["hello"];
    let described = hello["anyOf"]
        .as_array()
        .expect("hello lists its forms")
        .iter()
        .find_map(|form| form["$ref"].as_str())
        .expect("hello refers to the hello result");
    let info = schema
        .pointer(described.trim_start_matches('#'))
        .expect("the reference resolves");
    let declared = keys(&info["properties"]);
    let sent = keys(&checked["check"]["hello"]);
    assert!(!sent.is_empty(), "{checked}");
    for key in &sent {
        assert!(declared.contains(key), "hello key {key} is not declared");
    }
    for key in strings(&info["required"]) {
        assert!(sent.contains(&key), "hello key {key} is absent");
    }

    assert_described(&repo, "schema-output", &repo.json(&["schema"]));
    assert_described(&repo, "version-output", &repo.json(&["version"]));
}

/// The `error` schema describes a real failure: `ok` is the constant
/// `false`, the envelope is closed, `errors` has at least one entry, and
/// `remediation` is required and may be `null`.
#[cfg(debug_assertions)]
#[test]
fn the_error_schema_describes_a_real_failure() {
    let repo = Repo::new();
    let schema = schema_of(&repo, "error");
    assert_eq!(schema["properties"]["errors"]["minItems"], 1);
    let error = &schema["$defs"]["Error"];
    assert!(strings(&error["required"]).contains(&"remediation".to_owned()));
    assert_eq!(
        error["properties"]["remediation"]["type"],
        json!(["string", "null"])
    );
    // One failure with a remediation, one without, one with an index.
    let with = repo.run(&["list", "c1-0000"]);
    let without = repo
        .hibi()
        .env("HIBI_TEST_PANIC", "1")
        .args(["--format", "json", "list"])
        .run();
    let indexed = repo
        .hibi()
        .args(["--format", "json", "mark", "--stdin"])
        .stdin("[7]")
        .run();
    let mut remediations = Vec::new();
    for out in [with, without, indexed] {
        let v = out.json();
        assert_eq!(out.status, 1);
        assert_described(&repo, "error", &v);
        for e in v["errors"].as_array().unwrap() {
            assert_eq!(mismatch(error, e), None, "{e}");
            remediations.push(e["remediation"].is_null());
        }
    }
    assert!(remediations.contains(&true) && remediations.contains(&false));
}

/// Schema titles are names a reader recognises, not type names.
#[test]
fn schema_titles_are_readable_names() {
    let repo = Repo::bare();
    for (name, title) in [
        ("config", "hibi.toml"),
        ("user-config", "hibi user config"),
        ("error", "hibi error output"),
        ("status-output", "hibi status output"),
        ("suggest-evidence-output", "hibi suggest-evidence output"),
        ("claim-input", "hibi claim --stdin input"),
        ("mark-input", "hibi mark --stdin input"),
    ] {
        assert_eq!(schema_of(&repo, name)["title"], title, "{name}");
    }
    for name in hibi_cli::schema::NAMES {
        let title = schema_of(&repo, name)["title"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} has no title"))
            .to_owned();
        assert!(
            title.starts_with("hibi") && !title.contains("Raw") && !title.contains("Envelope"),
            "{name}: {title}"
        );
    }
}

// ---------------------------------------------------------------------------
// A panic
// ---------------------------------------------------------------------------

/// A panic in JSON format is one `internal` error on standard output with
/// exit status 1, and nothing on standard error.
#[cfg(debug_assertions)]
#[test]
fn a_panic_in_json_format_is_one_internal_error_and_standard_error_is_empty() {
    let repo = Repo::new();
    let out = repo
        .hibi()
        .env("HIBI_TEST_PANIC", "1")
        .args(["--format", "json", "status"])
        .run();
    assert_eq!(out.status, 1, "{out:?}");
    assert_eq!(out.stderr, "", "{out:?}");
    let v = out.json();
    assert_eq!(v["ok"], false);
    assert_eq!(v["command"], "status");
    let errors = v["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{v}");
    assert_eq!(errors[0]["code"], "internal");
    let message = errors[0]["message"].as_str().unwrap();
    // The text of the panic and where it happened.
    assert!(message.contains("HIBI_TEST_PANIC"), "{message}");
    assert!(message.contains("src/lib.rs:"), "{message}");
    assert_described(&repo, "error", &v);
}

/// In human format the same panic is one error in the usual form on
/// standard error, without the panic message of the standard library.
#[cfg(debug_assertions)]
#[test]
fn a_panic_in_human_format_is_one_error_line_on_standard_error() {
    let repo = Repo::new();
    let out = repo
        .hibi()
        .env("HIBI_TEST_PANIC", "1")
        .args(["--format", "human", "status"])
        .run();
    assert_eq!(out.status, 1, "{out:?}");
    assert_eq!(out.stdout, "");
    assert!(out.stderr.starts_with("error: internal: "), "{out:?}");
    assert!(out.stderr.contains("HIBI_TEST_PANIC"), "{out:?}");
    assert!(!out.stderr.contains("panicked at"), "{out:?}");
    assert!(!out.stderr.contains("RUST_BACKTRACE"), "{out:?}");
}

/// The trigger exists in debug builds only, which is what `cargo test`
/// builds. This test is compiled when the tests are built without debug
/// assertions (`cargo test --release`), where the binary is a release
/// build: the variable does nothing there.
#[cfg(not(debug_assertions))]
#[test]
fn a_release_build_ignores_the_panic_trigger() {
    let repo = Repo::new();
    let out = repo
        .hibi()
        .env("HIBI_TEST_PANIC", "1")
        .args(["--format", "json", "status"])
        .run();
    out.ok();
}

/// The trigger is the exact value `1`, and no help text names it.
#[test]
fn the_panic_trigger_is_hidden_and_needs_its_exact_value() {
    let repo = Repo::new();
    for value in ["", "0", "true", "11"] {
        let out = repo
            .hibi()
            .env("HIBI_TEST_PANIC", value)
            .args(["--format", "json", "status"])
            .run();
        out.ok();
    }
    let help = repo.hibi().args(["--help"]).run();
    assert_eq!(help.status, 0);
    assert!(!help.stdout.contains("HIBI_TEST_PANIC"));
    let reference = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/cli-reference.mdx");
    let reference = std::fs::read_to_string(reference).unwrap();
    assert!(!reference.contains("HIBI_TEST_PANIC"));
}

// ---------------------------------------------------------------------------
// --no-cache and --now help, and the largest --now
// ---------------------------------------------------------------------------

/// Help text with every run of white space as one space, so that line
/// wrapping does not matter.
fn help_text(repo: &Repo) -> String {
    let out = repo.hibi().args(["--help"]).run();
    assert_eq!(out.status, 0, "{out:?}");
    out.stdout.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `--no-cache` does not promise the same result for the judge commands:
/// for them the cache is what makes a second run repeat the first.
#[test]
fn the_no_cache_help_does_not_promise_repeatable_judge_results() {
    let repo = Repo::bare();
    let help = help_text(&repo);
    assert!(
        !help.contains("Results are the same with and without the cache"),
        "{help}"
    );
    assert!(
        help.contains(
            "Do not read or write the cache database. Every command that does not call the judge gives the same result with and without the cache. The judge commands (triage, suggest-evidence, status --judge-order) may rank differently from one run to the next without it"
        ),
        "{help}"
    );
}

/// The largest `--now` is 9999-12-30T22:00:00Z. The help says so, and one
/// second later is `invalid_time` with a message that says so.
#[test]
fn the_largest_now_is_accepted_and_stated() {
    let repo = Repo::new();
    let largest = "9999-12-30T22:00:00Z";
    let v = repo.json(&["--now", largest, "status"]);
    assert_eq!(v["now"], largest);
    let v = repo.json(&["--now", "9999-12-31T00:00:00.9+02:00", "status"]);
    assert_eq!(v["now"], largest);
    for late in [
        "9999-12-30T22:00:01Z",
        "9999-12-31T00:00:01+02:00",
        "9999-12-31T23:59:59Z",
    ] {
        let e = repo.fail(&["--now", late, "status"]);
        assert_eq!(e["code"], "invalid_time", "{late}");
        assert_eq!(
            e["message"],
            format!("{late:?} is later than the largest time hibi accepts, {largest}"),
        );
        assert_eq!(
            e["remediation"],
            format!("Give a time from 0000-01-01T00:00:00Z to {largest}.").as_str(),
            "{late}"
        );
    }
    // A value that is not a time keeps the remediation about its form.
    let e = repo.fail(&["--now", "9999-12-31", "status"]);
    assert_eq!(e["code"], "invalid_time");
    assert!(
        e["remediation"].as_str().unwrap().contains("RFC 3339"),
        "{e}"
    );
    assert!(
        help_text(&repo).contains(&format!("The largest value is {largest}")),
        "{}",
        help_text(&repo)
    );
}

// ---------------------------------------------------------------------------
// Scope kinds
// ---------------------------------------------------------------------------

/// A scope on `file` or `repo` with a kind the file provider never returns
/// is `invalid_config`, and the message names the scope, the kind and the
/// kinds that exist.
#[test]
fn a_mistyped_kind_of_a_file_scope_is_invalid_config() {
    for resource in ["file:docs/**/*.md", "repo:api//docs/**/*.md"] {
        let repo = Repo::new();
        repo.config(&format!(
            "[repos]\napi = \"../api\"\n\n[[scope]]\nname = \"docs\"\nresource = \"{resource}\"\nkinds = [\"sentence\", \"sentense\"]\n"
        ));
        repo.write("docs/a.md", "One sentence.\n");
        for args in [vec!["cover"], vec!["status"], vec!["list"]] {
            let e = repo.fail(&args);
            assert_eq!(e["code"], "invalid_config", "{resource} {args:?}");
            let message = e["message"].as_str().unwrap();
            assert!(message.contains("scope docs"), "{message}");
            assert!(message.contains("\"sentense\""), "{message}");
            assert!(
                message.contains("sentence, heading, list-item"),
                "{message}"
            );
        }
    }
    // The store that `init` writes, and its example scope once the comment
    // marks are removed, are accepted.
    let repo = Repo::new();
    assert_eq!(repo.run(&["cover"]).code(), "no_scopes");
    let template = repo.read(".hibi/hibi.toml");
    let example: String = template
        .lines()
        .skip_while(|l| *l != "# [[scope]]")
        .map(|l| format!("{}\n", l.trim_start_matches("# ")))
        .collect();
    assert!(example.contains("kinds = ["), "{template}");
    repo.config(&example);
    repo.json(&["cover"]);
}

/// A scope on an external scheme may name any kind. When `cover` starts
/// the provider, a kind that its `hello` does not list is reported as an
/// `unknown_kind` warning, and the units of the other kinds are counted.
#[test]
fn a_kind_the_external_provider_does_not_list_is_a_warning_of_cover() {
    let repo = Repo::new();
    register_reference(&repo);
    repo.config(
        "[[scope]]\nname = \"notes\"\nresource = \"ref:notes\"\nkinds = [\"note\", \"nite\"]\n\n[[scope]]\nname = \"tasks\"\nresource = \"ref:notes\"\nkinds = [\"task\"]\n",
    );
    let v = repo.json(&["cover"]);
    assert_eq!(
        v["warnings"],
        json!([{
            "code": "unknown_kind",
            "message": "scope notes: the ref provider lists no unit kind \"nite\", so it selects nothing; its kinds are note, task"
        }])
    );
    assert!(v["scopes"][0]["units"].as_u64().unwrap() > 0, "{v}");
    // A scope whose kinds are all listed gives no warning.
    let v = repo.json(&["cover", "--scope", "tasks"]);
    assert_eq!(v["warnings"], json!([]));
    // In human format the warning is a line on standard error.
    let out = repo.human(&["cover", "--scope", "notes"]);
    assert_eq!(out.status, 0);
    assert!(
        out.stderr
            .starts_with("warning: unknown_kind: scope notes: "),
        "{out:?}"
    );
}

/// A `.hibi` that is a file is not a store. Root discovery looks for
/// `.hibi/hibi.toml` (section 12.5) and finds none, so the command line
/// answers `not_initialized`, with and without `--root`. (The library's
/// `RepoConfig::load`, called on such a root directly, answers
/// `store_corrupt` by section 12.3.)
#[test]
fn a_hibi_that_is_a_file_is_not_initialized_on_the_command_line() {
    let repo = Repo::bare();
    repo.write(".hibi", "not a directory\n");
    for args in [vec!["status"], vec!["list"], vec!["cover"]] {
        let e = repo.fail(&args);
        assert_eq!(e["code"], "not_initialized", "{args:?}");
        assert_eq!(e["remediation"], "hibi init", "{args:?}");
    }
    let root = repo.root.to_str().unwrap().to_owned();
    let e = repo.fail(&["--root", &root, "status"]);
    assert_eq!(e["code"], "not_initialized");
    // `init` refuses to write through it.
    let e = repo.fail(&["init"]);
    assert_eq!(e["code"], "store_corrupt", "{e}");
    assert_eq!(repo.read(".hibi"), "not a directory\n");
}

/// The provider response schema accepts any JSON value as `result`, as the
/// reader does (section 13.2): `batch` and a provider's own methods are not
/// bound to an object.
#[test]
fn the_provider_response_schema_accepts_any_result() {
    let schema = hibi_cli::schema::schema("provider-response").unwrap();
    let v = serde_json::to_value(schema).unwrap();
    assert_eq!(v["oneOf"][0]["properties"]["result"], json!(true), "{v}");
    assert_eq!(v["oneOf"][0]["required"], json!(["id", "result"]));
    for line in [
        r#"{"id":1,"result":{}}"#,
        r#"{"id":1,"result":[1,2]}"#,
        r#"{"id":1,"result":"text"}"#,
        r#"{"id":1,"result":7}"#,
        r#"{"id":1,"result":null}"#,
    ] {
        let parsed: Result<hibi::provider::Response, _> = serde_json::from_str(line);
        assert!(parsed.is_ok(), "{line}: {:?}", parsed.err());
    }
}
