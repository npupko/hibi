//! Unit tests of `GitTextStore` that need the fault-injection seam:
//! crash safety at every step of the write protocol (R11) and temp-file
//! name collisions (R48).

use super::*;
use crate::model::{Anchor, Fingerprint, MarkSide, Selector, Timestamp};
use crate::store::Store;

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

fn mark(c: &Claim, at: &str) -> Mark {
    Mark::new(
        c.id.clone(),
        Timestamp::parse(at).unwrap(),
        None,
        vec![MarkSide {
            loc: c.subject.loc.clone(),
            fp: Fingerprint::new("b3:prose-v1:00112233445566778899aabbccddeeff").unwrap(),
        }],
        None,
    )
    .unwrap()
}

fn retirement(c: &Claim, at: &str) -> Retirement {
    Retirement::new(
        c.id.clone(),
        None,
        Timestamp::parse(at).unwrap(),
        None,
        None,
    )
    .unwrap()
}

fn new_store() -> (tempfile::TempDir, GitTextStore) {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path()).unwrap();
    let store = GitTextStore::open(dir.path()).unwrap();
    (dir, store)
}

/// Every mark and retirement in the ledger has its claim, and no shard is
/// corrupt.
fn assert_consistent(store: &GitTextStore) -> Ledger {
    let ledger = store.load().expect("the store loads after a crash");
    assert!(
        ledger.warnings.is_empty(),
        "no orphan after a crash: {:?}",
        ledger.warnings
    );
    for m in &ledger.marks {
        assert!(ledger.claim(m.claim.as_str()).is_some());
    }
    for r in &ledger.retirements {
        assert!(ledger.claim(r.retires.as_str()).is_some());
    }
    ledger
}

const STEPS: [Step; 6] = [
    Step::Locked,
    Step::TempCreated,
    Step::TempWritten,
    Step::TempSynced,
    Step::Renamed,
    Step::DirSynced,
];

/// R11: a crash at every step of every shard write leaves a store that
/// loads, with no mark or retirement without its claim; the next append
/// then completes the batch.
#[test]
fn crash_at_every_step_leaves_a_consistent_store() {
    // A batch touching two claims shards and two marks shards, plus a mark
    // and a retirement for a stored claim.
    let mut a = claim("a.md", "a");
    let mut b = claim("b.md", "b");
    let mut i = 0;
    while shard_of(&b.subject.loc) == shard_of(&a.subject.loc) {
        i += 1;
        b = claim(&format!("b{i}.md"), "b");
    }
    if shard_of(&a.subject.loc) > shard_of(&b.subject.loc) {
        std::mem::swap(&mut a, &mut b);
    }
    let old = claim("old.md", "old");
    let batch = Batch {
        claims: vec![a.clone(), b.clone()],
        marks: vec![
            mark(&a, "2026-11-20T00:00:00Z"),
            mark(&b, "2026-11-20T00:00:00Z"),
            mark(&old, "2026-11-20T00:00:01Z"),
        ],
        retirements: vec![retirement(&old, "2026-11-20T00:00:02Z")],
    };
    let shards_written = {
        let mut s: BTreeSet<_> = [shard_of(&a.subject.loc), shard_of(&b.subject.loc)].into();
        let claims = s.len();
        s.insert(shard_of(&old.subject.loc));
        claims + s.len()
    };
    let mut cases = 0;
    for step in STEPS {
        let occurrences = if step == Step::Locked {
            1
        } else {
            shards_written
        };
        for n in 0..occurrences {
            let (_dir, store) = new_store();
            store
                .append(&Batch {
                    claims: vec![old.clone()],
                    ..Batch::default()
                })
                .unwrap();
            fault::crash_at(step, n);
            let err = store.append(&batch).unwrap_err();
            fault::clear();
            assert_eq!(
                err.message.split(" (").next(),
                Some(fault::CRASH),
                "{step:?} {n}"
            );
            let ledger = assert_consistent(&store);
            // Claims are written before marks: once any mark of the batch is
            // visible, both new claims are.
            if !ledger.marks.is_empty() {
                assert!(ledger.claim(a.id.as_str()).is_some());
                assert!(ledger.claim(b.id.as_str()).is_some());
            }
            store.append(&batch).expect("the next append completes");
            let ledger = assert_consistent(&store);
            assert_eq!(ledger.claims.len(), 3);
            assert_eq!(ledger.marks.len(), 3);
            assert_eq!(ledger.retirements.len(), 1);
            cases += 1;
        }
    }
    assert_eq!(cases, 1 + 5 * shards_written);
}

/// R48: a temp-file name that already exists, as a file or as a symbolic
/// link, is never written through; a fresh name is chosen.
#[cfg(unix)]
#[test]
fn existing_temp_names_are_never_written_through() {
    let (dir, store) = new_store();
    let c = claim("a.md", "a");
    let shard = shard_digit(shard_of(&c.subject.loc));
    let claims_dir = dir.path().join(".hibi/claims");
    let outside = dir.path().join("outside.txt");
    fs::write(&outside, b"untouched").unwrap();
    let link_name = "1111111111111111";
    let file_name = "2222222222222222";
    std::os::unix::fs::symlink(
        &outside,
        claims_dir.join(format!("{shard}.jsonl.tmp-{link_name}")),
    )
    .unwrap();
    fs::write(
        claims_dir.join(format!("{shard}.jsonl.tmp-{file_name}")),
        b"existing",
    )
    .unwrap();
    fault::force_names(&[link_name, file_name, "3333333333333333"]);
    store
        .append(&Batch {
            claims: vec![c.clone()],
            ..Batch::default()
        })
        .unwrap();
    fault::clear();
    assert_eq!(fs::read(&outside).unwrap(), b"untouched");
    assert_eq!(
        fs::read(claims_dir.join(format!("{shard}.jsonl.tmp-{file_name}"))).unwrap(),
        b"existing"
    );
    assert!(
        !claims_dir
            .join(format!("{shard}.jsonl.tmp-3333333333333333"))
            .exists(),
        "the used temp file was renamed away"
    );
    assert_eq!(store.load().unwrap().claims, vec![c]);
}

