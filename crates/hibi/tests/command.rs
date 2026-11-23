//! `CommandProvider` against purpose-built misbehaving providers (spec
//! section 13.1 to 13.4): R30, R31, R50, R62, `batch` use, error codes, and
//! the absence of a shell. The providers are `tests/providers/scripted.sh`
//! run by `sh`, so the tests need nothing beyond a POSIX shell.

#![cfg(feature = "command")]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use hibi::model::{Anchor, Locator, Selector};
use hibi::provider::command::{
    CommandProvider, ConformanceCase, ConformanceOptions, STDERR_TAIL_BYTES, run_conformance,
};
use hibi::provider::{Provider, ProviderError, ProviderErrorCode, Unit};
use tempfile::TempDir;

const FP: &str = "b3:test-v1:0123456789abcdef0123456789abcdef";

fn script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/providers/scripted.sh")
}

fn hello_with(selectors: &str, capabilities: &str) -> String {
    format!(
        r#"{{"id":@ID@,"result":{{"protocol":1,"provider":{{"name":"scripted","version":"0"}},"schemes":["t"],"selectors":{selectors},"capabilities":{capabilities}}}}}"#
    )
}

fn hello() -> String {
    hello_with(r#"["key"]"#, r#"{"units":true,"render":true}"#)
}

fn hello_batch() -> String {
    hello_with(r#"["key"]"#, r#"{"units":true,"render":true,"batch":true}"#)
}

fn unit(key: &str) -> String {
    format!(r#"{{"key":"{key}","kind":"note","fp":"{FP}","sel":{{"key":"{key}"}},"text":"x"}}"#)
}

fn units_answer(units: &[String]) -> String {
    format!(
        r#"{{"id":@ID@,"result":{{"units":[{}]}}}}"#,
        units.join(",")
    )
}

fn error_answer(code: &str) -> String {
    format!(r#"{{"id":@ID@,"error":{{"code":"{code}","message":"m"}}}}"#)
}

/// A scripted provider: its directory (pid, log) and the provider.
struct Scripted {
    dir: TempDir,
    work: PathBuf,
    provider: CommandProvider,
}

fn scripted(lines: &[String], timeout: Duration) -> Scripted {
    scripted_in("work", lines, timeout)
}

fn scripted_in(work_name: &str, lines: &[String], timeout: Duration) -> Scripted {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join(work_name);
    std::fs::create_dir(&work).unwrap();
    let script = dir.path().join("script");
    std::fs::write(&script, lines.join("\n") + "\n").unwrap();
    let argv: Vec<OsString> = vec![
        "sh".into(),
        script_path().into(),
        script.into(),
        work.clone().into(),
    ];
    let provider = CommandProvider::new("t", argv, dir.path(), timeout);
    Scripted {
        dir,
        work,
        provider,
    }
}

impl Scripted {
    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(self.work.join("log"))
            .map(|s| s.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    fn starts(&self) -> usize {
        self.log().iter().filter(|l| l.starts_with("start")).count()
    }

    fn requests(&self) -> Vec<serde_json::Value> {
        self.log()
            .iter()
            .filter(|l| l.starts_with('{'))
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn pid(&self) -> String {
        std::fs::read_to_string(self.work.join("pid"))
            .unwrap()
            .trim()
            .to_owned()
    }
}

/// True while a process with this id exists, zombies included.
fn exists(pid: &str) -> bool {
    Command::new("sh")
        .args(["-c", "kill -0 \"$1\" 2>/dev/null", "sh", pid])
        .status()
        .unwrap()
        .success()
}

fn anchor(key: &str) -> Anchor {
    Anchor::new(
        Locator::new("t:res").unwrap(),
        Selector {
            key: Some(key.into()),
            ..Selector::default()
        },
    )
}

fn resolve_one(p: &mut CommandProvider, key: &str) -> Result<Vec<Unit>, ProviderError> {
    p.resolve(&[anchor(key)]).pop().unwrap()
}

fn code<T: std::fmt::Debug>(r: &Result<T, ProviderError>) -> ProviderErrorCode {
    r.as_ref().unwrap_err().code
}

const LONG: Duration = Duration::from_secs(10);

/// Asserts the failure path of section 13.1: `failed`, the process is gone
/// (killed and reaped), and nothing is restarted.
fn assert_failed_and_gone(s: &mut Scripted, r: Result<Vec<Unit>, ProviderError>, why: &str) {
    let e = r.unwrap_err();
    assert_eq!(e.code, ProviderErrorCode::Failed, "{e}");
    assert!(e.message.contains(why), "{e}");
    assert!(!exists(&s.pid()), "the provider process still exists");
    assert_eq!(
        code(&resolve_one(&mut s.provider, "k")),
        ProviderErrorCode::Failed
    );
    assert_eq!(code(&s.provider.info()), ProviderErrorCode::Failed);
    assert_eq!(s.starts(), 1, "the provider was restarted");
    assert!(s.provider.failure().is_some());
}

// ---------------------------------------------------------------------------
// R30
// ---------------------------------------------------------------------------

#[test]
fn r30_timeout_kills() {
    let mut s = scripted(&[hello(), "sleep".into()], Duration::from_millis(300));
    let t = Instant::now();
    let r = resolve_one(&mut s.provider, "k");
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert_failed_and_gone(&mut s, r, "within 300 ms");
}

/// The scripted provider makes its 17 MiB line with `dd | tr`, which needs
/// seconds of CPU on macOS and more under load. The timeout is set far above
/// that, so the line limit, not the timeout, is what stops the provider.
/// Without the limit the whole line is read and the provider fails as "not
/// a JSON object", so the message check still proves the limit.
const OVERSIZED_TIMEOUT: Duration = Duration::from_secs(120);

#[test]
fn r30_oversized_line_kills() {
    let mut s = scripted(&[hello(), "big".into()], OVERSIZED_TIMEOUT);
    let r = resolve_one(&mut s.provider, "k");
    assert_failed_and_gone(&mut s, r, "longer than 16777216 bytes");
}

#[test]
fn r30_non_json_line_kills() {
    let mut s = scripted(&[hello(), "garbage".into()], LONG);
    let r = resolve_one(&mut s.provider, "k");
    assert_failed_and_gone(&mut s, r, "not a JSON object");
}

#[test]
fn r30_json_line_that_is_not_an_object_kills() {
    let mut s = scripted(&[hello(), "[1, 2]".into()], LONG);
    let r = resolve_one(&mut s.provider, "k");
    assert_failed_and_gone(&mut s, r, "not a JSON object");
}

#[test]
fn exit_mid_request_is_failed_and_reaped() {
    let mut s = scripted(&[hello(), "exit".into()], LONG);
    let r = resolve_one(&mut s.provider, "k");
    assert_failed_and_gone(&mut s, r, "exited or closed its output");
}

#[test]
fn wrong_id_kills() {
    let mut s = scripted(&[hello(), units_answer(&[]).replace("@ID@", "99")], LONG);
    let r = resolve_one(&mut s.provider, "k");
    assert_failed_and_gone(&mut s, r, "id 99, not the request's id 2");
}

#[test]
fn null_id_answer_kills() {
    let mut s = scripted(
        &[
            hello(),
            r#"{"id":null,"error":{"code":"invalid_request","message":"m"}}"#.into(),
        ],
        LONG,
    );
    let r = resolve_one(&mut s.provider, "k");
    assert_failed_and_gone(&mut s, r, "id null");
}

#[test]
fn broken_envelope_kills() {
    for line in [
        r#"{"id":@ID@}"#.to_owned(),
        r#"{"id":@ID@,"result":{"units":[]},"error":{"code":"internal","message":"m"}}"#.into(),
        r#"{"result":{"units":[]}}"#.into(),
        // Section 13.2: ids are positive; a null id only with invalid_request.
        r#"{"id":0,"result":{"units":[]}}"#.into(),
        r#"{"id":null,"result":{"units":[]}}"#.into(),
        r#"{"id":null,"error":{"code":"not_found","message":"m"}}"#.into(),
    ] {
        let mut s = scripted(&[hello(), line], LONG);
        let r = resolve_one(&mut s.provider, "k");
        assert_failed_and_gone(&mut s, r, "breaks the envelope");
    }
}

/// Standard error is drained (1 MiB would fill the pipe and stall the
/// provider otherwise) and its last 4096 bytes go into the failure message.
#[test]
fn stderr_in_volume_is_drained_and_its_tail_kept() {
    let mut s = scripted(
        &[
            format!("noise 1024 {}", hello()),
            format!("noise 1024 {}", units_answer(&[unit("k")])),
            "exit".into(),
        ],
        Duration::from_secs(5),
    );
    assert_eq!(resolve_one(&mut s.provider, "k").unwrap().len(), 1);
    let e = resolve_one(&mut s.provider, "k").unwrap_err();
    assert_eq!(e.code, ProviderErrorCode::Failed);
    let tail = e
        .message
        .split("standard error ends with: ")
        .nth(1)
        .unwrap();
    assert!(
        tail.len() <= STDERR_TAIL_BYTES && tail.len() > 4000,
        "{}",
        tail.len()
    );
    assert!(tail.bytes().all(|b| b == b'e'));
    assert!(!exists(&s.pid()));
}

// ---------------------------------------------------------------------------
// The timeout covers the write; output is bounded; an exit is seen at once
// ---------------------------------------------------------------------------

/// Run `f` on another thread and fail when it has not returned in `limit`:
/// a hang becomes a test failure and does not block the suite.
fn within<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit)
        .unwrap_or_else(|_| panic!("still blocked after {limit:?}"))
}

/// A provider that stops reading its input cannot block hibi: the timeout
/// covers the whole exchange, the write included. The request here is far
/// larger than a pipe buffer, so a blocking write would never return.
#[test]
fn a_provider_that_stops_reading_does_not_block_the_request() {
    let mut s = scripted(
        &[format!("last {}", hello_batch())],
        Duration::from_millis(1000),
    );
    s.provider.info().unwrap();
    let anchors: Vec<Anchor> = (0..1000)
        .map(|i| anchor(&format!("{i:04}-{}", "k".repeat(200))))
        .collect();
    assert!(serde_json::to_string(&anchors).unwrap().len() > 200_000);
    let pid = s.pid();
    let started = Instant::now();
    let (results, mut provider) = within(Duration::from_secs(8), move || {
        let r = s.provider.resolve(&anchors);
        (r, s.provider)
    });
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(results.len(), 1000);
    for r in &results {
        let e = r.as_ref().unwrap_err();
        assert_eq!(e.code, ProviderErrorCode::Failed, "{e}");
        assert!(e.message.contains("within 1000 ms"), "{e}");
    }
    assert!(!exists(&pid), "the provider process still exists");
    // Shutdown and drop do not block either.
    within(Duration::from_secs(3), move || {
        provider.shutdown();
        drop(provider);
    });
}

/// Output that arrives when no request is outstanding is not stored: the
/// provider stalls on a full pipe after about one line, and the next
/// request finds the violation, kills it and fails.
#[test]
fn output_with_no_request_outstanding_is_bounded_and_fails_the_provider() {
    let mut s = scripted(&[format!("flood {}", hello())], LONG);
    s.provider.info().unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let wrote = |s: &Scripted| s.log().iter().filter(|l| *l == "wrote").count();
    let before = wrote(&s);
    assert!(
        before <= 3,
        "{before} lines of 1 MiB were taken from the provider while no request was outstanding"
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(wrote(&s), before, "the provider is still being read");
    let r = resolve_one(&mut s.provider, "k");
    assert_failed_and_gone(&mut s, r, "no request was outstanding");
}

/// A child that exits without answering is noticed when it exits, also
/// when a grandchild keeps its standard output open: not at the timeout.
#[test]
fn an_exit_is_noticed_while_a_grandchild_holds_the_output() {
    let mut s = scripted(&[hello(), "fork".into()], LONG);
    s.provider.info().unwrap();
    let started = Instant::now();
    let r = resolve_one(&mut s.provider, "k");
    let took = started.elapsed();
    assert!(took < Duration::from_secs(3), "noticed after {took:?}");
    assert_failed_and_gone(&mut s, r, "exited or closed its output");
}

/// The limits of section 13.1, by value.
#[test]
fn the_limits_have_their_specified_values() {
    use hibi::provider::command::{DEFAULT_TIMEOUT, MAX_TRAILING_BYTES, SHUTDOWN_GRACE};
    assert_eq!(STDERR_TAIL_BYTES, 4096);
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(10));
    assert_eq!(SHUTDOWN_GRACE, Duration::from_secs(2));
    assert_eq!(MAX_TRAILING_BYTES, 65_536);
    assert_eq!(hibi::provider::MAX_LINE_BYTES, 16 * 1024 * 1024);
    assert_eq!(hibi::provider::MAX_BATCH, 1000);
    assert_eq!(ConformanceOptions::new(".").timeout, DEFAULT_TIMEOUT);
}

// ---------------------------------------------------------------------------
// Start and hello
// ---------------------------------------------------------------------------

#[test]
fn r31_one_start_per_run_and_hello_first() {
    let mut s = scripted(
        &[
            hello(),
            units_answer(&[unit("a")]),
            units_answer(&[unit("b")]),
            units_answer(&[unit("a"), unit("b")]),
            r#"{"id":@ID@,"result":{"text":"x","context":["p"]}}"#.into(),
        ],
        LONG,
    );
    assert!(s.log().is_empty(), "nothing starts before the first call");
    assert_eq!(s.provider.info().unwrap().provider.name, "scripted");
    assert_eq!(resolve_one(&mut s.provider, "a").unwrap()[0].key, "a");
    assert_eq!(resolve_one(&mut s.provider, "b").unwrap()[0].key, "b");
    let loc = Locator::new("t:res").unwrap();
    assert_eq!(s.provider.units(&loc, Some("note")).unwrap().len(), 2);
    assert_eq!(s.provider.render(&loc, "a").unwrap().context, ["p"]);
    assert_eq!(s.starts(), 1);
    let req = s.requests();
    let methods: Vec<&str> = req.iter().map(|r| r["method"].as_str().unwrap()).collect();
    assert_eq!(methods, ["hello", "resolve", "resolve", "units", "render"]);
    let ids: Vec<u64> = req.iter().map(|r| r["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, [1, 2, 3, 4, 5]);
    assert_eq!(req[0]["params"]["protocol"], serde_json::json!([1]));
    assert_eq!(req[0]["params"]["client"]["name"], "hibi");
    assert_eq!(req[3]["params"]["kind"], "note");
}

/// Section 13.1: no shell, working directory and environment.
#[test]
fn argv_is_passed_without_a_shell() {
    let name = "a b;$(touch pwned)`touch pwned2`*?'\"|&>x";
    let mut s = scripted_in(name, &[hello(), units_answer(&[])], LONG);
    assert_eq!(resolve_one(&mut s.provider, "k").unwrap(), vec![]);
    assert!(
        s.work.join("log").is_file(),
        "the argument was not passed literally"
    );
    for f in ["pwned", "pwned2", "x"] {
        assert!(!s.dir.path().join(f).exists(), "{f} was created");
        assert!(!s.work.join(f).exists(), "{f} was created");
    }
}

#[test]
fn working_directory_and_environment() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("env");
    let script = format!(
        "printf '%s\\n%s\\n%s\\n' \"$HIBI_ROOT\" \"$HIBI_PROTOCOL\" \"$(pwd -P)\" > '{}'; read l; echo '{}'; read l",
        out.display(),
        hello().replace("@ID@", "1")
    );
    let mut p = CommandProvider::new("t", ["sh", "-c", script.as_str()], dir.path(), LONG);
    p.info().unwrap();
    p.shutdown();
    let env = std::fs::read_to_string(&out).unwrap();
    let lines: Vec<&str> = env.lines().collect();
    assert_eq!(lines[0], dir.path().to_str().unwrap());
    assert_eq!(lines[1], "1");
    assert_eq!(
        Path::new(lines[2]),
        dir.path().canonicalize().unwrap().as_path()
    );
}

#[test]
fn a_command_that_cannot_start_is_failed() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = CommandProvider::new(
        "t",
        ["/nonexistent/hibi-provider-for-tests"],
        dir.path(),
        LONG,
    );
    let e = p.info().unwrap_err();
    assert_eq!(e.code, ProviderErrorCode::Failed);
    assert!(e.message.contains("could not be started"), "{e}");
    let r = p.resolve(&[anchor("a"), anchor("b")]);
    assert!(r.iter().all(|r| code(r) == ProviderErrorCode::Failed));
    let empty: [&str; 0] = [];
    let mut p = CommandProvider::new("t", empty, dir.path(), LONG);
    assert_eq!(code(&p.info()), ProviderErrorCode::Failed);
}

#[test]
fn hello_failures_make_the_provider_unusable() {
    let cases = [
        (error_answer("unsupported_protocol"), "unsupported_protocol"),
        (
            hello().replace(r#""protocol":1"#, r#""protocol":2"#),
            "protocol 2, which hibi did not offer",
        ),
        (
            hello().replace(r#"["t"]"#, r#"["other"]"#),
            "do not include \"t\"",
        ),
        // `selectors` is the closed list quote, symbol, key: a provider that
        // lists prefix or pos is unusable for the run.
        (
            hello().replace(r#"["key"]"#, r#"["key","prefix"]"#),
            "hello:",
        ),
        (hello().replace(r#"["key"]"#, r#"["pos"]"#), "hello:"),
        (
            r#"{"id":@ID@,"result":{"protocol":1}}"#.to_owned(),
            "hello:",
        ),
    ];
    for (line, why) in cases {
        let mut s = scripted(&[line, units_answer(&[])], LONG);
        let e = s.provider.info().unwrap_err();
        assert_eq!(e.code, ProviderErrorCode::Failed, "{e}");
        assert!(e.message.contains(why), "{why}: {e}");
        assert!(!exists(&s.pid()));
        assert_eq!(
            code(&resolve_one(&mut s.provider, "k")),
            ProviderErrorCode::Failed
        );
        assert_eq!(s.starts(), 1);
        assert_eq!(s.requests().len(), 1, "only hello was sent");
    }
}

// ---------------------------------------------------------------------------
// R50, R62: validation of answers and of selectors before sending
// ---------------------------------------------------------------------------

#[test]
fn r50_invalid_units_are_provider_errors_for_the_affected_side() {
    let bad = [
        // unknown key inside sel
        format!(r#"{{"key":"k","kind":"n","fp":"{FP}","sel":{{"key":"k","bogus":1}}}}"#),
        // null inside sel
        format!(r#"{{"key":"k","kind":"n","fp":"{FP}","sel":{{"key":null}}}}"#),
        // prefix without quote
        format!(r#"{{"key":"k","kind":"n","fp":"{FP}","sel":{{"prefix":"p"}}}}"#),
        // a key outside hello.selectors (["key"])
        format!(r#"{{"key":"k","kind":"n","fp":"{FP}","sel":{{"quote":"q"}}}}"#),
        format!(r#"{{"key":"k","kind":"n","fp":"{FP}","sel":{{"symbol":"s"}}}}"#),
        // invalid fingerprint
        r#"{"key":"k","kind":"n","fp":"b3:test-v1:XYZ","sel":{"key":"k"}}"#.to_owned(),
        // missing fp
        r#"{"key":"k","kind":"n","sel":{"key":"k"}}"#.to_owned(),
    ];
    let mut lines = vec![hello()];
    lines.extend(bad.iter().map(|u| units_answer(std::slice::from_ref(u))));
    // duplicate keys in one answer
    lines.push(units_answer(&[unit("k"), unit("k")]));
    // a result of the wrong shape
    lines.push(r#"{"id":@ID@,"result":{"unit":[]}}"#.into());
    lines.push(units_answer(&[unit("ok")]));
    let mut s = scripted(&lines, LONG);
    for i in 0..bad.len() + 2 {
        let r = resolve_one(&mut s.provider, "k");
        assert_eq!(code(&r), ProviderErrorCode::Internal, "answer {i}: {r:?}");
        assert!(r.unwrap_err().message.contains("invalid answer"));
    }
    // The provider stays usable.
    assert_eq!(resolve_one(&mut s.provider, "k").unwrap()[0].key, "ok");
    assert!(s.provider.failure().is_none());
}

#[test]
fn r50_duplicate_keys_in_units_and_parent_cycles() {
    let cycle = [
        format!(r#"{{"key":"a","kind":"n","parent":"b","fp":"{FP}","sel":{{"key":"a"}}}}"#),
        format!(r#"{{"key":"b","kind":"n","parent":"a","fp":"{FP}","sel":{{"key":"b"}}}}"#),
    ];
    let mut s = scripted(
        &[
            hello(),
            units_answer(&[unit("k"), unit("k")]),
            units_answer(&cycle),
        ],
        LONG,
    );
    let loc = Locator::new("t:res").unwrap();
    assert_eq!(
        code(&s.provider.units(&loc, None)),
        ProviderErrorCode::Internal
    );
    // A parent cycle is passed through; nothing here walks the chain.
    let units = s.provider.units(&loc, None).unwrap();
    assert_eq!(units[0].parent.as_deref(), Some("b"));
    assert_eq!(units[1].parent.as_deref(), Some("a"));
}

#[test]
fn r62_a_selector_key_the_provider_did_not_list_is_never_sent() {
    let mut s = scripted(&[hello(), units_answer(&[unit("k")])], LONG);
    let loc = Locator::new("t:res").unwrap();
    let out = s.provider.resolve(&[
        Anchor::new(loc.clone(), Selector::quote("q")),
        anchor("k"),
        Anchor::new(
            loc.clone(),
            Selector {
                symbol: Some("s".into()),
                ..Selector::default()
            },
        ),
    ]);
    assert_eq!(code(&out[0]), ProviderErrorCode::InvalidSelector);
    assert_eq!(out[1].as_ref().unwrap()[0].key, "k");
    assert_eq!(code(&out[2]), ProviderErrorCode::InvalidSelector);
    let sent: Vec<serde_json::Value> = s.requests().into_iter().skip(1).collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["params"]["sel"], serde_json::json!({"key": "k"}));
    // The empty selector is always allowed.
    let mut s = scripted(&[hello(), units_answer(&[])], LONG);
    let out = s.provider.resolve(&[Anchor::new(loc, Selector::empty())]);
    assert!(out[0].is_ok());
}

// ---------------------------------------------------------------------------
// Error codes, capabilities, batch
// ---------------------------------------------------------------------------

#[test]
fn every_wire_error_code_reaches_the_caller() {
    let mut lines = vec![hello()];
    let mut want = Vec::new();
    for c in ProviderErrorCode::WIRE {
        lines.push(error_answer(c.as_str()));
        want.push(c);
    }
    // `unparsable` is one of them: a command provider may say that a
    // resource is there and cannot be split into units, and the caller
    // gets that code and not `unavailable` or `internal`.
    assert!(want.contains(&ProviderErrorCode::Unparsable));
    assert_eq!(ProviderErrorCode::Unparsable.as_str(), "unparsable");
    // An unknown code, and `failed` on the wire, are `internal`.
    lines.push(error_answer("weird"));
    want.push(ProviderErrorCode::Internal);
    lines.push(error_answer("failed"));
    want.push(ProviderErrorCode::Internal);
    let mut s = scripted(&lines, LONG);
    for w in want {
        let r = resolve_one(&mut s.provider, "k");
        assert_eq!(code(&r), w);
        assert_eq!(r.unwrap_err().message, "m");
    }
    assert!(s.provider.failure().is_none());
}

#[test]
fn methods_the_provider_does_not_offer_are_not_sent() {
    let mut s = scripted(&[hello_with(r#"["key"]"#, "{}")], LONG);
    let loc = Locator::new("t:res").unwrap();
    assert_eq!(
        code(&s.provider.units(&loc, None)),
        ProviderErrorCode::UnknownMethod
    );
    assert_eq!(
        code(&s.provider.render(&loc, "k")),
        ProviderErrorCode::UnknownMethod
    );
    assert_eq!(s.requests().len(), 1);
}

#[test]
fn batch_is_used_only_when_offered_and_for_several_anchors() {
    let inner_ok = |k: &str| format!(r#"{{"result":{{"units":[{}]}}}}"#, unit(k));
    let duplicate_keys = format!(r#"{{"result":{{"units":[{},{}]}}}}"#, unit("d"), unit("d"));
    let batch = format!(
        r#"{{"id":@ID@,"result":{{"responses":[{},{},{duplicate_keys}]}}}}"#,
        inner_ok("a"),
        r#"{"error":{"code":"not_found","message":"m"}}"#,
    );
    let mut s = scripted(&[hello_batch(), batch, units_answer(&[unit("z")])], LONG);
    let out = s.provider.resolve(&[anchor("a"), anchor("b"), anchor("c")]);
    assert_eq!(out[0].as_ref().unwrap()[0].key, "a");
    assert_eq!(code(&out[1]), ProviderErrorCode::NotFound);
    assert_eq!(code(&out[2]), ProviderErrorCode::Internal);
    // One anchor goes out as a plain resolve.
    assert_eq!(resolve_one(&mut s.provider, "z").unwrap()[0].key, "z");
    let req = s.requests();
    assert_eq!(req[1]["method"], "batch");
    let inner = req[1]["params"]["requests"].as_array().unwrap();
    assert_eq!(inner.len(), 3);
    assert!(inner.iter().all(|r| r.get("id").is_none()));
    assert_eq!(inner[2]["params"]["sel"]["key"], "c");
    assert_eq!(req[2]["method"], "resolve");

    // Without the capability every anchor is its own request.
    let mut s = scripted(&[hello(), units_answer(&[]), units_answer(&[])], LONG);
    s.provider.resolve(&[anchor("a"), anchor("b")]);
    let methods: Vec<_> = s.requests().iter().map(|r| r["method"].clone()).collect();
    assert_eq!(methods, ["hello", "resolve", "resolve"]);
}

#[test]
fn batches_hold_at_most_1000_requests() {
    let answer = |n: usize| {
        let inner = vec![r#"{"result":{"units":[]}}"#; n].join(",");
        format!(r#"{{"id":@ID@,"result":{{"responses":[{inner}]}}}}"#)
    };
    let mut s = scripted(&[hello_batch(), answer(1000), answer(1)], LONG);
    let anchors: Vec<Anchor> = (0..1001).map(|i| anchor(&format!("k{i}"))).collect();
    let out = s.provider.resolve(&anchors);
    assert!(out.iter().all(Result::is_ok));
    let req = s.requests();
    assert_eq!(req[1]["params"]["requests"].as_array().unwrap().len(), 1000);
    assert_eq!(req[2]["params"]["requests"].as_array().unwrap().len(), 1);
    assert_eq!(
        req[2]["params"]["requests"][0]["params"]["sel"]["key"],
        "k1000"
    );
}

#[test]
fn a_batch_answer_of_the_wrong_length_is_internal_for_its_entries() {
    let short = r#"{"id":@ID@,"result":{"responses":[{"result":{"units":[]}}]}}"#.to_owned();
    let mut s = scripted(&[hello_batch(), short], LONG);
    let out = s.provider.resolve(&[anchor("a"), anchor("b")]);
    assert!(out.iter().all(|r| code(r) == ProviderErrorCode::Internal));
    assert!(s.provider.failure().is_none());
}

/// Every request not yet answered when the provider fails gets `failed`.
#[test]
fn a_failure_inside_a_run_of_requests_fails_the_rest() {
    let mut s = scripted(
        &[hello(), units_answer(&[unit("a")]), "garbage".into()],
        LONG,
    );
    let out = s.provider.resolve(&[anchor("a"), anchor("b"), anchor("c")]);
    assert_eq!(out[0].as_ref().unwrap()[0].key, "a");
    assert_eq!(code(&out[1]), ProviderErrorCode::Failed);
    assert_eq!(code(&out[2]), ProviderErrorCode::Failed);
    assert_eq!(s.requests().len(), 3, "nothing is sent after the failure");
}

#[test]
fn shutdown_closes_input_and_the_provider_exits() {
    let mut s = scripted(&[hello()], LONG);
    s.provider.info().unwrap();
    let pid = s.pid();
    let t = Instant::now();
    s.provider.shutdown();
    assert!(t.elapsed() < Duration::from_secs(2));
    assert!(!exists(&pid));
    assert_eq!(code(&s.provider.info()), ProviderErrorCode::Failed);
}

#[test]
fn drop_kills_a_provider_that_ignores_end_of_input() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let script = format!(
        "echo $$ > '{}'; read l; echo '{}'; exec sleep 60",
        pidfile.display(),
        hello().replace("@ID@", "1")
    );
    let mut p = CommandProvider::new("t", ["sh", "-c", script.as_str()], dir.path(), LONG);
    p.info().unwrap();
    let t = Instant::now();
    drop(p);
    let waited = t.elapsed();
    assert!(
        waited >= Duration::from_millis(1900) && waited < Duration::from_secs(5),
        "{waited:?}"
    );
    let pid = std::fs::read_to_string(pidfile).unwrap();
    assert!(!exists(pid.trim()));
}

// ---------------------------------------------------------------------------
// The conformance runner reports failures
// ---------------------------------------------------------------------------

fn case(json: serde_json::Value) -> ConformanceCase {
    serde_json::from_value(json).unwrap()
}

fn hello_step() -> serde_json::Value {
    serde_json::json!({"send": {"method": "hello", "params": {"protocol": [1], "client": {"name": "c", "version": "0"}, "root": "."}},
                       "expect": {"result": {"protocol": 1}}})
}

#[test]
fn conformance_runner_reports_each_kind_of_failure() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script");
    std::fs::write(
        &script,
        [hello(), units_answer(&[unit("a")]), "garbage".into()].join("\n") + "\n",
    )
    .unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let argv: Vec<OsString> = vec![
        "sh".into(),
        script_path().into(),
        script.into(),
        work.into(),
    ];
    let resolve = serde_json::json!({"method": "resolve", "params": {"loc": "t:res", "sel": {}}});
    let cases = [
        case(serde_json::json!({"name": "passes", "steps": [hello_step(),
            {"send": resolve, "expect": {"id": 2, "result": {"units": [{"key": "a", "fp": "<fingerprint>"}]}}}], "end": "exit"})),
        case(
            serde_json::json!({"name": "wrong value", "steps": [hello_step(),
            {"send": resolve, "expect": {"result": {"units": [{"key": "b"}]}}}]}),
        ),
        case(
            serde_json::json!({"name": "non-JSON line", "steps": [hello_step(),
            {"send": resolve, "expect": "<any>"}, {"send": resolve, "expect": "<any>"}]}),
        ),
        case(
            serde_json::json!({"name": "needs batch", "requires": ["batch"], "steps": [hello_step(),
            {"send": resolve, "expect": {"result": "<any>"}}]}),
        ),
        case(serde_json::json!({"name": "needs a fixture", "fixture": "f.json", "steps": []})),
    ];
    let report = run_conformance(&argv, &cases, &ConformanceOptions::new(dir.path()));
    let r = &report.cases;
    assert!(r[0].ok && !r[0].skipped, "{:?}", r[0]);
    assert!(!r[1].ok);
    assert!(
        r[1].detail
            .as_ref()
            .unwrap()
            .contains("at $.result.units[0].key: expected \"b\", got \"a\""),
        "{:?}",
        r[1]
    );
    assert!(!r[2].ok);
    assert!(
        r[2].detail
            .as_ref()
            .unwrap()
            .contains("step 3: a standard-output line is not a JSON object"),
        "{:?}",
        r[2]
    );
    assert!(r[3].ok && r[3].skipped, "{:?}", r[3]);
    assert!(
        r[3].detail
            .as_ref()
            .unwrap()
            .contains("does not offer batch")
    );
    assert!(!r[4].ok && r[4].detail.as_ref().unwrap().contains("fixture"));
    assert_eq!((report.passed, report.failed, report.skipped), (1, 3, 1));
    assert_eq!(report.hello.unwrap()["provider"]["name"], "scripted");
}

#[test]
fn conformance_runner_checks_the_exit_at_end_of_input() {
    let dir = tempfile::tempdir().unwrap();
    let hello1 = hello().replace("@ID@", "1");
    let run = |script: String| {
        let c = case(serde_json::json!({"name": "exit", "steps": [hello_step()], "end": "exit"}));
        let argv: Vec<OsString> = vec!["sh".into(), "-c".into(), script.into()];
        run_conformance(&argv, &[c], &ConformanceOptions::new(dir.path()))
            .cases
            .remove(0)
    };
    let r = run(format!("read l; echo '{hello1}'; read l; exit 0"));
    assert!(r.ok, "{r:?}");
    let r = run(format!("read l; echo '{hello1}'; read l; exit 4"));
    assert!(!r.ok && r.detail.unwrap().contains("exited with"));
    let r = run(format!("read l; echo '{hello1}'; exec sleep 60"));
    assert!(!r.ok && r.detail.unwrap().contains("did not exit within 2 s"));
    let r = run(format!("read l; echo '{hello1}'; read l; echo oops"));
    assert!(!r.ok && r.detail.unwrap().contains("not a JSON object"));
}

/// The runner applies the envelope rules that `CommandProvider` enforces:
/// a response with both `result` and `error`, a wrong id, and an id of the
/// wrong type all fail the step.
#[test]
fn conformance_runner_validates_the_envelope_of_every_response() {
    let dir = tempfile::tempdir().unwrap();
    let run = |answer: &str| {
        let c = case(serde_json::json!({"name": "envelope", "steps": [
            {"send": {"method": "hello", "params": {"protocol": [1], "client": {"name": "c", "version": "0"}, "root": "."}},
             "expect": "<any>"}]}));
        let script = format!("read l; echo '{answer}'; read l");
        let argv: Vec<OsString> = vec!["sh".into(), "-c".into(), script.into()];
        run_conformance(&argv, &[c], &ConformanceOptions::new(dir.path()))
            .cases
            .remove(0)
    };
    let good = hello().replace("@ID@", "1");
    assert!(run(&good).ok);
    let detail = |answer: &str| {
        let r = run(answer);
        assert!(!r.ok, "{answer} passed");
        r.detail.unwrap()
    };
    let both = r#"{"id":1,"result":{},"error":{"code":"internal","message":"m"}}"#;
    assert!(
        detail(both).contains("both result and error"),
        "{}",
        detail(both)
    );
    let neither = r#"{"id":1}"#;
    assert!(detail(neither).contains("neither result nor error"));
    let wrong_id = hello().replace("@ID@", "99");
    assert!(
        detail(&wrong_id).contains("the response id is 99, the request id 1"),
        "{}",
        detail(&wrong_id)
    );
    let no_id = r#"{"result":{}}"#;
    assert!(detail(no_id).contains("has no id"), "{}", detail(no_id));
    let text_id = r#"{"id":"1","result":{}}"#;
    assert!(detail(text_id).contains("positive integer or null"));
    let null_id = r#"{"id":null,"result":{}}"#;
    assert!(detail(null_id).contains("invalid_request"));
}

/// Output after the last response is read with a bound, and output that
/// does not end in a line feed is a failure, not dropped.
#[test]
fn conformance_runner_fails_on_unterminated_and_on_excess_trailing_output() {
    let dir = tempfile::tempdir().unwrap();
    let hello1 = hello().replace("@ID@", "1");
    let run = |end: &str| {
        let c = case(serde_json::json!({"name": "tail", "steps": [hello_step()], "end": "exit"}));
        let script = format!("read l; echo '{hello1}'; read l; {end}");
        let argv: Vec<OsString> = vec!["sh".into(), "-c".into(), script.into()];
        let started = Instant::now();
        let r = run_conformance(&argv, &[c], &ConformanceOptions::new(dir.path()))
            .cases
            .remove(0);
        (r, started.elapsed())
    };
    assert!(run("exit 0").0.ok);
    // A terminated JSON object after the last response is allowed (13.6).
    assert!(run("echo '{}'; exit 0").0.ok);
    for end in ["printf oops; exit 0", "printf '{}'; exit 0"] {
        let (r, _) = run(end);
        assert!(!r.ok, "{end} passed: {r:?}");
        let detail = r.detail.unwrap();
        assert!(detail.contains("does not end with a line feed"), "{detail}");
    }
    // More than the bound: a failure, reached without reading it all.
    let flood = "while :; do echo '{\"k\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}'; done";
    let (r, took) = run(flood);
    assert!(!r.ok);
    let detail = r.detail.unwrap();
    assert!(detail.contains("more than 65536 bytes"), "{detail}");
    assert!(took < Duration::from_secs(5), "{took:?}");
}

/// `end: exit` is judged by the exit of the provider process itself: a
/// grandchild that keeps the output open neither hides an exit with status
/// 0 nor delays the verdict to the end of the grace period.
#[test]
fn conformance_runner_judges_the_exit_of_the_child_not_the_end_of_output() {
    let dir = tempfile::tempdir().unwrap();
    let hello1 = hello().replace("@ID@", "1");
    let c = case(serde_json::json!({"name": "exit", "steps": [hello_step()], "end": "exit"}));
    let script = format!("read l; echo '{hello1}'; read l; sleep 20 & exit 0");
    let argv: Vec<OsString> = vec!["sh".into(), "-c".into(), script.into()];
    let started = Instant::now();
    let r = run_conformance(&argv, &[c], &ConformanceOptions::new(dir.path()))
        .cases
        .remove(0);
    assert!(r.ok, "{r:?}");
    assert!(
        started.elapsed() < Duration::from_millis(1800),
        "{:?}",
        started.elapsed()
    );
}

/// The protocol case for an unoffered version asks for the error and for
/// nothing after it: hibi never sends a second `hello`, so a provider may
/// keep running or exit once it has answered, with any status.
#[test]
fn the_unsupported_protocol_case_passes_whether_the_provider_stays_or_exits() {
    use hibi::provider::command::protocol_cases;
    let dir = tempfile::tempdir().unwrap();
    let case: Vec<ConformanceCase> = protocol_cases()
        .into_iter()
        .filter(|c| c.name.contains("unsupported_protocol"))
        .collect();
    assert_eq!(case.len(), 1);
    assert_eq!(case[0].steps.len(), 1);
    assert!(case[0].end.is_none());
    let answer = r#"{"id":1,"error":{"code":"unsupported_protocol","message":"only 1"}}"#;
    for rest in [
        "exit 0",
        "exit 3",
        "read l; exit 0",
        "while read l; do :; done",
    ] {
        let script = format!("read l; echo '{answer}'; {rest}");
        let argv: Vec<OsString> = vec!["sh".into(), "-c".into(), script.into()];
        let r = run_conformance(&argv, &case, &ConformanceOptions::new(dir.path()))
            .cases
            .remove(0);
        assert!(r.ok, "{rest}: {r:?}");
    }
    // The error itself is still required.
    let script = format!("read l; echo '{}'; read l", hello().replace("@ID@", "1"));
    let argv: Vec<OsString> = vec!["sh".into(), "-c".into(), script.into()];
    let r = run_conformance(&argv, &case, &ConformanceOptions::new(dir.path()))
        .cases
        .remove(0);
    assert!(!r.ok, "{r:?}");
}

/// The wait for the exit after end of input is the 2 seconds of section
/// 13.6, measured: a provider that exits 3 seconds after its input closed
/// fails, and the verdict comes when the 2 seconds are over. The wait is
/// also the one the options carry, so that a shorter one can be tested.
#[test]
fn conformance_runner_waits_two_seconds_for_the_exit_and_no_longer() {
    use hibi::provider::command::SHUTDOWN_GRACE;
    let dir = tempfile::tempdir().unwrap();
    let hello1 = hello().replace("@ID@", "1");
    let run = |rest: &str, opts: &ConformanceOptions| {
        let c = case(serde_json::json!({"name": "exit", "steps": [hello_step()], "end": "exit"}));
        let script = format!("read l; echo '{hello1}'; read l; {rest}");
        let argv: Vec<OsString> = vec!["sh".into(), "-c".into(), script.into()];
        let started = Instant::now();
        let r = run_conformance(&argv, &[c], opts).cases.remove(0);
        (r, started.elapsed())
    };
    let default = ConformanceOptions::new(dir.path());
    assert_eq!(default.grace, SHUTDOWN_GRACE);
    assert_eq!(default.grace, Duration::from_secs(2));

    // Exits with status 0, but a second too late for the default wait.
    let (r, took) = run("sleep 3; exit 0", &default);
    assert!(!r.ok, "{r:?}");
    assert!(r.detail.unwrap().contains("did not exit within 2 s"));
    assert!(
        took >= Duration::from_millis(1900) && took < Duration::from_millis(2900),
        "the verdict came after {took:?}"
    );
    // Within the wait it passes, and the verdict does not take the whole
    // wait.
    let (r, took) = run("sleep 1; exit 0", &default);
    assert!(r.ok, "{r:?}");
    assert!(took < Duration::from_millis(1900), "{took:?}");

    // The wait given in the options is the one used.
    let short = ConformanceOptions {
        grace: Duration::from_millis(300),
        ..ConformanceOptions::new(dir.path())
    };
    let (r, took) = run("sleep 1; exit 0", &short);
    assert!(!r.ok, "{r:?}");
    assert!(r.detail.unwrap().contains("did not exit within 300 ms"));
    assert!(took < Duration::from_millis(900), "{took:?}");
    assert!(run("exit 0", &short).0.ok);
}
