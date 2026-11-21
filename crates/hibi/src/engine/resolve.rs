//! Side resolution: the provider boundary of spec sections 2.5 and 9.2.
//!
//! Anchors are grouped by scheme and each scheme's provider receives one
//! `resolve` call. Identical anchors are sent once. An anchor whose selector
//! has a key the provider does not allow is never sent (R62).

use std::collections::{BTreeMap, HashMap};

use crate::model::{Anchor, Locator, SelectorKey};
use crate::provider::{
    NoProvider, Provider, ProviderError, ProviderErrorCode, ProviderInfo, Providers, Unit,
    check_units,
};

use super::types::{Reason, SideState};

/// At most this many candidates are shown for a side, in an error message
/// and in a claim view. The count of all of them is always given.
pub const MAX_CANDIDATES: usize = 20;

/// The units an anchor could mean, when it does not resolve to exactly one.
/// A candidate is never used as the side's unit: the caller chooses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidates {
    /// Every unit the provider returned.
    pub units: Vec<Unit>,
    /// For the first [`MAX_CANDIDATES`] units: an anchor string that selects
    /// that unit and no other unit of the resource. It is `<loc>#<symbol>`
    /// when the unit's symbol is unique in the whole resource, and the
    /// unit's completed anchor as JSON otherwise.
    pub anchors: Vec<String>,
}

impl Candidates {
    /// The shown candidates with their anchor strings.
    pub fn shown(&self) -> impl Iterator<Item = (&Unit, &str)> {
        self.units
            .iter()
            .zip(self.anchors.iter().map(String::as_str))
    }

    /// One line per shown candidate: kind, label, line and the anchor that
    /// selects it; then how many more there are.
    pub fn describe(&self) -> String {
        let mut lines: Vec<String> = self
            .shown()
            .map(|(u, anchor)| {
                let label = u.label.as_deref().unwrap_or(&u.key);
                let line = u
                    .range
                    .map_or_else(String::new, |r| format!(", line {}", r.line));
                format!(
                    "{} {:?}{line}: {anchor}",
                    u.kind,
                    super::types::excerpt(label, 80)
                )
            })
            .collect();
        let more = self.units.len() - self.anchors.len();
        if more > 0 {
            lines.push(format!("and {more} more"));
        }
        lines.join("; ")
    }
}

/// The current resolution of one anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Exactly one unit, found exactly, validated by section 2.5.
    One(Box<Unit>),
    /// Several units: the selector is ambiguous.
    Several(Candidates),
    /// The provider found nothing exactly and returned what it found by
    /// similarity (units with `approx: true`). Similarity proposes and
    /// never resolves: the side is `missing` with reason `not_exact`, and
    /// the units are candidates for the caller to check.
    NotExact(Candidates),
    /// The resource exists and the selector found nothing.
    NotFound,
    /// The selector has a key the provider does not allow. Nothing was
    /// sent. For a new anchor this is `unsupported_selector`; for a stored
    /// one the side is `unavailable` with `provider_error`.
    KeyNotAllowed { key: SelectorKey },
    /// Any other row of the first table of section 9.2.
    Failed {
        state: SideState,
        reason: Reason,
        message: String,
    },
}

impl Resolved {
    /// The candidates of an anchor that is ambiguous or was found only by
    /// similarity.
    pub fn candidates(&self) -> Option<&Candidates> {
        match self {
            Resolved::Several(c) | Resolved::NotExact(c) => Some(c),
            _ => None,
        }
    }

    /// The unit, when the anchor resolved to exactly one.
    pub fn unit(&self) -> Option<&Unit> {
        match self {
            Resolved::One(u) => Some(u),
            _ => None,
        }
    }

    /// The side state and reason of section 9.2 for an anchor that did not
    /// resolve to exactly one unit; `None` for one unit.
    pub fn failure(&self) -> Option<(SideState, Reason)> {
        match self {
            Resolved::One(_) => None,
            Resolved::Several(_) => Some((SideState::Ambiguous, Reason::SeveralUnits)),
            Resolved::NotExact(_) => Some((SideState::Missing, Reason::NotExact)),
            Resolved::NotFound => Some((SideState::Missing, Reason::SelectorNotFound)),
            Resolved::KeyNotAllowed { .. } => Some((SideState::Unavailable, Reason::ProviderError)),
            Resolved::Failed { state, reason, .. } => Some((*state, *reason)),
        }
    }

    /// A sentence about the failure, for error messages.
    pub fn failure_message(&self) -> Option<String> {
        match self {
            Resolved::One(_) => None,
            Resolved::Several(c) => Some(format!(
                "the anchor matches {} units; use one of these anchors: {}",
                c.units.len(),
                c.describe()
            )),
            Resolved::NotExact(c) => Some(format!(
                "the anchor matches no unit exactly; {} found by similarity, each a guess to check before use: {}",
                if c.units.len() == 1 {
                    "1 unit was".to_owned()
                } else {
                    format!("{} units were", c.units.len())
                },
                c.describe()
            )),
            Resolved::NotFound => Some("the selector matches no unit".to_owned()),
            Resolved::KeyNotAllowed { key } => Some(format!(
                "the provider does not allow the selector key {key}"
            )),
            Resolved::Failed { message, .. } => Some(message.clone()),
        }
    }

