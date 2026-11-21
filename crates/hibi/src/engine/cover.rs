//! Coverage (spec section 10) and `cover --since` (section 10.3).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use schemars::JsonSchema;
use serde::Serialize;

use crate::config::{Exclusion, PatternKind, RepoConfig, Scope, UserConfig, allow_sibling};
use crate::error::{Code, Error};
use crate::git::Git;
use crate::model::{Anchor, Locator};
use crate::provider::{Provider, ProviderErrorCode, Providers, Unit};
use crate::store::Store;

use super::fresh::Index;
use super::read::{SinceRun, SinceView};
use super::resolve::{
    Resolved, info_of, reason_of_no_provider, resolve_anchors, side_of_error, validate_units,
};
use super::types::{Nullable, Reason, TEXT_EXCERPT_CHARS, UnitRef, Warning, WarningCode, cut_text};

/// The default of `cover --limit`.
pub const DEFAULT_COVER_LIMIT: usize = 200;

/// What coverage reads besides the store and the providers.
pub struct CoverEnv<'a> {
    pub config: &'a RepoConfig,
    pub user: &'a UserConfig,
    pub root: &'a Path,
    pub git: &'a Git,
}

/// The arguments of `hibi cover`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverQuery {
    /// `--scope` names. Empty means every scope.
    pub scopes: Vec<String>,
    /// `--limit`: the cap on `items` per scope; 0 for no cap.
    pub limit: usize,
}

impl Default for CoverQuery {
    fn default() -> Self {
        CoverQuery {
            scopes: Vec::new(),
            limit: DEFAULT_COVER_LIMIT,
        }
    }
}

/// Why a scope was not evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Skipped {
    /// `--since` on a `repo:` or external scope (section 10.3).
    SinceUnsupported,
}

/// One exclusion of a scope with what it took out (section 10.1). For an
/// exclusion without `unit`, `matched` is the number of resources it took
/// out of the scope; they are not read. For one with `unit` it is the
/// number of units it excluded. What several exclusions of a kind match
/// counts for the first in declaration order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ExclusionReport {
    pub resource: String,
    #[schemars(with = "Nullable<String>")]
    pub unit: Option<String>,
    pub reason: String,
    pub matched: usize,
}

/// A resource whose units cannot be listed, with the reason of section 9.2.
/// For a `repo:` scope whose alias is refused, `loc` is the scope's resource
/// pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct UnavailableResource {
    pub loc: String,
    pub reason: Reason,
}

/// An uncovered unit of a scope, as `Provider::units` returned it (without
/// text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncoveredUnit {
    pub loc: Locator,
    pub unit: Unit,
}

/// The coverage of one scope, with every uncovered unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeCoverage {
    pub name: String,
    pub resource: String,
    pub kinds: Vec<String>,
    pub units: usize,
    pub covered: usize,
    pub excluded: usize,
    /// How many resources exclusions without `unit` took out of the scope.
    pub excluded_resources: usize,
    /// Ordered by locator, then start offset, then key.
    pub uncovered: Vec<UncoveredUnit>,
    pub exclusions: Vec<ExclusionReport>,
    /// Ordered by locator.
    pub unavailable: Vec<UnavailableResource>,
    pub skipped: Option<Skipped>,
}

/// The coverage of the selected scopes, in `hibi.toml` order. The judge's
/// `triage` reads the uncovered units from here and their text with
/// [`unit_texts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub since: Option<SinceView>,
    pub scopes: Vec<ScopeCoverage>,
    pub warnings: Vec<Warning>,
}

/// `summary` in the output of `cover`: the sums over the listed scopes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CoverSummary {
    pub units: usize,
    pub covered: usize,
    /// Units excluded by exclusions that name a unit.
    pub excluded: usize,
    pub uncovered: usize,
    /// How many resources exclusions without `unit` took out of the listed scopes, summed
    /// over the scopes.
    /// They are not read, so their units are in none of the unit counts.
    #[serde(rename = "excludedResources")]
    pub excluded_resources: usize,
}

