//! External schemes and provider commands through the binary: spec sections
//! 9.2, 10.2, 11.1, 11.2, 13.1, 13.5 and 15.3; requirements R28, R29 (V7),
//! R56 and R62.

mod support;

use std::path::Path;

use serde_json::{Value, json};
use support::*;

const DOC: &str = "Alpha is one. Beta is two.\n";
const SCHEME: &str = "[schemes.ref]\ndescription = \"Notes served by the reference provider\"\n";
const NOTES_SCOPE: &str =
    "[[scope]]\nname = \"notes\"\nresource = \"ref:notes\"\nkinds = [\"note\"]\n";

fn toml_array(command: &[&str]) -> String {
    let parts: Vec<String> = command.iter().map(|c| format!("{c:?}")).collect();
    format!("[{}]", parts.join(", "))
}

/// Register `command` for scheme `ref` in the user config.
fn register(repo: &Repo, command: &[&str]) {
    repo.user_config(&format!(
        "[providers.ref]\ncommand = {}\n",
        toml_array(command)
    ));
}

/// Register the reference provider serving this fixture.
fn register_reference(repo: &Repo, fixture: &Path) -> Vec<String> {
    let command = vec![
        reference_provider().to_str().unwrap().to_owned(),
        "--fixture".to_owned(),
        fixture.to_str().unwrap().to_owned(),
    ];
    let refs: Vec<&str> = command.iter().map(String::as_str).collect();
    register(repo, &refs);
    command
}

fn unregister(repo: &Repo) {
    std::fs::remove_file(repo.user_config_path()).unwrap();
}

/// `hibi --now NOW claim --stdin` with this document.
fn claim_stdin(repo: &Repo, input: &Value) -> Out {
    repo.hibi()
        .args(["--format", "json", "--now", NOW, "claim", "--stdin"])
        .stdin(&input.to_string())
        .run()
}

