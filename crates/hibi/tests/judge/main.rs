//! The judge (spec section 14) against a scripted local HTTP server, on a
//! real git repository with the real file provider, store and cache.
//! Requirements R22 to R27, R61 and R64, and the transport policy of 14.7.

#![cfg(all(feature = "judge", feature = "file", feature = "git-store"))]

mod mock;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hibi::Code;
use hibi::cache::Cache;
use hibi::config::{RepoConfig, UserConfig};
use hibi::engine::{self, ClaimInput, ClaimState, StatusQuery};
use hibi::git::GitStatus;
use hibi::judge::{
    self, Band, EvidenceLabel, Judge, JudgeEnv, Policy, SuggestQuery, TriageQuery, questions,
};
use hibi::model::Timestamp;
use hibi::provider::Providers;
use hibi::provider::file::{FileOptions, FileProvider};
use hibi::store::git_text::{self, GitTextStore};
use serde_json::{Value, json};

use mock::{Mock, Reply, choice};

const NOW: &str = "2026-11-20T18:04:11Z";
const KEY: &str = "test-key";

const CONFIG: &str = r#"
version = 1

[[scope]]
name = "docs"
resource = "file:docs/**/*.md"
kinds = ["sentence", "list-item", "table-row", "heading", "fenced"]
"#;

const GUIDE: &str = "# Retries

The client retries a request three times.

- Timeouts are ten seconds.

| Setting | Value |
|---|---|
| model | jev |

```
let x = 1;
```
";

const LIB: &str = "pub fn retries() -> u32 {
    3
}

pub struct Client {
    pub timeout: u32,
}

pub fn unrelated() -> bool {
    true
}
";

fn now() -> Timestamp {
    Timestamp::parse(NOW).unwrap()
}

fn sh_git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    store: GitTextStore,
    config: RepoConfig,
    user: UserConfig,
    git: GitStatus,
    _cache_dir: tempfile::TempDir,
    cache: Cache,
    sleeps: Arc<Mutex<Vec<Duration>>>,
}

/// A git repository with `.hibi/`, `docs/guide.md`, `src/lib.rs`, and two
/// ignored files holding secrets, plus `files`.
fn fixture(files: &[(&str, &str)]) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    sh_git(&root, &["init", "-q"]);
    git_text::init(&root).unwrap();
    let mut all: Vec<(&str, &str)> = vec![
        (".hibi/hibi.toml", CONFIG),
        (".gitignore", "secret.md\nsrc/secret.rs\n"),
        ("docs/guide.md", GUIDE),
        ("src/lib.rs", LIB),
        ("secret.md", "The token is hunter2.\n"),
        ("src/secret.rs", "pub fn hunter2_key() -> u32 {\n    7\n}\n"),
    ];
    all.extend_from_slice(files);
    for (path, content) in all {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(&cache_dir.path().join("cache.sqlite3")).unwrap();
    Fx {
        store: GitTextStore::open(&root).unwrap(),
        config: RepoConfig::parse(&std::fs::read_to_string(root.join(".hibi/hibi.toml")).unwrap())
            .unwrap(),
        user: UserConfig::default(),
        git: GitStatus::probe(),
        root,
        _dir: dir,
        _cache_dir: cache_dir,
        cache,
        sleeps: Arc::default(),
    }
}

impl Fx {
    fn providers(&self) -> Providers {
        let mut p = Providers::new();
        let file = FileProvider::working_tree(
            &self.root,
            FileOptions {
                repo: self.config.clone(),
                user: self.user.clone(),
                git: self.git.clone(),
                cache: None,
            },
        );
        p.register(&["file", "repo"], Box::new(file));
        p
    }

    fn env(&self, cached: bool) -> JudgeEnv<'_> {
        JudgeEnv {
            root: &self.root,
            config: &self.config,
            user: &self.user,
            git: &self.git,
            cache: cached.then_some(&self.cache),
            now: now(),
        }
    }

    fn judge(&self, mock: &Mock) -> Judge {
        let sleeps = self.sleeps.clone();
        Judge::new(Some(KEY), Some(&mock.url))
            .unwrap()
            .with_policy(Policy {
                timeout: Duration::from_millis(400),
                sleep: Arc::new(move |d| sleeps.lock().unwrap().push(d)),
                ..Policy::default()
            })
    }

    fn sleeps(&self) -> Vec<u128> {
        self.sleeps
            .lock()
            .unwrap()
            .iter()
            .map(Duration::as_millis)
            .collect()
    }

    fn triage(&self, judge: &Judge, cached: bool) -> Result<judge::TriageOutput, hibi::Error> {
        let mut p = self.providers();
        judge::triage(
            judge,
            &self.store,
            &mut p,
            &self.env(cached),
            &TriageQuery::default(),
            None,
        )
    }

    fn suggest(
        &self,
        judge: &Judge,
        within: &[&str],
        limit: usize,
    ) -> Result<judge::SuggestOutput, hibi::Error> {
        let mut p = self.providers();
        let q = SuggestQuery {
            statement: "The client retries three times.".into(),
            within: within.iter().map(|s| (*s).to_owned()).collect(),
            kinds: Vec::new(),
            limit,
        };
        judge::suggest_evidence(judge, &mut p, &self.env(false), &q)
    }

    fn order(
        &self,
        judge: &Judge,
        states: &[ClaimState],
    ) -> Result<engine::StatusOutput, hibi::Error> {
        let mut p = self.providers();
        let q = StatusQuery {
            states: states.to_vec(),
            ..StatusQuery::default()
        };
        judge::judge_order(judge, &self.store, &mut p, &self.env(false), &q, None)
    }

    fn claim(&self, statement: &str, subject: &str) -> String {
        let mut p = self.providers();
        let input = ClaimInput {
            statement: statement.into(),
            subject: subject.into(),
            evidence: Vec::new(),
            recheck: None,
            note: None,
            by: None,
        };
        let out = engine::claim(&self.store, &mut p, &self.git, &[input], false, now())
            .unwrap_or_else(|e| panic!("{e:?}"));
        out.claims[0].id.to_string()
    }

    fn write(&self, path: &str, content: &str) {
        std::fs::write(self.root.join(path), content).unwrap();
    }

    /// Every file under `.hibi/` with its bytes.
    fn hibi_files(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![self.root.join(".hibi")];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    out.insert(e.path(), std::fs::read(e.path()).unwrap());
                }
            }
        }
        out
    }
}

