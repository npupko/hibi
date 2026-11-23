//! One test per rule of spec sections 9.3, 10.2, 10.3 and 15.2 that no
//! other engine test fails on when the rule is broken: a mark that does not
//! fit, the note in the rebuilt claim id, what covers a unit, and the
//! conditions of `--since`.

use hibi::config::{RepoConfig, UserConfig};
use hibi::engine::{
    self, ClaimState, CoverEnv, CoverQuery, MarkInput, Reason, SideState, Since, SinceRun, Skipped,
    StatusQuery, changed_since,
};
use hibi::fingerprint::prose_v1;
use hibi::model::{Claim, ClaimId, Mark, MarkSide};
use hibi::provider::Providers;
use hibi::store::Batch;

use crate::support::*;

// ---------------------------------------------------------------------------
// Section 9.3: a mark that does not fit
// ---------------------------------------------------------------------------

/// A mark with the right number of sides and a wrong locator on one side
/// does not fit the claim. It never matches and is never the reference
/// mark, also when its fingerprints equal the current ones and it is the
/// latest mark.
#[test]
fn a_mark_with_a_wrong_side_locator_is_ignored() {
    let w = key_world();
    set(&w, "t:a", vec![unit("k", "note", "subject")]);
    set(&w, "t:b", vec![unit("k", "note", "evidence")]);
    let mut p = providers(&w, &["t"]);
    let store = MemStore::new();

    // A claim with no mark: put in directly.
    let claim = Claim::new(
        "S",
        anchor("t:a", key_sel("k")),
        vec![anchor("t:b", key_sel("k"))],
        None,
        None,
    )
    .unwrap();
    let id = claim.id.clone();
    store.insert(Batch {
        claims: vec![claim],
        ..Batch::default()
    });

    // Two sides, the current fingerprints, and `t:c` where the claim has
    // `t:b`.
    let misfit = Mark::new(
        id.clone(),
        ts("2026-11-10T00:00:00Z"),
        None,
        vec![
            MarkSide {
                loc: loc("t:a"),
                fp: prose_v1("subject"),
            },
            MarkSide {
                loc: loc("t:c"),
                fp: prose_v1("evidence"),
            },
        ],
        None,
    )
    .unwrap();
    let misfit_id = misfit.id.clone();
    store.insert(Batch {
        marks: vec![misfit],
        ..Batch::default()
    });

    // The claim has only that mark: it is stale, and its sides say that
    // there is no mark.
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Stale);
    assert_eq!(v.mark, None);
    assert_eq!(v.marked_at, None);
    assert_eq!(v.sides.len(), 2);
    for side in &v.sides {
        assert_eq!(
            (side.state, side.reason),
            (SideState::Changed, Some(Reason::NoMark)),
            "side {}",
            side.index
        );
        assert_eq!(side.marked_fp, None);
    }

    // A mark that fits, made earlier than the other one, is the reference
    // mark while it matches.
    let out = engine::mark(
        &store,
        &mut p,
        &no_git(),
        &[MarkInput::new(id.as_str())],
        false,
        ts("2026-11-05T00:00:00Z"),
    )
    .unwrap();
    let fit_id = out.marks[0].mark.clone();
    assert_ne!(fit_id, misfit_id);
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Fresh);
    assert_eq!(v.mark, Some(fit_id.clone()));

    // When no mark matches, the reference mark is the latest mark that
    // fits, not the later one that does not.
    edit(&w, "t:b", "k", "evidence, edited");
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Stale);
    assert_eq!(v.mark, Some(fit_id));
    assert_eq!(v.marked_at, Some(ts("2026-11-05T00:00:00Z")));
    assert_eq!(
        (v.sides[1].state, v.sides[1].reason),
        (SideState::Changed, Some(Reason::FingerprintDiffers))
    );
}

// ---------------------------------------------------------------------------
// Section 10.2: what covers a unit
// ---------------------------------------------------------------------------

const MODEL_CONFIG: &str = r#"
version = 1

[schemes.arch]

[[scope]]
name = "model"
resource = "arch:model"
kinds = ["component"]
"#;

