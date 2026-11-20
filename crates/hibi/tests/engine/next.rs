//! The `next` table of spec section 15.2, row by row (R35, R37, R55).

use hibi::engine::{self, ClaimState, MarkInput, RetireInput};
use hibi::model::ClaimId;
use hibi::provider::{ProviderErrorCode as C, Providers, Unit};

use crate::support::*;

/// One claim with a subject on `t:a` and two evidence sides on `u:b` and
/// `v:c`, each scheme with its own world so that each side can fail alone.
struct Fixture {
    worlds: [Shared; 3],
    p: Providers,
    store: MemStore,
    id: ClaimId,
}

fn fixture(recheck: Option<&str>) -> Fixture {
    let worlds = [key_world(), key_world(), key_world()];
    set(&worlds[0], "t:a", vec![unit("k", "note", "subject")]);
    set(&worlds[1], "u:b", vec![unit("k", "note", "first")]);
    set(&worlds[2], "v:c", vec![unit("k", "note", "second")]);
    let mut p = Providers::new();
    p.register(&["t"], Box::new(Mem::new(&worlds[0])));
    p.register(&["u"], Box::new(Mem::new(&worlds[1])));
    p.register(&["v"], Box::new(Mem::new(&worlds[2])));
    let store = MemStore::new();
    let input = engine::ClaimInput {
        subject: json_anchor("t:a", key_sel("k")),
        evidence: vec![
            json_anchor("u:b", key_sel("k")),
            json_anchor("v:c", key_sel("k")),
        ],
        recheck: recheck.map(str::to_owned),
        ..claim_input("S", "t:a", &[])
    };
    let id = claim_at(&store, &mut p, input, "2026-11-01T00:00:00Z");
    Fixture {
        worlds,
        p,
        store,
        id,
    }
}

impl Fixture {
    fn next(&mut self) -> (ClaimState, Option<String>) {
        let v = view(&self.store, &mut self.p, &self.id);
        (v.state, v.next)
    }
}

fn approx(mut u: Unit) -> Unit {
    u.approx = true;
    u
}

#[test]
fn fresh_without_approx_has_no_next() {
    let mut f = fixture(None);
    assert_eq!(f.next(), (ClaimState::Fresh, None));
}

#[test]
fn missing_and_ambiguous_sides_give_reanchor_with_one_option_per_side() {
    let mut f = fixture(None);
    f.worlds[0].borrow_mut().resources.remove("t:a");
    assert_eq!(
        f.next(),
        (
            ClaimState::Stale,
            Some(format!("hibi reanchor {} --subject <anchor>", f.id))
        )
    );
    f.worlds[2].borrow_mut().overrides.insert(
        anchor("v:c", key_sel("k")),
        Ok(vec![unit("k", "note", "second"), unit("k2", "note", "x")]),
    );
    let both = format!(
        "hibi reanchor {} --subject <anchor> --evidence 2=<anchor>",
        f.id
    );
    assert_eq!(f.next().1, Some(both.clone()));
    // The reanchor row comes before the unavailable rows.
    set_err(&f.worlds[1], "u:b", C::Failed);
    assert_eq!(f.next().1, Some(both));
}

#[test]
fn unavailable_rows() {
    for code in [C::Failed, C::RepoNotAllowed] {
        let mut f = fixture(None);
        set_err(&f.worlds[1], "u:b", code);
        assert_eq!(
            f.next(),
            (ClaimState::Stale, Some("hibi providers".to_owned())),
            "{code}"
        );
    }
    for code in [C::RepoNotFound, C::NotText, C::TooLarge, C::Internal] {
        let mut f = fixture(None);
        set_err(&f.worlds[1], "u:b", code);
        assert_eq!(f.next(), (ClaimState::Stale, None), "{code}");
        // A changed side elsewhere does not turn `null` into `hibi mark`.
        edit(&f.worlds[0], "t:a", "k", "edited");
        assert_eq!(f.next().1, None);
        // The `hibi providers` row comes before the `null` row.
        set_err(&f.worlds[2], "v:c", C::Failed);
        assert_eq!(f.next().1, Some("hibi providers".to_owned()));
    }
    // scheme_not_declared, then provider_not_registered.
    let f = fixture(None);
    let mut p = Providers::new();
    p.register(&["t"], Box::new(Mem::new(&f.worlds[0])));
    p.register(&["u"], Box::new(Mem::new(&f.worlds[1])));
    assert_eq!(
        view(&f.store, &mut p, &f.id).next,
        Some("hibi providers".to_owned())
    );
    p.declare_unregistered("v");
    assert_eq!(
        view(&f.store, &mut p, &f.id).next,
        Some("hibi providers".to_owned())
    );
}

#[test]
fn changed_expired_and_profile_mismatch_give_mark() {
    let mut f = fixture(None);
    edit(&f.worlds[1], "u:b", "k", "edited");
    assert_eq!(
        f.next(),
        (ClaimState::Stale, Some(format!("hibi mark {}", f.id)))
    );

    let mut f = fixture(Some("P1D"));
    assert_eq!(
        f.next(),
        (ClaimState::Expired, Some(format!("hibi mark {}", f.id)))
    );

    let mut f = fixture(None);
    let mut other = unit("k", "note", "first");
    other.fp = fp("ast-v1", '0');
    set(&f.worlds[1], "u:b", vec![other]);
    assert_eq!(
        f.next(),
        (ClaimState::Stale, Some(format!("hibi mark {}", f.id)))
    );
}

