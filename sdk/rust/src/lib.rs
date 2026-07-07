//! Hibi — Rust resolver SDK (§7.1, §12).
//!
//! A thin SDK for authoring an out-of-process resolver in Rust. Implement the
//! [`Resolver`] trait and call [`serve`]; the SDK owns the JSONL-RPC framing and
//! dispatch over stdio. The protocol types mirror the canonical Zod model
//! (`schemas/*.v1.json`), the single source of truth.
//!
//! The model is two-axis (ADR-001): an anchor-resolution axis reported per side
//! (`doc` / `code`, sharing one `AnchorState` vocabulary) and an optional
//! behavioral-belief axis (`BehaviorState`). The side is a separate field, never
//! baked into the state word. All wire keys are camelCase via serde rename.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};

/// One side of a bidirectional [`Anchor`]: a multi-selector bundle resolving
/// against a single `file`. Selectors are kept opaque (`Value`) so the SDK stays
/// forward-compatible with new selector kinds (§4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectorBundle {
    pub file: String,
    pub selectors: Vec<Value>,
}

/// Bidirectional, composite anchor (§4): a doc-side bundle (the documented
/// sentence) plus zero or more code-side bundles (the code it describes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
    pub doc: SelectorBundle,
    #[serde(default)]
    pub code: Vec<SelectorBundle>,
}

/// Executable-evidence link that upgrades behavioral risk to a real verdict
/// (§5/§17.6). Run by an out-of-process runner resolver, never in core.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verifier {
    pub kind: String,
    #[serde(rename = "ref")]
    pub r#ref: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub proves: Option<String>,
}

/// Deterministic blast-radius the change-gate watches for a behavioral claim
/// (§5/§17.6, D14): the anchored file + its imports followed to `depth`, plus
/// `include` globs, minus `exclude` globs. Absent → `depth: 1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorScope {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Import hops to follow: 0, 1, or 2 (default 1).
    #[serde(default = "default_depth")]
    pub depth: u64,
}
fn default_depth() -> u64 {
    1
}

/// Assertion — one verification instance. Carries the bidirectional [`Anchor`],
/// the record's `enforcement` lifecycle, and the behavioral declarations (§5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assertion {
    pub id: String,
    #[serde(rename = "propositionId")]
    pub proposition_id: String,
    #[serde(rename = "documentId")]
    pub document_id: String,
    pub owner: String,
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub anchor: Anchor,
    /// suggested|enforced|retired. Only `enforced` can gate.
    #[serde(default = "default_enforcement")]
    pub enforcement: String,
    /// Author's behavioral declaration (§17.6, D12); absent → heuristic decides.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub behavioral: Option<bool>,
    #[serde(default)]
    pub verifiers: Vec<Verifier>,
    #[serde(
        rename = "behaviorScope",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub behavior_scope: Option<BehaviorScope>,
    /// The change-gate evidence baseline (§17.6, D14): path → xxHash64 hex.
    #[serde(
        rename = "evidenceBaseline",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub evidence_baseline: Option<HashMap<String, String>>,
    /// Authored suppression of a behavioral `at-risk` (§17.6, D14; `hibi ignore`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub suppressed: Option<Suppressed>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ttl: Option<String>,
    #[serde(default)]
    pub attrs: Value,
}

/// Authored suppression of a behavioral `at-risk` (§17.6, D14). `paths` is the
/// acknowledged `{path → hash}` map; the suppression lapses when any path's hash
/// moves or a new evidence path appears.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suppressed {
    pub paths: HashMap<String, String>,
    pub reason: String,
}
fn default_enforcement() -> String {
    "suggested".to_string()
}

/// Proposition — the timeless meaning. `textCache` is a non-authoritative copy
/// of the documented sentence (audit/diff/recovery only); the authoritative text
/// is the live doc span re-read at check time (§4, §18-B).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposition {
    pub id: String,
    #[serde(rename = "textCache")]
    pub text_cache: String,
    #[serde(rename = "authoredTrust")]
    pub authored_trust: String,
    pub fingerprint: String,
}

/// A located region in the current text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    pub start: u64,
    pub end: u64,
}

/// Per-selector contribution to the fused confidence (§17.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectorScore {
    pub kind: String,
    pub found: bool,
    pub score: f64,
    pub weight: f64,
}

