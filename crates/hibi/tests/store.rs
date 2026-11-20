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
    self, GITATTRIBUTES, GitTextStore, MAX_SHARD_BYTES, MAX_STORE_BYTES, check_root, discover_root,
    init, init_files, shard_of, shard_paths,
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

    // An append that adds nothing new to the shard still leaves it sorted
    // and without duplicates.
    store
        .append(&Batch {
            claims: vec![a.clone()],
            ..Batch::default()
        })
        .unwrap();
    assert_shards_canonical(dir.path());
    let mut sorted_lines = [line(&a), line(&b), line(&c)];
    sorted_lines.sort();
    assert_eq!(
        fs::read_to_string(&shard).unwrap(),
        sorted_lines.join("\n") + "\n"
    );

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

    // Section 12.4 step 3b: a write to the orphan's shard keeps its line.
    let shard = shard_of(&gone.subject.loc);
    store
        .append(&claim_and_mark(claim_in_shard(shard, "n", "n")))
        .unwrap();
    let orphan_line = String::from_utf8(m.canonical().unwrap()).unwrap();
    let lines: Vec<String> = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert!(lines.contains(&orphan_line), "the orphan line is kept");
    assert!(lines.len() >= 2, "the new mark is in the same shard");
    assert_shards_canonical(dir.path());
    assert!(store.load().unwrap().marks.iter().all(|x| x.id != m.id));
}

