//! The provider conformance cases of section 13.6 (`cases/provider/`).
//!
//! These runners load and check every case file. Running the cases against
//! the reference provider (R2) needs its binary, which only its own package
//! can locate in a build-order-safe way (`CARGO_BIN_EXE_*`), so R2 is in
//! `crates/hibi-provider-reference/tests/conformance.rs`.

use std::collections::HashSet;

use hibi::provider::command::{ConformanceCase, End, Outgoing};

use crate::harness::cases_dir;

fn load(file: &str) -> ConformanceCase {
    let text = std::fs::read_to_string(cases_dir().join(file)).unwrap();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file}: {e}"))
}

/// Every file parses as a case, names are unique, the first step is
/// `hello`, every case ends with `exit`, and `fixture` is present exactly
/// when `with_fixture` and names a file under `cases/provider/fixtures/`.
fn check(files: &[String], with_fixture: bool) {
    let mut names = HashSet::new();
    for f in files {
        assert!(f.ends_with(".json"), "{f} is not a .json case file");
        let case = load(f);
        assert!(names.insert(case.name.clone()), "{f}: duplicate case name");
        assert_eq!(case.end, Some(End::Exit), "{f}: a case ends with exit");
        let Some(first) = case.steps.first() else {
            panic!("{f}: no steps")
        };
        match &first.send {
            Outgoing::Request(r) => assert_eq!(r["method"], "hello", "{f}: hello comes first"),
            Outgoing::Raw(_) => panic!("{f}: hello comes first"),
        }
        match (&case.fixture, with_fixture) {
            (None, false) => {}
            (Some(fx), true) => {
                let path = cases_dir().join("provider/fixtures").join(fx);
                assert!(path.is_file(), "{f}: fixture {fx} does not exist");
            }
            (fx, _) => panic!("{f}: fixture {fx:?} is not allowed here"),
        }
    }
}

pub fn protocol(files: &[String]) {
    check(files, false);
}

pub fn reference(files: &[String]) {
    check(files, true);
}
