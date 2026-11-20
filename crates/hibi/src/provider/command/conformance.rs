//! The provider conformance runner (section 13.6) and the protocol cases
//! compiled into hibi for `hibi providers --check <scheme>` (section 15.3).
//!
//! A case starts the provider, sends each step and compares the response to
//! the step's `expect`. Case shape, as in section 13.6, plus `requires`:
//!
//! ```json
//! {"name": "...", "fixture": "notes.json", "requires": ["batch"],
//!  "steps": [{"send": {"method": "hello", "params": {}}, "expect": {"result": {}}}],
//!  "end": "exit"}
//! ```
//!
//! - `fixture` (optional): the runner appends `--fixture <dir>/<fixture>` to
//!   the command, with `<dir>` from [`ConformanceOptions::fixtures`].
//! - `requires` (optional): capability names (`units`, `render`, `batch`).
//!   When a `hello` step of the case returns capabilities that lack one, the
//!   case stops there and is reported as skipped, which is not a failure.
//! - `send`: an object gets the next `id` (1, 2, ...) and the response must
//!   carry that id; a string is written verbatim as a line and uses no id.
//! - `expect`: every key MUST be present in the response with a matching
//!   value; objects match recursively, arrays by length and element in
//!   order; `"<any>"`, `"<fingerprint>"` and `"<string>"` match any value,
//!   any valid fingerprint string and any string.
//! - `end: "exit"`: after standard input is closed the provider MUST exit
//!   with status 0 within 2 seconds.
//! - Every standard-output line MUST be a JSON object.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::process::{Process, ReadError};
use super::{DEFAULT_TIMEOUT, SHUTDOWN_GRACE, parse_response, spawn};
use crate::Error;
use crate::config::{RepoConfig, UserConfig};
use crate::model::Fingerprint;
use crate::provider::Response;

/// One conformance case (section 13.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConformanceCase {
    pub name: String,
    /// A fixture file name, passed to the provider as `--fixture <dir>/<name>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture: Option<String>,
    /// Capabilities the case needs. Without them the case is skipped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Capability>,
    pub steps: Vec<Step>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<End>,
}

/// A `hello` capability a case can require.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Units,
    Render,
    Batch,
}

impl Capability {
    fn as_str(self) -> &'static str {
        match self {
            Capability::Units => "units",
            Capability::Render => "render",
            Capability::Batch => "batch",
        }
    }
}

/// One request and the response it must get.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub send: Outgoing,
    pub expect: Value,
}

/// What a step writes: a request object that gets the next id, or a line
/// written verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Outgoing {
    Request(Map<String, Value>),
    Raw(String),
}

/// How a case ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum End {
    /// After standard input is closed the process exits with status 0
    /// within 2 seconds.
    Exit,
}

/// How to run cases.
#[derive(Debug, Clone)]
pub struct ConformanceOptions {
    /// The working directory and `HIBI_ROOT` of the provider.
    pub root: PathBuf,
    /// How long each response may take.
    pub timeout: Duration,
    /// The directory of the fixture files cases name.
    pub fixtures: Option<PathBuf>,
}

impl ConformanceOptions {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ConformanceOptions {
            root: root.into(),
            timeout: DEFAULT_TIMEOUT,
            fixtures: None,
        }
    }
}

/// The outcome of one case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CaseResult {
    pub name: String,
    /// True when the case passed or was skipped.
    pub ok: bool,
    /// True when the provider lacks a capability the case requires.
    pub skipped: bool,
    /// Why the case failed or was skipped.
    pub detail: Option<String>,
}

/// Schema of [`ConformanceReport::hello`]. The value is the `result` of the
/// provider's `hello` answer, copied without validation: the `hello` result
/// of section 13.3 from a provider that conforms, whatever else a provider
/// that does not conform sent, and `null` when it gave none.
fn hello_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let info = generator.subschema_for::<crate::provider::ProviderInfo>();
    schemars::json_schema!({
        "anyOf": [
            info,
            {
                "description": "No `hello` result was received.",
                "type": "null"
            },
            {
                "description": "The result of a provider that does not conform, as it sent it. The `hello` case then fails."
            }
        ]
    })
}

/// The outcome of a run.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ConformanceReport {
    /// The first `hello` result the provider gave, or `null`.
    #[schemars(schema_with = "hello_schema")]
    pub hello: Option<Value>,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub cases: Vec<CaseResult>,
}

/// Read every `*.json` file of `dir` as a case, in file-name order.
pub fn load_cases(dir: &Path) -> Result<Vec<ConformanceCase>, Error> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir.display(), &e))?;
    let mut paths = Vec::new();
    for e in entries {
        let path = e.map_err(|e| Error::io(dir.display(), &e))?.path();
        if path.extension().is_some_and(|x| x == "json") && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).map_err(|e| Error::io(p.display(), &e))?;
            parse_case(&text).map_err(|e| Error::invalid_argument(format!("{}: {e}", p.display())))
        })
        .collect()
}

