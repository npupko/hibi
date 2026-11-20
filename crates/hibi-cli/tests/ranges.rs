//! `--since` and coverage through the binary: spec sections 10.1 to 10.3 and
//! 15.3; requirements R19, R20, R21 (V5) and R38.

mod support;

use serde_json::{Value, json};
use support::*;

const SOURCE: &str = "pub fn first() -> u32 {\n    1\n}\n\npub fn second() -> u32 {\n    2\n}\n";
const GUIDE: &str = "# Guide\n\nFirst returns one. Second returns two.\n";
const NEW_SECTION: &str = "\n## Extra\n\nThird thing. Fourth thing.\n";

const DOCS_SCOPE: &str = "[[scope]]\nname = \"docs\"\nresource = \"file:docs/**/*.md\"\nkinds = [\"sentence\", \"heading\"]\n";

/// The ids of the listed claims, in output order.
fn ids(v: &Value) -> Vec<String> {
    v["claims"]
        .as_array()
        .expect("claims is an array")
        .iter()
        .map(|c| c["id"].as_str().expect("an id").to_owned())
        .collect()
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

/// `(kind, text)` of every item of one scope, in output order.
fn items(scope: &Value) -> Vec<(String, String)> {
    scope["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .map(|i| {
            assert_eq!(i["anchor"]["loc"], i["loc"], "{i}");
            (
                i["unit"]["kind"].as_str().expect("a kind").to_owned(),
                i["text"].as_str().expect("a text").to_owned(),
            )
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(k, t)| ((*k).to_owned(), (*t).to_owned()))
        .collect()
}

fn counts(v: &Value) -> (u64, u64, u64, u64) {
    (
        v["units"].as_u64().expect("units"),
        v["covered"].as_u64().expect("covered"),
        v["excluded"].as_u64().expect("excluded"),
        v["uncovered"].as_u64().expect("uncovered"),
    )
}

/// The one scope of a `cover` result.
fn only_scope(v: &Value) -> Value {
    let scopes = v["scopes"].as_array().expect("scopes is an array");
    assert_eq!(scopes.len(), 1, "{v}");
    scopes[0].clone()
}

fn replace(repo: &Repo, rel: &str, from: &str, to: &str) {
    let text = repo.read(rel);
    assert_eq!(text.matches(from).count(), 1, "{rel}: {from:?}");
    repo.write(rel, &text.replace(from, to));
}

/// The scenario of R19 and R20.
///
/// Base commit on `main`: `src/a.rs` with `first` and `second`,
/// `docs/guide.md` in scope `docs`, every unit of the scope claimed. One
/// claim with `second` as evidence is stale at the base, because `second`
/// was edited after the claim and before the commit.
///
/// Branch `feature`: the literal of `first` is edited and a section is
/// appended to the guide; committed or left in the working tree.
struct Branch {
    repo: Repo,
    base: String,
    /// The two claims that have `first` as a side.
    on_first: Vec<String>,
    /// Stale at the base: `second` as evidence, marked before its edit.
    second_stale_at_base: String,
    /// Fresh at the base: `second` as subject, claimed after its edit.
    second_fresh: String,
    heading: String,
}

fn branch(commit_the_edit: bool) -> Branch {
    let repo = Repo::new();
    repo.config(DOCS_SCOPE);
    repo.write("src/a.rs", SOURCE);
    repo.write("docs/guide.md", GUIDE);

    let first_doc = repo.claim(
        "first returns one",
        "file:docs/guide.md::First returns one.",
        &["file:src/a.rs#first"],
    );
    let first_fn = repo.claim("first is the constant 1", "file:src/a.rs#first", &[]);
    let second_stale_at_base = repo.claim(
        "second returns two",
        "file:docs/guide.md::Second returns two.",
        &["file:src/a.rs#second"],
    );
    replace(&repo, "src/a.rs", "    2\n", "    22\n");
    let second_fresh = repo.claim("second is a constant", "file:src/a.rs#second", &[]);
    let heading = repo.claim("the guide has a title", "file:docs/guide.md::# Guide", &[]);

    // The base: the scope is fully covered and exactly one claim is stale.
    let cover = repo.json(&["cover"]);
    assert_eq!(counts(&cover["summary"]), (3, 3, 0, 0), "{cover}");
    let status = repo.json(&["status"]);
    assert_eq!(ids(&status), vec![second_stale_at_base.clone()]);
    assert_eq!(
        status["summary"],
        json!({"claims": 5, "fresh": 4, "drifted": 0, "stale": 1, "expired": 0, "retired": 0})
    );

    let base = repo.commit("base");
    repo.git(&["checkout", "-q", "-b", "feature"]);
    replace(&repo, "src/a.rs", "    1\n", "    11\n");
    let mut guide = repo.read("docs/guide.md");
    guide.push_str(NEW_SECTION);
    repo.write("docs/guide.md", &guide);
    if commit_the_edit {
        let head = repo.commit("feature");
        assert_ne!(head, base);
    }

    Branch {
        repo,
        base,
        on_first: sorted(vec![first_doc, first_fn]),
        second_stale_at_base,
        second_fresh,
        heading,
    }
}

fn new_section_items() -> Vec<(String, String)> {
    pairs(&[
        ("heading", "## Extra"),
        ("sentence", "Third thing."),
        ("sentence", "Fourth thing."),
    ])
}

/// The checks of R19 on one `status --since` result.
fn assert_lists_the_first_function_claims(b: &Branch, v: &Value, range: &str, base: &str) {
    assert_eq!(v["command"], "status");
    assert_eq!(v["since"], json!({"range": range, "base": base}), "{range}");
    // Output order is by id.
    assert_eq!(ids(v), b.on_first, "{range}");
    assert!(!ids(v).contains(&b.second_stale_at_base));
    // The summary counts every claim in the store, before filtering.
    assert_eq!(
        v["summary"],
        json!({"claims": 5, "fresh": 2, "drifted": 0, "stale": 3, "expired": 0, "retired": 0}),
        "{range}"
    );
    for claim in v["claims"].as_array().unwrap() {
        assert_eq!(claim["state"], "stale", "{claim}");
        let id = claim["id"].as_str().unwrap();
        assert_eq!(claim["next"], format!("hibi mark {id}"));
        let changed: Vec<&Value> = claim["sides"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["state"] != "same")
            .collect();
        assert_eq!(changed.len(), 1, "{claim}");
        assert_eq!(changed[0]["anchor"]["loc"], "file:src/a.rs");
        assert_eq!(changed[0]["anchor"]["sel"]["symbol"], "first");
        assert_eq!(changed[0]["state"], "changed");
        assert_eq!(changed[0]["reason"], "fingerprint_differs");
        assert_eq!(changed[0]["unit"]["key"], "function:first");
    }
}

/// R19 (V5): `status --since <base>` lists exactly the claims anchored to the
/// edited function and not the claim that was already stale at the base, in
/// every accepted form of the range.
#[test]
fn r19_status_since_lists_exactly_the_claims_the_range_invalidated() {
    let b = branch(true);
    let repo = &b.repo;

    // Plain status also lists the claim that was stale before the range.
    let plain = repo.json(&["status"]);
    assert_eq!(plain["since"], Value::Null);
    let mut all_stale = b.on_first.clone();
    all_stale.push(b.second_stale_at_base.clone());
    assert_eq!(ids(&plain), sorted(all_stale));
    assert_eq!(repo.state(&b.second_fresh), "fresh");
    assert_eq!(repo.state(&b.heading), "fresh");

    let sha = b.base.as_str();
    for a in [sha, "main", "HEAD~1"] {
        for suffix in ["", "..", "..HEAD", "...", "...HEAD"] {
            let range = format!("{a}{suffix}");
            let v = repo.json(&["status", "--since", &range]);
            assert_lists_the_first_function_claims(&b, &v, &range, sha);
        }
    }

    // `--loc` filters the listed claims further.
    let v = repo.json(&["status", "--since", "main", "--loc", "file:docs/**"]);
    assert_eq!(ids(&v).len(), 1, "{v}");
    assert_eq!(v["claims"][0]["statement"], "first returns one");
    let v = repo.json(&["status", "--since", "main", "--loc", "file:nothing/**"]);
    assert_eq!(ids(&v), Vec::<String>::new());
}

/// Section 10.3: `A`, `A..` and `A..HEAD` use commit `A`; `A...` and
/// `A...HEAD` use the merge base of `A` and `HEAD`. Shown with a `main` that
/// moved after the branch was cut: its new commit edits `second` again.
#[test]
fn r19_the_three_dot_forms_use_the_merge_base() {
    let b = branch(true);
    let repo = &b.repo;
    repo.git(&["checkout", "-q", "main"]);
    replace(repo, "src/a.rs", "    22\n", "    222\n");
    let main_tip = repo.commit("main moves on");
    repo.git(&["checkout", "-q", "feature"]);
    assert_ne!(main_tip, b.base);
    assert_eq!(repo.git(&["merge-base", "main", "HEAD"]), b.base);

    for range in ["main...", "main...HEAD"] {
        let v = repo.json(&["status", "--since", range]);
        assert_lists_the_first_function_claims(&b, &v, range, &b.base);
    }

    // With the tip of main as the base, `second` differs between the base
    // and the working tree as well, so the claim that is stale on it is
    // listed too. The claim that is fresh on `second` is not: only stale
    // claims are listed.
    let mut expected = b.on_first.clone();
    expected.push(b.second_stale_at_base.clone());
    let expected = sorted(expected);
    for range in ["main", "main..", "main..HEAD"] {
        let v = repo.json(&["status", "--since", range]);
        assert_eq!(v["since"], json!({"range": range, "base": main_tip}));
        assert_eq!(ids(&v), expected, "{range}");
        assert!(!ids(&v).contains(&b.second_fresh));
    }
}

/// R20 (V5): `cover --since <base>` lists exactly the units of the new
/// section; plain `cover` lists the same units, because the base was fully
/// covered.
#[test]
fn r20_cover_since_lists_exactly_the_units_of_the_new_section() {
    let b = branch(true);
    let repo = &b.repo;
    let sha = b.base.as_str();

    for a in [sha, "main"] {
        for suffix in ["", "..", "..HEAD", "...", "...HEAD"] {
            let range = format!("{a}{suffix}");
            let v = repo.json(&["cover", "--since", &range]);
            assert_eq!(v["command"], "cover");
            assert_eq!(v["since"], json!({"range": range, "base": sha}));
            // With --since the counts cover only the kept units.
            assert_eq!(counts(&v["summary"]), (3, 0, 0, 3), "{range}: {v}");
            let scope = only_scope(&v);
            assert_eq!(scope["name"], "docs");
            assert_eq!(scope["skipped"], Value::Null);
            assert_eq!(scope["truncated"], false);
            assert_eq!(counts(&scope), (3, 0, 0, 3));
            assert_eq!(items(&scope), new_section_items(), "{range}");
            for item in scope["items"].as_array().unwrap() {
                assert_eq!(item["loc"], "file:docs/guide.md");
            }
            assert_eq!(scope["unavailable"], json!([]));
        }
    }

    let plain = repo.json(&["cover"]);
    assert_eq!(plain["since"], Value::Null);
    assert_eq!(counts(&plain["summary"]), (6, 3, 0, 3), "{plain}");
    let scope = only_scope(&plain);
    assert_eq!(counts(&scope), (6, 3, 0, 3));
    assert_eq!(items(&scope), new_section_items());
    let lines: Vec<(u64, u64)> = scope["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["unit"]["line"].as_u64().unwrap(),
                i["unit"]["endLine"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(lines, vec![(5, 5), (7, 7), (7, 7)]);

    // The anchors of the listed items can be passed to `claim --stdin`
    // (section 15.3); once claimed, the new section is covered.
    let input: Vec<Value> = scope["items"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(n, i)| json!({"statement": format!("new unit {n}"), "subject": i["anchor"]}))
        .collect();
    let claimed = repo
        .hibi()
        .args(["--format", "json", "--now", NOW, "claim", "--stdin"])
        .stdin(&Value::Array(input).to_string())
        .run()
        .ok();
    assert_eq!(claimed["claims"].as_array().unwrap().len(), 3);
    let after = repo.json(&["cover", "--since", sha]);
    assert_eq!(counts(&after["summary"]), (3, 3, 0, 0), "{after}");
    assert_eq!(items(&only_scope(&after)), Vec::new());
    let after = repo.json(&["cover"]);
    assert_eq!(counts(&after["summary"]), (6, 6, 0, 0), "{after}");
}

/// Section 10.3: the compared end is the working tree, so uncommitted edits
/// and untracked files count.
#[test]
fn r19_r20_uncommitted_and_untracked_changes_count() {
    let b = branch(false);
    let repo = &b.repo;
    let sha = b.base.as_str();
    assert_eq!(repo.git(&["rev-parse", "HEAD"]), sha);
    assert_ne!(repo.git(&["status", "--porcelain"]), "");

    for range in ["HEAD", "main", "main...", sha] {
        let v = repo.json(&["status", "--since", range]);
        assert_lists_the_first_function_claims(&b, &v, range, sha);

        let v = repo.json(&["cover", "--since", range]);
        assert_eq!(v["since"], json!({"range": range, "base": sha}));
        assert_eq!(counts(&v["summary"]), (3, 0, 0, 3), "{v}");
        assert_eq!(items(&only_scope(&v)), new_section_items());
    }

    // An untracked file in scope: all of its units are new.
    repo.write("docs/new.md", "Brand new sentence.\n");
    assert!(
        repo.git(&["status", "--porcelain"])
            .contains("?? docs/new.md")
    );
    let v = repo.json(&["cover", "--since", "HEAD"]);
    assert_eq!(counts(&v["summary"]), (4, 0, 0, 4), "{v}");
    let scope = only_scope(&v);
    let mut expected = new_section_items();
    expected.push(("sentence".to_owned(), "Brand new sentence.".to_owned()));
    assert_eq!(items(&scope), expected);
    assert_eq!(scope["items"][3]["loc"], "file:docs/new.md");

    // An ignored untracked file is not a changed path and not in the file
    // list.
    repo.write(".gitignore", "docs/ignored.md\n");
    repo.write("docs/ignored.md", "Ignored sentence.\n");
    let v = repo.json(&["cover", "--since", "HEAD"]);
    assert_eq!(items(&only_scope(&v)), expected);
    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (7, 3, 0, 4), "{v}");
    assert_eq!(items(&only_scope(&v)), expected);
}

/// R21 (V5): a right-hand side other than `HEAD`, an unknown revision and any
/// other form are `invalid_range`, for `status` and for `cover`. `--since`
/// with `--state` or with ids is `usage`.
#[test]
fn r21_other_range_forms_are_invalid_range() {
    let b = branch(true);
    let repo = &b.repo;
    let sha = b.base.as_str();
    let before = repo.files();

    let bad = [
        format!("{sha}..feature"),
        format!("{sha}..main"),
        format!("{sha}..{sha}"),
        "main..feature".to_owned(),
        "main...feature".to_owned(),
        format!("{sha}...main"),
        "no-such-revision".to_owned(),
        "no-such-revision..".to_owned(),
        "no-such-revision...HEAD".to_owned(),
        "main....".to_owned(),
        format!("{sha}...."),
        "..HEAD".to_owned(),
        "...".to_owned(),
        // A blob, not a commit.
        "HEAD:src/a.rs".to_owned(),
    ];
    for range in &bad {
        for command in ["status", "cover"] {
            let out = repo.run(&[command, "--since", range]);
            let e = out.err();
            assert_eq!(e["code"], "invalid_range", "{command} --since {range}: {e}");
            assert_eq!(out.json()["command"], command);
            assert_eq!(out.errs().len(), 1);
        }
    }

    let out = repo.run(&["status", "--since", sha, "--state", "stale"]);
    assert_eq!(out.code(), "usage", "{out:?}");
    let out = repo.run(&["status", "--since", sha, &b.on_first[0]]);
    assert_eq!(out.code(), "usage", "{out:?}");
    let out = repo.run(&[
        "status",
        &b.on_first[0],
        "--since",
        "main...",
        "--state",
        "fresh",
    ]);
    assert_eq!(out.code(), "usage", "{out:?}");

    assert_eq!(repo.files(), before);
}

const CODE: &str = "pub struct Counter;\n\nimpl Counter {\n    pub fn bump(&self) -> u32 {\n        1\n    }\n\n    pub fn reset(&self) -> u32 {\n        0\n    }\n}\n\npub fn free() -> u32 {\n    7\n}\n";

fn keys(scope: &Value) -> Vec<String> {
    scope["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .map(|i| i["unit"]["key"].as_str().expect("a key").to_owned())
        .collect()
}

/// R38: a unit is covered when a live claim side resolves to it or to a unit
/// nested in it (section 10.2: the side covers its unit and every ancestor by
/// the `parent` chain). Siblings and children of the claimed unit are not
/// covered.
#[test]
fn r38_a_claim_covers_its_unit_and_every_enclosing_unit() {
    let repo = Repo::new();
    repo.config(
        "[[scope]]\nname = \"code\"\nresource = \"file:src/**/*.rs\"\nkinds = [\"struct\", \"impl\", \"method\", \"function\"]\n\n[[scope]]\nname = \"files\"\nresource = \"file:src/**/*.rs\"\nkinds = [\"file\"]\n",
    );
    repo.write("src/lib.rs", CODE);

    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (6, 0, 0, 6), "{v}");
    let code = &v["scopes"][0];
    assert_eq!(code["name"], "code");
    assert_eq!(counts(code), (5, 0, 0, 5));
    // Ordered by locator, then start offset.
    assert_eq!(
        keys(code),
        [
            "struct:Counter",
            "impl:impl Counter",
            "method:Counter.bump",
            "method:Counter.reset",
            "function:free"
        ]
    );
    let files = &v["scopes"][1];
    assert_eq!(files["name"], "files");
    assert_eq!(counts(files), (1, 0, 0, 1));
    assert_eq!(keys(files), ["file"]);
    assert_eq!(
        files["items"][0]["anchor"],
        json!({"loc": "file:src/lib.rs", "sel": {}})
    );

    // A claim on the method covers the method, the impl around it and the
    // file; not the sibling method, the struct or the free function.
    let id = repo.claim("bump adds one", "file:src/lib.rs#Counter.bump", &[]);
    assert_eq!(
        repo.view(&id)["sides"][0]["unit"]["key"],
        "method:Counter.bump"
    );
    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (6, 3, 0, 3), "{v}");
    assert_eq!(counts(&v["scopes"][0]), (5, 2, 0, 3));
    assert_eq!(
        keys(&v["scopes"][0]),
        ["struct:Counter", "method:Counter.reset", "function:free"]
    );
    assert_eq!(counts(&v["scopes"][1]), (1, 1, 0, 0));
    assert_eq!(keys(&v["scopes"][1]), Vec::<String>::new());

    // An evidence side covers as a subject does.
    repo.write("docs/a.md", "Free returns seven.\n");
    repo.claim(
        "free returns seven",
        "file:docs/a.md::Free returns seven.",
        &["file:src/lib.rs#free"],
    );
    let v = repo.json(&["cover", "--scope", "code"]);
    assert_eq!(counts(&v["scopes"][0]), (5, 3, 0, 2));
    assert_eq!(
        keys(&v["scopes"][0]),
        ["struct:Counter", "method:Counter.reset"]
    );

    // A claim on the impl covers the impl, not the methods nested in it.
    let repo = Repo::new();
    repo.config(
        "[[scope]]\nname = \"code\"\nresource = \"file:src/**/*.rs\"\nkinds = [\"struct\", \"impl\", \"method\", \"function\"]\n",
    );
    repo.write("src/lib.rs", CODE);
    repo.claim("Counter has methods", "file:src/lib.rs#impl Counter", &[]);
    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (5, 1, 0, 4), "{v}");
    assert_eq!(
        keys(&only_scope(&v)),
        [
            "struct:Counter",
            "method:Counter.bump",
            "method:Counter.reset",
            "function:free"
        ]
    );
}

