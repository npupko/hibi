//! The data cases of `cases/` (spec section 16) and requirement R1: every
//! case file is loaded by a test.
//!
//! Bind a case file by adding one line to the `bindings!` table below:
//!
//! - `file "<path>" => <test name>, <check>;` loads the file with
//!   `harness::run` and applies `check: fn(&harness::Case) -> Result<(), String>`
//!   to every case.
//! - `dir "<prefix>/" => <test name>, <runner>;` covers every case file whose
//!   path starts with the prefix; `runner: fn(&[String])` receives those
//!   paths and must load each of them (for cases that are not
//!   `{name, input, expected, source, origin}` arrays, such as the provider
//!   conformance cases of section 13.6).
//!
//! Then remove the file from `PENDING`.

mod harness;

mod canon;
mod duration;
mod text;

/// Case files that no test loads yet, because the module they bind belongs
/// to a unit that has not landed. R1 fails if a file listed here does not
/// exist or is bound in `bindings!`. This list MUST be empty at release.
const PENDING: &[&str] = &[
    // locate (section 7, and quote_selector of section 16.1)
    "error-budget.json",
    "within-budget.json",
    "bitap.json",
    "fuzzy-locate.json",
    "locate.json",
    "quote-selector.json",
    // prose (section 6.3)
    "regions.json",
    // code (section 8.4)
    "ast-node.json",
    "ast-equivalence.json",
];

enum Binding {
    File(&'static str),
    // For the provider conformance directories (section 13.6), bound by a later unit.
    #[allow(dead_code)]
    Dir(&'static str),
}

macro_rules! bindings {
    ($( $kind:ident $path:literal => $test:ident, $check:path; )*) => {
        const BINDINGS: &[Binding] = &[ $( bindings!(@entry $kind $path) ),* ];
        $( bindings!(@test $kind $path, $test, $check); )*
    };
    (@entry file $path:literal) => { Binding::File($path) };
    (@entry dir $path:literal) => { Binding::Dir($path) };
    (@test file $path:literal, $test:ident, $check:path) => {
        #[test]
        fn $test() {
            harness::run($path, $check);
        }
    };
    (@test dir $path:literal, $test:ident, $runner:path) => {
        #[test]
        fn $test() {
            let files: Vec<String> = harness::all_case_files()
                .into_iter()
                .filter(|f| f.starts_with($path))
                .collect();
            assert!(!files.is_empty(), "no case files under {}", $path);
            $runner(&files);
        }
    };
}

bindings! {
    file "canon.json" => canon_json, canon::canon;
    file "ids.json" => ids_json, canon::ids;
    file "normalize.json" => normalize_json, text::normalize;
    file "levenshtein.json" => levenshtein_json, text::levenshtein;
    file "similarity.json" => similarity_json, text::similarity;
    file "duration.json" => duration_json, duration::duration;
    file "recheck-boundary.json" => recheck_boundary_json, duration::recheck_boundary;
}

fn is_bound(file: &str) -> bool {
    BINDINGS.iter().any(|b| match b {
        Binding::File(f) => *f == file,
        Binding::Dir(prefix) => file.starts_with(prefix),
    })
}

/// R1: every case file under `cases/` is loaded by a test, apart from the
/// files in `PENDING`; every pending file exists and is not bound.
#[test]
fn r1_every_case_file_is_loaded() {
    let files = harness::all_case_files();
    let mut problems = Vec::new();
    for f in &files {
        if !is_bound(f) && !PENDING.contains(&f.as_str()) {
            problems.push(format!(
                "{f} is loaded by no test: bind it in tests/cases/main.rs"
            ));
        }
    }
    for p in PENDING {
        if !files.iter().any(|f| f == p) {
            problems.push(format!(
                "PENDING lists {p}, which does not exist under cases/"
            ));
        }
        if is_bound(p) {
            problems.push(format!(
                "PENDING lists {p}, which is bound: remove it from PENDING"
            ));
        }
    }
    for b in BINDINGS {
        match b {
            Binding::File(f) if !files.iter().any(|x| x == f) => {
                problems.push(format!("{f} is bound but does not exist under cases/"));
            }
            Binding::Dir(d) if !files.iter().any(|x| x.starts_with(d)) => {
                problems.push(format!("{d} is bound but no case file is under it"));
            }
            _ => {}
        }
    }
    assert!(problems.is_empty(), "R1:\n{}", problems.join("\n"));
}
