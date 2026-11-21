//! The write commands of spec section 15.3: `claim`, `mark`, `reanchor`,
//! `retire`. Each builds one [`Batch`] and calls [`Store::append`] once;
//! when any element fails nothing is appended.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::Serialize;

use crate::duration;
use crate::error::{Code, Error};
use crate::git::GitStatus;
use crate::model::{Anchor, Claim, ClaimId, Mark, MarkId, MarkSide, Retirement, RetirementId};
use crate::model::{Locator, SelectorKey, Timestamp, parse_compact, same_quote};
use crate::provider::{Providers, Unit};
use crate::store::{Batch, Ledger, Store};

use super::fresh::{Index, ViewOptions, accepted, claim_view, evaluate, next_command};
use super::input::{AnchorInput, ClaimInput, MarkInput, ReanchorInput, RetireInput};
use super::resolve::{Once, Resolved, describe_unit, resolve_anchors};
use super::run_warnings;
use super::types::{
    Accepted, ClaimView, NEXT_NOTE, Nullable, Reason, SideState, TEXT_EXCERPT_CHARS, Warning,
    excerpt, sh_quote, side_remediation,
};

/// One entry of `claims` in the output of `claim`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClaimResult {
    pub id: ClaimId,
    /// `false` when the claim already existed and only received a mark.
    pub created: bool,
    pub mark: MarkId,
    /// For a claim that already existed: the state it was in before this
    /// mark and the sides that were not `same`. `null` for a new claim.
    #[schemars(with = "Nullable<Accepted>")]
    pub accepted: Option<Accepted>,
    pub claim: ClaimView,
}

/// The output of `claim`. `claims` has one entry per input, in input order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClaimOutput {
    pub claims: Vec<ClaimResult>,
    /// How to read `next` and `accepted` (section 15.2).
    pub next_note: String,
    pub warnings: Vec<Warning>,
}

/// One entry of `marks` in the output of `mark`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MarkResult {
    pub claim: ClaimId,
    pub mark: MarkId,
    /// The state the claim was in before this mark and the sides that were
    /// not `same`.
    pub accepted: Accepted,
    pub claim_view: ClaimView,
}

/// The output of `mark`. `marks` has one entry per input, in input order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MarkOutput {
    pub marks: Vec<MarkResult>,
    /// How to read `next` and `accepted` (section 15.2).
    pub next_note: String,
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
    /// True when `new` is a claim that already existed: the old claim was
    /// merged into it, and no claim was written.
    pub merged: bool,
    /// The state the old claim was in before this write and its sides that
    /// were not `same`, by the old claim's side numbers.
    pub accepted: Accepted,
    /// The view of the new claim.
    pub claim: ClaimView,
    /// How to read `next` and `accepted` (section 15.2).
    pub next_note: String,
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
        // The provider refuses the anchor: the caller's anchor is wrong
        // for this resource. The message is the provider's own.
        if reason == Reason::SelectorRefused {
            return Some(
                Error::new(Code::InvalidAnchor, format!("{what}: {detail}")).with_remediation(
                    "Change the anchor as the message says. hibi claim --help describes the anchor forms.",
                ),
            );
        }
    }
    // A new anchor that was found only by similarity: the caller asked for
    // text or a symbol that is not there.
    if new && let Resolved::NotExact(_) = r {
        return Some(Error::new(
            Code::SelectorNotExact,
            format!("{what}: {detail}"),
        ));
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
            match side_remediation(reason, Some(&detail)) {
                Some(fix) => e.with_remediation(fix),
                None => e.without_remediation(),
            }
        }
    })
}

/// Advice for an anchor that matched several units, from the selector keys
/// its provider accepts: a provider that takes no symbol is not told to
/// add one.
fn ambiguous_advice(keys: &[SelectorKey]) -> String {
    let mut ways = Vec::new();
    if keys.contains(&SelectorKey::Symbol) {
        ways.push("a symbol");
    }
    if keys.contains(&SelectorKey::Quote) {
        ways.push("a longer quote");
    }
    if keys.contains(&SelectorKey::Key) {
        ways.push("the unit's key");
    }
    match ways.as_slice() {
        [] => "This provider accepts no selector, so the anchor cannot be narrowed to one unit."
            .to_owned(),
        [one] => format!("Give {one}, so that the anchor matches one unit."),
        [first, rest @ ..] => format!(
            "Give {first}, or {}, so that the anchor matches one unit.",
            rest.join(" or ")
        ),
    }
}

