//! Integration tests of `FileProvider` on temporary directories, with a real
//! `git` where a requirement needs one (spec sections 6 and 10.4;
//! requirements R12 to R15, R40, R45 to R47, R49, R51).
//!
//! Every git process runs with `GIT_CONFIG_GLOBAL=/dev/null` and
//! `GIT_CONFIG_NOSYSTEM=1` (through a wrapper script for the provider), so
//! the developer's own git configuration cannot change a result.

#![cfg(all(unix, feature = "file", feature = "cache"))]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use hibi::Code;
use hibi::cache::Cache;
use hibi::config::{RepoConfig, RepoEntry, UserConfig};
use hibi::git::{GitStatus, ObjectId};
use hibi::model::{Anchor, Locator, Selector};
use hibi::provider::file::{
    FileOptions, FileProvider, MAX_CODE_FILE_BYTES, MAX_FILE_BYTES, UnitCache,
};
use hibi::provider::{Provider, ProviderError, ProviderErrorCode, Refuse, Unit};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sh_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// A usable git that runs the real git with an isolated configuration.
fn isolated_git(dir: &Path) -> GitStatus {
    let real = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    let wrapper = dir.join("git-isolated");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nGIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 exec '{real}' \"$@\"\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let status = GitStatus::probe_program(&wrapper);
    assert!(status.git().is_some(), "git is not usable: {status:?}");
    status
}

fn no_git() -> GitStatus {
    GitStatus::Missing {
        message: "git is not installed".into(),
    }
}

fn options(git: GitStatus) -> FileOptions {
    FileOptions {
        repo: RepoConfig::default(),
        user: UserConfig::default(),
        git,
        cache: None,
    }
}

fn wt(root: &Path) -> FileProvider {
    FileProvider::working_tree(root, options(no_git()))
}

fn loc(s: &str) -> Locator {
    Locator::new(s).unwrap()
}

fn sel(v: Value) -> Selector {
    serde_json::from_value(v).unwrap()
}

fn resolve(p: &mut dyn Provider, l: &str, s: Value) -> Result<Vec<Unit>, ProviderError> {
    p.resolve(&[Anchor::new(loc(l), sel(s))]).remove(0)
}

/// The one unit an anchor resolves to.
fn one(p: &mut dyn Provider, l: &str, s: Value) -> Unit {
    let units = resolve(p, l, s.clone()).unwrap_or_else(|e| panic!("{l} {s}: {e}"));
    assert_eq!(units.len(), 1, "{l} {s}: {units:#?}");
    units.into_iter().next().unwrap()
}

/// An `unavailable` error whose message ends at the fix: the engine adds
/// that the command can be run again (ruling on section 9.2).
fn assert_unavailable_ending(e: &ProviderError, fix: &str) {
    assert_eq!(e.code, ProviderErrorCode::Unavailable, "{}", e.message);
    assert_message_ends_at_the_fix(e, fix);
}

/// An `unparsable` error (section 13.4: the file is there and can be read,
/// and its content cannot be split into units), with the same message rule.
fn assert_unparsable_ending(e: &ProviderError, fix: &str) {
    assert_eq!(e.code, ProviderErrorCode::Unparsable, "{}", e.message);
    assert_message_ends_at_the_fix(e, fix);
}

fn assert_message_ends_at_the_fix(e: &ProviderError, fix: &str) {
    assert!(e.message.ends_with(fix), "{}", e.message);
    assert!(!e.message.contains("again"), "{}", e.message);
}

fn code_of(r: Result<Vec<Unit>, ProviderError>) -> ProviderErrorCode {
    r.expect_err("an error").code
}

fn write(root: &Path, path: &str, text: impl AsRef<[u8]>) {
    let f = root.join(path);
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(f, text).unwrap();
}

// ---------------------------------------------------------------------------
// R12 to R15: fingerprints of the resolved unit under edits
// ---------------------------------------------------------------------------

/// One language: a path, the symbol of the anchored declaration, the base
/// text, edits that must keep the fingerprint, and an edit of a literal
/// that must change it.
struct Lang {
    path: &'static str,
    symbol: &'static str,
    base: &'static str,
    same: &'static [(&'static str, &'static str)],
    literal: &'static str,
}

const LANGS: &[Lang] = &[
    Lang {
        path: "src/retry.ts",
        symbol: "retry",
        base: "export function retry(n: number): number {\n  const xs = [1, 2];\n  return Math.min(n, 5);\n}\n",
        same: &[
            (
                "whitespace",
                "export function retry(n: number): number\n{\n\n    const xs = [1,  2];\n    return Math.min( n, 5 );\n}\n",
            ),
            (
                "comments",
                "export function retry(n: number): number {\n  // cap\n  const xs = [1, 2]; /* two */\n  return Math.min(n, 5);\n}\n",
            ),
            (
                "trailing commas",
                "export function retry(n: number): number {\n  const xs = [1, 2,];\n  return Math.min(n, 5,);\n}\n",
            ),
        ],
        literal: "export function retry(n: number): number {\n  const xs = [1, 2];\n  return Math.min(n, 6);\n}\n",
    },
    Lang {
        path: "src/app.tsx",
        symbol: "App",
        base: "export function App() {\n  const xs = [1, 2];\n  return <div title=\"a\">{xs.length + 5}</div>;\n}\n",
        same: &[
            (
                "whitespace",
                "export function App() {\n    const xs = [1,   2];\n\n    return <div title=\"a\">{xs.length  +  5}</div>;\n}\n",
            ),
            (
                "comments",
                "export function App() {\n  // two\n  const xs = [1, 2]; /* c */\n  return <div title=\"a\">{xs.length + 5}</div>;\n}\n",
            ),
            (
                "trailing commas",
                "export function App() {\n  const xs = [1, 2,];\n  return <div title=\"a\">{xs.length + 5}</div>;\n}\n",
            ),
        ],
        literal: "export function App() {\n  const xs = [1, 2];\n  return <div title=\"b\">{xs.length + 5}</div>;\n}\n",
    },
    Lang {
        path: "retry.py",
        symbol: "retry",
        base: "def retry(n):\n    xs = [1, 2]\n    return min(n, 5)\n",
        same: &[
            (
                "whitespace",
                "def retry( n ):\n    xs = [1,   2]\n\n\n    return min(n,5)\n",
            ),
            (
                "comments",
                "def retry(n):\n    # cap\n    xs = [1, 2]  # two\n    return min(n, 5)\n",
            ),
            (
                "trailing commas",
                "def retry(n,):\n    xs = [1, 2,]\n    return min(n, 5,)\n",
            ),
        ],
        literal: "def retry(n):\n    xs = [1, 2]\n    return min(n, 6)\n",
    },
    Lang {
        path: "src/retry.rs",
        symbol: "retry",
        base: "pub fn retry(n: u32) -> u32 {\n    let xs = [1, 2];\n    n.min(5)\n}\n",
        same: &[
            (
                "whitespace",
                "pub fn retry(n: u32) -> u32\n{\n    let xs = [1,    2];\n\n    n.min( 5 )\n}\n",
            ),
            (
                "comments",
                "pub fn retry(n: u32) -> u32 {\n    // cap\n    let xs = [1, 2]; /* two */\n    n.min(5)\n}\n",
            ),
            (
                "trailing commas",
                "pub fn retry(n: u32,) -> u32 {\n    let xs = [1, 2,];\n    n.min(5,)\n}\n",
            ),
        ],
        literal: "pub fn retry(n: u32) -> u32 {\n    let xs = [1, 2];\n    n.min(6)\n}\n",
    },
    Lang {
        path: "retry.go",
        symbol: "Retry",
        base: "package p\n\nfunc Retry(n int) int {\n\txs := []int{1, 2}\n\t_ = xs\n\treturn min(n, 5)\n}\n",
        same: &[
            (
                "whitespace",
                "package p\n\nfunc Retry(n int) int {\n\n\txs := []int{1,   2}\n\t_ = xs\n\treturn min(n,5)\n}\n",
            ),
            (
                "comments",
                "package p\n\nfunc Retry(n int) int {\n\t// cap\n\txs := []int{1, 2} /* two */\n\t_ = xs\n\treturn min(n, 5)\n}\n",
            ),
            (
                "trailing commas",
                "package p\n\nfunc Retry(n int,) int {\n\txs := []int{1, 2,}\n\t_ = xs\n\treturn min(n, 5,)\n}\n",
            ),
        ],
        literal: "package p\n\nfunc Retry(n int) int {\n\txs := []int{1, 2}\n\t_ = xs\n\treturn min(n, 6)\n}\n",
    },
    Lang {
        path: "src/A.java",
        symbol: "A.retry",
        base: "class A {\n  int retry(int n) {\n    int[] xs = {1, 2};\n    return Math.min(n, 5);\n  }\n}\n",
        same: &[
            (
                "whitespace",
                "class A {\n  int retry(int n)\n  {\n      int[] xs = {1,   2};\n\n      return Math.min( n, 5 );\n  }\n}\n",
            ),
            (
                "comments",
                "class A {\n  int retry(int n) {\n    // cap\n    int[] xs = {1, 2}; /* two */\n    return Math.min(n, 5);\n  }\n}\n",
            ),
            (
                "trailing commas",
                "class A {\n  int retry(int n) {\n    int[] xs = {1, 2,};\n    return Math.min(n, 5);\n  }\n}\n",
            ),
        ],
        literal: "class A {\n  int retry(int n) {\n    int[] xs = {1, 2};\n    return Math.min(n, 7);\n  }\n}\n",
    },
];

#[test]
fn r12_r13_r14_code_fingerprints_per_language() {
    for lang in LANGS {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let l = format!("file:{}", lang.path);
        let anchor = json!({"symbol": lang.symbol});
        let fp = |text: &str| {
            write(root, lang.path, text);
            let u = one(&mut wt(root), &l, anchor.clone());
            assert!(!u.approx, "{}: approx", lang.path);
            assert_eq!(u.fp.profile(), "ast-v1", "{}: {text}", lang.path);
            u.fp
        };
        let base = fp(lang.base);
        for (what, text) in lang.same {
            assert_eq!(
                fp(text),
                base,
                "R12 {}: {what} changed the fingerprint",
                lang.path
            );
        }
        assert_ne!(
            fp(lang.literal),
            base,
            "R13 {}: a literal change kept the fingerprint",
            lang.path
        );
        assert_eq!(
            fp(lang.base),
            base,
            "R14 {}: reverting did not restore the fingerprint",
            lang.path
        );
    }
}