/// After 1 + 8 names that all exist, the append fails with `io_error` and
/// writes nothing.
#[test]
fn temp_names_give_up_after_eight_retries() {
    let (dir, store) = new_store();
    let c = claim("a.md", "a");
    let shard = shard_digit(shard_of(&c.subject.loc));
    let claims_dir = dir.path().join(".hibi/claims");
    let names: Vec<String> = (0..=TEMP_RETRIES).map(|i| format!("{i:016x}")).collect();
    for n in &names {
        fs::write(claims_dir.join(format!("{shard}.jsonl.tmp-{n}")), b"x").unwrap();
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    fault::force_names(&refs);
    let before = store.snapshot_id().unwrap();
    let err = store
        .append(&Batch {
            claims: vec![c],
            ..Batch::default()
        })
        .unwrap_err();
    fault::clear();
    assert_eq!(err.code, Code::IoError);
    assert_eq!(store.snapshot_id().unwrap(), before);
}

#[test]
fn random_suffixes_are_16_hex_digits_and_differ() {
    let a = random_suffix();
    let b = random_suffix();
    assert_eq!(a.len(), 16);
    assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
}

use std::collections::BTreeSet;

/// A write that would take a shard or the store over its limit is refused
/// with `store_full` before anything is written, so that it can never make
/// every later read fail. The limits here are small stand-ins for
/// `MAX_SHARD_BYTES` and `MAX_STORE_BYTES`.
#[test]
fn a_write_over_a_size_limit_is_store_full_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path()).unwrap();
    let store = GitTextStore::open(dir.path()).unwrap();
    let a = claim("a.md", "one");
    let b = claim("a.md", "two");
    let line = |c: &Claim| c.canonical().unwrap().len() as u64 + 1;
    let roomy = Caps {
        shard: 1 << 20,
        store: 1 << 20,
    };
    let batch_of = |c: &Claim| Batch {
        claims: vec![c.clone()],
        ..Batch::default()
    };
    store.append_within(&batch_of(&a), roomy).unwrap();
    let before = store.snapshot_id().unwrap();
    let files = |root: &Path| -> Vec<(String, u64)> {
        shard_paths()
            .into_iter()
            .map(|p| {
                let len = fs::metadata(root.join(&p)).map(|m| m.len()).unwrap_or(0);
                (p, len)
            })
            .collect()
    };
    let sizes = files(dir.path());

    // The shard limit: exactly the new size fits, one byte less does not.
    let after = line(&a) + line(&b);
    let e = store
        .append_within(
            &batch_of(&b),
            Caps {
                shard: after - 1,
                store: 1 << 20,
            },
        )
        .unwrap_err();
    assert_eq!(e.code, Code::StoreFull);
    assert!(e.message.contains(&format!("{after} bytes long")), "{e}");
    assert!(
        e.message
            .contains(&format!("limit of {} bytes for one shard", after - 1)),
        "{e}"
    );
    assert!(
        e.remediation
            .as_deref()
            .unwrap()
            .contains("Nothing was written")
    );
    assert_eq!(store.snapshot_id().unwrap(), before);
    assert_eq!(files(dir.path()), sizes);

    // The store limit, with a mark in another file of the same write.
    let m = mark(&b, "2026-11-20T18:04:11Z");
    let both = Batch {
        claims: vec![b.clone()],
        marks: vec![m.clone()],
        ..Batch::default()
    };
    let total = after + m.canonical().unwrap().len() as u64 + 1;
    let e = store
        .append_within(
            &both,
            Caps {
                shard: 1 << 20,
                store: total - 1,
            },
        )
        .unwrap_err();
    assert_eq!(e.code, Code::StoreFull);
    assert!(
        e.message.contains(&format!("{total} bytes in total")),
        "{e}"
    );
    assert!(
        e.message
            .contains(&format!("limit of {} bytes for the store", total - 1)),
        "{e}"
    );
    assert_eq!(store.snapshot_id().unwrap(), before);
    assert_eq!(files(dir.path()), sizes);

    // Exactly at both limits the write goes through.
    store
        .append_within(
            &both,
            Caps {
                shard: after,
                store: total,
            },
        )
        .unwrap();
    let ledger = store.load().unwrap();
    assert_eq!((ledger.claims.len(), ledger.marks.len()), (2, 1));
    // The real limits are the ones `append` uses.
    assert_eq!((CAPS.shard, CAPS.store), (MAX_SHARD_BYTES, MAX_STORE_BYTES));
}

#[test]
fn check_room_compares_the_sizes_after_the_write() {
    let caps = Caps {
        shard: 100,
        store: 250,
    };
    let w = |now: u64, after: u64| ("s".to_owned(), now, after);
    assert!(check_room(caps, 0, &[]).is_ok());
    assert!(check_room(caps, 200, &[w(50, 100)]).is_ok());
    assert_eq!(
        check_room(caps, 200, &[w(50, 101)]).unwrap_err().code,
        Code::StoreFull
    );
    // 201 now, a shard grows from 50 to 100: 251 in total.
    assert_eq!(
        check_room(caps, 201, &[w(50, 100)]).unwrap_err().code,
        Code::StoreFull
    );
    assert!(check_room(caps, 201, &[w(50, 99)]).is_ok());
    // A write that only sorts a shard does not grow the store.
    assert!(check_room(caps, 250, &[w(100, 100)]).is_ok());
}
