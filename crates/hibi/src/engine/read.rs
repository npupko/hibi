//! The read commands `status` and `list` (spec section 15.3), and
//! `status --since` (section 10.3).

use std::collections::BTreeSet;
use std::path::Path;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use schemars::JsonSchema;
use serde::Serialize;

use crate::error::Error;
use crate::git::{Git, GitStatus, ObjectId, parse_range};
use crate::model::{Anchor, Claim, ClaimId, Locator, LocatorParts, Mark, Retirement, Timestamp};
use crate::provider::{Provider, Providers};
use crate::store::{Ledger, Store};

use super::fresh::{Index, ViewOptions, claim_view, evaluate};
use super::resolve::{Resolved, resolve_anchors, resolve_with};
use super::run_warnings;
use super::types::{ClaimState, ClaimView, NEXT_NOTE, Nullable, Warning};

/// A parsed `--since` range with its base commit (section 10.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Since {
    /// The range as the caller wrote it.
    pub range: String,
    pub base: ObjectId,
}

/// `since` in the output of `status` and `cover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SinceView {
    pub range: String,
    /// The base commit id.
    pub base: String,
}

impl From<&Since> for SinceView {
    fn from(s: &Since) -> Self {
        SinceView {
            range: s.range.clone(),
            base: s.base.as_str().to_owned(),
        }
    }
}

/// Parse a `--since` argument and find its base commit. A form other than
/// `A`, `A..`, `A..HEAD`, `A...`, `A...HEAD`, or an `A` that is not a
/// commit, is `invalid_range`. The caller builds the base-side provider for
/// `Since::base` and passes both to [`status`] or [`super::cover`].
pub fn resolve_since(git: &Git, root: &Path, range: &str) -> Result<Since, Error> {
    let parsed = parse_range(range)?;
    let base = git.since_base(root, &parsed)?;
    Ok(Since {
        range: range.to_owned(),
        base,
    })
}

/// What a `--since` evaluation needs besides the registry: the range, a
/// provider for scheme `file` that reads the base commit, and git for the
/// changed paths.
pub struct SinceRun<'a> {
    pub since: &'a Since,
    pub base: &'a mut dyn Provider,
    pub git: &'a Git,
    pub root: &'a Path,
}

/// The selection of `hibi status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusQuery {
    /// Claim ids or id prefixes. When non-empty, exactly these claims are
    /// listed, in any state, and `states` and `locs` are not applied.
    pub ids: Vec<String>,
    /// `--state`. Empty means every claim that needs attention: `stale`,
    /// `expired`, and `fresh` claims that drifted (see [`StatusSummary`]).
    pub states: Vec<ClaimState>,
    /// `--loc` globs, matched against the whole locator string.
    pub locs: Vec<String>,
    /// `--detail`: sides carry their unit text.
    pub detail: bool,
}

/// `summary` in the output of `status`: every claim in the store.
/// `claims` is the sum of `fresh`, `stale`, `expired` and `retired`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct StatusSummary {
    pub claims: usize,
    pub fresh: usize,
    /// How many of the `fresh` claims drifted: a side was found only
    /// approximately, so the claim holds but its stored selector is out of
    /// date and its `next` is a `reanchor` command.
    pub drifted: usize,
    pub stale: usize,
    pub expired: usize,
    pub retired: usize,
}

/// The output of `status`. `claims` is ordered by id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusOutput {
    pub now: Timestamp,
    /// `Store::snapshot_id`.
    pub snapshot: String,
    #[schemars(with = "Nullable<SinceView>")]
    pub since: Option<SinceView>,
    pub summary: StatusSummary,
    pub claims: Vec<ClaimView>,
    pub next_note: String,
    pub warnings: Vec<Warning>,
}

/// A set of `--loc` globs. `*` and `?` stay within one `/`-separated
/// segment, `**` crosses segments; `[...]` and `{a,b}` work as in scope
/// patterns (section 10.1). A glob that does not compile is
/// `invalid_argument`.
pub struct LocFilter {
    set: Option<GlobSet>,
}