/// One uncovered unit in the output of `cover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CoverItem {
    pub loc: Locator,
    pub unit: UnitRef,
    /// The unit text cut to 240 characters; `null` when the provider did
    /// not return the unit for its own completed selector.
    #[schemars(with = "Nullable<String>")]
    pub text: Option<String>,
    /// The unit's locator with its completed selector; can be passed to
    /// `claim --stdin`.
    /// `null` when another uncovered unit of the resource carries the same
    /// printed anchor: the two are identical where they stand and no
    /// anchor selects this one alone. Such a unit is covered by a claim on
    /// the unit that contains it, or taken out of the scope by an exclusion
    /// that names its key.
    #[schemars(with = "Nullable<Anchor>")]
    pub anchor: Option<Anchor>,
}

/// One scope in the output of `cover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ScopeReport {
    pub name: String,
    pub resource: String,
    pub kinds: Vec<String>,
    pub units: usize,
    pub covered: usize,
    /// Units excluded by exclusions that name a unit.
    pub excluded: usize,
    pub uncovered: usize,
    /// How many resources exclusions without `unit` took out of the scope.
    /// They are not read, so their units are in none of the unit counts.
    #[serde(rename = "excludedResources")]
    pub excluded_resources: usize,
    /// True when `items` was cut to `--limit`.
    pub truncated: bool,
    #[schemars(with = "Nullable<Skipped>")]
    pub skipped: Option<Skipped>,
    pub items: Vec<CoverItem>,
    pub exclusions: Vec<ExclusionReport>,
    pub unavailable: Vec<UnavailableResource>,
}

/// The output of `cover`. Scopes are in `hibi.toml` order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CoverOutput {
    #[schemars(with = "Nullable<SinceView>")]
    pub since: Option<SinceView>,
    pub summary: CoverSummary,
    pub scopes: Vec<ScopeReport>,
    pub warnings: Vec<Warning>,
}

/// The keys of one resource that are covered (section 10.2): every key of
/// `resolved` (the keys that claim sides resolve to) that is in `units`,
/// and each ancestor by the `parent` chain. The walk stops at a `null`
/// parent, at a key that is not in `units`, and at a key already visited.
pub fn covered_keys<'u, 'k>(
    units: &'u [Unit],
    resolved: impl IntoIterator<Item = &'k str>,
) -> BTreeSet<&'u str> {
    let by_key: HashMap<&str, &Unit> = units.iter().map(|u| (u.key.as_str(), u)).collect();
    let mut covered: BTreeSet<&'u str> = BTreeSet::new();
    for start in resolved {
        let mut next = by_key.get(start).copied();
        while let Some(unit) = next {
            if !covered.insert(unit.key.as_str()) {
                break;
            }
            next = unit.parent.as_deref().and_then(|p| by_key.get(p).copied());
        }
    }
    covered
}

/// The coverage status of one scope unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitCoverage {
    Covered,
    /// Not covered, and the exclusion at this position is the first that
    /// matches.
    Excluded(usize),
    Uncovered,
}

/// Classify a scope unit (section 10.2): `covered` when some side covers
/// it, `excluded` when not covered and an exclusion matches, otherwise
/// `uncovered`.
pub fn classify(
    loc: &Locator,
    unit: &Unit,
    covered: &BTreeSet<&str>,
    exclusions: &[Exclusion],
) -> UnitCoverage {
    if covered.contains(unit.key.as_str()) {
        return UnitCoverage::Covered;
    }
    let hit = exclusions.iter().position(|x| {
        x.resource.matches_locator(loc) && x.unit.as_deref().is_none_or(|k| k == unit.key)
    });
    match hit {
        Some(i) => UnitCoverage::Excluded(i),
        None => UnitCoverage::Uncovered,
    }
}