/// Section 10.2: the claim's state does not matter, a stale claim still
/// covers; a retired claim is not live and covers nothing.
#[test]
fn r38_a_stale_claim_covers_and_a_retired_claim_does_not() {
    let repo = Repo::new();
    repo.config(
        "[[scope]]\nname = \"code\"\nresource = \"file:src/**/*.rs\"\nkinds = [\"function\"]\n",
    );
    repo.write("src/a.rs", SOURCE);
    let id = repo.claim("first is the constant 1", "file:src/a.rs#first", &[]);

    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (2, 1, 0, 1), "{v}");
    assert_eq!(keys(&only_scope(&v)), ["function:second"]);

    replace(&repo, "src/a.rs", "    1\n", "    11\n");
    assert_eq!(repo.state(&id), "stale");
    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (2, 1, 0, 1), "{v}");
    assert_eq!(keys(&only_scope(&v)), ["function:second"]);

    let retired = repo.json(&["retire", &id]);
    assert_eq!(retired["retired"][0]["claim"], id.as_str());
    assert_eq!(retired["retired"][0]["alreadyRetired"], false);
    assert_eq!(repo.state(&id), "retired");
    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (2, 0, 0, 2), "{v}");
    assert_eq!(keys(&only_scope(&v)), ["function:first", "function:second"]);
}

