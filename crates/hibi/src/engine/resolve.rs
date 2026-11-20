//! Side resolution: the provider boundary of spec sections 2.5 and 9.2.
//!
//! Anchors are grouped by scheme and each scheme's provider receives one
//! `resolve` call. Identical anchors are sent once. An anchor whose selector
//! has a key the provider does not allow is never sent (R62).

use std::collections::{BTreeMap, HashMap};

use crate::model::{Anchor, SelectorKey};
use crate::provider::{
    NoProvider, Provider, ProviderError, ProviderErrorCode, ProviderInfo, Providers, Unit,
    check_units,
};

use super::types::{Reason, SideState};

/// The current resolution of one anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Exactly one unit, validated by section 2.5.
    One(Box<Unit>),
    /// Several units: the selector is ambiguous.
    Several(Vec<Unit>),
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
            Resolved::NotFound => Some((SideState::Missing, Reason::SelectorNotFound)),
            Resolved::KeyNotAllowed { .. } => Some((SideState::Unavailable, Reason::ProviderError)),
            Resolved::Failed { state, reason, .. } => Some((*state, *reason)),
        }
    }

    /// A sentence about the failure, for error messages.
    pub fn failure_message(&self) -> Option<String> {
        match self {
            Resolved::One(_) => None,
            Resolved::Several(units) => Some(format!(
                "the selector matches {} units: {}",
                units.len(),
                units
                    .iter()
                    .map(describe_unit)
                    .collect::<Vec<_>>()
                    .join("; ")
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
        C::InvalidRequest
        | C::UnknownMethod
        | C::InvalidLocator
        | C::InvalidSelector
        | C::UnknownUnit
        | C::Unavailable
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
    Resolved::failed(state, reason, e.to_string())
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
            raw.into_iter()
                .map(|r| match r {
                    Err(e) => failed_from_error(&e),
                    Ok(units) => match validate_units(&info, &units) {
                        Err(m) => {
                            Resolved::failed(SideState::Unavailable, Reason::ProviderError, m)
                        }
                        Ok(()) => {
                            let mut units = units;
                            match units.len() {
                                0 => Resolved::NotFound,
                                1 => Resolved::One(Box::new(units.remove(0))),
                                _ => Resolved::Several(units),
                            }
                        }
                    },
                })
                .collect()
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