/// Ruling 14 on section 6.4: a claim quoted on a member resolves to the
/// member, so editing a sibling member keeps its fingerprint and editing
/// the member changes it. The container changes in both cases.
#[test]
fn members_are_units_with_their_own_fingerprints() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let cases: &[(&str, &str, &str, &str, &str)] = &[
        // (path, base, sibling edit, own edit, quote)
        (
            "a.rs",
            "pub struct C {\n    pub port: u16,\n    pub host: String,\n}\n",
            "pub struct C {\n    pub port: u32,\n    pub host: String,\n}\n",
            "pub struct C {\n    pub port: u16,\n    pub host: Vec<u8>,\n}\n",
            "pub host",
        ),
        (
            "b.rs",
            "enum L {\n    Low = 1,\n    High = 9,\n}\n",
            "enum L {\n    Low = 2,\n    High = 9,\n}\n",
            "enum L {\n    Low = 1,\n    High = 8,\n}\n",
            "High",
        ),
        (
            "c.ts",
            "interface C {\n  port: number;\n  host: string;\n}\n",
            "interface C {\n  port: string;\n  host: string;\n}\n",
            "interface C {\n  port: number;\n  host: number;\n}\n",
            "host",
        ),
        (
            "d.go",
            "package p\n\ntype C struct {\n\tPort int\n\tHost string\n}\n",
            "package p\n\ntype C struct {\n\tPort uint\n\tHost string\n}\n",
            "package p\n\ntype C struct {\n\tPort int\n\tHost []byte\n}\n",
            "Host",
        ),
        (
            "E.java",
            "enum E {\n    LOW(1),\n    HIGH(9);\n    E(int v) {}\n}\n",
            "enum E {\n    LOW(2),\n    HIGH(9);\n    E(int v) {}\n}\n",
            "enum E {\n    LOW(1),\n    HIGH(8);\n    E(int v) {}\n}\n",
            "HIGH",
        ),
        (
            "f.py",
            "class C:\n    port = 1\n    host = \"h\"\n",
            "class C:\n    port = 2\n    host = \"h\"\n",
            "class C:\n    port = 1\n    host = \"i\"\n",
            "host",
        ),
    ];
    for (path, base, sibling, own, quote) in cases {
        let l = format!("file:{path}");
        let resolve_at = |text: &str| {
            write(root, path, text);
            let member = one(&mut wt(root), &l, json!({"quote": quote}));
            let units = wt(root).units(&loc(&l), None).unwrap();
            let parent = member.parent.clone().unwrap();
            let container = units.iter().find(|u| u.key == parent).unwrap().fp.clone();
            (member, container)
        };
        let (m0, c0) = resolve_at(base);
        assert!(
            matches!(m0.kind.as_str(), "field" | "variant"),
            "{path}: {}",
            m0.kind
        );
        let (m1, c1) = resolve_at(sibling);
        assert_eq!((&m1.key, &m1.fp), (&m0.key, &m0.fp), "{path}: sibling edit");
        assert_ne!(c1, c0, "{path}: the container sees the sibling edit");
        let (m2, c2) = resolve_at(own);
        assert_eq!(m2.key, m0.key, "{path}");
        assert_ne!(m2.fp, m0.fp, "{path}: own edit");
        assert_ne!(c2, c0, "{path}");
    }
}

/// Copy every file under `cases/fixtures` into `root/fx/` and return the
/// copied paths, relative to `root`.
fn copy_fixtures(root: &Path) -> Vec<String> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cases/fixtures");
    let mut files = Vec::new();
    let mut stack = vec![fixtures.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p
                    .strip_prefix(&fixtures)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                let dest = format!("fx/{rel}");
                write(root, &dest, fs::read(&p).unwrap());
                files.push(dest);
            }
        }
    }
    files.sort();
    files
}

/// Declarations of one symbol that share a long common start and differ
/// only later: `copies` functions named `run` whose first `shared` lines are
/// equal, in Python and TypeScript (ruling on section 6.7: a declaration
/// whose scope selector another declaration reproduces takes whole-text
/// context, so each copy's anchor still selects it).
fn shared_start_copies(copies: usize, shared: usize) -> [(String, String); 2] {
    let mut py = String::new();
    let mut ts = String::new();
    for k in 0..copies {
        py.push_str("def run():\n");
        ts.push_str("function run(): number {\n");
        for line in 0..shared {
            py.push_str(&format!("    value_{line} = compute({line})\n"));
            ts.push_str(&format!("  const value_{line} = compute({line});\n"));
        }
        py.push_str(&format!("    return value_0 + {}\n\n\n", k + 1));
        ts.push_str(&format!("  return value_0 + {};\n}}\n\n", k + 1));
    }
    [
        (format!("gen/run_{copies}_{shared}.py"), py),
        (format!("gen/run_{copies}_{shared}.ts"), ts),
    ]
}