    fn failed(state: SideState, reason: Reason, message: impl Into<String>) -> Resolved {
        Resolved::Failed {
            state,
            reason,
            message: message.into(),
        }
    }
}

/// Key, kind, and label or line of a unit, for messages.
pub(crate) fn describe_unit(u: &Unit) -> String {
    let mut s = format!("{} ({})", u.key, u.kind);
    if let Some(r) = u.range {
        s.push_str(&format!(" at line {}", r.line));
    }
    if let Some(l) = &u.label {
        s.push_str(&format!(": {l}"));
    }
    s
}

/// The side state and reason of a provider error code (sections 9.2 and
/// 13.4). `unsupported_protocol` follows section 13.4: the provider is
/// unusable.
pub fn side_of_error(code: ProviderErrorCode) -> (SideState, Reason) {
    use ProviderErrorCode as C;
    match code {
        C::Failed | C::UnsupportedProtocol => (SideState::Unavailable, Reason::ProviderFailed),
        C::NotFound => (SideState::Missing, Reason::ResourceNotFound),
        C::NotText => (SideState::Unavailable, Reason::NotText),
        C::TooLarge => (SideState::Unavailable, Reason::TooLarge),
        C::RepoNotFound => (SideState::Unavailable, Reason::RepoNotFound),
        C::RepoNotAllowed => (SideState::Unavailable, Reason::RepoNotAllowed),
        // The resource exists but cannot be read now.
        C::Unavailable => (SideState::Unavailable, Reason::ResourceUnavailable),
        C::InvalidRequest
        | C::UnknownMethod
        | C::InvalidLocator
        | C::InvalidSelector
        | C::UnknownUnit
        | C::Internal => (SideState::Unavailable, Reason::ProviderError),
    }
}

/// The reason for a locator no provider can serve (section 9.2).
pub fn reason_of_no_provider(np: NoProvider) -> Reason {
    match np {
        NoProvider::SchemeNotDeclared => Reason::SchemeNotDeclared,
        NoProvider::ProviderNotRegistered => Reason::ProviderNotRegistered,
    }
}

pub(crate) fn failed_from_error(e: &ProviderError) -> Resolved {
    let (state, reason) = side_of_error(e.code);
    // For a resource that cannot be read now the message is the provider's
    // own sentence, which the side's remediation quotes.
    let message = if reason == Reason::ResourceUnavailable {
        e.message.clone()
    } else {
        e.to_string()
    };
    Resolved::failed(state, reason, message)
}

/// The anchor strings of the first [`MAX_CANDIDATES`] of `units`, all of
/// resource `loc`. A symbol is used alone only when the provider's unit
/// list of the resource holds it once; `listing` caches that list per
/// resource for one resolve pass.
fn candidates_of(
    provider: &mut dyn Provider,
    info: &ProviderInfo,
    listing: &mut HashMap<Locator, Option<Vec<Unit>>>,
    loc: &Locator,
    units: Vec<Unit>,
) -> Candidates {
    let all = listing.entry(loc.clone()).or_insert_with(|| {
        if info.capabilities.units {
            provider.units(loc, None).ok()
        } else {
            None
        }
    });
    let compact = loc.is_file_or_repo() && !loc.as_str().contains('#') && {
        // After the scheme, `::` would end the locator in compact syntax.
        let path = loc.as_str().split_once(':').map_or("", |(_, rest)| rest);
        !path.replacen("//", "", 1).contains("::")
    };
    let anchors = units
        .iter()
        .take(MAX_CANDIDATES)
        .map(|u| {
            let unique = u.sel.symbol.as_deref().filter(|symbol| {
                compact
                    && !symbol.contains('#')
                    && !symbol.contains("::")
                    && all.as_ref().is_some_and(|all| {
                        all.iter()
                            .filter(|x| x.sel.symbol.as_deref() == Some(symbol))
                            .count()
                            == 1
                    })
            });
            match unique {
                Some(symbol) => format!("{loc}#{symbol}"),
                None => serde_json::to_string(&Anchor::new(loc.clone(), u.sel.clone()))
                    .unwrap_or_default(),
            }
        })
        .collect();
    Candidates { units, anchors }
}

/// `info()` of a provider, cloned; any failure is `provider_failed`.
pub(crate) fn info_of(provider: &mut dyn Provider) -> Result<ProviderInfo, String> {
    provider.info().cloned().map_err(|e| e.to_string())
}

