//! Claim states (spec section 9.3), the reference mark, and the `status`
//! and `list` selections of section 15.3 (R13, R14, R16, R18).

use hibi::Code;
use hibi::engine::{
    self, ClaimState, ListOutput, ListQuery, MarkInput, RetireInput, StatusQuery, Warning,
    WarningCode,
};
use hibi::model::ClaimId;
use hibi::provider::Providers;

use crate::support::*;

fn setup() -> (Shared, Providers, MemStore) {
    let w = key_world();
    set(
        &w,
        "t:a",
        vec![
            unit("k1", "note", "retries are capped at 5"),
            unit("k2", "note", "two"),
        ],
    );
    set(&w, "u:b", vec![unit("k1", "note", "evidence")]);
    let p = providers(&w, &["t", "u"]);
    (w, p, MemStore::new())
}

fn one(store: &MemStore, p: &mut Providers, recheck: Option<&str>, at: &str) -> ClaimId {
    let input = engine::ClaimInput {
        subject: json_anchor("t:a", key_sel("k1")),
        evidence: vec![json_anchor("u:b", key_sel("k1"))],
        recheck: recheck.map(str::to_owned),
        ..claim_input("Retries are capped.", "t:a", &[])
    };
    claim_at(store, p, input, at)
}

fn mark_at(store: &MemStore, p: &mut Providers, id: &ClaimId, at: &str) -> String {
    let out = engine::mark(
        store,
        p,
        &no_git(),
        &[MarkInput::new(id.as_str())],
        false,
        ts(at),
    )
    .unwrap();
    out.marks[0].mark.to_string()
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

/// R13 and R14: changing the content makes the claim stale with that side
/// `changed`; restoring it makes the claim fresh without a new mark. Any
/// matching mark is enough, not only the newest.
#[test]
fn r13_r14_some_mark_not_the_newest() {
    let (w, mut p, store) = setup();
    let id = one(&store, &mut p, None, "2026-01-01T00:00:00Z");
    let first = view(&store, &mut p, &id).mark.unwrap();

    edit(&w, "t:a", "k1", "retries are capped at 7");
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Stale);
    assert_eq!(v.sides[0].state, engine::SideState::Changed);
    assert_eq!(v.sides[1].state, engine::SideState::Same);
    assert_eq!(v.mark, Some(first.clone()));

    // A newer mark covers the edited content.
    let second = mark_at(&store, &mut p, &id, "2026-02-01T00:00:00Z");
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Fresh);
    assert_eq!(v.mark.as_ref().unwrap().as_str(), second);

    // Back to the content the older mark covers: fresh, no new mark, and
    // the reference mark is the matching one, not the newest.
    edit(&w, "t:a", "k1", "retries are capped at 5");
    let marks_before = store.counts().1;
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Fresh);
    assert_eq!(v.mark, Some(first));
    assert_eq!(v.marked_at, Some(ts("2026-01-01T00:00:00Z")));
    assert_eq!(store.counts().1, marks_before);
}

/// R18: fresh one second before `add(mark.at, recheck)`, expired at that
/// instant.
#[test]
fn r18_recheck_boundary_is_inclusive() {
    let (_w, mut p, store) = setup();
    let id = one(&store, &mut p, Some("P1M"), "2026-01-31T10:00:00Z");
    // One month after Jan 31 is Feb 28 (section 9.4).
    let v = view_at(&store, &mut p, &id, "2026-02-28T09:59:59Z");
    assert_eq!(v.state, ClaimState::Fresh);
    assert_eq!(v.expires_at, Some(ts("2026-02-28T10:00:00Z")));
    assert_eq!(v.next, None);
    let v = view_at(&store, &mut p, &id, "2026-02-28T10:00:00Z");
    assert_eq!(v.state, ClaimState::Expired);
    assert_eq!(v.expires_at, Some(ts("2026-02-28T10:00:00Z")));
    assert!(v.sides.iter().all(|s| s.state == engine::SideState::Same));
    assert_eq!(v.next, Some(format!("hibi mark {id}")));
}