/// Coverage of the one external scope `arch:model`, which needs no file
/// list.
fn cover_model(store: &MemStore, p: &mut Providers) -> engine::CoverOutput {
    let config = RepoConfig::parse(MODEL_CONFIG).unwrap();
    let user = UserConfig::default();
    let dir = tempfile::tempdir().unwrap();
    let git = real_git();
    let env = CoverEnv {
        config: &config,
        user: &user,
        root: dir.path(),
        git: &git,
    };
    let query = CoverQuery {
        scopes: Vec::new(),
        limit: 0,
    };
    engine::cover(store, p, &env, &query, None).unwrap()
}

fn model_world() -> (Shared, Providers) {
    let w = key_world();
    set(
        &w,
        "arch:model",
        vec![
            unit("a", "component", "auth issues tokens"),
            unit("b", "component", "billing charges cards"),
        ],
    );
    let p = providers(&w, &["arch"]);
    (w, p)
}

/// A side whose anchor resolves to several units covers none of them.
#[test]
fn an_ambiguous_side_covers_no_unit() {
    let (w, mut p) = model_world();
    let store = MemStore::new();
    let id = claim_at(
        &store,
        &mut p,
        engine::ClaimInput {
            subject: json_anchor("arch:model", key_sel("a")),
            ..claim_input("S", "arch:model", &[])
        },
        NOW,
    );
    // While the side resolves to one unit, that unit is covered.
    let out = cover_model(&store, &mut p);
    assert_eq!(
        (
            out.summary.units,
            out.summary.covered,
            out.summary.uncovered
        ),
        (2, 1, 1)
    );

    // Now the stored anchor resolves to both units.
    w.borrow_mut().overrides.insert(
        anchor("arch:model", key_sel("a")),
        Ok(vec![
            unit("a", "component", "auth issues tokens"),
            unit("b", "component", "billing charges cards"),
        ]),
    );
    let v = view(&store, &mut p, &id);
    assert_eq!(
        (v.sides[0].state, v.sides[0].reason),
        (SideState::Ambiguous, Some(Reason::SeveralUnits))
    );
    let out = cover_model(&store, &mut p);
    assert_eq!(
        (
            out.summary.units,
            out.summary.covered,
            out.summary.uncovered
        ),
        (2, 0, 2)
    );
    let keys: Vec<&str> = out.scopes[0]
        .items
        .iter()
        .map(|i| i.unit.key.as_str())
        .collect();
    assert_eq!(keys, ["a", "b"]);
}

/// Coverage does not read marks: a live claim that was never marked covers
/// the unit its side resolves to.
#[test]
fn a_claim_with_no_mark_covers_its_unit() {
    let (_w, mut p) = model_world();
    let store = MemStore::new();
    let claim = Claim::new("S", anchor("arch:model", key_sel("a")), vec![], None, None).unwrap();
    let id = claim.id.clone();
    store.insert(Batch {
        claims: vec![claim],
        ..Batch::default()
    });
    assert_eq!(store.counts(), (1, 0, 0));
    let v = view(&store, &mut p, &id);
    assert_eq!(v.state, ClaimState::Stale);
    assert_eq!(v.mark, None);

    let out = cover_model(&store, &mut p);
    assert_eq!(
        (
            out.summary.units,
            out.summary.covered,
            out.summary.uncovered
        ),
        (2, 1, 1)
    );
    let keys: Vec<&str> = out.scopes[0]
        .items
        .iter()
        .map(|i| i.unit.key.as_str())
        .collect();
    assert_eq!(keys, ["b"]);
}

// ---------------------------------------------------------------------------
// Section 10.3: `--since`
// ---------------------------------------------------------------------------