/// Ruling on section 6.7: a completed selector is a fixed point. Resolving
/// it against unchanged text gives the same unit, not approximate, not
/// refused, with an identical selector. Checked for anchors generated from
/// every unit of every fixture (each line of the unit and each word of its
/// first lines, with and without the unit's symbol), for every unit's own
/// selector, and for generated copies of one declaration that share a long
/// common start, plus the verifier's repro, where a hit near the unit's edge
/// used to lose its stored context. The anchors of one file are resolved in
/// one batch, as the engine does, so the document is built once per batch.
#[test]
fn completed_selectors_are_fixed_points() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        root,
        "m.rs",
        "fn g() {\n    run(alpha); // run(alpha);\n}\n\n// run(alpha);\nfn f() {\n    run(alpha);\n    // run(alpha);\n    /* run(alpha); */\n    run(alpha); // run(alpha);\n}\n",
    );
    let mut files = vec!["m.rs".to_owned()];
    files.extend(copy_fixtures(root));
    for (copies, shared) in [(2, 3), (3, 4), (5, 6)] {
        for (path, text) in shared_start_copies(copies, shared) {
            write(root, &path, text);
            files.push(path);
        }
    }
    // The shapes where one `within` names several containers (section
    // 6.7): pieces with one name, table rows with one first cell, nested
    // list items, first sentences equal for 120 characters, a heading that
    // reads as a piece name, sections with one path, overloads and
    // same-named methods.
    let long = "a".repeat(130);
    let b1_shapes = [
        (
            "b1/pieces.md".to_owned(),
            "# Guide\n\n## Timeouts\n\nSet this in config.toml. The read timeout defaults to 45 s.\n\nSet this in config.toml. The write timeout defaults to 10 s.\n\n## Other\n\nNothing here. The read timeout defaults to 45 s.\n".to_owned(),
        ),
        (
            "b1/table.md".to_owned(),
            "# T\n\n| key | value. More. |\n| --- | --- |\n| port | 80. Plain. |\n| port | 443. Secure. |\n".to_owned(),
        ),
        (
            "b1/lists.md".to_owned(),
            "# L\n\n- Step one. Do it.\n  - Step one. Do it again.\n  - Step one. Do it.\n- Step one. Done.\n".to_owned(),
        ),
        (
            "b1/long.md".to_owned(),
            format!("# Long\n\n{long} first. Then one.\n\n{long} second. Then two.\n"),
        ),
        (
            "b1/collide.md".to_owned(),
            "# Guide\n\nInstall it. Then run it.\n\n## Install it.\n\nInstall it. Then stop.\n\nThen run it.\n".to_owned(),
        ),
        (
            "b1/sections.md".to_owned(),
            "# Guide\n\n## Timeouts\n\nRead: 45 s. Write: 10 s.\n\n## Timeouts\n\nRead: 45 s. Write: 20 s.\n\n### Timeouts\n\nRead: 45 s.\n".to_owned(),
        ),
        (
            "b1/over.ts".to_owned(),
            "function parse(s: string): number;\nfunction parse(s: number): number;\nfunction parse(s: any): number {\n  return Number(s) + 1;\n}\nfunction legacyParse(s: any): number {\n  return Number(s) + 1;\n}\n".to_owned(),
        ),
        (
            "b1/s.py".to_owned(),
            "class S:\n    def dispatch(self, event):\n        handlers = self.lookup_handlers(event)\n        return [h(event) for h in handlers]\n\n    def dispatch(self, event):\n        handlers = self.lookup_handlers(event)\n        return [h(event) for h in handlers]\n        self.log(event)\n\n    def dispatch(self, event):\n        handlers = self.lookup_handlers(event)\n        return [h(event) for h in handlers]\n".to_owned(),
        ),
    ];
    for (path, text) in b1_shapes {
        write(root, &path, text);
        files.push(path);
    }
    // The verifier's repro: `#f::(a` on m.rs.
    let mut p = wt(root);
    let first = one(&mut p, "file:m.rs", json!({"symbol": "f", "quote": "(a"}));
    assert!(first.refuse.is_none(), "{first:?}");
    let again = one(
        &mut p,
        "file:m.rs",
        serde_json::to_value(&first.sel).unwrap(),
    );
    assert_eq!(
        (&again.key, &again.sel, again.approx, again.refuse),
        (&first.key, &first.sel, false, None)
    );

    let mut checked = 0;
    let mut failures = Vec::new();
    for path in &files {
        let l = format!("file:{path}");
        let mut p = wt(root);
        let units = match p.units(&loc(&l), None) {
            Ok(units) => units,
            // A fixture of a code class that does not parse has no units
            // (section 6.1).
            Err(e) if e.code == ProviderErrorCode::Unparsable => {
                continue;
            }
            Err(e) => panic!("{path}: {e}"),
        };
        let text = fs::read_to_string(root.join(path))
            .unwrap()
            .replace("\r\n", "\n");
        // Every unit's own selector, then anchors from its lines and words.
        let mut sels: Vec<Value> = Vec::new();
        for u in units.iter().filter(|u| u.kind != "file") {
            sels.push(serde_json::to_value(&u.sel).unwrap());
            let r = u.range.unwrap();
            let unit_text = &text[r.start..r.end];
            let mut quotes: Vec<String> = unit_text
                .lines()
                .map(|line| line.trim().to_owned())
                .filter(|q| !q.is_empty() && q.chars().count() <= 200)
                .collect();
            quotes.extend(
                unit_text
                    .lines()
                    .take(3)
                    .flat_map(|line| line.split_whitespace())
                    .filter(|w| w.chars().count() >= 2)
                    .map(str::to_owned),
            );
            for q in quotes {
                sels.push(json!({"quote": q}));
                if let Some(s) = &u.sel.symbol {
                    sels.push(json!({"symbol": s, "quote": q}));
                }
            }
        }
        // Every unit's printed anchor selects that unit alone, unless the
        // unit is marked as one that no anchor selects alone; a marked
        // unit's anchor does not select it alone (section 6.7).
        let own: Vec<Anchor> = units
            .iter()
            .map(|u| Anchor::new(loc(&l), u.sel.clone()))
            .collect();
        for (u, got) in units.iter().zip(p.resolve(&own)) {
            let got = got.unwrap();
            let alone = got.len() == 1
                && got[0].key == u.key
                && !got[0].approx
                && got[0].identity.is_none();
            let marked = u.refuse == Some(Refuse::QuoteNotDistinguishable);
            if alone == marked || (u.refuse.is_some() && !marked) {
                failures.push(format!(
                    "{path}: the printed anchor of {} (marked: {marked}) gives {:?}",
                    u.key, got
                ));
            }
            checked += 1;
        }
        let anchors: Vec<Anchor> = sels
            .iter()
            .map(|s| Anchor::new(loc(&l), sel(s.clone())))
            .collect();
        let firsts: Vec<(usize, Unit)> = p
            .resolve(&anchors)
            .into_iter()
            .enumerate()
            .filter_map(|(k, r)| {
                let mut got = r.ok()?;
                (got.len() == 1 && got[0].refuse.is_none()).then(|| (k, got.remove(0)))
            })
            .collect();
        let again_anchors: Vec<Anchor> = firsts
            .iter()
            .map(|(_, u)| Anchor::new(loc(&l), u.sel.clone()))
            .collect();
        for ((k, first), again) in firsts.iter().zip(p.resolve(&again_anchors)) {
            let again = again.unwrap();
            checked += 1;
            let ok = again.len() == 1
                && again[0].key == first.key
                && again[0].sel == first.sel
                && !again[0].approx
                && again[0].identity.is_none()
                && again[0].refuse.is_none();
            if !ok && failures.len() < 10 {
                failures.push(format!(
                    "{path} {} -> {:?} then {:?}",
                    sels[*k], first.sel, again
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} not fixed points:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(checked > 5000, "{checked} anchors checked");
}

/// A default quote is never inside a comment (audit of section 6.7): no
/// unit of a file with a parse tree starts inside a comment node, so the
/// "first line starts with a comment" fallback is never taken, and every
/// unit's own completed selector resolves to exactly that unit, neither
/// refused nor approximate. Checked on every fixture under `cases/` and on
/// inputs with comments in every position, in every class.
#[test]
fn default_quotes_are_never_in_comments_and_round_trip() {
    let crafted: &[(&str, &str)] = &[
        (
            "c.rs",
            "// a\n/* b */ #[derive(Debug)] /* c */ struct S; // d\n#[cfg(x)] // e\nfn f() {}\n/// doc\n#[inline]\n/* k */ fn g() {}\nimpl S { /* m */ fn h() {} // n\n}\nenum E { /* p */ A, // q\n#[x] B }\n#[cfg(test)] /* r */ use std::fmt;\n",
        ),
        (
            "c.ts",
            "/* a */ export const x = 1; // b\nclass C { /* c */ @d /* e */ m() {} /** f */ p = 1; }\ninterface I { /* g */ q: number; // h\n}\nenum F { /* i */ A, B /* j */ }\n",
        ),
        (
            "c.tsx",
            "const v = <div>{/* a */}</div>; // b\n/** c */ function F() { return <p>{x}</p>; }\n",
        ),
        (
            "c.py",
            "# a\nx = 1  # b\nclass C:  # c\n    # d\n    y = 2\n    def m(self):  # e\n        pass\n@dec  # f\ndef g():\n    pass\n",
        ),
        (
            "c.go",
            "package p // a\n/* b */ func F() {}\ntype T struct { /* c */ A int // d\n\tB string\n}\nvar ( /* e */ v = 1 )\n",
        ),
        (
            "C.java",
            "/** a */ class C { /* b */ int x; // c\n  @A /* d */ void m() {}\n  enum E { /* e */ P, Q }\n}\n",
        ),
        (
            "c.md",
            "<!-- a -->\n// not a comment\nText here. More text.\n\n# H // x\n",
        ),
        ("c.txt", "# comment-like\n// x\n\n/* y */ z\n"),
        ("c.cfg", "; a\n# b\nkey = 1 // c\n"),
    ];
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mut files: Vec<String> = Vec::new();
    for (path, text) in crafted {
        write(root, path, text);
        files.push((*path).to_owned());
    }
    // Every fixture file under cases/fixtures.
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cases/fixtures");
    let mut stack = vec![fixtures.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p
                    .strip_prefix(&fixtures)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                let dest = format!("fx/{rel}");
                write(root, &dest, fs::read(&p).unwrap());
                files.push(dest);
            }
        }
    }
    let mut checked = 0;
    for path in &files {
        let l = format!("file:{path}");
        let mut p = wt(root);
        let units = match p.units(&loc(&l), None) {
            Ok(u) => u,
            // A fixture of a code class that does not parse has no units
            // (section 6.1).
            Err(e) if e.code == ProviderErrorCode::Unparsable => {
                continue;
            }
            Err(e) => panic!("{path}: {e}"),
        };
        // Comment spans of a file with a parse tree.
        let text = fs::read_to_string(root.join(path))
            .unwrap()
            .replace("\r\n", "\n");
        let class = hibi::code::Class::from_path(path);
        let comments = hibi::code::parse(&text, class)
            .filter(|t| !t.root_node().has_error())
            .map(|t| hibi::code::comment_spans(t.root_node()))
            .unwrap_or_default();
        for u in units.iter().filter(|u| u.kind != "file") {
            let start = u.range.unwrap().start;
            assert!(
                !comments.iter().any(|&(s, e)| s <= start && start < e),
                "{path}: unit {} starts inside a comment",
                u.key
            );
            let got = resolve(&mut p, &l, serde_json::to_value(&u.sel).unwrap()).unwrap();
            let keys: Vec<&str> = got.iter().map(|g| g.key.as_str()).collect();
            // A unit that no anchor selects alone is marked (section 6.7,
            // stated limits), and its anchor does not select it alone.
            if u.refuse == Some(Refuse::QuoteNotDistinguishable) {
                assert!(
                    keys != [u.key.as_str()] || got[0].approx,
                    "{path}: {} is marked, but its anchor selects it alone",
                    u.key
                );
                checked += 1;
                continue;
            }
            assert_eq!(keys, [u.key.as_str()], "{path}: {} with {:?}", u.key, u.sel);
            assert!(
                got[0].refuse.is_none() && !got[0].approx,
                "{path}: {} with {:?}: {:?}",
                u.key,
                u.sel,
                got[0]
            );
            assert_eq!(got[0].sel, u.sel, "{path}: {}", u.key);
            checked += 1;
        }
    }
    assert!(checked > 800, "{checked} units checked");
}

#[test]
fn r15_reflowing_a_paragraph_keeps_sentence_fingerprints() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let anchor = json!({"quote": "Pro costs $29 per"});
    let fp = |text: &str| {
        write(root, "docs/pricing.md", text);
        let u = one(&mut wt(root), "file:docs/pricing.md", anchor.clone());
        assert_eq!(u.kind, "sentence");
        assert_eq!(u.fp.profile(), "prose-v1");
        u.fp
    };
    let base = fp("# Plans\n\nPro costs $29 per month for each seat. Teams cost more.\n");
    assert_eq!(
        fp("# Plans\n\nPro costs $29 per\nmonth for   each\n  seat. Teams\ncost more.\n"),
        base
    );
    assert_ne!(
        fp("# Plans\n\nPro costs $29 per month for every seat. Teams cost more.\n"),
        base
    );
    assert_eq!(
        fp("# Plans\n\nPro costs $29 per month for each seat. Teams cost more.\n"),
        base
    );
}

// ---------------------------------------------------------------------------
// Classes, decoding, size, nesting
// ---------------------------------------------------------------------------

#[test]
fn javascript_extensions_use_the_tsx_grammar() {
    let tmp = tempfile::tempdir().unwrap();
    let text = "export function f(a) {\n  return <b>{a}</b>;\n}\n";
    let mut fps = Vec::new();
    for ext in ["tsx", "js", "jsx", "mjs", "cjs", "JS"] {
        write(tmp.path(), &format!("a.{ext}"), text);
        let u = one(
            &mut wt(tmp.path()),
            &format!("file:a.{ext}"),
            json!({"symbol": "f"}),
        );
        assert_eq!(
            (u.kind.as_str(), u.fp.profile()),
            ("function", "ast-v1"),
            "{ext}"
        );
        fps.push(u.fp);
    }
    assert!(
        fps.windows(2).all(|w| w[0] == w[1]),
        "every extension hashes as class tsx"
    );
    // The class is part of `ast-v1`: the same declaration as typescript differs.
    write(
        tmp.path(),
        "b.ts",
        "export function f(a) {\n  return a;\n}\n",
    );
    write(
        tmp.path(),
        "b.js",
        "export function f(a) {\n  return a;\n}\n",
    );
    let ts = one(&mut wt(tmp.path()), "file:b.ts", json!({"symbol": "f"})).fp;
    let js = one(&mut wt(tmp.path()), "file:b.js", json!({"symbol": "f"})).fp;
    assert_ne!(ts, js);
}