/// An expiry past the last representable instant (section 9.4) is never
/// reached: the mark keeps the claim fresh and `expiresAt` is `null`.
#[test]
fn an_expiry_past_the_last_instant_is_never_reached() {
    let (_w, mut p, store) = setup();
    let id = one(&store, &mut p, Some("P1D"), "9999-12-30T00:00:00Z");
    // The first mark expires at 9999-12-31T00:00:00Z.
    let v = view_at(&store, &mut p, &id, "9999-12-31T12:00:00Z");
    assert_eq!(v.state, ClaimState::Expired);
    assert_eq!(v.expires_at, Some(ts("9999-12-31T00:00:00Z")));
    // A mark whose `at` plus one day is past 9999-12-31T23:59:59Z.
    mark_at(&store, &mut p, &id, "9999-12-31T12:00:00Z");
    let v = view_at(&store, &mut p, &id, "9999-12-31T23:59:59Z");
    assert_eq!(v.state, ClaimState::Fresh);
    assert_eq!(v.marked_at, Some(ts("9999-12-31T12:00:00Z")));
    assert_eq!(v.expires_at, None);
    assert_eq!(v.next, None);
}

/// Rule 4 reads "some matching mark": an older matching mark that has
/// expired does not expire the claim while a newer matching one is live.
#[test]
fn expiry_uses_any_matching_mark() {
    let (_w, mut p, store) = setup();
    let id = one(&store, &mut p, Some("P10D"), "2026-01-01T00:00:00Z");
    mark_at(&store, &mut p, &id, "2026-01-08T00:00:00Z");
    let v = view_at(&store, &mut p, &id, "2026-01-12T00:00:00Z");
    assert_eq!(v.state, ClaimState::Fresh);
    assert_eq!(v.marked_at, Some(ts("2026-01-08T00:00:00Z")));
    assert_eq!(v.expires_at, Some(ts("2026-01-18T00:00:00Z")));
    let v = view_at(&store, &mut p, &id, "2026-01-18T00:00:00Z");
    assert_eq!(v.state, ClaimState::Expired);
}

#[test]
fn stale_wins_over_expired_and_has_no_expiry() {
    let (w, mut p, store) = setup();
    let id = one(&store, &mut p, Some("P1D"), "2026-01-01T00:00:00Z");
    edit(&w, "t:a", "k1", "changed");
    let v = view_at(&store, &mut p, &id, "2027-01-01T00:00:00Z");
    assert_eq!(v.state, ClaimState::Stale);
    assert_eq!(v.expires_at, None);
}

/// Sides of a retired claim are not resolved; its view has no sides.
#[test]
fn a_retired_claim_is_not_resolved() {
    let (w, mut p, store) = setup();
    let id = one(&store, &mut p, None, NOW);
    retire(&store, &id);
    w.borrow_mut().log.clear();
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Retired);
    assert!(v.sides.is_empty());
    assert_eq!(v.next, None);
    assert!(v.mark.is_some());
    assert!(log(&w).is_empty(), "{:?}", log(&w));
}

