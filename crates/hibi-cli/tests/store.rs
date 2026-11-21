//! The store through the binary (spec section 12): union merges of shards
//! (R7 to R9, the plan's V2), concurrent writers (R10, V3) and the
//! all-or-nothing standard-input batches (R33).

mod support;

use std::collections::BTreeSet;

use serde_json::{Value, json};
use support::*;

const HEX: &str = "0123456789abcdef";

/// A document of `n` one-line sentences. Sentence `i` is `fact(i)`.
fn facts(n: usize) -> String {
    let mut text = String::from("# Facts\n\n");
    for i in 0..n {
        text.push_str(&fact(i));
        text.push('\n');
    }
    text
}

fn fact(i: usize) -> String {
    format!("Fact number {i} holds here.")
}

/// The compact anchor of sentence `i` of a facts document.
fn fact_anchor(doc: &str, i: usize) -> String {
    format!("file:{doc}::{}", fact(i))
}

/// The ids of the records in a list, as strings.
fn ids_of(records: &[Value]) -> Vec<String> {
    records
        .iter()
        .map(|r| r["id"].as_str().expect("an id").to_owned())
        .collect()
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

/// The shard digit whose `claims/<d>.jsonl` holds this claim.
fn shard_of(repo: &Repo, claim: &str) -> char {
    let holding: Vec<char> = HEX
        .chars()
        .filter(|d| ids_of(&repo.shard(&format!("claims/{d}.jsonl"))).contains(&claim.to_owned()))
        .collect();
    assert_eq!(holding.len(), 1, "{claim} is in shards {holding:?}");
    holding[0]
}

/// Asserts the write rules of section 12.2 for every shard file: each line
/// is one JSON object followed by a line feed, ids ascend in byte order and
/// no id appears twice; claims shards hold claims only and marks shards hold
/// marks and retirements of claims of the same shard.
fn assert_shards_sorted(repo: &Repo) {
    for kind in ["claims", "marks"] {
        for d in HEX.chars() {
            let rel = format!("{kind}/{d}.jsonl");
            let text = repo.read(&format!(".hibi/{rel}"));
            assert!(text.is_empty() || text.ends_with('\n'), "{rel}: {text:?}");
            assert!(!text.contains("\n\n") && !text.contains('\r'), "{rel}");
            for line in text.lines() {
                let v: Value = serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("{rel}: not JSON ({e}): {line}"));
                assert!(v.is_object(), "{rel}: {line}");
            }
            let ids = ids_of(&repo.shard(&rel));
            assert!(
                ids.windows(2).all(|w| w[0].as_bytes() < w[1].as_bytes()),
                "{rel} is not sorted by id without duplicates: {ids:?}"
            );
            for id in &ids {
                let prefix = &id[..3];
                if kind == "claims" {
                    assert_eq!(prefix, "c1-", "{rel}: {id}");
                } else {
                    assert!(prefix == "m1-" || prefix == "r1-", "{rel}: {id}");
                }
            }
        }
    }
    let home: std::collections::BTreeMap<String, char> = HEX
        .chars()
        .flat_map(|d| {
            ids_of(&repo.shard(&format!("claims/{d}.jsonl")))
                .into_iter()
                .map(move |id| (id, d))
        })
        .collect();
    for d in HEX.chars() {
        for record in repo.shard(&format!("marks/{d}.jsonl")) {
            let claim = record["claim"].as_str().expect("a claim id");
            assert_eq!(home.get(claim), Some(&d), "{record}");
        }
    }
}

/// The ids `hibi list` shows, in its order.
fn listed_ids(repo: &Repo) -> Vec<String> {
    ids_of(repo.json(&["list"])["claims"].as_array().expect("claims"))
}

/// The stored marks of one claim, from `hibi list <id>`.
fn marks_of(repo: &Repo, id: &str) -> Vec<Value> {
    let v = repo.json(&["list", id]);
    assert_eq!(v["claim"]["id"], id, "{v}");
    v["marks"].as_array().expect("marks is an array").clone()
}

/// A fixed `now` after every mark these tests write, so that a claim with
/// `recheck` does not expire with the wall clock.
const LATER: &str = "2026-11-23T00:00:00Z";

