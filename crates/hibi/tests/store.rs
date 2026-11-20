//! Integration tests of `GitTextStore` (spec section 12; requirements R7 to
//! R11, R32, R48, R63 and the `append` contract of section 2.4).
//!
//! The multi-process tests start this test binary again with
//! `HIBI_TEST_CHILD` set; the test `child_role` then acts as the child.

#![cfg(feature = "git-store")]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use hibi::Code;
use hibi::model::{
    Anchor, Claim, Fingerprint, Locator, Mark, MarkSide, Retirement, Selector, Timestamp,
};
use hibi::store::git_text::{
    self, GITATTRIBUTES, GitTextStore, MAX_SHARD_BYTES, check_root, discover_root, init,
    init_files, shard_of, shard_paths,
};
use hibi::store::{Batch, Store};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn claim(path: &str, statement: &str) -> Claim {
    Claim::new(
        statement,
        Anchor::new(
            Locator::new(format!("file:{path}")).unwrap(),
            Selector::quote("x"),
        ),
        vec![],
        None,
        None,
    )
    .unwrap()
}

/// A claim whose subject path starts with `prefix` and that falls in
/// `shard`.
fn claim_in_shard(shard: u8, prefix: &str, statement: &str) -> Claim {
    (0..)
        .map(|i| claim(&format!("{prefix}{i}.md"), statement))
        .find(|c| shard_of(&c.subject.loc) == shard)
        .unwrap()
}

fn mark(c: &Claim, at: &str) -> Mark {
    Mark::new(
        c.id.clone(),
        Timestamp::parse(at).unwrap(),
        Some("tester".into()),
        c.sides()
            .map(|a| MarkSide {
                loc: a.loc.clone(),
                fp: Fingerprint::new("b3:prose-v1:00112233445566778899aabbccddeeff").unwrap(),
            })
            .collect(),
        None,
    )
    .unwrap()
}

fn claim_and_mark(c: Claim) -> Batch {
    let m = mark(&c, "2026-11-20T18:04:11Z");
    Batch {
        claims: vec![c],
        marks: vec![m],
        retirements: vec![],
    }
}

fn new_store() -> (tempfile::TempDir, GitTextStore) {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path()).unwrap();
    let store = GitTextStore::open(dir.path()).unwrap();
    (dir, store)
}

/// Every shard file is sorted by id with no duplicate, ends with a
/// newline, and holds only canonical lines (section 12.2).
fn assert_shards_canonical(root: &Path) {
    for rel in shard_paths() {
        let bytes = fs::read(root.join(&rel)).unwrap();
        if bytes.is_empty() {
            continue;
        }
        assert_eq!(bytes.last(), Some(&b'\n'), "{rel}");
        let ids: Vec<String> = bytes[..bytes.len() - 1]
            .split(|&b| b == b'\n')
            .map(|l| {
                let v: serde_json::Value = serde_json::from_slice(l).unwrap();
                v["id"].as_str().unwrap().to_owned()
            })
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(ids, sorted, "{rel} is sorted without duplicates");
    }
}

/// Every file under `dir`, with its bytes, for "changes nothing" checks.
fn tree_state(dir: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries: Vec<_> = fs::read_dir(&d).unwrap().map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            let p = e.path();
            let t = fs::symlink_metadata(&p).unwrap().file_type();
            if t.is_dir() {
                out.push((p.clone(), None));
                stack.push(p);
            } else if t.is_symlink() {
                out.push((
                    p.clone(),
                    Some(
                        fs::read_link(&p)
                            .unwrap()
                            .into_os_string()
                            .into_encoded_bytes(),
                    ),
                ));
            } else {
                out.push((p.clone(), Some(fs::read(&p).unwrap())));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// init, layout, discovery
// ---------------------------------------------------------------------------

#[test]
fn init_creates_the_layout() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("README.md"), "x").unwrap();
    let created = init(dir.path()).unwrap();
    assert_eq!(created, init_files());
    assert_eq!(created.len(), 4 + 32);
    assert!(created.iter().all(|f| f.starts_with(".hibi/")));
    for f in &created {
        assert!(dir.path().join(f).is_file(), "{f}");
    }
    assert_eq!(
        fs::read_to_string(dir.path().join(".hibi/.gitattributes")).unwrap(),
        GITATTRIBUTES
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".hibi/.ignore")).unwrap(),
        "claims/\nmarks/\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".hibi/.gitignore")).unwrap(),
        ".lock\n*.tmp-*\n"
    );
    for s in shard_paths() {
        assert_eq!(fs::read(dir.path().join(s)).unwrap(), b"");
    }
    // The template is a valid hibi.toml.
    let cfg = hibi::config::RepoConfig::load(dir.path()).unwrap();
    assert!(cfg.scopes.is_empty());
    assert!(!dir.path().join(".hibi/.lock").exists());

    let store = GitTextStore::open(dir.path()).unwrap();
    let ledger = store.load().unwrap();
    assert!(ledger.claims.is_empty() && ledger.warnings.is_empty());

    let before = tree_state(dir.path());
    let err = init(dir.path()).unwrap_err();
    assert_eq!(err.code, Code::AlreadyInitialized);
    assert_eq!(tree_state(dir.path()), before);
}