#[test]
fn status_selection_summary_and_order() {
    let (w, mut p, store) = setup();
    let fresh = one(&store, &mut p, None, "2026-01-01T00:00:00Z");
    let on_k2 = engine::ClaimInput {
        subject: json_anchor("t:a", key_sel("k2")),
        ..claim_input("Two.", "t:a", &[])
    };
    let stale = claim_at(&store, &mut p, on_k2, "2026-01-01T00:00:00Z");
    let on_u = engine::ClaimInput {
        subject: json_anchor("u:b", key_sel("k1")),
        recheck: Some("P1D".into()),
        ..claim_input("Evidence.", "u:b", &[])
    };
    let expired = claim_at(&store, &mut p, on_u, "2026-01-01T00:00:00Z");
    let gone_input = engine::ClaimInput {
        subject: json_anchor("u:b", key_sel("k1")),
        ..claim_input("Gone.", "u:b", &[])
    };
    let gone = claim_at(&store, &mut p, gone_input, "2026-01-01T00:00:00Z");
    retire(&store, &gone);
    edit(&w, "t:a", "k2", "two, edited");

    let run = |p: &mut Providers, q: StatusQuery| {
        engine::status(&store, p, &no_git(), &q, None, now()).unwrap()
    };
    let ids = |out: &engine::StatusOutput| -> Vec<ClaimId> {
        out.claims.iter().map(|c| c.id.clone()).collect()
    };
    let sorted = |mut v: Vec<ClaimId>| {
        v.sort();
        v
    };

    // Default: stale and expired, ordered by id.
    let out = run(&mut p, StatusQuery::default());
    assert_eq!(ids(&out), sorted(vec![stale.clone(), expired.clone()]));
    assert_eq!(
        serde_json::to_value(out.summary).unwrap(),
        serde_json::json!({"claims": 4, "fresh": 1, "drifted": 0, "stale": 1, "expired": 1, "retired": 1})
    );
    assert_eq!(out.since, None);
    assert_eq!(out.now, now());
    assert_eq!(out.snapshot, "mem:4:4:1");
    assert_eq!(
        out.next_note,
        "Re-check each statement before running its next command. When a statement no longer holds, do not run it: fix the text or the code, or retire the claim with hibi retire. A fresh claim is listed only because its anchor drifted: the statement still holds, so look at what the anchor points to now, then run its reanchor command."
    );

    // --state.
    let out = run(
        &mut p,
        StatusQuery {
            states: vec![ClaimState::Fresh, ClaimState::Retired],
            ..Default::default()
        },
    );
    assert_eq!(ids(&out), sorted(vec![fresh.clone(), gone.clone()]));
    assert_eq!(out.summary.claims, 4);

    // --loc matches the whole locator of any side.
    let out = run(
        &mut p,
        StatusQuery {
            locs: vec!["u:*".into()],
            ..Default::default()
        },
    );
    assert_eq!(ids(&out), vec![expired.clone()]);
    let out = run(
        &mut p,
        StatusQuery {
            locs: vec!["t:a".into()],
            states: ClaimState::ALL.to_vec(),
            ..Default::default()
        },
    );
    assert_eq!(ids(&out), sorted(vec![fresh.clone(), stale.clone()]));

    // Ids: exactly those claims in any state, by prefix, ordered by id.
    let out = run(
        &mut p,
        StatusQuery {
            ids: vec![gone.to_string(), fresh.as_str()[..9].to_owned()],
            ..Default::default()
        },
    );
    assert_eq!(ids(&out), sorted(vec![fresh.clone(), gone.clone()]));

    // --detail carries the unit text.
    let out = run(
        &mut p,
        StatusQuery {
            ids: vec![stale.to_string()],
            detail: true,
            ..Default::default()
        },
    );
    assert_eq!(out.claims[0].sides[0].text.as_deref(), Some("two, edited"));

    // Errors.
    let err = |q: StatusQuery| {
        let mut p = providers(&w, &["t", "u"]);
        engine::status(&store, &mut p, &no_git(), &q, None, now())
            .unwrap_err()
            .code
    };
    assert_eq!(
        err(StatusQuery {
            ids: vec!["c1-zzzz".into()],
            ..Default::default()
        }),
        Code::ClaimNotFound
    );
    assert_eq!(
        err(StatusQuery {
            ids: vec!["nope".into()],
            ..Default::default()
        }),
        Code::InvalidArgument
    );
    assert_eq!(
        err(StatusQuery {
            locs: vec!["t:[".into()],
            ..Default::default()
        }),
        Code::InvalidArgument
    );
}