/// True when `--since` keeps the unit (section 10.3): no unit of the same
/// resource at the base has both the same key and the same fingerprint.
pub fn changed_since(unit: &Unit, base_units: &[Unit]) -> bool {
    !base_units
        .iter()
        .any(|b| b.key == unit.key && b.fp == unit.fp)
}

type Listing = Result<Vec<Unit>, Reason>;

/// Every unit of a resource, validated by section 2.5, or the reason of
/// section 9.2 why they cannot be listed.
fn list_units(provider: &mut dyn Provider, loc: &Locator) -> Result<Vec<Unit>, ProviderErrorOr> {
    let info = info_of(provider).map_err(|_| ProviderErrorOr::Reason(Reason::ProviderFailed))?;
    let units = provider
        .units(loc, None)
        .map_err(|e| ProviderErrorOr::Code(e.code))?;
    validate_units(&info, &units).map_err(|_| ProviderErrorOr::Reason(Reason::ProviderError))?;
    Ok(units)
}

enum ProviderErrorOr {
    Code(ProviderErrorCode),
    Reason(Reason),
}

impl ProviderErrorOr {
    fn reason(&self) -> Reason {
        match self {
            ProviderErrorOr::Code(c) => side_of_error(*c).1,
            ProviderErrorOr::Reason(r) => *r,
        }
    }
}

fn list_registered(providers: &mut Providers, loc: &Locator) -> Listing {
    match providers.provider_for(loc) {
        Err(np) => Err(reason_of_no_provider(np)),
        Ok(p) => list_units(p, loc).map_err(|e| e.reason()),
    }
}

/// The file list of a repository directory (section 10.4) without the
/// paths under `.hibi/`, which are never resources (section 10.1). This
/// holds for the root and for a sibling repository alike.
fn listed_files(env: &CoverEnv<'_>, dir: &Path) -> Result<Vec<String>, Error> {
    let mut files = env.git.file_list(dir)?;
    files.retain(|p| p != ".hibi" && !p.starts_with(".hibi/"));
    Ok(files)
}

/// The resources of a scope, or the one `unavailable` entry of a refused
/// sibling alias.
fn scope_resources(
    scope: &Scope,
    env: &CoverEnv<'_>,
    root_files: &mut Option<Vec<String>>,
) -> Result<Result<Vec<Locator>, UnavailableResource>, Error> {
    let locators = |prefix: &str, files: &[String]| -> Vec<Locator> {
        files
            .iter()
            .filter(|p| scope.resource.matches_path(p))
            .filter_map(|p| Locator::new(format!("{prefix}{p}")).ok())
            .collect()
    };
    match scope.resource.kind() {
        PatternKind::File { .. } => {
            if root_files.is_none() {
                *root_files = Some(listed_files(env, env.root)?);
            }
            Ok(Ok(locators("file:", root_files.as_deref().unwrap_or(&[]))))
        }
        PatternKind::Repo { alias, .. } => {
            let dir = allow_sibling(alias, env.config, env.user, env.root, |d| {
                env.git.is_toplevel(d)
            });
            match dir {
                Err(refusal) => Ok(Err(UnavailableResource {
                    loc: scope.resource.as_str().to_owned(),
                    reason: match refusal.reason() {
                        "repo_not_found" => Reason::RepoNotFound,
                        _ => Reason::RepoNotAllowed,
                    },
                })),
                Ok(dir) => {
                    let files = listed_files(env, &dir)?;
                    Ok(Ok(locators(&format!("repo:{alias}//"), &files)))
                }
            }
        }
        PatternKind::Locator(loc) => Ok(Ok(vec![loc.clone()])),
    }
}