#[test]
fn init_refuses_leftover_files_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".hibi/marks")).unwrap();
    fs::write(dir.path().join(".hibi/marks/7.jsonl"), "").unwrap();
    let before = tree_state(dir.path());
    assert_eq!(init(dir.path()).unwrap_err().code, Code::AlreadyInitialized);
    assert_eq!(tree_state(dir.path()), before);
}

#[test]
fn discovery_and_root_option() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("r");
    fs::create_dir_all(root.join("a/b")).unwrap();
    assert_eq!(
        discover_root(&root.join("a/b")).unwrap_err().code,
        Code::NotInitialized
    );
    init(&root).unwrap();
    assert_eq!(discover_root(&root.join("a/b")).unwrap(), root);
    assert_eq!(discover_root(&root).unwrap(), root);
    assert_eq!(check_root(&root).unwrap(), root);
    assert_eq!(
        check_root(&root.join("a")).unwrap_err().code,
        Code::NotInitialized
    );
}

/// R32 for the store layer: `.claims/` at the root refuses with
/// `legacy_store` and changes nothing.
#[test]
fn r32_legacy_store_is_refused_without_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join(".claims")).unwrap();
    fs::write(root.join(".claims/c.json"), "{}").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    let before = tree_state(root);

    let e = init(root).unwrap_err();
    assert_eq!(e.code, Code::LegacyStore);
    assert_eq!(
        e.message,
        "this repository has a hibi 0.x store in .claims/, which this version does not read"
    );
    assert_eq!(
        e.remediation.as_deref(),
        Some("remove or move .claims/, or use hibi 0.7")
    );
    assert_eq!(
        discover_root(&root.join("sub")).unwrap_err().code,
        Code::LegacyStore
    );
    assert_eq!(check_root(root).unwrap_err().code, Code::LegacyStore);
    assert_eq!(
        GitTextStore::open(root).unwrap_err().code,
        Code::LegacyStore
    );
    assert_eq!(tree_state(root), before);

    // A v5 store next to a .claims/ directory is refused too.
    let dir2 = tempfile::tempdir().unwrap();
    init(dir2.path()).unwrap();
    fs::create_dir(dir2.path().join(".claims")).unwrap();
    let before = tree_state(dir2.path());
    assert_eq!(
        discover_root(dir2.path()).unwrap_err().code,
        Code::LegacyStore
    );
    assert_eq!(
        GitTextStore::open(dir2.path()).unwrap_err().code,
        Code::LegacyStore
    );
    assert_eq!(tree_state(dir2.path()), before);
}

// ---------------------------------------------------------------------------
// Reading and writing
// ---------------------------------------------------------------------------

