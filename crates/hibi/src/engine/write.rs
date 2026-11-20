//! The write commands of spec section 15.3: `claim`, `mark`, `reanchor`,
//! `retire`. Each builds one [`Batch`] and calls [`Store::append`] once;
//! when any element fails nothing is appended.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::Serialize;

use crate::canon::{self, RecordKind};
use crate::duration;
use crate::error::{Code, Error};
use crate::git::GitStatus;
use crate::model::{Anchor, Claim, ClaimId, Mark, MarkId, MarkSide, Retirement, RetirementId};
use crate::model::{Timestamp, parse_compact};
use crate::provider::{Providers, Unit};
use crate::store::{Batch, Ledger, Store};

use super::fresh::{Index, ViewOptions, claim_view, evaluate, next_command};
use super::input::{AnchorInput, ClaimInput, MarkInput, ReanchorInput, RetireInput};
use super::resolve::{Resolved, describe_unit, resolve_anchors};
use super::run_warnings;
use super::types::{
    ClaimView, Nullable, Reason, SideState, TEXT_EXCERPT_CHARS, Warning, excerpt, sh_quote,
};

/// One entry of `claims` in the output of `claim`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClaimResult {
    pub id: ClaimId,
    /// `false` when the claim already existed and only received a mark.
    pub created: bool,
    pub mark: MarkId,
    pub claim: ClaimView,
}

/// The output of `claim`. `claims` has one entry per input, in input order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClaimOutput {
    pub claims: Vec<ClaimResult>,
    pub warnings: Vec<Warning>,
}

/// One entry of `marks` in the output of `mark`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MarkResult {
    pub claim: ClaimId,
    pub mark: MarkId,
    pub claim_view: ClaimView,
}

/// The output of `mark`. `marks` has one entry per input, in input order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MarkOutput {
    pub marks: Vec<MarkResult>,
    pub warnings: Vec<Warning>,
}

/// The output of `reanchor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReanchorOutput {
    pub old: ClaimId,
    pub new: ClaimId,
    pub mark: MarkId,
    pub retirement: RetirementId,
    /// The view of the new claim.
    pub claim: ClaimView,
    pub warnings: Vec<Warning>,
}

/// One entry of `retired` in the output of `retire`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetireResult {
    pub claim: ClaimId,
    /// `null` when the claim was already retired.
    #[schemars(with = "Nullable<RetirementId>")]
    pub retirement: Option<RetirementId>,
    pub already_retired: bool,
}

/// The output of `retire`. `retired` has one entry per input, in input
/// order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetireOutput {
    pub retired: Vec<RetireResult>,
    pub warnings: Vec<Warning>,
}

fn side_name(index: usize) -> String {
    if index == 0 {
        "the subject".to_owned()
    } else {
        format!("evidence {index}")
    }
}

/// `side_missing`, `side_ambiguous` or `side_unavailable` for a side that
/// did not resolve to one unit. `unknown_scheme` and `unsupported_selector`
/// are reported instead for a new anchor.
fn unresolved_error(index: usize, anchor: &Anchor, r: &Resolved, new: bool) -> Option<Error> {
    let (state, reason) = r.failure()?;
    let detail = r.failure_message().unwrap_or_default();
    let what = format!("{} ({})", side_name(index), anchor.loc);
    if new {
        if let Resolved::KeyNotAllowed { key } = r {
            return Some(Error::new(
                Code::UnsupportedSelector,
                format!("{what}: the provider does not allow the selector key {key}"),
            ));
        }
        if reason == Reason::SchemeNotDeclared {
            return Some(Error::new(Code::UnknownScheme, format!("{what}: {detail}")));
        }
    }
    Some(match state {
        SideState::Missing => Error::new(
            Code::SideMissing,
            format!("{what} resolves to no unit ({reason}): {detail}"),
        ),
        SideState::Ambiguous => Error::new(Code::SideAmbiguous, format!("{what}: {detail}")),
        _ => {
            let e = Error::new(
                Code::SideUnavailable,
                format!("{what} cannot be evaluated ({reason}): {detail}"),
            );
            match reason {
                Reason::SchemeNotDeclared
                | Reason::ProviderNotRegistered
                | Reason::ProviderFailed
                | Reason::RepoNotAllowed => e,
                _ => e.without_remediation(),
            }
        }
    })
}

