//! `hibi suggest-evidence` (sections 14.4 and 15.3).

use schemars::JsonSchema;
use serde::Serialize;

use crate::config::{PatternKind, ResourcePattern};
use crate::engine::{UnitRef, Warning};
use crate::error::{Code, Error};
use crate::model::{Anchor, Locator};
use crate::provider::{Providers, Unit};

use super::ask::{Group, Question, ask};
use super::gate::Gate;
use super::questions as q;
use super::{Judge, JudgeEnv, MODEL, cut};

/// The default of `suggest-evidence --limit`.
pub const DEFAULT_SUGGEST_LIMIT: usize = 10;

/// More candidates than this is `too_many_candidates`.
pub const MAX_CANDIDATES: usize = 10_000;

/// The default `--kind` list.
pub const DEFAULT_KINDS: [&str; 9] = [
    "function",
    "method",
    "class",
    "struct",
    "interface",
    "trait",
    "enum",
    "type",
    "variable",
];

/// The arguments of `hibi suggest-evidence`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestQuery {
    pub statement: String,
    /// `--in`: locator globs on scheme `file` or `repo`. At least one.
    pub within: Vec<String>,
    /// `--kind`. Empty means [`DEFAULT_KINDS`].
    pub kinds: Vec<String>,
    /// `--limit`: the cap on `candidates`; 0 for no cap.
    pub limit: usize,
}

/// One entry of `candidates` in the output of `suggest-evidence`. A
/// suggestion to read, never a finding.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SuggestCandidate {
    pub loc: Locator,
    pub unit: UnitRef,
    /// The unit's locator with its printed selector, to give to `claim`.
    /// `null` when no anchor selects this unit alone (it is identical to
    /// another unit where it stands).
    #[schemars(with = "crate::engine::Nullable<Anchor>")]
    pub anchor: Option<Anchor>,
    /// The Noul probability that reading this unit settles the statement.
    pub p: f64,
    /// `p >= 0.7`.
    pub fits: bool,
}

/// The output of `suggest-evidence` (section 15.3).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestOutput {
    pub model: String,
    pub statement: String,
    /// The number of candidates ranked, before the cut.
    pub considered: usize,
    /// True when no candidate has `p >= 0.7`.
    pub none_fits: bool,
    /// Ordered by `p` descending, then locator, start offset and key; cut
    /// to `--limit`.
    pub candidates: Vec<SuggestCandidate>,
    pub warnings: Vec<Warning>,
}

fn patterns(within: &[String]) -> Result<Vec<ResourcePattern>, Error> {
    if within.is_empty() {
        return Err(Error::invalid_argument(
            "--in is required: give at least one file: or repo: glob",
        ));
    }
    within
        .iter()
        .map(|w| {
            let p = ResourcePattern::parse(w).map_err(Error::invalid_argument)?;
            match p.kind() {
                PatternKind::File { .. } | PatternKind::Repo { .. } => Ok(p),
                PatternKind::Locator(_) => Err(Error::invalid_argument(format!(
                    "--in {w:?} must be a glob on scheme file or repo"
                ))
                .with_remediation(
                    "Give --in a file: or repo: glob, for example file:src/** or repo:api//src/**.",
                )),
            }
        })
        .collect()
}

/// The resources the `--in` patterns match, in pattern order, each once:
/// paths of the root's file list (without `.hibi/`) or of an allowed
/// sibling's. A pattern on a sibling that fails section 11.1 is
/// `invalid_argument`.
fn resources(pats: &[ResourcePattern], gate: &mut Gate<'_>) -> Result<Vec<Locator>, Error> {
    let mut out: Vec<Locator> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for p in pats {
        let found: Vec<String> = match p.kind() {
            PatternKind::File { .. } => gate
                .root_files()?
                .iter()
                .filter(|f| *f != ".hibi" && !f.starts_with(".hibi/") && p.matches_path(f))
                .map(|f| format!("file:{f}"))
                .collect(),
            PatternKind::Repo { alias, .. } => match gate.sibling_files(alias)? {
                Some(files) => files
                    .iter()
                    .filter(|f| p.matches_path(f))
                    .map(|f| format!("repo:{alias}//{f}"))
                    .collect(),
                None => {
                    return Err(Error::invalid_argument(format!(
                        "--in {:?}: sibling repository {alias} is not allowed",
                        p.as_str()
                    ))
                    .with_remediation("hibi providers"));
                }
            },
            PatternKind::Locator(_) => Vec::new(),
        };
        let mut found: Vec<Locator> = found
            .into_iter()
            .filter_map(|s| Locator::new(s).ok())
            .collect();
        found.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        for loc in found {
            if seen.insert(loc.as_str().to_owned()) {
                out.push(loc);
            }
        }
    }
    Ok(out)
}

