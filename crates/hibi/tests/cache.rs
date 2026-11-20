//! Integration tests of `cache` (spec section 11.3).
//!
//! The multi-process test starts this test binary again with
//! `HIBI_TEST_CACHE_CHILD` set; the test `child_role` then opens the cache.

#![cfg(feature = "cache")]

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};

use hibi::cache::{Cache, ENGINE, JudgeEntry};

#[test]
fn creates_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deep/hibi/cache.sqlite3");
    let cache = Cache::open(&path).unwrap();
    assert_eq!(cache.engine(), ENGINE);
    assert_eq!(cache.units("b", "markdown").unwrap(), None);
    cache.put_units("b", "markdown", "[]").unwrap();
    cache.put_units("b", "text", "\"not_text\"").unwrap();
    assert_eq!(cache.units("b", "markdown").unwrap().as_deref(), Some("[]"));
    assert_eq!(
        cache.units("b", "text").unwrap().as_deref(),
        Some("\"not_text\"")
    );
    let entry = JudgeEntry {
        model: "jev-1.14.0".into(),
        answer: "{\"type\":\"noul\",\"noul\":0.5}".into(),
        at: "2026-11-20T18:04:11Z".into(),
    };
    assert_eq!(cache.judge("k").unwrap(), None);
    cache.put_judge("k", &entry).unwrap();
    assert_eq!(cache.judge("k").unwrap(), Some(entry));
    drop(cache);
    // Reopening keeps the rows.
    let cache = Cache::open(&path).unwrap();
    assert_eq!(cache.units("b", "markdown").unwrap().as_deref(), Some("[]"));
}

#[test]
fn a_file_that_is_not_a_database_is_unavailable_and_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.sqlite3");
    let junk = vec![0x5au8; 8192];
    fs::write(&path, &junk).unwrap();
    assert!(Cache::open(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), junk);
}

#[test]
fn an_unopenable_location_is_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a-file");
    fs::write(&file, "x").unwrap();
    assert!(Cache::open(&file.join("cache.sqlite3")).is_err());
}

#[test]
fn threads_opening_a_missing_cache_all_succeed() {
    for _ in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("cache.sqlite3"));
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let c = Cache::open(&path).expect("every opener succeeds");
                    c.put_units(&format!("b{i}"), "markdown", "[]").unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let c = Cache::open(&path).unwrap();
        for i in 0..8 {
            assert!(c.units(&format!("b{i}"), "markdown").unwrap().is_some());
        }
    }
}

const CHILD_ENV: &str = "HIBI_TEST_CACHE_CHILD";

#[test]
fn child_role() {
    let Ok(spec) = std::env::var(CHILD_ENV) else {
        return;
    };
    let (index, path) = spec.split_once(':').unwrap();
    let c = Cache::open(Path::new(path)).expect("the child opens the cache");
    c.put_units(&format!("child{index}"), "markdown", "[]")
        .unwrap();
}

#[test]
fn processes_opening_a_missing_cache_all_succeed() {
    for _ in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.sqlite3");
        let children: Vec<_> = (0..4)
            .map(|i| {
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "child_role", "--test-threads=1"])
                    .env(CHILD_ENV, format!("{i}:{}", path.display()))
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for mut c in children {
            assert!(c.wait().unwrap().success());
        }
        // Each child stored one row; the parent sees all four.
        let c = Cache::open(&path).unwrap();
        for i in 0..4 {
            assert!(c.units(&format!("child{i}"), "markdown").unwrap().is_some());
        }
    }
}
