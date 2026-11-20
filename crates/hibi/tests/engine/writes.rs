//! The write commands of spec section 15.3: `claim`, `mark`, `reanchor`,
//! `retire` (R33, R36, R52, R54, R55, R62).

use hibi::Code;
use hibi::engine::{
    self, AnchorInput, ClaimInput, ClaimState, MarkInput, ReanchorInput, RetireInput, SideState,
};
use hibi::model::{ClaimId, Selector};
use hibi::provider::{ProviderErrorCode as C, Providers, Refuse, Unit};

use crate::support::*;

fn setup() -> (Shared, Providers, MemStore) {
    let w = key_world();
    set(
        &w,
        "t:a",
        vec![
            unit("k1", "note", "Retries are capped at 5."),
            unit("k2", "note", "The cap is configurable."),
            unit("k3", "note", "Timeouts are 30 seconds."),
            unit("k4", "note", "Nothing is cached."),
        ],
    );
    let p = providers(&w, &["t"]);
    (w, p, MemStore::new())
}

fn on(statement: &str, key: &str) -> ClaimInput {
    ClaimInput {
        subject: json_anchor("t:a", key_sel(key)),
        ..claim_input(statement, "t:a", &[])
    }
}

fn claim(
    store: &MemStore,
    p: &mut Providers,
    inputs: &[ClaimInput],
    indexed: bool,
) -> Result<engine::ClaimOutput, Vec<hibi::Error>> {
    engine::claim(store, p, &no_git(), inputs, indexed, now())
}

fn reanchor(
    store: &MemStore,
    p: &mut Providers,
    input: ReanchorInput,
) -> Result<engine::ReanchorOutput, hibi::Error> {
    engine::reanchor(store, p, &no_git(), &input, ts("2026-12-01T00:00:00Z"))
}

fn retire(store: &MemStore, id: &ClaimId) {
    engine::retire(
        store,
        &RetireInput {
            claims: vec![id.to_string()],
            ..RetireInput::default()
        },
        now(),
    )
    .unwrap();
}

fn approx(mut u: Unit) -> Unit {
    u.approx = true;
    u
}

// ---------------------------------------------------------------------------
// claim
// ---------------------------------------------------------------------------

/// Section 5.4: the stored selector is the one the provider returns for
/// the resolved unit, not the one the caller typed.
#[test]
fn claim_stores_the_completed_selector_and_one_mark() {
    let (_w, mut p, store) = setup();
    let input = ClaimInput {
        subject: json_anchor("t:a", Selector::quote("capped at 5")),
        evidence: vec![json_anchor("t:a", Selector::quote("configurable"))],
        recheck: Some("P90D".into()),
        note: Some("  see retry.rs  ".into()),
        by: Some("  agent  ".into()),
        ..claim_input("  Retries are capped.  ", "t:a", &[])
    };
    let out = claim(&store, &mut p, &[input], false).unwrap();
    assert_eq!(store.appends.get(), 1);
    assert_eq!(store.counts(), (1, 1, 0));
    let ledger = store.ledger();
    let c = &ledger.claims[0];
    let m = &ledger.marks[0];
    assert_eq!(c.statement, "Retries are capped.");
    assert_eq!(c.subject, anchor("t:a", key_sel("k1")));
    assert_eq!(c.evidence, vec![anchor("t:a", key_sel("k2"))]);
    assert_eq!(c.note.as_deref(), Some("see retry.rs"));
    assert_eq!(m.claim, c.id);
    assert_eq!(m.at, now());
    assert_eq!(m.by.as_deref(), Some("agent"));
    assert_eq!(m.note, None);
    assert_eq!(m.sides.len(), 2);
    assert_eq!(
        m.sides[0].fp,
        hibi::fingerprint::prose_v1("Retries are capped at 5.")
    );

    let r = &out.claims[0];
    assert!(r.created);
    assert_eq!((&r.id, &r.mark), (&c.id, &m.id));
    assert_eq!(r.claim.state, ClaimState::Fresh);
    assert_eq!(r.claim.next, None);
    assert_eq!(r.claim.mark.as_ref(), Some(&m.id));
    assert_eq!(r.claim.expires_at, Some(ts("2027-02-18T18:04:11Z")));
    assert_eq!(r.claim.sides[0].anchor, c.subject);
    assert!(r.claim.sides.iter().all(|s| s.state == SideState::Same));
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["claims"][0]["created"], true);
    assert_eq!(json["claims"][0]["claim"]["state"], "fresh");
    assert_eq!(json["warnings"], serde_json::json!([]));
}

