//! The engine against the reference provider through `CommandProvider`:
//! R51 (`resource_not_found` and `selector_not_found` through the wire),
//! coverage through the `parent` chain of a real provider, and the cycle
//! stop of R50.

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hibi::config::{RepoConfig, UserConfig};
use hibi::engine::{
    self, AnchorInput, ClaimInput, ClaimState, CoverEnv, CoverQuery, Reason, SideState, StatusQuery,
};
use hibi::git::GitStatus;
use hibi::model::{Anchor, Claim, Locator, Selector, Timestamp};
use hibi::provider::Providers;
use hibi::provider::command::CommandProvider;
use hibi::store::{Batch, Ledger, Store};

const BIN: &str = env!("CARGO_BIN_EXE_hibi-provider-reference");

/// A `Store` in memory.
#[derive(Default)]
struct MemStore(RefCell<Ledger>);

impl Store for MemStore {
    fn load(&self) -> Result<Ledger, hibi::Error> {
        Ok(self.0.borrow().clone())
    }
    fn append(&self, batch: &Batch) -> Result<(), hibi::Error> {
        batch.verify_ids()?;
        let mut l = self.0.borrow_mut();
        batch.verify_claims(l.claims.iter().map(|c| &c.id))?;
        for c in &batch.claims {
            if !l.claims.iter().any(|x| x.id == c.id) {
                l.claims.push(c.clone());
            }
        }
        l.marks.extend(batch.marks.iter().cloned());
        l.retirements.extend(batch.retirements.iter().cloned());
        l.claims.sort_by(|a, b| a.id.cmp(&b.id));
        l.marks.sort_by(|a, b| a.id.cmp(&b.id));
        l.marks.dedup_by(|a, b| a.id == b.id);
        l.retirements.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(())
    }
    fn snapshot_id(&self) -> Result<String, hibi::Error> {
        Ok(format!("mem:{}", self.0.borrow().marks.len()))
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cases/provider/fixtures")
}

fn providers(fixture: &str) -> Providers {
    let command = [
        OsString::from(BIN),
        "--fixture".into(),
        fixtures().join(fixture).into(),
    ];
    let provider = CommandProvider::new(
        "ref",
        command,
        &std::env::temp_dir(),
        Duration::from_secs(10),
    );
    let mut p = Providers::new();
    p.register(&["ref"], Box::new(provider));
    p
}

fn now() -> Timestamp {
    Timestamp::parse("2026-11-20T18:04:11Z").unwrap()
}

fn no_git() -> GitStatus {
    GitStatus::Missing {
        message: "not used".into(),
    }
}

fn key(k: &str) -> Selector {
    Selector {
        key: Some(k.into()),
        ..Selector::default()
    }
}

fn input(statement: &str, loc: &str, sel: Selector) -> ClaimInput {
    ClaimInput {
        statement: statement.into(),
        subject: AnchorInput::Anchor(Anchor::new(Locator::new(loc).unwrap(), sel)),
        evidence: vec![],
        recheck: None,
        note: None,
        by: None,
    }
}

/// R51 through the wire, at the engine: `resolve` on an absent resource is
/// `missing` with `resource_not_found`; on an existing resource with a
/// selector that finds nothing it is `missing` with `selector_not_found`.
#[test]
fn r51_side_reasons_through_the_wire() {
    let store = MemStore::default();
    let mut p = providers("notes.json");
    // A claim made through the engine: the quote selector is completed to
    // the unit's key.
    let out = engine::claim(
        &store,
        &mut p,
        &no_git(),
        &[input(
            "The cap is configurable.",
            "ref:notes",
            Selector::quote("configurable"),
        )],
        false,
        now(),
    )
    .unwrap();
    assert_eq!(out.claims[0].claim.sides[0].anchor.sel, key("n2"));
    // Two stored claims whose anchors no longer resolve.
    let absent = Claim::new(
        "absent",
        Anchor::new(Locator::new("ref:absent").unwrap(), key("n1")),
        vec![],
        None,
        None,
    )
    .unwrap();
    let nothing = Claim::new(
        "nothing",
        Anchor::new(Locator::new("ref:notes").unwrap(), key("n9")),
        vec![],
        None,
        None,
    )
    .unwrap();
    store
        .append(&Batch {
            claims: vec![absent.clone(), nothing.clone()],
            ..Batch::default()
        })
        .unwrap();

    let query = StatusQuery {
        states: ClaimState::ALL.to_vec(),
        ..StatusQuery::default()
    };
    let status = engine::status(&store, &mut p, &no_git(), &query, None, now()).unwrap();
    let side = |id: &hibi::model::ClaimId| {
        let c = status.claims.iter().find(|c| &c.id == id).unwrap();
        (c.state, c.sides[0].state, c.sides[0].reason)
    };
    assert_eq!(
        side(&absent.id),
        (
            ClaimState::Stale,
            SideState::Missing,
            Some(Reason::ResourceNotFound)
        )
    );
    assert_eq!(
        side(&nothing.id),
        (
            ClaimState::Stale,
            SideState::Missing,
            Some(Reason::SelectorNotFound)
        )
    );
    assert_eq!(
        side(&out.claims[0].id),
        (ClaimState::Fresh, SideState::Same, None)
    );

    // The same two outcomes as write errors.
    let e = engine::claim(
        &store,
        &mut p,
        &no_git(),
        &[input("x", "ref:absent", key("n1"))],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(e[0].code, hibi::Code::SideMissing);
    assert!(e[0].message.contains("resource_not_found"));
    // The reference provider does not accept `symbol`: never sent.
    let symbol = Selector {
        symbol: Some("s".into()),
        ..Selector::default()
    };
    let e = engine::claim(
        &store,
        &mut p,
        &no_git(),
        &[input("x", "ref:notes", symbol)],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(e[0].code, hibi::Code::UnsupportedSelector);
}

fn cover(store: &MemStore, p: &mut Providers, resource: &str) -> Option<engine::CoverOutput> {
    let Some(git) = GitStatus::probe().git().cloned() else {
        eprintln!("git 2.45.0 or later is not installed; coverage part skipped");
        return None;
    };
    let config = RepoConfig::parse(&format!(
        "version = 1\n[schemes.ref]\n[[scope]]\nname = \"notes\"\nresource = \"{resource}\"\nkinds = [\"note\"]\n"
    ))
    .unwrap();
    let user = UserConfig::default();
    let env = CoverEnv {
        config: &config,
        user: &user,
        root: &std::env::temp_dir(),
        git: &git,
    };
    Some(engine::cover(store, p, &env, &CoverQuery::default(), None).unwrap())
}

/// R38 through the wire: a side on `n2` covers `n2` and its parent `n1`;
/// the `task` unit is not a unit of the scope.
#[test]
fn r38_coverage_through_the_parent_chain_of_the_reference_provider() {
    let store = MemStore::default();
    let mut p = providers("notes.json");
    let Some(before) = cover(&store, &mut p, "ref:notes") else {
        return;
    };
    let s = &before.scopes[0];
    assert_eq!((s.units, s.covered, s.uncovered), (2, 0, 2));
    assert_eq!(s.items[0].unit.key, "n1");
    assert_eq!(s.items[0].text.as_deref(), Some("Retries are capped at 5."));
    assert_eq!(s.items[0].anchor.sel, key("n1"));

    engine::claim(
        &store,
        &mut p,
        &no_git(),
        &[input("Configurable.", "ref:notes", key("n2"))],
        false,
        now(),
    )
    .unwrap();
    let after = cover(&store, &mut p, "ref:notes").unwrap();
    let s = &after.scopes[0];
    assert_eq!((s.units, s.covered, s.uncovered), (2, 2, 0));
}

/// R50 through the wire: a `parent` chain with a cycle terminates.
#[test]
fn r50_a_parent_cycle_from_a_real_provider_terminates() {
    let store = MemStore::default();
    let mut p = providers("cycle.json");
    engine::claim(
        &store,
        &mut p,
        &no_git(),
        &[input("Unit a.", "ref:cycle", key("a"))],
        false,
        now(),
    )
    .unwrap();
    let Some(out) = cover(&store, &mut p, "ref:cycle") else {
        return;
    };
    let s = &out.scopes[0];
    assert_eq!((s.units, s.covered, s.uncovered), (2, 2, 0));
}
