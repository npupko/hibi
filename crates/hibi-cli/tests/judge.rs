//! The judge commands through the binary, against a scripted HTTP server
//! on 127.0.0.1 given with `HIBI_JEV_URL` (spec section 14; requirements
//! R24 to R27 and R61 at the process level).

mod support;

#[path = "../../hibi/tests/judge/mock.rs"]
mod mock;

use mock::Mock;
use serde_json::{Value, json};
use support::*;

const KEY: &str = "test-key";

/// A repository with one covered sentence, two uncovered ones, a source
/// file with two declarations, an ignored file, and two stale claims.
struct Scene {
    repo: Repo,
    /// The claim on `MAX_RETRIES`, stale.
    retries: String,
    /// The claim on `delay`, stale.
    delay: String,
}

fn scene() -> Scene {
    let repo = Repo::new();
    repo.config(
        "[[scope]]\nname = \"docs\"\nresource = \"file:docs/**/*.md\"\nkinds = [\"sentence\"]\n",
    );
    repo.write(
        "docs/retry.md",
        "# Retries\n\nDownloads are retried up to 5 times. The delay doubles after each attempt. We like short documents.\n",
    );
    repo.write(
        "src/retry.rs",
        "pub const MAX_RETRIES: u32 = 5;\n\npub fn delay(attempt: u32) -> u64 {\n    100 * 2u64.pow(attempt)\n}\n",
    );
    repo.write(".gitignore", "secret/\n");
    repo.write("secret/notes.md", "The launch code is swordfish.\n");
    repo.write(
        "secret/key.rs",
        "pub const LAUNCH_CODE: &str = \"swordfish\";\n",
    );
    repo.commit("base");
    let retries = repo.claim(
        "Downloads are retried up to 5 times.",
        "file:docs/retry.md::retried up to 5 times",
        &["file:src/retry.rs#MAX_RETRIES"],
    );
    let delay = repo.claim(
        "The delay doubles.",
        "file:src/retry.rs#delay",
        &["file:src/retry.rs#delay"],
    );
    repo.write(
        "src/retry.rs",
        "pub const MAX_RETRIES: u32 = 7;\n\npub fn delay(attempt: u32) -> u64 {\n    200 * 2u64.pow(attempt)\n}\n",
    );
    Scene {
        repo,
        retries,
        delay,
    }
}

/// A server that rates text by what it mentions.
fn server() -> Mock {
    Mock::answering(|_name, question, state| {
        if question["type"] == "choice" {
            return mock::choice(question, "code", 0.8);
        }
        // `suggest-evidence` names the candidate in `where`; the other
        // commands are rated by their text and statement.
        if let Some(place) = question["instructions"]["where"].as_str() {
            return if place.contains("delay") {
                json!(0.9)
            } else {
                json!(0.4)
            };
        }
        let text = question["instructions"].to_string() + &state.to_string();
        if text.contains("doubles") {
            json!(0.9)
        } else if text.contains("MAX_RETRIES") || text.contains("retried") {
            json!(0.4)
        } else {
            json!(0.1)
        }
    })
}

fn judge(scene: &Scene, server: &Mock, args: &[&str]) -> Out {
    scene
        .repo
        .hibi()
        .env("TYPESAFE_API_KEY", KEY)
        .env("HIBI_JEV_URL", &server.url)
        .args(["--format", "json", "--now", NOW])
        .args(args)
        .run()
}

fn texts(items: &Value, key: &str) -> Vec<String> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i[key].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn triage_ranks_the_uncovered_prose_and_writes_nothing() {
    let scene = scene();
    let server = server();
    let before = scene.repo.store_files();
    let v = judge(&scene, &server, &["triage"]).ok();

    assert_eq!(v["command"], "triage");
    assert_eq!(v["model"], "jev-1.14.0");
    assert_eq!(v["considered"], 2);
    assert_eq!(v["skipped"], 0);
    assert_eq!(
        texts(&v["items"], "text"),
        [
            "The delay doubles after each attempt.",
            "We like short documents."
        ]
    );
    let first = &v["items"][0];
    assert_eq!(first["loc"], "file:docs/retry.md");
    assert_eq!(first["checkable"], 0.9);
    assert_eq!(first["band"], "likely");
    assert_eq!(first["evidence"], "code");
    assert_eq!(first["unit"]["kind"], "sentence");
    assert_eq!(first["anchor"]["loc"], "file:docs/retry.md");
    assert_eq!(v["items"][1]["band"], "unlikely");

    // R24: the model id, one question pair per unit, the key as a bearer.
    let requests = server.recorded();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].body["model"], "jev-1.14.0");
    assert_eq!(
        requests[0].names(),
        ["q0.checkable", "q0.evidence", "q1.checkable", "q1.evidence"]
    );
    assert_eq!(requests[0].header("authorization"), Some("Bearer test-key"));
    // R27: no judge command changes a file under .hibi/.
    assert_eq!(scene.repo.store_files(), before);

    // R25: a second identical run with a warm cache sends no request and
    // prints the same result.
    let again = judge(&scene, &server, &["triage"]).ok();
    assert_eq!(again, v);
    assert_eq!(server.count(), 1);

    // `--limit` cuts the list and not the count; 0 means no cap.
    let one = judge(&scene, &server, &["triage", "--limit", "1"]).ok();
    assert_eq!(one["items"].as_array().unwrap().len(), 1);
    assert_eq!(one["considered"], 2);
    let all = judge(&scene, &server, &["triage", "--limit", "0"]).ok();
    assert_eq!(all["items"].as_array().unwrap().len(), 2);
    assert_eq!(server.count(), 1);
}