/// `cover --since` drops a unit only when a base unit has both its key and
/// its fingerprint.
#[test]
fn changed_since_needs_the_same_key_and_the_same_fingerprint() {
    let base = [
        unit("same", "function", "fn same() {1}"),
        unit("edited", "function", "fn edited() {1}"),
        unit("original", "function", "fn body() {1}"),
    ];
    // Same key, same fingerprint: dropped.
    assert!(!changed_since(
        &unit("same", "function", "fn same() {1}"),
        &base
    ));
    // Same key, another fingerprint: kept.
    assert!(changed_since(
        &unit("edited", "function", "fn edited() {2}"),
        &base
    ));
    // Another key, the fingerprint of a base unit: kept.
    let renamed = unit("renamed", "function", "fn body() {1}");
    assert_eq!(renamed.fp, base[2].fp);
    assert!(changed_since(&renamed, &base));
    // The key of one base unit and the fingerprint of another: kept.
    let mixed = unit("edited", "function", "fn same() {1}");
    assert_eq!(mixed.fp, base[0].fp);
    assert!(changed_since(&mixed, &base));
    // A resource absent at the base has no base units: kept.
    assert!(changed_since(
        &unit("same", "function", "fn same() {1}"),
        &[]
    ));
}

const SINCE_CONFIG: &str = r#"
version = 1

[repos]
api = "../api"

[schemes.arch]

[[scope]]
name = "code"
resource = "file:src/**/*.rs"
kinds = ["function"]

[[scope]]
name = "sibling"
resource = "repo:api//src/*.rs"
kinds = ["function"]
exclude = [{ resource = "repo:api//src/gen.rs", reason = "generated" }]

[[scope]]
name = "model"
resource = "arch:model"
kinds = ["component"]
exclude = [{ resource = "arch:model", unit = "a", reason = "an example" }]
"#;

/// A real repository in which `src/a.rs` and `docs/d.md` differ from the
/// base commit and `src/b.rs` does not, with one scripted provider for the
/// working tree and one for the base commit.
struct SinceFixture {
    dir: tempfile::TempDir,
    base: Shared,
    p: Providers,
    store: MemStore,
    git: hibi::git::Git,
    since: Since,
    /// Stale; `src/a.rs`; one unit at the base, another fingerprint now.
    edited: ClaimId,
    /// Stale; `src/a.rs`; the anchor finds no unit at the base.
    renamed: ClaimId,
    /// Stale; `src/a.rs`; the anchor finds two units at the base.
    amb: ClaimId,
    /// Stale; `src/b.rs`, which is not a changed path; the base provider
    /// answers with another fingerprint than the working tree.
    unchanged_path: ClaimId,
    /// Expired; `docs/d.md`; one unit at the base, another fingerprint now.
    expired: ClaimId,
}