/// Bulky located evidence — trails the decision fields in the JSON shape (§9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerdictEvidence {
    #[serde(rename = "docRegion", skip_serializing_if = "Option::is_none", default)]
    pub doc_region: Option<Region>,
    #[serde(rename = "codeRegions", default)]
    pub code_regions: Vec<Region>,
    pub confidence: f64,
    #[serde(rename = "selectorScores", default)]
    pub selector_scores: Vec<SelectorScore>,
    #[serde(rename = "changedEvidence", default)]
    pub changed_evidence: Vec<Value>,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none", default)]
    pub r#ref: Option<String>,
}

/// Per-Assertion verdict — verdict-first, two-axis (§9). Leads with the decision
/// (the two per-side anchor states, the optional behavioral state, and the
/// `expired`/`gates` flags) and trails the bulky `evidence`. `doc` and `code`
/// share one `AnchorState` vocabulary (unchanged|moved|changed|ambiguous|
/// orphaned); `behavior` is one of unverified|at-risk|supported|refuted, absent
/// on non-behavioral claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    #[serde(rename = "assertionId")]
    pub assertion_id: String,
    #[serde(rename = "propositionId")]
    pub proposition_id: String,
    #[serde(rename = "documentId")]
    pub document_id: String,
    /// Axis 1 — anchor resolution, doc side.
    pub doc: String,
    /// Axis 1 — anchor resolution, code side (aggregated worst over bundles).
    pub code: String,
    /// Axis 2 — behavioral belief; absent on non-behavioral claims.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub behavior: Option<String>,
    /// Orthogonal time flag: past the Assertion's `ttl`.
    pub expired: bool,
    /// Whether this verdict gates the build (exit 2). Only `enforced` claims gate.
    pub gates: bool,
    pub evidence: VerdictEvidence,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub advisories: Vec<Advisory>,
}

/// Advisory note from a quarantined Tier-3 resolver — advises, never gates (§7.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Advisory {
    pub resolver: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub confidence: Option<f64>,
}

/// Result of a `describe` call — advertises this resolver's identity and the
/// claim kinds / verifier kinds it can handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeResult {
    pub name: String,
    pub version: String,
    pub kinds: Vec<String>,
    #[serde(default = "default_tier")]
    pub tier: i64,
    #[serde(default)]
    pub advisory: bool,
    #[serde(rename = "verifierKinds", default)]
    pub verifier_kinds: Vec<String>,
}
fn default_tier() -> i64 {
    1
}

/// The current artifact contents passed to `resolve`/`verify`: the doc file (or
/// `null` if missing) plus a map of code path → contents (or `null` if missing).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResolveFiles {
    pub doc: Option<String>,
    #[serde(default)]
    pub code: HashMap<String, Option<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveParams {
    pub assertion: Assertion,
    pub files: ResolveFiles,
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

/// Params for a `verify` call — run one [`Verifier`] for a behavioral claim and
/// report the resulting [`BehaviorState`] (§5/§17.6).
#[derive(Debug, Clone, Deserialize)]
pub struct VerifyParams {
    pub assertion: Assertion,
    pub verifier: Verifier,
    #[serde(default)]
    pub files: Option<ResolveFiles>,
    #[serde(rename = "changedEvidence", default)]
    pub changed_evidence: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct VerifyResult {
    /// One of unverified|at-risk|supported|refuted.
    pub behavior: String,
    #[serde(default)]
    pub advisories: Vec<Advisory>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Implement this trait, then call [`serve`]. `verify` is optional: the default
/// impl returns `None`, which the engine treats as a non-gating `unverified`
/// (back-compat — a resolver that only does anchor resolution need not change).
pub trait Resolver {
    fn describe(&self) -> DescribeResult;
    fn resolve(&self, params: ResolveParams) -> ResolveResult;
    fn verify(&self, _params: VerifyParams) -> Option<VerifyResult> {
        None
    }
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
            "verify" => match serde_json::from_value::<VerifyParams>(req.params) {
                Ok(params) => match resolver.verify(params) {
                    Some(result) => write_line(
                        &mut stdout,
                        &RpcResponse {
                            id: req.id,
                            result: Some(result),
                            error: None,
                        },
                    ),
                    None => write_line(
                        &mut stdout,
                        &RpcResponse::<VerifyResult> {
                            id: req.id,
                            result: None,
                            error: Some(RpcError {
                                message: "verify not supported by this resolver".to_string(),
                                code: -1,
                            }),
                        },
                    ),
                },
                Err(e) => write_line(
                    &mut stdout,
                    &RpcResponse::<VerifyResult> {
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
