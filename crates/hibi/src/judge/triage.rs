//! `hibi triage` (sections 14.3 and 15.3).

use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::Serialize;

use crate::engine::{
    self, CoverEnv, SinceRun, TEXT_EXCERPT_CHARS, UncoveredUnit, UnitRef, Warning, cut_text,
};
use crate::error::Error;
use crate::model::{Anchor, Locator};
use crate::provider::Providers;
use crate::store::Store;

use super::ask::{Answer, Group, Question, ask};
use super::gate::Gate;
use super::questions as q;
use super::{Judge, JudgeEnv, MODEL, cut};

/// The default of `triage --limit`.
pub const DEFAULT_TRIAGE_LIMIT: usize = 50;

/// The unit kinds triage sends (decision 55).
pub const PROSE_KINDS: [&str; 4] = ["sentence", "list-item", "table-row", "heading"];

/// The arguments of `hibi triage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageQuery {
    /// `--scope` names. Empty means every scope.
    pub scopes: Vec<String>,
    /// `--limit`: the cap on `items`; 0 for no cap.
    pub limit: usize,
}

impl Default for TriageQuery {
    fn default() -> Self {
        TriageQuery {
            scopes: Vec::new(),
            limit: DEFAULT_TRIAGE_LIMIT,
        }
    }
}

/// `band` of a triage item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    /// `checkable >= 0.7`.
    Likely,
    /// Between the two thresholds.
    Unsure,
    /// `checkable <= 0.3`.
    Unlikely,
}

impl Band {
    pub fn of(p: f64) -> Band {
        if p >= q::LIKELY_AT {
            Band::Likely
        } else if p <= q::UNLIKELY_AT {
            Band::Unlikely
        } else {
            Band::Unsure
        }
    }
}

/// `evidence` of a triage item: the chosen option when its probability is
/// at least 0.6, otherwise `unsure`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLabel {
    Code,
    Config,
    Doc,
    External,
    None,
    Unsure,
}

impl EvidenceLabel {
    /// The label of a Choice answer: its option when `p >= 0.6`, else
    /// `unsure`. A Noul answer is `unsure`.
    pub fn of(answer: &Answer) -> EvidenceLabel {
        let Answer::Choice { choice, p } = answer else {
            return EvidenceLabel::Unsure;
        };
        if *p < q::EVIDENCE_AT {
            return EvidenceLabel::Unsure;
        }
        match choice.as_str() {
            "code" => EvidenceLabel::Code,
            "config" => EvidenceLabel::Config,
            "doc" => EvidenceLabel::Doc,
            "external" => EvidenceLabel::External,
            "none" => EvidenceLabel::None,
            _ => EvidenceLabel::Unsure,
        }
    }
}

/// One entry of `items` in the output of `triage`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TriageItem {
    pub loc: Locator,
    pub unit: UnitRef,
    /// The unit text cut to 240 characters.
    pub text: String,
    /// The unit's locator with its completed selector, for `claim --stdin`.
    pub anchor: Anchor,
    /// The Noul probability that the text states a checkable fact.
    pub checkable: f64,
    pub band: Band,
    pub evidence: EvidenceLabel,
}

/// The output of `triage` (section 15.3).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct TriageOutput {
    /// The model id the answers come from.
    pub model: String,
    /// Ordered by `checkable` descending, then locator, start offset and
    /// key; cut to `--limit`.
    pub items: Vec<TriageItem>,
    /// The number of ranked items before the cut.
    pub considered: usize,
    /// Uncovered units that were not sent: not prose served by the file
    /// provider, not in a file list (section 14.2), or not renderable.
    pub skipped: usize,
    pub warnings: Vec<Warning>,
}

struct Pending<'u> {
    unit: &'u UncoveredUnit,
    text: String,
}