/// Replace the general remediation of `side_ambiguous` and
/// `unsupported_selector` with one that names the selector keys the
/// provider of `loc` accepts. Other errors, and a provider whose keys
/// cannot be read, are left as they are.
fn advise(providers: &mut Providers, loc: &Locator, e: Error) -> Error {
    if !matches!(e.code, Code::SideAmbiguous | Code::UnsupportedSelector) {
        return e;
    }
    let keys: Option<Vec<SelectorKey>> = providers
        .provider_for(loc)
        .ok()
        .and_then(|p| p.info().ok().map(|i| i.selectors.clone()));
    let Some(keys) = keys else { return e };
    match e.code {
        Code::SideAmbiguous => {
            let advice = ambiguous_advice(&keys);
            e.with_remediation(advice)
        }
        _ => {
            let names: Vec<String> = keys.iter().map(ToString::to_string).collect();
            let accepted = if names.is_empty() {
                "This provider accepts only the empty selector.".to_owned()
            } else {
                format!(
                    "Use only the selector keys this provider accepts: {}.",
                    names.join(", ")
                )
            };
            e.with_remediation(accepted)
        }
    }
}

/// The write checks of `claim` step 2 for a new anchor: first that applies.
fn check_new<'r>(index: usize, anchor: &Anchor, r: &'r Resolved) -> Result<&'r Unit, Error> {
    if let Some(e) = unresolved_error(index, anchor, r, true) {
        return Err(e);
    }
    let unit = r.unit().expect("no failure means one unit");
    let what = format!("{} ({})", side_name(index), anchor.loc);
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
    // The caller asked for text that the resolved unit does not hold: the
    // symbol found the unit and the provider replaced the quote.
    if let (Some(asked), Some(completed)) = (&anchor.sel.quote, &unit.sel.quote)
        && !same_quote(asked, completed)
    {
        let text = unit.text.as_deref().map_or_else(String::new, |t| {
            format!(", text {:?}", excerpt(t, TEXT_EXCERPT_CHARS))
        });
        return Err(Error::new(
            Code::SelectorNotExact,
            format!(
                "{what}: the quote {:?} does not occur in the unit the anchor resolves to; found {}{text}",
                excerpt(asked, TEXT_EXCERPT_CHARS),
                describe_unit(unit)
            ),
        )
        .with_remediation(
            "Quote text that occurs in that unit, or leave the quote out to anchor the whole unit.",
        ));
    }
    Ok(unit)
}

/// `invalid_argument` when two evidence sides are the same anchor: equal
/// locator, the same resolved unit and an equal completed selector.
/// `sides` holds the completed anchor and the unit key of every side, the
/// subject first; the subject is not compared.
fn duplicate_evidence(sides: &[(Anchor, &str)]) -> Result<(), Error> {
    for (j, (b, b_key)) in sides.iter().enumerate().skip(2) {
        for (i, (a, a_key)) in sides.iter().enumerate().take(j).skip(1) {
            if a == b && a_key == b_key {
                return Err(Error::invalid_argument(format!(
                    "evidence {i} and evidence {j} are the same anchor ({})",
                    a.loc
                ))
                .with_remediation("Give each evidence anchor once."));
            }
        }
    }
    Ok(())
}

