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
    /// The resource the units are in.
    pub loc: Locator,
    /// The first [`MAX_CANDIDATES`] units the provider returned. The rest
    /// are not kept: an anchor that matches every one of many identical
    /// units would otherwise hold all of them once per claim.
    pub units: Vec<Unit>,
    /// How many units the provider returned.
    pub total: usize,
    /// For the first [`MAX_CANDIDATES`] units: the anchor the provider
    /// printed for the unit (its `sel`, section 6.7), as text to give back.
    /// It is `<loc>#<symbol>` when the selector is a symbol alone, and the
    /// anchor as JSON otherwise. Units that are identical where they stand
    /// carry the same anchor: it selects all of them, so it is not shown as
    /// a way to pick one ([`Candidates::selects_alone`]).
    pub anchors: Vec<String>,
    /// How many of all the units the provider returned have each
    /// fingerprint. It makes the count of a merged list exact when only
    /// the first units are kept.
    pub by_fp: HashMap<crate::model::Fingerprint, usize>,
    /// The declaration that holds every shown unit that no anchor selects
    /// alone, when there is one: its key and the anchor that selects it.
    /// It is read from the unit list of the resource
    /// ([`Candidates::find_enclosing`]) and never from the text of a
    /// symbol: `Id.fmt` is a method of an `impl` block, and `#Id` is the
    /// struct, which holds no method.
    pub enclosing: Option<(String, String)>,
}

impl Candidates {
    /// The one list a side shows (section 15.2): the units of `first`,
    /// which have the marked content, then those of `second`, which the
    /// provider returned; a unit once, by its key; at most
    /// [`MAX_CANDIDATES`]. `total` counts the units of both once each, as
    /// `None` when both are `None`.
    pub fn merged(first: Option<&Candidates>, second: Option<&Candidates>) -> Option<Candidates> {
        let (first, second) = match (first, second) {
            (None, None) => return None,
            (Some(c), None) | (None, Some(c)) => return Some(c.clone()),
            (Some(f), Some(s)) => (f, s),
        };
        let mut units: Vec<Unit> = Vec::new();
        let mut anchors: Vec<String> = Vec::new();
        for (u, anchor) in first.shown().chain(second.shown()) {
            if units.len() < MAX_CANDIDATES && !units.iter().any(|x| x.key == u.key) {
                units.push(u.clone());
                anchors.push(anchor.to_owned());
            }
        }
        // Every unit of `first` has the marked fingerprint, and both lists
        // hold units of one resource: the units in both are the units of
        // `second` with that fingerprint.
        let twice = first
            .units
            .first()
            .and_then(|u| second.by_fp.get(&u.fp))
            .copied()
            .unwrap_or(0)
            .min(first.total);
        let mut by_fp = second.by_fp.clone();
        if let Some(u) = first.units.first() {
            by_fp.insert(u.fp.clone(), first.total.max(twice));
        }
        Some(Candidates {
            loc: first.loc.clone(),
            units,
            anchors,
            total: first.total + second.total - twice,
            by_fp,
            enclosing: first.enclosing.clone().or_else(|| second.enclosing.clone()),
        })
    }

    /// The shown candidates with their anchor strings.
    pub fn shown(&self) -> impl Iterator<Item = (&Unit, &str)> {
        self.units
            .iter()
            .zip(self.anchors.iter().map(String::as_str))
    }

    /// The shown candidates, each with its anchor when that anchor selects
    /// it alone, and `None` when it does not: the provider says so
    /// ([`selects_its_unit`]), or another shown candidate carries the same
    /// anchor. Such a unit cannot be told apart by any anchor of this form.
    pub fn selectable(&self) -> impl Iterator<Item = (&Unit, Option<&str>)> {
        self.shown().map(|(u, anchor)| {
            let alone =
                selects_its_unit(u) && self.anchors.iter().filter(|a| *a == anchor).count() == 1;
            (u, alone.then_some(anchor))
        })
    }

    /// The shown units that no anchor selects alone.
    fn twins(&self) -> Vec<&Unit> {
        self.selectable()
            .filter(|(_, anchor)| anchor.is_none())
            .map(|(u, _)| u)
            .collect()
    }

