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
    // The provider's sentence as it is: no error code in front of it.
    assert!(
        said.starts_with("the helper exited with status 3"),
        "{said}"
    );
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
    // renamed. The provider can only guess that it is the one. The unit is
    // also found by its content, and it is listed once.
    set(&f.worlds[1], "u:b", vec![unit("moved", "note", "first")]);
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
    assert_eq!(
        c.anchor.as_deref(),
        Some(r#"{"loc":"u:b","sel":{"key":"moved"}}"#)
    );
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
    // The cause is said, in the side and in the error. A candidate may
    // hold the stored text exactly, so the sentence does not claim that
    // nothing matches.
    assert_eq!(side.detail.as_deref(), Some(engine::NOT_IDENTIFIED));
    assert!(
        errors[0]
            .message
            .contains("(not_exact): the stored anchor no longer identifies one unit; 1 candidate,"),
        "{}",
        errors[0]
    );
    assert!(!errors[0].message.contains("quoted text was found"));
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
    set(
        &f.worlds[1],
        "u:b",
        vec![
            unit("renamed", "note", "first, reworded"),
            unit("other", "note", "something else"),
        ],
    );
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

/// Candidates by content: for a side whose stored selector finds nothing
/// exactly, the units of the resource that have the fingerprint of the
/// reference mark are listed first, then what the provider found by
/// similarity. They are candidates only.
#[test]
fn units_with_the_marked_content_are_the_first_candidates() {
    let mut f = fixture(None);
    // The marked unit has another key now; the provider finds nothing.
    set(
        &f.worlds[1],
        "u:b",
        vec![
            unit("other", "note", "something else"),
            unit("renamed", "note", "first"),
        ],
    );
    let v = view(&f.store, &mut f.p, &f.id);
    assert_eq!(v.state, ClaimState::Stale);
    let side = &v.sides[1];
    assert_eq!(
        (side.state, side.reason),
        (
            engine::SideState::Missing,
            Some(engine::Reason::SelectorNotFound)
        )
    );
    let cs = side.candidates.as_ref().unwrap();
    assert_eq!(side.candidates_total, Some(1));
    assert_eq!(cs[0].key, "renamed");
    assert!(cs[0].same_fingerprint);
    assert_eq!(cs[0].text.as_deref(), Some("first"));
    assert_eq!(
        cs[0].anchor.as_deref(),
        Some(r#"{"loc":"u:b","sel":{"key":"renamed"}}"#)
    );
    assert_eq!(
        v.next,
        Some(format!("hibi reanchor {} --evidence 1=<anchor>", f.id))
    );
    // A side that resolves has none.
    assert!(v.sides[0].candidates.is_none());
    // `mark` and a `reanchor` that keeps the side name the same candidate.
    let renamed = r#"note "first": {"loc":"u:b","sel":{"key":"renamed"}}"#;
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
    assert!(
        errors[0].message.ends_with(&format!(
            "the selector matches no unit; 1 candidate, each a guess to check before use: {renamed}"
        )),
        "{}",
        errors[0]
    );
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
    assert!(e.message.ends_with(renamed), "{e}");

    // With a guess of the provider as well: the unit with the marked
    // content first, then the guess.
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(vec![approx(unit("other", "note", "something else"))]),
    );
    let v = view(&f.store, &mut f.p, &f.id);
    let side = &v.sides[1];
    assert_eq!(side.reason, Some(engine::Reason::NotExact));
    let keys: Vec<(&str, bool)> = side
        .candidates
        .as_ref()
        .unwrap()
        .iter()
        .map(|c| (c.key.as_str(), c.same_fingerprint))
        .collect();
    assert_eq!(keys, [("renamed", true), ("other", false)]);
    assert_eq!(side.candidates_total, Some(2));

    // Still a candidate: `mark` is refused, with the list in that order.
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
    let message = &errors[0].message;
    assert!(message.contains("2 candidates, each a guess"), "{message}");
    let (first, second) = (
        message.find(r#""key":"renamed""#).unwrap(),
        message.find(r#""key":"other""#).unwrap(),
    );
    assert!(first < second, "{message}");
}

/// A unit resolved by its symbol alone: the stored quote is gone from the
/// resource (`identity: "symbol"`, section 6.7).
fn by_symbol_only(mut u: Unit) -> Unit {
    u.identity = Some(hibi::provider::Identity::Symbol);
    u
}

/// A claim whose subject is stored as `#limit::return 64` on `t:a`. The
/// world answers that stored anchor with whatever `overrides` holds.
fn named_and_quoted() -> (Shared, Providers, MemStore, ClaimId, hibi::model::Anchor) {
    let w = file_world();
    let sel = hibi::model::Selector {
        quote: Some("return 64".into()),
        ..symbol_sel("limit")
    };
    let mut unit = sym_unit("limit", "function", "fn limit() { return 64 }");
    unit.sel = sel.clone();
    set(&w, "t:a", vec![unit.clone()]);
    let stored = anchor("t:a", sel.clone());
    w.borrow_mut()
        .overrides
        .insert(stored.clone(), Ok(vec![unit]));
    let mut p = providers(&w, &["t"]);
    let store = MemStore::new();
    let id = claim_at(
        &store,
        &mut p,
        engine::ClaimInput {
            subject: json_anchor("t:a", sel),
            ..claim_input("S", "t:a", &[])
        },
        NOW,
    );
    (w, p, store, id, stored)
}

/// Section 9.2: a side whose unit is resolved by its symbol alone, the
/// stored quote gone, is `changed`, says so in `detail`, and needs a new
/// anchor: `next` is the reanchor template, and `mark` and a `reanchor`
/// that keeps the side are refused, because a mark would keep the stale
/// quote for good. No fingerprint stands in for the quote.
#[test]
fn a_symbol_keeps_its_unit_when_the_stored_quote_is_gone() {
    let (w, mut p, store, id, stored) = named_and_quoted();
    w.borrow_mut().overrides.insert(
        stored,
        Ok(vec![by_symbol_only(sym_unit(
            "limit",
            "function",
            "fn limit() { return 128 }",
        ))]),
    );
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Stale);
    let side = &v.sides[0];
    assert_eq!(
        (side.state, side.reason),
        (
            engine::SideState::Changed,
            Some(engine::Reason::FingerprintDiffers)
        )
    );
    assert_eq!(side.detail.as_deref(), Some(engine::QUOTE_GONE));
    assert_eq!(side.unit.as_ref().unwrap().key, "limit");
    assert!(side.candidates.is_none());
    let next = format!("hibi reanchor {id} --subject <anchor>");
    assert_eq!(v.next, Some(next.clone()));

    let before = store.counts();
    let errors = engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new(id.as_str())],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(errors[0].code, hibi::Code::SelectorNotExact);
    assert!(
        errors[0].message.contains(engine::QUOTE_GONE),
        "{}",
        errors[0]
    );
    assert_eq!(errors[0].remediation, Some(next.clone()));
    let e = engine::reanchor(
        &store,
        &mut p,
        &no_git(),
        &engine::ReanchorInput {
            claim: id.to_string(),
            statement: Some("S2".into()),
            ..engine::ReanchorInput::default()
        },
        now(),
    )
    .unwrap_err();
    assert_eq!(e.code, hibi::Code::SelectorNotExact);
    assert_eq!(e.remediation, Some(next));
    assert_eq!(store.counts(), before);

    // With a new anchor the claim is fresh, and nothing is left to say.
    set(
        &w,
        "t:a",
        vec![sym_unit("limit", "function", "fn limit() { return 128 }")],
    );
    let out = engine::reanchor(
        &store,
        &mut p,
        &no_git(),
        &engine::ReanchorInput {
            claim: id.to_string(),
            subject: Some(json_anchor("t:a", symbol_sel("limit"))),
            ..engine::ReanchorInput::default()
        },
        now(),
    )
    .unwrap();
    assert_eq!(out.claim.state, ClaimState::Fresh);
    assert!(out.claim.sides[0].detail.is_none());
    assert_eq!(out.claim.next, None);
}

/// Section 6.7, A1 with conflicting evidence: the name still names one
/// declaration and the quoted text is in other units. Nothing resolves;
/// the named unit is the first candidate, and the side says that name and
/// quote point to different units.
#[test]
fn a_name_and_a_quote_that_point_apart_are_candidates_with_that_cause() {
    let (w, mut p, store, id, stored) = named_and_quoted();
    let named = sym_unit("limit", "function", "fn limit() { return 128 }");
    let mut holder = sym_unit("other", "function", "fn other() { return 64 }");
    holder.sel = hibi::model::Selector {
        quote: Some("return 64".into()),
        ..symbol_sel("other")
    };
    w.borrow_mut()
        .overrides
        .insert(stored, Ok(vec![approx(named), approx(holder)]));
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Stale);
    let side = &v.sides[0];
    assert_eq!(
        (side.state, side.reason),
        (engine::SideState::Missing, Some(engine::Reason::NotExact))
    );
    assert_eq!(side.detail.as_deref(), Some(engine::NAME_AND_QUOTE_APART));
    let cs = side.candidates.as_ref().unwrap();
    assert_eq!(
        cs.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
        ["limit", "other"]
    );
    assert!(cs.iter().all(|c| c.anchor.is_some()));
    let next = format!("hibi reanchor {id} --subject <anchor>");
    assert_eq!(v.next, Some(next.clone()));
    let errors = engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new(id.as_str())],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(errors[0].code, hibi::Code::SideMissing);
    assert!(
        errors[0].message.contains(engine::NAME_AND_QUOTE_APART),
        "{}",
        errors[0]
    );
    assert_eq!(errors[0].remediation, Some(next));

    // Any other answer of candidates only says that nothing is identified.
    let (w, mut p, store, id, stored) = named_and_quoted();
    w.borrow_mut().overrides.insert(
        stored,
        Ok(vec![approx(sym_unit("other", "function", "fn other() {}"))]),
    );
    let v = view(&store, &mut p, &id);
    assert_eq!(v.sides[0].detail.as_deref(), Some(engine::NOT_IDENTIFIED));
}