#[test]
fn triage_since_considers_only_the_new_units() {
    let scene = scene();
    let server = server();
    let base = scene.repo.git(&["rev-parse", "HEAD"]);
    scene.repo.write(
        "docs/retry.md",
        "# Retries\n\nDownloads are retried up to 5 times. The delay doubles after each attempt. We like short documents.\n\n## Limits\n\nThe delay never doubles past one minute.\n",
    );
    let v = judge(&scene, &server, &["triage", "--since", &base]).ok();
    assert_eq!(
        texts(&v["items"], "text"),
        ["The delay never doubles past one minute."]
    );
    assert_eq!(v["considered"], 1);

    let bad = judge(&scene, &server, &["triage", "--since", "HEAD..main"]);
    assert_eq!(bad.code(), "invalid_range");
    let none = judge(&scene, &server, &["triage", "--scope", "nope"]);
    assert_eq!(none.code(), "unknown_scope");
}

#[test]
fn suggest_evidence_ranks_candidates_and_says_when_none_fits() {
    let scene = scene();
    let server = server();
    let before = scene.repo.store_files();
    let v = judge(
        &scene,
        &server,
        &[
            "suggest-evidence",
            "The delay doubles.",
            "--in",
            "file:src/**/*.rs",
        ],
    )
    .ok();
    assert_eq!(v["command"], "suggest-evidence");
    assert_eq!(v["model"], "jev-1.14.0");
    assert_eq!(v["statement"], "The delay doubles.");
    assert_eq!(v["considered"], 2);
    assert_eq!(v["noneFits"], false);
    let candidates = v["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0]["unit"]["label"], "delay");
    assert_eq!(candidates[0]["unit"]["kind"], "function");
    assert_eq!(candidates[0]["loc"], "file:src/retry.rs");
    assert_eq!(candidates[0]["fits"], true);
    assert_eq!(candidates[1]["unit"]["label"], "MAX_RETRIES");
    assert_eq!(candidates[1]["fits"], false);
    assert!(candidates[0]["p"].as_f64() > candidates[1]["p"].as_f64());
    assert_eq!(scene.repo.store_files(), before);

    // One question per candidate (R24).
    let requests = server.recorded();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].names(), ["q0", "q1"]);

    // `--kind` narrows the candidates; `--limit` cuts the list.
    let only = judge(
        &scene,
        &server,
        &[
            "suggest-evidence",
            "The delay doubles.",
            "--in",
            "file:src/**/*.rs",
            "--kind",
            "variable",
            "--limit",
            "5",
        ],
    )
    .ok();
    assert_eq!(only["considered"], 1);
    assert_eq!(only["candidates"][0]["unit"]["label"], "MAX_RETRIES");
    assert_eq!(only["noneFits"], true);

    let human = scene
        .repo
        .hibi()
        .env("TYPESAFE_API_KEY", KEY)
        .env("HIBI_JEV_URL", &server.url)
        .args([
            "--format",
            "human",
            "suggest-evidence",
            "The delay doubles.",
            "--in",
            "file:src/**/*.rs",
            "--kind",
            "variable",
        ])
        .run();
    assert_eq!(human.status, 0, "{human:?}");
    assert!(
        human
            .stdout
            .contains("None of the candidates fits the statement."),
        "{}",
        human.stdout
    );

    // `--in` is required.
    let missing = judge(&scene, &server, &["suggest-evidence", "The delay doubles."]);
    assert_eq!(missing.code(), "usage");
}

#[test]
fn status_judge_order_reorders_the_stale_claims_and_changes_nothing_else() {
    let scene = scene();
    let server = server();
    let plain = scene.repo.json(&["--now", NOW, "status"]);
    let ids = texts(&plain["claims"], "id");
    let mut by_id = vec![scene.retries.clone(), scene.delay.clone()];
    by_id.sort();
    assert_eq!(ids, by_id);

    // The server rates the `delay` claim 0.9 (still true) and the retries
    // claim 0.4, so the retries claim (most likely broken) comes first.
    let ordered = judge(&scene, &server, &["status", "--judge-order"]).ok();
    assert_eq!(
        texts(&ordered["claims"], "id"),
        [scene.retries.clone(), scene.delay.clone()]
    );
    assert_eq!(ordered["summary"], plain["summary"]);
    assert_eq!(ordered["snapshot"], plain["snapshot"]);
    let sorted = |v: &Value| {
        let mut claims = v["claims"].as_array().unwrap().clone();
        claims.sort_by_key(|c| c["id"].as_str().unwrap().to_owned());
        claims
    };
    assert_eq!(sorted(&ordered), sorted(&plain));

    // One request per stale claim, one question each (R24).
    let requests = server.recorded();
    assert_eq!(requests.len(), 2);
    for r in &requests {
        assert_eq!(r.names(), ["q0"]);
        assert_eq!(r.body["model"], "jev-1.14.0");
    }
}

