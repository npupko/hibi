//! The write and read commands end to end on `GitTextStore` in a temporary
//! directory, with a scripted provider.

#![cfg(feature = "git-store")]

use hibi::engine::{
    self, ClaimState, ListOutput, ListQuery, MarkInput, ReanchorInput, RetireInput, StatusQuery,
};
use hibi::store::Store;
use hibi::store::git_text::{GitTextStore, init};

use crate::support::*;

#[test]
fn claim_status_mark_reanchor_retire_on_the_git_text_store() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path()).unwrap();
    let store = GitTextStore::open(dir.path()).unwrap();
    let w = file_world();
    set(
        &w,
        "file:src/retry.rs",
        vec![
            sym_unit("function:retry", "function", "fn retry() { cap(5) }"),
            sym_unit("function:cap", "function", "fn cap(n: u32) {}"),
        ],
    );
    set(
        &w,
        "file:docs/retry.md",
        vec![sym_unit("s1", "sentence", "Retries are capped at 5.")],
    );
    let mut p = providers(&w, &["file"]);
    let empty = store.snapshot_id().unwrap();

    // claim
    let inputs = [
        claim_input(
            "Retries are capped at 5.",
            "file:docs/retry.md#s1",
            &["file:src/retry.rs#function:retry"],
        ),
        claim_input("cap takes a count.", "file:src/retry.rs#function:cap", &[]),
    ];
    let t0 = ts("2026-11-01T00:00:00Z");
    let claimed = engine::claim(&store, &mut p, &no_git(), &inputs, true, t0).unwrap();
    let doc = claimed.claims[0].id.clone();
    let cap = claimed.claims[1].id.clone();
    let after_claim = store.snapshot_id().unwrap();
    assert_ne!(after_claim, empty);

    // status: all fresh, nothing listed by default.
    let q = StatusQuery::default();
    let out = engine::status(&store, &mut p, &no_git(), &q, None, now()).unwrap();
    assert!(out.claims.is_empty());
    assert_eq!(out.snapshot, after_claim);
    assert_eq!((out.summary.claims, out.summary.fresh), (2, 2));

    // The code changes: the claim that cites it goes stale.
    edit(
        &w,
        "file:src/retry.rs",
        "function:retry",
        "fn retry() { cap(7) }",
    );
    let out = engine::status(&store, &mut p, &no_git(), &q, None, now()).unwrap();
    assert_eq!(out.claims.len(), 1);
    assert_eq!(out.claims[0].id, doc);
    assert_eq!(out.claims[0].state, ClaimState::Stale);
    assert_eq!(out.claims[0].next, Some(format!("hibi mark {doc}")));

    // mark clears it.
    engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new(doc.as_str())],
        false,
        now(),
    )
    .unwrap();
    let out = engine::status(&store, &mut p, &no_git(), &q, None, now()).unwrap();
    assert_eq!((out.summary.fresh, out.summary.stale), (2, 0));

    // reanchor: a new statement, one append with claim, mark, retirement.
    let re = engine::reanchor(
        &store,
        &mut p,
        &no_git(),
        &ReanchorInput {
            claim: doc.to_string(),
            statement: Some("Retries are capped at 7.".into()),
            ..ReanchorInput::default()
        },
        now(),
    )
    .unwrap();
    // retire the other claim, replaced by the new one.
    engine::retire(
        &store,
        &RetireInput {
            claims: vec![cap.to_string()],
            replaced_by: Some(re.new.to_string()),
            ..RetireInput::default()
        },
        now(),
    )
    .unwrap();

    // A second handle reads what was written.
    let reopened = GitTextStore::open(dir.path()).unwrap();
    let ledger = reopened.load().unwrap();
    assert_eq!(
        (
            ledger.claims.len(),
            ledger.marks.len(),
            ledger.retirements.len()
        ),
        (3, 4, 2)
    );
    assert!(ledger.warnings.is_empty());
    let out = engine::status(
        &reopened,
        &mut p,
        &no_git(),
        &StatusQuery {
            states: ClaimState::ALL.to_vec(),
            ..Default::default()
        },
        None,
        now(),
    )
    .unwrap();
    let states: Vec<(String, ClaimState)> = out
        .claims
        .iter()
        .map(|c| (c.statement.clone(), c.state))
        .collect();
    assert!(states.contains(&("Retries are capped at 7.".into(), ClaimState::Fresh)));
    assert!(states.contains(&("Retries are capped at 5.".into(), ClaimState::Retired)));
    assert!(states.contains(&("cap takes a count.".into(), ClaimState::Retired)));
    match engine::list(&reopened, &ListQuery::default()).unwrap() {
        ListOutput::Claims { claims, .. } => {
            assert_eq!(claims.len(), 1);
            assert_eq!(claims[0].id, re.new);
            assert_eq!(claims[0].marks, 1);
        }
        other => panic!("{other:?}"),
    }
}
