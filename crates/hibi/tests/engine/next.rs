//! The `next` table of spec section 15.2, row by row (R35, R37, R55).

use hibi::engine::{self, ClaimState, MarkInput};
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
    // provider_not_registered: `hibi providers` says what is wrong.
    let f = fixture(None);
    let mut p = Providers::new();
    p.register(&["t"], Box::new(Mem::new(&f.worlds[0])));
    p.register(&["u"], Box::new(Mem::new(&f.worlds[1])));
    p.declare_unregistered("v");
    assert_eq!(
        view(&f.store, &mut p, &f.id).next,
        Some("hibi providers".to_owned())
    );
}

/// A stored claim on a scheme that the configuration no longer declares:
/// no command fits, and the side says what to do.
#[test]
fn a_scheme_that_is_no_longer_declared_has_no_next() {
    let f = fixture(None);
    let mut p = Providers::new();
    p.register(&["t"], Box::new(Mem::new(&f.worlds[0])));
    p.register(&["u"], Box::new(Mem::new(&f.worlds[1])));
    let v = view(&f.store, &mut p, &f.id);
    assert_eq!((v.state, v.next), (ClaimState::Stale, None));
    let side = &v.sides[2];
    assert_eq!(side.reason, Some(engine::Reason::SchemeNotDeclared));
    assert_eq!(
        side.remediation.as_deref(),
        Some("Declare the scheme under [schemes] in .hibi/hibi.toml, or retire the claim.")
    );
    assert!(side.detail.as_deref().unwrap().contains('v'), "{side:?}");
}

/// An anchor the provider refuses (`invalid_selector`, `invalid_locator`)
/// is the anchor being wrong for the resource, not the provider failing. A
/// stored side is `unavailable` with reason `selector_refused`, says what
/// the provider said, and asks for another anchor; a new anchor is
/// `invalid_anchor` with the provider's message.
#[test]
fn an_anchor_the_provider_refuses_is_not_a_provider_fault() {
    for code in [C::InvalidSelector, C::InvalidLocator] {
        let mut f = fixture(None);
        set_err_message(&f.worlds[1], "u:b", code, "u:b has no symbols; use a quote");
        let v = view(&f.store, &mut f.p, &f.id);
        let next = format!("hibi reanchor {} --evidence 1=<anchor>", f.id);
        assert_eq!(
            (v.state, v.next.clone()),
            (ClaimState::Stale, Some(next.clone()))
        );
        let side = &v.sides[1];
        assert_eq!(
            (side.state, side.reason),
            (
                engine::SideState::Unavailable,
                Some(engine::Reason::SelectorRefused)
            ),
            "{code}"
        );
        assert_eq!(
            side.detail.as_deref(),
            Some("u:b has no symbols; use a quote")
        );
        assert_eq!(
            side.remediation.as_deref(),
            Some(
                "The stored anchor is no longer accepted for this resource. Give the side another anchor with hibi reanchor, or retire the claim."
            )
        );

        let before = f.store.counts();
        let errors = engine::mark(
            &f.store,
            &mut f.p,
            &no_git(),
            &[MarkInput::new(f.id.as_str())],
            false,
            now(),
        )
        .unwrap_err();
        assert_eq!(errors[0].code, hibi::Code::SideUnavailable);
        assert!(
            errors[0].message.contains("selector_refused"),
            "{}",
            errors[0]
        );
        assert_eq!(errors[0].remediation, Some(next.clone()));
        // A `reanchor` that keeps the side names the same command.
        let e = engine::reanchor(
            &f.store,
            &mut f.p,
            &no_git(),
            &engine::ReanchorInput {
                claim: f.id.to_string(),
                statement: Some("S2".into()),
                ..engine::ReanchorInput::default()
            },
            now(),
        )
        .unwrap_err();
        assert_eq!(e.code, hibi::Code::SideUnavailable);
        assert_eq!(e.remediation, Some(next));

        // A new anchor: the caller's mistake, in the provider's words.
        let errors = engine::claim(
            &f.store,
            &mut f.p,
            &no_git(),
            &[engine::ClaimInput {
                subject: json_anchor("u:b", key_sel("k")),
                ..claim_input("Another", "u:b", &[])
            }],
            false,
            now(),
        )
        .unwrap_err();
        assert_eq!(errors[0].code, hibi::Code::InvalidAnchor, "{code}");
        assert_eq!(
            errors[0].message,
            "the subject (u:b): u:b has no symbols; use a quote"
        );
        assert!(
            errors[0]
                .remediation
                .as_deref()
                .unwrap()
                .starts_with("Change the anchor"),
            "{}",
            errors[0]
        );
        assert_eq!(f.store.counts(), before);
    }
}