/// Section 14.5: a claim with a side found only by similarity has no text
/// to send. It is not sent, and it comes before the claims that were.
#[test]
fn status_judge_order_does_not_send_a_claim_with_a_side_that_is_not_exact() {
    let scene = scene();
    let short = scene.repo.claim(
        "Documents are short.",
        "file:docs/retry.md::We like short documents.",
        &[],
    );
    let text = scene.repo.read("docs/retry.md");
    scene.repo.write(
        "docs/retry.md",
        &text.replace("We like short documents.", "We like brief documents."),
    );
    let view = scene.repo.view(&short);
    assert_eq!(view["sides"][0]["reason"], "not_exact", "{view}");
    assert_eq!(
        view["sides"][0]["candidates"][0]["text"],
        "We like brief documents."
    );

    let server = server();
    let ordered = judge(&scene, &server, &["status", "--judge-order"]).ok();
    assert_eq!(
        texts(&ordered["claims"], "id"),
        [short, scene.retries.clone(), scene.delay.clone()]
    );
    // One request for each of the other two claims; the candidate's text
    // and the claim's statement went nowhere.
    let requests = server.recorded();
    assert_eq!(requests.len(), 2);
    for r in &requests {
        let body = r.body.to_string();
        assert!(!body.contains("brief documents"), "{body}");
        assert!(!body.contains("Documents are short."), "{body}");
    }
}

/// R26: a request that fails on every attempt makes the command fail with
/// `judge_failed`, and nothing is ranked.
#[test]
fn r26_a_failing_judge_is_judge_failed_with_no_ranking() {
    let scene = scene();
    let server = Mock::start(std::sync::Arc::new(|_, _| mock::Reply::status(500)));
    let out = judge(&scene, &server, &["triage"]);
    assert_eq!(out.code(), "judge_failed");
    assert!(out.json().get("items").is_none());
    assert_eq!(server.count(), 3, "three attempts");
}

/// R61: no text of an ignored file reaches a request body.
#[test]
fn r61_ignored_files_are_never_sent() {
    let scene = scene();
    scene
        .repo
        .config("[[scope]]\nname = \"all\"\nresource = \"file:**/*.md\"\nkinds = [\"sentence\"]\n");
    let server = server();
    let v = judge(&scene, &server, &["triage", "--scope", "all"]).ok();
    assert!(
        !texts(&v["items"], "text")
            .iter()
            .any(|t| t.contains("swordfish"))
    );
    let s = judge(
        &scene,
        &server,
        &[
            "suggest-evidence",
            "The launch code.",
            "--in",
            "file:**/*.rs",
        ],
    )
    .ok();
    assert!(
        s["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["loc"] == "file:src/retry.rs")
    );
    assert!(server.count() >= 2);
    assert!(!server.all_bodies().contains("swordfish"));
}

#[test]
fn the_endpoint_must_be_https_or_loopback() {
    let scene = scene();
    let out = scene
        .repo
        .hibi()
        .env("TYPESAFE_API_KEY", KEY)
        .env("HIBI_JEV_URL", "http://example.com/v1/systemone")
        .args(["--format", "json", "triage"])
        .run();
    assert_eq!(out.code(), "invalid_argument");
}

/// Section 14.1: without the key the three commands fail with
/// `judge_unavailable` before any other work, so no connection is made and
/// the answer is the same outside a repository.
#[test]
fn without_the_key_nothing_is_contacted() {
    let scene = scene();
    let server = server();
    for args in [
        vec!["triage"],
        vec!["suggest-evidence", "x", "--in", "file:src/**"],
        vec!["status", "--judge-order"],
    ] {
        for key in [None, Some("")] {
            let mut cmd = scene.repo.hibi().env("HIBI_JEV_URL", &server.url);
            if let Some(k) = key {
                cmd = cmd.env("TYPESAFE_API_KEY", k);
            }
            let out = cmd.args(["--format", "json"]).args(&args).run();
            assert_eq!(out.code(), "judge_unavailable", "{args:?}");
        }
        let outside = scene
            .repo
            .hibi()
            .cwd(&scene.repo.home)
            .args(["--format", "json"])
            .args(&args)
            .run();
        assert_eq!(outside.code(), "judge_unavailable", "{args:?}");
    }
    assert_eq!(
        server.connections.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}