/// For a scope on an external scheme: one `unknown_kind` warning per kind
/// of the scope that the provider's `hello` does not list. The list is
/// informational and may be empty, which means the kinds are not known; a
/// provider that is not registered or does not start states none either.
/// Neither case gives a warning.
fn unknown_kinds(scope: &Scope, providers: &mut Providers, loc: &Locator) -> Vec<Warning> {
    let Some(info) = providers
        .provider_for(loc)
        .ok()
        .and_then(|p| info_of(p).ok())
    else {
        return Vec::new();
    };
    if info.kinds.is_empty() {
        return Vec::new();
    }
    scope
        .kinds
        .iter()
        .filter(|kind| !info.kinds.contains(kind))
        .map(|kind| {
            Warning::new(
                WarningCode::UnknownKind,
                format!(
                    "scope {}: the {} provider lists no unit kind {kind:?}, so it selects nothing; its kinds are {}",
                    scope.name,
                    scope.resource.scheme(),
                    info.kinds.join(", ")
                ),
            )
        })
        .collect()
}

/// Compute coverage (section 10) for the named scopes, or for every scope
/// when `scopes` is empty.
///
/// Every side of every live claim is resolved once, whatever the claim's
/// state. No scope in `hibi.toml` is `no_scopes`; an unknown name is
/// `unknown_scope`. With `since`, only scopes on scheme `file` are
/// evaluated, only their changed paths are listed, and a unit is kept only
/// when no base unit has its key and fingerprint; a base listing that fails
/// with anything but `not_found` puts the resource under `unavailable`.
pub fn coverage(
    store: &dyn Store,
    providers: &mut Providers,
    env: &CoverEnv<'_>,
    scopes: &[String],
    mut since: Option<SinceRun<'_>>,
) -> Result<Coverage, Error> {
    if env.config.scopes.is_empty() {
        return Err(Error::new(
            Code::NoScopes,
            "no scope is declared in .hibi/hibi.toml",
        ));
    }
    for name in scopes {
        if env.config.scope(name).is_none() {
            return Err(Error::new(
                Code::UnknownScope,
                format!("no scope is named {name}"),
            ));
        }
    }
    let selected: Vec<&Scope> = env
        .config
        .scopes
        .iter()
        .filter(|s| scopes.is_empty() || scopes.contains(&s.name))
        .collect();

    let ledger = store.load()?;
    let mut warnings = Warning::from_store_all(&ledger.warnings)?;
    let index = Index::new(&ledger);
    let anchors: Vec<&Anchor> = index.live().flat_map(|c| c.sides()).collect();
    let resolved = resolve_anchors(providers, &anchors);
    // Locator to the keys that sides resolve to.
    let mut direct: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (a, r) in anchors.iter().zip(&resolved) {
        if let Resolved::One(u) = r {
            direct
                .entry(a.loc.as_str())
                .or_default()
                .insert(u.key.as_str());
        }
    }

    let changed = match &since {
        None => None,
        Some(run) => Some(run.git.changed_paths(run.root, &run.since.base)?),
    };
    let mut root_files: Option<Vec<String>> = None;
    let mut listings: HashMap<Locator, Listing> = HashMap::new();
    let mut base_listings: HashMap<Locator, Listing> = HashMap::new();
    let mut out = Vec::with_capacity(selected.len());

    for scope in selected {
        let mut sc = ScopeCoverage {
            name: scope.name.clone(),
            resource: scope.resource.as_str().to_owned(),
            kinds: scope.kinds.clone(),
            units: 0,
            covered: 0,
            excluded: 0,
            excluded_resources: 0,
            uncovered: Vec::new(),
            exclusions: scope
                .exclude
                .iter()
                .map(|x| ExclusionReport {
                    resource: x.resource.as_str().to_owned(),
                    unit: x.unit.clone(),
                    reason: x.reason.clone(),
                    matched: 0,
                })
                .collect(),
            unavailable: Vec::new(),
            skipped: None,
        };
        if since.is_some() && scope.resource.scheme() != "file" {
            sc.skipped = Some(Skipped::SinceUnsupported);
            out.push(sc);
            continue;
        }
        let mut resources = match scope_resources(scope, env, &mut root_files)? {
            Ok(r) => r,
            Err(unavailable) => {
                sc.unavailable.push(unavailable);
                out.push(sc);
                continue;
            }
        };
        if let PatternKind::Locator(loc) = scope.resource.kind() {
            warnings.extend(unknown_kinds(scope, providers, loc));
        }
        if let Some(changed) = &changed {
            resources.retain(|loc| {
                loc.as_str()
                    .strip_prefix("file:")
                    .is_some_and(|p| changed.contains(p))
            });
        }
        for loc in resources {
            // A resource that an exclusion without `unit` matches is out of
            // the scope as a whole. It is not read: a file that cannot be
            // read or parsed is excluded this way, so asking for its units
            // would only report it as unavailable again. The exclusion
            // counts the resource; an exclusion with `unit` counts units
            // and needs the resource read.
            if let Some(i) = scope
                .exclude
                .iter()
                .position(|x| x.unit.is_none() && x.resource.matches_locator(&loc))
            {
                sc.exclusions[i].matched += 1;
                sc.excluded_resources += 1;
                continue;
            }
            let listing = listings
                .entry(loc.clone())
                .or_insert_with(|| list_registered(providers, &loc));
            let units = match listing {
                Ok(units) => &*units,
                Err(reason) => {
                    sc.unavailable.push(UnavailableResource {
                        loc: loc.as_str().to_owned(),
                        reason: *reason,
                    });
                    continue;
                }
            };
            let base_units: Option<&[Unit]> = match &mut since {
                None => None,
                Some(run) => {
                    let listing = base_listings.entry(loc.clone()).or_insert_with(|| {
                        match list_units(run.base, &loc) {
                            Ok(units) => Ok(units),
                            Err(ProviderErrorOr::Code(ProviderErrorCode::NotFound)) => {
                                Ok(Vec::new())
                            }
                            Err(e) => Err(e.reason()),
                        }
                    });
                    match listing {
                        Ok(units) => Some(units.as_slice()),
                        Err(reason) => {
                            sc.unavailable.push(UnavailableResource {
                                loc: loc.as_str().to_owned(),
                                reason: *reason,
                            });
                            continue;
                        }
                    }
                }
            };
            let keys = direct.get(loc.as_str());
            let covered = covered_keys(units, keys.into_iter().flatten().copied());
            for unit in units {
                if !scope.kinds.contains(&unit.kind) {
                    continue;
                }
                if base_units.is_some_and(|base| !changed_since(unit, base)) {
                    continue;
                }
                sc.units += 1;
                match classify(&loc, unit, &covered, &scope.exclude) {
                    UnitCoverage::Covered => sc.covered += 1,
                    UnitCoverage::Excluded(i) => {
                        sc.excluded += 1;
                        sc.exclusions[i].matched += 1;
                    }
                    UnitCoverage::Uncovered => sc.uncovered.push(UncoveredUnit {
                        loc: loc.clone(),
                        unit: unit.clone(),
                    }),
                }
            }
        }
        sc.uncovered.sort_by(|a, b| {
            (a.loc.as_str(), a.unit.range.map(|r| r.start), &a.unit.key).cmp(&(
                b.loc.as_str(),
                b.unit.range.map(|r| r.start),
                &b.unit.key,
            ))
        });
        sc.unavailable.sort_by(|a, b| a.loc.cmp(&b.loc));
        out.push(sc);
    }

    Ok(Coverage {
        since: since.as_ref().map(|run| SinceView::from(run.since)),
        scopes: out,
        warnings,
    })
}