impl LocFilter {
    pub fn new(globs: &[String]) -> Result<LocFilter, Error> {
        if globs.is_empty() {
            return Ok(LocFilter { set: None });
        }
        let mut builder = GlobSetBuilder::new();
        for g in globs {
            let glob = GlobBuilder::new(g)
                .literal_separator(true)
                .backslash_escape(true)
                .build()
                .map_err(|e| Error::invalid_argument(format!("{g:?} is not a valid glob: {e}")))?;
            builder.add(glob);
        }
        let set = builder
            .build()
            .map_err(|e| Error::invalid_argument(format!("the --loc globs do not compile: {e}")))?;
        Ok(LocFilter { set: Some(set) })
    }

    /// True when there is no glob, or any glob matches the locator.
    pub fn matches(&self, loc: &Locator) -> bool {
        self.set.as_ref().is_none_or(|s| s.is_match(loc.as_str()))
    }

    /// True when there is no glob, or any side's locator matches any glob.
    pub fn keeps(&self, claim: &Claim) -> bool {
        self.set.is_none() || claim.sides().any(|a| self.matches(&a.loc))
    }
}

/// The path of a `file:` locator.
fn file_path(loc: &Locator) -> Option<&str> {
    match loc.parts() {
        LocatorParts::File { path } => Some(path),
        _ => None,
    }
}