/// Decision 38: `claim` on an existing live claim adds a mark; on a retired
/// claim it fails with `claim_retired`.
#[test]
fn claim_on_an_existing_claim_marks_it_and_on_a_retired_one_fails() {
    let (_w, mut p, store) = setup();
    let first = claim(&store, &mut p, &[on("S", "k1")], false).unwrap();
    let again = engine::claim(
        &store,
        &mut p,
        &no_git(),
        &[on("S", "k1")],
        false,
        ts("2026-12-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(again.claims[0].id, first.claims[0].id);
    assert!(!again.claims[0].created);
    assert_ne!(again.claims[0].mark, first.claims[0].mark);
    assert_eq!(store.counts(), (1, 2, 0));

    retire(&store, &first.claims[0].id);
    let before = store.appends.get();
    let errors = claim(&store, &mut p, &[on("S", "k1")], false).unwrap_err();
    assert_eq!(codes(&errors), [(Code::ClaimRetired, None)]);
    assert!(
        errors[0]
            .remediation
            .as_deref()
            .unwrap()
            .contains("different --note")
    );
    assert_eq!(store.appends.get(), before);
    // A different note makes a different claim.
    let noted = ClaimInput {
        note: Some("second time".into()),
        ..on("S", "k1")
    };
    assert!(claim(&store, &mut p, &[noted], false).unwrap().claims[0].created);
}

/// R33: a batch with invalid elements writes nothing and reports each
/// failing element's `index`.
#[test]
fn r33_claim_batch_is_all_or_nothing_with_indexes() {
    let (w, mut p, store) = setup();
    set_err(&w, "t:b", C::NotText);
    let inputs = [
        on("good", "k1"),
        on("missing unit", "nope"),
        on("", "k1"),
        on("also good", "k2"),
        ClaimInput {
            subject: json_anchor("t:b", key_sel("x")),
            ..claim_input("unavailable", "t:b", &[])
        },
        ClaimInput {
            subject: json_anchor("t:zzz", key_sel("x")),
            ..claim_input("no resource", "t:zzz", &[])
        },
        ClaimInput {
            recheck: Some("90 days".into()),
            ..on("bad duration", "k1")
        },
        ClaimInput {
            subject: "nolocator".into(),
            ..on("bad anchor", "k1")
        },
        ClaimInput {
            subject: "other:x".into(),
            ..on("unknown scheme", "k1")
        },
        ClaimInput {
            recheck: Some("P9999Y".into()),
            ..on("overflow", "k1")
        },
    ];
    let errors = claim(&store, &mut p, &inputs, true).unwrap_err();
    assert_eq!(
        codes(&errors),
        [
            (Code::SideMissing, Some(1)),
            (Code::InvalidArgument, Some(2)),
            (Code::SideUnavailable, Some(4)),
            (Code::SideMissing, Some(5)),
            (Code::InvalidDuration, Some(6)),
            (Code::InvalidAnchor, Some(7)),
            (Code::UnknownScheme, Some(8)),
            (Code::InvalidDuration, Some(9)),
        ]
    );
    assert!(
        errors[2].message.contains("not_text"),
        "{}",
        errors[2].message
    );
    assert_eq!(errors[2].remediation, None);
    assert!(errors[3].message.contains("resource_not_found"));
    assert_eq!(store.appends.get(), 0);
    assert_eq!(store.counts(), (0, 0, 0));

    // The argument form reports no index.
    let errors = claim(&store, &mut p, &[on("x", "nope")], false).unwrap_err();
    assert_eq!(codes(&errors), [(Code::SideMissing, None)]);

    // The same batch without the bad elements is written in one append.
    let out = claim(
        &store,
        &mut p,
        &[on("good", "k1"), on("also good", "k2")],
        true,
    )
    .unwrap();
    assert_eq!(out.claims.len(), 2);
    assert_eq!(store.appends.get(), 1);
    assert_eq!(store.counts(), (2, 2, 0));
}

#[test]
fn claim_write_checks() {
    let (w, mut p, store) = setup();
    let check = |p: &mut Providers, input: ClaimInput| -> hibi::Error {
        let mut errors = claim(&store, p, &[input], false).unwrap_err();
        assert_eq!(errors.len(), 1);
        errors.remove(0)
    };

    // Several units: the message lists them.
    let e = check(
        &mut p,
        ClaimInput {
            subject: json_anchor("t:a", Selector::quote("cap")),
            ..claim_input("S", "t:a", &[])
        },
    );
    assert_eq!(e.code, Code::SideAmbiguous);
    assert!(
        e.message.contains("k1") && e.message.contains("k2"),
        "{}",
        e.message
    );

    // Approximate: selector_not_exact, showing what was found.
    w.borrow_mut().overrides.insert(
        anchor("t:a", Selector::quote("capped at five")),
        Ok(vec![approx(unit("k1", "note", "Retries are capped at 5."))]),
    );
    let e = check(
        &mut p,
        ClaimInput {
            subject: json_anchor("t:a", Selector::quote("capped at five")),
            ..claim_input("S", "t:a", &[])
        },
    );
    assert_eq!(e.code, Code::SelectorNotExact);
    assert!(
        e.message.contains("Retries are capped at 5."),
        "{}",
        e.message
    );

    // R52: refuse maps to the code of the same name. Evidence fails too.
    for (refuse, code) in [
        (Refuse::QuoteInComment, Code::QuoteInComment),
        (Refuse::QuoteSpansUnits, Code::QuoteSpansUnits),
    ] {
        let mut refused = unit("k1", "note", "Retries are capped at 5.");
        refused.refuse = Some(refuse);
        w.borrow_mut().overrides.insert(
            anchor("t:a", Selector::quote("// capped")),
            Ok(vec![refused]),
        );
        let e = check(
            &mut p,
            ClaimInput {
                evidence: vec![json_anchor("t:a", Selector::quote("// capped"))],
                ..on("S", "k2")
            },
        );
        assert_eq!(e.code, code);
        assert!(e.message.contains("evidence 1"), "{}", e.message);
    }

    // Approximate comes before refuse.
    let mut both = approx(unit("k1", "note", "x"));
    both.refuse = Some(Refuse::QuoteInComment);
    w.borrow_mut()
        .overrides
        .insert(anchor("t:a", Selector::quote("both")), Ok(vec![both]));
    let e = check(
        &mut p,
        ClaimInput {
            subject: json_anchor("t:a", Selector::quote("both")),
            ..claim_input("S", "t:a", &[])
        },
    );
    assert_eq!(e.code, Code::SelectorNotExact);

    // A declared scheme without a registered provider.
    p.declare_unregistered("arch");
    let e = check(&mut p, claim_input("S", "arch:model", &[]));
    assert_eq!(e.code, Code::SideUnavailable);
    assert!(e.message.contains("provider_not_registered"));
    assert_eq!(e.remediation.as_deref(), Some("hibi providers"));

    assert_eq!(store.counts(), (0, 0, 0));
    assert_eq!(store.appends.get(), 0);
}

/// R62 for a new anchor: a selector key the provider does not allow is
/// `unsupported_selector` and is never sent.
#[test]
fn r62_unsupported_selector_is_not_sent() {
    let (w, mut p, store) = setup();
    w.borrow_mut().log.clear();
    let errors = claim(
        &store,
        &mut p,
        &[ClaimInput {
            subject: json_anchor("t:a", symbol_sel("retry")),
            ..claim_input("S", "t:a", &[])
        }],
        true,
    )
    .unwrap_err();
    assert_eq!(codes(&errors), [(Code::UnsupportedSelector, Some(0))]);
    assert!(errors[0].message.contains("symbol"));
    assert!(
        log(&w).iter().all(|l| !l.starts_with("resolve")),
        "{:?}",
        log(&w)
    );
    assert_eq!(store.appends.get(), 0);
}

#[test]
fn two_elements_that_build_the_same_claim_write_it_once() {
    let (_w, mut p, store) = setup();
    let via_quote = ClaimInput {
        subject: json_anchor("t:a", Selector::quote("capped at 5")),
        ..claim_input("S", "t:a", &[])
    };
    let out = claim(&store, &mut p, &[on("S", "k1"), via_quote], true).unwrap();
    assert_eq!(out.claims[0].id, out.claims[1].id);
    assert_eq!(
        (out.claims[0].created, out.claims[1].created),
        (true, false)
    );
    assert_eq!(store.counts(), (1, 1, 0));
}

#[test]
fn stdin_documents() {
    let ok = engine::parse_claim_stdin(
        r#"[{"statement":"S","subject":"file:a.md::quote","evidence":[{"loc":"t:a","sel":{"key":"k1"}}],"recheck":"P90D"},
            {"statement":"T","subject":{"loc":"t:a","sel":{}}}]"#,
    )
    .unwrap();
    assert_eq!(ok.len(), 2);
    assert_eq!(
        ok[0].subject,
        AnchorInput::Compact("file:a.md::quote".into())
    );
    assert_eq!(ok[0].evidence, vec![json_anchor("t:a", key_sel("k1"))]);
    assert_eq!(ok[0].recheck.as_deref(), Some("P90D"));
    assert!(ok[1].evidence.is_empty());

    for text in ["", "{}", "[]", "\"x\"", "[1"] {
        let e = engine::parse_claim_stdin(text).unwrap_err();
        assert_eq!(codes(&e), [(Code::InvalidJson, None)], "{text}");
        let e = engine::parse_mark_stdin(text).unwrap_err();
        assert_eq!(codes(&e), [(Code::InvalidJson, None)], "{text}");
    }
    let e = engine::parse_claim_stdin(
        r#"[{"statement":"S","subject":"t:a"},
            {"statement":"S","subject":"t:a","extra":1},
            {"subject":"t:a"},
            {"statement":"S","subject":{"loc":"t:a","sel":{"key":"k","x":1}}},
            {"statement":"S","subject":"t:a","evidence":[{"loc":"nope","sel":{}}]},
            7]"#,
    )
    .unwrap_err();
    assert_eq!(
        codes(&e),
        [
            (Code::InvalidJson, Some(1)),
            (Code::InvalidJson, Some(2)),
            (Code::InvalidAnchor, Some(3)),
            (Code::InvalidAnchor, Some(4)),
            (Code::InvalidJson, Some(5)),
        ]
    );

    let marks = engine::parse_mark_stdin(
        r#"["c1-0123", {"claim":"c1-4567","note":"n","by":"b"}, {"claim":"c1-89ab"}]"#,
    )
    .unwrap();
    assert_eq!(marks[0], MarkInput::new("c1-0123"));
    assert_eq!(marks[1].note.as_deref(), Some("n"));
    assert_eq!(marks[2].by, None);
    let e = engine::parse_mark_stdin(r#"["c1-0123", {"id":"c1-4567"}, 3]"#).unwrap_err();
    assert_eq!(
        codes(&e),
        [(Code::InvalidJson, Some(1)), (Code::InvalidJson, Some(2))]
    );
}

// ---------------------------------------------------------------------------
// mark
// ---------------------------------------------------------------------------

#[test]
fn mark_writes_one_mark_with_the_current_fingerprints() {
    let (w, mut p, store) = setup();
    let id = claim_at(&store, &mut p, on("S", "k1"), "2026-01-01T00:00:00Z");
    edit(&w, "t:a", "k1", "Retries are capped at 7.");
    let input = MarkInput {
        claim: id.as_str()[..10].to_owned(),
        note: Some(" re-read retry.rs ".into()),
        by: Some("agent".into()),
    };
    let out = engine::mark(&store, &mut p, &no_git(), &[input], false, now()).unwrap();
    assert_eq!(store.appends.get(), 2);
    assert_eq!(store.counts(), (1, 2, 0));
    let ledger = store.ledger();
    let m = ledger
        .marks
        .iter()
        .find(|m| m.id == out.marks[0].mark)
        .unwrap();
    assert_eq!(m.at, now());
    assert_eq!(m.note.as_deref(), Some("re-read retry.rs"));
    assert_eq!(m.by.as_deref(), Some("agent"));
    assert_eq!(
        m.sides[0].fp,
        hibi::fingerprint::prose_v1("Retries are capped at 7.")
    );
    let v = &out.marks[0].claim_view;
    assert_eq!(v.state, ClaimState::Fresh);
    assert_eq!(v.mark.as_ref(), Some(&m.id));
    assert_eq!(v.next, None);
    assert!(v.sides[0].text.is_none());
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["marks"][0]["claim"], id.as_str());
    assert_eq!(json["marks"][0]["claimView"]["id"], id.as_str());
}

