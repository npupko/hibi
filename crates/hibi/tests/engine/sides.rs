//! Every row of the two tables of spec section 9.2, and the validation
//! rules of section 2.5 (R17, R28, R50, R51, R62).

use hibi::engine::{self, ClaimState, Reason, SideState};
use hibi::model::{ClaimId, Mark, MarkSide, Selector, SelectorKey};
use hibi::provider::{ProviderErrorCode as C, Providers};
use hibi::store::Batch;

use crate::support::*;

/// A store with one claim "S" on `t:a` key `k1`, marked at NOW.
fn setup() -> (Shared, Providers, MemStore, ClaimId) {
    let w = key_world();
    set(
        &w,
        "t:a",
        vec![unit("k1", "note", "one"), unit("k2", "note", "two")],
    );
    let mut p = providers(&w, &["t"]);
    let store = MemStore::new();
    let input = hibi::engine::ClaimInput {
        subject: json_anchor("t:a", key_sel("k1")),
        ..claim_input("S", "t:a", &[])
    };
    let id = claim_at(&store, &mut p, input, NOW);
    (w, p, store, id)
}

fn side(store: &MemStore, p: &mut Providers, id: &ClaimId) -> (SideState, Option<Reason>) {
    let v = view(store, p, id);
    assert_eq!(v.sides.len(), 1);
    (v.sides[0].state, v.sides[0].reason)
}

#[test]
fn one_unit_with_the_marked_fingerprint_is_same() {
    let (_w, mut p, store, id) = setup();
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Fresh);
    let s = &v.sides[0];
    assert_eq!((s.state, s.reason), (SideState::Same, None));
    assert_eq!(s.fp, s.marked_fp);
    assert_eq!(s.unit.as_ref().unwrap().key, "k1");
    assert!(s.candidates.is_none() && s.text.is_none() && s.detail.is_none());
}

#[test]
fn scheme_not_declared() {
    let (_w, _p, store, id) = setup();
    let mut empty = Providers::new();
    assert_eq!(
        side(&store, &mut empty, &id),
        (SideState::Unavailable, Some(Reason::SchemeNotDeclared))
    );
}

/// R28: a scheme listed in `hibi.toml` with no user registration reports
/// `provider_not_registered`, and nothing is called or started.
#[test]
fn r28_provider_not_registered_starts_nothing() {
    let (w, _p, store, id) = setup();
    let before = log(&w).len();
    let mut declared = Providers::new();
    declared.declare_unregistered("t");
    assert_eq!(
        side(&store, &mut declared, &id),
        (SideState::Unavailable, Some(Reason::ProviderNotRegistered))
    );
    assert_eq!(log(&w).len(), before);
    let v = view(&store, &mut declared, &id);
    assert_eq!(v.state, ClaimState::Stale);
    assert_eq!(v.next.as_deref(), Some("hibi providers"));
}

#[test]
fn every_provider_error_code_maps_by_the_table() {
    use Reason as R;
    use SideState as S;
    let rows = [
        (C::Failed, S::Unavailable, R::ProviderFailed),
        (C::NotFound, S::Missing, R::ResourceNotFound),
        (C::NotText, S::Unavailable, R::NotText),
        (C::TooLarge, S::Unavailable, R::TooLarge),
        (C::RepoNotFound, S::Unavailable, R::RepoNotFound),
        (C::RepoNotAllowed, S::Unavailable, R::RepoNotAllowed),
        (C::InvalidRequest, S::Unavailable, R::ProviderError),
        (C::UnknownMethod, S::Unavailable, R::ProviderError),
        (C::InvalidLocator, S::Unavailable, R::SelectorRefused),
        (C::InvalidSelector, S::Unavailable, R::SelectorRefused),
        (C::UnknownUnit, S::Unavailable, R::ProviderError),
        (C::Unavailable, S::Unavailable, R::ResourceUnavailable),
        // The same reason: a side's JSON does not tell the two apart.
        (C::Unparsable, S::Unavailable, R::ResourceUnavailable),
        (C::Internal, S::Unavailable, R::ProviderError),
        // Section 13.4: the provider is unusable.
        (C::UnsupportedProtocol, S::Unavailable, R::ProviderFailed),
    ];
    // Every code of the provider boundary is in the table above.
    assert_eq!(rows.len(), C::WIRE.len() + 1);
    for (code, state, reason) in rows {
        let (w, mut p, store, id) = setup();
        set_err(&w, "t:a", code);
        assert_eq!(side(&store, &mut p, &id), (state, Some(reason)), "{code}");
        assert_eq!(engine::side_of_error(code), (state, reason));
    }
}

