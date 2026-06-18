//! Hibi — Rust resolver SDK (§7.1, §12).
//!
//! A thin SDK for authoring an out-of-process resolver in Rust. Implement the
//! [`Resolver`] trait and call [`serve`]; the SDK owns the JSONL-RPC framing and
//! dispatch over stdio. The protocol types mirror the canonical Zod model
//! (`schemas/*.v1.json`), the single source of truth.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Write};

/// W3C-style anchor selector (discriminated on `kind`). Opaque payload beyond
/// `kind` keeps the SDK forward-compatible with new selector kinds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
    pub file: String,
    pub selectors: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assertion {
    pub id: String,
    #[serde(rename = "propositionId")]
    pub proposition_id: String,
    #[serde(rename = "documentId")]
    pub document_id: String,
    pub owner: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub anchor: Anchor,
    #[serde(default)]
    pub ttl: Option<String>,
    #[serde(default)]
    pub attrs: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposition {
    pub id: String,
    pub text: String,
    #[serde(rename = "authoredTrust")]
    pub authored_trust: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectorScore {
    pub kind: String,
    pub found: bool,
    pub score: f64,
    pub weight: f64,
}

/// Per-Assertion verdict. `state` is one of fresh|moved|stale|ghost|expired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    #[serde(rename = "assertionId")]
    pub assertion_id: String,
    #[serde(rename = "propositionId")]
    pub proposition_id: String,
    #[serde(rename = "documentId")]
    pub document_id: String,
    pub state: String,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub region: Option<Region>,
    #[serde(rename = "selectorScores", default)]
    pub selector_scores: Vec<SelectorScore>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub r#ref: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub advisories: Vec<Advisory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Advisory {
    pub resolver: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeResult {
    pub name: String,
    pub version: String,
    pub kinds: Vec<String>,
    #[serde(default = "default_tier")]
    pub tier: i64,
    #[serde(default)]
    pub advisory: bool,
}
fn default_tier() -> i64 {
    1
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveParams {
    pub assertion: Assertion,
    pub text: Option<String>,
    #[serde(default)]
    pub proposition: Option<Proposition>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ResolveResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    #[serde(default)]
    pub advisories: Vec<Advisory>,
}

/// Implement this trait, then call [`serve`].
pub trait Resolver {
    fn describe(&self) -> DescribeResult;
    fn resolve(&self, params: ResolveParams) -> ResolveResult;
}

#[derive(Deserialize)]
struct RpcRequest {
    id: i64,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct RpcResponse<T: Serialize> {
    id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Serialize)]
struct RpcError {
    message: String,
    code: i64,
}

fn write_line<T: Serialize>(out: &mut impl Write, msg: &T) {
    let line = serde_json::to_string(msg).unwrap_or_else(|_| "{}".to_string());
    let _ = writeln!(out, "{}", line);
    let _ = out.flush();
}

/// Run the JSONL-RPC serve loop over stdin/stdout until EOF.
pub fn serve<R: Resolver>(resolver: R) {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: RpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        match req.method.as_str() {
            "describe" => write_line(
                &mut stdout,
                &RpcResponse {
                    id: req.id,
                    result: Some(resolver.describe()),
                    error: None,
                },
            ),
            "resolve" => match serde_json::from_value::<ResolveParams>(req.params) {
                Ok(params) => write_line(
                    &mut stdout,
                    &RpcResponse {
                        id: req.id,
                        result: Some(resolver.resolve(params)),
                        error: None,
                    },
                ),
                Err(e) => write_line(
                    &mut stdout,
                    &RpcResponse::<ResolveResult> {
                        id: req.id,
                        result: None,
                        error: Some(RpcError {
                            message: e.to_string(),
                            code: -1,
                        }),
                    },
                ),
            },
            other => write_line(
                &mut stdout,
                &RpcResponse::<ResolveResult> {
                    id: req.id,
                    result: None,
                    error: Some(RpcError {
                        message: format!("unknown method: {}", other),
                        code: -1,
                    }),
                },
            ),
        }
    }
}
