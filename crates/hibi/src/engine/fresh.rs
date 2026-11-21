//! Freshness (spec section 9): side states, claim states, the reference
//! mark, and the `next` suggestion of section 15.2.
//!
//! A state is a function of the claim, its marks, whether it is retired,
//! the current resolution of each side, and `now`. Fingerprints are compared
//! for equality only; no similarity score is consulted (R16).

use std::collections::{HashMap, HashSet};

use crate::duration;
use crate::error::Error;
use crate::fingerprint::{Comparison, compare};
use crate::model::{Claim, Mark, Timestamp};
use crate::store::Ledger;

use super::resolve::Resolved;
use super::types::{
    Accepted, AcceptedSide, Candidate, ClaimState, ClaimView, DETAIL_CHARS, Reason, Role, Side,
    SideState, TEXT_EXCERPT_CHARS, UnitRef, excerpt, side_remediation,
};

/// A ledger with its marks grouped by claim and its retired claims as a
/// set.
pub struct Index<'a> {
    pub ledger: &'a Ledger,
    marks: HashMap<&'a str, Vec<&'a Mark>>,
    retired: HashSet<&'a str>,
}

impl<'a> Index<'a> {
    pub fn new(ledger: &'a Ledger) -> Self {
        let mut marks: HashMap<&str, Vec<&Mark>> = HashMap::new();
        for m in &ledger.marks {
            marks.entry(m.claim.as_str()).or_default().push(m);
        }
        let retired = ledger
            .retirements
            .iter()
            .map(|r| r.retires.as_str())
            .collect();
        Index {
            ledger,
            marks,
            retired,
        }
    }

    /// The marks of a claim, in id order.
    pub fn marks_of(&self, claim: &str) -> &[&'a Mark] {
        self.marks.get(claim).map_or(&[], Vec::as_slice)
    }

    pub fn is_retired(&self, claim: &str) -> bool {
        self.retired.contains(claim)
    }

    pub fn claim(&self, id: &str) -> Option<&'a Claim> {
        self.ledger.claim(id)
    }

    /// The live claims, in id order.
    pub fn live(&self) -> impl Iterator<Item = &'a Claim> + '_ {
        self.ledger
            .claims
            .iter()
            .filter(|c| !self.is_retired(c.id.as_str()))
    }
}

/// The freshness of one claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation<'m> {
    pub state: ClaimState,
    /// The reference mark (section 9.3).
    pub reference: Option<&'m Mark>,
    /// `add(reference.at, recheck)` when the claim is `fresh` or `expired`
    /// and `recheck` is set.
    pub expires_at: Option<Timestamp>,
    /// State and reason per side; empty for a retired claim.
    pub sides: Vec<(SideState, Option<Reason>)>,
}

/// True when every side resolves to one unit whose fingerprint equals the
/// mark's fingerprint for that side.
fn mark_matches(mark: &Mark, resolved: &[Resolved]) -> bool {
    mark.sides.len() == resolved.len()
        && mark
            .sides
            .iter()
            .zip(resolved)
            .all(|(s, r)| r.unit().is_some_and(|u| u.fp == s.fp))
}

/// The mark with the latest `at`; ties go to the greater id.
fn latest<'m>(marks: impl Iterator<Item = &'m Mark>) -> Option<&'m Mark> {
    marks.max_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)))
}

/// Evaluate a claim by sections 9.2 and 9.3.
///
/// `marks` are the claim's marks; a mark that does not fit the claim (wrong
/// claim id, side count or locators) is ignored. `resolved` holds one entry
/// per side and is not read for a retired claim.
pub fn evaluate<'m>(
    claim: &Claim,
    marks: &[&'m Mark],
    retired: bool,
    resolved: &[Resolved],
    now: Timestamp,
) -> Result<Evaluation<'m>, Error> {
    let marks: Vec<&'m Mark> = marks
        .iter()
        .copied()
        .filter(|m| m.check_against(claim).is_ok())
        .collect();
    if retired {
        return Ok(Evaluation {
            state: ClaimState::Retired,
            reference: latest(marks.iter().copied()),
            expires_at: None,
            sides: Vec::new(),
        });
    }
    if resolved.len() != claim.side_count() {
        return Err(Error::internal(format!(
            "claim {} has {} sides and {} resolutions",
            claim.id,
            claim.side_count(),
            resolved.len()
        )));
    }
    let matching: Vec<&'m Mark> = marks
        .iter()
        .copied()
        .filter(|m| mark_matches(m, resolved))
        .collect();
    let reference = latest(matching.iter().copied()).or_else(|| latest(marks.iter().copied()));
    let recheck = claim
        .recheck
        .as_deref()
        .map(duration::parse)
        .transpose()
        .map_err(|e| Error::internal(format!("claim {}: {}", claim.id, e.message)))?;

    let state = if matching.is_empty() {
        ClaimState::Stale
    } else {
        match &recheck {
            None => ClaimState::Fresh,
            // An expiry past the last representable instant is never
            // reached: `add` fails for it, and the mark keeps the claim
            // fresh. `expiresAt` is then `null`.
            Some(d) => {
                let live = matching.iter().any(|m| {
                    duration::add(m.at.datetime(), d).map_or(true, |end| now.datetime() < end)
                });
                if live {
                    ClaimState::Fresh
                } else {
                    ClaimState::Expired
                }
            }
        }
    };
    let expires_at = match (&recheck, reference, state) {
        (Some(d), Some(m), ClaimState::Fresh | ClaimState::Expired) => {
            duration::add(m.at.datetime(), d)
                .ok()
                .and_then(|end| Timestamp::from_datetime(end).ok())
        }
        _ => None,
    };
    let sides = resolved
        .iter()
        .enumerate()
        .map(|(i, r)| match r.failure() {
            Some((state, reason)) => (state, Some(reason)),
            None => {
                let unit = r.unit().expect("no failure means one unit");
                match reference {
                    None => (SideState::Changed, Some(Reason::NoMark)),
                    Some(m) => match compare(&m.sides[i].fp, &unit.fp) {
                        Comparison::Same => (SideState::Same, None),
                        Comparison::Changed => {
                            (SideState::Changed, Some(Reason::FingerprintDiffers))
                        }
                        Comparison::ProfileMismatch => {
                            (SideState::Unavailable, Some(Reason::ProfileMismatch))
                        }
                    },
                }
            }
        })
        .collect();
    Ok(Evaluation {
        state,
        reference,
        expires_at,
        sides,
    })
}