/// The ids of the stale claims a range invalidated (section 10.3): a claim
/// qualifies when one of its sides is on a changed `file:` path, resolved
/// to exactly one unit at the base, and now does not resolve to exactly one
/// unit or resolves to a unit with another fingerprint.
fn invalidated<'a>(
    stale: &[(&'a Claim, &[Resolved])],
    run: &mut SinceRun<'_>,
) -> Result<BTreeSet<&'a str>, Error> {
    let changed = run.git.changed_paths(run.root, &run.since.base)?;
    let mut candidates: Vec<(&'a Claim, &Anchor, &Resolved)> = Vec::new();
    for (claim, resolved) in stale {
        for (anchor, r) in claim.sides().zip(resolved.iter()) {
            if file_path(&anchor.loc).is_some_and(|p| changed.contains(p)) {
                candidates.push((claim, anchor, r));
            }
        }
    }
    let anchors: Vec<&Anchor> = candidates.iter().map(|(_, a, _)| *a).collect();
    let at_base = resolve_with(run.base, &anchors);
    let mut out = BTreeSet::new();
    for ((claim, _, now), base) in candidates.iter().zip(&at_base) {
        let Some(base_unit) = base.unit() else {
            continue;
        };
        if now.unit().is_none_or(|u| u.fp != base_unit.fp) {
            out.insert(claim.id.as_str());
        }
    }
    Ok(out)
}

/// True when a side of a claim resolved only approximately. For a fresh
/// claim this is what "drifted" means.
fn drifted(resolved: &[Resolved]) -> bool {
    resolved.iter().any(|r| r.unit().is_some_and(|u| u.approx))
}

/// `hibi status` (section 15.3): evaluate the freshness of every claim and
/// list the selected ones, ordered by id. Without ids, `--state` or
/// `--since`, the selected claims are the stale, the expired and the
/// drifted fresh ones: every claim whose `next` asks for something.
///
/// Every side of every live claim is resolved once; sides of retired claims
/// are not resolved. `summary` counts every claim before filtering. With
/// `since`, the listed claims are those of section 10.3, further filtered by
/// `locs`; combining it with `ids` or `states` is `usage`.
pub fn status(
    store: &dyn Store,
    providers: &mut Providers,
    git: &GitStatus,
    query: &StatusQuery,
    mut since: Option<SinceRun<'_>>,
    now: Timestamp,
) -> Result<StatusOutput, Error> {
    if since.is_some() && (!query.ids.is_empty() || !query.states.is_empty()) {
        return Err(Error::usage(
            "--since cannot be combined with --state or with claim ids",
        ));
    }
    let filter = LocFilter::new(&query.locs)?;
    let snapshot = store.snapshot_id()?;
    let ledger = store.load()?;
    let index = Index::new(&ledger);

    let wanted: Option<BTreeSet<&str>> = if query.ids.is_empty() {
        None
    } else {
        let all = || ledger.claims.iter().map(|c| c.id.as_str());
        Some(
            query
                .ids
                .iter()
                .map(|arg| super::resolve_claim_arg(arg, all()))
                .collect::<Result<_, _>>()?,
        )
    };

    let live: Vec<&Claim> = index.live().collect();
    let anchors: Vec<&Anchor> = live.iter().flat_map(|c| c.sides()).collect();
    let all_resolved = resolve_anchors(providers, &anchors);
    let warnings = run_warnings(&ledger, git, anchors.iter().copied())?;

    // Per claim in id order: its resolutions (empty when retired).
    let mut offset = 0;
    let mut rows: Vec<(&Claim, &[Resolved])> = Vec::with_capacity(ledger.claims.len());
    for claim in &ledger.claims {
        if index.is_retired(claim.id.as_str()) {
            rows.push((claim, &[]));
        } else {
            let n = claim.side_count();
            rows.push((claim, &all_resolved[offset..offset + n]));
            offset += n;
        }
    }

    let mut summary = StatusSummary {
        claims: rows.len(),
        ..StatusSummary::default()
    };
    let mut evals = Vec::with_capacity(rows.len());
    for (claim, resolved) in &rows {
        let id = claim.id.as_str();
        let eval = evaluate(
            claim,
            index.marks_of(id),
            index.is_retired(id),
            resolved,
            now,
        )?;
        match eval.state {
            ClaimState::Fresh => {
                summary.fresh += 1;
                if drifted(resolved) {
                    summary.drifted += 1;
                }
            }
            ClaimState::Stale => summary.stale += 1,
            ClaimState::Expired => summary.expired += 1,
            ClaimState::Retired => summary.retired += 1,
        }
        evals.push(eval);
    }

    let invalidated_ids = match &mut since {
        None => None,
        Some(run) => {
            let stale: Vec<(&Claim, &[Resolved])> = rows
                .iter()
                .zip(&evals)
                .filter(|(_, e)| e.state == ClaimState::Stale)
                .map(|(row, _)| *row)
                .collect();
            Some(invalidated(&stale, run)?)
        }
    };

    // The default selection is every claim that needs attention.
    let selected = |eval: &super::fresh::Evaluation<'_>, resolved: &[Resolved]| {
        if query.states.is_empty() {
            match eval.state {
                ClaimState::Stale | ClaimState::Expired => true,
                ClaimState::Fresh => drifted(resolved),
                ClaimState::Retired => false,
            }
        } else {
            query.states.contains(&eval.state)
        }
    };
    let options = ViewOptions {
        detail: query.detail,
        approx_excerpt: false,
    };
    let mut claims = Vec::new();
    for ((claim, resolved), eval) in rows.iter().zip(&evals) {
        let id = claim.id.as_str();
        let listed = match (&wanted, &invalidated_ids) {
            (Some(ids), _) => ids.contains(id),
            (None, Some(ids)) => ids.contains(id) && filter.keeps(claim),
            (None, None) => selected(eval, resolved) && filter.keeps(claim),
        };
        if listed {
            claims.push(claim_view(claim, eval, resolved, options, &|id| {
                index.is_retired(id)
            }));
        }
    }

    Ok(StatusOutput {
        now,
        snapshot,
        since: since.as_ref().map(|run| SinceView::from(run.since)),
        summary,
        claims,
        next_note: NEXT_NOTE.to_owned(),
        warnings,
    })
}