fn parse_case(text: &str) -> Result<ConformanceCase, String> {
    serde_json::from_str(text).map_err(|e| format!("not a conformance case: {e}"))
}

macro_rules! protocol_case_files {
    ($($name:literal),* $(,)?) => {
        /// The files of `cases/provider/protocol/`, compiled in.
        const PROTOCOL_CASE_FILES: &[(&str, &str)] = &[$((
            $name,
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../cases/provider/protocol/", $name)),
        )),*];
    };
}

protocol_case_files!(
    "batch-order.json",
    "hello.json",
    "malformed-line.json",
    "resolve-absent-resource.json",
    "shutdown.json",
    "unknown-method.json",
    "unsupported-protocol.json",
);

/// The cases of `cases/provider/protocol/` that every provider MUST pass,
/// as compiled into this library.
pub fn protocol_cases() -> Vec<ConformanceCase> {
    PROTOCOL_CASE_FILES
        .iter()
        .map(|(name, text)| {
            parse_case(text).unwrap_or_else(|e| panic!("compiled-in case {name}: {e}"))
        })
        .collect()
}

/// Run `cases` against the provider started from `command` (no shell),
/// one process per case.
pub fn run_conformance(
    command: &[OsString],
    cases: &[ConformanceCase],
    opts: &ConformanceOptions,
) -> ConformanceReport {
    let mut hello = None;
    let results: Vec<CaseResult> = cases
        .iter()
        .map(|case| {
            let outcome = run_case(command, case, opts, &mut hello);
            let (ok, skipped, detail) = match outcome {
                Outcome::Passed => (true, false, None),
                Outcome::Skipped(why) => (true, true, Some(why)),
                Outcome::Failed(why) => (false, false, Some(why)),
            };
            CaseResult {
                name: case.name.clone(),
                ok,
                skipped,
                detail,
            }
        })
        .collect();
    let skipped = results.iter().filter(|r| r.skipped).count();
    let failed = results.iter().filter(|r| !r.ok).count();
    ConformanceReport {
        hello,
        passed: results.len() - failed - skipped,
        failed,
        skipped,
        cases: results,
    }
}

/// `check` in the output of `hibi providers --check <scheme>` (section
/// 15.3): the scheme and the outcome of the protocol cases.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ProviderCheck {
    pub scheme: String,
    #[serde(flatten)]
    pub report: ConformanceReport,
}

/// `hibi providers --check <scheme>` (section 15.3): start the provider
/// the user config registers for `scheme`, in `root`, with its configured
/// timeout, and run [`protocol_cases`] against it. A failing case is part
/// of the report and not an error.
///
/// A command is run only for a scheme that `hibi.toml` lists and the user
/// config registers (section 11.2); any other scheme is `invalid_argument`
/// and nothing is started.
pub fn check_provider(
    config: &RepoConfig,
    user: &UserConfig,
    root: &Path,
    scheme: &str,
) -> Result<ProviderCheck, Error> {
    if !config.schemes.contains_key(scheme) {
        return Err(Error::invalid_argument(format!(
            "the scheme {scheme:?} is not listed under [schemes] in .hibi/hibi.toml"
        ))
        .with_remediation("hibi providers"));
    }
    let Some(registered) = user.providers.get(scheme) else {
        return Err(Error::invalid_argument(format!(
            "the scheme {scheme:?} has no provider command in the user config"
        ))
        .with_remediation("hibi providers"));
    };
    let command: Vec<OsString> = registered.command.iter().map(OsString::from).collect();
    let opts = ConformanceOptions {
        root: root.to_path_buf(),
        timeout: Duration::from_millis(registered.timeout_ms),
        fixtures: None,
    };
    Ok(ProviderCheck {
        scheme: scheme.to_owned(),
        report: run_conformance(&command, &protocol_cases(), &opts),
    })
}

enum Outcome {
    Passed,
    Skipped(String),
    Failed(String),
}