#[test]
fn decoding_crlf_bom_and_not_text() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "crlf.md", "One here.\r\nTwo here.\r\n");
    write(root, "lf.md", "One here.\nTwo here.\n");
    let crlf = wt(root).units(&loc("file:crlf.md"), None).unwrap();
    let lf = wt(root).units(&loc("file:lf.md"), None).unwrap();
    assert_eq!(
        crlf, lf,
        "CRLF is LF after decoding: same ranges, keys and selectors"
    );
    let u = one(&mut wt(root), "file:crlf.md", json!({"quote": "Two"}));
    assert!(!u.text.unwrap().contains('\r'));

    // A lone CR is kept.
    write(root, "cr.txt", "a\rb\n");
    assert_eq!(
        wt(root).render(&loc("file:cr.txt"), "file").unwrap().text,
        "a\rb\n"
    );

    // A leading BOM stays in the text and belongs to the first unit.
    write(root, "bom.md", "\u{feff}First one. Second one.\n");
    let units = wt(root).units(&loc("file:bom.md"), None).unwrap();
    assert_eq!(units[1].range.unwrap().start, 0);
    assert_eq!(units[1].sel.quote.as_deref(), Some("\u{feff}First one."));

    // Not UTF-8 is not_text; NUL bytes in valid UTF-8 are text.
    write(root, "bin.dat", [0x66u8, 0xff, 0xfe, 0x00]);
    assert_eq!(
        code_of(resolve(&mut wt(root), "file:bin.dat", json!({}))),
        ProviderErrorCode::NotText
    );
    assert_eq!(
        wt(root).units(&loc("file:bin.dat"), None).unwrap_err().code,
        ProviderErrorCode::NotText
    );
    write(root, "nul.dat", "a\0b\n");
    assert_eq!(one(&mut wt(root), "file:nul.dat", json!({})).kind, "file");
}

#[test]
fn r49_size_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    assert_eq!(MAX_FILE_BYTES, 8_388_608, "the cap of section 6.1");
    let at_cap = vec![b'a'; MAX_FILE_BYTES as usize];
    write(root, "at-cap.cfg", &at_cap);
    assert_eq!(
        one(&mut wt(root), "file:at-cap.cfg", json!({}))
            .range
            .unwrap()
            .end,
        MAX_FILE_BYTES as usize
    );
    let mut over = at_cap;
    over.push(b'a');
    write(root, "over.cfg", &over);
    // Unreadable as well: the size check happens before the file is opened.
    fs::set_permissions(root.join("over.cfg"), fs::Permissions::from_mode(0o000)).unwrap();
    assert_eq!(
        code_of(resolve(&mut wt(root), "file:over.cfg", json!({}))),
        ProviderErrorCode::TooLarge
    );
    assert_eq!(
        wt(root)
            .units(&loc("file:over.cfg"), None)
            .unwrap_err()
            .code,
        ProviderErrorCode::TooLarge
    );
    fs::set_permissions(root.join("over.cfg"), fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn r49_deep_nesting_does_not_overflow() {
    const DEPTH: usize = 100_000;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let files = [
        (
            "deep.ts",
            format!("const a = {}1{};\n", "[".repeat(DEPTH), "]".repeat(DEPTH)),
        ),
        (
            "deep.py",
            format!("a = {}1{}\n", "(".repeat(DEPTH), ")".repeat(DEPTH)),
        ),
        (
            "deep.rs",
            format!(
                "const A: i32 = {}1{};\n",
                "(".repeat(DEPTH),
                ")".repeat(DEPTH)
            ),
        ),
        ("deep.md", format!("{}deep 1\n", "> ".repeat(DEPTH))),
    ];
    for (path, text) in &files {
        write(root, path, text);
    }
    // Run on a thread with the default 2 MiB stack.
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn({
            let root = root.to_path_buf();
            move || {
                for (path, _) in files {
                    let l = format!("file:{path}");
                    let mut p = wt(&root);
                    let units = p.units(&loc(&l), None).unwrap();
                    assert!(!units.is_empty(), "{path}");
                    let u = one(&mut p, &l, json!({"quote": "1"}).clone());
                    p.render(&loc(&l), &u.key).unwrap();
                }
            }
        })
        .unwrap()
        .join()
        .expect("no stack overflow");
}

// ---------------------------------------------------------------------------
// units, render, errors
// ---------------------------------------------------------------------------

#[test]
fn units_kind_filter_and_render() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        root,
        "doc.md",
        "# Top\n\n## Sub\n\nA sentence here. Another.\n\n- item\n",
    );
    write(
        root,
        "lib.rs",
        "struct S;\nimpl S {\n    fn m(&self) {}\n}\nfn free() {}\n",
    );
    let mut p = wt(root);
    let all = p.units(&loc("file:doc.md"), None).unwrap();
    assert_eq!(all[0].key, "file");
    assert_eq!(all[0].parent, None);
    assert!(
        all.iter()
            .all(|u| u.text.is_none() && !u.approx && u.refuse.is_none())
    );
    assert!(all[1..].iter().all(|u| u.parent.as_deref() == Some("file")));
    let sentences = p.units(&loc("file:doc.md"), Some("sentence")).unwrap();
    assert_eq!(sentences.len(), 2);
    assert!(sentences.iter().all(|u| u.kind == "sentence"));
    assert!(
        p.units(&loc("file:doc.md"), Some("method"))
            .unwrap()
            .is_empty()
    );
    // Every unit's own selector resolves to exactly that unit.
    for u in &all {
        let got = one(&mut p, "file:doc.md", serde_json::to_value(&u.sel).unwrap());
        assert_eq!(got.key, u.key);
    }

    let r = p.render(&loc("file:doc.md"), &sentences[0].key).unwrap();
    assert_eq!(
        (r.text.as_str(), r.context.clone()),
        ("A sentence here.", vec!["Top".to_owned(), "Sub".to_owned()])
    );
    assert_eq!(
        p.render(&loc("file:doc.md"), "file").unwrap().context,
        Vec::<String>::new()
    );

    let m = p.render(&loc("file:lib.rs"), "method:S.m").unwrap();
    assert_eq!(
        (m.text.as_str(), m.context.clone()),
        ("fn m(&self) {}", vec!["impl S".to_owned()])
    );
    assert!(
        p.render(&loc("file:lib.rs"), "function:free")
            .unwrap()
            .context
            .is_empty()
    );
    assert_eq!(
        p.render(&loc("file:lib.rs"), "function:nope")
            .unwrap_err()
            .code,
        ProviderErrorCode::UnknownUnit
    );
    let labels: BTreeMap<String, Option<String>> = p
        .units(&loc("file:lib.rs"), None)
        .unwrap()
        .into_iter()
        .map(|u| (u.key, u.label))
        .collect();
    assert_eq!(labels["method:S.m"].as_deref(), Some("S.m"));
}

#[test]
fn r51_missing_resource_versus_no_unit_and_other_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "a.md", "Hello there.\n");
    fs::create_dir(root.join("dir.md")).unwrap();
    let mut p = wt(root);
    assert_eq!(
        code_of(resolve(&mut p, "file:absent.md", json!({}))),
        ProviderErrorCode::NotFound
    );
    assert_eq!(
        code_of(resolve(&mut p, "file:a.md/x", json!({}))),
        ProviderErrorCode::NotFound
    );
    assert_eq!(
        code_of(resolve(&mut p, "file:dir.md", json!({}))),
        ProviderErrorCode::NotFound
    );
    assert!(
        resolve(
            &mut p,
            "file:a.md",
            json!({"quote": "absent words entirely"})
        )
        .unwrap()
        .is_empty()
    );
    assert_eq!(
        code_of(resolve(&mut p, "ref:notes", json!({}))),
        ProviderErrorCode::InvalidLocator
    );
    assert_eq!(
        code_of(resolve(&mut p, "file:a.md", json!({"key": "file"}))),
        ProviderErrorCode::InvalidSelector
    );
    assert_eq!(
        p.units(&loc("file:absent.md"), None).unwrap_err().code,
        ProviderErrorCode::NotFound
    );
    assert_eq!(
        p.render(&loc("file:absent.md"), "file").unwrap_err().code,
        ProviderErrorCode::NotFound
    );
    // A batch answers per anchor, in order.
    let batch = p.resolve(&[
        Anchor::new(loc("file:a.md"), Selector::empty()),
        Anchor::new(loc("file:absent.md"), Selector::empty()),
        Anchor::new(loc("file:a.md"), sel(json!({"quote": "there"}))),
    ]);
    assert_eq!(batch.len(), 3);
    assert_eq!(batch[0].as_ref().unwrap()[0].key, "file");
    assert_eq!(
        batch[1].as_ref().unwrap_err().code,
        ProviderErrorCode::NotFound
    );
    assert_eq!(batch[2].as_ref().unwrap()[0].kind, "sentence");
    let info = p.info().unwrap();
    assert_eq!(info.schemes, ["file", "repo"]);
    assert_eq!(
        serde_json::to_value(&info.selectors).unwrap(),
        json!(["quote", "symbol"])
    );
}

// ---------------------------------------------------------------------------
// R47: symbolic links, working tree
// ---------------------------------------------------------------------------