/// The text of each side of a claim view as `Provider::render` gives it,
/// for the judge (section 14.5). `None` when a side cannot be rendered: it
/// does not resolve to one unit, its provider has `render: false`, or
/// `render` fails.
pub fn render_sides(providers: &mut Providers, view: &ClaimView) -> Option<Vec<String>> {
    let mut out = Vec::with_capacity(view.sides.len());
    for side in &view.sides {
        let unit = side.unit.as_ref()?;
        let provider = providers.provider_for(&side.anchor.loc).ok()?;
        if !provider.info().ok()?.capabilities.render {
            return None;
        }
        out.push(provider.render(&side.anchor.loc, &unit.key).ok()?.text);
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// The selection of `hibi list`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListQuery {
    /// A claim id or id prefix: show that claim's record and history. The
    /// other fields are then not applied.
    pub id: Option<String>,
    /// `--loc` globs, as in [`StatusQuery`].
    pub locs: Vec<String>,
    /// `--text`: a case-sensitive substring of the statement.
    pub text: Option<String>,
    /// `--retired`: include retired claims.
    pub retired: bool,
}

/// One entry of `claims` in the output of `list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListItem {
    pub id: ClaimId,
    pub statement: String,
    /// The subject's locator.
    pub subject: Locator,
    /// The locator of each evidence anchor.
    pub evidence: Vec<Locator>,
    #[schemars(with = "Nullable<String>")]
    pub recheck: Option<String>,
    #[schemars(with = "Nullable<String>")]
    pub note: Option<String>,
    /// The number of marks.
    pub marks: usize,
    #[schemars(with = "Nullable<Timestamp>")]
    pub last_marked_at: Option<Timestamp>,
    pub retired: bool,
}

/// The output of `list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ListOutput {
    /// Without an id: the selected claims, ordered by id.
    Claims {
        claims: Vec<ListItem>,
        warnings: Vec<Warning>,
    },
    /// With an id: the stored records. `marks` and `retirements` are
    /// ordered by `at`, then id.
    Record {
        claim: Box<Claim>,
        marks: Vec<Mark>,
        retirements: Vec<Retirement>,
        warnings: Vec<Warning>,
    },
}

/// `hibi list` (section 15.3). Reads the store only: no provider is
/// contacted and no state is computed.
pub fn list(store: &dyn Store, query: &ListQuery) -> Result<ListOutput, Error> {
    let ledger: Ledger = store.load()?;
    let warnings = Warning::from_store_all(&ledger.warnings)?;
    let index = Index::new(&ledger);

    if let Some(arg) = &query.id {
        let id = super::resolve_claim_arg(arg, ledger.claims.iter().map(|c| c.id.as_str()))?;
        let claim = index
            .claim(id)
            .ok_or_else(|| Error::internal(format!("claim {id} disappeared from the ledger")))?;
        let mut marks: Vec<Mark> = index.marks_of(id).iter().map(|m| (*m).clone()).collect();
        marks.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
        let mut retirements: Vec<Retirement> = ledger.retirements_of(&claim.id).cloned().collect();
        retirements.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
        return Ok(ListOutput::Record {
            claim: Box::new(claim.clone()),
            marks,
            retirements,
            warnings,
        });
    }

    let filter = LocFilter::new(&query.locs)?;
    let claims = ledger
        .claims
        .iter()
        .filter(|c| query.retired || !index.is_retired(c.id.as_str()))
        .filter(|c| filter.keeps(c))
        .filter(|c| {
            query
                .text
                .as_deref()
                .is_none_or(|t| c.statement.contains(t))
        })
        .map(|c| {
            let marks = index.marks_of(c.id.as_str());
            ListItem {
                id: c.id.clone(),
                statement: c.statement.clone(),
                subject: c.subject.loc.clone(),
                evidence: c.evidence.iter().map(|a| a.loc.clone()).collect(),
                recheck: c.recheck.clone(),
                note: c.note.clone(),
                marks: marks.len(),
                last_marked_at: marks.iter().map(|m| m.at).max(),
                retired: index.is_retired(c.id.as_str()),
            }
        })
        .collect();
    Ok(ListOutput::Claims { claims, warnings })
}