fn run_case(
    command: &[OsString],
    case: &ConformanceCase,
    opts: &ConformanceOptions,
    hello: &mut Option<Value>,
) -> Outcome {
    let mut argv = command.to_vec();
    if let Some(fixture) = &case.fixture {
        let Some(dir) = &opts.fixtures else {
            return Outcome::Failed(format!(
                "the case needs fixture {fixture:?} and no fixture directory was given"
            ));
        };
        argv.push("--fixture".into());
        argv.push(dir.join(fixture).into_os_string());
    }
    let mut process = match spawn(&argv, &opts.root) {
        Ok(p) => p,
        Err(e) => return Outcome::Failed(format!("the provider could not be started: {e}")),
    };
    let mut next_id = 1u64;
    for (n, step) in case.steps.iter().enumerate() {
        let n = n + 1;
        let (line, id, method) = match &step.send {
            Outgoing::Request(map) => {
                let mut map = map.clone();
                map.insert("id".into(), Value::from(next_id));
                next_id += 1;
                let method = map.get("method").and_then(Value::as_str).map(str::to_owned);
                (Value::Object(map).to_string(), Some(next_id - 1), method)
            }
            Outgoing::Raw(s) => (s.clone(), None, None),
        };
        let (response, read) = match exchange(&mut process, &line, opts.timeout) {
            Ok(r) => r,
            Err(why) => return failed(&process, format!("step {n}: {why}")),
        };
        if let Some(id) = id
            && read.id != Some(id)
        {
            return failed(
                &process,
                format!(
                    "step {n}: the response id is {}, the request id {id}",
                    read.id.map_or("null".to_owned(), |i| i.to_string())
                ),
            );
        }
        if let Err(why) = matches(&step.expect, &response, "$") {
            return failed(&process, format!("step {n}: {why}; response: {response}"));
        }
        if method.as_deref() == Some("hello")
            && let Some(result) = response.get("result")
        {
            hello.get_or_insert_with(|| result.clone());
            let lacking: Vec<&str> = case
                .requires
                .iter()
                .filter(|c| {
                    result.pointer(&format!("/capabilities/{}", c.as_str()))
                        != Some(&Value::Bool(true))
                })
                .map(|c| c.as_str())
                .collect();
            if !lacking.is_empty() {
                return Outcome::Skipped(format!(
                    "skipped: the provider does not offer {}",
                    lacking.join(", ")
                ));
            }
        }
    }
    let end = process.finish(SHUTDOWN_GRACE);
    if let Some(problem) = end.problem {
        return failed(&process, format!("after the last step: {problem}"));
    }
    if let Some(bad) = end.lines.iter().find(|l| !is_object(l)) {
        return failed(
            &process,
            format!(
                "after the last step: a standard-output line is not a JSON object: {:?}",
                String::from_utf8_lossy(bad)
            ),
        );
    }
    // The verdict is the exit of the provider process itself.
    match (case.end, end.status) {
        (None, _) => Outcome::Passed,
        (Some(End::Exit), Some(s)) if s.success() => Outcome::Passed,
        (Some(End::Exit), Some(s)) => failed(
            &process,
            format!("after end of input the provider exited with {s}"),
        ),
        (Some(End::Exit), None) => failed(
            &process,
            format!(
                "the provider did not exit within {} s after end of input",
                SHUTDOWN_GRACE.as_secs()
            ),
        ),
    }
}

fn failed(process: &Process, why: String) -> Outcome {
    let tail = process.stderr_tail();
    if tail.is_empty() {
        Outcome::Failed(why)
    } else {
        Outcome::Failed(format!("{why}; standard error ends with: {tail}"))
    }
}

fn is_object(line: &[u8]) -> bool {
    matches!(serde_json::from_slice::<Value>(line), Ok(Value::Object(_)))
}

/// Write one line and read the response within `timeout`. The response
/// must pass the envelope rules that `CommandProvider` applies.
fn exchange(
    process: &mut Process,
    line: &str,
    timeout: Duration,
) -> Result<(Value, Response), String> {
    process
        .send(line)
        .map_err(|e| format!("writing the request failed: {e}"))?;
    let bytes = process.read_line(timeout).map_err(|e| match e {
        ReadError::Timeout => format!("no response within {} ms", timeout.as_millis()),
        ReadError::TooLong => "the response line is longer than 16 MiB".to_owned(),
        ReadError::Ended {
            unterminated: true, ..
        } => "the provider exited or closed its output without answering: its last output does not end with a line feed"
            .to_owned(),
        ReadError::Ended { .. } => {
            "the provider exited or closed its output without answering".to_owned()
        }
        ReadError::WriteFailed(why) => format!("writing the request failed: {why}"),
    })?;
    parse_response(&bytes).map_err(|why| {
        if why.starts_with("is not a JSON object") {
            format!("a standard-output line {why}")
        } else {
            format!("the response {why}")
        }
    })
}