/// `hibi --now LATER status <id>...`: `(id, state)` of every view, in output order.
fn states(repo: &Repo, ids: &[String]) -> Vec<(String, String)> {
    let mut args = vec!["--now", LATER, "status"];
    args.extend(ids.iter().map(String::as_str));
    repo.json(&args)["claims"]
        .as_array()
        .expect("claims")
        .iter()
        .map(|c| {
            (
                c["id"].as_str().unwrap().to_owned(),
                c["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn all_fresh(ids: &[String]) -> Vec<(String, String)> {
    sorted(ids.to_vec())
        .into_iter()
        .map(|id| (id, "fresh".to_owned()))
        .collect()
}

/// The paths that differ between two commits.
fn changed_paths(repo: &Repo, from: &str, to: &str) -> BTreeSet<String> {
    repo.git(&["diff", "--name-only", from, to])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// R7 and R8: two branches add claims to the same shard files, the merge
/// has no conflict, and the merged store holds the base claim and both.
#[test]
fn r7_r8_two_branches_claim_in_one_shard_and_merge_without_conflict() {
    let repo = Repo::new();
    repo.write("docs/facts.md", &facts(6));
    assert_eq!(
        repo.read(".hibi/.gitattributes"),
        "claims/*.jsonl text eol=lf merge=union linguist-generated\n\
         marks/*.jsonl text eol=lf merge=union linguist-generated\n"
    );
    let base = repo.claim("Fact 0.", &fact_anchor("docs/facts.md", 0), &[]);
    let d = shard_of(&repo, &base);
    let shard_files: BTreeSet<String> = [
        format!(".hibi/claims/{d}.jsonl"),
        format!(".hibi/marks/{d}.jsonl"),
    ]
    .into();
    repo.commit("base");
    // The lock is not committed.
    assert!(repo.root.join(".hibi/.lock").is_file());
    assert_eq!(repo.git(&["ls-files", ".hibi/.lock"]), "");

    repo.git(&["checkout", "-q", "-b", "left", "main"]);
    let left = [
        repo.claim("Fact 1.", &fact_anchor("docs/facts.md", 1), &[]),
        repo.claim("Fact 2.", &fact_anchor("docs/facts.md", 2), &[]),
    ];
    repo.commit("left");

    repo.git(&["checkout", "-q", "-b", "right", "main"]);
    assert_eq!(listed_ids(&repo), std::slice::from_ref(&base));
    let right = [
        repo.claim("Fact 3.", &fact_anchor("docs/facts.md", 3), &[]),
        repo.claim("Fact 4.", &fact_anchor("docs/facts.md", 4), &[]),
    ];
    repo.commit("right");

    // Both branches changed the same two shard files and nothing else.
    assert_eq!(changed_paths(&repo, "main", "left"), shard_files);
    assert_eq!(changed_paths(&repo, "main", "right"), shard_files);
    let mut all = vec![base.clone()];
    all.extend(left.iter().cloned());
    all.extend(right.iter().cloned());
    assert_eq!(all.iter().collect::<BTreeSet<_>>().len(), 5, "{all:?}");
    for id in &all {
        assert!(id.starts_with("c1-"), "{id}");
    }

    // `Repo::git` panics unless git exits 0.
    repo.git(&["checkout", "-q", "left"]);
    repo.git(&["merge", "-q", "--no-edit", "right"]);
    assert_eq!(repo.git(&["ls-files", "--unmerged"]), "");
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
    assert_eq!(
        repo.git(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .split(' ')
            .count(),
        3,
        "HEAD is a merge commit with two parents"
    );
    for (rel, bytes) in repo.store_files() {
        let text = String::from_utf8(bytes).expect("UTF-8");
        for marker in ["<<<<<<<", "=======", ">>>>>>>", "|||||||"] {
            assert!(!text.contains(marker), "{rel} has {marker}");
        }
    }

    // R8.
    let status = repo.run(&["status"]);
    assert_eq!(status.status, 0, "{status:?}");
    let status = status.ok();
    assert_eq!(
        status["summary"],
        json!({"claims": 5, "fresh": 5, "stale": 0, "expired": 0, "retired": 0})
    );
    assert_eq!(status["claims"], json!([]));
    assert_eq!(status["warnings"], json!([]));
    assert_eq!(listed_ids(&repo), sorted(all.clone()));
    assert_eq!(states(&repo, &all), all_fresh(&all));
    assert_eq!(sorted(ids_of(&repo.records("claims"))), sorted(all.clone()));
    assert_eq!(
        sorted(ids_of(&repo.shard(&format!("claims/{d}.jsonl")))),
        sorted(all.clone())
    );
    let marks = repo.records("marks");
    assert_eq!(marks.len(), 5);
    assert_eq!(repo.shard(&format!("marks/{d}.jsonl")).len(), 5);
    for id in &all {
        let own = marks_of(&repo, id);
        assert_eq!(own.len(), 1, "{id}");
        assert_eq!(own[0]["claim"], id.as_str());
        assert_eq!(own[0]["at"], NOW);
    }

    // The other direction merges as well and gives the same store content.
    repo.git(&["checkout", "-q", "right"]);
    repo.git(&["merge", "-q", "--no-edit", "left"]);
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
    assert_eq!(listed_ids(&repo), sorted(all.clone()));
    assert_eq!(states(&repo, &all), all_fresh(&all));
    assert_eq!(repo.records("marks").len(), 5);

    // A write after the merge leaves the merged shards in writer order.
    let v = repo.json(&["--now", "2026-11-21T12:00:00Z", "mark", &base]);
    assert_eq!(v["marks"].as_array().unwrap().len(), 1);
    assert_shards_sorted(&repo);
    assert_eq!(repo.records("claims").len(), 5);
    assert_eq!(repo.records("marks").len(), 6);
    assert_eq!(marks_of(&repo, &base).len(), 2);
}

/// R7 and R8 when both branches also write the same record: each marks the
/// base claim at the same instant, so the merged marks shard may hold that
/// line twice and still counts it once.
#[test]
fn r7_r8_the_same_mark_written_on_both_branches_counts_once() {
    let repo = Repo::new();
    repo.write("docs/facts.md", &facts(4));
    let base = repo.claim("Fact 0.", &fact_anchor("docs/facts.md", 0), &[]);
    repo.commit("base");
    let later = "2026-11-21T12:00:00Z";

    repo.git(&["checkout", "-q", "-b", "left", "main"]);
    let left = repo.claim("Fact 1.", &fact_anchor("docs/facts.md", 1), &[]);
    let left_mark = repo.json(&["--now", later, "mark", &base])["marks"][0]["mark"].clone();
    repo.commit("left");

    repo.git(&["checkout", "-q", "-b", "right", "main"]);
    let right = repo.claim("Fact 2.", &fact_anchor("docs/facts.md", 2), &[]);
    let right_mark = repo.json(&["--now", later, "mark", &base])["marks"][0]["mark"].clone();
    repo.commit("right");
    assert_eq!(
        left_mark, right_mark,
        "the mark id is derived from its body"
    );

    repo.git(&["merge", "-q", "--no-edit", "left"]);
    assert_eq!(repo.git(&["ls-files", "--unmerged"]), "");
    assert_eq!(repo.git(&["status", "--porcelain"]), "");

    let all = vec![base.clone(), left, right];
    let status = repo.json(&["status"]);
    assert_eq!(
        status["summary"],
        json!({"claims": 3, "fresh": 3, "stale": 0, "expired": 0, "retired": 0})
    );
    assert_eq!(status["warnings"], json!([]));
    assert_eq!(listed_ids(&repo), sorted(all.clone()));
    assert_eq!(states(&repo, &all), all_fresh(&all));
    // Four distinct marks: one per claim and the later one of the base.
    let mark_ids: BTreeSet<String> = ids_of(&repo.records("marks")).into_iter().collect();
    assert_eq!(mark_ids.len(), 4, "{mark_ids:?}");
    let base_marks = marks_of(&repo, &base);
    assert_eq!(base_marks.len(), 2);
    assert_eq!(base_marks[0]["at"], NOW);
    assert_eq!(base_marks[1]["at"], later);
    assert_eq!(base_marks[1]["id"], left_mark);
    let listed = repo.json(&["list"]);
    for claim in listed["claims"].as_array().unwrap() {
        let expected = if claim["id"] == base.as_str() { 2 } else { 1 };
        assert_eq!(claim["marks"], expected, "{claim}");
    }
}

/// R9: a shard with unsorted lines and a duplicated line loads, and the
/// next write leaves it sorted without duplicates.
#[test]
fn r9_an_unsorted_shard_with_a_duplicate_loads_and_the_next_write_sorts_it() {
    let repo = Repo::new();
    repo.write("docs/facts.md", &facts(6));
    let ids: Vec<String> = (0..4)
        .map(|i| repo.claim(&format!("Fact {i}."), &fact_anchor("docs/facts.md", i), &[]))
        .collect();
    let d = shard_of(&repo, &ids[0]);
    let claims_rel = format!(".hibi/claims/{d}.jsonl");
    let marks_rel = format!(".hibi/marks/{d}.jsonl");
    // What a union merge can leave: lines in descending order, one of them
    // twice, in both files.
    let scramble = |rel: &str| {
        let text = repo.read(rel);
        let mut lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{rel}");
        lines.reverse();
        lines.push(lines[1]);
        lines.insert(0, lines[2]);
        let mut out = lines.join("\n");
        out.push('\n');
        repo.write(rel, &out);
        assert_eq!(repo.read(rel).lines().count(), 6);
    };
    scramble(&claims_rel);
    scramble(&marks_rel);
    let scrambled = repo.store_files();
    let claim_lines = ids_of(&repo.shard(&format!("claims/{d}.jsonl")));
    assert!(claim_lines.windows(2).any(|w| w[0] > w[1]), "unsorted");
    assert_eq!(claim_lines.iter().collect::<BTreeSet<_>>().len(), 4);
    assert_eq!(claim_lines.len(), 6);

    // It loads: one record per id, no warning, and reading rewrites nothing.
    let status = repo.json(&["status"]);
    assert_eq!(
        status["summary"],
        json!({"claims": 4, "fresh": 4, "stale": 0, "expired": 0, "retired": 0})
    );
    assert_eq!(status["warnings"], json!([]));
    assert_eq!(listed_ids(&repo), sorted(ids.clone()));
    assert_eq!(states(&repo, &ids), all_fresh(&ids));
    for id in &ids {
        assert_eq!(marks_of(&repo, id).len(), 1, "{id}");
    }
    let listed = repo.json(&["list"]);
    for claim in listed["claims"].as_array().unwrap() {
        assert_eq!(claim["marks"], 1, "{claim}");
    }
    assert_eq!(repo.store_files(), scrambled);

    // The next write: a new claim and its mark go to the same two files.
    let added = repo.claim("Fact 4.", &fact_anchor("docs/facts.md", 4), &[]);
    assert_eq!(shard_of(&repo, &added), d);
    assert_shards_sorted(&repo);
    let mut all = ids.clone();
    all.push(added);
    assert_eq!(
        ids_of(&repo.shard(&format!("claims/{d}.jsonl"))),
        sorted(all.clone())
    );
    let marks = repo.shard(&format!("marks/{d}.jsonl"));
    assert_eq!(marks.len(), 5);
    let marked: BTreeSet<String> = marks
        .iter()
        .map(|m| m["claim"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(marked, all.iter().cloned().collect::<BTreeSet<_>>());
    assert_eq!(listed_ids(&repo), sorted(all.clone()));
    assert_eq!(states(&repo, &all), all_fresh(&all));
}

/// R9 for a write that adds only a mark: the marks shard is rewritten
/// sorted without duplicates.
#[test]
fn r9_a_mark_sorts_an_unsorted_marks_shard() {
    let repo = Repo::new();
    repo.write("docs/facts.md", &facts(4));
    let ids: Vec<String> = (0..3)
        .map(|i| repo.claim(&format!("Fact {i}."), &fact_anchor("docs/facts.md", i), &[]))
        .collect();
    let d = shard_of(&repo, &ids[0]);
    let marks_rel = format!(".hibi/marks/{d}.jsonl");
    let text = repo.read(&marks_rel);
    let mut lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    lines.reverse();
    lines.push(lines[0]);
    repo.write(&marks_rel, &(lines.join("\n") + "\n"));

    let v = repo.json(&["--now", "2026-11-21T12:00:00Z", "mark", &ids[1]]);
    let new_mark = v["marks"][0]["mark"].as_str().unwrap().to_owned();
    assert_eq!(v["marks"][0]["claim"], ids[1].as_str());
    assert_eq!(v["marks"][0]["claimView"]["state"], "fresh");
    assert_shards_sorted(&repo);
    let marks = repo.shard(&format!("marks/{d}.jsonl"));
    assert_eq!(marks.len(), 4);
    assert!(ids_of(&marks).contains(&new_mark));
    assert_eq!(marks_of(&repo, &ids[1]).len(), 2);
    assert_eq!(marks_of(&repo, &ids[0]).len(), 1);
    assert_eq!(marks_of(&repo, &ids[2]).len(), 1);
    assert_eq!(states(&repo, &ids), all_fresh(&ids));
}

/// One `hibi claim` started without waiting for it.
fn spawn_claim(repo: &Repo, statement: &str, subject: &str) -> std::process::Child {
    repo.hibi()
        .args([
            "--format",
            "json",
            "--now",
            NOW,
            "claim",
            statement,
            "--subject",
            subject,
        ])
        .spawn()
}

/// Waits for a spawned `hibi claim` and returns `(claim id, mark id)`.
fn finish_claim(child: std::process::Child) -> (String, String) {
    let out = out_of(child.wait_with_output().expect("hibi exits"));
    assert_eq!(out.status, 0, "{out:?}");
    let v = out.ok();
    assert_eq!(v["command"], "claim");
    assert_eq!(v["claims"].as_array().map(Vec::len), Some(1), "{v}");
    let made = &v["claims"][0];
    assert_eq!(made["created"], true, "{v}");
    assert_eq!(made["claim"]["state"], "fresh", "{v}");
    (
        made["id"].as_str().expect("an id").to_owned(),
        made["mark"].as_str().expect("a mark id").to_owned(),
    )
}

/// R10: two `hibi claim` processes at the same time, ten rounds. Even
/// rounds put both claims in one shard, odd rounds in two different shards.
#[test]
fn r10_two_claim_processes_at_the_same_time() {
    const ROUNDS: usize = 10;
    let repo = Repo::new();
    // Find two documents whose claims go to different shards.
    let mut docs: Vec<(String, char)> = Vec::new();
    let mut setup: Vec<(String, String)> = Vec::new();
    for n in 0..16 {
        let doc = format!("docs/facts-{n}.md");
        repo.write(&doc, &facts(2 * ROUNDS + 1));
        let v = repo.json(&[
            "--now",
            NOW,
            "claim",
            "The last fact.",
            "--subject",
            &fact_anchor(&doc, 2 * ROUNDS),
        ]);
        let id = v["claims"][0]["id"].as_str().unwrap().to_owned();
        let mark = v["claims"][0]["mark"].as_str().unwrap().to_owned();
        let d = shard_of(&repo, &id);
        setup.push((id, mark));
        match docs.first() {
            None => docs.push((doc, d)),
            Some((_, first)) if *first != d => {
                docs.push((doc, d));
                break;
            }
            Some(_) => {}
        }
    }
    assert_eq!(docs.len(), 2, "no two documents in different shards");
    let (doc_a, shard_a) = docs[0].clone();
    let (doc_b, shard_b) = docs[1].clone();
    assert_ne!(shard_a, shard_b);

    let mut claims: Vec<String> = setup.iter().map(|(c, _)| c.clone()).collect();
    let mut marks: Vec<String> = setup.iter().map(|(_, m)| m.clone()).collect();
    for round in 0..ROUNDS {
        let same_shard = round % 2 == 0;
        let second_doc = if same_shard { &doc_a } else { &doc_b };
        // Both are started before either is waited for.
        let first = spawn_claim(
            &repo,
            &format!("Round {round}, first."),
            &fact_anchor(&doc_a, 2 * round),
        );
        let second = spawn_claim(
            &repo,
            &format!("Round {round}, second."),
            &fact_anchor(second_doc, 2 * round + 1),
        );
        let (claim_1, mark_1) = finish_claim(first);
        let (claim_2, mark_2) = finish_claim(second);
        assert_ne!(claim_1, claim_2);
        assert_ne!(mark_1, mark_2);

        // Each claim and its mark are in the shard of its subject.
        assert_eq!(shard_of(&repo, &claim_1), shard_a, "round {round}");
        let expected = if same_shard { shard_a } else { shard_b };
        assert_eq!(shard_of(&repo, &claim_2), expected, "round {round}");
        for (claim, mark, d) in [(&claim_1, &mark_1, shard_a), (&claim_2, &mark_2, expected)] {
            let stored = repo.shard(&format!("marks/{d}.jsonl"));
            let line = stored
                .iter()
                .find(|m| m["id"] == mark.as_str())
                .unwrap_or_else(|| panic!("round {round}: mark {mark} is not in marks/{d}"));
            assert_eq!(line["claim"], claim.as_str(), "round {round}");
            assert_eq!(line["at"], NOW);
        }
        claims.extend([claim_1, claim_2]);
        marks.extend([mark_1, mark_2]);

        // Nothing is lost and nothing is added, after every round.
        assert_eq!(
            sorted(ids_of(&repo.records("claims"))),
            sorted(claims.clone()),
            "round {round}"
        );
        assert_eq!(
            sorted(ids_of(&repo.records("marks"))),
            sorted(marks.clone()),
            "round {round}"
        );
        assert_shards_sorted(&repo);
    }
    let total = setup.len() + 2 * ROUNDS;
    assert_eq!(claims.len(), total);
    assert_eq!(claims.iter().collect::<BTreeSet<_>>().len(), total);
    assert_eq!(marks.iter().collect::<BTreeSet<_>>().len(), total);

    // No temporary file is left, and the only other file is the lock.
    for rel in repo.store_files().keys() {
        assert!(!rel.contains(".tmp-"), "{rel}");
    }

    // The store passes the read checks: both readers succeed.
    let status = repo.run(&["status"]);
    assert_eq!(status.status, 0, "{status:?}");
    let status = status.ok();
    assert_eq!(
        status["summary"],
        json!({"claims": total, "fresh": total, "stale": 0, "expired": 0, "retired": 0})
    );
    assert_eq!(status["warnings"], json!([]));
    let list = repo.run(&["list"]);
    assert_eq!(list.status, 0, "{list:?}");
    assert_eq!(listed_ids(&repo), sorted(claims.clone()));
    assert_eq!(states(&repo, &claims), all_fresh(&claims));
    for claim in list.ok()["claims"].as_array().unwrap() {
        assert_eq!(claim["marks"], 1, "{claim}");
        assert_eq!(claim["lastMarkedAt"], NOW, "{claim}");
    }
}

/// R10 with more writers than two: eight `hibi claim` processes on one
/// shard at once.
#[test]
fn r10_eight_claim_processes_on_one_shard() {
    let repo = Repo::new();
    repo.write("docs/facts.md", &facts(8));
    let children: Vec<std::process::Child> = (0..8)
        .map(|i| {
            spawn_claim(
                &repo,
                &format!("Fact {i}."),
                &fact_anchor("docs/facts.md", i),
            )
        })
        .collect();
    let made: Vec<(String, String)> = children.into_iter().map(finish_claim).collect();
    let claims: Vec<String> = made.iter().map(|(c, _)| c.clone()).collect();
    let marks: Vec<String> = made.iter().map(|(_, m)| m.clone()).collect();
    assert_eq!(claims.iter().collect::<BTreeSet<_>>().len(), 8);
    let d = shard_of(&repo, &claims[0]);
    assert_eq!(
        ids_of(&repo.shard(&format!("claims/{d}.jsonl"))),
        sorted(claims.clone())
    );
    assert_eq!(
        ids_of(&repo.shard(&format!("marks/{d}.jsonl"))),
        sorted(marks.clone())
    );
    assert_eq!(repo.records("claims").len(), 8);
    assert_eq!(repo.records("marks").len(), 8);
    assert_shards_sorted(&repo);
    assert_eq!(listed_ids(&repo), sorted(claims.clone()));
    assert_eq!(states(&repo, &claims), all_fresh(&claims));
    assert_eq!(
        repo.json(&["status"])["summary"],
        json!({"claims": 8, "fresh": 8, "stale": 0, "expired": 0, "retired": 0})
    );
}

/// Run `hibi --format json --now <now> <command> --stdin` with this input.
fn stdin(repo: &Repo, now: &str, command: &str, input: &str) -> Out {
    repo.hibi()
        .args(["--format", "json", "--now", now, command, "--stdin"])
        .stdin(input)
        .run()
}

/// A repository with a document, a source file and one claim on them.
fn stdin_repo() -> (Repo, String) {
    let repo = Repo::new();
    repo.write("docs/facts.md", &facts(6));
    repo.write(
        "src/limits.rs",
        "pub const MAX: u32 = 5;\n\npub fn limit(a: u32) -> u32 {\n    a + 7\n}\n",
    );
    let id = repo.claim(
        "Fact 0.",
        &fact_anchor("docs/facts.md", 0),
        &["file:src/limits.rs#MAX"],
    );
    (repo, id)
}

/// The stored record with this id.
fn record(repo: &Repo, kind: &str, id: &str) -> Value {
    let found: Vec<Value> = repo
        .records(kind)
        .into_iter()
        .filter(|r| r["id"] == id)
        .collect();
    assert_eq!(found.len(), 1, "{kind} {id}");
    found[0].clone()
}

/// R33 happy path: an array of several claims with compact-string and
/// object anchors.
#[test]
fn r33_claim_stdin_writes_every_element() {
    let (repo, first) = stdin_repo();
    let input = json!([
        {
            "statement": "Fact 1.",
            "subject": fact_anchor("docs/facts.md", 1),
            "evidence": ["file:src/limits.rs#limit", "file:src/limits.rs#MAX"],
            "recheck": "P30D",
            "note": "  checked by hand  ",
            "by": "  ada  "
        },
        {
            "statement": "Fact 2.",
            "subject": {"loc": "file:docs/facts.md", "sel": {"quote": fact(2)}},
            "evidence": [
                {"loc": "file:src/limits.rs", "sel": {"symbol": "limit"}},
                "file:src/limits.rs::a + 7"
            ]
        },
        {
            "statement": "Fact 3.",
            "subject": fact_anchor("docs/facts.md", 3),
            "note": "   ",
            "by": "   "
        }
    ]);
    let at = "2026-11-21T08:00:00Z";
    let v = stdin(&repo, at, "claim", &input.to_string()).ok();
    assert_eq!(v["command"], "claim");
    assert_eq!(v["warnings"], json!([]));
    let made = v["claims"].as_array().unwrap();
    assert_eq!(made.len(), 3, "{v}");
    let ids = ids_of(made);
    let mark_ids: Vec<String> = made
        .iter()
        .map(|m| m["mark"].as_str().unwrap().to_owned())
        .collect();
    for (i, m) in made.iter().enumerate() {
        assert_eq!(m["created"], true, "{m}");
        assert_eq!(m["claim"]["id"], m["id"], "{m}");
        assert_eq!(m["claim"]["state"], "fresh", "{m}");
        assert_eq!(m["claim"]["mark"], m["mark"], "{m}");
        assert_eq!(m["claim"]["markedAt"], at, "{m}");
        assert_eq!(m["claim"]["statement"], format!("Fact {}.", i + 1));
    }

    // Element 0: trimmed note on the claim, trimmed `by` on the mark.
    let claim = record(&repo, "claims", &ids[0]);
    assert_eq!(claim["statement"], "Fact 1.");
    assert_eq!(claim["recheck"], "P30D");
    assert_eq!(claim["note"], "checked by hand");
    assert_eq!(claim["subject"]["loc"], "file:docs/facts.md");
    assert_eq!(claim["subject"]["sel"]["quote"], fact(1));
    assert_eq!(claim["evidence"].as_array().unwrap().len(), 2);
    assert_eq!(claim["evidence"][0]["sel"]["symbol"], "limit");
    assert_eq!(claim["evidence"][1]["sel"]["symbol"], "MAX");
    let mark = record(&repo, "marks", &mark_ids[0]);
    assert_eq!(mark["claim"], ids[0].as_str());
    assert_eq!(mark["at"], at);
    assert_eq!(mark["by"], "ada");
    assert_eq!(mark["note"], Value::Null);
    assert_eq!(mark["sides"].as_array().unwrap().len(), 3);
    assert_eq!(made[0]["claim"]["expiresAt"], "2026-12-21T08:00:00Z");

    // Element 1: object anchors and a compact quote; no `by`, no note.
    let claim = record(&repo, "claims", &ids[1]);
    assert_eq!(claim["recheck"], Value::Null);
    assert_eq!(claim["note"], Value::Null);
    assert_eq!(claim["subject"]["sel"]["quote"], fact(2));
    assert_eq!(claim["evidence"][0]["loc"], "file:src/limits.rs");
    assert_eq!(claim["evidence"][0]["sel"]["symbol"], "limit");
    assert_eq!(claim["evidence"][1]["sel"]["quote"], "a + 7");
    let mark = record(&repo, "marks", &mark_ids[1]);
    assert_eq!(mark["claim"], ids[1].as_str());
    assert_eq!(mark["by"], Value::Null);
    assert_eq!(mark["note"], Value::Null);
    assert_eq!(mark["sides"].as_array().unwrap().len(), 3);

    // Element 2: `by` and `note` that are empty after trimming are null.
    let claim = record(&repo, "claims", &ids[2]);
    assert_eq!(claim["note"], Value::Null);
    assert_eq!(claim["evidence"], json!([]));
    let mark = record(&repo, "marks", &mark_ids[2]);
    assert_eq!(mark["by"], Value::Null);
    assert_eq!(mark["note"], Value::Null);
    assert_eq!(mark["sides"].as_array().unwrap().len(), 1);

    // The store holds the earlier claim and these three, each with one
    // mark, and all are fresh.
    let mut all = vec![first];
    all.extend(ids.iter().cloned());
    assert_eq!(listed_ids(&repo), sorted(all.clone()));
    assert_eq!(states(&repo, &all), all_fresh(&all));
    assert_eq!(repo.records("claims").len(), 4);
    assert_eq!(repo.records("marks").len(), 4);
    assert_shards_sorted(&repo);

    // The same input again creates no claim and, at a later time, one new
    // mark per element.
    let again = stdin(&repo, "2026-11-22T08:00:00Z", "claim", &input.to_string()).ok();
    let again = again["claims"].as_array().unwrap();
    assert_eq!(ids_of(again), ids);
    for m in again {
        assert_eq!(m["created"], false, "{m}");
    }
    assert_eq!(repo.records("claims").len(), 4);
    assert_eq!(repo.records("marks").len(), 7);
}

/// R33: one invalid element among valid ones writes nothing and is
/// reported with its index.
#[test]
fn r33_claim_stdin_with_one_invalid_element_writes_nothing() {
    let (repo, first) = stdin_repo();
    let before = repo.store_files();
    let valid_0 = json!({"statement": "Fact 1.", "subject": fact_anchor("docs/facts.md", 1)});
    let valid_2 = json!({"statement": "Fact 2.", "subject": fact_anchor("docs/facts.md", 2),
        "evidence": ["file:src/limits.rs#limit"]});
    let cases = [
        (
            json!({"statement": "Nowhere.", "subject": "file:docs/facts.md::This sentence is not in the document."}),
            "side_missing",
        ),
        (
            json!({"statement": "No file.", "subject": "file:docs/absent.md::Fact number 1 holds here."}),
            "side_missing",
        ),
        (
            json!({"statement": "Bad evidence.", "subject": fact_anchor("docs/facts.md", 3),
                "evidence": ["file:src/limits.rs#no_such_symbol"]}),
            "side_missing",
        ),
        (
            json!({"statement": "Bad duration.", "subject": fact_anchor("docs/facts.md", 3),
                "recheck": "90 days"}),
            "invalid_duration",
        ),
        (
            json!({"statement": "Undeclared scheme.", "subject": "jira:ABC-1"}),
            "unknown_scheme",
        ),
        (
            json!({"statement": "Key selector.",
                "subject": {"loc": "file:docs/facts.md", "sel": {"key": "sentence:0123456789ab"}}}),
            "unsupported_selector",
        ),
    ];
    for (invalid, code) in &cases {
        let input = json!([valid_0, invalid, valid_2]);
        let out = stdin(&repo, NOW, "claim", &input.to_string());
        let errors = out.errs();
        assert_eq!(out.json()["command"], "claim");
        assert_eq!(errors.len(), 1, "{code}: {out:?}");
        assert_eq!(errors[0]["code"], *code, "{out:?}");
        assert_eq!(errors[0]["index"], 1, "{code}: {out:?}");
        assert_eq!(repo.store_files(), before, "{code}");
    }

    // The invalid element first and last, and two invalid elements: every
    // failing element is reported, and still nothing is written.
    let invalid = &cases[0].0;
    let out = stdin(&repo, NOW, "claim", &json!([invalid, valid_0]).to_string());
    assert_eq!(out.errs().len(), 1, "{out:?}");
    assert_eq!(out.err()["index"], 0);
    let input = json!([valid_0, invalid, valid_2, cases[3].0]);
    let out = stdin(&repo, NOW, "claim", &input.to_string());
    let errors = out.errs();
    let reported: Vec<(Value, Value)> = errors
        .iter()
        .map(|e| (e["index"].clone(), e["code"].clone()))
        .collect();
    assert_eq!(
        reported,
        [
            (json!(1), json!("side_missing")),
            (json!(3), json!("invalid_duration"))
        ],
        "{out:?}"
    );
    assert_eq!(repo.store_files(), before);

    // Only the earlier claim is in the store.
    assert_eq!(listed_ids(&repo), std::slice::from_ref(&first));
    assert_eq!(repo.records("marks").len(), 1);
    assert_eq!(repo.state(&first), "fresh");

    // The valid elements alone are accepted, so they were not the cause.
    let v = stdin(&repo, NOW, "claim", &json!([valid_0, valid_2]).to_string()).ok();
    assert_eq!(v["claims"].as_array().unwrap().len(), 2);
    assert_eq!(repo.records("claims").len(), 3);
    assert_eq!(repo.records("marks").len(), 3);
}

/// R33 happy path of `mark --stdin`: id strings and objects.
#[test]
fn r33_mark_stdin_writes_every_element() {
    let (repo, first) = stdin_repo();
    let second = repo.claim("Fact 1.", &fact_anchor("docs/facts.md", 1), &[]);
    let third = repo.claim("Fact 2.", &fact_anchor("docs/facts.md", 2), &[]);
    let fourth = repo.claim("Fact 3.", &fact_anchor("docs/facts.md", 3), &[]);
    assert_eq!(repo.records("marks").len(), 4);
    let at = "2026-11-21T08:00:00Z";
    let input = json!([
        first,
        {"claim": second, "note": "  read again  ", "by": "  grace  "},
        {"claim": third},
        {"claim": fourth, "note": "  ", "by": ""}
    ]);
    let v = stdin(&repo, at, "mark", &input.to_string()).ok();
    assert_eq!(v["command"], "mark");
    assert_eq!(v["warnings"], json!([]));
    let made = v["marks"].as_array().unwrap();
    assert_eq!(made.len(), 4, "{v}");
    let all = [first.clone(), second.clone(), third.clone(), fourth.clone()];
    let expected = [
        (Value::Null, Value::Null),
        (json!("grace"), json!("read again")),
        (Value::Null, Value::Null),
        (Value::Null, Value::Null),
    ];
    for (i, m) in made.iter().enumerate() {
        assert_eq!(m["claim"], all[i].as_str(), "{m}");
        assert_eq!(m["claimView"]["id"], all[i].as_str(), "{m}");
        assert_eq!(m["claimView"]["state"], "fresh", "{m}");
        assert_eq!(m["claimView"]["mark"], m["mark"], "{m}");
        assert_eq!(m["claimView"]["markedAt"], at, "{m}");
        let mark = record(&repo, "marks", m["mark"].as_str().unwrap());
        assert_eq!(mark["claim"], all[i].as_str());
        assert_eq!(mark["at"], at);
        assert_eq!(mark["by"], expected[i].0, "element {i}: {mark}");
        assert_eq!(mark["note"], expected[i].1, "element {i}: {mark}");
        // `hibi list <id>` shows the first mark and this one, by `at`.
        let stored = marks_of(&repo, &all[i]);
        assert_eq!(stored.len(), 2, "element {i}");
        assert_eq!(stored[0]["at"], NOW);
        assert_eq!(stored[1], mark);
    }
    assert_eq!(repo.records("claims").len(), 4);
    assert_eq!(repo.records("marks").len(), 8);
    assert_shards_sorted(&repo);
    assert_eq!(states(&repo, &all), all_fresh(&all));
}

/// R33 for `mark --stdin`: one invalid element among valid ones.
#[test]
fn r33_mark_stdin_with_one_invalid_element_writes_nothing() {
    let (repo, first) = stdin_repo();
    let second = repo.claim("Fact 1.", &fact_anchor("docs/facts.md", 1), &[]);
    let retired = repo.claim("Fact 2.", &fact_anchor("docs/facts.md", 2), &[]);
    let v = repo.json(&["--now", NOW, "retire", &retired]);
    assert_eq!(v["retired"][0]["claim"], retired.as_str());
    repo.write("src/gone.rs", "pub const GONE: u32 = 1;\n");
    let broken = repo.claim(
        "Fact 3.",
        &fact_anchor("docs/facts.md", 3),
        &["file:src/gone.rs"],
    );
    let before = repo.store_files();
    let marks_before = repo.records("marks").len();
    let at = "2026-11-21T08:00:00Z";

    // An id that names no claim.
    let unknown = "c1-zzzzzzzzzzzzzzzzzzzz";
    let input = json!([first, {"claim": unknown, "note": "x"}, {"claim": second, "by": "al"}]);
    let out = stdin(&repo, at, "mark", &input.to_string());
    let errors = out.errs();
    assert_eq!(out.json()["command"], "mark");
    assert_eq!(errors.len(), 1, "{out:?}");
    assert_eq!(errors[0]["code"], "claim_not_found");
    assert_eq!(errors[0]["index"], 1);
    assert_eq!(repo.store_files(), before);

    // A retired claim, as the last element.
    let input = json!([first, second, retired]);
    let out = stdin(&repo, at, "mark", &input.to_string());
    let errors = out.errs();
    assert_eq!(errors.len(), 1, "{out:?}");
    assert_eq!(errors[0]["code"], "claim_retired");
    assert_eq!(errors[0]["index"], 2);
    assert_eq!(repo.store_files(), before);

    // A claim with a side that no longer resolves, as the first element.
    std::fs::remove_file(repo.root.join("src/gone.rs")).expect("the file is removed");
    assert_eq!(repo.state(&broken), "stale");
    let input = json!([{"claim": broken}, first, second]);
    let out = stdin(&repo, at, "mark", &input.to_string());
    let errors = out.errs();
    assert_eq!(errors.len(), 1, "{out:?}");
    assert_eq!(errors[0]["code"], "side_missing");
    assert_eq!(errors[0]["index"], 0);
    assert_eq!(repo.store_files(), before);

    assert_eq!(repo.records("marks").len(), marks_before);
    assert_eq!(marks_of(&repo, &first).len(), 1);
    assert_eq!(marks_of(&repo, &second).len(), 1);

    // The valid elements alone are accepted.
    let v = stdin(&repo, at, "mark", &json!([first, second]).to_string()).ok();
    assert_eq!(v["marks"].as_array().unwrap().len(), 2);
    assert_eq!(repo.records("marks").len(), marks_before + 2);
}

/// Section 15.3: an empty array and input that is not JSON are
/// `invalid_json`, for both commands, and write nothing.
#[test]
fn r33_stdin_that_is_empty_or_not_json_is_invalid_json() {
    let (repo, first) = stdin_repo();
    let before = repo.store_files();
    for command in ["claim", "mark"] {
        for input in [
            "[]",
            "  [ ]\n",
            "nope",
            "",
            "[",
            "{\"statement\": \"x\"}",
            "42",
        ] {
            let out = stdin(&repo, NOW, command, input);
            assert_eq!(out.code(), "invalid_json", "{command} {input:?}: {out:?}");
            assert_eq!(out.json()["command"], command);
            assert_eq!(repo.store_files(), before, "{command} {input:?}");
        }
    }
    assert_eq!(listed_ids(&repo), std::slice::from_ref(&first));
    assert_eq!(repo.state(&first), "fresh");
}