/// Validate the units of one answer by section 2.5: every `sel` is a valid
/// selector with only allowed keys, and keys are unique. (`fp` is valid by
/// construction of [`crate::model::Fingerprint`].)
pub(crate) fn validate_units(info: &ProviderInfo, units: &[Unit]) -> Result<(), String> {
    for u in units {
        u.sel
            .validate()
            .map_err(|e| format!("unit {:?}: {}", u.key, e.message))?;
    }
    check_units(info, units)
}

/// Resolve anchors with one provider. One result per anchor, in order.
pub fn resolve_with(provider: &mut dyn Provider, anchors: &[&Anchor]) -> Vec<Resolved> {
    let info = match info_of(provider) {
        Ok(info) => info,
        Err(message) => {
            return anchors
                .iter()
                .map(|_| Resolved::failed(SideState::Unavailable, Reason::ProviderFailed, &message))
                .collect();
        }
    };
    // Position in `unique` of each anchor that is sent.
    let mut unique: Vec<Anchor> = Vec::new();
    let mut seen: HashMap<&Anchor, usize> = HashMap::new();
    let mut slots: Vec<Result<usize, SelectorKey>> = Vec::with_capacity(anchors.len());
    for a in anchors {
        if let Some(key) = a.sel.disallowed_key(&info.selectors) {
            slots.push(Err(key));
            continue;
        }
        let at = *seen.entry(a).or_insert_with(|| {
            unique.push((*a).clone());
            unique.len() - 1
        });
        slots.push(Ok(at));
    }
    let answers: Vec<Resolved> = if unique.is_empty() {
        Vec::new()
    } else {
        let raw = provider.resolve(&unique);
        if raw.len() != unique.len() {
            let message = format!(
                "the provider answered {} of {} resolve requests",
                raw.len(),
                unique.len()
            );
            unique
                .iter()
                .map(|_| Resolved::failed(SideState::Unavailable, Reason::ProviderError, &message))
                .collect()
        } else {
            let mut listing: HashMap<Locator, Option<Vec<Unit>>> = HashMap::new();
            let mut out = Vec::with_capacity(raw.len());
            for (r, anchor) in raw.into_iter().zip(&unique) {
                out.push(match r {
                    Err(e) => failed_from_error(&e),
                    Ok(units) => match validate_units(&info, &units) {
                        Err(m) => {
                            Resolved::failed(SideState::Unavailable, Reason::ProviderError, m)
                        }
                        Ok(()) => {
                            let mut units = units;
                            // A unit found by similarity is a guess: with
                            // one in the answer nothing resolves.
                            let guessed = units.iter().any(|u| u.approx);
                            match (units.len(), guessed) {
                                (0, _) => Resolved::NotFound,
                                (1, false) => Resolved::One(Box::new(units.remove(0))),
                                (_, false) => Resolved::Several(candidates_of(
                                    provider,
                                    &info,
                                    &mut listing,
                                    &anchor.loc,
                                    units,
                                )),
                                (_, true) => Resolved::NotExact(candidates_of(
                                    provider,
                                    &info,
                                    &mut listing,
                                    &anchor.loc,
                                    units,
                                )),
                            }
                        }
                    },
                });
            }
            out
        }
    };
    slots
        .into_iter()
        .map(|s| match s {
            Ok(at) => answers[at].clone(),
            Err(key) => Resolved::KeyNotAllowed { key },
        })
        .collect()
}

/// Resolve anchors through the registry: one `resolve` call per scheme,
/// schemes in ascending order. One result per anchor, in order.
pub fn resolve_anchors(providers: &mut Providers, anchors: &[&Anchor]) -> Vec<Resolved> {
    let mut by_scheme: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, a) in anchors.iter().enumerate() {
        by_scheme.entry(a.loc.scheme()).or_default().push(i);
    }
    let mut out: Vec<Option<Resolved>> = vec![None; anchors.len()];
    for (scheme, indexes) in by_scheme {
        let results = match providers.provider_for_scheme(scheme) {
            Err(np) => {
                let reason = reason_of_no_provider(np);
                let message = match np {
                    NoProvider::SchemeNotDeclared => {
                        format!("the scheme {scheme} is not declared in .hibi/hibi.toml")
                    }
                    NoProvider::ProviderNotRegistered => {
                        format!("the user config has no provider command for scheme {scheme}")
                    }
                };
                indexes
                    .iter()
                    .map(|_| Resolved::failed(SideState::Unavailable, reason, &message))
                    .collect()
            }
            Ok(provider) => {
                let subset: Vec<&Anchor> = indexes.iter().map(|&i| anchors[i]).collect();
                resolve_with(provider, &subset)
            }
        };
        for (i, r) in indexes.into_iter().zip(results) {
            out[i] = Some(r);
        }
    }
    out.into_iter()
        .map(|r| r.expect("every anchor belongs to one scheme group"))
        .collect()
}