/// `hibi suggest-evidence` (section 14.4): rank candidate units for a
/// statement, with a "none fits" signal.
///
/// Checks git first (section 14.1). A resource whose units cannot be
/// listed (not text, too large, unreadable) contributes no candidates; a
/// candidate whose text cannot be rendered is not sent and not counted.
/// More than [`MAX_CANDIDATES`] candidates is `too_many_candidates`,
/// before any request.
pub fn suggest_evidence(
    judge: &Judge,
    providers: &mut Providers,
    env: &JudgeEnv<'_>,
    query: &SuggestQuery,
) -> Result<SuggestOutput, Error> {
    let git = env.require_git()?;
    if query.statement.trim().is_empty() {
        return Err(Error::invalid_argument("the statement is empty"));
    }
    let pats = patterns(&query.within)?;
    let kinds: Vec<&str> = if query.kinds.is_empty() {
        DEFAULT_KINDS.to_vec()
    } else {
        query.kinds.iter().map(String::as_str).collect()
    };
    let mut gate = Gate::new(env, git);
    let locs = resources(&pats, &mut gate)?;

    let mut candidates: Vec<(Locator, Unit)> = Vec::new();
    for loc in &locs {
        if !gate.allows(loc)? {
            continue;
        }
        let Ok(provider) = providers.provider_for(loc) else {
            continue;
        };
        let Ok(units) = provider.units(loc, None) else {
            continue;
        };
        for u in units {
            if kinds.contains(&u.kind.as_str()) {
                candidates.push((loc.clone(), u));
            }
        }
        if candidates.len() > MAX_CANDIDATES {
            break;
        }
    }
    if candidates.len() > MAX_CANDIDATES {
        return Err(Error::new(
            Code::TooManyCandidates,
            format!(
                "--in and --kind select more than {MAX_CANDIDATES} candidate units; narrow them"
            ),
        ));
    }

    let mut sent: Vec<(Locator, Unit)> = Vec::new();
    let mut items: Vec<Vec<Question>> = Vec::new();
    for (loc, unit) in candidates {
        let rendered = match providers.provider_for(&loc) {
            Ok(p) => p.render(&loc, &unit.key).ok(),
            Err(_) => None,
        };
        let Some(rendered) = rendered else {
            continue;
        };
        let place = match &unit.label {
            Some(label) => format!("{} {label}", loc.as_str()),
            None => loc.as_str().to_owned(),
        };
        items.push(vec![Question {
            suffix: "",
            body: q::suggest_question(&place, &cut(&rendered.text)),
        }]);
        sent.push((loc, unit));
    }
    let groups = [Group {
        state: q::suggest_state(&query.statement),
        items,
    }];
    let answers = ask(judge, env.cache, env.now, &groups)?;

    let mut ranked: Vec<(SuggestCandidate, Option<usize>)> = Vec::with_capacity(sent.len());
    for ((loc, unit), a) in sent.into_iter().zip(&answers[0]) {
        let p = a[0]
            .noul()
            .ok_or_else(|| Error::internal("a suggest-evidence question got a non-Noul answer"))?;
        ranked.push((
            SuggestCandidate {
                anchor: crate::engine::selects_its_unit(&unit)
                    .then(|| Anchor::new(loc.clone(), unit.sel.clone())),
                unit: UnitRef::from(&unit),
                loc,
                p,
                fits: p >= q::FITS_AT,
            },
            unit.range.map(|r| r.start),
        ));
    }
    ranked.sort_by(|(a, sa), (b, sb)| {
        b.p.total_cmp(&a.p)
            .then_with(|| a.loc.as_str().cmp(b.loc.as_str()))
            .then_with(|| sa.cmp(sb))
            .then_with(|| a.unit.key.cmp(&b.unit.key))
    });
    let considered = ranked.len();
    let none_fits = !ranked.iter().any(|(c, _)| c.fits);
    if query.limit > 0 {
        ranked.truncate(query.limit);
    }
    Ok(SuggestOutput {
        model: MODEL.to_owned(),
        statement: query.statement.clone(),
        considered,
        none_fits,
        candidates: ranked.into_iter().map(|(c, _)| c).collect(),
        warnings: Vec::new(),
    })
}