#[test]
fn append_and_load_round_trip() {
    let (dir, store) = new_store();
    let a = claim("docs/a.md", "a");
    let b = claim("docs/b.md", "b");
    let ma = mark(&a, "2026-11-20T18:04:11Z");
    let r = Retirement::new(
        b.id.clone(),
        Some(a.id.clone()),
        Timestamp::parse("2026-11-21T00:00:00Z").unwrap(),
        None,
        Some("replaced".into()),
    )
    .unwrap();
    let s0 = store.snapshot_id().unwrap();
    assert!(s0.starts_with("b3:") && s0.len() == 3 + 64);
    store
        .append(&Batch {
            claims: vec![b.clone(), a.clone()],
            marks: vec![ma.clone()],
            retirements: vec![r.clone()],
        })
        .unwrap();
    let s1 = store.snapshot_id().unwrap();
    assert_ne!(s0, s1);
    let ledger = store.load().unwrap();
    let mut claims = vec![a.clone(), b.clone()];
    claims.sort_by(|x, y| x.id.cmp(&y.id));
    assert_eq!(ledger.claims, claims);
    assert_eq!(ledger.marks, vec![ma.clone()]);
    assert_eq!(ledger.retirements, vec![r.clone()]);
    assert!(ledger.warnings.is_empty());

    // Idempotent: the same batch again writes nothing.
    store
        .append(&Batch {
            claims: vec![a.clone()],
            marks: vec![ma],
            retirements: vec![r],
        })
        .unwrap();
    assert_eq!(store.snapshot_id().unwrap(), s1);
    assert_shards_canonical(dir.path());

    // The claim is in its shard; the mark and retirement in their claims'.
    let sa = git_text::shard_digit(shard_of(&a.subject.loc));
    let line = fs::read_to_string(dir.path().join(format!(".hibi/claims/{sa}.jsonl"))).unwrap();
    assert!(line.contains(a.id.as_str()));
    let sb = git_text::shard_digit(shard_of(&b.subject.loc));
    let marks = fs::read_to_string(dir.path().join(format!(".hibi/marks/{sb}.jsonl"))).unwrap();
    assert!(marks.contains("r1-"));
}

#[test]
fn append_rejects_bad_ids_and_missing_claims() {
    let (dir, store) = new_store();
    let before = tree_state(dir.path());

    // An id that does not match the body.
    let mut c = claim("a.md", "a");
    c.statement = "changed".into();
    let e = store
        .append(&Batch {
            claims: vec![c],
            ..Batch::default()
        })
        .unwrap_err();
    assert_eq!(e.code, Code::Internal);

    // A mark whose claim is in neither the store nor the batch.
    let other = claim("b.md", "b");
    let e = store
        .append(&Batch {
            claims: vec![claim("c.md", "c")],
            marks: vec![mark(&other, "2026-11-20T00:00:00Z")],
            ..Batch::default()
        })
        .unwrap_err();
    assert_eq!(e.code, Code::Internal);

    // A retirement likewise.
    let r = Retirement::new(
        other.id.clone(),
        None,
        Timestamp::parse("2026-11-20T00:00:00Z").unwrap(),
        None,
        None,
    )
    .unwrap();
    let e = store
        .append(&Batch {
            retirements: vec![r],
            ..Batch::default()
        })
        .unwrap_err();
    assert_eq!(e.code, Code::Internal);

    // A mark whose sides do not match its claim.
    let d = claim("d.md", "d");
    let mut bad = mark(&d, "2026-11-20T00:00:00Z");
    bad.sides.push(bad.sides[0].clone());
    let bad = Mark::new(bad.claim, bad.at, bad.by, bad.sides, bad.note).unwrap();
    let e = store
        .append(&Batch {
            claims: vec![d],
            marks: vec![bad],
            ..Batch::default()
        })
        .unwrap_err();
    assert_eq!(e.code, Code::Internal);

    // Nothing was written, not even the lock file in the id case.
    let after: Vec<_> = tree_state(dir.path())
        .into_iter()
        .filter(|(p, _)| !p.ends_with(".lock"))
        .collect();
    assert_eq!(after, before);

    // A mark for a stored claim is accepted in a later batch.
    let c = claim("e.md", "e");
    store
        .append(&Batch {
            claims: vec![c.clone()],
            ..Batch::default()
        })
        .unwrap();
    store
        .append(&Batch {
            marks: vec![mark(&c, "2026-11-20T00:00:00Z")],
            ..Batch::default()
        })
        .unwrap();
    assert_eq!(store.load().unwrap().marks.len(), 1);
}