/// A resolution as it will be once the completed selector is stored: the
/// same unit, with nothing to refuse.
fn as_stored(r: &Resolved) -> Resolved {
    match r {
        Resolved::One(u) => {
            let mut u = u.clone();
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
    let id = super::resolve_claim_arg(arg, ledger.claims.iter().map(|c| c.id.as_str()))?;
    ledger
        .claim(id)
        .ok_or_else(|| Error::internal(format!("claim {id} disappeared from the ledger")))
}

/// `claim_retired` for a write on a retired claim. The remediation names
/// the claim that replaced it when there is one, and otherwise shows the
/// retired claim's record.
fn retired_error(ledger: &Ledger, claim: &Claim, what: &str) -> Error {
    let replacement = ledger
        .retirements_of(&claim.id)
        .find_map(|r| r.replaced_by.clone());
    let e = Error::new(
        Code::ClaimRetired,
        format!("the claim {} is retired and cannot be {what}", claim.id),
    );
    match replacement {
        Some(new) => e.with_remediation(format!(
            "The claim was replaced by {new}: run hibi status {new}"
        )),
        None => e.with_remediation(format!("hibi list {}", claim.id)),
    }
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
    Ok(claim_view(claim, &eval, resolved, &[], options))
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

/// One element of `claim` once its mark is built: the claim that is marked,
/// whether this run creates it, the mark, the resolutions of its sides and
/// what the mark took over.
type ClaimOutcome = (Claim, bool, MarkId, Vec<Resolved>, Option<Accepted>);

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
/// same locator, the same resolved unit and an equal completed `symbol`.
/// The completed `quote` must be equal too when the caller gave a quote.
/// When the caller gave none, the provider chose the quote from the unit's
/// content, and it changes whenever that content does: it is then not
/// part of what the caller named. The context and position of a quote
/// (`prefix`, `suffix`, `pos`) never count: they change when text near the
/// quote is edited.
///
/// `new` is the claim built from the completed selectors, `keys` the key
/// of the unit each of its sides resolved to, and `asked_quote` whether
/// the caller gave a quote for that side.
fn same_claim(
    existing: &Claim,
    resolved: &[Resolved],
    new: &Claim,
    keys: &[&str],
    asked_quote: &[bool],
) -> bool {
    same_fields(existing, new)
        && existing
            .sides()
            .zip(resolved)
            .zip(new.sides())
            .zip(keys.iter().zip(asked_quote))
            .all(|(((old, old_r), side), (key, asked))| {
                old.sel.symbol == side.sel.symbol
                    && (!asked || old.sel.quote == side.sel.quote)
                    && old_r.unit().is_some_and(|u| u.key == *key)
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
    let elements: Vec<Result<ClaimInput, Error>> = inputs.iter().cloned().map(Ok).collect();
    claim_elements(store, providers, git, &elements, indexed, now)
}

/// `hibi claim --stdin`: [`claim`] for the elements that
/// [`read_claim_stdin`](super::read_claim_stdin) gave. An element that did
/// not parse is reported with its own error next to the errors of the
/// elements that did, so one run names every failing element, in index
/// order. Nothing is written when any element fails.
pub fn claim_batch(
    store: &dyn Store,
    providers: &mut Providers,
    git: &GitStatus,
    elements: &[Result<ClaimInput, Error>],
    now: Timestamp,
) -> Result<ClaimOutput, Vec<Error>> {
    claim_elements(store, providers, git, elements, true, now)
}

fn claim_elements(
    store: &dyn Store,
    providers: &mut Providers,
    git: &GitStatus,
    inputs: &[Result<ClaimInput, Error>],
    indexed: bool,
    now: Timestamp,
) -> Result<ClaimOutput, Vec<Error>> {
    let ledger = store.load().map_err(|e| vec![e])?;
    let index = Index::new(&ledger);
    let mut errors: Vec<Error> = Vec::new();

    // Step 1.
    let mut prepared: Vec<Option<NewClaim>> = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        let input = match input {
            Ok(input) => input,
            Err(e) => {
                errors.push(e.clone().with_index(i));
                prepared.push(None);
                continue;
            }
        };
        match prepare_claim(input, providers, now) {
            Ok(p) => prepared.push(Some(p)),
            Err(e) => {
                errors.push(e.with_index(i));
                prepared.push(None);
            }
        }
    }

    // Step 2: one resolution pass for every anchor of every element.
    let mut once = Once::default();
    let all: Vec<&Anchor> = prepared.iter().flatten().flat_map(|p| &p.anchors).collect();
    let mut resolved = once.resolve(providers, &all).into_iter();
    let warnings = run_warnings(&ledger, git, all.iter().copied()).map_err(|e| vec![e])?;

    // Step 3: the claim of each element, from the completed selectors.
    let mut built: Vec<Built<'_>> = Vec::new();
    for (i, p) in prepared.iter().enumerate() {
        let Some(p) = p else { continue };
        let rs: Vec<Resolved> = resolved.by_ref().take(p.anchors.len()).collect();
        let claim = (|| -> Result<Claim, Error> {
            let mut stored = Vec::with_capacity(rs.len());
            let mut keyed = Vec::with_capacity(rs.len());
            for (n, (a, r)) in p.anchors.iter().zip(&rs).enumerate() {
                let unit = check_new(n, a, r).map_err(|e| advise(providers, &a.loc, e))?;
                let anchor = Anchor::new(a.loc.clone(), unit.sel.clone());
                keyed.push((anchor.clone(), unit.key.as_str()));
                stored.push(anchor);
            }
            duplicate_evidence(&keyed)?;
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

    // An element may be a claim that exists: the live claim with its id, or
    // a live claim that agrees with it on the fields that need no
    // resolution. The stored anchors of each such claim are resolved in
    // one pass. What a mark on an existing claim covers and what it takes
    // over is read from the claim's own stored anchors, never from the
    // anchor the caller gave: that one may reach the same unit while the
    // stored anchor no longer does.
    let is_live = |id: &str| index.claim(id).is_some() && !index.is_retired(id);
    let mut candidates: Vec<&Claim> = Vec::new();
    for b in &built {
        let id = b.claim.id.as_str();
        let found: Vec<&Claim> = if is_live(id) {
            index.claim(id).into_iter().collect()
        } else {
            index.live().filter(|c| same_fields(c, &b.claim)).collect()
        };
        for c in found {
            if !candidates.iter().any(|x| x.id == c.id) {
                candidates.push(c);
            }
        }
    }
    let candidate_anchors: Vec<&Anchor> = candidates.iter().flat_map(|c| c.sides()).collect();
    let mut candidate_resolved = once.resolve(providers, &candidate_anchors).into_iter();
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
    let mut results: Vec<ClaimOutcome> = Vec::new();
    for b in &built {
        let id = b.claim.id.as_str();
        let existing = if is_live(id) {
            // The live claim with this id. A stored side that does not
            // resolve exactly now makes the element fail as `mark` would.
            let Some((c, rs)) = candidates.iter().find(|(c, _)| c.id.as_str() == id) else {
                errors.push(
                    Error::internal(format!("the live claim {id} was not resolved"))
                        .with_index(b.index),
                );
                continue;
            };
            let failure = c
                .sides()
                .zip(rs)
                .enumerate()
                .find_map(|(n, (a, r))| unresolved_error(n, a, r, false));
            if let Some(e) = failure {
                let e = Error::new(
                    e.code,
                    format!(
                        "this is the claim {id}, and its stored anchor does not resolve: {}",
                        e.message
                    ),
                );
                let e = match evaluate(c, index.marks_of(id), false, rs, now) {
                    Ok(eval) => match next_command(c, &eval) {
                        Some(next) => e.with_remediation(next),
                        None => e,
                    },
                    Err(_) => e,
                };
                errors.push(e.with_index(b.index));
                continue;
            }
            Some(
                &candidates[candidates
                    .iter()
                    .position(|(x, _)| x.id == c.id)
                    .unwrap_or(0)],
            )
        } else {
            candidates
                .iter()
                .filter(|(c, rs)| {
                    let keys: Vec<&str> = b
                        .resolved
                        .iter()
                        .filter_map(|r| r.unit().map(|u| u.key.as_str()))
                        .collect();
                    let asked: Vec<bool> = b
                        .input
                        .anchors
                        .iter()
                        .map(|a| a.sel.quote.is_some())
                        .collect();
                    keys.len() == b.resolved.len() && same_claim(c, rs, &b.claim, &keys, &asked)
                })
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
                // What the mark takes over for a claim that was in the
                // store before this run.
                let before = match index.claim(target.id.as_str()) {
                    None => None,
                    Some(_) => {
                        let marks = index.marks_of(target.id.as_str());
                        match evaluate(target, marks, false, &target_resolved, now) {
                            Ok(eval) => Some(accepted(target, &eval)),
                            Err(e) => {
                                errors.push(e.with_index(b.index));
                                continue;
                            }
                        }
                    }
                };
                results.push((target.clone(), !exists, mark.id, target_resolved, before));
            }
        }
    }
    if !errors.is_empty() {
        errors.sort_by_key(|e| e.index);
        return Err(finish(errors, indexed));
    }

    let mut claims = Vec::with_capacity(results.len());
    for (claim, created, mark, rs, before) in &results {
        let view = view_after(claim, &index, &batch, rs, ViewOptions::default(), now)
            .map_err(|e| vec![e])?;
        claims.push(ClaimResult {
            id: claim.id.clone(),
            created: *created,
            mark: mark.clone(),
            accepted: before.clone(),
            claim: view,
        });
    }
    store.append(&batch).map_err(|e| vec![e])?;
    Ok(ClaimOutput {
        claims,
        next_note: NEXT_NOTE.to_owned(),
        warnings,
    })
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
    let elements: Vec<Result<MarkInput, Error>> = inputs.iter().cloned().map(Ok).collect();
    mark_elements(store, providers, git, &elements, indexed, now)
}

/// `hibi mark --stdin`: [`mark`] for the elements that
/// [`read_mark_stdin`](super::read_mark_stdin) gave, reporting every
/// failing element as [`claim_batch`] does.
pub fn mark_batch(
    store: &dyn Store,
    providers: &mut Providers,
    git: &GitStatus,
    elements: &[Result<MarkInput, Error>],
    now: Timestamp,
) -> Result<MarkOutput, Vec<Error>> {
    mark_elements(store, providers, git, elements, true, now)
}

fn mark_elements(
    store: &dyn Store,
    providers: &mut Providers,
    git: &GitStatus,
    inputs: &[Result<MarkInput, Error>],
    indexed: bool,
    now: Timestamp,
) -> Result<MarkOutput, Vec<Error>> {
    let ledger = store.load().map_err(|e| vec![e])?;
    let index = Index::new(&ledger);
    let mut errors: Vec<Error> = Vec::new();

    let mut targets: Vec<Option<&Claim>> = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        let input = match input {
            Ok(input) => input,
            Err(e) => {
                errors.push(e.clone().with_index(i));
                targets.push(None);
                continue;
            }
        };
        let target = resolve_claim_id(&ledger, &input.claim).and_then(|c| {
            if index.is_retired(c.id.as_str()) {
                Err(retired_error(&ledger, c, "marked"))
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
    let mut results: Vec<(&Claim, MarkId, Vec<Resolved>, Accepted)> = Vec::new();
    for (i, (input, target)) in inputs.iter().zip(&targets).enumerate() {
        let (Ok(input), Some(claim)) = (input, *target) else {
            continue;
        };
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
            // Where the table gives no command, the error keeps its own
            // remediation: the cause and what the author can do.
            let e = match next_command(claim, &eval) {
                Some(next) => e.with_remediation(next),
                None => e,
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
                let marks = index.marks_of(claim.id.as_str());
                let before = evaluate(claim, marks, false, &rs, now)
                    .map(|eval| accepted(claim, &eval))
                    .map_err(|e| vec![e])?;
                results.push((claim, m.id, rs, before));
            }
        }
    }
    if !errors.is_empty() {
        errors.sort_by_key(|e| e.index);
        return Err(finish(errors, indexed));
    }

    let options = ViewOptions::default();
    let mut marks = Vec::with_capacity(results.len());
    for (claim, mark, rs, before) in &results {
        let view = view_after(claim, &index, &batch, rs, options, now).map_err(|e| vec![e])?;
        marks.push(MarkResult {
            claim: claim.id.clone(),
            mark: mark.clone(),
            accepted: before.clone(),
            claim_view: view,
        });
    }
    store.append(&batch).map_err(|e| vec![e])?;
    Ok(MarkOutput {
        marks,
        next_note: NEXT_NOTE.to_owned(),
        warnings,
    })
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
        return Err(retired_error(&ledger, old, "re-anchored"));
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
    // `numbers` holds, for each side, the number a message names it by: a
    // kept or replaced side keeps its number in the old claim's list, which
    // is the number `--evidence` takes; an added one has its number in the
    // new list.
    let mut numbers: Vec<usize> = vec![0];
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
        numbers.push(n);
        match input.evidence.iter().find(|(m, _)| *m == n) {
            Some((_, a)) => plan.push((a.parse()?, true)),
            None => plan.push((kept.clone(), false)),
        }
    }
    for a in &input.add_evidence {
        numbers.push(plan.len());
        plan.push((a.parse()?, true));
    }

    // What the write takes over: the old claim as it stands now, by its
    // own stored anchors. A kept side that changed is marked by this write.
    let mut once = Once::default();
    let old_anchors: Vec<&Anchor> = old.sides().collect();
    let old_resolved = once.resolve(providers, &old_anchors);
    let before = accepted(
        old,
        &evaluate(
            old,
            index.marks_of(old.id.as_str()),
            false,
            &old_resolved,
            now,
        )?,
    );

    // Step 3.
    // A kept side has the old claim's stored anchor: `once` gives it the
    // resolution that `before` was read from, so this write never marks a
    // kept side that it reports as not resolved.
    let anchors: Vec<&Anchor> = plan.iter().map(|(a, _)| a).collect();
    let resolved = once.resolve(providers, &anchors);
    let warnings = run_warnings(&ledger, git, anchors.iter().copied())?;
    let mut units: Vec<&Unit> = Vec::with_capacity(plan.len());
    for (((anchor, new), r), n) in plan.iter().zip(&resolved).zip(numbers.iter().copied()) {
        if *new {
            units.push(check_new(n, anchor, r).map_err(|e| advise(providers, &anchor.loc, e))?);
        } else if let Some(e) = unresolved_error(n, anchor, r, false) {
            // A kept side that no longer resolves to one unit has to be
            // replaced, with an anchor the caller chooses.
            return Err(match r.failure() {
                Some((SideState::Missing | SideState::Ambiguous, _))
                | Some((_, Reason::SelectorRefused)) => {
                    let option = if n == 0 {
                        "--subject <anchor>".to_owned()
                    } else {
                        format!("--evidence {n}=<anchor>")
                    };
                    e.with_remediation(format!("hibi reanchor {} {option}", old.id))
                }
                _ => advise(providers, &anchor.loc, e),
            });
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
    let keyed: Vec<(Anchor, &str)> = stored
        .iter()
        .cloned()
        .zip(units.iter().map(|u| u.key.as_str()))
        .collect();
    duplicate_evidence(&keyed)?;
    let subject = stored.remove(0);
    let new = Claim::new(statement, subject, stored, recheck, note)?;

    // Step 5.
    if new.id == old.id {
        return Err(Error::new(
            Code::NoChange,
            format!("re-anchoring {} would produce the same claim", old.id),
        )
        .with_remediation(format!(
            "The stored anchors are up to date. To record a new check, run hibi mark {}",
            old.id
        )));
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

    // The new claim may be a live claim under another id: the same claim
    // by the rule of `claim`, stored with a context that has since gone out
    // of date. The old claim is then merged into that one.
    let twin: Option<(&Claim, Vec<Resolved>)> = if let Some(same) = index.claim(new.id.as_str()) {
        // The live claim with the new id. It is the target only when its
        // own stored anchors resolve exactly in this run.
        let same_anchors: Vec<&Anchor> = same.sides().collect();
        let rs = once.resolve(providers, &same_anchors);
        let failure = same
            .sides()
            .zip(&rs)
            .enumerate()
            .find_map(|(n, (a, r))| unresolved_error(n, a, r, false));
        if let Some(e) = failure {
            let e = Error::new(
                e.code,
                format!(
                    "re-anchoring {} would merge it into the claim {}, whose stored anchor does not resolve: {}",
                    old.id, same.id, e.message
                ),
            );
            let eval = evaluate(same, index.marks_of(same.id.as_str()), false, &rs, now)?;
            return Err(match next_command(same, &eval) {
                Some(next) => e.with_remediation(next),
                None => e,
            });
        }
        Some((same, rs))
    } else {
        let keys: Vec<&str> = units.iter().map(|u| u.key.as_str()).collect();
        let asked: Vec<bool> = plan
            .iter()
            .map(|(a, replaced)| *replaced && a.sel.quote.is_some())
            .collect();
        let others: Vec<&Claim> = index
            .live()
            .filter(|c| c.id != old.id && same_fields(c, &new))
            .collect();
        let other_anchors: Vec<&Anchor> = others.iter().flat_map(|c| c.sides()).collect();
        let mut other_resolved = once.resolve(providers, &other_anchors).into_iter();
        others
            .into_iter()
            .map(|c| {
                let rs: Vec<Resolved> = other_resolved.by_ref().take(c.side_count()).collect();
                (c, rs)
            })
            .filter(|(c, rs)| same_claim(c, rs, &new, &keys, &asked))
            .min_by(|(x, _), (y, _)| x.id.cmp(&y.id))
    };

    // Step 6.
    let mut batch = Batch::default();
    let merged = twin.is_some();
    let (target, target_resolved): (Claim, Vec<Resolved>) = match twin {
        Some((c, rs)) => (c.clone(), rs),
        None => (new, resolved.iter().map(as_stored).collect()),
    };
    if !merged {
        batch.claims.push(target.clone());
    }
    // The mark covers the target's sides as the target's own anchors
    // resolve in this run.
    let target_anchors: Vec<Anchor> = target.sides().cloned().collect();
    let target_units: Vec<&Unit> = target_resolved.iter().filter_map(Resolved::unit).collect();
    if target_units.len() != target_anchors.len() {
        return Err(Error::internal(format!(
            "claim {} lost a resolved side while it was being marked",
            target.id
        )));
    }
    let mark = Mark::new(
        target.id.clone(),
        now,
        by.clone(),
        mark_sides(&target_anchors, &target_units),
        None,
    )?;
    let retirement = Retirement::new(old.id.clone(), Some(target.id.clone()), now, by, None)?;
    batch.marks.push(mark.clone());
    batch.retirements.push(retirement.clone());

    let view = view_after(
        &target,
        &index,
        &batch,
        &target_resolved,
        ViewOptions::default(),
        now,
    )?;
    let new = target;
    store.append(&batch)?;
    Ok(ReanchorOutput {
        old: old.id.clone(),
        new: new.id,
        mark: mark.id,
        retirement: retirement.id,
        merged,
        accepted: before,
        claim: view,
        next_note: NEXT_NOTE.to_owned(),
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
            if index.is_retired(c.id.as_str()) {
                return Err(Error::invalid_argument(format!(
                    "--replaced-by names {}, which is retired; a replacement must be a live claim",
                    c.id
                ))
                .with_remediation(format!("hibi list {}", c.id)));
            }
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

/// Parse one anchor argument: compact syntax (section 5.5), or, when the
/// argument starts with `{`, an anchor as a JSON object
/// `{"loc": ..., "sel": {...}}`. The JSON form is how a path that contains
/// `#` or `::`, or a selector for an external scheme, is given on the
/// command line. Errors are `invalid_anchor`.
pub fn compact_anchor(arg: &str) -> Result<AnchorInput, Error> {
    if arg.trim_start().starts_with('{') {
        let anchor: Anchor = serde_json::from_str(arg)
            .map_err(|e| Error::invalid_anchor(format!("the JSON anchor does not parse: {e}")))?;
        anchor.sel.validate()?;
        return Ok(AnchorInput::Anchor(anchor));
    }
    parse_compact(arg).map(|_| AnchorInput::Compact(arg.to_owned()))
}