fn since_fixture() -> SinceFixture {
    let dir = repo(&[
        ("src/a.rs", "base a\n"),
        ("src/b.rs", "base b\n"),
        ("docs/d.md", "One.\n"),
    ]);
    let base_commit = commit_all(dir.path(), "base");
    write_files(
        dir.path(),
        &[("src/a.rs", "work a\n"), ("docs/d.md", "Two.\n")],
    );

    let work = file_world();
    let base = file_world();
    let arch = key_world();
    set(
        &arch,
        "arch:model",
        vec![
            unit("a", "component", "auth issues tokens"),
            unit("b", "component", "billing charges cards"),
        ],
    );
    let mut p = Providers::new();
    p.register(&["file", "repo"], Box::new(Mem::new(&work)));
    p.register(&["arch"], Box::new(Mem::new(&arch)));
    let store = MemStore::new();

    // The contents when the claims were made and marked.
    set(
        &work,
        "file:src/a.rs",
        vec![
            sym_unit("same", "function", "fn same() {1}"),
            sym_unit("edited", "function", "fn edited() {1}"),
            sym_unit("renamed", "function", "fn body() {0}"),
            sym_unit("amb", "function", "fn amb() {0}"),
        ],
    );
    set(
        &work,
        "file:src/b.rs",
        vec![sym_unit("h", "function", "fn h() {0}")],
    );
    set(
        &work,
        "file:docs/d.md",
        vec![sym_unit("s1", "sentence", "Two.")],
    );
    let mut claim = |statement: &str, subject: &str, recheck: Option<&str>| {
        let input = engine::ClaimInput {
            recheck: recheck.map(str::to_owned),
            ..claim_input(statement, subject, &[])
        };
        claim_at(&store, &mut p, input, "2026-01-01T00:00:00Z")
    };
    let edited = claim("edited returns 1", "file:src/a.rs#edited", None);
    let renamed = claim("renamed returns 0", "file:src/a.rs#renamed", None);
    let amb = claim("amb returns 0", "file:src/a.rs#amb", None);
    let unchanged_path = claim("h returns 0", "file:src/b.rs#h", None);
    let expired = claim("two", "file:docs/d.md#s1", Some("P1D"));

    // The base commit. `amb` is not in the unit list, and the anchor of
    // the claim on it finds two units there.
    set(
        &base,
        "file:src/a.rs",
        vec![
            sym_unit("same", "function", "fn same() {1}"),
            sym_unit("edited", "function", "fn edited() {1}"),
            sym_unit("original", "function", "fn body() {1}"),
        ],
    );
    base.borrow_mut().overrides.insert(
        anchor("file:src/a.rs", symbol_sel("amb")),
        Ok(vec![
            sym_unit("amb", "function", "fn amb() {0}"),
            sym_unit("amb2", "function", "fn amb() {0}"),
        ]),
    );
    set(
        &base,
        "file:src/b.rs",
        vec![sym_unit("h", "function", "fn h() {1}")],
    );
    set(
        &base,
        "file:docs/d.md",
        vec![sym_unit("s1", "sentence", "One.")],
    );
    // The working tree.
    set(
        &work,
        "file:src/a.rs",
        vec![
            sym_unit("same", "function", "fn same() {1}"),
            sym_unit("edited", "function", "fn edited() {2}"),
            sym_unit("renamed", "function", "fn body() {1}"),
            sym_unit("amb", "function", "fn amb() {1}"),
        ],
    );
    set(
        &work,
        "file:src/b.rs",
        vec![sym_unit("h", "function", "fn h() {2}")],
    );

    // The claims of this fixture are older than the range: the base commit
    // holds their ids.
    let first_commit = base_commit;
    let base_commit = commit_claim_ids(dir.path(), &store);
    assert_ne!(base_commit, first_commit);
    let git = real_git();
    let since = engine::resolve_since(&git, dir.path(), "HEAD").unwrap();
    assert_eq!(since.base.as_str(), base_commit);
    let changed = git.changed_paths(dir.path(), &since.base).unwrap();
    assert_eq!(
        changed.iter().map(String::as_str).collect::<Vec<_>>(),
        ["docs/d.md", "src/a.rs"]
    );
    SinceFixture {
        dir,
        base,
        p,
        store,
        git,
        since,
        edited,
        renamed,
        amb,
        unchanged_path,
        expired,
    }
}

impl SinceFixture {
    /// The ids `status --since` lists, after a check of the state of every
    /// claim without `--since`.
    fn listed(&mut self) -> Vec<ClaimId> {
        let all = engine::status(
            &self.store,
            &mut self.p,
            &no_git(),
            &StatusQuery::default(),
            None,
            now(),
        )
        .unwrap();
        let state = |id: &ClaimId| all.claims.iter().find(|c| c.id == *id).unwrap().state;
        for id in [&self.edited, &self.renamed, &self.amb, &self.unchanged_path] {
            assert_eq!(state(id), ClaimState::Stale);
        }
        assert_eq!(state(&self.expired), ClaimState::Expired);

        let mut base = Mem::new(&self.base);
        let run = SinceRun {
            since: &self.since,
            base: &mut base,
            git: &self.git,
            root: self.dir.path(),
        };
        let out = engine::status(
            &self.store,
            &mut self.p,
            &no_git(),
            &StatusQuery::default(),
            Some(run),
            now(),
        )
        .unwrap();
        out.claims.into_iter().map(|c| c.id).collect()
    }