/// R9: unsorted and duplicated lines load; the next write sorts and
/// deduplicates the shard.
#[test]
fn r9_unsorted_and_duplicate_lines() {
    let (dir, store) = new_store();
    let a = claim_in_shard(5, "a", "a");
    let b = claim_in_shard(5, "b", "b");
    let c = claim_in_shard(5, "c", "c");
    let line = |c: &Claim| String::from_utf8(c.canonical().unwrap()).unwrap();
    let mut lines = vec![line(&a), line(&b), line(&c)];
    lines.sort();
    lines.reverse();
    lines.push(lines[0].clone());
    let shard = dir.path().join(".hibi/claims/5.jsonl");
    fs::write(&shard, lines.join("\n") + "\n").unwrap();

    let ledger = store.load().unwrap();
    assert_eq!(ledger.claims.len(), 3);
    let ids: Vec<_> = ledger.claims.iter().map(|c| c.id.clone()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);

    let d = claim_in_shard(5, "d", "d");
    store.append(&claim_and_mark(d)).unwrap();
    assert_shards_canonical(dir.path());
    assert_eq!(fs::read_to_string(&shard).unwrap().lines().count(), 4);
    assert_eq!(store.load().unwrap().claims.len(), 4);
}

#[test]
fn damaged_shards_are_store_corrupt() {
    let a = claim("a.md", "a");
    let sa = git_text::shard_digit(shard_of(&a.subject.loc));
    let other = git_text::shard_digit((shard_of(&a.subject.loc) + 1) % 16);
    let canonical = String::from_utf8(a.canonical().unwrap()).unwrap();
    let pretty = serde_json::to_string_pretty(&a).unwrap().replace('\n', "");
    let mut tampered: serde_json::Value = serde_json::from_str(&canonical).unwrap();
    tampered["statement"] = "b".into();
    let cases: Vec<(String, String)> = vec![
        (format!("claims/{sa}.jsonl"), "not json\n".into()),
        (format!("claims/{sa}.jsonl"), canonical.clone()),
        (format!("claims/{sa}.jsonl"), format!("{canonical}\n\n")),
        (format!("claims/{sa}.jsonl"), format!("{pretty}\n")),
        (
            format!("claims/{sa}.jsonl"),
            format!("{}\n", serde_json::to_string(&tampered).unwrap()),
        ),
        (
            format!("claims/{sa}.jsonl"),
            format!("{}\n", canonical.replace("}", ",\"extra\":1}")),
        ),
        (format!("claims/{other}.jsonl"), format!("{canonical}\n")),
        (format!("marks/{sa}.jsonl"), format!("{canonical}\n")),
        (
            format!("claims/{sa}.jsonl"),
            format!("{}\n", canonical.replace("\n", "\r\n") + "\r"),
        ),
    ];
    for (rel, content) in cases {
        let (dir, store) = new_store();
        fs::write(dir.path().join(".hibi").join(&rel), &content).unwrap();
        let e = store.load().unwrap_err();
        assert_eq!(e.code, Code::StoreCorrupt, "{rel}: {content:?}");
        assert!(
            e.message.contains(".hibi/"),
            "names the file: {}",
            e.message
        );
        // append and snapshot_id refuse to build on a corrupt store too.
        let before = tree_state(dir.path());
        assert_eq!(
            store
                .append(&claim_and_mark(claim("z.md", "z")))
                .unwrap_err()
                .code,
            Code::StoreCorrupt
        );
        let after: Vec<_> = tree_state(dir.path())
            .into_iter()
            .filter(|(p, _)| !p.ends_with(".lock"))
            .collect();
        assert_eq!(after, before);
    }
}

