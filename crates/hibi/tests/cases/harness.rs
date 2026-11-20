//! Loader and runner for the data cases of `cases/` (spec section 16).
//!
//! A case file is a JSON array of `{name, input, expected, source, origin}`.
//! [`run`] loads one file and applies a check to every case; it reports
//! every failing case, not only the first.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

/// One data case.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub name: String,
    pub input: Value,
    pub expected: Value,
    pub source: String,
    pub origin: String,
}

/// The values of `origin`: the four of cases/README.md, plus `rfc8785`
/// (taken from RFC 8785) and `independent` (computed by an implementation
/// that shares no code with this crate) for the files of section 16.3.
pub const ORIGINS: &[&str] = &[
    "copied",
    "narrowed",
    "computed",
    "v5-adjusted",
    "rfc8785",
    "independent",
];

/// The `cases/` directory of the repository.
pub fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cases")
}

/// Load a case file by its path relative to `cases/`. Panics on a file that
/// is missing, is not an array of cases, or has two cases with one name.
pub fn load(file: &str) -> Vec<Case> {
    let path = cases_dir().join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let cases: Vec<Case> = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
    assert!(!cases.is_empty(), "{file} has no cases");
    for c in &cases {
        assert!(
            !c.source.is_empty(),
            "{file}: case {:?} has no source",
            c.name
        );
        assert!(
            ORIGINS.contains(&c.origin.as_str()),
            "{file}: case {:?} has origin {:?}, not one of {ORIGINS:?}",
            c.name,
            c.origin
        );
    }
    let mut names: Vec<&str> = cases.iter().map(|c| c.name.as_str()).collect();
    names.sort_unstable();
    for pair in names.windows(2) {
        assert_ne!(pair[0], pair[1], "{file}: duplicate case name");
    }
    cases
}

/// Load `file` and check every case with `check`, which returns a
/// description of the difference on failure. Panics listing every failing
/// case. Returns the number of cases run.
pub fn run(file: &str, check: fn(&Case) -> Result<(), String>) -> usize {
    let cases = load(file);
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|c| {
            let outcome = std::panic::catch_unwind(|| check(c))
                .unwrap_or_else(|p| Err(format!("panicked: {}", panic_text(&p))));
            outcome
                .err()
                .map(|e| format!("  [{}] ({}) {e}", c.name, c.origin))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{file}: {} of {} cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    cases.len()
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "non-string panic".to_owned())
}

/// Compare two values and describe the difference.
pub fn expect_eq<T: PartialEq + std::fmt::Debug>(
    what: &str,
    got: T,
    want: T,
) -> Result<(), String> {
    if got == want {
        Ok(())
    } else {
        Err(format!("{what}: got {got:?}, want {want:?}"))
    }
}

/// A string field of a JSON object.
pub fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string field {key:?} in {v}"))
}

/// Every case file under `cases/`, as paths relative to it with `/`
/// separators: every file except `README.md` and anything inside a
/// directory named `fixtures` (inputs that case files refer to).
pub fn all_case_files() -> Vec<String> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| e.unwrap())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if e.file_type().unwrap().is_dir() {
                if name != "fixtures" {
                    walk(&e.path(), &path, out);
                }
            } else if path != "README.md" {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&cases_dir(), "", &mut out);
    out
}