/// What a provider said when it failed is on the side, cut to a bounded
/// length; a resource that cannot be read now has its own reason, no
/// `next`, and a remediation that quotes the provider.
#[test]
fn a_side_that_cannot_be_evaluated_says_what_the_provider_said() {
    let mut f = fixture(None);
    let long = format!("the helper exited with status 3: {}", "x".repeat(2000));
    set_err_message(&f.worlds[1], "u:b", C::Failed, &long);
    let v = view(&f.store, &mut f.p, &f.id);
    let side = &v.sides[1];
    assert_eq!(side.reason, Some(engine::Reason::ProviderFailed));
    let said = side.detail.as_deref().unwrap();
    assert!(said.contains("the helper exited with status 3"), "{said}");
    assert!(
        said.chars().count() <= engine::DETAIL_CHARS + 1,
        "{}",
        said.len()
    );
    assert!(v.sides[0].detail.is_none());

    let mut f = fixture(None);
    set_err_message(
        &f.worlds[1],
        "u:b",
        C::Unavailable,
        "src/a.rs does not parse as Rust: line 3.",
    );
    let v = view(&f.store, &mut f.p, &f.id);
    assert_eq!((v.state, v.next.clone()), (ClaimState::Stale, None));
    let side = &v.sides[1];
    assert_eq!(
        (side.state, side.reason),
        (
            engine::SideState::Unavailable,
            Some(engine::Reason::ResourceUnavailable)
        )
    );
    assert_eq!(
        side.remediation.as_deref(),
        Some("src/a.rs does not parse as Rust: line 3. Run the command again once that is fixed.")
    );
    // `mark` fails with the same remediation and writes nothing.
    let before = f.store.counts();
    let errors = engine::mark(
        &f.store,
        &mut f.p,
        &no_git(),
        &[MarkInput::new(f.id.as_str())],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(errors[0].code, hibi::Code::SideUnavailable);
    assert!(
        errors[0].message.contains("resource_unavailable"),
        "{}",
        errors[0]
    );
    assert_eq!(errors[0].remediation, side.remediation);
    assert_eq!(f.store.counts(), before);
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

/// Similarity proposes and never resolves: a unit the provider returns
/// with `approx: true` is a candidate under a `missing` side, whatever its
/// content, and the only way forward is an anchor the caller chooses.
#[test]
fn a_unit_found_by_similarity_never_resolves_a_side() {
    let mut f = fixture(None);
    // The marked unit under another key: the same content, moved or
    // renamed. The provider can only guess that it is the one.
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(vec![approx(unit("moved", "note", "first"))]),
    );
    let v = view(&f.store, &mut f.p, &f.id);
    assert_eq!(v.state, ClaimState::Stale);
    assert_eq!(
        v.next,
        Some(format!("hibi reanchor {} --evidence 1=<anchor>", f.id))
    );
    let side = &v.sides[1];
    assert_eq!(
        (side.state, side.reason),
        (engine::SideState::Missing, Some(engine::Reason::NotExact))
    );
    assert!(side.unit.is_none() && side.fp.is_none());
    let candidates = side.candidates.as_ref().unwrap();
    assert_eq!(side.candidates_total, Some(1));
    assert_eq!(candidates.len(), 1);
    let c = &candidates[0];
    assert_eq!((c.key.as_str(), c.kind.as_str()), ("moved", "note"));
    assert_eq!(c.text.as_deref(), Some("first"));
    assert_eq!(c.anchor, r#"{"loc":"u:b","sel":{"key":"moved"}}"#);
    assert!(c.same_fingerprint, "the content is the marked content");
    // The other sides are untouched.
    assert_eq!(v.sides[0].state, engine::SideState::Same);
    assert!(v.sides[0].candidates.is_none());
    let json = serde_json::to_value(side).unwrap();
    assert!(json.get("approx").is_none());
    assert_eq!(json["candidates"][0]["sameFingerprint"], true);
    assert_eq!(json["candidatesTotal"], 1);

    // Default status lists the claim as stale; no summary key counts
    // drifted claims.
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
    assert_eq!((out.summary.fresh, out.summary.stale), (0, 1));
    assert!(
        serde_json::to_value(out.summary)
            .unwrap()
            .get("drifted")
            .is_none()
    );
    assert!(!out.next_note.contains("drifted"));

    // `mark` does not take the guess: the claim stays as it is.
    let before = f.store.counts();
    let errors = engine::mark(
        &f.store,
        &mut f.p,
        &no_git(),
        &[MarkInput::new(f.id.as_str())],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(errors[0].code, hibi::Code::SideMissing);
    assert!(errors[0].message.contains("not_exact"), "{}", errors[0]);
    assert_eq!(errors[0].remediation, v.next);
    assert_eq!(f.store.counts(), before);

    // `reanchor` that keeps the side fails the same way.
    let e = engine::reanchor(
        &f.store,
        &mut f.p,
        &no_git(),
        &engine::ReanchorInput {
            claim: f.id.to_string(),
            statement: Some("S2".into()),
            ..engine::ReanchorInput::default()
        },
        now(),
    )
    .unwrap_err();
    assert_eq!(e.code, hibi::Code::SideMissing);
    assert_eq!(f.store.counts(), before);

    // A different content under the guess: not the same fingerprint.
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(vec![
            approx(unit("renamed", "note", "first, reworded")),
            approx(unit("other", "note", "something else")),
        ]),
    );
    let v = view(&f.store, &mut f.p, &f.id);
    let side = &v.sides[1];
    assert_eq!(side.reason, Some(engine::Reason::NotExact));
    let cs = side.candidates.as_ref().unwrap();
    assert_eq!(cs.len(), 2);
    assert!(cs.iter().all(|c| !c.same_fingerprint));

    // The caller chooses: `reanchor` with the candidate's anchor works,
    // and the new claim resolves exactly.
    set(
        &f.worlds[1],
        "u:b",
        vec![unit("renamed", "note", "first, reworded")],
    );
    let chosen: hibi::model::Anchor =
        serde_json::from_str(r#"{"loc":"u:b","sel":{"key":"renamed"}}"#).unwrap();
    let out = engine::reanchor(
        &f.store,
        &mut f.p,
        &no_git(),
        &engine::ReanchorInput {
            claim: f.id.to_string(),
            evidence: vec![(1, chosen.into())],
            ..engine::ReanchorInput::default()
        },
        now(),
    )
    .unwrap();
    assert_eq!(out.claim.state, ClaimState::Fresh);
    assert_eq!(out.claim.next, None);
    assert_eq!(out.accepted.was, ClaimState::Stale);
    assert_eq!(out.accepted.sides[0].reason, Some(engine::Reason::NotExact));
}

/// A stored side that matches very many units shows twenty candidates
/// and the count of all of them.
#[test]
fn a_side_shows_at_most_twenty_candidates_and_the_total() {
    let mut f = fixture(None);
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok((0..25)
            .map(|n| unit(&format!("k{n}"), "note", "first"))
            .collect()),
    );
    let v = view(&f.store, &mut f.p, &f.id);
    let side = &v.sides[1];
    assert_eq!(
        (side.state, side.reason),
        (
            engine::SideState::Ambiguous,
            Some(engine::Reason::SeveralUnits)
        )
    );
    let shown = side.candidates.as_ref().unwrap();
    assert_eq!(shown.len(), engine::MAX_CANDIDATES);
    assert_eq!(side.candidates_total, Some(25));
    assert_eq!(shown[19].key, "k19");
    let errors = engine::mark(
        &f.store,
        &mut f.p,
        &no_git(),
        &[MarkInput::new(f.id.as_str())],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(errors[0].code, hibi::Code::SideAmbiguous);
    assert!(errors[0].message.ends_with("; and 5 more"), "{}", errors[0]);
}

/// An expired claim whose side is found only by similarity is stale: no
/// mark matches, because the side has no unit.
#[test]
fn a_guess_does_not_keep_a_claim_expired_or_fresh() {
    let mut f = fixture(Some("P1D"));
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(vec![approx(unit("k", "note", "first"))]),
    );
    assert_eq!(
        f.next(),
        (
            ClaimState::Stale,
            Some(format!("hibi reanchor {} --evidence 1=<anchor>", f.id))
        )
    );
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