/// The write checks of `claim` step 2 for a new anchor: first that applies.
fn check_new<'r>(index: usize, anchor: &Anchor, r: &'r Resolved) -> Result<&'r Unit, Error> {
    if let Some(e) = unresolved_error(index, anchor, r, true) {
        return Err(e);
    }
    let unit = r.unit().expect("no failure means one unit");
    let what = format!("{} ({})", side_name(index), anchor.loc);
    if unit.approx {
        let found = match &unit.text {
            Some(t) => format!(
                "{}, text {:?}",
                describe_unit(unit),
                excerpt(t, TEXT_EXCERPT_CHARS)
            ),
            None => describe_unit(unit),
        };
        return Err(Error::new(
            Code::SelectorNotExact,
            format!("{what} resolved only approximately; found {found}"),
        ));
    }
    if let Some(refuse) = unit.refuse {
        let (code, why) = match refuse {
            crate::provider::Refuse::QuoteSpansUnits => {
                (Code::QuoteSpansUnits, "the quote reaches outside one unit")
            }
            crate::provider::Refuse::QuoteInComment => (
                Code::QuoteInComment,
                "the quote lies wholly or partly inside a code comment",
            ),
        };
        return Err(Error::new(code, format!("{what}: {why}")));
    }
    Ok(unit)
}

/// A resolution as it will be once the completed selector is stored: the
/// same unit, exact.
fn as_stored(r: &Resolved) -> Resolved {
    match r {
        Resolved::One(u) => {
            let mut u = u.clone();
            u.approx = false;
            u.refuse = None;
            Resolved::One(u)
        }
        other => other.clone(),
    }
}

fn mark_sides(anchors: &[Anchor], units: &[&Unit]) -> Vec<MarkSide> {
    anchors
        .iter()
        .zip(units)
        .map(|(a, u)| MarkSide {
            loc: a.loc.clone(),
            fp: u.fp.clone(),
        })
        .collect()
}

/// `recheck` checked by section 9.4: it parses and `add(now, recheck)`
/// stays in range.
fn check_recheck(recheck: &str, now: Timestamp) -> Result<(), Error> {
    let d = duration::parse(recheck)?;
    duration::add(now.datetime(), &d).map(|_| ())
}

fn resolve_claim_id<'a>(ledger: &'a Ledger, arg: &str) -> Result<&'a Claim, Error> {
    let id = canon::resolve_prefix(
        arg,
        RecordKind::Claim,
        ledger.claims.iter().map(|c| c.id.as_str()),
    )?;
    ledger
        .claim(id)
        .ok_or_else(|| Error::internal(format!("claim {id} disappeared from the ledger")))
}

fn push_mark(batch: &mut Batch, mark: &Mark) {
    if !batch.marks.iter().any(|m| m.id == mark.id) {
        batch.marks.push(mark.clone());
    }
}

/// The view of `claim` after the marks of `batch` are stored.
fn view_after(
    claim: &Claim,
    index: &Index<'_>,
    batch: &Batch,
    resolved: &[Resolved],
    options: ViewOptions,
    now: Timestamp,
) -> Result<ClaimView, Error> {
    let mut marks: Vec<&Mark> = index.marks_of(claim.id.as_str()).to_vec();
    for m in &batch.marks {
        if m.claim == claim.id && !marks.iter().any(|x| x.id == m.id) {
            marks.push(m);
        }
    }
    let eval = evaluate(claim, &marks, false, resolved, now)?;
    Ok(claim_view(claim, &eval, resolved, options, &|id| {
        index.is_retired(id)
    }))
}

fn finish(errors: Vec<Error>, indexed: bool) -> Vec<Error> {
    errors
        .into_iter()
        .map(|mut e| {
            if !indexed {
                e.index = None;
            }
            e
        })
        .collect()
}

// ---------------------------------------------------------------------------
// claim
// ---------------------------------------------------------------------------

struct NewClaim {
    statement: String,
    anchors: Vec<Anchor>,
    recheck: Option<String>,
    note: Option<String>,
    by: Option<String>,
}