/// R38: every exclusion is reported with its reason and the number of units
/// it excluded. A covered unit is counted as covered, not as excluded.
#[test]
fn r38_exclusions_are_reported_with_reason_and_match_count() {
    let repo = Repo::new();
    repo.write("docs/a.md", "Alpha is one. Beta is two. Gamma is three.\n");
    repo.write("docs/changelog.md", "Released one. Released two.\n");

    // The key of one sentence of docs/a.md, for the unit exclusion.
    repo.write(
        ".hibi/hibi.toml",
        "version = 1\n\n[[scope]]\nname = \"docs\"\nresource = \"file:docs/**/*.md\"\nkinds = [\"sentence\"]\n",
    );
    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (5, 0, 0, 5), "{v}");
    let scope = only_scope(&v);
    assert_eq!(scope["exclusions"], json!([]));
    let gamma = scope["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["text"] == "Gamma is three.")
        .expect("the sentence is listed")["unit"]["key"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(gamma.starts_with("sentence:"), "{gamma}");

    repo.write(
        ".hibi/hibi.toml",
        &format!(
            "version = 1\n\n[[scope]]\nname = \"docs\"\nresource = \"file:docs/**/*.md\"\nkinds = [\"sentence\"]\nexclude = [\n  {{ resource = \"file:docs/changelog.md\", reason = \"release notes, not claims\" }},\n  {{ resource = \"file:docs/a.md\", unit = \"{gamma}\", reason = \"an aside\" }},\n  {{ resource = \"file:docs/absent.md\", reason = \"matches nothing\" }},\n]\n"
        ),
    );
    let expected_exclusions = |changelog: u64| {
        json!([
            {"resource": "file:docs/changelog.md", "unit": null,
             "reason": "release notes, not claims", "matched": changelog},
            {"resource": "file:docs/a.md", "unit": gamma, "reason": "an aside", "matched": 1},
            {"resource": "file:docs/absent.md", "unit": null, "reason": "matches nothing", "matched": 0},
        ])
    };
    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (5, 0, 3, 2), "{v}");
    let scope = only_scope(&v);
    assert_eq!(counts(&scope), (5, 0, 3, 2));
    assert_eq!(
        items(&scope),
        pairs(&[("sentence", "Alpha is one."), ("sentence", "Beta is two.")])
    );
    assert_eq!(scope["exclusions"], expected_exclusions(2));

    // The human output names each exclusion with its reason.
    let human = repo.human(&["cover"]);
    assert_eq!(human.status, 0, "{human:?}");
    assert_eq!(human.stderr, "");
    for needle in [
        "release notes, not claims",
        "an aside",
        "matches nothing",
        "Alpha is one.",
        "Beta is two.",
    ] {
        assert!(human.stdout.contains(needle), "{needle}: {}", human.stdout);
    }
    assert!(!human.stdout.contains("Released one."), "{}", human.stdout);

    // A claimed unit of the excluded file is covered, so the exclusion
    // excluded one unit fewer.
    repo.claim(
        "one was released",
        "file:docs/changelog.md::Released one.",
        &[],
    );
    let v = repo.json(&["cover"]);
    assert_eq!(counts(&v["summary"]), (5, 1, 2, 2), "{v}");
    assert_eq!(only_scope(&v)["exclusions"], expected_exclusions(1));
}