#[test]
fn r47_symbolic_links_in_the_working_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let outside = tmp.path().join("outside");
    write(&root, "real/a.md", "Inside text.\n");
    write(&outside, "o.md", "Outside text.\n");
    std::os::unix::fs::symlink("a.md", root.join("real/link.md")).unwrap();
    std::os::unix::fs::symlink("real", root.join("linkdir")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("out")).unwrap();
    std::os::unix::fs::symlink(outside.join("o.md"), root.join("o.md")).unwrap();
    let mut p = wt(&root);
    assert_eq!(one(&mut p, "file:real/a.md", json!({})).kind, "file");
    for l in [
        "file:real/link.md",
        "file:linkdir/a.md",
        "file:out/o.md",
        "file:o.md",
    ] {
        assert_eq!(
            code_of(resolve(&mut p, l, json!({}))),
            ProviderErrorCode::NotFound,
            "{l}"
        );
        assert_eq!(
            p.units(&loc(l), None).unwrap_err().code,
            ProviderErrorCode::NotFound,
            "{l}"
        );
    }
    // Components above the root are not examined: a root reached through a
    // link works.
    let via = tmp.path().join("via");
    std::os::unix::fs::symlink(&root, &via).unwrap();
    assert_eq!(one(&mut wt(&via), "file:real/a.md", json!({})).kind, "file");
}

// ---------------------------------------------------------------------------
// Revision source
// ---------------------------------------------------------------------------

fn head(root: &Path) -> ObjectId {
    ObjectId::new(sh_git(root, &["rev-parse", "HEAD"]).trim()).unwrap()
}

#[test]
fn revision_reads_the_base_commit_and_applies_the_path_rule() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    fs::create_dir(&root).unwrap();
    sh_git(&root, &["init", "-q"]);
    write(&root, "docs/a.md", "Alpha at base. More.\n");
    write(&root, "docs/b.md", "Beta at base.\n");
    write(&root, "src/x.rs", "fn x() -> u32 { 1 }\n");
    std::os::unix::fs::symlink("a.md", root.join("docs/link.md")).unwrap();
    std::os::unix::fs::symlink("docs", root.join("linkdir")).unwrap();
    sh_git(&root, &["add", "-A"]);
    sh_git(&root, &["commit", "-qm", "base"]);
    let base = head(&root);
    let base_x = one(&mut wt(&root), "file:src/x.rs", json!({"symbol": "x"})).fp;
    // Change and delete in the working tree after the commit.
    write(&root, "docs/a.md", "Alpha changed. More.\n");
    fs::remove_file(root.join("docs/b.md")).unwrap();
    write(&root, "src/x.rs", "fn x() -> u32 { 2 }\n");
    write(&root, "new.md", "Only in the working tree.\n");

    let git = isolated_git(tmp.path());
    let mut rev = FileProvider::revision(&root, base, options(git)).unwrap();
    let a = one(&mut rev, "file:docs/a.md", json!({"quote": "Alpha"}));
    assert_eq!(a.text.as_deref(), Some("Alpha at base."));
    assert_eq!(
        one(&mut rev, "file:docs/b.md", json!({})).text.as_deref(),
        Some("Beta at base.\n")
    );
    assert_eq!(
        one(&mut rev, "file:src/x.rs", json!({"symbol": "x"})).fp,
        base_x
    );
    assert_eq!(
        rev.render(&loc("file:docs/a.md"), "file").unwrap().text,
        "Alpha at base. More.\n"
    );
    assert_eq!(
        rev.units(&loc("file:docs/b.md"), Some("sentence"))
            .unwrap()
            .len(),
        1
    );
    for l in [
        "file:new.md",
        "file:docs/link.md",
        "file:linkdir/a.md",
        "file:docs",
        "file:docs/a.md/x",
    ] {
        assert_eq!(
            code_of(resolve(&mut rev, l, json!({}))),
            ProviderErrorCode::NotFound,
            "{l}"
        );
    }
    assert_eq!(
        code_of(resolve(&mut rev, "repo:api//a.md", json!({}))),
        ProviderErrorCode::InvalidLocator
    );
    // A batch with several paths at the base.
    let batch = rev.resolve(&[
        Anchor::new(loc("file:docs/b.md"), Selector::empty()),
        Anchor::new(loc("file:new.md"), Selector::empty()),
        Anchor::new(loc("file:docs/a.md"), Selector::empty()),
    ]);
    assert!(batch[0].is_ok() && batch[2].is_ok());
    assert_eq!(
        batch[1].as_ref().unwrap_err().code,
        ProviderErrorCode::NotFound
    );

    // The working tree sees the changes.
    let mut now = wt(&root);
    assert_eq!(
        code_of(resolve(&mut now, "file:docs/b.md", json!({}))),
        ProviderErrorCode::NotFound
    );
    assert_ne!(
        one(&mut now, "file:src/x.rs", json!({"symbol": "x"})).fp,
        base_x
    );
}

/// A TypeScript file of exactly `len` bytes that parses into one
/// declaration: a single string literal, which parses in a few nodes, so
/// the boundary is checked without the cost of a large tree.
fn literal_file(len: usize) -> String {
    let (head, tail) = ("const a = \"", "\";\n");
    let fill = len - head.len() - tail.len();
    let text = format!("{head}{}{tail}", "x".repeat(fill));
    assert_eq!(text.len(), len);
    text
}

#[test]
fn code_files_have_a_lower_size_cap() {
    // Ruling on section 6.1: a file of a code class longer than
    // MAX_CODE_FILE_BYTES is too_large, in both sources, before it is
    // parsed; markdown, prose and text keep MAX_FILE_BYTES.
    assert_eq!(
        MAX_CODE_FILE_BYTES, 2_097_152,
        "the code cap of section 6.1"
    );
    let cap = MAX_CODE_FILE_BYTES as usize;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    fs::create_dir(&root).unwrap();
    write(&root, "at.ts", literal_file(cap));
    write(&root, "over.ts", literal_file(cap + 1));
    write(&root, "over.md", "word ".repeat(cap / 5 + 1));
    sh_git(&root, &["init", "-q"]);
    sh_git(&root, &["add", "-A"]);
    sh_git(&root, &["commit", "-qm", "base"]);
    let base = head(&root);
    let git = isolated_git(tmp.path());
    let mut sources = [
        ("working tree", wt(&root)),
        (
            "revision",
            FileProvider::revision(&root, base, options(git)).unwrap(),
        ),
    ];
    for (what, p) in &mut sources {
        let u = one(p, "file:at.ts", json!({"symbol": "a"}));
        assert_eq!(u.key, "variable:a", "{what}");
        let e = resolve(p, "file:over.ts", json!({})).unwrap_err();
        assert_eq!(e.code, ProviderErrorCode::TooLarge, "{what}");
        assert!(
            e.message.contains("2097153 bytes")
                && e.message.contains("2097152 bytes for a code file"),
            "{what}: {}",
            e.message
        );
        assert_eq!(
            p.units(&loc("file:over.ts"), None).unwrap_err().code,
            ProviderErrorCode::TooLarge,
            "{what}"
        );
        assert!(
            fs::metadata(root.join("over.md")).unwrap().len() > MAX_CODE_FILE_BYTES,
            "the markdown file is over the code cap"
        );
        assert_eq!(
            one(p, "file:over.md", json!({})).key,
            "file",
            "{what}: markdown keeps the 8 MiB cap"
        );
    }
}

#[test]
fn revision_size_cap_and_git_requirements() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    fs::create_dir(&root).unwrap();
    sh_git(&root, &["init", "-q"]);
    write(&root, "big.cfg", vec![b'a'; MAX_FILE_BYTES as usize + 1]);
    write(&root, "bad.dat", [0xffu8, 0xfe]);
    sh_git(&root, &["add", "-A"]);
    sh_git(&root, &["commit", "-qm", "base"]);
    let base = head(&root);
    let git = isolated_git(tmp.path());
    let mut rev = FileProvider::revision(&root, base.clone(), options(git)).unwrap();
    assert_eq!(
        code_of(resolve(&mut rev, "file:big.cfg", json!({}))),
        ProviderErrorCode::TooLarge
    );
    assert_eq!(
        code_of(resolve(&mut rev, "file:bad.dat", json!({}))),
        ProviderErrorCode::NotText
    );

    let missing = FileProvider::revision(&root, base.clone(), options(no_git()))
        .err()
        .unwrap();
    assert_eq!(missing.code, Code::NotAGitRepository);
    let old = GitStatus::from_version_output("git", "git version 2.30.0");
    let too_old = FileProvider::revision(&root, base, options(old))
        .err()
        .unwrap();
    assert_eq!(too_old.code, Code::GitTooOld);
}

// ---------------------------------------------------------------------------
// R45, R46, R47: sibling repositories
// ---------------------------------------------------------------------------

struct Siblings {
    _tmp: tempfile::TempDir,
    tmp: PathBuf,
    root: PathBuf,
    api: PathBuf,
}

/// A root repository `shop` and a sibling `api` with a tracked file, an
/// untracked file, an ignored file that is too large and unreadable, and a
/// tracked symbolic link.
fn siblings() -> Siblings {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let root = base.join("shop");
    let api = base.join("api");
    for d in [&root, &api] {
        fs::create_dir(d).unwrap();
        sh_git(d, &["init", "-q"]);
    }
    write(&api, "docs/listed.md", "Listed text here.\n");
    write(&api, ".gitignore", "secret.md\n");
    std::os::unix::fs::symlink("docs/listed.md", api.join("link.md")).unwrap();
    sh_git(&api, &["add", "-A"]);
    sh_git(&api, &["commit", "-qm", "base"]);
    write(&api, "untracked.md", "Untracked but not ignored.\n");
    write(&api, "secret.md", vec![b'a'; MAX_FILE_BYTES as usize + 1]);
    fs::set_permissions(api.join("secret.md"), fs::Permissions::from_mode(0o000)).unwrap();
    Siblings {
        tmp: base,
        _tmp: tmp,
        root,
        api,
    }
}

fn repo_config(alias: &str) -> RepoConfig {
    let mut c = RepoConfig::default();
    c.repos.insert(alias.into(), format!("../{alias}"));
    c
}

fn entry(root: &Path, alias: &str, path: &Path) -> RepoEntry {
    RepoEntry {
        root: root.to_path_buf(),
        alias: alias.into(),
        path: path.to_path_buf(),
    }
}