/// A claim whose subject is note `key` of `ref:notes` and whose evidence is
/// the first sentence of `docs/a.md`. Returns its id.
fn claim_note(repo: &Repo, key: &str) -> String {
    let out = claim_stdin(
        repo,
        &json!([{
            "statement": format!("note {key} holds"),
            "subject": {"loc": "ref:notes", "sel": {"key": key}},
            "evidence": ["file:docs/a.md::Alpha is one."],
        }]),
    );
    let v = out.ok();
    assert_eq!(v["claims"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["claims"][0]["created"], true);
    v["claims"][0]["id"].as_str().unwrap().to_owned()
}

/// A repository that lists scheme `ref`, with the reference provider
/// registered and one document.
fn repo_with_ref() -> Repo {
    let repo = Repo::new();
    repo.config(SCHEME);
    repo.write("docs/a.md", DOC);
    register_reference(&repo, &notes_fixture());
    repo
}

/// A `PATH` directory holding a recording `git` and recording stand-ins for
/// provider commands. Nothing else can be started by name.
fn recording_path(repo: &Repo, programs: &[&str]) -> Bin {
    let bin = Bin::new(&repo.home, "bin");
    bin.recording_git();
    for p in programs {
        bin.recording(p);
    }
    bin
}

fn assert_only_git(calls: &[String]) {
    for line in calls {
        assert!(line.starts_with("git "), "a non-git process ran: {calls:?}");
    }
}

fn ids(v: &Value) -> Vec<String> {
    v["claims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
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

/// R28 (V7): a scheme listed in `hibi.toml` with no user registration. A side
/// on it is `unavailable` with `provider_not_registered`, `next` is
/// `hibi providers`, new writes on it fail with `side_unavailable`, and no
/// child process other than git is started.
#[test]
fn r28_a_listed_scheme_without_registration_is_unavailable_and_starts_nothing() {
    let repo = repo_with_ref();
    repo.config(NOTES_SCOPE);
    let id = claim_note(&repo, "n1");
    assert_eq!(repo.state(&id), "fresh");
    repo.commit("base");
    unregister(&repo);

    // Stand-ins for anything hibi could try to start for the scheme by name.
    let bin = recording_path(
        &repo,
        &["hibi-provider-reference", "ref", "hibi-provider-ref"],
    );
    let run = |args: &[&str]| {
        repo.hibi()
            .env("PATH", bin.only())
            .args(["--format", "json"])
            .args(args)
            .run()
    };

    let v = run(&["status"]).ok();
    assert_eq!(ids(&v), vec![id.clone()]);
    assert_eq!(
        v["summary"],
        json!({"claims": 1, "fresh": 0, "drifted": 0, "stale": 1, "expired": 0, "retired": 0})
    );
    let claim = &v["claims"][0];
    assert_eq!(claim["state"], "stale");
    assert_eq!(claim["next"], "hibi providers");
    let subject = &claim["sides"][0];
    assert_eq!(subject["role"], "subject");
    assert_eq!(
        subject["anchor"],
        json!({"loc": "ref:notes", "sel": {"key": "n1"}})
    );
    assert_eq!(subject["state"], "unavailable");
    assert_eq!(subject["reason"], "provider_not_registered");
    assert_eq!(subject["unit"], Value::Null);
    assert_eq!(subject["fp"], Value::Null);
    assert!(
        subject["markedFp"]
            .as_str()
            .unwrap()
            .starts_with("b3:prose-v1:")
    );
    let evidence = &claim["sides"][1];
    assert_eq!(evidence["anchor"]["loc"], "file:docs/a.md");
    assert_eq!(evidence["state"], "same");
    assert_eq!(evidence["reason"], Value::Null);

    // New writes on the scheme fail and write nothing.
    let before = repo.store_files();
    let out = run(&["claim", "the cap", "--subject", "ref:notes"]);
    let e = out.err();
    assert_eq!(e["code"], "side_unavailable", "{e}");
    assert!(
        e["message"]
            .as_str()
            .unwrap()
            .contains("provider_not_registered"),
        "the message gives the reason code: {e}"
    );
    let out = run(&[
        "claim",
        "alpha",
        "--subject",
        "file:docs/a.md::Beta is two.",
        "--evidence",
        "ref:notes",
    ]);
    assert_eq!(out.code(), "side_unavailable", "{out:?}");
    let out = run(&["mark", &id]);
    assert_eq!(out.code(), "side_unavailable", "{out:?}");
    assert_eq!(repo.store_files(), before);

    // Coverage: the resource cannot be listed and is reported, not dropped.
    let v = run(&["cover"]).ok();
    let scope = &v["scopes"][0];
    assert_eq!(scope["name"], "notes");
    assert_eq!(
        scope["unavailable"],
        json!([{"loc": "ref:notes", "reason": "provider_not_registered"}])
    );
    assert_eq!(counts(scope), (0, 0, 0, 0), "{v}");
    assert_eq!(scope["items"], json!([]));

    let v = run(&["providers"]).ok();
    assert_eq!(
        v["schemes"],
        json!([{"scheme": "ref", "registered": false, "command": null,
                "remediation": "[providers.ref]\ncommand = [\"<program>\", \"<argument>\"]"}])
    );
    run(&["list"]).ok();
    run(&["status", "--since", "HEAD"]).ok();

    // The recorder works (cover and --since need git) and saw git only.
    let calls = bin.calls();
    assert!(calls.iter().any(|c| c == "git --version"), "{calls:?}");
    assert!(calls.len() > 2, "{calls:?}");
    assert_only_git(&calls);
}

/// Section 15.3 claim step 1 and section 9.2: a scheme that `hibi.toml` does
/// not list is `unknown_scheme` for a new anchor, and `scheme_not_declared`
/// for a stored side whose scheme was removed from `hibi.toml`. A user
/// registration alone does not make a scheme usable, and its command is not
/// started.
#[test]
fn r28_a_scheme_that_is_not_listed_is_unknown() {
    let repo = repo_with_ref();
    let id = claim_note(&repo, "n1");

    let before = repo.store_files();
    let out = repo.run(&["claim", "x", "--subject", "zzz:notes"]);
    let e = out.err();
    assert_eq!(e["code"], "unknown_scheme", "{e}");
    assert!(e["message"].as_str().unwrap().contains("zzz"));
    let out = claim_stdin(
        &repo,
        &json!([{"statement": "x", "subject": "file:docs/a.md::Beta is two.",
                 "evidence": [{"loc": "zzz:notes", "sel": {"key": "n1"}}]}]),
    );
    let e = out.err();
    assert_eq!(e["code"], "unknown_scheme", "{e}");
    assert_eq!(e["index"], 0);
    assert_eq!(repo.store_files(), before);

    // Remove the scheme from hibi.toml; keep a user registration that points
    // at a recording stand-in.
    let bin = recording_path(&repo, &["refprov"]);
    let stand_in = bin.dir.join("refprov");
    register(&repo, &[stand_in.to_str().unwrap()]);
    repo.write(".hibi/hibi.toml", "version = 1\n");
    let run = |args: &[&str]| {
        repo.hibi()
            .env("PATH", bin.only())
            .args(["--format", "json"])
            .args(args)
            .run()
    };
    let v = run(&["status"]).ok();
    assert_eq!(ids(&v), vec![id.clone()]);
    let claim = &v["claims"][0];
    assert_eq!(claim["state"], "stale");
    assert_eq!(claim["sides"][0]["state"], "unavailable");
    assert_eq!(claim["sides"][0]["reason"], "scheme_not_declared");
    assert_eq!(claim["sides"][1]["state"], "same");
    assert_eq!(claim["next"], "hibi providers");

    let out = run(&["claim", "x", "--subject", "ref:notes"]);
    assert_eq!(out.code(), "unknown_scheme", "{out:?}");
    let v = run(&["providers"]).ok();
    assert_eq!(v["schemes"], json!([]));
    let out = run(&["providers", "--check", "ref"]);
    assert_eq!(out.code(), "invalid_argument", "{out:?}");
    assert_only_git(&bin.calls());
}

/// R29 (V7): a `command` key under a scheme table of `hibi.toml` is
/// `invalid_config` for every command that reads the store, and nothing is
/// run: neither the command the key names nor the registered one.
#[test]
fn r29_a_command_key_in_hibi_toml_is_invalid_config_and_runs_nothing() {
    let repo = repo_with_ref();
    repo.config(NOTES_SCOPE);
    let id = claim_note(&repo, "n1");
    repo.commit("base");
    let clean_config = repo.read(".hibi/hibi.toml");

    let bin = recording_path(&repo, &["evil", "registered"]);
    let evil = bin.dir.join("evil");
    let registered = bin.dir.join("registered");
    register(&repo, &[registered.to_str().unwrap()]);

    // By name (found on PATH) and by absolute path; as an array and as a
    // string.
    let keys = [
        "command = [\"evil\"]".to_owned(),
        format!("command = [{:?}, \"--serve\"]", evil.to_str().unwrap()),
        format!("command = {:?}", evil.to_str().unwrap()),
    ];
    for key in &keys {
        let config = clean_config.replace(
            SCHEME,
            &format!("[schemes.ref]\ndescription = \"notes\"\n{key}\n"),
        );
        assert_ne!(config, clean_config);
        repo.write(".hibi/hibi.toml", &config);
        let before = repo.files();

        let run = |args: &[&str]| {
            repo.hibi()
                .env("PATH", bin.only())
                .args(["--format", "json"])
                .args(args)
                .run()
        };
        let invocations: Vec<Vec<&str>> = vec![
            vec!["claim", "x", "--subject", "ref:notes"],
            vec!["claim", "x", "--subject", "file:docs/a.md::Beta is two."],
            vec!["status"],
            vec!["status", &id],
            vec!["status", "--since", "HEAD"],
            vec!["mark", &id],
            vec!["reanchor", &id, "--statement", "other"],
            vec!["retire", &id],
            vec!["cover"],
            vec!["cover", "--since", "HEAD"],
            vec!["list"],
            vec!["list", &id],
            vec!["providers"],
            vec!["providers", "--check", "ref"],
        ];
        for args in &invocations {
            let out = run(args);
            let e = out.err();
            assert_eq!(e["code"], "invalid_config", "{key}: {args:?}: {e}");
            assert_eq!(out.errs().len(), 1);
            assert_eq!(out.json()["command"], args[0]);
        }
        let out = claim_stdin(
            &repo,
            &json!([{"statement": "x", "subject": {"loc": "ref:notes", "sel": {"key": "n2"}}}]),
        );
        assert_eq!(out.code(), "invalid_config", "{key}: {out:?}");

        assert_eq!(repo.files(), before, "{key}");
        let calls = bin.calls();
        assert!(
            !calls
                .iter()
                .any(|c| c.starts_with("evil") || c.starts_with("registered")),
            "{key}: {calls:?}"
        );
        assert_only_git(&calls);
    }

    // Any other key than `description` in a scheme table is rejected too.
    let config = clean_config.replace(SCHEME, "[schemes.ref]\ntimeout_ms = 5\n");
    repo.write(".hibi/hibi.toml", &config);
    assert_eq!(repo.run(&["status"]).code(), "invalid_config");

    // With the key removed the store works again and nothing was lost.
    repo.write(".hibi/hibi.toml", &clean_config);
    register_reference(&repo, &notes_fixture());
    assert_eq!(repo.state(&id), "fresh");
}

/// A registered reference provider end to end: claim with a `key` selector,
/// fresh, changed after the note's text changes, fresh again after a mark.
#[test]
fn a_registered_provider_resolves_and_detects_a_changed_note() {
    let repo = repo_with_ref();
    let id = claim_note(&repo, "n1");

    let view = repo.view(&id);
    assert_eq!(view["state"], "fresh");
    assert_eq!(view["next"], Value::Null);
    let subject = &view["sides"][0];
    assert_eq!(subject["index"], 0);
    assert_eq!(subject["role"], "subject");
    assert_eq!(
        subject["anchor"],
        json!({"loc": "ref:notes", "sel": {"key": "n1"}})
    );
    assert_eq!(subject["state"], "same");
    assert_eq!(subject["reason"], Value::Null);
    assert_eq!(subject["approx"], false);
    assert_eq!(
        subject["unit"],
        json!({"key": "n1", "kind": "note", "label": "Retries are capped at 5.",
               "line": null, "endLine": null})
    );
    let marked = subject["fp"].as_str().unwrap().to_owned();
    assert!(marked.starts_with("b3:prose-v1:"), "{marked}");
    assert_eq!(subject["markedFp"], marked.as_str());
    let evidence = &view["sides"][1];
    assert_eq!(evidence["role"], "evidence");
    assert_eq!(evidence["anchor"]["loc"], "file:docs/a.md");
    assert_eq!(evidence["unit"]["label"], "Alpha is one.");
    assert_eq!(evidence["state"], "same");

    let v = repo.json(&["status"]);
    assert_eq!(ids(&v), Vec::<String>::new());
    assert_eq!(
        v["summary"],
        json!({"claims": 1, "fresh": 1, "drifted": 0, "stale": 0, "expired": 0, "retired": 0})
    );
    let detail = repo.json(&["status", "--detail", &id]);
    assert_eq!(
        detail["claims"][0]["sides"][0]["text"],
        "Retries are capped at 5."
    );

    // The stored claim holds the selector as given, and the listing shows
    // the locators.
    let listed = repo.json(&["list"]);
    assert_eq!(listed["claims"][0]["id"], id.as_str());
    assert_eq!(listed["claims"][0]["subject"], "ref:notes");
    assert_eq!(listed["claims"][0]["evidence"], json!(["file:docs/a.md"]));

    // The same note with another text.
    let text = std::fs::read_to_string(notes_fixture()).unwrap();
    assert_eq!(text.matches("Retries are capped at 5.").count(), 1);
    let changed = repo.home.join("notes-changed.json");
    std::fs::write(
        &changed,
        text.replace("Retries are capped at 5.", "Retries are capped at 7."),
    )
    .unwrap();
    register_reference(&repo, &changed);

    let v = repo.json(&["status"]);
    assert_eq!(ids(&v), vec![id.clone()]);
    let claim = &v["claims"][0];
    assert_eq!(claim["state"], "stale");
    assert_eq!(claim["next"], format!("hibi mark {id}"));
    let subject = &claim["sides"][0];
    assert_eq!(subject["state"], "changed");
    assert_eq!(subject["reason"], "fingerprint_differs");
    assert_eq!(subject["unit"]["key"], "n1");
    assert_eq!(subject["unit"]["label"], "Retries are capped at 7.");
    assert_eq!(subject["markedFp"], marked.as_str());
    assert_ne!(subject["fp"], marked.as_str());
    assert_eq!(claim["sides"][1]["state"], "same");

    // Back to the first fixture: fresh again without a new mark.
    register_reference(&repo, &notes_fixture());
    assert_eq!(repo.state(&id), "fresh");

    // With the changed fixture, a mark makes it fresh.
    register_reference(&repo, &changed);
    let v = repo.json(&["--now", NOW, "mark", &id]);
    assert_eq!(v["marks"][0]["claim"], id.as_str());
    assert_eq!(v["marks"][0]["claimView"]["state"], "fresh");
    assert_eq!(repo.state(&id), "fresh");

    // A note whose key is gone is missing.
    let gone = repo.home.join("notes-gone.json");
    std::fs::write(&gone, text.replace("\"key\": \"n1\"", "\"key\": \"n9\"")).unwrap();
    assert_ne!(std::fs::read_to_string(&gone).unwrap(), text);
    register_reference(&repo, &gone);
    let view = repo.view(&id);
    assert_eq!(view["state"], "stale");
    assert_eq!(view["sides"][0]["state"], "missing");
    assert_eq!(view["sides"][0]["reason"], "selector_not_found");
    assert!(
        view["next"]
            .as_str()
            .unwrap()
            .starts_with(&format!("hibi reanchor {id}")),
        "{view}"
    );
}

/// Section 5.5: the compact form of an external anchor is the bare locator
/// with selector `{}`. Against the reference provider `{}` resolves to every
/// unit of the resource, so `ref:notes` is ambiguous and `ref:empty` is
/// missing; a locator the fixture does not have is missing too.
#[test]
fn a_bare_external_locator_has_the_empty_selector() {
    let repo = repo_with_ref();
    let before = repo.store_files();

    let out = repo.run(&["claim", "x", "--subject", "ref:notes"]);
    assert_eq!(out.code(), "side_ambiguous", "{out:?}");
    let out = repo.run(&["claim", "x", "--subject", "ref:empty"]);
    assert_eq!(out.code(), "side_missing", "{out:?}");
    let out = repo.run(&["claim", "x", "--subject", "ref:absent"]);
    assert_eq!(out.code(), "side_missing", "{out:?}");
    let out = claim_stdin(
        &repo,
        &json!([{"statement": "x", "subject": {"loc": "ref:notes", "sel": {"key": "zz"}}}]),
    );
    assert_eq!(out.code(), "side_missing", "{out:?}");
    assert_eq!(repo.store_files(), before);

    // A quote selector that names one note resolves, and is stored completed
    // as the provider's selector for the unit.
    let out = claim_stdin(
        &repo,
        &json!([{"statement": "x", "subject": {"loc": "ref:notes", "sel": {"quote": "configurable"}}}]),
    );
    let v = out.ok();
    let side = &v["claims"][0]["claim"]["sides"][0];
    assert_eq!(side["unit"]["key"], "n2");
    assert_eq!(
        side["anchor"],
        json!({"loc": "ref:notes", "sel": {"key": "n2"}})
    );
}

/// Section 10.2 with an external scope: the units come from the provider's
/// `units`; a side covers its unit and every ancestor by the `parent` chain.
/// In the fixture n2's parent is n1.
#[test]
fn cover_of_an_external_scope_follows_the_parent_chain() {
    // A claim on n1 covers n1 only: n2 is nested in n1, not the reverse.
    let repo = repo_with_ref();
    repo.config(NOTES_SCOPE);
    repo.config("[[scope]]\nname = \"tasks\"\nresource = \"ref:notes\"\nkinds = [\"task\"]\n\n[[scope]]\nname = \"none\"\nresource = \"ref:empty\"\nkinds = [\"note\"]\n");

    let v = repo.json(&["cover", "--scope", "notes"]);
    assert_eq!(counts(&v["summary"]), (2, 0, 0, 2), "{v}");
    let keys = |v: &Value| -> Vec<String> {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["unit"]["key"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(keys(&v["scopes"][0]), ["n1", "n2"]);

    claim_note(&repo, "n1");
    let v = repo.json(&["cover"]);
    assert_eq!(v["scopes"].as_array().unwrap().len(), 3);
    assert_eq!(counts(&v["summary"]), (3, 1, 0, 2), "{v}");
    let notes = &v["scopes"][0];
    assert_eq!(notes["name"], "notes");
    assert_eq!(notes["resource"], "ref:notes");
    assert_eq!(notes["kinds"], json!(["note"]));
    assert_eq!(counts(notes), (2, 1, 0, 1));
    assert_eq!(notes["skipped"], Value::Null);
    assert_eq!(notes["unavailable"], json!([]));
    assert_eq!(
        notes["items"],
        json!([{
            "loc": "ref:notes",
            "unit": {"key": "n2", "kind": "note", "label": "The cap is configurable.",
                     "line": null, "endLine": null},
            "text": "The cap is configurable.",
            "anchor": {"loc": "ref:notes", "sel": {"key": "n2"}},
        }])
    );
    let tasks = &v["scopes"][1];
    assert_eq!(tasks["name"], "tasks");
    assert_eq!(counts(tasks), (1, 0, 0, 1));
    assert_eq!(keys(tasks), ["t1"]);
    assert_eq!(tasks["items"][0]["text"], "Document the retry cap.");
    let none = &v["scopes"][2];
    assert_eq!(none["name"], "none");
    assert_eq!(counts(none), (0, 0, 0, 0));
    assert_eq!(none["items"], json!([]));
    assert_eq!(none["unavailable"], json!([]));

    // The listed anchor can be claimed as it is; then both notes are covered.
    let out = claim_stdin(
        &repo,
        &json!([{"statement": "the cap can be set", "subject": notes["items"][0]["anchor"]}]),
    );
    out.ok();
    let v = repo.json(&["cover", "--scope", "notes"]);
    assert_eq!(counts(&v["summary"]), (2, 2, 0, 0), "{v}");

    // `--since` does not evaluate external scopes.
    repo.commit("base");
    let v = repo.json(&["cover", "--since", "HEAD"]);
    for scope in v["scopes"].as_array().unwrap() {
        assert_eq!(scope["skipped"], "since_unsupported", "{scope}");
        assert_eq!(scope["items"], json!([]));
    }

    // A claim on n2 alone covers n2 and its parent n1.
    let repo = repo_with_ref();
    repo.config(NOTES_SCOPE);
    claim_note(&repo, "n2");
    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (2, 2, 0, 0), "{v}");
    assert_eq!(v["scopes"][0]["items"], json!([]));

    // A resource the provider does not have is reported as unavailable.
    let repo = repo_with_ref();
    repo.config("[[scope]]\nname = \"absent\"\nresource = \"ref:absent\"\nkinds = [\"note\"]\n");
    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (0, 0, 0, 0), "{v}");
    assert_eq!(
        v["scopes"][0]["unavailable"],
        json!([{"loc": "ref:absent", "reason": "resource_not_found"}])
    );
}

/// Section 15.3: `providers` lists the scheme with its registration, and
/// `providers --check <scheme>` runs the conformance cases against it.
#[test]
fn providers_lists_the_registration_and_check_runs_the_conformance_cases() {
    let repo = Repo::new();
    repo.config(SCHEME);
    repo.config("[schemes.other]\n");
    let command = register_reference(&repo, &notes_fixture());

    let v = repo.json(&["providers"]);
    assert_eq!(v["command"], "providers");
    assert_eq!(
        v["builtin"],
        json!({"schemes": ["file", "repo"],
               "classes": ["markdown", "prose", "typescript", "tsx", "python", "rust", "go", "java", "text"],
               "profiles": ["prose-v1", "ast-v1"]})
    );
    assert_eq!(v["repos"], json!([]));
    assert_eq!(
        v["schemes"],
        json!([
            {"scheme": "other", "registered": false, "command": null,
             "remediation": "[providers.other]\ncommand = [\"<program>\", \"<argument>\"]"},
            {"scheme": "ref", "registered": true, "command": command, "remediation": null},
        ])
    );
    assert_eq!(v["userConfig"], repo.user_config_path().to_str().unwrap());
    assert_eq!(v["warnings"], json!([]));
    assert!(v.get("check").is_none(), "{v}");

    let human = repo.human(&["providers"]);
    assert_eq!(human.status, 0, "{human:?}");
    assert_eq!(human.stderr, "");
    assert!(
        human.stdout.contains("[providers.other]"),
        "{}",
        human.stdout
    );
    assert!(human.stdout.contains("command = "), "{}", human.stdout);
    assert!(
        !human.stdout.contains("[providers.ref]"),
        "{}",
        human.stdout
    );
    assert!(human.stdout.contains(&command[0]), "{}", human.stdout);

    let v = repo.json(&["providers", "--check", "ref"]);
    assert_eq!(v["schemes"][1]["registered"], true);
    let check = &v["check"];
    assert_eq!(check["scheme"], "ref");
    let hello = &check["hello"];
    assert_eq!(hello["protocol"], 1);
    assert_eq!(hello["provider"]["name"], "hibi-provider-reference");
    assert_eq!(hello["schemes"], json!(["ref"]));
    assert_eq!(hello["selectors"], json!(["key", "quote"]));
    assert_eq!(
        hello["capabilities"],
        json!({"batch": true, "render": true, "units": true})
    );
    let cases = check["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    let mut passed = 0;
    for case in cases {
        assert!(!case["name"].as_str().unwrap().is_empty(), "{case}");
        assert_eq!(case["ok"], true, "{case}");
        assert!(case["skipped"].is_boolean(), "{case}");
        assert!(case.get("detail").is_some(), "{case}");
        if case["skipped"] == false {
            passed += 1;
        }
    }
    assert!(check["passed"].as_u64().unwrap() > 0, "{check}");
    assert_eq!(check["passed"], passed);
    assert_eq!(check["failed"], 0);
    let mut names: Vec<&str> = cases.iter().map(|c| c["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), cases.len(), "case names are distinct");

    // A scheme that is listed but not registered, and one that is not
    // listed: invalid_argument.
    let out = repo.run(&["providers", "--check", "other"]);
    assert_eq!(out.code(), "invalid_argument", "{out:?}");
    let out = repo.run(&["providers", "--check", "nope"]);
    assert_eq!(out.code(), "invalid_argument", "{out:?}");
    let out = repo.run(&["providers", "--check", "file"]);
    assert_eq!(out.code(), "invalid_argument", "{out:?}");
}

/// R56: plain `hibi providers` starts no child process other than git, even
/// with a provider registered and a sibling repository allowed;
/// `providers --check` does start the registered command.
#[test]
fn r56_plain_providers_starts_nothing_and_check_starts_the_provider() {
    let repo = Repo::new();
    repo.config(SCHEME);
    repo.config("[repos]\napi = \"../api\"\n");
    repo.write("docs/a.md", DOC);

    let bin = recording_path(&repo, &["refprov"]);
    let stand_in = bin.dir.join("refprov");
    let stand_in = stand_in.to_str().unwrap();
    for command in [vec![stand_in, "--serve"], vec!["refprov", "--serve"]] {
        std::fs::remove_file(&bin.log).ok();
        register(&repo, &command);
        let run = |args: &[&str]| {
            repo.hibi()
                .env("PATH", bin.only())
                .args(["--format", "json"])
                .args(args)
                .run()
        };

        let v = run(&["providers"]).ok();
        assert_eq!(
            v["schemes"],
            json!([{"scheme": "ref", "registered": true, "command": command, "remediation": null}])
        );
        let human = repo
            .hibi()
            .env("PATH", bin.only())
            .args(["--format", "human", "providers"])
            .run();
        assert_eq!(human.status, 0, "{human:?}");
        // Commands that need no locator of the scheme start nothing either.
        run(&["status"]).ok();
        run(&["list"]).ok();
        run(&[
            "claim",
            "alpha",
            "--subject",
            "file:docs/a.md::Alpha is one.",
        ])
        .ok();
        let calls = bin.calls();
        assert!(!calls.iter().any(|c| c.starts_with("refprov")), "{calls:?}");
        assert_only_git(&calls);

        // --check starts it. The stand-in answers nothing, so there is no
        // hello; the command itself still succeeds.
        let v = run(&["providers", "--check", "ref"]).ok();
        assert_eq!(v["check"]["scheme"], "ref");
        assert_eq!(v["check"]["hello"], Value::Null);
        assert_eq!(v["check"]["passed"], 0);
        let calls = bin.calls();
        let started: Vec<&String> = calls.iter().filter(|c| c.starts_with("refprov")).collect();
        // The conformance runner starts the command once per case
        // (section 13.6), always with the registered arguments.
        assert!(!started.is_empty(), "{calls:?}");
        assert!(started.iter().all(|c| *c == "refprov --serve"), "{calls:?}");
    }
}

/// R31 seen from outside: a command that needs a locator of the scheme
/// starts the registered command once per run, however many sides use it.
#[test]
fn a_run_that_needs_the_scheme_starts_the_provider_once() {
    let repo = repo_with_ref();
    repo.config(NOTES_SCOPE);
    let wrapper = repo.home.join("counting-provider");
    let log = repo.home.join("starts.log");
    write_script(
        &wrapper,
        &format!(
            "echo start >> '{}'\nexec '{}' --fixture '{}'",
            log.display(),
            reference_provider().display(),
            notes_fixture().display()
        ),
    );
    register(&repo, &[wrapper.to_str().unwrap()]);
    let starts = || {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .count()
    };

    let out = claim_stdin(
        &repo,
        &json!([
            {"statement": "one", "subject": {"loc": "ref:notes", "sel": {"key": "n1"}},
             "evidence": [{"loc": "ref:notes", "sel": {"key": "t1"}}]},
            {"statement": "two", "subject": {"loc": "ref:notes", "sel": {"key": "n2"}}},
        ]),
    );
    assert_eq!(out.ok()["claims"].as_array().unwrap().len(), 2);
    assert_eq!(starts(), 1);
    repo.json(&["status", "--state", "fresh"]);
    assert_eq!(starts(), 2);
    repo.json(&["cover"]);
    assert_eq!(starts(), 3);
    // No locator of the scheme is needed here.
    repo.json(&["list"]);
    repo.json(&["providers"]);
    assert_eq!(starts(), 3);
}

fn write_script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// R62: a selector with a key the provider does not allow is never written
/// to the provider's standard input. The registered command copies its
/// standard input to a file and passes it on to the reference provider.
#[test]
fn r62_an_unsupported_selector_key_is_never_sent_to_the_provider() {
    let repo = Repo::new();
    repo.config(SCHEME);
    repo.write("docs/a.md", DOC);
    let capture = repo.home.join("provider-stdin.log");
    let wrapper = repo.home.join("capturing-provider");
    write_script(
        &wrapper,
        &format!(
            "tee -a '{}' | '{}' --fixture '{}'",
            capture.display(),
            reference_provider().display(),
            notes_fixture().display()
        ),
    );
    register(&repo, &[wrapper.to_str().unwrap()]);
    let captured = || std::fs::read_to_string(&capture).unwrap_or_default();

    // The capture works: a supported selector reaches the provider, after
    // `hello`.
    let id = claim_note(&repo, "n1");
    let seen = captured();
    let first: Value = serde_json::from_str(seen.lines().next().expect("a request")).unwrap();
    assert_eq!(first["method"], "hello", "{seen}");
    assert!(seen.contains("\"n1\""), "{seen}");
    assert!(!seen.contains("symbol"), "{seen}");
    std::fs::remove_file(&capture).unwrap();

    let before = repo.store_files();
    let out = claim_stdin(
        &repo,
        &json!([{"statement": "x", "subject": {"loc": "ref:notes", "sel": {"symbol": "sym-marker"}}}]),
    );
    let e = out.err();
    assert_eq!(e["code"], "unsupported_selector", "{e}");
    assert_eq!(e["index"], 0);
    assert_eq!(out.errs().len(), 1);
    let seen = captured();
    assert!(!seen.contains("symbol"), "{seen}");
    assert!(!seen.contains("sym-marker"), "{seen}");

    // In a batch with a valid element: nothing is written, the bad element
    // is reported by index, and its selector is still not sent. Also as an
    // evidence side and together with an allowed key.
    let out = claim_stdin(
        &repo,
        &json!([
            {"statement": "good", "subject": {"loc": "ref:notes", "sel": {"key": "n2"}}},
            {"statement": "bad", "subject": "file:docs/a.md::Beta is two.",
             "evidence": [{"loc": "ref:notes", "sel": {"key": "n1", "symbol": "sym-marker"}}]},
            {"statement": "bad too", "subject": {"loc": "ref:notes", "sel": {"symbol": "sym-marker", "quote": "configurable"}}},
        ]),
    );
    let errors = out.errs();
    let reported: Vec<(String, u64)> = errors
        .iter()
        .map(|e| {
            (
                e["code"].as_str().unwrap().to_owned(),
                e["index"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        reported,
        [
            ("unsupported_selector".to_owned(), 1),
            ("unsupported_selector".to_owned(), 2)
        ]
    );
    let seen = captured();
    assert!(!seen.contains("symbol"), "{seen}");
    assert!(!seen.contains("sym-marker"), "{seen}");
    assert_eq!(repo.store_files(), before);

    // `key` is not a selector key of the file provider.
    let out = claim_stdin(
        &repo,
        &json!([{"statement": "x", "subject": {"loc": "file:docs/a.md", "sel": {"key": "sentence:0"}}}]),
    );
    let e = out.err();
    assert_eq!(e["code"], "unsupported_selector", "{e}");
    assert_eq!(e["index"], 0);
    assert_eq!(repo.store_files(), before);
    assert_eq!(repo.state(&id), "fresh");
}

/// Section 15.3 and 11.1: `providers` reports every `[repos]` alias with the
/// result of the checks. An alias with no user entry is `repo_not_allowed`
/// with `path` null, and the human output prints the `[[repo]]` lines to add.
#[test]
fn providers_reports_repo_aliases_and_the_lines_to_add() {
    let repo = Repo::new();
    repo.config("[repos]\napi = \"../api\"\ngone = \"../gone\"\n");
    let root = repo.root.to_str().unwrap();
    let parent = repo.root.parent().unwrap();

    let v = repo.json(&["providers"]);
    // The lines to add are in the JSON too: the agent reads JSON.
    let to_add = |alias: &str| {
        format!(
            "Add these lines to the user config:\n[[repo]]\nroot = \"{root}\"\nalias = \"{alias}\"\npath = \"{}\"",
            parent.join(alias).display()
        )
    };
    assert_eq!(
        v["repos"],
        json!([
            {"alias": "api", "suggested": "../api", "path": null, "error": "repo_not_allowed",
             "remediation": to_add("api")},
            {"alias": "gone", "suggested": "../gone", "path": null, "error": "repo_not_allowed",
             "remediation": to_add("gone")},
        ])
    );
    assert_eq!(v["schemes"], json!([]));

    let human = repo.human(&["providers"]);
    assert_eq!(human.status, 0, "{human:?}");
    assert_eq!(human.stderr, "");
    let lines: Vec<&str> = human.stdout.lines().map(str::trim).collect();
    for alias in ["api", "gone"] {
        // The four lines of one entry, in the order of section 11.2, with
        // the suggested location made absolute as `path`.
        let entry = [
            "[[repo]]".to_owned(),
            format!("root = {root:?}"),
            format!("alias = {alias:?}"),
            format!("path = {:?}", parent.join(alias).to_str().unwrap()),
        ];
        assert!(
            lines.windows(4).any(|w| w == entry),
            "{alias}: {}",
            human.stdout
        );
    }
    assert!(
        human.stdout.contains("repo_not_allowed"),
        "{}",
        human.stdout
    );

    // Allow `api` at a real sibling repository, and `gone` at a directory
    // that does not exist.
    let api = parent.join("api");
    std::fs::create_dir_all(&api).unwrap();
    let init = std::process::Command::new(real_git())
        .current_dir(&api)
        .args(["init", "-q", "-b", "main", "."])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(init.success());
    let api = api.to_str().unwrap();
    let gone = parent.join("gone");
    let gone = gone.to_str().unwrap();
    repo.user_config(&format!(
        "[[repo]]\nroot = {root:?}\nalias = \"api\"\npath = {api:?}\n\n[[repo]]\nroot = {root:?}\nalias = \"gone\"\npath = {gone:?}\n"
    ));
    let v = repo.json(&["providers"]);
    assert_eq!(
        v["repos"],
        json!([
            {"alias": "api", "suggested": "../api", "path": api, "error": null,
             "remediation": null},
            {"alias": "gone", "suggested": "../gone", "path": gone, "error": "repo_not_found",
             "remediation": null},
        ])
    );
    let human = repo.human(&["providers"]);
    assert_eq!(human.status, 0, "{human:?}");
    assert!(!human.stdout.contains("[[repo]]"), "{}", human.stdout);

    // An entry for another root does not allow the alias here; a directory
    // inside the root is not allowed either.
    let inside = repo.root.join("vendor");
    std::fs::create_dir_all(&inside).unwrap();
    let inside = inside.to_str().unwrap();
    repo.user_config(&format!(
        "[[repo]]\nroot = {api:?}\nalias = \"api\"\npath = {api:?}\n\n[[repo]]\nroot = {root:?}\nalias = \"gone\"\npath = {inside:?}\n"
    ));
    let v = repo.json(&["providers"]);
    // The alias with no entry for this root comes with the lines to add;
    // the entry that points inside the root has none, since adding lines
    // would not help.
    assert_eq!(
        v["repos"],
        json!([
            {"alias": "api", "suggested": "../api", "path": null, "error": "repo_not_allowed",
             "remediation": to_add("api")},
            {"alias": "gone", "suggested": "../gone", "path": inside, "error": "repo_not_allowed",
             "remediation": v["repos"][1]["remediation"]},
        ])
    );
    assert!(
        v["repos"][1]["remediation"].is_null()
            || !v["repos"][1]["remediation"].as_str().unwrap().is_empty()
    );
}