fn prepare_claim(
    input: &ClaimInput,
    providers: &mut Providers,
    now: Timestamp,
) -> Result<NewClaim, Error> {
    let statement = input.statement.trim_matches(char::is_whitespace);
    if statement.is_empty() {
        return Err(Error::invalid_argument("the statement is empty"));
    }
    let mut anchors = Vec::with_capacity(1 + input.evidence.len());
    for a in std::iter::once(&input.subject).chain(&input.evidence) {
        let anchor = a.parse()?;
        if let Err(crate::provider::NoProvider::SchemeNotDeclared) =
            providers.provider_for_scheme(anchor.loc.scheme())
        {
            return Err(Error::new(
                Code::UnknownScheme,
                format!(
                    "{}: the scheme {} is not file, not repo and not declared in .hibi/hibi.toml",
                    anchor.loc,
                    anchor.loc.scheme()
                ),
            ));
        }
        anchors.push(anchor);
    }
    if let Some(r) = &input.recheck {
        check_recheck(r, now)?;
    }
    Ok(NewClaim {
        statement: statement.to_owned(),
        anchors,
        recheck: input.recheck.clone(),
        note: input.note.clone(),
        by: input.by.clone(),
    })
}

/// One element of `claim` after its anchors resolved: the claim built from
/// the completed selectors, and the resolutions it was built from.
struct Built<'p> {
    index: usize,
    input: &'p NewClaim,
    claim: Claim,
    resolved: Vec<Resolved>,
}

/// True when `existing`, whose stored anchors resolve to `resolved`, is
/// the same claim as `new` in the sense a caller means: equal statement,
/// `recheck` and `note`, the same number of sides, and for each side the
/// same locator, the same resolved unit, and an equal completed `quote`
/// and `symbol`. The context and position of a quote (`prefix`, `suffix`,
/// `pos`) do not count: they change when text near the quote is edited.
fn same_claim(existing: &Claim, resolved: &[Resolved], new: &Built<'_>) -> bool {
    same_fields(existing, &new.claim)
        && existing
            .sides()
            .zip(resolved)
            .zip(new.claim.sides().zip(&new.resolved))
            .all(|((old, old_r), (side, new_r))| {
                old.sel.quote == side.sel.quote
                    && old.sel.symbol == side.sel.symbol
                    && match (old_r.unit(), new_r.unit()) {
                        (Some(a), Some(b)) => a.key == b.key,
                        _ => false,
                    }
            })
}

/// The part of [`same_claim`] that needs no resolution.
fn same_fields(existing: &Claim, new: &Claim) -> bool {
    existing.statement == new.statement
        && existing.recheck == new.recheck
        && existing.note == new.note
        && existing.side_count() == new.side_count()
        && existing
            .sides()
            .zip(new.sides())
            .all(|(a, b)| a.loc == b.loc)
}