/// Decision 4: `mark` accepts a profile mismatch.
#[test]
fn mark_accepts_a_profile_mismatch() {
    let (w, mut p, store) = setup();
    let id = claim_at(&store, &mut p, on("S", "k1"), "2026-01-01T00:00:00Z");
    let mut other = unit("k1", "note", "Retries are capped at 5.");
    other.fp = fp("ast-v1", 'c');
    set(&w, "t:a", vec![other]);
    let out = engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new(id.as_str())],
        false,
        now(),
    )
    .unwrap();
    assert_eq!(out.marks[0].claim_view.state, ClaimState::Fresh);
    assert_eq!(out.marks[0].claim_view.sides[0].state, SideState::Same);
}

/// R33 for `mark`: one bad element, nothing written, its index reported.
#[test]
fn r33_mark_batch_is_all_or_nothing_with_indexes() {
    let (w, mut p, store) = setup();
    let a = claim_at(&store, &mut p, on("A", "k1"), "2026-01-01T00:00:00Z");
    let b = claim_at(&store, &mut p, on("B", "k2"), "2026-01-01T00:00:00Z");
    let c = claim_at(&store, &mut p, on("C", "k3"), "2026-01-01T00:00:00Z");
    let d = claim_at(&store, &mut p, on("D", "k3"), "2026-01-01T00:00:00Z");
    retire(&store, &d);
    // b's unit is gone; c's becomes ambiguous.
    w.borrow_mut()
        .overrides
        .insert(anchor("t:a", key_sel("k2")), Ok(vec![]));
    w.borrow_mut().overrides.insert(
        anchor("t:a", key_sel("k3")),
        Ok(vec![unit("k3", "note", "x"), unit("k4", "note", "y")]),
    );
    let before = (store.appends.get(), store.counts());
    let inputs = [
        MarkInput::new(a.as_str()),
        MarkInput::new(b.as_str()),
        MarkInput::new("c1-zzzzzz"),
        MarkInput::new(c.as_str()),
        MarkInput::new(d.as_str()),
        MarkInput::new("bogus"),
    ];
    let errors = engine::mark(&store, &mut p, &no_git(), &inputs, true, now()).unwrap_err();
    assert_eq!(
        codes(&errors),
        [
            (Code::SideMissing, Some(1)),
            (Code::ClaimNotFound, Some(2)),
            (Code::SideAmbiguous, Some(3)),
            (Code::ClaimRetired, Some(4)),
            (Code::InvalidArgument, Some(5)),
        ]
    );
    // The remediation of a missing side is the claim's `next`.
    assert_eq!(
        errors[0].remediation,
        Some(format!("hibi reanchor {b} --subject <anchor>"))
    );
    assert_eq!((store.appends.get(), store.counts()), before);

    // Unavailable: side_unavailable with the reason code.
    set_err(&w, "t:a", C::TooLarge);
    let errors = engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new(a.as_str())],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(codes(&errors), [(Code::SideUnavailable, None)]);
    assert!(errors[0].message.contains("too_large"));
    assert_eq!(errors[0].remediation, None);
    assert_eq!((store.appends.get(), store.counts()), before);
}

// ---------------------------------------------------------------------------
// reanchor
// ---------------------------------------------------------------------------

/// A claim with a subject and three evidence sides.
fn wide(store: &MemStore, p: &mut Providers) -> ClaimId {
    let input = ClaimInput {
        evidence: vec![
            json_anchor("t:a", key_sel("k2")),
            json_anchor("t:a", key_sel("k3")),
            json_anchor("t:a", key_sel("k4")),
        ],
        recheck: Some("P30D".into()),
        note: Some("how to check".into()),
        ..on("S", "k1")
    };
    claim_at(store, p, input, "2026-01-01T00:00:00Z")
}

/// R36: `reanchor` writes a new claim, a mark and a retirement of the old
/// claim with `replacedBy`, in one append; the old claim then reports
/// `retired`.
#[test]
fn r36_reanchor_writes_claim_mark_and_retirement() {
    let (_w, mut p, store) = setup();
    let old = wide(&store, &mut p);
    let appends = store.appends.get();
    let out = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: old.to_string(),
            statement: Some("S, reworded".into()),
            evidence: vec![(1, json_anchor("t:a", Selector::quote("Timeouts")))],
            drop_evidence: vec![2],
            add_evidence: vec![json_anchor("t:a", key_sel("k1"))],
            recheck: Some("none".into()),
            by: Some("agent".into()),
            ..ReanchorInput::default()
        },
    )
    .unwrap();
    assert_eq!(store.appends.get(), appends + 1);
    assert_eq!(store.counts(), (2, 2, 1));
    assert_eq!(out.old, old);
    assert_ne!(out.new, old);

    let ledger = store.ledger();
    let new = ledger.claim(out.new.as_str()).unwrap();
    assert_eq!(new.statement, "S, reworded");
    assert_eq!(new.subject, anchor("t:a", key_sel("k1")));
    assert_eq!(
        new.evidence,
        vec![
            anchor("t:a", key_sel("k3")),
            anchor("t:a", key_sel("k4")),
            anchor("t:a", key_sel("k1")),
        ]
    );
    assert_eq!(new.recheck, None);
    assert_eq!(new.note.as_deref(), Some("how to check"));
    let m = ledger.marks.iter().find(|m| m.id == out.mark).unwrap();
    assert_eq!(m.claim, out.new);
    assert_eq!(m.at, ts("2026-12-01T00:00:00Z"));
    assert_eq!(m.by.as_deref(), Some("agent"));
    assert_eq!(m.sides.len(), 4);
    let r = &ledger.retirements[0];
    assert_eq!(r.id, out.retirement);
    assert_eq!(r.retires, old);
    assert_eq!(r.replaced_by.as_ref(), Some(&out.new));
    assert_eq!(r.at, ts("2026-12-01T00:00:00Z"));
    assert_eq!(r.by.as_deref(), Some("agent"));

    assert_eq!(out.claim.id, out.new);
    assert_eq!(out.claim.state, ClaimState::Fresh);
    assert_eq!(out.claim.sides.len(), 4);
    assert_eq!(view(&store, &mut p, &old).state, ClaimState::Retired);
    assert_eq!(view(&store, &mut p, &out.new).state, ClaimState::Fresh);
    let json = serde_json::to_value(&out).unwrap();
    for key in ["old", "new", "mark", "retirement", "claim", "warnings"] {
        assert!(json.get(key).is_some(), "{key}");
    }

    // A second reanchor of the old claim fails: it is retired.
    let e = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: old.to_string(),
            statement: Some("again".into()),
            ..ReanchorInput::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, Code::ClaimRetired);
}