fn sibling_provider(
    s: &Siblings,
    repo: RepoConfig,
    entries: Vec<RepoEntry>,
    git: GitStatus,
) -> FileProvider {
    FileProvider::working_tree(
        &s.root,
        FileOptions {
            repo,
            user: UserConfig {
                providers: BTreeMap::new(),
                repos: entries,
            },
            git,
            cache: None,
        },
    )
}

#[test]
fn r46_r47_allowed_sibling_reads_only_listed_regular_files() {
    let s = siblings();
    let git = isolated_git(&s.tmp);
    let mut p = sibling_provider(
        &s,
        repo_config("api"),
        vec![entry(&s.root, "api", &s.api)],
        git,
    );
    let u = one(
        &mut p,
        "repo:api//docs/listed.md",
        json!({"quote": "Listed"}),
    );
    assert_eq!(u.kind, "sentence");
    assert_eq!(
        one(&mut p, "repo:api//untracked.md", json!({})).kind,
        "file"
    );
    // R46: ignored, so not listed. It is larger than the cap and unreadable:
    // opening or measuring it would give too_large or unavailable.
    assert_eq!(
        code_of(resolve(&mut p, "repo:api//secret.md", json!({}))),
        ProviderErrorCode::NotFound
    );
    assert_eq!(
        p.units(&loc("repo:api//secret.md"), None).unwrap_err().code,
        ProviderErrorCode::NotFound
    );
    assert_eq!(
        code_of(resolve(&mut p, "repo:api//absent.md", json!({}))),
        ProviderErrorCode::NotFound
    );
    // R47: a listed path that is a symbolic link.
    assert_eq!(
        code_of(resolve(&mut p, "repo:api//link.md", json!({}))),
        ProviderErrorCode::NotFound
    );
    assert_eq!(
        p.render(&loc("repo:api//docs/listed.md"), "file")
            .unwrap()
            .text,
        "Listed text here.\n"
    );
    fs::set_permissions(s.api.join("secret.md"), fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn r45_refused_siblings_open_nothing() {
    let s = siblings();
    let git = isolated_git(&s.tmp);
    // Make the listed file unreadable: any read would be `unavailable`.
    fs::set_permissions(
        s.api.join("docs/listed.md"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    let plain = s.tmp.join("plain");
    fs::create_dir(&plain).unwrap();
    write(&plain, "docs/listed.md", "x\n");
    fs::create_dir(s.root.join("inner")).unwrap();
    let l = "repo:api//docs/listed.md";
    let cases: Vec<(
        &str,
        RepoConfig,
        Vec<RepoEntry>,
        GitStatus,
        ProviderErrorCode,
    )> = vec![
        (
            "no [[repo]] entry",
            repo_config("api"),
            vec![],
            git.clone(),
            ProviderErrorCode::RepoNotAllowed,
        ),
        (
            "entry for another root",
            repo_config("api"),
            vec![entry(&s.api, "api", &s.api)],
            git.clone(),
            ProviderErrorCode::RepoNotAllowed,
        ),
        (
            "directory is the root",
            repo_config("api"),
            vec![entry(&s.root, "api", &s.root)],
            git.clone(),
            ProviderErrorCode::RepoNotAllowed,
        ),
        (
            "directory inside the root",
            repo_config("api"),
            vec![entry(&s.root, "api", &s.root.join("inner"))],
            git.clone(),
            ProviderErrorCode::RepoNotAllowed,
        ),
        (
            "directory above the root",
            repo_config("api"),
            vec![entry(&s.root, "api", &s.tmp)],
            git.clone(),
            ProviderErrorCode::RepoNotAllowed,
        ),
        (
            "directory not a git top level",
            repo_config("api"),
            vec![entry(&s.root, "api", &plain)],
            git.clone(),
            ProviderErrorCode::RepoNotAllowed,
        ),
        (
            "directory inside the sibling",
            repo_config("api"),
            vec![entry(&s.root, "api", &s.api.join("docs"))],
            git.clone(),
            ProviderErrorCode::RepoNotAllowed,
        ),
        (
            "alias not declared",
            RepoConfig::default(),
            vec![entry(&s.root, "api", &s.api)],
            git.clone(),
            ProviderErrorCode::RepoNotFound,
        ),
        (
            "directory missing",
            repo_config("api"),
            vec![entry(&s.root, "api", &s.tmp.join("gone"))],
            git.clone(),
            ProviderErrorCode::RepoNotFound,
        ),
        (
            "git missing",
            repo_config("api"),
            vec![entry(&s.root, "api", &s.api)],
            no_git(),
            ProviderErrorCode::RepoNotFound,
        ),
        (
            "git too old",
            repo_config("api"),
            vec![entry(&s.root, "api", &s.api)],
            GitStatus::from_version_output("git", "git version 2.44.9"),
            ProviderErrorCode::RepoNotFound,
        ),
    ];
    for (what, repo, entries, git, want) in cases {
        let mut p = sibling_provider(&s, repo, entries, git);
        assert_eq!(code_of(resolve(&mut p, l, json!({}))), want, "{what}");
        assert_eq!(p.units(&loc(l), None).unwrap_err().code, want, "{what}");
        assert_eq!(p.render(&loc(l), "file").unwrap_err().code, want, "{what}");
    }
    // The same alias, allowed, reaches the read: the file is unreadable.
    let mut p = sibling_provider(
        &s,
        repo_config("api"),
        vec![entry(&s.root, "api", &s.api)],
        git,
    );
    assert_unavailable_ending(
        &resolve(&mut p, l, json!({})).unwrap_err(),
        "; make it readable by this user",
    );
    fs::set_permissions(
        s.api.join("docs/listed.md"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    fs::set_permissions(s.api.join("secret.md"), fs::Permissions::from_mode(0o644)).unwrap();
}

// ---------------------------------------------------------------------------
// R40: the cache changes no result
// ---------------------------------------------------------------------------

/// A cache that counts reads that hit.
struct Counting {
    inner: Rc<Cache>,
    hits: RefCell<usize>,
}

impl UnitCache for Counting {
    fn get_units(&self, blob: &str, class: &str) -> Option<String> {
        let r = self.inner.get_units(blob, class);
        if r.is_some() {
            *self.hits.borrow_mut() += 1;
        }
        r
    }
    fn put_units(&self, blob: &str, class: &str, units: &str) {
        self.inner.put_units(blob, class, units)
    }
}

const CORPUS: &[(&str, &str)] = &[
    (
        "docs/a.md",
        "---\nt: x\n---\n# Top\n\nOne here. Two here.\n\n## Sub\n\n- item one\n| a | b |\n|---|---|\n| c | d |\n",
    ),
    (
        "src/lib.rs",
        "// lead\nuse std::fmt;\npub struct S; // c\nimpl S {\n    /// doc\n    pub fn m(&self) -> u32 { 1 } // tail\n}\nfn f() {}\n",
    ),
    (
        "src/a.ts",
        "export function f(a: string): void;\nexport function f(a: any) { return a; }\nclass C { m(/* x */ y) { return y; } }\nconst g = () => 1;\n",
    ),
    (
        "a.py",
        "import os\n\nclass K:\n    \"\"\"Doc.\"\"\"\n    def m(self):  # c\n        return 1\n\nX = 2\n",
    ),
    (
        "a.go",
        "package p\n\ntype T struct{}\n\nfunc (t *T) M() int { return 1 } // c\n",
    ),
    (
        "A.java",
        "class A {\n  /** doc */\n  int f = 1;\n  void m() {}\n}\n",
    ),
    ("bad.py", "def f(:\n  pass\n"),
    ("notes.cfg", "a = 1\n\nb = 2\n"),
    ("crlf.txt", "One.\r\nTwo.\r\n"),
];

type Answers = Vec<Value>;

fn answers(p: &mut FileProvider) -> Answers {
    let mut out = Vec::new();
    for (path, _) in CORPUS {
        let l = loc(&format!("file:{path}"));
        // A code file that does not parse (`bad.py`) is `unavailable`.
        let units = match p.units(&l, None) {
            Ok(units) => units,
            Err(e) => {
                out.push(serde_json::to_value(e).unwrap());
                continue;
            }
        };
        out.push(serde_json::to_value(&units).unwrap());
        for u in &units {
            out.push(serde_json::to_value(p.render(&l, &u.key).unwrap()).unwrap());
            for s in [
                serde_json::to_value(&u.sel).unwrap(),
                json!({"quote": "1"}),
                json!({"quote": "c"}),
                json!({"symbol": "S.m", "quote": "tail"}),
            ] {
                let r = p.resolve(&[Anchor::new(l.clone(), sel(s))]).remove(0);
                out.push(match r {
                    Ok(us) => serde_json::to_value(us).unwrap(),
                    Err(e) => serde_json::to_value(e).unwrap(),
                });
            }
        }
    }
    out.push(serde_json::to_value(p.units(&loc("file:bin.dat"), None).unwrap_err()).unwrap());
    out
}

#[test]
fn r40_no_cache_cold_cache_and_warm_cache_agree() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    for (path, text) in CORPUS {
        write(&root, path, text);
    }
    write(&root, "bin.dat", [0xffu8]);
    let plain = answers(&mut wt(&root));

    let db = Rc::new(Cache::open(&tmp.path().join("cache/cache.sqlite3")).unwrap());
    let counting = Rc::new(Counting {
        inner: db.clone(),
        hits: RefCell::new(0),
    });
    let with_cache = |c: &Rc<Counting>| {
        let mut o = options(no_git());
        o.cache = Some(Box::new(c.clone()));
        FileProvider::working_tree(&root, o)
    };
    let cold = answers(&mut with_cache(&counting));
    let hits_after_cold = *counting.hits.borrow();
    let warm = answers(&mut with_cache(&counting));
    assert!(
        *counting.hits.borrow() > hits_after_cold,
        "the warm run reads the cache"
    );
    assert_eq!(cold, plain, "cold cache");
    assert_eq!(warm, plain, "warm cache");

    // A damaged row is ignored and rewritten.
    // A code file that does not parse gets no row, so its damaged row stays.
    let mut rows: HashMap<String, (String, bool)> = HashMap::new();
    for (path, text) in CORPUS {
        let blob = hibi::git::blob_id(text.as_bytes());
        let class = hibi::code::Class::from_path(path).name();
        Cache::put_units(&db, &blob, class, "[{\"not\":\"a unit\"}]").unwrap();
        rows.insert(blob, (class.to_owned(), *path == "bad.py"));
    }
    let damaged = answers(&mut with_cache(&counting));
    assert_eq!(damaged, plain, "damaged rows");
    for (blob, (class, broken)) in rows {
        let row = db.units(&blob, &class).unwrap().unwrap();
        if broken {
            assert_eq!(row, "[{\"not\":\"a unit\"}]");
        } else {
            assert!(row.starts_with("[{\"key\":\"file\""));
        }
    }
}

// ---------------------------------------------------------------------------
// Reads limited to git's file list (ruling on section 6.8)
// ---------------------------------------------------------------------------

/// A repository with a tracked README, an ignored `.env`, an untracked file
/// that is not ignored, and a store file under `.hibi/`.
fn listed_repo(tmp: &Path) -> PathBuf {
    let root = tmp.join("repo");
    fs::create_dir(&root).unwrap();
    sh_git(&root, &["init", "-q"]);
    write(&root, "README.md", "Tracked text.\n");
    write(&root, ".gitignore", ".env\n");
    write(&root, ".hibi/hibi.toml", "version = 1\n");
    sh_git(&root, &["add", "-A"]);
    sh_git(&root, &["commit", "-qm", "base"]);
    write(&root, ".env", "API_TOKEN=second-secret-value\n");
    write(&root, "notes.md", "Untracked but not ignored.\n");
    root
}

#[test]
fn file_reads_follow_the_root_file_list_when_git_is_usable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = listed_repo(tmp.path());
    let git = isolated_git(tmp.path());
    let mut p = FileProvider::working_tree(&root, options(git.clone()));
    assert_eq!(one(&mut p, "file:README.md", json!({})).kind, "file");
    assert_eq!(one(&mut p, "file:notes.md", json!({})).kind, "file");
    let e = resolve(&mut p, "file:.env", json!({"quote": "API_TOKEN"})).unwrap_err();
    assert_eq!(e.code, ProviderErrorCode::NotFound);
    assert!(e.message.contains("file list"), "{}", e.message);
    assert!(!e.message.contains("second-secret"), "{}", e.message);
    for l in [
        "file:.git/config",
        "file:.GIT/config",
        "file:.hibi/hibi.toml",
        "file:README.MD",
    ] {
        assert_eq!(
            code_of(resolve(&mut p, l, json!({}))),
            ProviderErrorCode::NotFound,
            "{l}"
        );
        assert_eq!(
            p.units(&loc(l), None).unwrap_err().code,
            ProviderErrorCode::NotFound,
            "{l}"
        );
    }
    // Without git the working-tree rule stands: the ignored file is read,
    // and .git/ and .hibi/ still are not.
    let mut no = wt(&root);
    assert_eq!(
        one(&mut no, "file:.env", json!({"quote": "API_TOKEN"})).kind,
        "block"
    );
    for l in [
        "file:.git/config",
        "file:.hibi/hibi.toml",
        "file:sub/.git/config",
    ] {
        assert_eq!(
            code_of(resolve(&mut no, l, json!({}))),
            ProviderErrorCode::NotFound,
            "{l}"
        );
    }
    // A revision source never reads .hibi/ either.
    let base = head(&root);
    let mut rev = FileProvider::revision(&root, base, options(git)).unwrap();
    assert_eq!(
        code_of(resolve(&mut rev, "file:.hibi/hibi.toml", json!({}))),
        ProviderErrorCode::NotFound
    );
    assert_eq!(one(&mut rev, "file:README.md", json!({})).kind, "file");
}

/// A failing file list is `unavailable`, never `not_found` (section 6.7,
/// error 2), for the root and for a sibling.
#[test]
fn a_failing_file_list_is_unavailable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = listed_repo(tmp.path());
    let git = isolated_git(tmp.path());
    fs::write(root.join(".git/index"), b"not an index").unwrap();
    let mut p = FileProvider::working_tree(&root, options(git.clone()));
    let e = resolve(&mut p, "file:README.md", json!({})).unwrap_err();
    assert_unavailable_ending(
        &e,
        "; check that the root is a git repository that git can read",
    );

    let s = siblings();
    fs::set_permissions(s.api.join("secret.md"), fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(s.api.join(".git/index"), b"not an index").unwrap();
    let mut p = sibling_provider(
        &s,
        repo_config("api"),
        vec![entry(&s.root, "api", &s.api)],
        isolated_git(&s.tmp),
    );
    let e = resolve(&mut p, "repo:api//docs/listed.md", json!({})).unwrap_err();
    assert_unavailable_ending(&e, " is a git repository that git can read");
    assert!(
        e.message.contains(&s.api.display().to_string()),
        "{}",
        e.message
    );
}

// ---------------------------------------------------------------------------
// Cache rows and selectors (rulings on sections 11.3 and 6.7)
// ---------------------------------------------------------------------------

/// A cache that records every row.
#[derive(Default)]
struct Rows(RefCell<HashMap<(String, String), String>>);

impl UnitCache for Rows {
    fn get_units(&self, blob: &str, class: &str) -> Option<String> {
        self.0
            .borrow()
            .get(&(blob.to_owned(), class.to_owned()))
            .cloned()
    }
    fn put_units(&self, blob: &str, class: &str, units: &str) {
        self.0
            .borrow_mut()
            .insert((blob.to_owned(), class.to_owned()), units.to_owned());
    }
}

fn cached(root: &Path, rows: &Rc<Rows>) -> FileProvider {
    let mut o = options(no_git());
    o.cache = Some(Box::new(rows.clone()));
    FileProvider::working_tree(root, o)
}

#[test]
fn a_not_text_row_for_valid_utf8_is_a_miss_and_is_rewritten() {
    let tmp = tempfile::tempdir().unwrap();
    let text = "Valid text here.\n";
    write(tmp.path(), "a.md", text);
    let rows = Rc::new(Rows::default());
    let key = (hibi::git::blob_id(text.as_bytes()), "markdown".to_owned());
    rows.0.borrow_mut().insert(key.clone(), "not_text".into());
    let u = one(
        &mut cached(tmp.path(), &rows),
        "file:a.md",
        json!({"quote": "Valid"}),
    );
    assert_eq!(u.kind, "sentence");
    assert!(
        rows.0.borrow()[&key].starts_with("[{"),
        "the row is rewritten"
    );
    // Invalid UTF-8 is still not_text, and its row says so.
    write(tmp.path(), "b.md", [0xffu8, 0xfe]);
    assert_eq!(
        code_of(resolve(
            &mut cached(tmp.path(), &rows),
            "file:b.md",
            json!({})
        )),
        ProviderErrorCode::NotText
    );
}

#[test]
fn a_cache_row_larger_than_8_mib_is_not_written() {
    let tmp = tempfile::tempdir().unwrap();
    // 99,999 sentences: within the unit cap, but the row is far over 8 MiB.
    let big = "S.\n\n".repeat(99_999);
    write(tmp.path(), "big.md", &big);
    write(tmp.path(), "small.md", "One sentence.\n");
    let rows = Rc::new(Rows::default());
    let mut p = cached(tmp.path(), &rows);
    assert_eq!(p.units(&loc("file:big.md"), None).unwrap().len(), 100_000);
    p.units(&loc("file:small.md"), None).unwrap();
    let written: Vec<usize> = rows.0.borrow().values().map(String::len).collect();
    assert_eq!(written.len(), 1, "only the small row is written");
    assert!(written[0] <= hibi::provider::file::MAX_CACHE_ROW_BYTES);
    // One more unit is too_large through the provider.
    write(tmp.path(), "over.md", "S.\n\n".repeat(100_000));
    assert_eq!(
        code_of(resolve(&mut p, "file:over.md", json!({}))),
        ProviderErrorCode::TooLarge
    );
}

/// Resolve error 8: a selector that is invalid in another way than its
/// keys cannot arrive over JSON (deserialization refuses it) but can be
/// built through the Rust API, and is `invalid_request`.
#[test]
fn an_invalid_selector_built_in_rust_is_invalid_request() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "a.md", "Text.\n");
    let sel = Selector {
        prefix: Some("x".into()),
        ..Selector::default()
    };
    let r = wt(tmp.path())
        .resolve(&[Anchor::new(loc("file:a.md"), sel)])
        .remove(0);
    assert_eq!(r.unwrap_err().code, ProviderErrorCode::InvalidRequest);
}

// ---------------------------------------------------------------------------
// Symbols on resources without declarations (ruling on section 6.7)
// ---------------------------------------------------------------------------

#[test]
fn a_symbol_on_a_class_without_symbols_is_invalid_selector() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        root,
        "docs/limiter.md",
        "# Backoff\n\nWait twice as long each time.\n",
    );
    write(root, "notes.txt", "Backoff waits.\n");
    write(root, "limits.cfg", "backoff = 2\n");
    for (l, s) in [
        ("file:docs/limiter.md", json!({"symbol": "Backoff::wait"})),
        ("file:docs/limiter.md", json!({"symbol": "Backoff"})),
        (
            "file:docs/limiter.md",
            json!({"symbol": "Backoff", "quote": "Wait twice"}),
        ),
        (
            "file:docs/limiter.md",
            json!({"symbol": "Backoff", "quote": "Wait twice", "prefix": "", "suffix": " as"}),
        ),
        ("file:notes.txt", json!({"symbol": "Backoff"})),
        ("file:limits.cfg", json!({"symbol": "backoff"})),
    ] {
        let e = resolve(&mut wt(root), l, s.clone()).unwrap_err();
        assert_eq!(e.code, ProviderErrorCode::InvalidSelector, "{l} {s}");
        assert!(
            e.message.contains("has no symbols") && e.message.contains("quote"),
            "{l} {s}: {}",
            e.message
        );
    }
    // Without the symbol the quote resolves as usual.
    let u = one(
        &mut wt(root),
        "file:docs/limiter.md",
        json!({"quote": "Wait twice"}),
    );
    assert!(!u.approx);
}

