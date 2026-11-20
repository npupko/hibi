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
            json_anchor("t:a", key_sel("k2")),
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
            anchor("t:a", key_sel("k2")),
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