#[test]
fn reanchor_note_and_recheck_settings() {
    let (_w, mut p, store) = setup();
    let old = wide(&store, &mut p);
    let out = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: old.to_string(),
            note: Some("none".into()),
            recheck: Some("P1Y".into()),
            ..ReanchorInput::default()
        },
    )
    .unwrap();
    let ledger = store.ledger();
    let new = ledger.claim(out.new.as_str()).unwrap();
    assert_eq!(
        (new.note.as_deref(), new.recheck.as_deref()),
        (None, Some("P1Y"))
    );
    assert_eq!(out.claim.expires_at, Some(ts("2027-12-01T00:00:00Z")));

    let e = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: out.new.to_string(),
            recheck: Some("soon".into()),
            ..ReanchorInput::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, Code::InvalidDuration);
}

#[test]
fn reanchor_errors_write_nothing() {
    let (w, mut p, store) = setup();
    let old = wide(&store, &mut p);
    let before = (store.appends.get(), store.counts());
    let err = |p: &mut Providers, input: ReanchorInput| reanchor(&store, p, input).unwrap_err();
    let base = || ReanchorInput {
        claim: old.to_string(),
        ..ReanchorInput::default()
    };

    assert_eq!(err(&mut p, base()).code, Code::NoChange);
    // The same `n` twice, in any combination, or an `n` that does not exist.
    for (evidence, drop) in [
        (vec![1, 1], vec![]),
        (vec![2], vec![2]),
        (vec![], vec![3, 3]),
        (vec![4], vec![]),
        (vec![], vec![0]),
    ] {
        let input = ReanchorInput {
            evidence: evidence
                .into_iter()
                .map(|n| (n, json_anchor("t:a", key_sel("k1"))))
                .collect(),
            drop_evidence: drop,
            ..base()
        };
        assert_eq!(err(&mut p, input).code, Code::Usage);
    }
    assert_eq!(
        err(
            &mut p,
            ReanchorInput {
                claim: "c1-zzzz".into(),
                ..ReanchorInput::default()
            }
        )
        .code,
        Code::ClaimNotFound
    );
    assert_eq!(
        err(
            &mut p,
            ReanchorInput {
                subject: Some("::".into()),
                ..base()
            }
        )
        .code,
        Code::InvalidAnchor
    );
    assert_eq!(
        err(
            &mut p,
            ReanchorInput {
                subject: Some("undeclared:x".into()),
                ..base()
            }
        )
        .code,
        Code::UnknownScheme
    );
    assert_eq!(
        err(
            &mut p,
            ReanchorInput {
                subject: Some(json_anchor("t:a", symbol_sel("s"))),
                ..base()
            }
        )
        .code,
        Code::UnsupportedSelector
    );
    assert_eq!(
        err(
            &mut p,
            ReanchorInput {
                add_evidence: vec![json_anchor("t:a", key_sel("nope"))],
                ..base()
            }
        )
        .code,
        Code::SideMissing
    );

    // A replaced side that resolves approximately, and one that is refused
    // (R52).
    w.borrow_mut().overrides.insert(
        anchor("t:a", Selector::quote("capped at five")),
        Ok(vec![approx(unit("k1", "note", "Retries are capped at 5."))]),
    );
    let e = err(
        &mut p,
        ReanchorInput {
            subject: Some(json_anchor("t:a", Selector::quote("capped at five"))),
            ..base()
        },
    );
    assert_eq!(e.code, Code::SelectorNotExact);
    let mut refused = unit("k1", "note", "Retries are capped at 5.");
    refused.refuse = Some(Refuse::QuoteInComment);
    w.borrow_mut()
        .overrides
        .insert(anchor("t:a", Selector::quote("// note")), Ok(vec![refused]));
    let e = err(
        &mut p,
        ReanchorInput {
            evidence: vec![(2, json_anchor("t:a", Selector::quote("// note")))],
            ..base()
        },
    );
    assert_eq!(e.code, Code::QuoteInComment);

    // A kept side that no longer resolves.
    w.borrow_mut()
        .overrides
        .insert(anchor("t:a", key_sel("k3")), Ok(vec![]));
    let e = err(
        &mut p,
        ReanchorInput {
            statement: Some("new".into()),
            ..base()
        },
    );
    assert_eq!(e.code, Code::SideMissing);
    assert!(e.message.contains("evidence 2"), "{}", e.message);

    assert_eq!((store.appends.get(), store.counts()), before);
}

/// Steps 3 and 4: a kept side may be approximate and refused; its selector
/// is refreshed to the completed selector of the unit found.
#[test]
fn reanchor_refreshes_a_kept_approximate_side() {
    let (w, mut p, store) = setup();
    let old = claim_at(&store, &mut p, on("S", "k1"), "2026-01-01T00:00:00Z");
    let mut renamed = approx(unit("k9", "note", "Retries are capped at 9."));
    renamed.refuse = Some(Refuse::QuoteSpansUnits);
    w.borrow_mut()
        .overrides
        .insert(anchor("t:a", key_sel("k1")), Ok(vec![renamed]));
    {
        let mut world = w.borrow_mut();
        let units = world.resources.get_mut("t:a").unwrap().as_mut().unwrap();
        units[0] = unit("k9", "note", "Retries are capped at 9.");
    }
    let out = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: old.to_string(),
            ..ReanchorInput::default()
        },
    )
    .unwrap();
    let ledger = store.ledger();
    assert_eq!(
        ledger.claim(out.new.as_str()).unwrap().subject,
        anchor("t:a", key_sel("k9"))
    );
    assert!(!out.claim.sides[0].approx);
    assert_eq!(out.claim.next, None);
    let v = view(&store, &mut p, &out.new);
    assert_eq!((v.state, v.sides[0].approx), (ClaimState::Fresh, false));
}

/// R54: a reanchor whose result equals a retired claim fails with
/// `claim_retired`, writes nothing, and names the `--note` command with
/// the caller's other arguments repeated.
#[test]
fn r54_reanchor_onto_a_retired_claim_fails() {
    let (_w, mut p, store) = setup();
    let first = claim_at(&store, &mut p, on("S", "k1"), "2026-01-01T00:00:00Z");
    let moved = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: first.to_string(),
            subject: Some(json_anchor("t:a", key_sel("k2"))),
            ..ReanchorInput::default()
        },
    )
    .unwrap();
    // Back to the earlier form: that claim is retired.
    let before = (store.appends.get(), store.counts());
    let e = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: moved.new.as_str()[..12].to_owned(),
            subject: Some(json_anchor("t:a", Selector::quote("it's capped at 5"))),
            by: Some("an agent".into()),
            ..ReanchorInput::default()
        },
    );
    // That quote matches nothing; use one that does.
    assert_eq!(e.unwrap_err().code, Code::SideMissing);
    let e = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: moved.new.as_str()[..12].to_owned(),
            subject: Some(json_anchor("t:a", key_sel("k1"))),
            by: Some("an agent".into()),
            ..ReanchorInput::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, Code::ClaimRetired);
    assert!(e.message.contains(first.as_str()));
    assert_eq!(
        e.remediation,
        Some(format!(
            "hibi reanchor {} --subject '{{\"loc\":\"t:a\",\"sel\":{{\"key\":\"k1\"}}}}' --by 'an agent' --note <text>",
            moved.new
        ))
    );
    assert_eq!((store.appends.get(), store.counts()), before);

    // With a note it is a different claim and succeeds.
    let ok = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: moved.new.to_string(),
            subject: Some(json_anchor("t:a", key_sel("k1"))),
            note: Some("moved back".into()),
            ..ReanchorInput::default()
        },
    )
    .unwrap();
    assert_ne!(ok.new, first);
}

#[test]
fn the_reanchor_remediation_quotes_for_a_posix_shell() {
    assert_eq!(engine::sh_quote("file:a.md#f"), "'file:a.md#f'");
    assert_eq!(engine::sh_quote("file:src/a-b_c.rs"), "file:src/a-b_c.rs");
    assert_eq!(engine::sh_quote(""), "''");
    assert_eq!(engine::sh_quote("it's"), "'it'\\''s'");
    assert_eq!(engine::sh_quote("a b"), "'a b'");
}

