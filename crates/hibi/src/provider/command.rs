//! `CommandProvider`, the external-command adapter, and the conformance
//! runner (spec sections 13.1, 13.6 and 15.3 `providers --check`).
//!
//! A `CommandProvider` serves one scheme through one child process started
//! from an argv without a shell. The process is started on the first call of
//! any [`Provider`] method and at most once: after any failure every request
//! answers `failed` and nothing is restarted.

mod conformance;
mod process;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

pub use conformance::{
    Capability, CaseResult, ConformanceCase, ConformanceOptions, ConformanceReport, End, Outgoing,
    Step, load_cases, protocol_cases, run_conformance,
};
pub use process::STDERR_TAIL_BYTES;

use self::process::{Process, ReadError};
use super::{
    BatchParams, BatchResult, Call, HelloParams, MAX_BATCH, MAX_LINE_BYTES, NameVersion,
    PROTOCOL_VERSION, Provider, ProviderError, ProviderErrorCode, ProviderInfo, RawCall,
    RenderParams, Rendered, Request, ResolveParams, Response, Unit, UnitsParams, UnitsResult,
    check_units, read_result,
};
use crate::model::{Anchor, Locator};

/// The default response timeout (`timeout_ms`, section 11.2).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long hibi waits for a provider to exit after closing its standard
/// input before killing it.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// The client name and version sent in `hello`.
fn client() -> NameVersion {
    NameVersion {
        name: "hibi".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

/// The environment of section 13.1, added to the inherited one.
fn provider_env(root: &Path) -> [(&'static str, OsString); 2] {
    [
        ("HIBI_ROOT", root.as_os_str().to_owned()),
        (
            "HIBI_PROTOCOL",
            OsString::from(PROTOCOL_VERSION.to_string()),
        ),
    ]
}

/// Start a provider process for `argv` in `root` with the environment of
/// section 13.1.
fn spawn(argv: &[OsString], root: &Path) -> std::io::Result<Process> {
    let env = provider_env(root);
    let env: Vec<(&str, &OsString)> = env.iter().map(|(k, v)| (*k, v)).collect();
    Process::spawn(argv, root, &env, MAX_LINE_BYTES)
}

enum State {
    NotStarted,
    Running {
        process: Process,
        info: ProviderInfo,
        next_id: u64,
    },
    Failed(ProviderError),
}

/// The provider of one scheme, served by an external command (section 13).
pub struct CommandProvider {
    scheme: String,
    argv: Vec<OsString>,
    root: PathBuf,
    timeout: Duration,
    state: State,
}

impl CommandProvider {
    /// A provider for `scheme` served by `command` (from the user config,
    /// section 11.2), run in `root` with `timeout` per response. Nothing is
    /// started until the first call.
    pub fn new<I, S>(scheme: impl Into<String>, command: I, root: &Path, timeout: Duration) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        CommandProvider {
            scheme: scheme.into(),
            argv: command.into_iter().map(Into::into).collect(),
            root,
            timeout,
            state: State::NotStarted,
        }
    }

    /// The scheme this provider was registered for.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The error that made this provider unusable for the run, if any.
    pub fn failure(&self) -> Option<&ProviderError> {
        match &self.state {
            State::Failed(e) => Some(e),
            _ => None,
        }
    }

    /// Close the provider's standard input and wait [`SHUTDOWN_GRACE`] for
    /// it to exit, then kill it. Later calls answer `failed`. Also done on
    /// drop.
    pub fn shutdown(&mut self) {
        if let State::Running { process, .. } = &mut self.state {
            process.close(SHUTDOWN_GRACE);
        }
        if !matches!(self.state, State::Failed(_)) {
            self.state = State::Failed(ProviderError::failed("the provider was shut down"));
        }
    }

    /// Start the process and run `hello`, once.
    fn ensure_started(&mut self) -> Result<(), ProviderError> {
        match &self.state {
            State::Running { .. } => return Ok(()),
            State::Failed(e) => return Err(e.clone()),
            State::NotStarted => {}
        }
        let process = match spawn(&self.argv, &self.root) {
            Ok(p) => p,
            Err(e) => {
                let program = self.argv.first().map(|p| p.to_string_lossy().into_owned());
                let e = ProviderError::failed(format!(
                    "the provider command {:?} for scheme {:?} could not be started: {e}",
                    program.unwrap_or_default(),
                    self.scheme
                ));
                self.state = State::Failed(e.clone());
                return Err(e);
            }
        };
        let mut process = process;
        let hello = Call::Hello(HelloParams {
            protocol: vec![PROTOCOL_VERSION],
            client: client(),
            root: self.root.to_string_lossy().into_owned(),
        });
        let outcome = exchange(&mut process, 1, &hello.to_raw(), self.timeout)
            .and_then(|r| r.map_err(|e| format!("hello failed: {e}")))
            .and_then(|v| {
                read_result::<ProviderInfo>(v).map_err(|e| format!("hello: {}", e.message))
            })
            .and_then(|info| {
                if info.protocol != PROTOCOL_VERSION {
                    Err(format!(
                        "hello chose protocol {}, which hibi did not offer ({PROTOCOL_VERSION})",
                        info.protocol
                    ))
                } else if !info.schemes.contains(&self.scheme) {
                    Err(format!(
                        "hello lists schemes {:?}, which do not include {:?}",
                        info.schemes, self.scheme
                    ))
                } else {
                    Ok(info)
                }
            });
        match outcome {
            Ok(info) => {
                self.state = State::Running {
                    process,
                    info,
                    next_id: 2,
                };
                Ok(())
            }
            Err(why) => Err(self.fail_with(process, why)),
        }
    }

    /// Kill `process`, record the failure, return the `failed` error.
    fn fail_with(&mut self, mut process: Process, why: String) -> ProviderError {
        process.kill();
        let tail = process.stderr_tail();
        let mut message = format!("provider for scheme {:?}: {why}", self.scheme);
        if !tail.is_empty() {
            message.push_str("; its standard error ends with: ");
            message.push_str(&tail);
        }
        let e = ProviderError::failed(message);
        self.state = State::Failed(e.clone());
        e
    }

    /// Send one request and return its outcome. A transport or envelope
    /// violation kills the process and is `failed`, also for every later
    /// request.
    fn call(&mut self, call: &Call) -> Result<Value, ProviderError> {
        self.ensure_started()?;
        let State::Running {
            process, next_id, ..
        } = &mut self.state
        else {
            unreachable!("ensure_started leaves the provider running or returns an error")
        };
        let id = *next_id;
        *next_id += 1;
        match exchange(process, id, &call.to_raw(), self.timeout) {
            Ok(outcome) => outcome,
            Err(why) => {
                let State::Running { process, .. } =
                    std::mem::replace(&mut self.state, State::NotStarted)
                else {
                    unreachable!()
                };
                Err(self.fail_with(process, why))
            }
        }
    }

    fn running_info(&self) -> &ProviderInfo {
        match &self.state {
            State::Running { info, .. } => info,
            _ => unreachable!("called after a successful ensure_started"),
        }
    }

    /// Read a `{"units": [...]}` result and validate it (section 2.5).
    fn units_result(&self, value: Value) -> Result<Vec<Unit>, ProviderError> {
        let units = read_result::<UnitsResult>(value).map_err(|e| invalid_answer(&e.message))?;
        check_units(self.running_info(), &units.units).map_err(|m| invalid_answer(&m))?;
        Ok(units.units)
    }

    /// Answer the anchors at `indexes` with one `resolve` each.
    fn resolve_each(
        &mut self,
        anchors: &[Anchor],
        indexes: &[usize],
        out: &mut [Option<Result<Vec<Unit>, ProviderError>>],
    ) {
        for &i in indexes {
            let call = Call::Resolve(ResolveParams {
                loc: anchors[i].loc.clone(),
                sel: anchors[i].sel.clone(),
            });
            out[i] = Some(self.call(&call).and_then(|v| self.units_result(v)));
        }
    }

    /// Answer the anchors at `indexes` with one `batch` per [`MAX_BATCH`].
    fn resolve_batched(
        &mut self,
        anchors: &[Anchor],
        indexes: &[usize],
        out: &mut [Option<Result<Vec<Unit>, ProviderError>>],
    ) {
        for chunk in indexes.chunks(MAX_BATCH) {
            let requests = chunk
                .iter()
                .map(|&i| {
                    Call::Resolve(ResolveParams {
                        loc: anchors[i].loc.clone(),
                        sel: anchors[i].sel.clone(),
                    })
                    .to_raw()
                })
                .collect::<Vec<RawCall>>();
            let answer = self
                .call(&Call::Batch(BatchParams { requests }))
                .and_then(|v| read_result::<BatchResult>(v).map_err(|e| invalid_answer(&e.message)))
                .and_then(|b| {
                    if b.responses.len() == chunk.len() {
                        Ok(b.responses)
                    } else {
                        Err(invalid_answer(&format!(
                            "a batch of {} requests got {} responses",
                            chunk.len(),
                            b.responses.len()
                        )))
                    }
                });
            match answer {
                Ok(responses) => {
                    for (&i, r) in chunk.iter().zip(responses) {
                        out[i] = Some(r.0.and_then(|v| self.units_result(v)));
                    }
                }
                Err(e) => {
                    for &i in chunk {
                        out[i] = Some(Err(e.clone()));
                    }
                }
            }
        }
    }
}

impl Drop for CommandProvider {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// `internal` for an answer that fails the checks of section 2.5 or does
/// not have the shape of its method's result. The engine maps it to
/// `provider_error` for the affected side or resource.
fn invalid_answer(why: &str) -> ProviderError {
    ProviderError::new(
        ProviderErrorCode::Internal,
        format!("the provider returned an invalid answer: {why}"),
    )
}

/// Write one request and read its response. `Err` is a violation that makes
/// the provider unusable (section 13.1); `Ok` holds the response's result
/// or its error.
fn exchange(
    process: &mut Process,
    id: u64,
    call: &RawCall,
    timeout: Duration,
) -> Result<Result<Value, ProviderError>, String> {
    let line = Request {
        id,
        call: call.clone(),
    }
    .to_line();
    process
        .send(&line)
        .map_err(|e| format!("writing request {id} failed: {e}"))?;
    let bytes = process.read_line(timeout).map_err(|e| match e {
        ReadError::Timeout => format!(
            "no response to request {id} ({}) within {} ms",
            call.method,
            timeout.as_millis()
        ),
        ReadError::TooLong => {
            format!("the response to request {id} is longer than {MAX_LINE_BYTES} bytes")
        }
        ReadError::Ended(None) => {
            format!("the provider exited or closed its output before answering request {id}")
        }
        ReadError::Ended(Some(why)) => {
            format!("reading the response to request {id} failed: {why}")
        }
    })?;
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(v @ Value::Object(_)) => v,
        _ => {
            return Err(format!(
                "the response to request {id} is not a JSON object: {}",
                excerpt(&bytes)
            ));
        }
    };
    let response: Response = serde_json::from_value(value)
        .map_err(|e| format!("the response to request {id} breaks the envelope: {e}"))?;
    if response.id != Some(id) {
        return Err(format!(
            "the response carries id {}, not the request's id {id}",
            response.id.map_or("null".to_owned(), |i| i.to_string())
        ));
    }
    Ok(response.outcome)
}

/// The start of a line, for an error message.
fn excerpt(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]);
    let mut s: String = text.chars().take(120).collect();
    if bytes.len() > 120 {
        s.push_str("...");
    }
    format!("{s:?}")
}