#[test]
fn mark_in_wrong_shard_or_with_wrong_sides_is_corrupt() {
    let a = claim("a.md", "a");
    let s = shard_of(&a.subject.loc);
    let m = mark(&a, "2026-11-20T00:00:00Z");
    let mline = String::from_utf8(m.canonical().unwrap()).unwrap();

    let (dir, store) = new_store();
    store
        .append(&Batch {
            claims: vec![a.clone()],
            ..Batch::default()
        })
        .unwrap();
    let wrong = git_text::shard_digit((s + 3) % 16);
    fs::write(
        dir.path().join(format!(".hibi/marks/{wrong}.jsonl")),
        format!("{mline}\n"),
    )
    .unwrap();
    assert_eq!(store.load().unwrap_err().code, Code::StoreCorrupt);

    let (dir, store) = new_store();
    store
        .append(&Batch {
            claims: vec![a.clone()],
            ..Batch::default()
        })
        .unwrap();
    let mut two = m.clone();
    two.sides.push(two.sides[0].clone());
    let two = Mark::new(two.claim, two.at, two.by, two.sides, two.note).unwrap();
    let sd = git_text::shard_digit(s);
    fs::write(
        dir.path().join(format!(".hibi/marks/{sd}.jsonl")),
        format!("{}\n", String::from_utf8(two.canonical().unwrap()).unwrap()),
    )
    .unwrap();
    assert_eq!(store.load().unwrap_err().code, Code::StoreCorrupt);
}

#[test]
fn orphans_and_unknown_replacements_are_warnings() {
    let (dir, store) = new_store();
    let a = claim("a.md", "a");
    let gone = claim("gone.md", "gone");
    store
        .append(&Batch {
            claims: vec![a.clone()],
            ..Batch::default()
        })
        .unwrap();
    // An orphan mark (in the shard its claim would have had).
    let m = mark(&gone, "2026-11-20T00:00:00Z");
    let sg = git_text::shard_digit(shard_of(&gone.subject.loc));
    let path = dir.path().join(format!(".hibi/marks/{sg}.jsonl"));
    let mut content = fs::read_to_string(&path).unwrap();
    content.push_str(&String::from_utf8(m.canonical().unwrap()).unwrap());
    content.push('\n');
    fs::write(&path, content).unwrap();
    // A retirement of `a` replaced by a claim that is not stored.
    let r = Retirement::new(
        a.id.clone(),
        Some(gone.id.clone()),
        Timestamp::parse("2026-11-20T00:00:00Z").unwrap(),
        None,
        None,
    )
    .unwrap();
    store
        .append(&Batch {
            retirements: vec![r.clone()],
            ..Batch::default()
        })
        .unwrap();
    let ledger = store.load().unwrap();
    assert!(ledger.marks.is_empty(), "the orphan is ignored");
    assert_eq!(ledger.retirements, vec![r], "the retirement stands");
    let codes: BTreeSet<&str> = ledger.warnings.iter().map(|w| w.code.as_str()).collect();
    assert_eq!(codes, ["orphan_record", "unknown_replacement"].into());
}

/// R63: a shard longer than `MAX_SHARD_BYTES` is `store_corrupt`.
#[test]
fn r63_oversized_shard() {
    let (dir, store) = new_store();
    let f = fs::File::create(dir.path().join(".hibi/claims/0.jsonl")).unwrap();
    f.set_len(MAX_SHARD_BYTES + 1).unwrap();
    assert_eq!(store.load().unwrap_err().code, Code::StoreCorrupt);
    assert_eq!(store.snapshot_id().unwrap_err().code, Code::StoreCorrupt);
    assert_eq!(
        store
            .append(&claim_and_mark(claim("a.md", "a")))
            .unwrap_err()
            .code,
        Code::StoreCorrupt
    );
}

#[test]
fn missing_shards_are_empty() {
    let (dir, store) = new_store();
    fs::remove_file(dir.path().join(".hibi/claims/3.jsonl")).unwrap();
    fs::remove_dir_all(dir.path().join(".hibi/marks")).unwrap();
    assert!(store.load().unwrap().claims.is_empty());
    store.snapshot_id().unwrap();
}