/// Section 6.7, stated limits: the edits next to a claimed sentence that
/// rename its piece (its name is the first 120 characters of the piece's
/// first unit) leave the claim unresolved; an edit that does not name
/// the piece leaves it resolved. The claimed sentence is never the piece's
/// first one.
#[test]
fn neighbour_edits_that_rename_a_piece_turn_its_claims_into_candidates() {
    let renaming = [
        (
            "renumbering an ordered list",
            "# L\n\n1. Install it. Then run it.\n",
            "# L\n\n2. Install it. Then run it.\n",
        ),
        (
            "changing a list marker",
            "# L\n\n- Install it. Then run it.\n",
            "# L\n\n* Install it. Then run it.\n",
        ),
        (
            "reflowing a blockquote",
            "# L\n\n> Install it now. Then\n> run it.\n",
            "# L\n\n> Install it\n> now. Then run it.\n",
        ),
        (
            "deleting a paragraph's first sentence",
            "# L\n\nFirst do this. Install it. Then run it.\n",
            "# L\n\nInstall it. Then run it.\n",
        ),
        (
            "inserting a paragraph's first sentence",
            "# L\n\nInstall it. Then run it.\n",
            "# L\n\nFirst do this. Install it. Then run it.\n",
        ),
        (
            "splitting a paragraph",
            "# L\n\nInstall it. Wait. Then run it.\n",
            "# L\n\nInstall it. Wait.\n\nThen run it.\n",
        ),
    ];
    let keeping = [
        (
            "editing a neighbour that is not the first sentence",
            "# L\n\nInstall it. Wait. Then run it.\n",
            "# L\n\nInstall it. Wait a minute. Then run it.\n",
        ),
        (
            "splitting the paragraph after the claimed sentence",
            "# L\n\nInstall it. Then run it. Wait.\n",
            "# L\n\nInstall it. Then run it.\n\nWait.\n",
        ),
    ];
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for (renames, cases) in [(true, &renaming[..]), (false, &keeping[..])] {
        for (what, before, after) in cases {
            write(root, "d.md", before);
            let mut p = wt(root);
            let claimed = one(&mut p, "file:d.md", json!({"quote": "Then run it"}));
            assert!(claimed.refuse.is_none(), "{what}: {claimed:?}");
            let stored = serde_json::to_value(&claimed.sel).unwrap();
            write(root, "d.md", after);
            let mut p = wt(root);
            let got = resolve(&mut p, "file:d.md", stored).unwrap();
            // Renamed: nothing resolves; the claim has candidates, or none
            // when nothing in the scope is near the quote.
            if renames {
                assert!(
                    got.iter().all(|u| u.approx),
                    "{what}: {:?} gives {got:?}",
                    claimed.sel
                );
            } else {
                assert!(
                    got.len() == 1 && !got[0].approx,
                    "{what}: {:?} gives {got:?}",
                    claimed.sel
                );
            }
        }
    }
}

