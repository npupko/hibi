//! `hibi status --judge-order` (section 14.5).

use crate::engine::{self, ClaimState, ClaimView, SinceRun, StatusOutput, StatusQuery};
use crate::error::Error;
use crate::provider::Providers;
use crate::store::Store;

use super::ask::{Group, Question, ask};
use super::gate::Gate;
use super::questions as q;
use super::{Judge, JudgeEnv, cut};

/// The rendered text of each side, or `None` when a side cannot be
/// rendered: a `file:` or `repo:` side outside the file lists (section
/// 14.2), or any case of [`engine::render_sides`].
fn contents(
    view: &ClaimView,
    providers: &mut Providers,
    gate: &mut Gate<'_>,
) -> Result<Option<Vec<String>>, Error> {
    for side in &view.sides {
        if !gate.allows(&side.anchor.loc)? {
            return Ok(None);
        }
    }
    Ok(engine::render_sides(providers, view).map(|texts| texts.iter().map(|t| cut(t)).collect()))
}

/// `hibi status --judge-order`: [`engine::status`], with the listed stale
/// claims reordered by the judge (section 14.5).
///
/// Checks git first (section 14.1). Each listed stale claim whose sides can
/// all be rendered is one request with one question. The listed claims
/// come in this order: stale claims with a side that cannot be rendered, by
/// id; then the other stale claims by the answer ascending (most likely
/// broken first), ties by id; then every other listed claim, by id. The
/// answer is not part of the output, and the set of claims, their states
/// and every other field are those of plain `status`.
pub fn judge_order(
    judge: &Judge,
    store: &dyn Store,
    providers: &mut Providers,
    env: &JudgeEnv<'_>,
    query: &StatusQuery,
    since: Option<SinceRun<'_>>,
) -> Result<StatusOutput, Error> {
    let git = env.require_git()?;
    let mut out = engine::status(store, providers, env.git, query, since, env.now)?;
    let mut gate = Gate::new(env, git);

    let mut unrendered = Vec::new();
    let mut asked: Vec<ClaimView> = Vec::new();
    let mut groups = Vec::new();
    let mut rest = Vec::new();
    for view in std::mem::take(&mut out.claims) {
        if view.state != ClaimState::Stale {
            rest.push(view);
            continue;
        }
        match contents(&view, providers, &mut gate)? {
            None => unrendered.push(view),
            Some(content) => {
                groups.push(Group {
                    state: q::order_state(&view.statement),
                    items: vec![vec![Question {
                        suffix: "",
                        body: q::order_question(&content),
                    }]],
                });
                asked.push(view);
            }
        }
    }
    let answers = ask(judge, env.cache, env.now, &groups)?;
    let mut scored = Vec::with_capacity(asked.len());
    for (view, a) in asked.into_iter().zip(&answers) {
        let p = a[0][0]
            .noul()
            .ok_or_else(|| Error::internal("a judge-order question got a non-Noul answer"))?;
        scored.push((p, view));
    }
    unrendered.sort_by(|a, b| a.id.cmp(&b.id));
    scored.sort_by(|(pa, a), (pb, b)| pa.total_cmp(pb).then_with(|| a.id.cmp(&b.id)));
    rest.sort_by(|a, b| a.id.cmp(&b.id));
    out.claims = unrendered
        .into_iter()
        .chain(scored.into_iter().map(|(_, v)| v))
        .chain(rest)
        .collect();
    Ok(out)
}
