//! Freshness, coverage, `--since`, and one function per command (spec
//! sections 9, 10 and 15).
//!
//! The engine reads records through [`crate::store::Store`] and resolves
//! anchors through [`crate::provider::Providers`]; it uses nothing else
//! from a store or a provider. It never reads the clock: `now` is a
//! parameter everywhere. It compiles with every feature off.
//!
//! Ordering is deterministic: claims by id; sides by index; `list` records
//! by `at`, then id; scopes in `hibi.toml` order; uncovered units by
//! locator, start offset, key; unavailable resources by locator; batch
//! results and errors in input order.
//!
//! | Command | Function | Output |
//! |---|---|---|
//! | `claim` | [`claim`] ([`parse_claim_stdin`]) | [`ClaimOutput`] |
//! | `mark` | [`mark`] ([`parse_mark_stdin`]) | [`MarkOutput`] |
//! | `reanchor` | [`reanchor`] | [`ReanchorOutput`] |
//! | `retire` | [`retire`] | [`RetireOutput`] |
//! | `status` | [`status`] ([`resolve_since`]) | [`StatusOutput`] |
//! | `list` | [`list`] | [`ListOutput`] |
//! | `cover` | [`cover`] | [`CoverOutput`] |
//! | `providers` | [`providers_report`] | [`ProvidersOutput`] |
//!
//! For the judge: [`coverage`] and [`unit_texts`] give the uncovered units
//! with their text; [`status`] and [`render_sides`] give the stale claims
//! with the rendered text of each side.

mod cover;
mod fresh;
mod input;
mod read;
mod report;
mod resolve;
mod types;
mod write;

pub use cover::{
    CoverEnv, CoverItem, CoverOutput, CoverQuery, CoverSummary, Coverage, DEFAULT_COVER_LIMIT,
    ExclusionReport, ScopeCoverage, ScopeReport, Skipped, UnavailableResource, UncoveredUnit,
    UnitCoverage, changed_since, classify, cover, coverage, covered_keys, unit_texts,
};
pub use fresh::{Evaluation, Index, ViewOptions, accepted, claim_view, evaluate, next_command};
pub use input::{
    AnchorInput, ClaimInput, MarkInput, ReanchorInput, RetireInput, parse_claim_stdin,
    parse_mark_stdin, read_claim_stdin, read_mark_stdin,
};
pub use read::{
    ListItem, ListOutput, ListQuery, LocFilter, Since, SinceRun, SinceView, StatusOutput,
    StatusQuery, StatusSummary, list, render_sides, resolve_since, status,
};
pub use report::{
    Builtin, ProvidersOutput, RepoReport, SchemeReport, providers_report, repo_reports,
    scheme_reports,
};
pub use resolve::{
    Candidates, MAX_CANDIDATES, Resolved, content_candidates, quoted, reason_of_no_provider,
    resolve_anchors, resolve_with, side_of_error,
};
pub use types::{
    Accepted, AcceptedSide, Candidate, ClaimState, ClaimView, DETAIL_CHARS, Envelope,
    ErrorEnvelope, NEXT_NOTE, Reason, Role, Side, SideState, TEXT_EXCERPT_CHARS, UnitRef, Warning,
    WarningCode, excerpt, sh_quote, side_remediation, unavailable_remediation,
};
pub use write::{
    ClaimOutput, ClaimResult, MarkOutput, MarkResult, ReanchorOutput, RetireOutput, RetireResult,
    claim, claim_batch, compact_anchor, mark, mark_batch, reanchor, retire,
};

use crate::error::Error;
use crate::git::GitStatus;
use crate::model::Anchor;
use crate::store::Ledger;

/// Resolve a claim id argument (section 3.6) against `ids`. An argument
/// that is not an id or an id prefix is `invalid_argument` and says how to
/// find the ids.
pub(crate) fn resolve_claim_arg<'a>(
    arg: &str,
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<&'a str, Error> {
    crate::canon::resolve_prefix(arg, crate::canon::RecordKind::Claim, ids).map_err(|e| {
        if e.code == crate::error::Code::InvalidArgument && e.remediation.is_none() {
            e.with_remediation(
                "Give a claim id as `hibi list` prints it, or its beginning: c1- and at least four more characters.",
            )
        } else {
            e
        }
    })
}

/// The warnings of a run that resolved `anchors`: the store's warnings,
/// then `git_too_old` once when git is older than `MIN_GIT` and a `repo:`
/// anchor was among them (section 10.4).
pub(crate) fn run_warnings<'a>(
    ledger: &Ledger,
    git: &GitStatus,
    anchors: impl IntoIterator<Item = &'a Anchor>,
) -> Result<Vec<Warning>, Error> {
    let mut warnings = Warning::from_store_all(&ledger.warnings)?;
    if let Some(w) = Warning::git_too_old(git)
        && anchors.into_iter().any(|a| a.loc.scheme() == "repo")
    {
        warnings.push(w);
    }
    Ok(warnings)
}