/// Section 6.1: a file of a code class that does not parse is `unavailable`
/// for `resolve` with any selector, new or stored, for `units` and for
/// `render`; nothing is cached for it, and every stored anchor resolves
/// unchanged once the file parses again.
#[test]
fn a_code_file_that_does_not_parse_is_unavailable_for_every_request() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let good = "def wait(n):\n    return n * 2\n\n\nprint(wait(3))\n";
    write(root, "pkg/limiter.py", good);
    // Stored anchors from the parsed file: a symbol, a quote inside a
    // declaration (stored with `within`), a top-level quote and the whole
    // file.
    let mut stored = Vec::new();
    for (what, s) in [
        ("symbol", json!({"symbol": "wait", "quote": "n * 2"})),
        (
            "quote inside a declaration",
            json!({"quote": "return n * 2"}),
        ),
        ("top-level quote", json!({"quote": "print(wait(3))"})),
        ("whole file", json!({})),
    ] {
        let u = one(&mut wt(root), "file:pkg/limiter.py", s);
        stored.push((what, u.key, serde_json::to_value(&u.sel).unwrap()));
    }
    assert_eq!(stored[1].2["within"], json!(["wait"]), "{stored:?}");
    assert_eq!(stored[2].2.get("within"), None, "{stored:?}");

    // A syntax error on line 8.
    let broken = format!("{good}\n\ndef other(:\n    pass\n");
    write(root, "pkg/limiter.py", &broken);
    let rows = Rc::new(Rows::default());
    let new_anchors = [
        ("new symbol", json!({"symbol": "wait"})),
        ("new quote", json!({"quote": "n * 2"})),
        ("a symbol that never existed", json!({"symbol": "gone"})),
    ];
    let all = stored
        .iter()
        .map(|(w, _, s)| (*w, s.clone()))
        .chain(new_anchors);
    let check = |e: &ProviderError, what: &str| {
        assert_unparsable_ending(e, "; fix the syntax error");
        assert!(
            e.message.contains("does not parse as python") && e.message.contains("line 8"),
            "{what}: {}",
            e.message
        );
    };
    for (what, s) in all {
        // Without a cache, through an empty cache and through it again.
        for p in [
            &mut wt(root),
            &mut cached(root, &rows),
            &mut cached(root, &rows),
        ] {
            check(
                &resolve(p, "file:pkg/limiter.py", s.clone()).unwrap_err(),
                what,
            );
        }
    }
    let l = loc("file:pkg/limiter.py");
    check(&wt(root).units(&l, None).unwrap_err(), "units");
    check(&cached(root, &rows).units(&l, None).unwrap_err(), "units");
    check(&wt(root).render(&l, "file").unwrap_err(), "render");
    assert_eq!(
        rows.0.borrow().len(),
        0,
        "a row was cached for a broken file"
    );

    // Fixed again: every stored anchor resolves to its unit, unchanged.
    write(root, "pkg/limiter.py", good);
    for (what, key, sel) in &stored {
        let u = one(&mut cached(root, &rows), "file:pkg/limiter.py", sel.clone());
        assert_eq!(&u.key, key, "{what}");
        assert!(!u.approx && u.identity.is_none(), "{what}");
        assert_eq!(&serde_json::to_value(&u.sel).unwrap(), sel, "{what}");
    }
}

#[test]
fn a_missed_symbol_only_proposes_candidates() {
    // Ruling on section 6.7: similarity proposes, it never resolves. With a
    // stored symbol that names no declaration, every unit found through the
    // quote is approx, one unit or several; a unique symbol and an exact
    // quote without a symbol are not.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(
        root,
        "src/m.rs",
        "fn a() -> u32 { 1 }\nfn b() -> u32 { 1 }\nfn c() { other() }\n",
    );
    let units = resolve(
        &mut wt(root),
        "file:src/m.rs",
        json!({"symbol": "gone", "quote": "-> u32 { 1 }"}),
    )
    .unwrap();
    let keys: Vec<(&str, bool)> = units.iter().map(|u| (u.key.as_str(), u.approx)).collect();
    assert_eq!(keys, [("function:a", true), ("function:b", true)]);
    let units = resolve(
        &mut wt(root),
        "file:src/m.rs",
        json!({"quote": "-> u32 { 1 }"}),
    )
    .unwrap();
    assert!(units.iter().all(|u| !u.approx));
    let u = one(
        &mut wt(root),
        "file:src/m.rs",
        json!({"symbol": "gone", "quote": "fn c() { other() }"}),
    );
    assert_eq!((u.key.as_str(), u.approx), ("function:c", true));
    let u = one(
        &mut wt(root),
        "file:src/m.rs",
        json!({"symbol": "c", "quote": "fn c() { another() }"}),
    );
    assert_eq!((u.key.as_str(), u.approx), ("function:c", false));
}

/// A cache that counts every lookup, so a test can see when the provider
/// analyses a resource again.
#[derive(Default)]
struct Lookups(RefCell<usize>);

impl UnitCache for Lookups {
    fn get_units(&self, _: &str, _: &str) -> Option<String> {
        *self.0.borrow_mut() += 1;
        None
    }
    fn put_units(&self, _: &str, _: &str, _: &str) {}
}

#[test]
fn an_analysed_document_is_reused_between_calls_until_its_bytes_change() {
    // Ruling on section 6.7: `render`, `units` and `resolve` of one run
    // analyse a resource once while its bytes are unchanged, and never
    // serve a document for bytes that changed.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "a.md", "First sentence. Second sentence.\n");
    write(root, "b.md", "Other file.\n");
    let lookups = Rc::new(Lookups::default());
    let mut o = options(no_git());
    o.cache = Some(Box::new(lookups.clone()));
    let mut p = FileProvider::working_tree(root, o);
    let analyses = || *lookups.0.borrow();

    let units = p.units(&loc("file:a.md"), None).unwrap();
    assert_eq!(analyses(), 1);
    let key = units[1].key.clone();
    p.render(&loc("file:a.md"), &key).unwrap();
    one(&mut p, "file:a.md", json!({"quote": "Second"}));
    p.units(&loc("file:b.md"), None).unwrap();
    p.units(&loc("file:a.md"), None).unwrap();
    assert_eq!(analyses(), 2, "a.md analysed once, b.md once");

    // Changed bytes: analysed again, and the answer is the new text's.
    write(root, "a.md", "First sentence. Third sentence.\n");
    let u = one(&mut p, "file:a.md", json!({"quote": "Third"}));
    assert_eq!(analyses(), 3);
    assert_eq!(u.text.as_deref(), Some("Third sentence."));
    assert!(
        resolve(&mut p, "file:a.md", json!({"quote": "Second"}))
            .unwrap()
            .is_empty()
    );
    // The same length and different bytes are a different document too.
    write(root, "a.md", "First sentence. Fifth sentence.\n");
    assert_eq!(one(&mut p, "file:a.md", json!({"quote": "Fifth"})).key, {
        let units = p.units(&loc("file:a.md"), None).unwrap();
        units[2].key.clone()
    });
    assert_eq!(analyses(), 4);
}
