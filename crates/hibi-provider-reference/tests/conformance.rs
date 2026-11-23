//! R2: the reference provider passes every case in `cases/provider/protocol/`
//! and `cases/provider/reference/`. R51 for the reference provider through
//! the wire. Both run the built binary of this package.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hibi::fingerprint::prose_v1;
use hibi::model::{Anchor, Locator, Selector};
use hibi::provider::command::{
    CommandProvider, ConformanceOptions, load_cases, protocol_cases, run_conformance,
};
use hibi::provider::{Provider, ProviderErrorCode};

const BIN: &str = env!("CARGO_BIN_EXE_hibi-provider-reference");

fn provider_cases() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cases/provider")
}

/// The command a user would register: protocol cases have no fixture, so
/// the provider gets one in its argv, as from the user config.
fn registered_command() -> Vec<OsString> {
    vec![
        BIN.into(),
        "--fixture".into(),
        provider_cases().join("fixtures/notes.json").into(),
    ]
}

fn run_dir(sub: &str, command: &[OsString]) -> usize {
    let dir = provider_cases().join(sub);
    for e in std::fs::read_dir(&dir).unwrap() {
        let name = e.unwrap().file_name();
        assert!(
            name.to_string_lossy().ends_with(".json"),
            "{sub}/{name:?} is not a .json case file"
        );
    }
    let cases = load_cases(&dir).unwrap();
    assert!(!cases.is_empty());
    let tmp = tempfile::tempdir().unwrap();
    let mut opts = ConformanceOptions::new(tmp.path());
    opts.fixtures = Some(provider_cases().join("fixtures"));
    let report = run_conformance(command, &cases, &opts);
    let bad: Vec<String> = report
        .cases
        .iter()
        .filter(|c| !c.ok || c.skipped)
        .map(|c| format!("  [{}] {}", c.name, c.detail.as_deref().unwrap_or("")))
        .collect();
    assert!(
        bad.is_empty(),
        "{sub}: cases not passed:\n{}",
        bad.join("\n")
    );
    assert_eq!(report.passed, cases.len());
    assert!(report.hello.is_some());
    cases.len()
}

#[test]
fn r2_protocol_cases() {
    run_dir("protocol", &registered_command());
}

#[test]
fn r2_reference_cases() {
    run_dir("reference", &[OsString::from(BIN)]);
}

/// The cases compiled into hibi for `providers --check` pass too.
#[test]
fn r2_compiled_in_protocol_cases() {
    let tmp = tempfile::tempdir().unwrap();
    let report = run_conformance(
        &registered_command(),
        &protocol_cases(),
        &ConformanceOptions::new(tmp.path()),
    );
    assert_eq!(report.failed, 0, "{report:#?}");
    assert_eq!(report.passed, protocol_cases().len());
}

fn reference() -> CommandProvider {
    let fixture = provider_cases().join("fixtures/notes.json");
    CommandProvider::new(
        "ref",
        [OsString::from(BIN), "--fixture".into(), fixture.into()],
        &std::env::temp_dir(),
        Duration::from_secs(10),
    )
}

fn anchor(loc: &str, sel: Selector) -> Anchor {
    Anchor::new(Locator::new(loc).unwrap(), sel)
}

/// R51 through the wire: an absent resource is `not_found`, an existing one
/// with a selector that finds nothing is an empty list. Also: units carry
/// the `prose-v1` fingerprint of their text, and several anchors go out in
/// one `batch`.
#[test]
fn r51_through_command_provider() {
    let mut p = reference();
    let key = |k: &str| Selector {
        key: Some(k.into()),
        ..Selector::default()
    };
    let out = p.resolve(&[
        anchor("ref:absent", key("n1")),
        anchor("ref:notes", key("n9")),
        anchor("ref:notes", key("n1")),
        anchor("ref:notes", Selector::quote("cap")),
    ]);
    assert_eq!(
        out[0].as_ref().unwrap_err().code,
        ProviderErrorCode::NotFound
    );
    assert_eq!(out[1].as_ref().unwrap(), &vec![]);
    let n1 = &out[2].as_ref().unwrap()[0];
    assert_eq!(n1.key, "n1");
    assert_eq!(n1.text.as_deref(), Some("Retries are capped at 5."));
    assert_eq!(n1.fp, prose_v1("Retries are capped at 5."));
    assert_eq!(out[3].as_ref().unwrap().len(), 3);

    let units = p.units(&Locator::new("ref:notes").unwrap(), None).unwrap();
    assert_eq!(units.len(), 3);
    assert_eq!(units[1].parent.as_deref(), Some("n1"));
    let r = p.render(&Locator::new("ref:notes").unwrap(), "n2").unwrap();
    assert_eq!(r.context, ["Retries are capped at 5."]);
    assert!(p.failure().is_none());
}