/// Units that are identical where they stand carry the same printed
/// anchor. They are listed as a group that cannot be told apart: no anchor
/// is offered for them, and the remedy is given.
#[test]
fn identical_twins_are_listed_without_an_anchor() {
    let mut f = fixture(None);
    let twin = |key: &str| {
        let mut u = unit(key, "note", "the same text");
        u.sel = hibi::model::Selector::quote("the same text");
        u
    };
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(vec![twin("t1"), twin("t2"), unit("k3", "note", "other")]),
    );
    let v = view(&f.store, &mut f.p, &f.id);
    let side = &v.sides[1];
    assert_eq!(side.state, engine::SideState::Ambiguous);
    let cs = side.candidates.as_ref().unwrap();
    assert_eq!(cs.len(), 3);
    assert!(cs[0].anchor.is_none() && cs[1].anchor.is_none());
    assert_eq!(
        cs[2].anchor.as_deref(),
        Some(r#"{"loc":"u:b","sel":{"key":"k3"}}"#)
    );
    let json = serde_json::to_value(side).unwrap();
    assert_eq!(json["candidates"][0]["anchor"], serde_json::Value::Null);
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
    let message = &errors[0].message;
    assert_eq!(
        message
            .matches("identical to another one, no anchor selects it alone")
            .count(),
        2,
        "{message}"
    );

    // The provider can say it of one unit: a printed selector that does
    // not select its unit alone comes with `refuse:
    // quote_not_distinguishable`, also when no other listed unit has the
    // same selector. No anchor is offered for it.
    let mut marked_so = unit("k4", "note", "a sentence in a longer one");
    marked_so.refuse = Some(hibi::provider::Refuse::QuoteNotDistinguishable);
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(vec![marked_so, unit("k3", "note", "other")]),
    );
    let v = view(&f.store, &mut f.p, &f.id);
    let cs = v.sides[1].candidates.as_ref().unwrap();
    assert_eq!(cs[0].key, "k4");
    assert!(cs[0].anchor.is_none(), "{cs:?}");
    assert!(cs[1].anchor.is_some(), "{cs:?}");
    assert!(message.ends_with(engine::TWINS_REMEDY), "{message}");
    assert!(!message.contains(r#""quote":"the same text""#), "{message}");
}

/// `candidatesTotal` counts each unit once, also when the units with the
/// marked content and the units the provider returned are the same many
/// units, of which only the first are kept.
#[test]
fn the_candidate_total_counts_each_unit_once() {
    let mut f = fixture(None);
    // Fifty units with the marked content; the provider guesses at all of
    // them and at three others.
    let same: Vec<Unit> = (0..50)
        .map(|n| unit(&format!("s{n:02}"), "note", "first"))
        .collect();
    let others: Vec<Unit> = (0..3)
        .map(|n| unit(&format!("o{n}"), "note", &format!("other {n}")))
        .collect();
    let all: Vec<Unit> = same.iter().chain(&others).cloned().collect();
    set(&f.worlds[1], "u:b", all.clone());
    f.worlds[1].borrow_mut().overrides.insert(
        anchor("u:b", key_sel("k")),
        Ok(all.into_iter().map(approx).collect()),
    );
    let v = view(&f.store, &mut f.p, &f.id);
    let side = &v.sides[1];
    assert_eq!(side.reason, Some(engine::Reason::NotExact));
    assert_eq!(side.candidates_total, Some(53));
    let shown = side.candidates.as_ref().unwrap();
    assert_eq!(shown.len(), engine::MAX_CANDIDATES);
    assert!(shown.iter().all(|c| c.same_fingerprint));
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