#[test]
fn status_json_shape() {
    let (w, mut p, store) = setup();
    let id = one(&store, &mut p, Some("P90D"), "2026-11-01T00:00:00Z");
    edit(&w, "t:a", "k1", "new text");
    let out = engine::status(
        &store,
        &mut p,
        &no_git(),
        &StatusQuery::default(),
        None,
        now(),
    )
    .unwrap();
    let json = serde_json::to_value(engine::Envelope::new("status", &out)).unwrap();
    let mark = out.claims[0].mark.clone().unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "ok": true, "command": "status", "now": NOW, "snapshot": "mem:1:1:0", "since": null,
            "summary": {"claims": 1, "fresh": 0, "drifted": 0, "stale": 1, "expired": 0, "retired": 0},
            "claims": [{
                "id": id, "statement": "Retries are capped.", "recheck": "P90D", "note": null,
                "state": "stale", "mark": mark, "markedAt": "2026-11-01T00:00:00Z",
                "expiresAt": null,
                "sides": [
                    {"index": 0, "role": "subject",
                     "anchor": {"loc": "t:a", "sel": {"key": "k1"}},
                     "state": "changed", "reason": "fingerprint_differs", "approx": false,
                     "unit": {"key": "k1", "kind": "note", "label": "new text",
                              "line": null, "endLine": null},
                     "fp": hibi::fingerprint::prose_v1("new text"),
                     "markedFp": hibi::fingerprint::prose_v1("retries are capped at 5"),
                     "remediation": null},
                    {"index": 1, "role": "evidence",
                     "anchor": {"loc": "u:b", "sel": {"key": "k1"}},
                     "state": "same", "reason": null, "approx": false,
                     "unit": {"key": "k1", "kind": "note", "label": "evidence",
                              "line": null, "endLine": null},
                     "fp": hibi::fingerprint::prose_v1("evidence"),
                     "markedFp": hibi::fingerprint::prose_v1("evidence"),
                     "remediation": null}
                ],
                "next": format!("hibi mark {id}")
            }],
            "nextNote": engine::NEXT_NOTE,
            "warnings": []
        })
    );
}

#[test]
fn warnings_are_converted_and_git_too_old_is_added_once_for_repo_sides() {
    let w = file_world();
    set(&w, "repo:api//a.md", vec![sym_unit("s", "sentence", "x")]);
    set(&w, "file:a.md", vec![sym_unit("s", "sentence", "x")]);
    let mut p = providers(&w, &["file", "repo"]);
    let store = MemStore::new();
    let too_old = hibi::git::GitStatus::from_version_output("git", "git version 2.30.1");
    let inputs = [
        claim_input("A", "repo:api//a.md#s", &["repo:api//a.md#s"]),
        claim_input("B", "file:a.md#s", &[]),
    ];
    let out = engine::claim(&store, &mut p, &too_old, &inputs, true, now()).unwrap();
    assert_eq!(out.warnings.len(), 1);
    assert_eq!(out.warnings[0].code, WarningCode::GitTooOld);
    assert!(out.warnings[0].message.contains("2.30.1"));
    assert!(out.warnings[0].message.contains("2.45.0"));

    store.warn("orphan_record", "mark m1 names a claim that is not stored");
    let q = StatusQuery::default();
    let out = engine::status(&store, &mut p, &too_old, &q, None, now()).unwrap();
    let codes: Vec<WarningCode> = out.warnings.iter().map(|w| w.code).collect();
    assert_eq!(codes, [WarningCode::OrphanRecord, WarningCode::GitTooOld]);
    assert_eq!(
        serde_json::to_value(&out.warnings[0]).unwrap(),
        serde_json::json!({"code": "orphan_record",
                           "message": "mark m1 names a claim that is not stored"})
    );

    // No repo: side in the run, no git warning.
    let only_file = [claim_input("C", "file:a.md#s", &[])];
    let store2 = MemStore::new();
    let out = engine::claim(&store2, &mut p, &too_old, &only_file, false, now()).unwrap();
    assert!(out.warnings.is_empty());

    // A code outside the closed list breaks the Store contract.
    store.warn("surprise", "x");
    let e = engine::status(&store, &mut p, &too_old, &q, None, now()).unwrap_err();
    assert_eq!(e.code, Code::Internal);
    assert!(Warning::git_too_old(&no_git()).is_none());
}