/// The text of a triage or suggest question.
fn text_of(q: &Value) -> String {
    let i = &q["instructions"];
    i.get("text")
        .or_else(|| i.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Answers by content: retries 0.9 and `code` at 0.8; timeouts 0.8 and
/// `config` at 0.55 (below 0.6, so `unsure`); a heading 0.1; else 0.5.
fn by_content(_: &str, q: &Value, _: &Value) -> Value {
    let t = text_of(q);
    let (p, label, lp) = if t.contains("retries") {
        (0.9, "code", 0.8)
    } else if t.contains("Timeouts") {
        (0.8, "config", 0.55)
    } else if t.starts_with('#') {
        (0.1, "none", 0.9)
    } else {
        (0.5, "doc", 0.7)
    };
    match q["type"].as_str() {
        Some("choice") => choice(q, label, lp),
        _ => json!(p),
    }
}

fn ok_mock() -> Mock {
    Mock::answering(by_content)
}

fn err_code(r: Result<impl std::fmt::Debug, hibi::Error>) -> Code {
    r.unwrap_err().code
}

// ---------------------------------------------------------------------------
// R24 and the triage contract (14.3)
// ---------------------------------------------------------------------------

#[test]
fn r24_triage_sends_the_pinned_model_and_one_question_pair_per_unit() {
    let fx = fixture(&[]);
    let mock = ok_mock();
    let out = fx.triage(&fx.judge(&mock), false).unwrap();

    let reqs = mock.recorded();
    assert_eq!(reqs.len(), 1, "one resource, one request");
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/v1/systemone");
    assert_eq!(r.header("authorization"), Some("Bearer test-key"));
    assert_eq!(r.header("content-type"), Some("application/json"));
    assert_eq!(r.body["model"], "jev-1.14.0");
    assert_eq!(
        r.body["state"],
        json!({"task": "hibi-triage-v1", "document": "file:docs/guide.md"})
    );
    // Prose units: the heading, the sentence, the list item and the two
    // table rows (the separator row is not a unit). The fenced block is
    // skipped.
    let names = r.names();
    assert_eq!(names.len(), 2 * out.considered);
    for i in 0..out.considered {
        assert!(names.contains(&format!("q{i}.checkable")), "{names:?}");
        assert!(names.contains(&format!("q{i}.evidence")), "{names:?}");
    }
    let q = &r.body["questions"]["q1.checkable"];
    assert_eq!(q["type"], "noul");
    assert_eq!(q["instructions"]["ask"], questions::TRIAGE_CHECKABLE_ASK);
    assert_eq!(q["instructions"]["headings"], json!(["Retries"]));
    assert_eq!(
        q["instructions"]["text"],
        "The client retries a request three times."
    );
    let e = &r.body["questions"]["q1.evidence"];
    assert_eq!(e["type"], "choice");
    assert_eq!(
        e["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["code", "config", "doc", "external", "none"]
    );
    assert_eq!(
        *e,
        questions::triage_evidence(
            &["Retries".to_owned()],
            "The client retries a request three times."
        )
    );

    assert_eq!(out.model, "jev-1.14.0");
    let got: Vec<(&str, f64, Band, EvidenceLabel)> = out
        .items
        .iter()
        .map(|i| (i.unit.kind.as_str(), i.checkable, i.band, i.evidence))
        .collect();
    assert_eq!(
        got,
        [
            ("sentence", 0.9, Band::Likely, EvidenceLabel::Code),
            ("list-item", 0.8, Band::Likely, EvidenceLabel::Unsure),
            ("table-row", 0.5, Band::Unsure, EvidenceLabel::Doc),
            ("table-row", 0.5, Band::Unsure, EvidenceLabel::Doc),
            ("heading", 0.1, Band::Unlikely, EvidenceLabel::None),
        ]
    );
    // Equal probabilities keep document order.
    assert!(out.items[2].unit.line < out.items[3].unit.line);
    assert_eq!(out.considered, 5);
    assert_eq!(out.skipped, 1, "the fenced block is not prose");
    assert_eq!(
        out.items[0].anchor.as_ref().unwrap().loc.as_str(),
        "file:docs/guide.md"
    );
}

#[test]
fn triage_limit_cuts_items_but_not_considered() {
    let fx = fixture(&[]);
    let mock = ok_mock();
    let mut p = fx.providers();
    let q = TriageQuery {
        scopes: vec!["docs".into()],
        limit: 2,
    };
    let out = judge::triage(
        &fx.judge(&mock),
        &fx.store,
        &mut p,
        &fx.env(false),
        &q,
        None,
    )
    .unwrap();
    assert_eq!(out.items.len(), 2);
    assert_eq!(out.considered, 5);
    let q = TriageQuery {
        scopes: vec!["nope".into()],
        limit: 2,
    };
    let e = judge::triage(
        &fx.judge(&mock),
        &fx.store,
        &mut p,
        &fx.env(false),
        &q,
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, Code::UnknownScope);
}

#[test]
fn triage_skips_covered_units() {
    let fx = fixture(&[]);
    fx.claim(
        "Retries.",
        "file:docs/guide.md::The client retries a request three times.",
    );
    let mock = ok_mock();
    let out = fx.triage(&fx.judge(&mock), false).unwrap();
    assert!(out.items.iter().all(|i| i.checkable != 0.9));
    assert_eq!(out.considered, 4, "the claimed sentence is covered");
}

// ---------------------------------------------------------------------------
// R25 and the cache (14.6)
// ---------------------------------------------------------------------------

#[test]
fn r25_a_warm_cache_sends_no_request() {
    let fx = fixture(&[]);
    let mock = ok_mock();
    let judge = fx.judge(&mock);
    let first = fx.triage(&judge, true).unwrap();
    assert_eq!(mock.count(), 1);
    let second = fx.triage(&judge, true).unwrap();
    assert_eq!(mock.count(), 1, "R25: nothing sent with a warm cache");
    assert_eq!(first, second);
    let opened = mock.connections.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(opened, 1, "no connection either");
    // Without the cache every question is asked again.
    fx.triage(&judge, false).unwrap();
    assert_eq!(mock.count(), 2);
}

#[test]
fn an_edited_unit_is_asked_again_and_only_it() {
    let fx = fixture(&[]);
    let mock = ok_mock();
    let judge = fx.judge(&mock);
    fx.triage(&judge, true).unwrap();
    fx.write("docs/guide.md", &GUIDE.replace("three times", "four times"));
    fx.triage(&judge, true).unwrap();
    let reqs = mock.recorded();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[1].names(), ["q0.checkable", "q0.evidence"]);
    assert!(text_of(&reqs[1].body["questions"]["q0.checkable"]).contains("four times"));
}

#[test]
fn a_cached_answer_of_the_wrong_shape_is_a_miss() {
    let fx = fixture(&[]);
    let mock = ok_mock();
    let judge = fx.judge(&mock);
    let state = questions::triage_state("file:docs/guide.md");
    let q = questions::triage_checkable(
        &["Retries".to_owned()],
        "The client retries a request three times.",
    );
    let key = judge::cache_key(judge::MODEL, &state, &q).unwrap();
    fx.cache
        .put_judge(
            &key,
            &hibi::cache::JudgeEntry {
                model: judge::MODEL.into(),
                answer: r#"{"type":"choice","choice":"x"}"#.into(),
                at: NOW.into(),
            },
        )
        .unwrap();
    fx.triage(&judge, true).unwrap();
    assert_eq!(
        mock.recorded()[0].names().len(),
        10,
        "the bad entry is asked again"
    );
    // The good answer replaced it.
    let entry = fx.cache.judge(&key).unwrap().unwrap();
    assert_eq!(entry.at, NOW);
    assert_eq!(
        serde_json::from_str::<Value>(&entry.answer).unwrap()["noul"],
        0.9
    );
}

// ---------------------------------------------------------------------------
// R26 and the transport policy (14.7)
// ---------------------------------------------------------------------------

#[test]
fn r26_three_failures_fail_the_command_with_no_ranking() {
    let fx = fixture(&[]);
    let mock = Mock::start(Arc::new(|_, _| Reply::status(500)));
    let e = fx.triage(&fx.judge(&mock), true).unwrap_err();
    assert_eq!(e.code, Code::JudgeFailed);
    assert!(e.message.contains("500"), "{}", e.message);
    assert!(
        !e.message.contains(KEY),
        "the key never appears in a message"
    );
    assert_eq!(mock.count(), 3, "three attempts");
    assert_eq!(fx.sleeps(), [500, 1000]);
    // Nothing was cached: a good server is asked everything.
    let good = ok_mock();
    fx.triage(&fx.judge(&good), true).unwrap();
    assert_eq!(good.recorded()[0].names().len(), 10);
}

#[test]
fn every_failure_kind_is_judge_failed() {
    let fx = fixture(&[]);
    let mock = ok_mock();
    let judge = fx.judge(&mock);
    for status in [408, 429, 500, 502, 529, 599] {
        mock.set_script(Arc::new(move |_, _| Reply::status(status)));
        let before = mock.count();
        assert_eq!(err_code(fx.triage(&judge, false)), Code::JudgeFailed);
        assert_eq!(mock.count() - before, 3, "{status} is retried");
    }
    for status in [301, 400, 401, 403, 404, 413, 415, 422] {
        mock.set_script(Arc::new(move |_, _| Reply::status(status)));
        let before = mock.count();
        assert_eq!(err_code(fx.triage(&judge, false)), Code::JudgeFailed);
        assert_eq!(mock.count() - before, 1, "{status} is not retried");
    }
}

#[test]
fn backoff_on_429_and_529_with_retry_after() {
    let fx = fixture(&[]);
    let mock = Mock::start(Arc::new(|r, n| match n {
        0 => Reply::status(429).header("Retry-After", "2"),
        1 => Reply::status(529),
        _ => Reply::json(200, &mock::answers(&r.body, &by_content)),
    }));
    fx.triage(&fx.judge(&mock), false).unwrap();
    assert_eq!(
        fx.sleeps(),
        [2000, 1000],
        "Retry-After replaces the first delay"
    );

    let fx = fixture(&[]);
    let mock = Mock::start(Arc::new(|r, n| match n {
        0 => Reply::status(429).header("Retry-After", "30"),
        _ => Reply::json(200, &mock::answers(&r.body, &by_content)),
    }));
    fx.triage(&fx.judge(&mock), false).unwrap();
    assert_eq!(fx.sleeps(), [5000], "capped at 5 s");
}

#[test]
fn a_timeout_is_retried() {
    let fx = fixture(&[]);
    let mock = Mock::start(Arc::new(|r, n| {
        let reply = Reply::json(200, &mock::answers(&r.body, &by_content));
        if n == 0 {
            reply.after(Duration::from_millis(1500))
        } else {
            reply
        }
    }));
    let out = fx.triage(&fx.judge(&mock), false).unwrap();
    assert_eq!(out.considered, 5);
    assert_eq!(mock.count(), 2);
    assert_eq!(fx.sleeps(), [500]);

    let mock = Mock::start(Arc::new(|_, _| {
        Reply::status(200).after(Duration::from_millis(1500))
    }));
    let e = fx.triage(&fx.judge(&mock), false).unwrap_err();
    assert_eq!(e.code, Code::JudgeFailed);
    assert_eq!(mock.count(), 3);
}

#[test]
fn a_refused_connection_is_judge_failed_after_three_attempts() {
    let fx = fixture(&[]);
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let judge = Judge::new(
        Some(KEY),
        Some(&format!("http://127.0.0.1:{port}/v1/systemone")),
    )
    .unwrap()
    .with_policy(Policy {
        sleep: {
            let s = fx.sleeps.clone();
            Arc::new(move |d| s.lock().unwrap().push(d))
        },
        ..Policy::default()
    });
    assert_eq!(err_code(fx.triage(&judge, false)), Code::JudgeFailed);
    assert_eq!(fx.sleeps(), [500, 1000]);
}

#[test]
fn malformed_and_partial_answers_fail_without_retry_or_cache() {
    let fx = fixture(&[]);
    let mock = ok_mock();
    let judge = fx.judge(&mock);
    type Mangle = fn(&mut Value);
    let cases: [(&str, Mangle); 7] = [
        ("not json", |v| *v = json!("<html>")),
        ("no answers", |v| {
            v.as_object_mut().unwrap().remove("answers");
        }),
        ("a missing answer", |v| {
            v["answers"].as_object_mut().unwrap().remove("q2.evidence");
        }),
        ("a choice where a noul was asked", |v| {
            v["answers"]["q0.checkable"] = json!({"type": "choice", "choice": "code", "confidence": 1, "probabilities": {"code": 1}});
        }),
        ("a probability above 1", |v| {
            v["answers"]["q0.checkable"]["noul"] = json!(1.2)
        }),
        ("an option that was not offered", |v| {
            v["answers"]["q0.evidence"]["choice"] = json!("magic");
        }),
        ("another model", |v| v["model"] = json!("jev-1.13.0")),
    ];
    for (what, mangle) in cases {
        mock.set_script(Arc::new(move |r, _| {
            let mut v = mock::answers(&r.body, &by_content);
            mangle(&mut v);
            let body = if v.is_string() {
                v.as_str().unwrap().to_owned()
            } else {
                v.to_string()
            };
            Reply {
                body,
                ..Reply::status(200)
            }
        }));
        let before = mock.count();
        let e = fx.triage(&judge, true).unwrap_err();
        assert_eq!(e.code, Code::JudgeFailed, "{what}");
        assert_eq!(mock.count() - before, 1, "{what} is not retried");
    }
    // Nothing of the failed requests was cached.
    mock.set_script(Arc::new(|r, _| {
        Reply::json(200, &mock::answers(&r.body, &by_content))
    }));
    let before = mock.count();
    fx.triage(&judge, true).unwrap();
    assert_eq!(mock.recorded()[before].names().len(), 10);
}

#[test]
fn a_failed_request_keeps_the_answers_of_successful_ones() {
    let fx = fixture(&[("docs/other.md", "Other text is here.\n")]);
    let mock = Mock::start(Arc::new(|r, _| {
        if r.body["state"]["document"] == "file:docs/other.md" {
            Reply::status(400)
        } else {
            Reply::json(200, &mock::answers(&r.body, &by_content))
        }
    }));
    let judge = fx.judge(&mock);
    // One at a time, so that the failure cannot stop the other request.
    let policy = Policy {
        concurrency: 1,
        ..judge.policy().clone()
    };
    let judge = judge.with_policy(policy);
    assert_eq!(err_code(fx.triage(&judge, true)), Code::JudgeFailed);
    assert_eq!(mock.count(), 2);
    let good = ok_mock();
    fx.triage(&fx.judge(&good), true).unwrap();
    let reqs = good.recorded();
    assert_eq!(reqs.len(), 1, "only the failed resource is asked again");
    assert_eq!(reqs[0].body["state"]["document"], "file:docs/other.md");
}

#[test]
fn at_most_six_requests_in_flight() {
    let docs: Vec<(String, String)> = (0..14)
        .map(|i| {
            (
                format!("docs/d{i:02}.md"),
                format!("Document {i} says hello.\n"),
            )
        })
        .collect();
    let refs: Vec<(&str, &str)> = docs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let fx = fixture(&refs);
    let mock = Mock::start(Arc::new(|r, _| {
        Reply::json(200, &mock::answers(&r.body, &by_content)).after(Duration::from_millis(150))
    }));
    let out = fx.triage(&fx.judge(&mock), false).unwrap();
    assert_eq!(mock.count(), 15, "one request per resource");
    assert_eq!(out.considered, 19);
    assert_eq!(
        mock.max_in_flight.load(std::sync::atomic::Ordering::SeqCst),
        6
    );
}

#[test]
fn requests_are_split_by_question_count_and_bytes() {
    let many: String = (0..120)
        .map(|i| format!("Sentence number {i} is here. "))
        .collect();
    let long: String = (0..30)
        .map(|i| format!("{}{i}.\n\n", "Long words go on and on ".repeat(120)))
        .collect();
    let fx = fixture(&[("docs/many.md", &many), ("docs/long.md", &long)]);
    let mock = ok_mock();
    let out = fx.triage(&fx.judge(&mock), false).unwrap();
    let mut sizes: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for r in mock.recorded() {
        let bytes = serde_json::to_vec(&r.body["questions"]).unwrap().len();
        assert!(r.names().len() <= judge::MAX_QUESTIONS_PER_REQUEST);
        assert!(bytes <= judge::MAX_QUESTION_BYTES_PER_REQUEST, "{bytes}");
        sizes
            .entry(r.body["state"]["document"].as_str().unwrap().to_owned())
            .or_default()
            .push((r.names().len(), bytes));
    }
    // Requests arrive in any order; sort them by their first sentence.
    let mut many: Vec<(u64, usize)> = mock
        .recorded()
        .iter()
        .filter(|r| r.body["state"]["document"] == "file:docs/many.md")
        .map(|r| {
            let first = text_of(&r.body["questions"]["q0.checkable"]);
            let n = first.split(' ').nth(2).unwrap().parse().unwrap();
            (n, r.names().len())
        })
        .collect();
    many.sort_unstable();
    assert_eq!(
        many,
        [(0, 100), (50, 100), (100, 40)],
        "50 units per request, input order"
    );
    let long = &sizes["file:docs/long.md"];
    assert!(long.len() >= 2, "split by bytes: {long:?}");
    assert_eq!(long.iter().map(|s| s.0).sum::<usize>(), 60);
    assert!(
        long.iter().all(|s| s.0 % 2 == 0),
        "a unit's two questions stay together"
    );
    assert_eq!(out.considered, 120 + 30 + 5);
}

#[test]
fn item_text_is_cut_to_4000_characters() {
    let big = format!("{}.\n", "é".repeat(5000));
    let fx = fixture(&[("docs/big.md", &big)]);
    let mock = ok_mock();
    fx.triage(&fx.judge(&mock), false).unwrap();
    let r = mock
        .recorded()
        .into_iter()
        .find(|r| r.body["state"]["document"] == "file:docs/big.md")
        .unwrap();
    let t = text_of(&r.body["questions"]["q0.checkable"]);
    assert_eq!(t.chars().count(), 4000);
}

// ---------------------------------------------------------------------------
// The key never reaches an error, the output or the cache
// ---------------------------------------------------------------------------

/// A key that does not occur by chance in a message.
const SECRET: &str = "sk-test-9f3k2m7q4x8tbh2d5wqa";

const LATER: &str = "Run the command again later.";
const CHECK_KEY: &str = "Check TYPESAFE_API_KEY: the service refused it.";
const CHECK_URL: &str = "Check HIBI_JEV_URL: the endpoint did not answer as the Jev service does.";

fn report_bug(status: u16) -> String {
    format!(
        "This is a bug in hibi: the service rejected the request. Report it at https://github.com/npupko/hibi/issues with the status {status}."
    )
}

/// The client of [`Fx::judge`] with another key.
fn judge_with_key(fx: &Fx, mock: &Mock, key: &str) -> Judge {
    Judge::new(Some(key), Some(&mock.url))
        .unwrap()
        .with_policy(fx.judge(mock).policy().clone())
}

/// Neither the message, the remediation nor the error as the CLI prints it
/// (JSON) holds the key, a leading part of it, or the header scheme.
fn assert_no_key(e: &hibi::Error, what: &str) {
    let json = serde_json::to_string(e).unwrap();
    let remediation = e.remediation.clone().unwrap_or_default();
    for text in [&e.message, &remediation, &json, &e.to_string()] {
        assert!(!text.contains(SECRET), "{what}: the key is in {text:?}");
        assert!(
            !text.contains(&SECRET[..12]),
            "{what}: part of the key is in {text:?}"
        );
        assert!(
            !text.to_ascii_lowercase().contains("bearer"),
            "{what}: the header value is in {text:?}"
        );
    }
}

/// What a proxy that echoes the request headers answers.
fn echo(r: &mock::Recorded) -> String {
    json!({"detail": "refused", "headers": {"authorization": r.header("authorization")}})
        .to_string()
}

#[test]
fn key_leak_an_echoed_authorization_header_is_redacted_from_errors() {
    let fx = fixture(&[]);
    let mock = ok_mock();
    let judge = judge_with_key(&fx, &mock, SECRET);
    for status in [401, 400, 500] {
        mock.set_script(Arc::new(move |r, _| Reply {
            body: echo(r),
            ..Reply::status(status)
        }));
        let e = fx.triage(&judge, true).unwrap_err();
        let what = format!("status {status}");
        assert_eq!(
            mock.recorded().last().unwrap().header("authorization"),
            Some(format!("Bearer {SECRET}").as_str()),
            "the server did receive the key"
        );
        assert_eq!(e.code, Code::JudgeFailed, "{what}");
        assert!(e.message.contains(&status.to_string()), "{}", e.message);
        assert_no_key(&e, &what);
        assert!(e.message.contains("[redacted]"), "{}", e.message);
        assert!(e.message.contains("refused"), "{}", e.message);
    }
}

/// A key with `/`, which JSON may write as `\/`.
const SLASH_SECRET: &str = "sk-FAKE/CANARY/9f3k2m7q";

#[test]
fn key_leak_an_echoed_header_with_escaped_slashes_is_redacted_from_errors() {
    let fx = fixture(&[]);
    // The body is JSON in which every `/` is written as `\/`.
    let mock = Mock::start(Arc::new(|r, _| Reply {
        body: echo(r).replace('/', "\\/"),
        ..Reply::status(401)
    }));
    let escaped = SLASH_SECRET.replace('/', "\\/");
    let e = fx
        .triage(&judge_with_key(&fx, &mock, SLASH_SECRET), false)
        .unwrap_err();
    let recorded = mock.recorded();
    let sent = recorded.last().unwrap();
    assert_eq!(
        sent.header("authorization"),
        Some(format!("Bearer {SLASH_SECRET}").as_str()),
        "the server did receive the key"
    );
    let body = echo(sent).replace('/', "\\/");
    assert!(body.contains(&escaped), "the body holds the key: {body}");
    assert!(!body.contains(SLASH_SECRET), "only with `\\/`: {body}");
    assert_eq!(e.code, Code::JudgeFailed);
    assert!(e.message.contains("401"), "{}", e.message);
    assert_eq!(e.remediation.as_deref(), Some(CHECK_KEY));
    let json = serde_json::to_string(&e).unwrap();
    let remediation = e.remediation.clone().unwrap_or_default();
    // The parts between the slashes are looked for as well: the JSON form
    // of the error writes the `\` of an escaped slash as `\\`.
    let mut parts = vec![SLASH_SECRET, escaped.as_str(), &SLASH_SECRET[..10]];
    parts.extend(SLASH_SECRET.split('/'));
    for text in [&e.message, &remediation, &e.to_string(), &json] {
        for part in &parts {
            assert!(!text.contains(part), "{part:?} of the key is in {text:?}");
        }
        assert!(
            !text.to_ascii_lowercase().contains("bearer"),
            "the header value is in {text:?}"
        );
    }
    assert!(e.message.contains("[redacted]"), "{}", e.message);
    assert!(e.message.contains("refused"), "{}", e.message);
}

#[test]
fn key_leak_a_cut_body_leaves_no_part_of_the_key() {
    let fx = fixture(&[]);
    // The key starts 12 characters before the 300 character cut.
    let mock = Mock::start(Arc::new(|r, _| Reply {
        body: format!(
            "{}{}",
            "x".repeat(281),
            r.header("authorization").unwrap_or_default()
        ),
        ..Reply::status(400)
    }));
    let e = fx
        .triage(&judge_with_key(&fx, &mock, SECRET), false)
        .unwrap_err();
    assert!(e.message.contains("400"), "{}", e.message);
    assert_no_key(&e, "a cut body");
}

#[test]
fn key_leak_a_200_that_is_not_json_and_holds_the_key_is_redacted() {
    let fx = fixture(&[]);
    let mock = Mock::start(Arc::new(|r, _| Reply {
        body: format!(
            "<html>authorization: {}</html>",
            r.header("authorization").unwrap_or_default()
        ),
        ..Reply::status(200)
    }));
    let e = fx
        .triage(&judge_with_key(&fx, &mock, SECRET), true)
        .unwrap_err();
    assert_eq!(e.code, Code::JudgeFailed);
    assert_eq!(mock.count(), 1);
    assert!(e.message.contains("not JSON"), "{}", e.message);
    assert_no_key(&e, "a 200 that is not JSON");
}

#[test]
fn key_leak_a_200_that_names_the_key_as_its_model_is_redacted() {
    let fx = fixture(&[]);
    let mock = Mock::start(Arc::new(|r, _| {
        let mut v = mock::answers(&r.body, &by_content);
        v["model"] = json!(r.header("authorization"));
        Reply::json(200, &v)
    }));
    let e = fx
        .triage(&judge_with_key(&fx, &mock, SECRET), true)
        .unwrap_err();
    assert_eq!(e.code, Code::JudgeFailed);
    assert_eq!(mock.count(), 1);
    assert!(e.message.contains("model"), "{}", e.message);
    assert_no_key(&e, "a 200 from another model");
    assert!(e.message.contains("[redacted]"), "{}", e.message);
}

#[test]
fn key_leak_an_echoed_key_in_an_answer_is_not_cached() {
    let fx = fixture(&[]);
    // A usable response whose answers carry an extra member with the header.
    let mock = Mock::start(Arc::new(|r, _| {
        let mut v = mock::answers(&r.body, &by_content);
        for (_, a) in v["answers"].as_object_mut().unwrap() {
            a["echo"] = json!(r.header("authorization"));
        }
        Reply::json(200, &v)
    }));
    let judge = judge_with_key(&fx, &mock, SECRET);
    let out = fx.triage(&judge, true).unwrap();
    assert_eq!(out.considered, 5);
    assert!(!serde_json::to_string(&out).unwrap().contains(SECRET));
    // The answers were cached.
    fx.triage(&judge, true).unwrap();
    assert_eq!(mock.count(), 1);
    let key = judge::cache_key(
        judge::MODEL,
        &questions::triage_state("file:docs/guide.md"),
        &questions::triage_checkable(
            &["Retries".to_owned()],
            "The client retries a request three times.",
        ),
    )
    .unwrap();
    let entry = fx.cache.judge(&key).unwrap().unwrap();
    assert!(
        !entry.answer.contains(SECRET),
        "the key is in the cached answer {}",
        entry.answer
    );
    // The cache holds the typed fields only: the extra member is gone.
    assert!(!entry.answer.contains("echo"), "{}", entry.answer);
    assert!(entry.answer.contains("\"noul\""), "{}", entry.answer);
    // No file of the cache database holds the key.
    for e in std::fs::read_dir(fx._cache_dir.path()).unwrap() {
        let path = e.unwrap().path();
        let bytes = std::fs::read(&path).unwrap();
        let found = bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes());
        assert!(!found, "the key is in {}", path.display());
    }
}

// ---------------------------------------------------------------------------
// The remediation of judge_failed by cause, and what is retried (14.7)
// ---------------------------------------------------------------------------

/// One failing triage against `mock`: the error and the requests it took.
fn fail_once(fx: &Fx, mock: &Mock, judge: &Judge) -> (hibi::Error, usize) {
    let before = mock.count();
    let e = fx.triage(judge, false).unwrap_err();
    assert_eq!(e.code, Code::JudgeFailed);
    (e, mock.count() - before)
}

fn remediation_for_statuses(
    statuses: &[u16],
    requests: usize,
    remediation: &dyn Fn(u16) -> String,
) {
    let fx = fixture(&[]);
    let mock = ok_mock();
    let judge = fx.judge(&mock);
    for &status in statuses {
        mock.set_script(Arc::new(move |_, _| Reply::status(status)));
        let (e, sent) = fail_once(&fx, &mock, &judge);
        assert_eq!(
            mock.recorded().last().unwrap().method,
            "POST",
            "status {status}"
        );
        assert_eq!(sent, requests, "requests for status {status}");
        assert!(e.message.contains(&status.to_string()), "{}", e.message);
        assert_eq!(
            e.remediation.as_deref(),
            Some(remediation(status).as_str()),
            "status {status}"
        );
    }
}

#[test]
fn remediation_authentication_names_the_key_variable() {
    remediation_for_statuses(&[401, 403], 1, &|_| CHECK_KEY.to_owned());
}

#[test]
fn remediation_a_rejected_request_is_a_bug_to_report_with_the_status() {
    remediation_for_statuses(&[400, 413, 415, 422], 1, &report_bug);
}

#[test]
fn remediation_rate_limit_and_server_errors_say_run_again_later() {
    remediation_for_statuses(&[408, 429, 500, 503, 529], 3, &|_| LATER.to_owned());
}

#[test]
fn remediation_a_redirect_or_a_missing_path_names_the_endpoint_variable() {
    remediation_for_statuses(&[301, 302, 307, 308, 404], 1, &|_| CHECK_URL.to_owned());
}

#[test]
fn remediation_an_answer_of_another_shape_names_the_endpoint_variable() {
    let fx = fixture(&[]);
    let mock = ok_mock();
    let judge = fx.judge(&mock);
    type Mangle = fn(&mut Value);
    let cases: [(&str, Mangle); 3] = [
        ("another model", |v| v["model"] = json!("jev-1.13.0")),
        ("no answers", |v| {
            v.as_object_mut().unwrap().remove("answers");
        }),
        ("not json", |v| *v = json!("<html>")),
    ];
    for (what, mangle) in cases {
        mock.set_script(Arc::new(move |r, _| {
            let mut v = mock::answers(&r.body, &by_content);
            mangle(&mut v);
            let body = match v.as_str() {
                Some(s) => s.to_owned(),
                None => v.to_string(),
            };
            Reply {
                body,
                ..Reply::status(200)
            }
        }));
        let (e, sent) = fail_once(&fx, &mock, &judge);
        assert_eq!(sent, 1, "{what}");
        assert_eq!(e.remediation.as_deref(), Some(CHECK_URL), "{what}");
    }
}

#[test]
fn remediation_connection_errors_and_timeouts_say_run_again_later() {
    let fx = fixture(&[]);
    // A timeout: three attempts.
    let mock = Mock::start(Arc::new(|_, _| {
        Reply::status(200).after(Duration::from_millis(1500))
    }));
    let (e, sent) = fail_once(&fx, &mock, &fx.judge(&mock));
    assert_eq!(sent, 3);
    // No HTTP answer, and HIBI_JEV_URL is set (the mock's address): the
    // endpoint is named as a possible cause.
    let fix = e.remediation.as_deref().unwrap();
    assert!(
        fix.starts_with("Run the command again later. If it keeps failing, check HIBI_JEV_URL: it is set to \"http://127.0.0.1:"),
        "{fix}"
    );
    assert_eq!(fx.sleeps(), [500, 1000]);

    // A refused connection: three attempts, seen as two backoffs.
    let fx = fixture(&[]);
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let sleeps = fx.sleeps.clone();
    let judge = Judge::new(
        Some(KEY),
        Some(&format!("http://127.0.0.1:{port}/v1/systemone")),
    )
    .unwrap()
    .with_policy(Policy {
        sleep: Arc::new(move |d| sleeps.lock().unwrap().push(d)),
        ..Policy::default()
    });
    let e = fx.triage(&judge, false).unwrap_err();
    assert_eq!(e.code, Code::JudgeFailed);
    assert_eq!(
        e.remediation.as_deref(),
        Some(
            format!(
                "Run the command again later. If it keeps failing, check HIBI_JEV_URL: it is set to \"http://127.0.0.1:{port}/v1/systemone\", not the default endpoint."
            )
            .as_str()
        )
    );
    assert_eq!(fx.sleeps(), [500, 1000]);
}

/// A key the HTTP library cannot send: nothing is sent, nothing is waited
/// for, and the error names the variable and the fault, not the key.
fn a_key_that_cannot_be_sent(key: &str, fault: &str) {
    let fx = fixture(&[]);
    let mock = ok_mock();
    // A real sleep of 5 s per backoff, so that a retry would show in the
    // elapsed time.
    let sleeps = fx.sleeps.clone();
    let judge = Judge::new(Some(key), Some(&mock.url))
        .unwrap()
        .with_policy(Policy {
            backoff_initial: Duration::from_secs(5),
            sleep: Arc::new(move |d| {
                sleeps.lock().unwrap().push(d);
                std::thread::sleep(d);
            }),
            ..Policy::default()
        });
    let start = std::time::Instant::now();
    let e = fx.triage(&judge, false).unwrap_err();
    let took = start.elapsed();
    assert_eq!(e.code, Code::JudgeFailed);
    assert_eq!(mock.count(), 0, "no request");
    assert_eq!(fx.sleeps(), [] as [u128; 0], "no backoff");
    assert!(took < Duration::from_secs(3), "took {took:?}");
    assert_eq!(
        mock.connections.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no connection"
    );
    assert!(e.message.contains("TYPESAFE_API_KEY"), "{}", e.message);
    assert!(e.message.contains(fault), "{}", e.message);
    assert!(!e.message.contains("attempts"), "{}", e.message);
    let remediation = e.remediation.clone().unwrap_or_default();
    assert!(remediation.contains("TYPESAFE_API_KEY"), "{remediation}");
    let json = serde_json::to_string(&e).unwrap();
    for part in key.split(['\n', '\r']).filter(|p| !p.is_empty()) {
        assert!(!json.contains(part), "the key is in {json}");
    }
}

#[test]
fn a_key_with_a_non_ascii_character_is_not_sent_and_not_retried() {
    a_key_that_cannot_be_sent("sk-test-9f3k2m7qé4x8tbh2d5wqa", "not printable ASCII");
}

#[test]
fn a_key_with_a_line_break_is_not_sent_and_not_retried() {
    a_key_that_cannot_be_sent("sk-test-9f3k2m7q\n4x8tbh2d5wqa", "control character");
    a_key_that_cannot_be_sent("sk-test-9f3k2m7q4x8tbh2d5wqa\r\n", "control character");
}

// ---------------------------------------------------------------------------
// R27, R22, R23, R64
// ---------------------------------------------------------------------------

#[test]
fn r27_no_judge_command_changes_hibi() {
    let fx = fixture(&[]);
    fx.claim("Retries.", "file:src/lib.rs#retries");
    fx.write("src/lib.rs", &LIB.replace("    3\n", "    4\n"));
    let before = fx.hibi_files();
    let mock = ok_mock();
    let judge = fx.judge(&mock);
    fx.triage(&judge, true).unwrap();
    fx.suggest(&judge, &["file:src/**"], 10).unwrap();
    let out = fx.order(&judge, &[]).unwrap();
    assert_eq!(out.claims.len(), 1);
    mock.set_script(Arc::new(|_, _| Reply::status(500)));
    fx.triage(&judge, false).unwrap_err();
    fx.order(&judge, &[]).unwrap_err();
    assert_eq!(fx.hibi_files(), before, "R27");
}

#[test]
fn r23_without_the_key_nothing_is_contacted() {
    let mock = ok_mock();
    for (key, what) in [(None, "is not set"), (Some(""), "is empty")] {
        let e = Judge::new(key, Some(&mock.url)).unwrap_err();
        assert_eq!(e.code, Code::JudgeUnavailable);
        assert_eq!(
            e.message,
            format!("TYPESAFE_API_KEY {what}; the judge commands need it")
        );
        assert!(e.remediation.is_some());
    }
    assert_eq!(
        mock.connections.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

/// R22 at the library level: `ureq` is used only by the judge module, so
/// no other command can open a network connection. The command-level check
/// belongs to the CLI tests.
#[test]
fn r22_only_the_judge_module_uses_the_network() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src.clone()];
    let mut users = Vec::new();
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                if text.contains("ureq") || text.contains("TcpStream") || text.contains("std::net")
                {
                    users.push(p.strip_prefix(&src).unwrap().to_string_lossy().into_owned());
                }
            }
        }
    }
    users.sort();
    assert!(
        users.iter().all(|u| u.starts_with("judge")),
        "network code outside src/judge: {users:?}"
    );
}