impl Provider for CommandProvider {
    fn info(&mut self) -> Result<&ProviderInfo, ProviderError> {
        self.ensure_started()?;
        Ok(self.running_info())
    }

    fn resolve(&mut self, anchors: &[Anchor]) -> Vec<Result<Vec<Unit>, ProviderError>> {
        if let Err(e) = self.ensure_started() {
            return anchors.iter().map(|_| Err(e.clone())).collect();
        }
        let info = self.running_info();
        let batch = info.capabilities.batch;
        let mut out: Vec<Option<Result<Vec<Unit>, ProviderError>>> = vec![None; anchors.len()];
        let mut send = Vec::new();
        for (i, a) in anchors.iter().enumerate() {
            // A selector with a key the provider does not allow is never
            // written to its input (section 2.5, R62).
            match a.sel.disallowed_key(&info.selectors) {
                Some(k) => {
                    out[i] = Some(Err(ProviderError::new(
                        ProviderErrorCode::InvalidSelector,
                        format!(
                            "not sent: the provider for scheme {:?} does not accept selector key {k}",
                            self.scheme
                        ),
                    )));
                }
                None => send.push(i),
            }
        }
        if batch && send.len() > 1 {
            self.resolve_batched(anchors, &send, &mut out);
        } else {
            self.resolve_each(anchors, &send, &mut out);
        }
        out.into_iter()
            .map(|r| r.expect("every anchor is answered"))
            .collect()
    }

    fn units(&mut self, loc: &Locator, kind: Option<&str>) -> Result<Vec<Unit>, ProviderError> {
        self.ensure_started()?;
        if !self.running_info().capabilities.units {
            return Err(not_offered("units"));
        }
        let call = Call::Units(UnitsParams {
            loc: loc.clone(),
            kind: kind.map(str::to_owned),
        });
        self.call(&call).and_then(|v| self.units_result(v))
    }

    fn render(&mut self, loc: &Locator, unit: &str) -> Result<Rendered, ProviderError> {
        self.ensure_started()?;
        if !self.running_info().capabilities.render {
            return Err(not_offered("render"));
        }
        let call = Call::Render(RenderParams {
            loc: loc.clone(),
            unit: unit.to_owned(),
        });
        self.call(&call)
            .and_then(|v| read_result::<Rendered>(v).map_err(|e| invalid_answer(&e.message)))
    }
}

/// A method the provider's `hello` capabilities do not offer is not sent.
fn not_offered(method: &str) -> ProviderError {
    ProviderError::new(
        ProviderErrorCode::UnknownMethod,
        format!("not sent: the provider does not offer {method} (hello capabilities)"),
    )
}