    /// The key of the unit that is the parent of every one of the
    /// [`twins`](Self::twins), when they have one and the same parent.
    fn twins_parent(&self) -> Option<&str> {
        let twins = self.twins();
        let parent = twins.first()?.parent.as_deref()?;
        twins
            .iter()
            .all(|u| u.parent.as_deref() == Some(parent))
            .then_some(parent)
    }

    /// Set [`enclosing`](Self::enclosing) from `all`, the unit list of the
    /// resource. A declaration is named only when it is there to anchor:
    /// the parent of every twin is one unit, that unit has a symbol and is
    /// not the unit of the whole file, and its printed selector selects it
    /// alone (two `interface Props` blocks do not).
    pub(crate) fn find_enclosing(&mut self, all: &[Unit]) {
        let Some(parent) = self.twins_parent() else {
            return;
        };
        let Some(decl) = all.iter().find(|u| u.key == parent) else {
            return;
        };
        if decl.parent.is_none()
            || decl.sel.symbol.is_none()
            || !selects_its_unit(decl)
            || all.iter().filter(|u| u.sel == decl.sel).count() != 1
        {
            return;
        }
        let anchor = printed_anchor(&self.loc, is_compact(&self.loc), decl);
        self.enclosing = Some((decl.key.clone(), anchor));
    }

    /// What to do, for a claim, about shown units that no anchor selects
    /// alone; `None` when every shown unit has an anchor. Such units cannot
    /// be anchored one by one: a longer quote reaches outside the unit or
    /// matches both again, and an exclusion in a scope plays no part in
    /// resolving a claim. What works is the unit that contains them. That
    /// is the whole file, and in code also the declaration that encloses
    /// all of them, when [`find_enclosing`](Self::find_enclosing) found
    /// one. Nothing follows a locator in the sentence, so that it can be
    /// copied as it stands.
    pub fn twins_remedy(&self) -> Option<String> {
        if self.twins().is_empty() {
            return None;
        }
        let loc = &self.loc;
        let whole = if loc.is_file_or_repo() {
            "the whole file"
        } else {
            "the whole resource"
        };
        let parent = self.twins_parent();
        let declaration = self
            .enclosing
            .as_ref()
            .filter(|(key, _)| parent == Some(key.as_str()))
            .map(|(_, anchor)| anchor);
        Some(match declaration {
            Some(anchor) => format!(
                "Units that are identical where they stand cannot be anchored one by one. Anchor the unit that contains them: the enclosing declaration {anchor} or {whole} {loc}"
            ),
            None => format!(
                "Units that are identical where they stand cannot be anchored one by one. Anchor {whole} instead: {loc}"
            ),
        })
    }

    /// True when the anchor of every shown candidate selects it alone.
    pub fn selects_alone(&self) -> bool {
        self.selectable().all(|(_, anchor)| anchor.is_some())
    }

    /// One line per shown candidate: kind, label, line and the anchor that
    /// selects it; then how many more there are.
    pub fn describe(&self) -> String {
        let mut lines: Vec<String> = self
            .selectable()
            .map(|(u, anchor)| {
                format!(
                    "{}: {}",
                    describe_unit(u),
                    anchor.unwrap_or("identical to another one, no anchor selects it alone")
                )
            })
            .collect();
        let more = self.total.saturating_sub(self.anchors.len());
        if more > 0 {
            lines.push(format!("and {more} more"));
        }
        let mut out = lines.join("; ");
        if let Some(remedy) = self.twins_remedy() {
            out.push_str(". ");
            out.push_str(&remedy);
        }
        out
    }
}