#[test]
fn a_provider_whose_info_fails_is_provider_failed() {
    let (w, mut p, store, id) = setup();
    w.borrow_mut().info_error = Some(perr(C::Internal));
    let before = log(&w).len();
    assert_eq!(
        side(&store, &mut p, &id),
        (SideState::Unavailable, Some(Reason::ProviderFailed))
    );
    assert!(log(&w)[before..].iter().all(|l| l == "info"));
}

/// R51 at the engine: an absent resource is `resource_not_found`, an
/// existing one where the selector finds nothing is `selector_not_found`.
#[test]
fn r51_absent_resource_and_selector_that_finds_nothing() {
    let (w, mut p, store, id) = setup();
    set(&w, "t:a", vec![unit("k2", "note", "two")]);
    assert_eq!(
        side(&store, &mut p, &id),
        (SideState::Missing, Some(Reason::SelectorNotFound))
    );
    w.borrow_mut().resources.remove("t:a");
    assert_eq!(
        side(&store, &mut p, &id),
        (SideState::Missing, Some(Reason::ResourceNotFound))
    );
}

#[test]
fn several_units_are_ambiguous_with_candidates() {
    let (w, mut p, store, id) = setup();
    w.borrow_mut().overrides.insert(
        anchor("t:a", key_sel("k1")),
        Ok(vec![unit("k1", "note", "one"), unit("k2", "note", "two")]),
    );
    let v = view(&store, &mut p, &id);
    let s = &v.sides[0];
    assert_eq!(
        (s.state, s.reason),
        (SideState::Ambiguous, Some(Reason::SeveralUnits))
    );
    let keys: Vec<&str> = s
        .candidates
        .as_ref()
        .unwrap()
        .iter()
        .map(|u| u.key.as_str())
        .collect();
    assert_eq!(keys, ["k1", "k2"]);
    assert!(s.unit.is_none() && s.fp.is_none());
    assert!(s.marked_fp.is_some());
}

#[test]
fn a_different_fingerprint_of_the_same_profile_is_changed() {
    let (w, mut p, store, id) = setup();
    edit(&w, "t:a", "k1", "one, edited");
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Stale);
    let s = &v.sides[0];
    assert_eq!(
        (s.state, s.reason),
        (SideState::Changed, Some(Reason::FingerprintDiffers))
    );
    assert_ne!(s.fp, s.marked_fp);
}

/// R17: a mark with profile `ast-v1` against a current `prose-v1`
/// fingerprint is `unavailable` with `profile_mismatch`, never `changed`.
#[test]
fn r17_profile_mismatch_is_unavailable() {
    let (w, mut p, store, _) = setup();
    // Mark an `ast-v1` fingerprint, then let the unit answer `prose-v1`.
    let mut code = unit("k3", "note", "three");
    code.fp = fp("ast-v1", 'a');
    set(&w, "t:a", vec![code]);
    let input = hibi::engine::ClaimInput {
        subject: json_anchor("t:a", key_sel("k3")),
        ..claim_input("R17", "t:a", &[])
    };
    let id = claim_at(&store, &mut p, input, NOW);
    set(&w, "t:a", vec![unit("k3", "note", "three")]);
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Stale);
    assert_eq!(
        (v.sides[0].state, v.sides[0].reason),
        (SideState::Unavailable, Some(Reason::ProfileMismatch))
    );
    assert_eq!(v.sides[0].marked_fp.as_ref().unwrap().profile(), "ast-v1");
    assert_eq!(v.sides[0].fp.as_ref().unwrap().profile(), "prose-v1");
    assert_eq!(v.next, Some(format!("hibi mark {id}")));
}

#[test]
fn a_claim_with_no_mark_is_stale_with_no_mark() {
    let w = key_world();
    set(&w, "t:a", vec![unit("k1", "note", "one")]);
    let mut p = providers(&w, &["t"]);
    let store = MemStore::new();
    let claim =
        hibi::model::Claim::new("S", anchor("t:a", key_sel("k1")), vec![], None, None).unwrap();
    store.insert(Batch {
        claims: vec![claim.clone()],
        ..Batch::default()
    });
    let v = view(&store, &mut p, &claim.id);
    assert_eq!(v.state, ClaimState::Stale);
    assert_eq!(
        (v.sides[0].state, v.sides[0].reason),
        (SideState::Changed, Some(Reason::NoMark))
    );
    assert!(v.mark.is_none() && v.marked_at.is_none() && v.sides[0].marked_fp.is_none());
    assert_eq!(v.next, Some(format!("hibi mark {}", claim.id)));
}