/// Section 15.3: `--limit` caps `items` per scope and never the counts;
/// `--scope` selects scopes; an unknown scope and a config with no scope are
/// errors.
#[test]
fn r38_limit_scope_selection_and_scope_errors() {
    let repo = Repo::new();
    let before = repo.files();
    let e = repo.fail(&["cover"]);
    assert_eq!(e["code"], "no_scopes");
    let e = repo.fail(&["cover", "--scope", "docs"]);
    assert_eq!(e["code"], "no_scopes");
    assert_eq!(repo.files(), before);

    repo.config(
        "[[scope]]\nname = \"docs\"\nresource = \"file:docs/**/*.md\"\nkinds = [\"sentence\"]\n\n[[scope]]\nname = \"titles\"\nresource = \"file:docs/**/*.md\"\nkinds = [\"heading\"]\n",
    );
    repo.write(
        "docs/a.md",
        "# One\n\nAlpha is one. Beta is two. Gamma is three.\n",
    );
    repo.write("docs/b.md", "# Two\n\nDelta is four.\n");
    repo.claim("beta", "file:docs/a.md::Beta is two.", &[]);

    let all = repo.json(&["cover"]);
    assert_eq!(counts(&all["summary"]), (6, 1, 0, 5), "{all}");
    assert_eq!(all["scopes"].as_array().unwrap().len(), 2);
    let docs = &all["scopes"][0];
    assert_eq!(docs["name"], "docs");
    assert_eq!(docs["resource"], "file:docs/**/*.md");
    assert_eq!(docs["kinds"], json!(["sentence"]));
    assert_eq!(counts(docs), (4, 1, 0, 3));
    assert_eq!(docs["truncated"], false);
    // Ordered by locator, then start offset.
    assert_eq!(
        items(docs),
        pairs(&[
            ("sentence", "Alpha is one."),
            ("sentence", "Gamma is three."),
            ("sentence", "Delta is four.")
        ])
    );
    let titles = &all["scopes"][1];
    assert_eq!(titles["name"], "titles");
    assert_eq!(counts(titles), (2, 0, 0, 2));
    assert_eq!(
        items(titles),
        pairs(&[("heading", "# One"), ("heading", "# Two")])
    );

    // --limit 1: one item per scope, both truncated, counts as before.
    let v = repo.json(&["cover", "--limit", "1"]);
    assert_eq!(counts(&v["summary"]), (6, 1, 0, 5), "{v}");
    assert_eq!(counts(&v["scopes"][0]), (4, 1, 0, 3));
    assert_eq!(v["scopes"][0]["truncated"], true);
    assert_eq!(
        items(&v["scopes"][0]),
        pairs(&[("sentence", "Alpha is one.")])
    );
    assert_eq!(counts(&v["scopes"][1]), (2, 0, 0, 2));
    assert_eq!(v["scopes"][1]["truncated"], true);
    assert_eq!(items(&v["scopes"][1]), pairs(&[("heading", "# One")]));

    // A limit that is not reached does not truncate: 2 cuts only `docs`.
    let v = repo.json(&["cover", "--limit", "2"]);
    assert_eq!(v["scopes"][0]["truncated"], true);
    assert_eq!(v["scopes"][0]["items"].as_array().unwrap().len(), 2);
    assert_eq!(v["scopes"][1]["truncated"], false);
    assert_eq!(v["scopes"][1]["items"].as_array().unwrap().len(), 2);
    assert_eq!(counts(&v["summary"]), (6, 1, 0, 5));

    // 0 is no cap.
    let v = repo.json(&["cover", "--limit", "0"]);
    assert_eq!(v["scopes"], all["scopes"]);

    // --scope selects; the summary counts the selected scopes.
    let v = repo.json(&["cover", "--scope", "titles"]);
    assert_eq!(counts(&v["summary"]), (2, 0, 0, 2), "{v}");
    let scope = only_scope(&v);
    assert_eq!(scope, all["scopes"][1]);
    let v = repo.json(&["cover", "--scope", "docs", "--limit", "1"]);
    assert_eq!(counts(&v["summary"]), (4, 1, 0, 3), "{v}");
    let scope = only_scope(&v);
    assert_eq!(scope["name"], "docs");
    assert_eq!(scope["truncated"], true);
    let v = repo.json(&["cover", "--scope", "docs", "--scope", "titles"]);
    assert_eq!(v["scopes"], all["scopes"]);
    assert_eq!(v["summary"], all["summary"]);

    let before = repo.files();
    let out = repo.run(&["cover", "--scope", "nope"]);
    assert_eq!(out.code(), "unknown_scope", "{out:?}");
    assert!(out.err()["message"].as_str().unwrap().contains("nope"));
    let out = repo.run(&["cover", "--scope", "docs", "--scope", "nope"]);
    assert_eq!(out.code(), "unknown_scope", "{out:?}");
    assert_eq!(repo.files(), before);
}