/// The errors of `load`, `append` and `snapshot_id`, in that order. Each
/// must fail.
fn three_errors(store: &GitTextStore) -> [(&'static str, hibi::Error); 3] {
    [
        ("load", store.load().unwrap_err()),
        (
            "append",
            store
                .append(&claim_and_mark(claim("a.md", "a")))
                .unwrap_err(),
        ),
        ("snapshot_id", store.snapshot_id().unwrap_err()),
    ]
}

/// Set the length of an existing shard file without writing data (a sparse
/// file of zero bytes).
fn set_shard_len(root: &Path, rel: &str, len: u64) {
    fs::OpenOptions::new()
        .write(true)
        .open(root.join(".hibi").join(rel))
        .unwrap()
        .set_len(len)
        .unwrap();
}

/// R63: a shard longer than `MAX_SHARD_BYTES` is `store_corrupt`; a shard of
/// exactly `MAX_SHARD_BYTES` is not refused for its size.
///
/// The size is checked when a shard is read, by one comparison that
/// `load`, `append` (its read under the lock) and `snapshot_id` share. The
/// write path of `append` has no size check of its own.
///
/// The shard at the cap is a sparse file of zero bytes. `load` and `append`
/// read it and fail on its content, with an error that is not the size
/// error. Acceptance is not shown with a shard that loads: 64 MiB of valid
/// records take too long to build and parse in a test, and `snapshot_id`,
/// which does not parse, takes seconds to hash 64 MiB in a debug build.
#[test]
fn r63_shard_size_boundary() {
    let size_error = format!("is longer than {MAX_SHARD_BYTES} bytes");
    for rel in ["claims/0.jsonl", "marks/0.jsonl"] {
        let (dir, store) = new_store();
        set_shard_len(dir.path(), rel, MAX_SHARD_BYTES);
        for (op, e) in [
            ("load", store.load().unwrap_err()),
            (
                "append",
                store
                    .append(&claim_and_mark(claim("a.md", "a")))
                    .unwrap_err(),
            ),
        ] {
            assert_eq!(e.code, Code::StoreCorrupt, "{op} {rel}");
            assert_eq!(
                e.message,
                format!(".hibi/{rel} line 1 does not end with a newline"),
                "{op} {rel}: the content is refused, not the size"
            );
        }

        set_shard_len(dir.path(), rel, MAX_SHARD_BYTES + 1);
        for (op, e) in three_errors(&store) {
            assert_eq!(e.code, Code::StoreCorrupt, "{op} {rel}");
            assert_eq!(e.message, format!(".hibi/{rel} {size_error}"), "{op} {rel}");
        }
    }
}

/// The 32 shard files together may hold `MAX_STORE_BYTES`; one byte more is
/// `store_corrupt`, found from the file sizes before any shard is read.
///
/// Every file stays at or under `MAX_SHARD_BYTES`, so the per-file check
/// cannot be what refuses. The files are sparse and hold zero bytes: at
/// exactly the cap `load` and `append` get as far as reading a shard and
/// fail on its content. `snapshot_id` makes the same check through the same
/// function; it is not run at the cap, because hashing 256 MiB takes many
/// seconds in a debug build.
#[test]
fn total_store_size_boundary() {
    assert_eq!(MAX_STORE_BYTES, 268_435_456);
    assert_eq!(MAX_STORE_BYTES, 4 * MAX_SHARD_BYTES);
    let (dir, store) = new_store();
    // Two claims shards and two marks shards, each exactly at the per-file
    // cap: the total is exactly MAX_STORE_BYTES.
    for rel in [
        "claims/0.jsonl",
        "claims/f.jsonl",
        "marks/0.jsonl",
        "marks/f.jsonl",
    ] {
        set_shard_len(dir.path(), rel, MAX_SHARD_BYTES);
    }
    for (op, e) in [
        ("load", store.load().unwrap_err()),
        (
            "append",
            store
                .append(&claim_and_mark(claim("a.md", "a")))
                .unwrap_err(),
        ),
    ] {
        assert_eq!(e.code, Code::StoreCorrupt, "{op}");
        assert_eq!(
            e.message, ".hibi/marks/0.jsonl line 1 does not end with a newline",
            "{op}: a shard was read and its content refused, not the total size"
        );
    }

    // One byte more, in a fifth file.
    set_shard_len(dir.path(), "marks/7.jsonl", 1);
    for (op, e) in three_errors(&store) {
        assert_eq!(e.code, Code::StoreCorrupt, "{op}");
        assert_eq!(
            e.message,
            format!(
                "the shard files under .hibi/claims and .hibi/marks hold {} bytes in total, more than the limit of {MAX_STORE_BYTES} bytes",
                MAX_STORE_BYTES + 1
            ),
            "{op}"
        );
        let remediation = e.remediation.as_deref().unwrap_or("");
        assert!(
            remediation.contains(".hibi/claims") && remediation.contains(".hibi/marks"),
            "{op}: {remediation}"
        );
    }
}

/// Section 12.3: a missing shard file is an empty shard, for reading and
/// for writing.
#[test]
fn missing_shards_are_empty() {
    let (dir, store) = new_store();
    let empty = store.snapshot_id().unwrap();
    for rel in shard_paths() {
        fs::remove_file(dir.path().join(rel)).unwrap();
    }
    let ledger = store.load().unwrap();
    assert!(ledger.claims.is_empty() && ledger.warnings.is_empty());
    assert_eq!(store.snapshot_id().unwrap(), empty);
    let c = claim("a.md", "a");
    store.append(&claim_and_mark(c.clone())).unwrap();
    assert_eq!(store.load().unwrap().claims, vec![c]);
}

/// A missing `claims` or `marks` directory is `store_corrupt` in `load`,
/// `append` and `snapshot_id`. The message names the directory and the
/// remediation says how to restore it. Nothing is created, not even
/// `.hibi/.lock`.
#[test]
fn missing_shard_directory_is_store_corrupt() {
    for d in ["claims", "marks"] {
        let (dir, store) = new_store();
        fs::remove_dir_all(dir.path().join(".hibi").join(d)).unwrap();
        let before = tree_state(dir.path());
        for (op, e) in three_errors(&store) {
            assert_eq!(e.code, Code::StoreCorrupt, "{op} {d}");
            assert_eq!(
                e.message,
                format!("the directory .hibi/{d} is missing"),
                "{op} {d}"
            );
            assert_eq!(
                e.remediation.as_deref(),
                Some(
                    format!(
                        "Restore .hibi/{d} from git, for example with git checkout -- .hibi/{d}, or create the empty directory again with mkdir .hibi/{d}."
                    )
                    .as_str()
                ),
                "{op} {d}"
            );
        }
        assert_eq!(tree_state(dir.path()), before, "nothing written: {d}");

        // The second remediation works: an empty directory is a store whose
        // shards of that kind are all missing, that is, empty.
        fs::create_dir(dir.path().join(".hibi").join(d)).unwrap();
        assert!(store.load().unwrap().claims.is_empty());
        store.snapshot_id().unwrap();
        let c = claim("a.md", "a");
        store.append(&claim_and_mark(c.clone())).unwrap();
        let ledger = store.load().unwrap();
        assert_eq!(ledger.claims, vec![c]);
        assert_eq!(ledger.marks.len(), 1);
    }
}

/// Section 12.2: the shard is the first hex digit of BLAKE3 over the UTF-8
/// bytes of the subject locator, that is the high four bits of digest
/// byte 0.
///
/// The expected values were not produced by `shard_of`. A separate scratch
/// program that depends only on the `blake3` crate (1.8.x) printed
/// `blake3::hash(locator.as_bytes()).to_hex()` for each locator; the two
/// hex digits below are the first two characters of that text, copied by
/// hand. Neither the `b3sum` tool nor a Python `blake3` module was
/// installed, so the digests come from the same BLAKE3 implementation that
/// the library links, but not through the library's code and without any
/// shift or mask. To check a row with another implementation:
/// `printf %s file:README.md | b3sum` must print a digest that starts with
/// the two digits of that row.
///
/// In 12 of the 14 rows the two digits differ, so taking the low four bits
/// of byte 0 gives a different shard.
#[test]
fn shard_is_the_first_hex_digit_of_blake3_of_the_subject_locator() {
    // (locator, first two hex digits of its BLAKE3 digest)
    let known = [
        ("file:README.md", "60"),
        ("file:docs/a.md", "27"),
        ("file:docs/b.md", "b9"),
        ("file:docs/guide/install.md", "fe"),
        ("file:src/lib.rs", "e3"),
        ("file:src/store/git_text.rs", "04"),
        ("file:crates/hibi/Cargo.toml", "00"),
        ("file:docs/日本語.md", "df"),
        ("repo:api//openapi.yaml", "a6"),
        ("repo:web//src/index.ts", "07"),
        ("archctl:component/store", "1a"),
        ("file:a", "78"),
        ("file:c", "89"),
        ("file:d", "88"),
    ];
    let mut differ = 0;
    for (loc, byte0) in known {
        let first = byte0.chars().next().unwrap();
        let second = byte0.chars().nth(1).unwrap();
        differ += usize::from(first != second);
        let shard = shard_of(&Locator::new(loc).unwrap());
        assert_eq!(git_text::shard_digit(shard), first, "{loc}");
        assert_eq!(u32::from(shard), first.to_digit(16).unwrap(), "{loc}");
    }
    assert_eq!(differ, 12);

    // A claim is written to the shard file of that digit.
    let (dir, store) = new_store();
    let c = claim("docs/b.md", "b");
    store.append(&claim_and_mark(c.clone())).unwrap();
    for (kind, id) in [("claims", c.id.as_str()), ("marks", "m1-")] {
        let text = fs::read_to_string(dir.path().join(format!(".hibi/{kind}/b.jsonl"))).unwrap();
        assert!(text.contains(id), "{kind}/b.jsonl holds the record");
        let other = fs::read(dir.path().join(format!(".hibi/{kind}/9.jsonl"))).unwrap();
        assert!(other.is_empty(), "{kind}/9.jsonl is empty");
    }
}

/// Section 12.3: a damaged line is reported with its file and its line
/// number.
#[test]
fn damaged_lines_name_file_and_line() {
    let a = claim_in_shard(5, "a", "a");
    let b = claim_in_shard(5, "b", "b");
    let line = |c: &Claim| String::from_utf8(c.canonical().unwrap()).unwrap();
    let (la, lb) = (line(&a), line(&b));
    let m = mark(&a, "2026-11-20T00:00:00Z");
    let lm = String::from_utf8(m.canonical().unwrap()).unwrap();
    let mut tampered: serde_json::Value = serde_json::from_str(&lb).unwrap();
    tampered["statement"] = "changed".into();
    let reordered = {
        // The same record with its keys in another order: valid JSON, right
        // id, not canonical.
        let v: serde_json::Value = serde_json::from_str(&lb).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().iter().collect();
        keys.reverse();
        let body: Vec<String> = keys
            .iter()
            .map(|(k, v)| format!("{}:{v}", serde_json::to_string(k).unwrap()))
            .collect();
        format!("{{{}}}", body.join(","))
    };
    assert_ne!(reordered, lb);
    let cases: Vec<(&str, String, String)> = vec![
        (
            "claims/5.jsonl",
            format!("{la}\nnot json\n"),
            ".hibi/claims/5.jsonl line 2: not valid JSON: expected ident at line 1 column 2".into(),
        ),
        (
            "claims/5.jsonl",
            format!("{la}\n\n{lb}\n"),
            ".hibi/claims/5.jsonl line 2: not valid JSON: EOF while parsing a value at line 1 column 0".into(),
        ),
        (
            "claims/5.jsonl",
            format!("{la}\n{lb}"),
            ".hibi/claims/5.jsonl line 2 does not end with a newline".into(),
        ),
        (
            "claims/5.jsonl",
            format!("{la}\n{lb}\n{reordered}\n"),
            ".hibi/claims/5.jsonl line 3: the line is not the canonical form of its record".into(),
        ),
        (
            "claims/5.jsonl",
            format!("{la}\n{}\n", lb.replacen('{', "{\"extra\":1,", 1)),
            ".hibi/claims/5.jsonl line 2: unknown field `extra`, expected one of `id`, `statement`, `subject`, `evidence`, `recheck`, `note`".into(),
        ),
        (
            "claims/5.jsonl",
            format!("{la}\n{}\n", serde_json::to_string(&tampered).unwrap()),
            format!(
                ".hibi/claims/5.jsonl line 2: the id {} is not the id derived from the record body ({})",
                b.id,
                claim(
                    b.subject.loc.as_str().strip_prefix("file:").unwrap(),
                    "changed"
                )
                .id
            ),
        ),
        (
            "marks/5.jsonl",
            format!("{lm}\n{la}\n"),
            format!(
                ".hibi/marks/5.jsonl line 2: id {:?} is neither a mark nor a retirement id",
                a.id.as_str()
            ),
        ),
        (
            "marks/5.jsonl",
            format!("{lm}\n[1]\n"),
            ".hibi/marks/5.jsonl line 2: id \"\" is neither a mark nor a retirement id".into(),
        ),
        (
            "marks/5.jsonl",
            format!("{lm}\n{}\n", lm.replacen('{', "{ ", 1)),
            ".hibi/marks/5.jsonl line 2: the line is not the canonical form of its record".into(),
        ),
    ];
    for (rel, content, message) in cases {
        let (dir, store) = new_store();
        fs::write(dir.path().join(".hibi").join(rel), &content).unwrap();
        let e = store.load().unwrap_err();
        assert_eq!(e.code, Code::StoreCorrupt, "{content:?}");
        assert_eq!(e.message, message, "{content:?}");
    }
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
