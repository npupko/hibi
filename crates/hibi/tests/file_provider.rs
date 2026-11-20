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
use hibi::provider::file::{FileOptions, FileProvider, MAX_FILE_BYTES, UnitCache};
use hibi::provider::{Provider, ProviderError, ProviderErrorCode, Unit};
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
fn a_syntax_error_makes_the_file_text() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "bad.py", "def f(:\n    pass\n\nx = 1\n");
    let units = wt(tmp.path()).units(&loc("file:bad.py"), None).unwrap();
    let kinds: Vec<&str> = units.iter().map(|u| u.kind.as_str()).collect();
    assert_eq!(kinds, ["file", "block", "block"]);
    assert!(units.iter().all(|u| u.fp.profile() == "prose-v1"));
    assert!(units.iter().all(|u| u.sel.symbol.is_none()));
    assert!(
        resolve(&mut wt(tmp.path()), "file:bad.py", json!({"symbol": "f"}))
            .unwrap()
            .is_empty()
    );
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
    assert_eq!(
        code_of(resolve(&mut p, l, json!({}))),
        ProviderErrorCode::Unavailable
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
        let units = p.units(&l, None).unwrap();
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
    let mut rows: HashMap<String, String> = HashMap::new();
    for (path, text) in CORPUS {
        let blob = hibi::git::blob_id(text.as_bytes());
        let class = hibi::code::Class::from_path(path).name();
        Cache::put_units(&db, &blob, class, "[{\"not\":\"a unit\"}]").unwrap();
        rows.insert(blob, class.to_owned());
    }
    let damaged = answers(&mut with_cache(&counting));
    assert_eq!(damaged, plain, "damaged rows");
    for (blob, class) in rows {
        assert!(
            db.units(&blob, &class)
                .unwrap()
                .unwrap()
                .starts_with("[{\"key\":\"file\"")
        );
    }
}