/// `hibi claim` (section 15.3). `inputs` holds one element for the argument
/// form and the array for `--stdin`; with `indexed` every error carries the
/// `index` of its element. All or nothing: when any element fails, every
/// failing element is reported (its first failing check) and nothing is
/// written.
///
/// An element adds a mark only, and reports `created: false`, when its
/// claim already exists and is live. That is the claim with the same id,
/// or else a live claim that is the same claim by [`same_claim`]: one whose
/// stored context or position went out of date because text near its quote
/// was edited. When several such claims exist the one with the smallest id
/// is marked.
pub fn claim(
    store: &dyn Store,
    providers: &mut Providers,
    git: &GitStatus,
    inputs: &[ClaimInput],
    indexed: bool,
    now: Timestamp,
) -> Result<ClaimOutput, Vec<Error>> {
    let ledger = store.load().map_err(|e| vec![e])?;
    let index = Index::new(&ledger);
    let mut errors: Vec<Error> = Vec::new();

    // Step 1.
    let mut prepared: Vec<Option<NewClaim>> = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        match prepare_claim(input, providers, now) {
            Ok(p) => prepared.push(Some(p)),
            Err(e) => {
                errors.push(e.with_index(i));
                prepared.push(None);
            }
        }
    }

    // Step 2: one resolution pass for every anchor of every element.
    let all: Vec<&Anchor> = prepared.iter().flatten().flat_map(|p| &p.anchors).collect();
    let mut resolved = resolve_anchors(providers, &all).into_iter();
    let warnings = run_warnings(&ledger, git, all.iter().copied()).map_err(|e| vec![e])?;

    // Step 3: the claim of each element, from the completed selectors.
    let mut built: Vec<Built<'_>> = Vec::new();
    for (i, p) in prepared.iter().enumerate() {
        let Some(p) = p else { continue };
        let rs: Vec<Resolved> = resolved.by_ref().take(p.anchors.len()).collect();
        let claim = (|| -> Result<Claim, Error> {
            let mut stored = Vec::with_capacity(rs.len());
            for (n, (a, r)) in p.anchors.iter().zip(&rs).enumerate() {
                let unit = check_new(n, a, r)?;
                stored.push(Anchor::new(a.loc.clone(), unit.sel.clone()));
            }
            let subject = stored.remove(0);
            Claim::new(
                &p.statement,
                subject,
                stored,
                p.recheck.clone(),
                p.note.clone(),
            )
        })();
        match claim {
            Err(e) => errors.push(e.with_index(i)),
            Ok(claim) => built.push(Built {
                index: i,
                input: p,
                claim,
                resolved: rs,
            }),
        }
    }

    // An element whose id is not that of a live claim may still be an
    // existing claim: resolve the stored anchors of every live claim that
    // agrees with it on the fields that need no resolution, in one pass.
    let is_live = |id: &str| index.claim(id).is_some() && !index.is_retired(id);
    let mut candidates: Vec<&Claim> = Vec::new();
    for b in built.iter().filter(|b| !is_live(b.claim.id.as_str())) {
        for c in index.live().filter(|c| same_fields(c, &b.claim)) {
            if !candidates.iter().any(|x| x.id == c.id) {
                candidates.push(c);
            }
        }
    }
    let candidate_anchors: Vec<&Anchor> = candidates.iter().flat_map(|c| c.sides()).collect();
    let mut candidate_resolved = resolve_anchors(providers, &candidate_anchors).into_iter();
    let candidates: Vec<(&Claim, Vec<Resolved>)> = candidates
        .into_iter()
        .map(|c| {
            let rs = candidate_resolved.by_ref().take(c.side_count()).collect();
            (c, rs)
        })
        .collect();

    // Step 4: the mark of each element, on its own claim or on the live
    // claim it is the same as.
    let mut batch = Batch::default();
    let mut results: Vec<(Claim, bool, MarkId, Vec<Resolved>)> = Vec::new();
    for b in &built {
        let id = b.claim.id.as_str();
        let existing = if is_live(id) {
            None
        } else {
            candidates
                .iter()
                .filter(|(c, rs)| same_claim(c, rs, b))
                .min_by(|(x, _), (y, _)| x.id.cmp(&y.id))
        };
        let (target, target_resolved): (&Claim, Vec<Resolved>) = match existing {
            Some((c, rs)) => (c, rs.clone()),
            None => (&b.claim, b.resolved.iter().map(as_stored).collect()),
        };
        let marked = (|| -> Result<Mark, Error> {
            if existing.is_none() && index.is_retired(id) {
                return Err(Error::new(
                    Code::ClaimRetired,
                    format!(
                        "the claim {id} with this statement, these anchors and this note is retired"
                    ),
                ));
            }
            let anchors: Vec<Anchor> = target.sides().cloned().collect();
            let units: Vec<&Unit> = target_resolved.iter().filter_map(Resolved::unit).collect();
            if units.len() != anchors.len() {
                return Err(Error::internal(format!(
                    "claim {} lost a resolved side while it was being marked",
                    target.id
                )));
            }
            Mark::new(
                target.id.clone(),
                now,
                b.input.by.clone(),
                mark_sides(&anchors, &units),
                None,
            )
        })();
        match marked {
            Err(e) => errors.push(e.with_index(b.index)),
            Ok(mark) => {
                let exists = index.claim(target.id.as_str()).is_some()
                    || batch.claims.iter().any(|c| c.id == target.id);
                if !exists {
                    batch.claims.push(target.clone());
                }
                push_mark(&mut batch, &mark);
                results.push((target.clone(), !exists, mark.id, target_resolved));
            }
        }
    }
    if !errors.is_empty() {
        errors.sort_by_key(|e| e.index);
        return Err(finish(errors, indexed));
    }

    let mut claims = Vec::with_capacity(results.len());
    for (claim, created, mark, rs) in &results {
        let view = view_after(claim, &index, &batch, rs, ViewOptions::default(), now)
            .map_err(|e| vec![e])?;
        claims.push(ClaimResult {
            id: claim.id.clone(),
            created: *created,
            mark: mark.clone(),
            claim: view,
        });
    }
    store.append(&batch).map_err(|e| vec![e])?;
    Ok(ClaimOutput { claims, warnings })
}