/// The full text of each unit. A provider that can `render` is asked for
/// the unit by its key, one call per unit: the answer is one text, whatever
/// else the resource holds. Resolving the unit's completed selector instead
/// returns every unit the selector matches, which for a resource of many
/// identical units is all of them, once per listed unit. A provider without
/// `render` is asked that way, in one batched `resolve` per scheme. `None`
/// where the provider gives no text for the unit.
pub fn unit_texts(providers: &mut Providers, units: &[&UncoveredUnit]) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = vec![None; units.len()];
    let mut by_selector: Vec<usize> = Vec::new();
    for (i, u) in units.iter().enumerate() {
        let Ok(provider) = providers.provider_for_scheme(u.loc.scheme()) else {
            continue;
        };
        let can_render = provider.info().is_ok_and(|info| info.capabilities.render);
        if can_render {
            out[i] = provider.render(&u.loc, &u.unit.key).ok().map(|r| r.text);
        } else {
            by_selector.push(i);
        }
    }
    let anchors: Vec<Anchor> = by_selector
        .iter()
        .map(|&i| Anchor::new(units[i].loc.clone(), units[i].unit.sel.clone()))
        .collect();
    let refs: Vec<&Anchor> = anchors.iter().collect();
    for (i, r) in by_selector
        .into_iter()
        .zip(resolve_anchors(providers, &refs))
    {
        let key = &units[i].unit.key;
        let found = match &r {
            Resolved::One(x) if x.key == *key => Some(&**x),
            Resolved::Several(xs) => xs.units.iter().find(|x| x.key == *key),
            _ => None,
        };
        out[i] = found.and_then(|x| x.text.clone());
    }
    out
}