// ---------------------------------------------------------------------------
// R48: symbolic links
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn r48_symbolic_links_are_refused_and_never_written_through() {
    use std::os::unix::fs::symlink;
    let targets = [
        ".hibi",
        ".hibi/hibi.toml",
        ".hibi/claims",
        ".hibi/marks",
        ".hibi/claims/SHARD.jsonl",
        ".hibi/marks/SHARD.jsonl",
        ".hibi/.lock",
    ];
    let c = claim("a.md", "a");
    let shard = git_text::shard_digit(shard_of(&c.subject.loc)).to_string();
    for t in targets {
        let t = t.replace("SHARD", &shard);
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        init(&root).unwrap();
        // Move the real thing outside the root and link to it.
        let real = root.join(&t);
        let outside = dir.path().join("outside");
        if real.exists() {
            fs::rename(&real, &outside).unwrap();
        } else {
            fs::write(&outside, "").unwrap();
        }
        symlink(&outside, &real).unwrap();
        let before_outside = tree_state(dir.path());
        let store = GitTextStore::open(&root).unwrap();

        assert_eq!(
            store.load().unwrap_err().code,
            Code::StoreCorrupt,
            "load {t}"
        );
        assert_eq!(
            store.append(&claim_and_mark(c.clone())).unwrap_err().code,
            Code::StoreCorrupt,
            "append {t}"
        );
        assert_eq!(
            store.snapshot_id().unwrap_err().code,
            Code::StoreCorrupt,
            "snapshot {t}"
        );
        let e = init(&root).unwrap_err();
        assert_eq!(e.code, Code::StoreCorrupt, "init {t}");
        assert_eq!(
            tree_state(dir.path()),
            before_outside,
            "nothing written: {t}"
        );
    }

    // init on a fresh tree whose .hibi is a link to an empty directory.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::create_dir(dir.path().join("elsewhere")).unwrap();
    symlink(dir.path().join("elsewhere"), root.join(".hibi")).unwrap();
    assert_eq!(init(&root).unwrap_err().code, Code::StoreCorrupt);
    assert_eq!(
        fs::read_dir(dir.path().join("elsewhere")).unwrap().count(),
        0
    );
}

// ---------------------------------------------------------------------------
// R7, R8: merging
// ---------------------------------------------------------------------------

fn sh_git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
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
        .unwrap()
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = sh_git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Two branches append claims and marks to the same shards; returns the
/// merge's exit status and the number of claims the merge should hold.
fn branch_and_merge(with_attributes: bool) -> (tempfile::TempDir, bool, usize, usize) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_ok(root, &["init", "-q"]);
    init(root).unwrap();
    if !with_attributes {
        fs::remove_file(root.join(".hibi/.gitattributes")).unwrap();
    }
    let store = GitTextStore::open(root).unwrap();
    store
        .append(&claim_and_mark(claim_in_shard(7, "base", "base")))
        .unwrap();
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-qm", "base"]);

    git_ok(root, &["checkout", "-qb", "left"]);
    for i in 0..3 {
        store
            .append(&claim_and_mark(claim_in_shard(
                7,
                &format!("left{i}-"),
                "left",
            )))
            .unwrap();
    }
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-qm", "left"]);

    git_ok(root, &["checkout", "-q", "main"]);
    for i in 0..2 {
        store
            .append(&claim_and_mark(claim_in_shard(
                7,
                &format!("right{i}-"),
                "right",
            )))
            .unwrap();
    }
    git_ok(root, &["add", "-A"]);
    git_ok(root, &["commit", "-qm", "right"]);

    let merged = sh_git(root, &["merge", "--no-edit", "-q", "left"])
        .status
        .success();
    (dir, merged, 1 + 3 + 2, 1 + 3 + 2)
}

#[test]
fn r7_r8_branches_merge_without_conflict_and_load_every_record() {
    let (dir, merged, claims, marks) = branch_and_merge(true);
    assert!(merged, "git merge completes without conflict");
    let status = sh_git(dir.path(), &["status", "--porcelain"]);
    assert!(status.stdout.is_empty(), "clean after the merge");
    let store = GitTextStore::open(dir.path()).unwrap();
    let ledger = store.load().unwrap();
    assert_eq!(ledger.claims.len(), claims);
    assert_eq!(ledger.marks.len(), marks);
    assert!(ledger.warnings.is_empty());
    // The merged shard is accepted even though a union merge need not sort;
    // the next write leaves it canonical.
    store
        .append(&claim_and_mark(claim_in_shard(7, "after", "after")))
        .unwrap();
    assert_shards_canonical(dir.path());
    assert_eq!(store.load().unwrap().claims.len(), claims + 1);
}

#[test]
fn control_without_union_attributes_the_merge_conflicts() {
    let (_dir, merged, _, _) = branch_and_merge(false);
    assert!(!merged, "without .gitattributes the same merge conflicts");
}