/// The record of what a write accepted for `claim`, from its evaluation
/// before the write.
pub fn accepted(claim: &Claim, eval: &Evaluation<'_>) -> Accepted {
    Accepted {
        was: eval.state,
        sides: claim
            .sides()
            .zip(&eval.sides)
            .enumerate()
            .filter(|(_, (_, (state, _)))| *state != SideState::Same)
            .map(|(index, (anchor, (state, reason)))| AcceptedSide {
                index,
                role: Role::of(index),
                loc: anchor.loc.clone(),
                state: *state,
                reason: *reason,
            })
            .collect(),
    }
}

/// Which side texts a [`ClaimView`] carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewOptions {
    /// `--detail`: every resolved side carries its full unit text.
    pub detail: bool,
}

/// The `next` suggestion for a claim: the first row of the table in
/// section 15.2 that applies. It is a command for the caller to complete
/// and run after a check, never one that contains a candidate: where a
/// side has to be anchored again, the anchor is written as `<anchor>`.
pub fn next_command(claim: &Claim, eval: &Evaluation<'_>) -> Option<String> {
    let id = &claim.id;
    if matches!(eval.state, ClaimState::Retired | ClaimState::Fresh) {
        return None;
    }
    let mut reanchor = String::new();
    for (i, (state, _)) in eval.sides.iter().enumerate() {
        if matches!(state, SideState::Missing | SideState::Ambiguous) {
            if i == 0 {
                reanchor.push_str(" --subject <anchor>");
            } else {
                reanchor.push_str(&format!(" --evidence {i}=<anchor>"));
            }
        }
    }
    if !reanchor.is_empty() {
        return Some(format!("hibi reanchor {id}{reanchor}"));
    }
    let unavailable = |reasons: &[Reason]| {
        eval.sides.iter().any(|(state, reason)| {
            *state == SideState::Unavailable && reason.is_some_and(|r| reasons.contains(&r))
        })
    };
    if unavailable(&[
        Reason::ProviderNotRegistered,
        Reason::ProviderFailed,
        Reason::RepoNotAllowed,
    ]) {
        return Some("hibi providers".to_owned());
    }
    // No command fits these: the side's `remediation` says what to do.
    if unavailable(&[
        Reason::SchemeNotDeclared,
        Reason::RepoNotFound,
        Reason::NotText,
        Reason::TooLarge,
        Reason::ProviderError,
        Reason::ResourceUnavailable,
    ]) {
        return None;
    }
    Some(format!("hibi mark {id}"))
}

/// Build the ClaimView of section 15.2.
pub fn claim_view(
    claim: &Claim,
    eval: &Evaluation<'_>,
    resolved: &[Resolved],
    options: ViewOptions,
) -> ClaimView {
    let sides = eval
        .sides
        .iter()
        .enumerate()
        .zip(claim.sides().zip(resolved))
        .map(|((index, (state, reason)), (anchor, r))| {
            let unit = r.unit();
            let text = unit
                .and_then(|u| u.text.as_deref())
                .filter(|_| options.detail)
                .map(str::to_owned);
            let marked_fp = eval.reference.map(|m| m.sides[index].fp.clone());
            let said = match r {
                Resolved::Failed { message, .. } if *state == SideState::Unavailable => {
                    Some(message.as_str())
                }
                _ => None,
            };
            let candidates = r.candidates().map(|c| {
                c.shown()
                    .map(|(u, select)| Candidate {
                        key: u.key.clone(),
                        kind: u.kind.clone(),
                        label: u.label.clone(),
                        line: u.range.map(|r| r.line),
                        end_line: u.range.map(|r| r.end_line),
                        text: u.text.as_deref().map(|t| excerpt(t, TEXT_EXCERPT_CHARS)),
                        anchor: select.to_owned(),
                        same_fingerprint: marked_fp.as_ref() == Some(&u.fp),
                    })
                    .collect::<Vec<_>>()
            });
            Side {
                index,
                role: Role::of(index),
                anchor: anchor.clone(),
                state: *state,
                reason: *reason,
                unit: unit.map(UnitRef::from),
                fp: unit.map(|u| u.fp.clone()),
                marked_fp,
                remediation: match (state, reason) {
                    (SideState::Unavailable, Some(reason)) => side_remediation(*reason, said),
                    _ => None,
                },
                detail: said.map(|m| excerpt(m, DETAIL_CHARS)),
                candidates_total: r.candidates().map(|c| c.units.len()),
                candidates,
                text,
            }
        })
        .collect();
    ClaimView {
        id: claim.id.clone(),
        statement: claim.statement.clone(),
        recheck: claim.recheck.clone(),
        note: claim.note.clone(),
        state: eval.state,
        mark: eval.reference.map(|m| m.id.clone()),
        marked_at: eval.reference.map(|m| m.at),
        expires_at: eval.expires_at,
        sides,
        next: next_command(claim, eval),
    }
}