/// `label` inside double quotes, for a message a person or an agent reads.
/// The label is written as it is: only `"` is escaped (as `\"`), and a
/// control character is written as `\n`, `\r`, `\t` or `\u{..}`, so that
/// the label stays on one line and its end is unambiguous. A backslash and
/// every other character are not escaped. JSON output does not use this:
/// there a label is a JSON string.
pub fn quoted(label: &str) -> String {
    let mut out = String::with_capacity(label.len() + 2);
    out.push('"');
    for c in label.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// False when the provider printed a selector for `unit` that does not
/// select it alone: the unit then carries `refuse:
/// quote_not_distinguishable` (section 6.7), also in `units`, in candidate
/// lists and in ambiguous lists. No anchor may be built or printed from
/// the `sel` of such a unit.
pub fn selects_its_unit(unit: &Unit) -> bool {
    unit.refuse != Some(crate::provider::Refuse::QuoteNotDistinguishable)
}

/// A new anchor that the provider could only guess at.
pub const BY_SIMILARITY: &str = "the anchor matches no unit exactly";

/// A stored anchor that resolves nothing: its container or name is gone or
/// names several, its text stands somewhere the stored surroundings do not
/// fit, or only similar text was found (section 6.7). The candidates may
/// hold the stored text exactly, so this does not say that nothing matches.
pub const NOT_IDENTIFIED: &str = "the stored anchor no longer identifies one unit";

/// A stored `#name::<quote>` anchor whose name and quote disagree: the name
/// still names one declaration, and the quoted text is in other units.
pub const NAME_AND_QUOTE_APART: &str = "the name and the quoted text now point to different units: the first candidate has the name, the others hold the quoted text";

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
    /// Why a stored side is `missing` with reason `not_exact`: one
    /// sentence, used in the side's `detail`, in the text output and in
    /// error messages. `stored` is the side's stored anchor. `None` for
    /// every other resolution.
    pub fn not_exact_cause(&self, stored: &Anchor) -> Option<&'static str> {
        let Resolved::NotExact(c) = self else {
            return None;
        };
        // Section 6.7, A1 with conflicting evidence: the declaration the
        // symbol names comes first, then the units that hold the quote.
        let sel = &stored.sel;
        let apart = sel.symbol.is_some()
            && sel.quote.is_some()
            && sel.count.is_none()
            && c.units.len() >= 2
            && c.units[0].sel.symbol == sel.symbol
            && c.units[1..].iter().all(|u| u.sel.symbol != sel.symbol);
        Some(if apart {
            NAME_AND_QUOTE_APART
        } else {
            NOT_IDENTIFIED
        })
    }

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
            // The stored anchor is no longer accepted: the side needs
            // another one, as for an anchor the provider refuses.
            Resolved::KeyNotAllowed { .. } => {
                Some((SideState::Unavailable, Reason::SelectorRefused))
            }
            Resolved::Failed { state, reason, .. } => Some((*state, *reason)),
        }
    }

    /// A sentence about the failure, for error messages.
    pub fn failure_message(&self) -> Option<String> {
        self.failure_message_with(None, None)
    }

    /// [`failure_message`](Self::failure_message) for a stored side, with
    /// the units of the resource that have the marked content
    /// ([`content_candidates`]) listed first, as `status` lists them.
    ///
    /// `stored` is the side's stored anchor, or `None` for a new anchor: it
    /// chooses the sentence that says why nothing resolved.
    pub fn failure_message_with(
        &self,
        by_content: Option<&Candidates>,
        stored: Option<&Anchor>,
    ) -> Option<String> {
        let units = |n: usize| {
            if n == 1 {
                "1 candidate".to_owned()
            } else {
                format!("{n} candidates")
            }
        };
        match self {
            Resolved::One(_) => None,
            // "use one of these anchors" is said only when the list holds
            // one: units that cannot be told apart have none.
            Resolved::Several(c) => Some(format!(
                "the anchor matches {} units{}: {}",
                c.total,
                if c.selectable().any(|(_, anchor)| anchor.is_some()) {
                    "; use one of these anchors"
                } else {
                    " that cannot be told apart"
                },
                c.describe()
            )),
            Resolved::NotExact(c) => {
                let all = Candidates::merged(by_content, Some(c)).unwrap_or_else(|| c.clone());
                Some(format!(
                    "{}; {}, each a guess to check before use: {}",
                    stored
                        .and_then(|a| self.not_exact_cause(a))
                        .unwrap_or(BY_SIMILARITY),
                    units(all.total),
                    all.describe()
                ))
            }
            Resolved::NotFound => Some(match by_content {
                None => "the selector matches no unit".to_owned(),
                Some(c) => format!(
                    "the selector matches no unit; {}, each a guess to check before use: {}",
                    units(c.total),
                    c.describe()
                ),
            }),
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

/// A unit as every message and the text output name it:
/// `<kind> "<label>", lines <a>-<b>`, with `line <a>` for a unit on one
/// line and nothing after the label for a unit without a range. The label
/// (the key when the unit has none) is cut to 80 characters and written
/// by [`quoted`].
pub(crate) fn describe_unit(u: &Unit) -> String {
    let label = u.label.as_deref().unwrap_or(&u.key);
    let lines = match u.range {
        Some(r) if r.end_line > r.line => format!(", lines {}-{}", r.line, r.end_line),
        Some(r) => format!(", line {}", r.line),
        None => String::new(),
    };
    format!(
        "{} {}{lines}",
        u.kind,
        quoted(&super::types::excerpt(label, 80))
    )
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
        // The provider refuses the anchor: the anchor is wrong for this
        // resource, the provider did not fail.
        C::InvalidLocator | C::InvalidSelector => (SideState::Unavailable, Reason::SelectorRefused),
        C::InvalidRequest | C::UnknownMethod | C::UnknownUnit | C::Internal => {
            (SideState::Unavailable, Reason::ProviderError)
        }
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
    // The message is the provider's own sentence, without the error code:
    // the side's `reason` already names the kind, and `detail`, the
    // remediation and the error message quote the sentence as it is.
    let message = e.message.clone();
    Resolved::failed(state, reason, message)
}

/// True when an anchor on `loc` can be written in compact syntax. Compact
/// syntax has no escapes: a locator with a control character, a `#` or a
/// `::` in its path is given as JSON.
fn is_compact(loc: &Locator) -> bool {
    loc.is_file_or_repo()
        && !loc.as_str().contains('#')
        && !loc.as_str().chars().any(char::is_control)
        && {
            // After the scheme, `::` would end the locator in compact syntax.
            let path = loc.as_str().split_once(':').map_or("", |(_, rest)| rest);
            !path.replacen("//", "", 1).contains("::")
        }
}

/// The provider's printed anchor of `u` (section 6.7) as text to give
/// back: a symbol alone is written compactly, anything else as JSON.
fn printed_anchor(loc: &Locator, compact: bool, u: &Unit) -> String {
    let sel = &u.sel;
    let only_symbol = sel.quote.is_none()
        && sel.prefix.is_none()
        && sel.suffix.is_none()
        && sel.within.is_none()
        && sel.count.is_none()
        && sel.key.is_none();
    let symbol = u.sel.symbol.as_deref().filter(|symbol| {
        only_symbol
            && compact
            && !symbol.chars().any(char::is_control)
            && !symbol.contains('#')
            && !symbol.contains("::")
    });
    match symbol {
        Some(symbol) => format!("{loc}#{symbol}"),
        None => super::types::escape_controls(
            &serde_json::to_string(&Anchor::new(loc.clone(), u.sel.clone())).unwrap_or_default(),
        ),
    }
}

/// The candidates `units`, all of resource `loc`, with the anchor strings
/// of the first [`MAX_CANDIDATES`] of them.
fn candidates_of(loc: &Locator, units: Vec<Unit>) -> Candidates {
    let compact = is_compact(loc);
    let anchors = units
        .iter()
        .take(MAX_CANDIDATES)
        .map(|u| printed_anchor(loc, compact, u))
        .collect();
    let total = units.len();
    let mut by_fp: HashMap<crate::model::Fingerprint, usize> = HashMap::new();
    for u in &units {
        *by_fp.entry(u.fp.clone()).or_default() += 1;
    }
    let mut units = units;
    units.truncate(MAX_CANDIDATES);
    Candidates {
        loc: loc.clone(),
        units,
        anchors,
        total,
        by_fp,
        enclosing: None,
    }
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
                                (_, false) => Resolved::Several(candidates_of(&anchor.loc, units)),
                                (_, true) => Resolved::NotExact(candidates_of(&anchor.loc, units)),
                            }
                        }
                    },
                });
            }
            // Units that no anchor selects alone, with one parent: the unit
            // list of the resource says whether that parent can be
            // anchored. One `units` call per such resource.
            if info.capabilities.units {
                let mut listing: HashMap<Locator, Option<Vec<Unit>>> = HashMap::new();
                for (r, anchor) in out.iter_mut().zip(&unique) {
                    let (Resolved::Several(c) | Resolved::NotExact(c)) = r else {
                        continue;
                    };
                    if c.twins_parent().is_none() {
                        continue;
                    }
                    let all = listing.entry(anchor.loc.clone()).or_insert_with(|| {
                        provider
                            .units(&anchor.loc, None)
                            .ok()
                            .filter(|units| validate_units(&info, units).is_ok())
                    });
                    if let Some(all) = all {
                        c.find_enclosing(all);
                    }
                }
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
/// Candidates by content (section 9.2). For each `(anchor, marked)`: the
/// units of the anchor's resource whose fingerprint equals `marked`, the
/// side's fingerprint in the reference mark, with an anchor for each. Such
/// a unit has the content that was marked, so it is the first thing to
/// offer for a side whose stored anchor finds nothing exactly: a renamed
/// class or constant is found this way when the provider's own search by
/// similarity finds nothing or something else. It is still only a
/// candidate. `None` when the provider cannot list units, the listing
/// fails, or nothing matches. One `units` call per resource; the text of a
/// shown unit that came without one is fetched with `render`.
pub fn content_candidates(
    providers: &mut Providers,
    wanted: &[Option<(&Anchor, &crate::model::Fingerprint)>],
) -> Vec<Option<Candidates>> {
    let mut listing: HashMap<Locator, Option<Vec<Unit>>> = HashMap::new();
    wanted
        .iter()
        .map(|w| {
            let (anchor, marked) = (*w)?;
            let provider = providers.provider_for_scheme(anchor.loc.scheme()).ok()?;
            let info = provider.info().ok()?.clone();
            if !info.capabilities.units {
                return None;
            }
            let all = listing
                .entry(anchor.loc.clone())
                .or_insert_with(|| {
                    provider
                        .units(&anchor.loc, None)
                        .ok()
                        .filter(|units| validate_units(&info, units).is_ok())
                })
                .as_ref()?;
            let mut same: Vec<Unit> = all.iter().filter(|u| &u.fp == marked).cloned().collect();
            if same.is_empty() {
                return None;
            }
            if info.capabilities.render {
                for u in same.iter_mut().take(MAX_CANDIDATES) {
                    if u.text.is_none() {
                        u.text = provider.render(&anchor.loc, &u.key).ok().map(|r| r.text);
                    }
                }
            }
            let mut found = candidates_of(&anchor.loc, same);
            found.find_enclosing(all);
            Some(found)
        })
        .collect()
}

/// The resolutions of one run of a write command. An anchor is sent to its
/// provider once, and every later question about it gets that answer: what
/// a write accepts, what it compares and what it marks then rest on one
/// reading of the resource, also when a provider would answer differently
/// the second time.
#[derive(Default)]
pub(crate) struct Once {
    seen: HashMap<Anchor, Resolved>,
}

impl Once {
    /// One resolution per anchor, in order.
    pub(crate) fn resolve(
        &mut self,
        providers: &mut Providers,
        anchors: &[&Anchor],
    ) -> Vec<Resolved> {
        let mut unseen: Vec<&Anchor> = Vec::new();
        for a in anchors {
            if !self.seen.contains_key(*a) && !unseen.contains(a) {
                unseen.push(a);
            }
        }
        let answers = resolve_anchors(providers, &unseen);
        for (a, r) in unseen.into_iter().zip(answers) {
            self.seen.insert(a.clone(), r);
        }
        anchors.iter().map(|a| self.seen[*a].clone()).collect()
    }
}

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

#[cfg(test)]
mod tests {
    use super::quoted;

    #[test]
    fn a_label_is_quoted_as_it_is() {
        assert_eq!(quoted(r"C:\dir and a 'word'"), r#""C:\dir and a 'word'""#);
        assert_eq!(quoted(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quoted("a\nb\tc\u{1b}"), r#""a\nb\tc\u{1b}""#);
        assert_eq!(quoted("日本語 é"), "\"日本語 é\"");
    }
}