/// R55: a fresh claim with an approximate side gets `hibi reanchor <id>`;
/// a stale or expired one gets `hibi mark <id>`, and the output of that
/// mark then suggests the reanchor. (R37 names `reanchor` for the stale
/// case; section 15.2 and R55 are followed.)
#[test]
fn r55_approx_side_rows() {
    let mut f = fixture(None);
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(vec![approx(unit("k", "note", "first"))]),
    );
    let v = view(&f.store, &mut f.p, &f.id);
    assert_eq!(v.state, ClaimState::Fresh);
    assert!(v.sides[1].approx && !v.sides[0].approx);
    // A drifted fresh claim is in the default selection and is counted.
    let out = engine::status(
        &f.store,
        &mut f.p,
        &no_git(),
        &engine::StatusQuery::default(),
        None,
        now(),
    )
    .unwrap();
    assert_eq!(
        out.claims
            .iter()
            .map(|c| (c.id.clone(), c.state))
            .collect::<Vec<_>>(),
        [(v.id.clone(), engine::ClaimState::Fresh)]
    );
    assert_eq!(out.claims[0].next, v.next);
    assert_eq!((out.summary.fresh, out.summary.drifted), (1, 1));
    assert!(out.next_note.contains("drifted"));
    assert_eq!(v.next, Some(format!("hibi reanchor {}", f.id)));

    // Stale with an approximate, changed side (R37's rename): `hibi mark`.
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(vec![approx(unit("renamed", "note", "first, renamed"))]),
    );
    let v = view(&f.store, &mut f.p, &f.id);
    assert_eq!(v.state, ClaimState::Stale);
    assert_eq!(v.sides[1].state, engine::SideState::Changed);
    assert!(v.sides[1].approx);
    assert_eq!(v.next, Some(format!("hibi mark {}", f.id)));

    // The mark succeeds, keeps the id, shows the unit and text, and its
    // view suggests the reanchor.
    let long = "é".repeat(300);
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(vec![approx(unit("renamed", "note", &long))]),
    );
    let out = engine::mark(
        &f.store,
        &mut f.p,
        &no_git(),
        &[MarkInput::new(f.id.as_str())],
        false,
        now(),
    )
    .unwrap();
    let mv = &out.marks[0].claim_view;
    assert_eq!(out.marks[0].claim, f.id);
    assert_eq!(mv.id, f.id);
    assert_eq!(mv.state, ClaimState::Fresh);
    assert_eq!(mv.next, Some(format!("hibi reanchor {}", f.id)));
    let side = &mv.sides[1];
    assert!(side.approx);
    assert_eq!(side.state, engine::SideState::Same);
    assert_eq!(side.unit.as_ref().unwrap().key, "renamed");
    assert_eq!(side.text.as_deref(), Some("é".repeat(240).as_str()));
    // An exact side carries no text.
    assert!(mv.sides[0].text.is_none());

    // Expired with an approximate side: `hibi mark`.
    let mut f = fixture(Some("P1D"));
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(vec![approx(unit("k", "note", "first"))]),
    );
    assert_eq!(
        f.next(),
        (ClaimState::Expired, Some(format!("hibi mark {}", f.id)))
    );
}

/// The fresh-and-approx row when the claim rebuilt from the completed
/// selectors is a retired claim: `--note <text>`.
#[test]
fn approx_reanchor_that_would_land_on_a_retired_claim() {
    let w = key_world();
    set(&w, "t:a", vec![unit("old", "note", "text")]);
    let mut p = providers(&w, &["t"]);
    let store = MemStore::new();
    let a = claim_at(
        &store,
        &mut p,
        engine::ClaimInput {
            subject: json_anchor("t:a", key_sel("old")),
            ..claim_input("S", "t:a", &[])
        },
        NOW,
    );
    // The unit is renamed; the same statement was claimed on the new key
    // and retired.
    set(&w, "t:a", vec![unit("new", "note", "text")]);
    let b = claim_at(
        &store,
        &mut p,
        engine::ClaimInput {
            subject: json_anchor("t:a", key_sel("new")),
            ..claim_input("S", "t:a", &[])
        },
        NOW,
    );
    engine::retire(
        &store,
        &RetireInput {
            claims: vec![b.to_string()],
            ..RetireInput::default()
        },
        now(),
    )
    .unwrap();
    w.borrow_mut().overrides.insert(
        anchor("t:a", key_sel("old")),
        Ok(vec![approx(unit("new", "note", "text"))]),
    );
    let v = view(&store, &mut p, &a);
    assert_eq!(v.state, ClaimState::Fresh);
    assert_eq!(v.next, Some(format!("hibi reanchor {a} --note <text>")));
}

/// R35: every stale or expired claim in `status` output has a `next` that
/// follows the table.
#[test]
fn r35_every_listed_claim_follows_the_table() {
    let mut f = fixture(Some("P1D"));
    let out = engine::status(
        &f.store,
        &mut f.p,
        &no_git(),
        &engine::StatusQuery::default(),
        None,
        now(),
    )
    .unwrap();
    assert_eq!(out.claims.len(), 1);
    for c in &out.claims {
        assert_eq!(c.next, Some(format!("hibi mark {}", c.id)));
    }
}