/// R50: a returned `sel` with a key the provider does not allow, an invalid
/// `sel`, or two units with one key is `provider_error` for the side. (An
/// unknown key inside `sel` and an invalid fingerprint cannot be built as a
/// `Unit`: deserialization rejects them, which the `provider` tests cover.)
#[test]
fn r50_invalid_units_are_provider_error() {
    let bad_units = [
        // A key the provider does not list.
        vec![hibi::provider::Unit {
            sel: symbol_sel("x"),
            ..unit("k1", "note", "one")
        }],
        // Not a valid selector: prefix without quote.
        vec![hibi::provider::Unit {
            sel: Selector {
                prefix: Some("p".into()),
                ..Selector::default()
            },
            ..unit("k1", "note", "one")
        }],
        // Two units with one key.
        vec![unit("k1", "note", "one"), unit("k1", "note", "other")],
    ];
    for units in bad_units {
        let (w, mut p, store, id) = setup();
        w.borrow_mut()
            .overrides
            .insert(anchor("t:a", key_sel("k1")), Ok(units));
        assert_eq!(
            side(&store, &mut p, &id),
            (SideState::Unavailable, Some(Reason::ProviderError))
        );
    }
}

/// R62: a stored selector with a key the provider no longer allows is not
/// sent; the side is `unavailable` with `selector_refused`, and the claim
/// asks for a new anchor.
#[test]
fn r62_a_disallowed_stored_key_is_never_sent() {
    let (w, mut p, store, id) = setup();
    w.borrow_mut().selectors = vec![SelectorKey::Quote];
    let before = log(&w).len();
    assert_eq!(
        side(&store, &mut p, &id),
        (SideState::Unavailable, Some(Reason::SelectorRefused))
    );
    let v = view(&store, &mut p, &id);
    assert_eq!(
        v.next,
        Some(format!("hibi reanchor {id} --subject <anchor>"))
    );
    assert_eq!(
        v.sides[0].detail.as_deref(),
        Some("the provider does not allow the selector key key")
    );
    let after = log(&w);
    assert!(
        after[before..].iter().all(|l| !l.starts_with("resolve")),
        "{after:?}"
    );
}

#[test]
fn an_answer_list_of_the_wrong_length_is_provider_error() {
    let (w, mut p, store, id) = setup();
    w.borrow_mut().short_answers = true;
    assert_eq!(
        side(&store, &mut p, &id),
        (SideState::Unavailable, Some(Reason::ProviderError))
    );
}

/// Section 9.1: every side is resolved once per run, in one `resolve` per
/// scheme; identical anchors are sent once.
#[test]
fn sides_are_batched_per_provider_and_sent_once() {
    let (w, mut p, store, id) = setup();
    let other = hibi::engine::ClaimInput {
        subject: json_anchor("t:a", key_sel("k2")),
        evidence: vec![json_anchor("t:a", key_sel("k1"))],
        ..claim_input("T", "t:a", &[])
    };
    claim_at(&store, &mut p, other, NOW);
    w.borrow_mut().log.clear();
    let out = engine::status(
        &store,
        &mut p,
        &no_git(),
        &engine::StatusQuery {
            states: vec![ClaimState::Fresh],
            ..Default::default()
        },
        None,
        now(),
    )
    .unwrap();
    assert_eq!(out.claims.len(), 2);
    assert!(out.claims.iter().any(|c| c.id == id));
    let resolves: Vec<String> = log(&w)
        .into_iter()
        .filter(|l| l.starts_with("resolve"))
        .collect();
    // Three sides, two distinct anchors.
    assert_eq!(resolves.len(), 2, "{resolves:?}");
}

/// A mark whose side count does not fit its claim is ignored.
#[test]
fn a_mark_that_does_not_fit_its_claim_is_ignored() {
    let (_w, mut p, store, id) = setup();
    let two = vec![
        MarkSide {
            loc: loc("t:a"),
            fp: fp("prose-v1", '1'),
        },
        MarkSide {
            loc: loc("t:a"),
            fp: fp("prose-v1", '2'),
        },
    ];
    let odd = Mark::new(id.clone(), ts("2027-01-01T00:00:00Z"), None, two, None).unwrap();
    store.insert(Batch {
        marks: vec![odd.clone()],
        ..Batch::default()
    });
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Fresh);
    assert_ne!(v.mark, Some(odd.id));
}