/// Step 5: a result that equals another live claim marks that claim and
/// retires the old one with `replacedBy` naming it. No claim is written.
#[test]
fn reanchor_onto_another_live_claim_marks_it() {
    let (_w, mut p, store) = setup();
    let a = claim_at(&store, &mut p, on("S", "k1"), "2026-01-01T00:00:00Z");
    let b = claim_at(&store, &mut p, on("S", "k2"), "2026-01-01T00:00:00Z");
    let out = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: a.to_string(),
            subject: Some(json_anchor("t:a", key_sel("k2"))),
            ..ReanchorInput::default()
        },
    )
    .unwrap();
    assert_eq!(out.new, b);
    assert_eq!(store.counts(), (2, 3, 1));
    let ledger = store.ledger();
    assert_eq!(ledger.retirements[0].retires, a);
    assert_eq!(ledger.retirements[0].replaced_by.as_ref(), Some(&b));
    assert_eq!(ledger.marks.iter().filter(|m| m.claim == b).count(), 2);
    assert_eq!(out.claim.mark.as_ref(), Some(&out.mark));
    assert_eq!(view(&store, &mut p, &a).state, ClaimState::Retired);
    assert_eq!(view(&store, &mut p, &b).state, ClaimState::Fresh);
}

/// Spec 15.3: every failing element of a standard-input batch is reported
/// in one run, whether it fails to parse or fails a later check; nothing
/// is written.
#[test]
fn a_batch_reports_malformed_elements_together_with_the_other_failures() {
    let (_w, mut p, store) = setup();
    let text = r#"[
        {"statement":"a","subject":{"loc":"t:a","sel":{"key":"nope"}}},
        {"statement":"b","subject":{"loc":"t:a","sel":{"key":"k1"}},"bogus":1},
        {"statement":"c","subject":{"loc":"t:a","sel":{"key":"k1"}},"recheck":"P1X"},
        {"statement":"d","subject":{"loc":"t:a","sel":{"key":"k2"}}},
        {"statement":"e","subject":{"loc":"t:a","sel":{"nokey":1}}}
    ]"#;
    let elements = engine::read_claim_stdin(text).unwrap();
    assert_eq!(elements.len(), 5);
    let errors = engine::claim_batch(&store, &mut p, &no_git(), &elements, now()).unwrap_err();
    assert_eq!(
        codes(&errors),
        [
            (Code::SideMissing, Some(0)),
            (Code::InvalidJson, Some(1)),
            (Code::InvalidDuration, Some(2)),
            (Code::InvalidAnchor, Some(4)),
        ]
    );
    assert_eq!(store.counts(), (0, 0, 0));
    // The strict reader still refuses the document on its own.
    assert_eq!(
        codes(&engine::parse_claim_stdin(text).unwrap_err()),
        [(Code::InvalidJson, Some(1)), (Code::InvalidAnchor, Some(4))]
    );

    let good = claim(&store, &mut p, &[on("S", "k1")], false).unwrap();
    let id = good.claims[0].id.to_string();
    let text = format!(r#"["c1-zzzz", {{"id":"{id}"}}, "{id}", 3, {{"claim":"not an id"}}]"#);
    let elements = engine::read_mark_stdin(&text).unwrap();
    let errors = engine::mark_batch(&store, &mut p, &no_git(), &elements, now()).unwrap_err();
    assert_eq!(
        codes(&errors),
        [
            (Code::ClaimNotFound, Some(0)),
            (Code::InvalidJson, Some(1)),
            (Code::InvalidJson, Some(3)),
            (Code::InvalidArgument, Some(4)),
        ]
    );
    assert_eq!(store.counts(), (1, 1, 0));
}

// ---------------------------------------------------------------------------
// What a write accepted
// ---------------------------------------------------------------------------

/// Every write that produces a mark says what state the claim was in
/// before, and which sides were not `same`, and carries `nextNote`.
#[test]
fn writes_report_the_state_they_accepted() {
    let (w, mut p, store) = setup();
    let input = || ClaimInput {
        evidence: vec![json_anchor("t:a", key_sel("k2"))],
        ..on("S", "k1")
    };
    // A new claim accepted nothing.
    let first = claim(&store, &mut p, &[input()], false).unwrap();
    let id = first.claims[0].id.clone();
    assert_eq!(first.claims[0].accepted, None);
    assert_eq!(first.next_note, engine::NEXT_NOTE);
    assert!(engine::NEXT_NOTE.contains("retire the claim"));

    // The same claim again while fresh: was fresh, nothing to list.
    let again = claim(&store, &mut p, &[input()], false).unwrap();
    let a = again.claims[0].accepted.as_ref().unwrap();
    assert_eq!((a.was, a.sides.len()), (ClaimState::Fresh, 0));

    // The evidence changes: the claim is stale, and each write that marks
    // it says so.
    edit(&w, "t:a", "k2", "The cap is fixed.");
    let stale = serde_json::json!({
        "was": "stale",
        "sides": [{"index": 1, "role": "evidence", "loc": "t:a", "state": "changed",
                   "reason": "fingerprint_differs"}]
    });
    let again = claim(&store, &mut p, &[input()], false).unwrap();
    assert!(!again.claims[0].created);
    assert_eq!(
        serde_json::to_value(&again.claims[0].accepted).unwrap(),
        stale
    );
    assert_eq!(again.claims[0].claim.state, ClaimState::Fresh);

    edit(&w, "t:a", "k2", "The cap is fixed at 9.");
    let marked = engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new(id.as_str())],
        false,
        ts("2026-12-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&marked.marks[0].accepted).unwrap(),
        stale
    );
    assert_eq!(marked.next_note, engine::NEXT_NOTE);
    let json = serde_json::to_value(&marked).unwrap();
    assert_eq!(json["nextNote"], engine::NEXT_NOTE);
    assert_eq!(json["marks"][0]["accepted"], stale);

    // `reanchor` that replaces the subject also marks the kept evidence,
    // which changed: the output says the old claim was stale and why.
    edit(&w, "t:a", "k2", "The cap is gone.");
    let out = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: id.to_string(),
            subject: Some(json_anchor("t:a", key_sel("k3"))),
            ..ReanchorInput::default()
        },
    )
    .unwrap();
    assert_eq!(serde_json::to_value(&out.accepted).unwrap(), stale);
    assert!(!out.merged);
    assert_eq!(out.next_note, engine::NEXT_NOTE);
    assert_eq!(out.claim.state, ClaimState::Fresh);

    // An expired claim: was expired, no side to list.
    let timed = claim(
        &store,
        &mut p,
        &[ClaimInput {
            recheck: Some("P1D".into()),
            ..on("T", "k1")
        }],
        false,
    )
    .unwrap();
    let marked = engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new(timed.claims[0].id.as_str())],
        false,
        ts("2027-06-01T00:00:00Z"),
    )
    .unwrap();
    let a = &marked.marks[0].accepted;
    assert_eq!((a.was, a.sides.len()), (ClaimState::Expired, 0));
}