#[test]
fn r64_without_usable_git_nothing_is_contacted() {
    let mut fx = fixture(&[]);
    let mock = ok_mock();
    let judge = fx.judge(&mock);
    let missing = GitStatus::Missing {
        message: "git could not be started".into(),
    };
    let old = GitStatus::from_version_output("git", "git version 2.30.1");
    for (git, code) in [(missing, Code::NotAGitRepository), (old, Code::GitTooOld)] {
        fx.git = git;
        assert_eq!(err_code(fx.triage(&judge, false)), code);
        assert_eq!(err_code(fx.suggest(&judge, &["file:**"], 10)), code);
        assert_eq!(err_code(fx.order(&judge, &[])), code);
    }
    // A root that is not a git top level.
    fx.git = GitStatus::probe();
    let sub = fx.root.join("docs");
    let env = JudgeEnv {
        root: &sub,
        ..fx.env(false)
    };
    let mut p = fx.providers();
    let e = judge::triage(
        &judge,
        &fx.store,
        &mut p,
        &env,
        &TriageQuery::default(),
        None,
    )
    .unwrap_err();
    assert_eq!(e.code, Code::NotAGitRepository);
    assert_eq!(
        mock.connections.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

// ---------------------------------------------------------------------------
// R61
// ---------------------------------------------------------------------------

#[test]
fn r61_ignored_text_is_never_sent() {
    let fx = fixture(&[
        ("docs/ignored.md", "Ignored hunter2 sentence.\n"),
        ("notes.md", "The token is hunter2.\n"),
    ]);
    fx.write(".gitignore", "secret.md\nsrc/secret.rs\ndocs/ignored.md\n");
    // A file that git ignores cannot be claimed at all.
    let mut p = fx.providers();
    let refused = engine::claim(
        &fx.store,
        &mut p,
        &fx.git,
        &[ClaimInput {
            statement: "The token is set.".into(),
            subject: "file:secret.md::The token is hunter2.".into(),
            evidence: Vec::new(),
            recheck: None,
            note: None,
            by: None,
        }],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(refused[0].code, hibi::Code::SideMissing);
    // A claim made while its file was in git's file list; the file is
    // ignored afterwards, and its text changes.
    let secret = fx.claim("The token is set.", "file:notes.md::The token is hunter2.");
    fx.write(
        ".gitignore",
        "secret.md\nsrc/secret.rs\ndocs/ignored.md\nnotes.md\n",
    );
    fx.write("notes.md", "The token is hunter2 now.\n");
    let open = fx.claim("Retries.", "file:src/lib.rs#retries");
    fx.write("src/lib.rs", &LIB.replace("    3\n", "    4\n"));

    let mock = ok_mock();
    let judge = fx.judge(&mock);
    let t = fx.triage(&judge, false).unwrap();
    assert!(t.items.iter().all(|i| !i.loc.as_str().contains("ignored")));
    let s = fx.suggest(&judge, &["file:**"], 0).unwrap();
    assert!(
        s.candidates
            .iter()
            .all(|c| !c.loc.as_str().contains("secret"))
    );
    assert!(s.considered >= 3);
    let o = fx.order(&judge, &[]).unwrap();
    let ids: Vec<String> = o.claims.iter().map(|c| c.id.to_string()).collect();
    assert_eq!(
        ids,
        [secret.clone(), open],
        "the claim on an ignored file is not sent and comes first"
    );
    assert_eq!(mock.count(), 3);
    assert!(!mock.all_bodies().contains("hunter2"), "R61");
    // For contrast: the text of the claim on a listed file was sent.
    assert!(mock.all_bodies().contains("pub fn retries() -> u32"));
    // The stored claim on the ignored file reads as missing, and no view
    // of it carries the file's text.
    let listed = o.claims.iter().find(|c| c.id.as_str() == secret).unwrap();
    assert_eq!(listed.sides[0].state, hibi::engine::SideState::Missing);
    assert!(!serde_json::to_string(&o).unwrap().contains("hunter2 now"));
}

// ---------------------------------------------------------------------------
// status --judge-order (14.5)
// ---------------------------------------------------------------------------

#[test]
fn judge_order_ranks_stale_claims_most_likely_broken_first() {
    // Sides that are `changed` have text to send. A function anchored by
    // its symbol is `changed` when its body is edited; an edited sentence
    // is a missing side (section 9.2) and is not sent.
    let source = |changed: u32| -> String {
        ["alpha", "beta", "gamma"]
            .iter()
            .map(|f| format!("pub fn {f}() -> u32 {{\n    {changed}\n}}\n\n"))
            .chain(["pub fn delta() -> u32 {\n    1\n}\n".to_owned()])
            .collect()
    };
    let before = source(1);
    let fx = fixture(&[("src/order.rs", before.as_str())]);
    let mut ids = BTreeMap::new();
    for w in ["Alpha", "Beta", "Gamma", "Delta"] {
        ids.insert(
            w,
            fx.claim(
                &format!("{w} is true."),
                &format!("file:src/order.rs#{}", w.to_lowercase()),
            ),
        );
    }
    // Alpha, Beta and Gamma go stale; Delta stays fresh.
    fx.write("src/order.rs", &source(2));
    let p_of = |statement: &str| match statement {
        s if s.starts_with("Alpha") => 0.6,
        s if s.starts_with("Beta") => 0.2,
        _ => 0.9,
    };
    let mock =
        Mock::answering(move |_, _, state| json!(p_of(state["statement"].as_str().unwrap())));
    let judge = fx.judge(&mock);

    let out = fx
        .order(&judge, &[ClaimState::Stale, ClaimState::Fresh])
        .unwrap();
    let got: Vec<&str> = out.claims.iter().map(|c| c.statement.as_str()).collect();
    assert_eq!(
        got,
        [
            "Beta is true.",
            "Alpha is true.",
            "Gamma is true.",
            "Delta is true."
        ]
    );
    assert_eq!(out.summary.stale, 3);
    let reqs = mock.recorded();
    assert_eq!(reqs.len(), 3, "one request per stale claim");
    for r in &reqs {
        assert_eq!(r.names(), ["q0"]);
        assert_eq!(r.body["state"]["task"], "hibi-order-v1");
        let q = &r.body["questions"]["q0"];
        assert_eq!(q["instructions"]["ask"], questions::ORDER_ASK);
        let w = r.body["state"]["statement"]
            .as_str()
            .unwrap()
            .split(' ')
            .next()
            .unwrap();
        assert_eq!(
            q["instructions"]["content"],
            json!([format!(
                "pub fn {}() -> u32 {{\n    2\n}}",
                w.to_lowercase()
            )])
        );
    }
    // The answer is not printed: the output equals plain status, reordered.
    let mut p = fx.providers();
    let q = StatusQuery {
        states: vec![ClaimState::Stale, ClaimState::Fresh],
        ..StatusQuery::default()
    };
    let mut plain = engine::status(&fx.store, &mut p, &fx.git, &q, None, now()).unwrap();
    let mut ordered = out.clone();
    plain.claims.sort_by(|a, b| a.id.cmp(&b.id));
    ordered.claims.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(plain, ordered);
    let text = serde_json::to_string(&out).unwrap();
    assert!(!text.contains("0.6") && !text.contains("0.2"));
    let _ = ids;
}

#[test]
fn judge_order_with_nothing_stale_sends_nothing() {
    let fx = fixture(&[]);
    fx.claim(
        "Retries.",
        "file:docs/guide.md::The client retries a request three times.",
    );
    let mock = ok_mock();
    let out = fx.order(&fx.judge(&mock), &[]).unwrap();
    assert!(out.claims.is_empty());
    assert_eq!(
        mock.connections.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

// ---------------------------------------------------------------------------
// suggest-evidence (14.4)
// ---------------------------------------------------------------------------

#[test]
fn suggest_evidence_ranks_candidates_with_a_none_fits_signal() {
    let fx = fixture(&[]);
    let mock = Mock::answering(|_, q, _| {
        let code = text_of(q);
        json!(if code.contains("retries") {
            0.85
        } else if code.contains("Client") {
            0.4
        } else {
            0.1
        })
    });
    let judge = fx.judge(&mock);
    let out = fx.suggest(&judge, &["file:src/**"], 10).unwrap();
    let got: Vec<(&str, f64, bool)> = out
        .candidates
        .iter()
        .map(|c| (c.unit.label.as_deref().unwrap_or(&c.unit.key), c.p, c.fits))
        .collect();
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!((got[0].1, got[0].2), (0.85, true));
    assert_eq!((got[1].1, got[1].2), (0.4, false));
    assert!(!out.none_fits);
    assert_eq!(out.considered, 3);
    assert_eq!(out.statement, "The client retries three times.");

    let r = &mock.recorded()[0];
    assert_eq!(
        r.body["state"],
        json!({"task": "hibi-suggest-evidence-v1", "statement": "The client retries three times."})
    );
    assert_eq!(r.names(), ["q0", "q1", "q2"]);
    let q = &r.body["questions"]["q0"];
    assert_eq!(q["type"], "noul");
    assert_eq!(q["instructions"]["ask"], questions::SUGGEST_ASK);
    assert!(
        q["instructions"]["where"]
            .as_str()
            .unwrap()
            .starts_with("file:src/lib.rs ")
    );

    let out = fx.suggest(&judge, &["file:src/**"], 1).unwrap();
    assert_eq!((out.candidates.len(), out.considered), (1, 3));

    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["noneFits"], false);
    assert!(json["candidates"][0]["anchor"]["loc"].is_string());

    mock.set_script(Arc::new(|r, _| {
        Reply::json(200, &mock::answers(&r.body, &|_, _, _| json!(0.69)))
    }));
    let out = fx.suggest(&judge, &["file:src/**"], 10).unwrap();
    assert!(out.none_fits, "no candidate reaches 0.7");
}

#[test]
fn suggest_evidence_argument_errors_contact_nothing() {
    let many: String = (0..10_001).map(|i| format!("fn f{i}() {{}}\n")).collect();
    let fx = fixture(&[("src/many.rs", &many)]);
    let mock = ok_mock();
    let judge = fx.judge(&mock);
    assert_eq!(
        err_code(fx.suggest(&judge, &["file:src/**"], 10)),
        Code::TooManyCandidates
    );
    assert_eq!(
        err_code(fx.suggest(&judge, &["arch:model"], 10)),
        Code::InvalidArgument
    );
    assert_eq!(err_code(fx.suggest(&judge, &[], 10)), Code::InvalidArgument);
    assert_eq!(
        err_code(fx.suggest(&judge, &["repo:api//**"], 10)),
        Code::InvalidArgument
    );
    assert_eq!(
        mock.connections.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let out = fx.suggest(&judge, &["file:docs/**"], 10).unwrap();
    assert_eq!(
        (out.considered, out.none_fits),
        (0, true),
        "no code units in docs"
    );
}

/// A key that occurs inside an otherwise valid response does not break the
/// request: the response is parsed as it was written, and nothing stored or
/// returned contains the key.
#[test]
fn key_leak_a_key_that_occurs_in_a_valid_response_does_not_break_it() {
    // `answers` is a member name of every valid response.
    const WORD: &str = "answers";
    let fx = fixture(&[]);
    let mock = Mock::start(Arc::new(|r, _| {
        let mut v = mock::answers(&r.body, &by_content);
        v["note"] = json!(format!("your key is {WORD}"));
        for (_, a) in v["answers"].as_object_mut().unwrap() {
            a["echo"] = json!(r.header("authorization"));
        }
        Reply::json(200, &v)
    }));
    let judge = judge_with_key(&fx, &mock, WORD);
    let out = fx.triage(&judge, true).unwrap();
    assert_eq!(out.considered, 5);
    assert!(!serde_json::to_string(&out).unwrap().contains(WORD));
    // Cached, and read back from the cache without a request.
    let again = fx.triage(&judge, true).unwrap();
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        serde_json::to_value(&out).unwrap()
    );
    assert_eq!(mock.count(), 1);
    for e in std::fs::read_dir(fx._cache_dir.path()).unwrap() {
        let path = e.unwrap().path();
        let bytes = std::fs::read(&path).unwrap();
        let found = bytes.windows(WORD.len()).any(|w| w == WORD.as_bytes());
        assert!(!found, "the key is in {}", path.display());
    }
    // An error for such a key is still redacted.
    mock.set_script(Arc::new(|r, _| {
        Reply::json(401, &json!({"detail": r.header("authorization")}))
    }));
    let fresh = fixture(&[]);
    let judge = judge_with_key(&fresh, &mock, WORD);
    let e = fresh.triage(&judge, true).unwrap_err();
    assert!(!e.message.contains(WORD), "{e}");
    assert!(e.message.contains("[redacted]"), "{e}");
}