// ---------------------------------------------------------------------------
// R10: concurrent appends
// ---------------------------------------------------------------------------

#[test]
fn r10_threads_with_separate_handles() {
    let (dir, _) = new_store();
    let root = dir.path().to_path_buf();
    let handles: Vec<_> = (0..4)
        .map(|t| {
            let root = root.clone();
            std::thread::spawn(move || {
                let store = GitTextStore::open(&root).unwrap();
                for i in 0..10 {
                    store
                        .append(&claim_and_mark(claim_in_shard(
                            (i % 2) as u8,
                            &format!("t{t}-{i}-"),
                            "thread",
                        )))
                        .unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let ledger = GitTextStore::open(&root).unwrap().load().unwrap();
    assert_eq!(ledger.claims.len(), 40);
    assert_eq!(ledger.marks.len(), 40);
    assert_shards_canonical(&root);
}

const CHILD_ENV: &str = "HIBI_TEST_CHILD";

/// Start this test binary as a child running `child_role`.
fn spawn_child(root: &Path, role: &str, tag: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_role", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, format!("{role}:{tag}:{}", root.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}

/// The child side of the multi-process tests. Does nothing unless started
/// by `spawn_child`.
#[test]
fn child_role() {
    let Ok(spec) = std::env::var(CHILD_ENV) else {
        return;
    };
    let mut parts = spec.splitn(3, ':');
    let (role, tag, root) = (
        parts.next().unwrap().to_owned(),
        parts.next().unwrap().to_owned(),
        PathBuf::from(parts.next().unwrap()),
    );
    let store = GitTextStore::open(&root).unwrap();
    match role.as_str() {
        // Append 20 claims with marks, all in shard 3.
        "append" => {
            for i in 0..20 {
                store
                    .append(&claim_and_mark(claim_in_shard(
                        3,
                        &format!("{tag}-{i}-"),
                        "proc",
                    )))
                    .unwrap();
            }
        }
        // Append forever, until killed.
        "loop" => {
            for i in 0u64..u64::MAX {
                let c = claim_in_shard((i % 4) as u8, &format!("{tag}-{i}-"), "loop");
                let m = mark(&c, "2026-11-20T00:00:00Z");
                store
                    .append(&Batch {
                        claims: vec![c],
                        marks: vec![m],
                        ..Batch::default()
                    })
                    .unwrap();
            }
        }
        other => panic!("unknown role {other}"),
    }
}

#[test]
fn r10_two_processes_append_at_once() {
    let (dir, _) = new_store();
    let a = spawn_child(dir.path(), "append", "a");
    let b = spawn_child(dir.path(), "append", "b");
    for mut child in [a, b] {
        assert!(child.wait().unwrap().success(), "both processes exit 0");
    }
    let ledger = GitTextStore::open(dir.path()).unwrap().load().unwrap();
    assert_eq!(ledger.claims.len(), 40);
    assert_eq!(ledger.marks.len(), 40);
    assert!(ledger.warnings.is_empty());
    assert_shards_canonical(dir.path());
}

// ---------------------------------------------------------------------------
// R11: killing a writer
// ---------------------------------------------------------------------------

#[test]
fn r11_killed_writer_leaves_a_loadable_store() {
    let (dir, _) = new_store();
    let store = GitTextStore::open(dir.path()).unwrap();
    let mut total = 0;
    for round in 0..12u64 {
        let mut child = spawn_child(dir.path(), "loop", &format!("k{round}"));
        std::thread::sleep(Duration::from_millis(40 + (round * 37) % 150));
        child.kill().unwrap();
        child.wait().unwrap();
        let ledger = store.load().expect("loads after a kill");
        assert!(ledger.warnings.is_empty(), "{:?}", ledger.warnings);
        for m in &ledger.marks {
            assert!(
                ledger.claim(m.claim.as_str()).is_some(),
                "mark without claim"
            );
        }
        assert!(ledger.claims.len() >= total);
        total = ledger.claims.len();
        // A later writer is not blocked by the killed one's lock or temp
        // files.
        store
            .append(&claim_and_mark(claim(&format!("after{round}.md"), "after")))
            .unwrap();
        total += 1;
    }
    assert!(total > 12, "the children wrote something: {total}");
}