/// The matching rule of section 13.6. `Err` says where and how the
/// response differs.
pub(crate) fn matches(expect: &Value, actual: &Value, at: &str) -> Result<(), String> {
    let differs = || Err(format!("at {at}: expected {expect}, got {actual}"));
    match expect {
        Value::String(s) if s == "<any>" => Ok(()),
        Value::String(s) if s == "<string>" => match actual {
            Value::String(_) => Ok(()),
            _ => differs(),
        },
        Value::String(s) if s == "<fingerprint>" => match actual {
            Value::String(a) if Fingerprint::new(a.as_str()).is_ok() => Ok(()),
            _ => differs(),
        },
        Value::Object(e) => {
            let Value::Object(a) = actual else {
                return differs();
            };
            for (k, v) in e {
                let Some(av) = a.get(k) else {
                    return Err(format!("at {at}: key {k:?} is missing"));
                };
                matches(v, av, &format!("{at}.{k}"))?;
            }
            Ok(())
        }
        Value::Array(e) => {
            let Value::Array(a) = actual else {
                return differs();
            };
            if e.len() != a.len() {
                return Err(format!(
                    "at {at}: expected {} elements, got {}: {actual}",
                    e.len(),
                    a.len()
                ));
            }
            for (i, (ev, av)) in e.iter().zip(a).enumerate() {
                matches(ev, av, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        _ if expect == actual => Ok(()),
        _ => differs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn check_provider_refuses_a_scheme_that_is_not_listed_or_not_registered() {
        let config = RepoConfig::parse("version = 1\n[schemes.ref]\n").unwrap();
        let none = UserConfig::default();
        let dir = std::env::temp_dir();

        let e = check_provider(&config, &none, &dir, "ref").unwrap_err();
        assert_eq!(e.code, crate::Code::InvalidArgument);
        assert!(e.message.contains("user config"), "{}", e.message);

        // Registered by the user but not listed by the repository: the
        // command, which does not exist, is never started.
        let user =
            UserConfig::parse("[providers.other]\ncommand = [\"/nonexistent/hibi-provider\"]\n")
                .unwrap();
        let e = check_provider(&config, &user, &dir, "other").unwrap_err();
        assert_eq!(e.code, crate::Code::InvalidArgument);
        assert!(e.message.contains("hibi.toml"), "{}", e.message);
    }

    #[test]
    fn check_provider_reports_a_command_that_cannot_start_as_failed_cases() {
        let config = RepoConfig::parse("version = 1\n[schemes.ref]\n").unwrap();
        let user =
            UserConfig::parse("[providers.ref]\ncommand = [\"/nonexistent/hibi-provider\"]\n")
                .unwrap();
        let check = check_provider(&config, &user, &std::env::temp_dir(), "ref").unwrap();
        assert_eq!(check.scheme, "ref");
        assert_eq!(check.report.passed, 0);
        assert_eq!(check.report.failed, check.report.cases.len());
        assert!(check.report.hello.is_none());
        let json = serde_json::to_value(&check).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["cases", "failed", "hello", "passed", "scheme", "skipped"]
        );
    }

    #[test]
    fn matching_rule() {
        let fp = "b3:prose-v1:0123456789abcdef0123456789abcdef";
        let actual = json!({"id": 2, "result": {"units": [{"key": "n1", "fp": fp, "extra": [1]}]}});
        assert!(matches(&json!({"result": {"units": [{"key": "n1"}]}}), &actual, "$").is_ok());
        assert!(
            matches(
                &json!({"result": {"units": [{"fp": "<fingerprint>"}]}}),
                &actual,
                "$"
            )
            .is_ok()
        );
        assert!(
            matches(
                &json!({"result": {"units": [{"key": "<string>", "extra": "<any>"}]}}),
                &actual,
                "$"
            )
            .is_ok()
        );
        assert!(matches(&json!({"id": 2}), &actual, "$").is_ok());
        // A missing key fails, also with "<any>".
        let e = matches(&json!({"error": "<any>"}), &actual, "$").unwrap_err();
        assert!(e.contains("\"error\" is missing"), "{e}");
        // Arrays need the same length.
        assert!(matches(&json!({"result": {"units": []}}), &actual, "$").is_err());
        assert!(matches(&json!({"result": {"units": [{}, {}]}}), &actual, "$").is_err());
        // Placeholders check the type.
        assert!(matches(&json!("<string>"), &json!(1), "$").is_err());
        assert!(matches(&json!("<fingerprint>"), &json!("b3:x:zz"), "$").is_err());
        assert!(matches(&json!(null), &json!(null), "$").is_ok());
        let e = matches(&json!({"id": 3}), &actual, "$").unwrap_err();
        assert_eq!(e, "at $.id: expected 3, got 2");
    }

    /// The compiled-in protocol cases are exactly the files of
    /// `cases/provider/protocol/`, with the same content.
    #[test]
    fn compiled_in_cases_equal_the_directory() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cases/provider/protocol");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        on_disk.sort();
        let compiled: Vec<&str> = PROTOCOL_CASE_FILES.iter().map(|(n, _)| *n).collect();
        assert_eq!(on_disk, compiled);
        assert_eq!(protocol_cases(), load_cases(&dir).unwrap());
        for case in protocol_cases() {
            assert!(
                case.fixture.is_none(),
                "{}: protocol cases have no fixture",
                case.name
            );
        }
    }
}