// ---------------------------------------------------------------------------
// mark
// ---------------------------------------------------------------------------

/// `hibi mark` (section 15.3). One element per claim; with `indexed` every
/// error carries the `index` of its element. All or nothing, as for
/// [`claim`].
///
/// A side that is `missing`, `ambiguous` or `unavailable` fails; a profile
/// mismatch and an approximate resolution are accepted. In the output every
/// approximate side carries its unit and its text cut to 240 characters.
pub fn mark(
    store: &dyn Store,
    providers: &mut Providers,
    git: &GitStatus,
    inputs: &[MarkInput],
    indexed: bool,
    now: Timestamp,
) -> Result<MarkOutput, Vec<Error>> {
    let ledger = store.load().map_err(|e| vec![e])?;
    let index = Index::new(&ledger);
    let mut errors: Vec<Error> = Vec::new();

    let mut targets: Vec<Option<&Claim>> = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        let target = resolve_claim_id(&ledger, &input.claim).and_then(|c| {
            if index.is_retired(c.id.as_str()) {
                Err(Error::new(
                    Code::ClaimRetired,
                    format!("the claim {} is retired and cannot be marked", c.id),
                )
                .without_remediation())
            } else {
                Ok(c)
            }
        });
        match target {
            Ok(c) => targets.push(Some(c)),
            Err(e) => {
                errors.push(e.with_index(i));
                targets.push(None);
            }
        }
    }

    let all: Vec<&Anchor> = targets.iter().flatten().flat_map(|c| c.sides()).collect();
    let mut resolved = resolve_anchors(providers, &all).into_iter();
    let warnings = run_warnings(&ledger, git, all.iter().copied()).map_err(|e| vec![e])?;

    let mut batch = Batch::default();
    let mut results: Vec<(&Claim, MarkId, Vec<Resolved>)> = Vec::new();
    for (i, (input, target)) in inputs.iter().zip(&targets).enumerate() {
        let Some(claim) = *target else { continue };
        let rs: Vec<Resolved> = resolved.by_ref().take(claim.side_count()).collect();
        let failure = claim
            .sides()
            .zip(&rs)
            .enumerate()
            .find_map(|(n, (a, r))| unresolved_error(n, a, r, false));
        if let Some(e) = failure {
            // The remediation is the claim's `next` (section 15.2).
            let eval = evaluate(claim, index.marks_of(claim.id.as_str()), false, &rs, now)
                .map_err(|e| vec![e])?;
            let e = match next_command(claim, &eval, &rs, &|id| index.is_retired(id)) {
                Some(next) => e.with_remediation(next),
                None => e.without_remediation(),
            };
            errors.push(e.with_index(i));
            continue;
        }
        let units: Vec<&Unit> = rs.iter().filter_map(Resolved::unit).collect();
        let anchors: Vec<Anchor> = claim.sides().cloned().collect();
        let built = Mark::new(
            claim.id.clone(),
            now,
            input.by.clone(),
            mark_sides(&anchors, &units),
            input.note.clone(),
        );
        match built {
            Err(e) => errors.push(e.with_index(i)),
            Ok(m) => {
                push_mark(&mut batch, &m);
                results.push((claim, m.id, rs));
            }
        }
    }
    if !errors.is_empty() {
        errors.sort_by_key(|e| e.index);
        return Err(finish(errors, indexed));
    }

    let options = ViewOptions {
        detail: false,
        approx_excerpt: true,
    };
    let mut marks = Vec::with_capacity(results.len());
    for (claim, mark, rs) in &results {
        let view = view_after(claim, &index, &batch, rs, options, now).map_err(|e| vec![e])?;
        marks.push(MarkResult {
            claim: claim.id.clone(),
            mark: mark.clone(),
            claim_view: view,
        });
    }
    store.append(&batch).map_err(|e| vec![e])?;
    Ok(MarkOutput { marks, warnings })
}

// ---------------------------------------------------------------------------
// reanchor
// ---------------------------------------------------------------------------