/// `reanchor` that lands on another live claim says it merged; a
/// replacement named in `retire` must be live; a write on a retired claim
/// points at its replacement.
#[test]
fn merges_and_retired_claims_are_said_plainly() {
    let (_w, mut p, store) = setup();
    let a = claim_at(&store, &mut p, on("A", "k1"), NOW);
    let b = claim_at(&store, &mut p, on("B", "k1"), NOW);
    let c = claim_at(&store, &mut p, on("C", "k2"), NOW);
    // Re-anchoring A to B's statement gives B's id.
    let out = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: a.to_string(),
            statement: Some("B".into()),
            ..ReanchorInput::default()
        },
    )
    .unwrap();
    assert_eq!(out.new, b);
    assert!(out.merged);
    assert_eq!(serde_json::to_value(&out).unwrap()["merged"], true);
    assert_eq!(store.counts().0, 3);

    // A is retired with a replacement: a write on it names B.
    let e = engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new(a.as_str())],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(e[0].code, Code::ClaimRetired);
    assert_eq!(
        e[0].remediation.as_deref(),
        Some(format!("The claim was replaced by {b}: run hibi status {b}").as_str())
    );
    let e = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: a.to_string(),
            statement: Some("Z".into()),
            ..ReanchorInput::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, Code::ClaimRetired);
    assert!(e.remediation.as_deref().unwrap().contains(b.as_str()));

    // A retired claim cannot be a replacement.
    let before = store.counts();
    let e = engine::retire(
        &store,
        &RetireInput {
            claims: vec![c.to_string()],
            replaced_by: Some(a.to_string()),
            ..RetireInput::default()
        },
        now(),
    )
    .unwrap_err();
    assert_eq!(e.code, Code::InvalidArgument);
    assert!(e.message.contains("is retired"), "{e}");
    assert_eq!(store.counts(), before);

    // Retired without a replacement: the remediation shows its record.
    retire(&store, &c);
    let e = engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new(c.as_str())],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(
        e[0].remediation.as_deref(),
        Some(format!("hibi list {c}").as_str())
    );

    // `no_change` and a malformed id say what to do.
    let e = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: b.to_string(),
            ..ReanchorInput::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, Code::NoChange);
    assert_eq!(
        e.remediation.as_deref(),
        Some(
            format!("The stored anchors are up to date. To record a new check, run hibi mark {b}")
                .as_str()
        )
    );
    let e = engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new("not-an-id")],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(e[0].code, Code::InvalidArgument);
    assert!(e[0].remediation.as_deref().unwrap().contains("hibi list"));
}

/// An anchor argument that starts with `{` is a JSON anchor, so that a
/// path with `#` or `::` can be given on the command line.
#[test]
fn an_anchor_argument_may_be_a_json_object() {
    let a = engine::compact_anchor(r#"{"loc":"file:docs/a#b::c.md","sel":{"quote":"x"}}"#)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(a.loc, "file:docs/a#b::c.md");
    assert_eq!(a.sel.quote.as_deref(), Some("x"));
    let a = engine::compact_anchor(r#" {"loc":"t:a","sel":{"key":"k1"}}"#)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(a.sel.key.as_deref(), Some("k1"));
    for bad in [
        r#"{"loc":"file:a.md"}"#,
        r#"{"loc":"file:a.md","sel":{"prefix":"p"}}"#,
        r#"{"loc":"nope","sel":{}}"#,
        r#"{"loc":"file:a.md","sel":{}} trailing"#,
        "{",
    ] {
        let e = engine::compact_anchor(bad).unwrap_err();
        assert_eq!(e.code, Code::InvalidAnchor, "{bad}");
        assert!(
            e.remediation.as_deref().unwrap().contains("JSON object"),
            "{bad}"
        );
    }
    // The compact form is unchanged.
    let a = engine::compact_anchor("file:a.md#sym::q")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        (a.sel.symbol.as_deref(), a.sel.quote.as_deref()),
        (Some("sym"), Some("q"))
    );
}

// ---------------------------------------------------------------------------
// Rules for new claims and new sides
// ---------------------------------------------------------------------------

/// A recheck period of zero would expire the claim at once: it is
/// `invalid_duration` in `claim`, in `reanchor` and when a stored claim is
/// read. `duration::parse` itself accepts it.
#[test]
fn a_zero_recheck_period_is_invalid() {
    let (_w, mut p, store) = setup();
    for zero in ["P0D", "PT0S", "P0Y0M0W0DT0H0M0S"] {
        assert!(hibi::duration::parse(zero).is_ok());
        let errors = claim(
            &store,
            &mut p,
            &[ClaimInput {
                recheck: Some(zero.into()),
                ..on("S", "k1")
            }],
            false,
        )
        .unwrap_err();
        assert_eq!(codes(&errors), [(Code::InvalidDuration, None)], "{zero}");
        assert!(
            errors[0].message.contains("longer than zero"),
            "{}",
            errors[0]
        );
    }
    assert_eq!(store.counts(), (0, 0, 0));
    let id = claim(&store, &mut p, &[on("S", "k1")], false)
        .unwrap()
        .claims[0]
        .id
        .clone();
    let e = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: id.to_string(),
            recheck: Some("PT0S".into()),
            ..ReanchorInput::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, Code::InvalidDuration);
    assert_eq!(store.counts(), (1, 1, 0));
    // On read: a stored record with a zero period does not parse.
    let stored = serde_json::to_value(&store.ledger().claims[0]).unwrap();
    let mut zero = stored.clone();
    zero["recheck"] = "P0D".into();
    assert!(serde_json::from_value::<hibi::model::Claim>(zero).is_err());
    let mut one = stored;
    one["recheck"] = "P1D".into();
    // (The id no longer matches the body, which is a different error.)
    let e = serde_json::from_value::<hibi::model::Claim>(one).unwrap_err();
    assert!(!e.to_string().contains("zero"), "{e}");
}

/// The same evidence anchor twice is `invalid_argument`, in `claim` and
/// for an added side in `reanchor`. The subject may also be evidence.
#[test]
fn duplicate_evidence_anchors_are_refused() {
    let (_w, mut p, store) = setup();
    let twice = ClaimInput {
        evidence: vec![
            json_anchor("t:a", key_sel("k2")),
            json_anchor("t:a", key_sel("k3")),
            json_anchor("t:a", key_sel("k2")),
        ],
        ..on("S", "k1")
    };
    let errors = claim(&store, &mut p, &[twice], false).unwrap_err();
    assert_eq!(codes(&errors), [(Code::InvalidArgument, None)]);
    assert!(
        errors[0]
            .message
            .contains("evidence 1 and evidence 3 are the same anchor"),
        "{}",
        errors[0]
    );
    assert_eq!(store.counts(), (0, 0, 0));

    // The subject as evidence is allowed.
    let own = ClaimInput {
        evidence: vec![json_anchor("t:a", key_sel("k1"))],
        ..on("S", "k1")
    };
    let id = claim(&store, &mut p, &[own], false).unwrap().claims[0]
        .id
        .clone();
    let e = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: id.to_string(),
            add_evidence: vec![json_anchor("t:a", key_sel("k1"))],
            ..ReanchorInput::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, Code::InvalidArgument);
    assert!(e.message.contains("evidence 1 and evidence 2"), "{e}");
    assert_eq!(store.counts(), (1, 1, 0));
}