/// `hibi triage` (section 14.3): rank the uncovered prose units that `cover`
/// would list with the same scopes and `since`.
///
/// Checks git first (section 14.1). A unit seen in several scopes is
/// ranked once. One request group per resource, with the document's
/// locator as state; two questions per unit.
pub fn triage(
    judge: &Judge,
    store: &dyn Store,
    providers: &mut Providers,
    env: &JudgeEnv<'_>,
    query: &TriageQuery,
    since: Option<SinceRun<'_>>,
) -> Result<TriageOutput, Error> {
    let git = env.require_git()?;
    let cover_env = CoverEnv {
        config: env.config,
        user: env.user,
        root: env.root,
        git,
    };
    let coverage = engine::coverage(store, providers, &cover_env, &query.scopes, since)?;
    let mut gate = Gate::new(env, git);

    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    let mut skipped = 0usize;
    // Per resource, in coverage order: the items to send.
    let mut pending: Vec<Vec<Pending<'_>>> = Vec::new();
    let mut request_groups: Vec<Group> = Vec::new();
    let mut group_of: HashMap<&str, usize> = HashMap::new();
    for unit in coverage.scopes.iter().flat_map(|s| &s.uncovered) {
        if !seen.insert((unit.loc.as_str(), unit.unit.key.as_str())) {
            continue;
        }
        if !unit.loc.is_file_or_repo()
            || !PROSE_KINDS.contains(&unit.unit.kind.as_str())
            || !gate.allows(&unit.loc)?
        {
            skipped += 1;
            continue;
        }
        let rendered = match providers.provider_for(&unit.loc) {
            Ok(p) => p.render(&unit.loc, &unit.unit.key).ok(),
            Err(_) => None,
        };
        let Some(rendered) = rendered else {
            skipped += 1;
            continue;
        };
        let text = cut(&rendered.text);
        let questions = vec![
            Question {
                suffix: q::CHECKABLE,
                body: q::triage_checkable(&rendered.context, &text),
            },
            Question {
                suffix: q::EVIDENCE,
                body: q::triage_evidence(&rendered.context, &text),
            },
        ];
        let g = *group_of.entry(unit.loc.as_str()).or_insert_with(|| {
            pending.push(Vec::new());
            request_groups.push(Group {
                state: q::triage_state(unit.loc.as_str()),
                items: Vec::new(),
            });
            request_groups.len() - 1
        });
        pending[g].push(Pending {
            unit,
            text: rendered.text,
        });
        request_groups[g].items.push(questions);
    }

    let answers = ask(judge, env.cache, env.now, &request_groups)?;

    let mut items: Vec<(TriageItem, Option<usize>)> = Vec::new();
    for (group, group_answers) in pending.iter().zip(&answers) {
        for (p, a) in group.iter().zip(group_answers) {
            let checkable = a[0].noul().ok_or_else(|| {
                Error::internal("the triage checkable question got a non-Noul answer")
            })?;
            items.push((
                TriageItem {
                    loc: p.unit.loc.clone(),
                    unit: UnitRef::from(&p.unit.unit),
                    text: cut_text(&p.text, TEXT_EXCERPT_CHARS),
                    anchor: Anchor::new(p.unit.loc.clone(), p.unit.unit.sel.clone()),
                    checkable,
                    band: Band::of(checkable),
                    evidence: EvidenceLabel::of(&a[1]),
                },
                p.unit.unit.range.map(|r| r.start),
            ));
        }
    }
    items.sort_by(|(a, sa), (b, sb)| {
        b.checkable
            .total_cmp(&a.checkable)
            .then_with(|| a.loc.as_str().cmp(b.loc.as_str()))
            .then_with(|| sa.cmp(sb))
            .then_with(|| a.unit.key.cmp(&b.unit.key))
    });
    let considered = items.len();
    if query.limit > 0 {
        items.truncate(query.limit);
    }
    Ok(TriageOutput {
        model: MODEL.to_owned(),
        items: items.into_iter().map(|(i, _)| i).collect(),
        considered,
        skipped,
        warnings: coverage.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_and_labels() {
        assert_eq!(Band::of(0.7), Band::Likely);
        assert_eq!(Band::of(0.69), Band::Unsure);
        assert_eq!(Band::of(0.31), Band::Unsure);
        assert_eq!(Band::of(0.3), Band::Unlikely);
        let c = |choice: &str, p| Answer::Choice {
            choice: choice.into(),
            p,
        };
        assert_eq!(EvidenceLabel::of(&c("config", 0.6)), EvidenceLabel::Config);
        assert_eq!(EvidenceLabel::of(&c("config", 0.59)), EvidenceLabel::Unsure);
        assert_eq!(EvidenceLabel::of(&c("none", 0.9)), EvidenceLabel::None);
    }
}