    fn cover(&mut self) -> engine::CoverOutput {
        let config = RepoConfig::parse(SINCE_CONFIG).unwrap();
        let user = UserConfig::default();
        let env = CoverEnv {
            config: &config,
            user: &user,
            root: self.dir.path(),
            git: &self.git,
        };
        let mut base = Mem::new(&self.base);
        let run = SinceRun {
            since: &self.since,
            base: &mut base,
            git: &self.git,
            root: self.dir.path(),
        };
        let query = CoverQuery {
            scopes: Vec::new(),
            limit: 0,
        };
        engine::cover(&self.store, &mut self.p, &env, &query, Some(run)).unwrap()
    }
}

/// `cover --since` through the engine: of the units of a changed path, the
/// one with the same key and fingerprint at the base is dropped; the one
/// with the same key and another fingerprint, the one whose fingerprint is
/// at the base under another key, and the one absent at the base are kept.
#[test]
fn cover_since_keeps_a_unit_unless_key_and_fingerprint_are_at_the_base() {
    let mut x = since_fixture();
    // No claim, so that every kept unit is listed.
    x.store = MemStore::new();
    let out = x.cover();
    let code = &out.scopes[0];
    assert_eq!(code.name, "code");
    let items: Vec<(&str, &str)> = code
        .items
        .iter()
        .map(|i| (i.loc.as_str(), i.unit.key.as_str()))
        .collect();
    assert_eq!(
        items,
        [
            ("file:src/a.rs", "amb"),
            ("file:src/a.rs", "edited"),
            ("file:src/a.rs", "renamed"),
        ]
    );
    assert_eq!((code.units, code.covered, code.uncovered), (3, 0, 3));
    assert!(code.unavailable.is_empty());
}

/// Under `--since` a scope on `repo:` and one on an external scheme are
/// not evaluated: `skipped` says so, every count is 0, nothing is listed.
#[test]
fn cover_since_skips_repo_and_external_scopes() {
    let mut x = since_fixture();
    let out = x.cover();
    assert_eq!(out.scopes.len(), 3);
    for (i, name) in [(1, "sibling"), (2, "model")] {
        let sc = &out.scopes[i];
        assert_eq!(sc.name, name);
        assert_eq!(sc.skipped, Some(Skipped::SinceUnsupported), "{name}");
        assert_eq!(
            (sc.units, sc.covered, sc.excluded, sc.uncovered),
            (0, 0, 0, 0),
            "{name}"
        );
        assert!(sc.items.is_empty(), "{name}");
        assert!(sc.unavailable.is_empty(), "{name}");
        assert!(!sc.truncated, "{name}");
        assert_eq!(sc.exclusions.len(), 1, "{name}");
        assert_eq!(sc.exclusions[0].matched, 0, "{name}");
        assert_eq!(
            serde_json::to_value(sc).unwrap()["skipped"],
            "since_unsupported",
            "{name}"
        );
    }
    // The `file` scope is evaluated.
    assert_eq!(out.scopes[0].skipped, None);
    assert_eq!(out.scopes[0].units, 3);
    assert_eq!(out.summary.units, 3);
}

/// `status --since` never lists an expired claim, also when its side is on
/// a changed path, resolved to one unit at the base, and has another
/// fingerprint now.
#[test]
fn status_since_never_lists_an_expired_claim() {
    let mut x = since_fixture();
    let got = x.listed();
    assert!(!got.contains(&x.expired));
    // A stale claim in the same situation is listed.
    assert!(got.contains(&x.edited));
}

/// `status --since` lists a stale claim only for a side on a changed path
/// whose anchor gave exactly one unit at the base.
#[test]
fn status_since_needs_a_changed_path_and_one_unit_at_the_base() {
    let mut x = since_fixture();
    let got = x.listed();
    // Stale, and the base provider answers with another fingerprint, but
    // `src/b.rs` is not a changed path.
    assert!(!got.contains(&x.unchanged_path));
    // Stale and on a changed path, but the anchor found no unit at the
    // base.
    assert!(!got.contains(&x.renamed));
    // Stale and on a changed path, but the anchor found two units at the
    // base.
    assert!(!got.contains(&x.amb));
    assert_eq!(got, [x.edited.clone()]);
}
