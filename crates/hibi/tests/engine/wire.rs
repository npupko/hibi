//! The engine through `CommandProvider` and the scripted provider of
//! `tests/providers/scripted.sh`, whose log shows every line written to a
//! provider's standard input (R28, R62).

#![cfg(all(unix, feature = "command"))]

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use hibi::engine::{self, ClaimState, Reason, SideState, StatusQuery};
use hibi::model::{Claim, Selector};
use hibi::provider::Providers;
use hibi::provider::command::CommandProvider;
use hibi::store::Batch;

use crate::support::*;

const FP: &str = "b3:test-v1:0123456789abcdef0123456789abcdef";

struct Scripted {
    dir: tempfile::TempDir,
    p: Providers,
}

impl Scripted {
    /// The lines the provider read, after `start`.
    fn requests(&self) -> Vec<String> {
        match std::fs::read_to_string(self.dir.path().join("work/log")) {
            Ok(text) => text.lines().skip(1).map(str::to_owned).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn started(&self) -> bool {
        self.dir.path().join("work/log").exists()
    }
}

/// A provider for scheme `t` that accepts `key` only and answers request
/// `n` with line `n` of `answers` (after `hello`).
fn scripted(answers: &[&str]) -> Scripted {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let hello = r#"{"id":@ID@,"result":{"protocol":1,"provider":{"name":"scripted","version":"0"},"schemes":["t"],"selectors":["key"],"capabilities":{"units":true}}}"#;
    let mut lines = vec![hello.to_owned()];
    lines.extend(answers.iter().map(|s| (*s).to_owned()));
    let script = dir.path().join("script");
    std::fs::write(&script, lines.join("\n") + "\n").unwrap();
    let scripted_sh = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/providers/scripted.sh");
    let argv: Vec<OsString> = vec!["sh".into(), scripted_sh.into(), script.into(), work.into()];
    let provider = CommandProvider::new("t", argv, dir.path(), Duration::from_secs(10));
    let mut p = Providers::new();
    p.register(&["t"], Box::new(provider));
    Scripted { dir, p }
}

fn stored(store: &MemStore, statement: &str, sel: Selector) -> Claim {
    let claim = Claim::new(statement, anchor("t:a", sel), vec![], None, None).unwrap();
    store.insert(Batch {
        claims: vec![claim.clone()],
        ..Batch::default()
    });
    claim
}

/// R62: a selector with a key the provider does not allow is never written
/// to the provider's standard input; an allowed one is.
#[test]
fn r62_a_disallowed_key_never_reaches_standard_input() {
    let answer = format!(
        r#"{{"id":@ID@,"result":{{"units":[{{"key":"k1","kind":"note","fp":"{FP}","sel":{{"key":"k1"}},"text":"x"}}]}}}}"#
    );
    let mut s = scripted(&[&answer]);
    let store = MemStore::new();
    let quoted = stored(&store, "quoted", Selector::quote("secret quote text"));
    let keyed = stored(&store, "keyed", key_sel("k1"));
    let q = StatusQuery {
        states: ClaimState::ALL.to_vec(),
        ..StatusQuery::default()
    };
    let out = engine::status(&store, &mut s.p, &no_git(), &q, None, now()).unwrap();
    let side = |id| {
        let c = out.claims.iter().find(|c| &c.id == id).unwrap();
        (c.sides[0].state, c.sides[0].reason)
    };
    assert_eq!(
        side(&quoted.id),
        (SideState::Unavailable, Some(Reason::ProviderError))
    );
    assert_eq!(side(&keyed.id), (SideState::Changed, Some(Reason::NoMark)));

    let requests = s.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(requests[0].contains(r#""method":"hello""#));
    assert!(requests[1].contains(r#""method":"resolve""#));
    assert!(requests[1].contains(r#""key":"k1""#));
    for r in &requests {
        assert!(!r.contains("quote"), "{r}");
    }
}

/// Section 13.1 and R28: a provider command is not started when no locator
/// of its scheme needs it, and an unregistered scheme starts nothing.
#[test]
fn r28_no_process_is_started_without_need() {
    let mut s = scripted(&[]);
    let store = MemStore::new();
    let claim = stored(&store, "retired", key_sel("k1"));
    engine::retire(
        &store,
        &engine::RetireInput {
            claims: vec![claim.id.to_string()],
            ..Default::default()
        },
        now(),
    )
    .unwrap();
    // Only a retired claim uses scheme `t`: its sides are not resolved.
    let q = StatusQuery {
        states: ClaimState::ALL.to_vec(),
        ..StatusQuery::default()
    };
    let out = engine::status(&store, &mut s.p, &no_git(), &q, None, now()).unwrap();
    assert_eq!(out.claims[0].state, ClaimState::Retired);
    engine::list(&store, &engine::ListQuery::default()).unwrap();
    assert!(!s.started());

    // A live claim on a scheme that is declared but not registered.
    let live = stored(&store, "live", key_sel("k2"));
    let mut declared = Providers::new();
    declared.declare_unregistered("t");
    let out = engine::status(&store, &mut declared, &no_git(), &q, None, now()).unwrap();
    let c = out.claims.iter().find(|c| c.id == live.id).unwrap();
    assert_eq!(
        (c.sides[0].state, c.sides[0].reason),
        (SideState::Unavailable, Some(Reason::ProviderNotRegistered))
    );
    assert!(!s.started());
}