/// The command that repeats the caller's arguments and ends with
/// `--note <text>`: the remediation when the rebuilt claim is a retired one.
fn reanchor_with_note(old: &ClaimId, input: &ReanchorInput) -> String {
    let mut cmd = format!("hibi reanchor {old}");
    let mut arg = |flag: &str, value: &str| {
        cmd.push_str(&format!(" {flag} {}", sh_quote(value)));
    };
    if let Some(s) = &input.statement {
        arg("--statement", s);
    }
    if let Some(a) = &input.subject {
        arg("--subject", &a.to_argument());
    }
    for (n, a) in &input.evidence {
        arg("--evidence", &format!("{n}={}", a.to_argument()));
    }
    for a in &input.add_evidence {
        arg("--add-evidence", &a.to_argument());
    }
    for n in &input.drop_evidence {
        arg("--drop-evidence", &n.to_string());
    }
    if let Some(r) = &input.recheck {
        arg("--recheck", r);
    }
    if let Some(b) = &input.by {
        arg("--by", b);
    }
    cmd.push_str(" --note <text>");
    cmd
}

/// `hibi reanchor` (section 15.3): build a new claim from an old one.
///
/// A replaced or added side must pass the write checks of [`claim`]; a kept
/// side may be approximate and its `refuse` is not checked. Every stored
/// selector becomes the completed selector of its resolved unit. The result
/// is `no_change` when the new id equals the old one and `claim_retired`
/// when it equals a retired claim's. When it equals another live claim's,
/// that claim gets the mark and no claim is appended. One `append` holds
/// the new claim, its mark and the retirement of the old claim with
/// `replacedBy`.
pub fn reanchor(
    store: &dyn Store,
    providers: &mut Providers,
    git: &GitStatus,
    input: &ReanchorInput,
    now: Timestamp,
) -> Result<ReanchorOutput, Error> {
    let ledger = store.load()?;
    let index = Index::new(&ledger);
    let old = resolve_claim_id(&ledger, &input.claim)?;
    if index.is_retired(old.id.as_str()) {
        return Err(Error::new(
            Code::ClaimRetired,
            format!("the claim {} is retired and cannot be re-anchored", old.id),
        )
        .without_remediation());
    }

    // Step 1.
    let statement = input.statement.as_deref().unwrap_or(&old.statement);
    let recheck = match input.recheck.as_deref() {
        None => old.recheck.clone(),
        Some("none") => None,
        Some(r) => Some(r.to_owned()),
    };
    if let Some(r) = &recheck {
        check_recheck(r, now)?;
    }
    let note = match input.note.as_deref() {
        None => old.note.clone(),
        Some("none") => None,
        Some(n) => Some(n.to_owned()),
    };
    let by = input.by.clone();

    // Step 2.
    let count = old.evidence.len();
    let mut named: BTreeSet<usize> = BTreeSet::new();
    for n in input
        .evidence
        .iter()
        .map(|(n, _)| *n)
        .chain(input.drop_evidence.iter().copied())
    {
        if n == 0 || n > count {
            return Err(Error::usage(format!(
                "the claim {} has no evidence number {n}; it has {count}",
                old.id
            )));
        }
        if !named.insert(n) {
            return Err(Error::usage(format!(
                "evidence number {n} is named more than once"
            )));
        }
    }
    // Each side: its anchor, and whether it is new (replaced or added).
    let mut plan: Vec<(Anchor, bool)> = Vec::new();
    plan.push(match &input.subject {
        Some(a) => (a.parse()?, true),
        None => (old.subject.clone(), false),
    });
    for (i, kept) in old.evidence.iter().enumerate() {
        let n = i + 1;
        if input.drop_evidence.contains(&n) {
            continue;
        }
        match input.evidence.iter().find(|(m, _)| *m == n) {
            Some((_, a)) => plan.push((a.parse()?, true)),
            None => plan.push((kept.clone(), false)),
        }
    }
    for a in &input.add_evidence {
        plan.push((a.parse()?, true));
    }

    // Step 3.
    let anchors: Vec<&Anchor> = plan.iter().map(|(a, _)| a).collect();
    let resolved = resolve_anchors(providers, &anchors);
    let warnings = run_warnings(&ledger, git, anchors.iter().copied())?;
    let mut units: Vec<&Unit> = Vec::with_capacity(plan.len());
    for (n, ((anchor, new), r)) in plan.iter().zip(&resolved).enumerate() {
        if *new {
            units.push(check_new(n, anchor, r)?);
        } else if let Some(e) = unresolved_error(n, anchor, r, false) {
            return Err(e);
        } else {
            units.push(r.unit().expect("no failure means one unit"));
        }
    }

    // Step 4.
    let mut stored: Vec<Anchor> = plan
        .iter()
        .zip(&units)
        .map(|((a, _), u)| Anchor::new(a.loc.clone(), u.sel.clone()))
        .collect();
    let sides = mark_sides(&stored, &units);
    let subject = stored.remove(0);
    let new = Claim::new(statement, subject, stored, recheck, note)?;

    // Step 5.
    if new.id == old.id {
        return Err(Error::new(
            Code::NoChange,
            format!("re-anchoring {} would produce the same claim", old.id),
        ));
    }
    if index.is_retired(new.id.as_str()) {
        return Err(Error::new(
            Code::ClaimRetired,
            format!(
                "re-anchoring {} would produce the retired claim {}",
                old.id, new.id
            ),
        )
        .with_remediation(reanchor_with_note(&old.id, input)));
    }

    // Step 6.
    let mut batch = Batch::default();
    if index.claim(new.id.as_str()).is_none() {
        batch.claims.push(new.clone());
    }
    let mark = Mark::new(new.id.clone(), now, by.clone(), sides, None)?;
    let retirement = Retirement::new(old.id.clone(), Some(new.id.clone()), now, by, None)?;
    batch.marks.push(mark.clone());
    batch.retirements.push(retirement.clone());

    let stored_resolutions: Vec<Resolved> = resolved.iter().map(as_stored).collect();
    let view = view_after(
        &new,
        &index,
        &batch,
        &stored_resolutions,
        ViewOptions::default(),
        now,
    )?;
    store.append(&batch)?;
    Ok(ReanchorOutput {
        old: old.id.clone(),
        new: new.id,
        mark: mark.id,
        retirement: retirement.id,
        claim: view,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// retire
// ---------------------------------------------------------------------------

/// `hibi retire` (section 15.3): one retirement per claim that is not
/// already retired. A claim named more than once counts once: the output
/// has one entry per claim, in the order of first occurrence. No provider is contacted. `replaced_by` must name an
/// existing claim that is not among those being retired; an unknown id or
/// one of the retired ids is `invalid_argument`.
pub fn retire(
    store: &dyn Store,
    input: &RetireInput,
    now: Timestamp,
) -> Result<RetireOutput, Error> {
    let ledger = store.load()?;
    let index = Index::new(&ledger);
    let warnings = Warning::from_store_all(&ledger.warnings)?;
    // The same claim named twice (by id or by two prefixes) is one target,
    // at its first position.
    let mut targets: Vec<&Claim> = Vec::with_capacity(input.claims.len());
    for arg in &input.claims {
        let claim = resolve_claim_id(&ledger, arg)?;
        if !targets.iter().any(|t| t.id == claim.id) {
            targets.push(claim);
        }
    }
    let replaced_by = match &input.replaced_by {
        None => None,
        Some(arg) => {
            let c = resolve_claim_id(&ledger, arg).map_err(|e| match e.code {
                Code::ClaimNotFound => e.recode(Code::InvalidArgument),
                _ => e,
            })?;
            if targets.iter().any(|t| t.id == c.id) {
                return Err(Error::invalid_argument(format!(
                    "--replaced-by names {}, which is one of the claims being retired",
                    c.id
                )));
            }
            Some(c.id.clone())
        }
    };
    let by = input.by.clone();
    let note = input.note.clone();

    let mut batch = Batch::default();
    let mut retired = Vec::with_capacity(targets.len());
    for claim in targets {
        if index.is_retired(claim.id.as_str()) {
            retired.push(RetireResult {
                claim: claim.id.clone(),
                retirement: None,
                already_retired: true,
            });
            continue;
        }
        let r = Retirement::new(
            claim.id.clone(),
            replaced_by.clone(),
            now,
            by.clone(),
            note.clone(),
        )?;
        batch.retirements.push(r.clone());
        retired.push(RetireResult {
            claim: claim.id.clone(),
            retirement: Some(r.id),
            already_retired: false,
        });
    }
    if !batch.is_empty() {
        store.append(&batch)?;
    }
    Ok(RetireOutput { retired, warnings })
}

/// Parse one compact anchor argument (section 5.5). Errors are
/// `invalid_anchor`.
pub fn compact_anchor(arg: &str) -> Result<AnchorInput, Error> {
    parse_compact(arg).map(|_| AnchorInput::Compact(arg.to_owned()))
}