/// `hibi cover` (section 15.3): [`coverage`], with `items` cut to
/// `query.limit` per scope and each listed item's text cut to 240
/// characters. Counts are never capped.
pub fn cover(
    store: &dyn Store,
    providers: &mut Providers,
    env: &CoverEnv<'_>,
    query: &CoverQuery,
    since: Option<SinceRun<'_>>,
) -> Result<CoverOutput, Error> {
    let coverage = coverage(store, providers, env, &query.scopes, since)?;
    let shown = |sc: &ScopeCoverage| match query.limit {
        0 => sc.uncovered.len(),
        n => n.min(sc.uncovered.len()),
    };
    let listed: Vec<&UncoveredUnit> = coverage
        .scopes
        .iter()
        .flat_map(|sc| &sc.uncovered[..shown(sc)])
        .collect();
    let mut texts = unit_texts(providers, &listed).into_iter();

    let mut summary = CoverSummary::default();
    let mut scopes = Vec::with_capacity(coverage.scopes.len());
    for sc in &coverage.scopes {
        let n = shown(sc);
        // How many uncovered units of the scope carry each printed anchor.
        let mut printed: HashMap<(&Locator, &crate::model::Selector), usize> = HashMap::new();
        for u in &sc.uncovered {
            *printed.entry((&u.loc, &u.unit.sel)).or_default() += 1;
        }
        let items = sc.uncovered[..n]
            .iter()
            .map(|u| CoverItem {
                loc: u.loc.clone(),
                unit: UnitRef::from(&u.unit),
                text: texts
                    .next()
                    .flatten()
                    .map(|t| cut_text(&t, TEXT_EXCERPT_CHARS)),
                anchor: (printed.get(&(&u.loc, &u.unit.sel)) == Some(&1))
                    .then(|| Anchor::new(u.loc.clone(), u.unit.sel.clone())),
            })
            .collect();
        summary.units += sc.units;
        summary.covered += sc.covered;
        summary.excluded += sc.excluded;
        summary.uncovered += sc.uncovered.len();
        summary.excluded_resources += sc.excluded_resources;
        scopes.push(ScopeReport {
            name: sc.name.clone(),
            resource: sc.resource.clone(),
            kinds: sc.kinds.clone(),
            units: sc.units,
            covered: sc.covered,
            excluded: sc.excluded,
            uncovered: sc.uncovered.len(),
            excluded_resources: sc.excluded_resources,
            truncated: n < sc.uncovered.len(),
            skipped: sc.skipped,
            items,
            exclusions: sc.exclusions.clone(),
            unavailable: sc.unavailable.clone(),
        });
    }
    Ok(CoverOutput {
        since: coverage.since,
        summary,
        scopes,
        warnings: coverage.warnings,
    })
}