/// A new anchor with a symbol and a quote: when the provider resolves the
/// symbol and completes the selector with another quote, the text asked
/// for is not in that unit and the write fails with `selector_not_exact`.
/// A stored anchor in the same situation keeps resolving by its symbol.
#[test]
fn a_new_anchor_whose_quote_is_not_in_the_resolved_unit_is_refused() {
    let w = file_world();
    let body = "fn delay_ms() -> u64 { 300 * 2 }";
    let unit_for = |quote: &str| Unit {
        sel: Selector {
            quote: Some(quote.into()),
            symbol: Some("delay_ms".into()),
            prefix: Some(String::new()),
            suffix: Some(String::new()),
            pos: Some(0),
            ..Selector::default()
        },
        ..unit("function:delay_ms", "function", body)
    };
    // Whatever quote is asked, this provider answers with the default
    // quote of the declaration, as the file provider does for a quote that
    // does not occur in it.
    set(
        &w,
        "file:r.rs",
        vec![unit_for("fn delay_ms() -> u64 { 300 * 2 }")],
    );
    let mut p = providers(&w, &["file"]);
    let store = MemStore::new();
    let asked = |quote: &str| ClaimInput {
        subject: json_anchor(
            "file:r.rs",
            Selector {
                quote: Some(quote.into()),
                symbol: Some("delay_ms".into()),
                ..Selector::default()
            },
        ),
        ..claim_input("S", "file:r.rs", &[])
    };
    w.borrow_mut().overrides.insert(
        hibi::model::Anchor::new(
            loc("file:r.rs"),
            Selector {
                quote: Some("300 * 7".into()),
                symbol: Some("delay_ms".into()),
                ..Selector::default()
            },
        ),
        Ok(vec![unit_for(body)]),
    );
    let errors = claim(&store, &mut p, &[asked("300 * 7")], false).unwrap_err();
    assert_eq!(codes(&errors), [(Code::SelectorNotExact, None)]);
    assert!(
        errors[0].message.contains("\"300 * 7\" does not occur"),
        "{}",
        errors[0]
    );
    assert!(errors[0].message.contains("delay_ms"), "{}", errors[0]);
    assert!(
        errors[0]
            .remediation
            .as_deref()
            .unwrap()
            .contains("leave the quote out"),
        "{:?}",
        errors[0].remediation
    );
    assert_eq!(store.counts(), (0, 0, 0));

    // A quote that differs from the completed one only in whitespace is
    // the same quote.
    w.borrow_mut().overrides.clear();
    w.borrow_mut().overrides.insert(
        hibi::model::Anchor::new(
            loc("file:r.rs"),
            Selector {
                quote: Some("fn delay_ms()   -> u64 {\n 300 * 2 }".into()),
                symbol: Some("delay_ms".into()),
                ..Selector::default()
            },
        ),
        Ok(vec![unit_for(body)]),
    );
    let ok = claim(
        &store,
        &mut p,
        &[asked("fn delay_ms()   -> u64 {\n 300 * 2 }")],
        false,
    )
    .unwrap();
    let id = ok.claims[0].id.clone();

    // A replaced side in `reanchor` is checked the same way; the kept
    // stored anchor is not.
    w.borrow_mut().overrides.insert(
        hibi::model::Anchor::new(
            loc("file:r.rs"),
            Selector {
                quote: Some("nowhere".into()),
                symbol: Some("delay_ms".into()),
                ..Selector::default()
            },
        ),
        Ok(vec![unit_for(body)]),
    );
    let e = reanchor(
        &store,
        &mut p,
        ReanchorInput {
            claim: id.to_string(),
            subject: Some(json_anchor(
                "file:r.rs",
                Selector {
                    quote: Some("nowhere".into()),
                    symbol: Some("delay_ms".into()),
                    ..Selector::default()
                },
            )),
            ..ReanchorInput::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, Code::SelectorNotExact);
    assert_eq!(store.counts(), (1, 1, 0));
}

/// `side_ambiguous` on a new anchor lists every candidate with its kind,
/// label and an anchor that selects it.
#[test]
fn an_ambiguous_new_anchor_lists_each_candidate_with_an_anchor() {
    let w = file_world();
    let def = |pos: u64, text: &str| Unit {
        sel: Selector {
            quote: Some(text.into()),
            symbol: Some("max_size".into()),
            prefix: Some(String::new()),
            suffix: Some(String::new()),
            pos: Some(pos),
            ..Selector::default()
        },
        ..at(
            unit(&format!("function:max_size@{pos}"), "function", text),
            pos as usize,
            pos as usize + 1,
        )
    };
    set(
        &w,
        "file:dup.py",
        vec![
            def(0, "def max_size(): return 1"),
            def(30, "def max_size(): return 2"),
        ],
    );
    let mut p = providers(&w, &["file"]);
    let store = MemStore::new();
    let input = ClaimInput {
        subject: json_anchor("file:dup.py", symbol_sel("max_size")),
        ..claim_input("S", "file:dup.py", &[])
    };
    w.borrow_mut().overrides.insert(
        hibi::model::Anchor::new(loc("file:dup.py"), symbol_sel("max_size")),
        Ok(vec![
            def(0, "def max_size(): return 1"),
            def(30, "def max_size(): return 2"),
        ]),
    );
    let errors = claim(&store, &mut p, &[input], false).unwrap_err();
    assert_eq!(codes(&errors), [(Code::SideAmbiguous, None)]);
    let m = &errors[0].message;
    assert!(m.contains("matches 2 units"), "{m}");
    // The same symbol twice: each candidate comes with its full anchor.
    for (line, pos, text) in [(1, 0, "return 1"), (31, 30, "return 2")] {
        assert!(
            m.contains(&format!(
                "function \"def max_size(): {text}\", line {line}: "
            )),
            "{m}"
        );
        assert!(m.contains(&format!("\"pos\":{pos}")), "{m}");
    }
    assert!(
        m.contains(r#"{"loc":"file:dup.py","sel":{"quote":"def max_size(): return 1""#),
        "{m}"
    );
    assert_eq!(store.counts(), (0, 0, 0));
}

// ---------------------------------------------------------------------------
// claim on a claim whose stored context went out of date
// ---------------------------------------------------------------------------

/// A sentence unit whose completed selector is a quote with context, as
/// the file provider gives it.
fn quoted(key: &str, text: &str, quote: &str, prefix: &str, pos: u64) -> Unit {
    Unit {
        sel: Selector {
            quote: Some(quote.into()),
            prefix: Some(prefix.into()),
            suffix: Some(String::new()),
            pos: Some(pos),
            ..Selector::default()
        },
        ..unit(key, "sentence", text)
    }
}

fn by_quote(statement: &str, quote: &str) -> ClaimInput {
    ClaimInput {
        subject: json_anchor("file:d.md", Selector::quote(quote)),
        ..claim_input(statement, "file:d.md", &[])
    }
}

/// The same `claim` command after text near the quote was edited marks the
/// existing claim: the completed context and position differ, the
/// statement, the unit and the quote do not.
#[test]
fn claim_again_after_an_edit_near_the_quote_marks_the_existing_claim() {
    let w = file_world();
    let text = "Retries are capped at 5.";
    set(
        &w,
        "file:d.md",
        vec![
            quoted("s1", text, "capped at 5", "Intro.\n\nRetries are ", 20),
            quoted("s2", "Timeouts are 30 seconds.", "30 seconds", "", 60),
        ],
    );
    let mut p = providers(&w, &["file"]);
    let store = MemStore::new();
    let first = claim(&store, &mut p, &[by_quote("S", "capped at 5")], false).unwrap();
    let id = first.claims[0].id.clone();
    assert!(first.claims[0].created);

    // The line above the sentence is deleted: same unit, same quote, new
    // context and position.
    let moved = quoted("s1", text, "capped at 5", "Retries are ", 12);
    set(
        &w,
        "file:d.md",
        vec![
            moved.clone(),
            quoted("s2", "Timeouts are 30 seconds.", "30 seconds", "", 52),
        ],
    );
    let later = ts("2026-12-01T00:00:00Z");
    let again = engine::claim(
        &store,
        &mut p,
        &no_git(),
        &[by_quote("S", "capped at 5")],
        false,
        later,
    )
    .unwrap();
    assert_eq!(again.claims[0].id, id);
    assert!(!again.claims[0].created);
    assert_eq!(again.claims[0].claim.state, ClaimState::Fresh);
    assert_eq!(store.counts(), (1, 2, 0));
    let ledger = store.ledger();
    let mark = ledger
        .marks
        .iter()
        .find(|m| m.id == again.claims[0].mark)
        .unwrap();
    assert_eq!((&mark.claim, mark.at), (&id, later));
    assert_eq!(mark.sides[0].fp, moved.fp);
    // The stored anchor is unchanged: `reanchor` refreshes it.
    assert_eq!(
        ledger.claims[0].subject.sel.prefix.as_deref(),
        Some("Intro.\n\nRetries are ")
    );

    // Two elements that are the same existing claim: one mark, two results.
    let twice = engine::claim(
        &store,
        &mut p,
        &no_git(),
        &[by_quote("S", "capped at 5"), by_quote(" S ", "capped at 5")],
        true,
        ts("2026-12-02T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(
        twice
            .claims
            .iter()
            .map(|c| (&c.id, c.created))
            .collect::<Vec<_>>(),
        [(&id, false), (&id, false)]
    );
    assert_eq!(twice.claims[0].mark, twice.claims[1].mark);
    assert_eq!(store.counts(), (1, 3, 0));

    // Not the same claim: another statement, another note, another
    // recheck, another unit, another quote in the same unit.
    let mut other = |input: ClaimInput| {
        let out = claim(&store, &mut p, &[input], false).unwrap();
        assert!(out.claims[0].created);
        assert_ne!(out.claims[0].id, id);
    };
    other(by_quote("T", "capped at 5"));
    other(ClaimInput {
        note: Some("n".into()),
        ..by_quote("S", "capped at 5")
    });
    other(ClaimInput {
        recheck: Some("P1D".into()),
        ..by_quote("S", "capped at 5")
    });
    other(by_quote("S", "30 seconds"));
    assert_eq!(store.counts().0, 5);
}

/// The existing claim's stored anchor must resolve to the same unit as the
/// new anchor; and among several claims that are the same, the smallest id
/// is marked. A retired claim is never the match.
#[test]
fn claim_matches_by_resolved_unit_and_prefers_the_smallest_id() {
    let w = file_world();
    let text = "Retries are capped at 5.";
    let unit_with = |prefix: &str, pos: u64| quoted("s1", text, "capped at 5", prefix, pos);
    let store = MemStore::new();
    let mut p = providers(&w, &["file"]);
    let mut ids = Vec::new();
    // Three claims that differ only in stored context, as older versions
    // could leave behind.
    for (prefix, pos) in [("a ", 1), ("bb ", 2), ("ccc ", 3)] {
        set(&w, "file:d.md", vec![unit_with(prefix, pos)]);
        let out = engine::claim(
            &MemStore::new(),
            &mut p,
            &no_git(),
            &[by_quote("S", "capped at 5")],
            false,
            now(),
        )
        .unwrap();
        let mut batch = hibi::store::Batch::default();
        let c = &out.claims[0].claim;
        batch.claims.push(
            hibi::model::Claim::new(&c.statement, c.sides[0].anchor.clone(), vec![], None, None)
                .unwrap(),
        );
        ids.push(batch.claims[0].id.clone());
        store.insert(batch);
    }
    ids.sort();
    assert_eq!(store.counts(), (3, 0, 0));

    set(&w, "file:d.md", vec![unit_with("dddd ", 4)]);
    let out = claim(&store, &mut p, &[by_quote("S", "capped at 5")], false).unwrap();
    assert_eq!((&out.claims[0].id, out.claims[0].created), (&ids[0], false));
    assert_eq!(store.counts(), (3, 1, 0));

    // With the smallest retired, the next one is marked.
    retire(&store, &ids[0]);
    let out = claim(&store, &mut p, &[by_quote("S", "capped at 5")], false).unwrap();
    assert_eq!((&out.claims[0].id, out.claims[0].created), (&ids[1], false));

    // A stored anchor that now resolves to another unit is not the same
    // claim, and one that resolves to no unit is not either.
    let ledger = store.ledger();
    for c in ledger.claims.iter().filter(|c| c.id != ids[0]) {
        let answer = if c.id == ids[1] {
            Ok(vec![quoted(
                "s9",
                "Elsewhere capped at 5.",
                "capped at 5",
                "",
                0,
            )])
        } else {
            Ok(vec![])
        };
        w.borrow_mut().overrides.insert(c.subject.clone(), answer);
    }
    let out = claim(&store, &mut p, &[by_quote("S", "capped at 5")], false).unwrap();
    assert!(out.claims[0].created);
    assert!(!ids.contains(&out.claims[0].id));
    assert_eq!(store.counts().0, 4);
}

// ---------------------------------------------------------------------------
// retire
// ---------------------------------------------------------------------------

/// A claim named twice, by its id and by a prefix, is retired once: one
/// output entry per claim, in the order of first occurrence.
#[test]
fn retire_collapses_a_claim_named_twice() {
    let (_w, mut p, store) = setup();
    let a = claim_at(&store, &mut p, on("A", "k1"), NOW);
    let b = claim_at(&store, &mut p, on("B", "k2"), NOW);
    let out = engine::retire(
        &store,
        &RetireInput {
            claims: vec![
                b.to_string(),
                a.to_string(),
                b.as_str()[..8].to_owned(),
                a.to_string(),
            ],
            ..RetireInput::default()
        },
        ts("2026-12-01T00:00:00Z"),
    )
    .unwrap();
    let ledger = store.ledger();
    assert_eq!(ledger.retirements.len(), 2);
    let of = |id: &ClaimId| {
        ledger
            .retirements
            .iter()
            .find(|r| &r.retires == id)
            .unwrap()
            .id
            .clone()
    };
    assert_eq!(
        serde_json::to_value(&out.retired).unwrap(),
        serde_json::json!([
            {"claim": b, "retirement": of(&b), "alreadyRetired": false},
            {"claim": a, "retirement": of(&a), "alreadyRetired": false}
        ])
    );

    // The same for a claim that is already retired.
    let out = engine::retire(
        &store,
        &RetireInput {
            claims: vec![a.to_string(), a.to_string()],
            ..RetireInput::default()
        },
        ts("2026-12-02T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&out.retired).unwrap(),
        serde_json::json!([{"claim": a, "retirement": null, "alreadyRetired": true}])
    );
    assert_eq!(store.ledger().retirements.len(), 2);
}

#[test]
fn retire_appends_one_retirement_per_live_claim_and_contacts_no_provider() {
    let (w, mut p, store) = setup();
    let a = claim_at(&store, &mut p, on("A", "k1"), NOW);
    let b = claim_at(&store, &mut p, on("B", "k2"), NOW);
    let c = claim_at(&store, &mut p, on("C", "k3"), NOW);
    retire(&store, &b);
    w.borrow_mut().log.clear();
    let appends = store.appends.get();

    let out = engine::retire(
        &store,
        &RetireInput {
            claims: vec![a.as_str()[..8].to_owned(), b.to_string()],
            replaced_by: Some(c.to_string()),
            note: Some(" merged into C ".into()),
            by: Some("agent".into()),
        },
        ts("2026-12-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(store.appends.get(), appends + 1);
    assert_eq!(store.counts(), (3, 3, 2));
    let ledger = store.ledger();
    let r = ledger.retirements.iter().find(|r| r.retires == a).unwrap();
    assert_eq!(r.replaced_by.as_ref(), Some(&c));
    assert_eq!(r.note.as_deref(), Some("merged into C"));
    assert_eq!(r.by.as_deref(), Some("agent"));
    assert_eq!(r.at, ts("2026-12-01T00:00:00Z"));
    assert_eq!(
        serde_json::to_value(&out).unwrap(),
        serde_json::json!({
            "retired": [
                {"claim": a, "retirement": r.id, "alreadyRetired": false},
                {"claim": b, "retirement": null, "alreadyRetired": true}
            ],
            "warnings": []
        })
    );
    assert!(log(&w).is_empty());

    // Nothing to write: no append.
    let out = engine::retire(
        &store,
        &RetireInput {
            claims: vec![a.to_string()],
            ..RetireInput::default()
        },
        now(),
    )
    .unwrap();
    assert!(out.retired[0].already_retired);
    assert_eq!(store.appends.get(), appends + 1);

    let err = |input: RetireInput| engine::retire(&store, &input, now()).unwrap_err().code;
    assert_eq!(
        err(RetireInput {
            claims: vec![c.to_string()],
            replaced_by: Some(c.to_string()),
            ..RetireInput::default()
        }),
        Code::InvalidArgument
    );
    assert_eq!(
        err(RetireInput {
            claims: vec![c.to_string()],
            replaced_by: Some("c1-zzzz".into()),
            ..RetireInput::default()
        }),
        Code::InvalidArgument
    );
    assert_eq!(
        err(RetireInput {
            claims: vec!["c1-zzzz".into()],
            ..RetireInput::default()
        }),
        Code::ClaimNotFound
    );
    assert_eq!(store.counts(), (3, 3, 2));
}