#[test]
fn list_reads_the_store_only() {
    let (w, mut p, store) = setup();
    let a = one(&store, &mut p, Some("P90D"), "2026-01-01T00:00:00Z");
    mark_at(&store, &mut p, &a, "2026-03-01T00:00:00Z");
    let on_k2 = engine::ClaimInput {
        subject: json_anchor("t:a", key_sel("k2")),
        note: Some("see the config".into()),
        ..claim_input("Two is two.", "t:a", &[])
    };
    let b = claim_at(&store, &mut p, on_k2, "2026-02-01T00:00:00Z");
    retire(&store, &b);
    w.borrow_mut().log.clear();

    let claims = |q: ListQuery| match engine::list(&store, &q).unwrap() {
        ListOutput::Claims { claims, .. } => claims,
        other => panic!("{other:?}"),
    };
    let live = claims(ListQuery::default());
    assert_eq!(live.len(), 1);
    assert_eq!(
        serde_json::to_value(&live[0]).unwrap(),
        serde_json::json!({
            "id": a, "statement": "Retries are capped.", "subject": "t:a", "evidence": ["u:b"],
            "recheck": "P90D", "note": null, "marks": 2,
            "lastMarkedAt": "2026-03-01T00:00:00Z", "retired": false
        })
    );
    let all = claims(ListQuery {
        retired: true,
        ..Default::default()
    });
    assert_eq!(all.len(), 2);
    assert!(all.windows(2).all(|p| p[0].id < p[1].id));
    assert!(all.iter().any(|c| c.id == b && c.retired));
    assert_eq!(
        claims(ListQuery {
            retired: true,
            text: Some("is two".into()),
            ..Default::default()
        })
        .len(),
        1
    );
    assert!(
        claims(ListQuery {
            text: Some("retries".into()),
            ..Default::default()
        })
        .is_empty(),
        "--text is case-sensitive"
    );
    assert_eq!(
        claims(ListQuery {
            locs: vec!["u:*".into()],
            retired: true,
            ..Default::default()
        })
        .len(),
        1
    );

    // With an id: the stored records, marks by `at`.
    let out = engine::list(
        &store,
        &ListQuery {
            id: Some(a.as_str()[..8].to_owned()),
            ..Default::default()
        },
    )
    .unwrap();
    let json = serde_json::to_value(&out).unwrap();
    let ledger = store.ledger();
    assert_eq!(
        json["claim"],
        serde_json::to_value(ledger.claim(a.as_str())).unwrap()
    );
    let ats: Vec<&str> = json["marks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["at"].as_str().unwrap())
        .collect();
    assert_eq!(ats, ["2026-01-01T00:00:00Z", "2026-03-01T00:00:00Z"]);
    assert_eq!(json["retirements"], serde_json::json!([]));
    let out = engine::list(
        &store,
        &ListQuery {
            id: Some(b.to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&out).unwrap()["retirements"][0]["retires"],
        b.as_str()
    );

    assert!(
        log(&w).is_empty(),
        "list contacted a provider: {:?}",
        log(&w)
    );
}

/// R16: no engine source consults a similarity score or an edit distance.
#[test]
fn r16_no_similarity_in_any_state_computation() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/engine");
    let mut files = vec![dir.with_extension("rs")];
    for e in std::fs::read_dir(&dir).unwrap() {
        files.push(e.unwrap().path());
    }
    assert!(files.len() >= 8);
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for word in ["similarity(", "levenshtein", "crate::text", "crate::locate"] {
            assert!(!text.contains(word), "{} uses {word}", f.display());
        }
    }
}
