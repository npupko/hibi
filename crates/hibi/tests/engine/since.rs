//! `--since` for `status` and `cover` (spec section 10.3; R19, R20, R21):
//! a real git repository for the changed paths, and two scripted providers
//! for scheme `file`, one for the working tree and one for the base commit.

use hibi::Code;
use hibi::config::{RepoConfig, UserConfig};
use hibi::engine::{
    self, ClaimState, CoverEnv, CoverQuery, Reason, Since, SinceRun, Skipped, StatusQuery,
};
use hibi::model::ClaimId;
use hibi::provider::{ProviderErrorCode as C, Providers};

use crate::support::*;

const CONFIG: &str = r#"
version = 1

[schemes.arch]

[[scope]]
name = "docs"
resource = "file:docs/**/*.md"
kinds = ["sentence"]

[[scope]]
name = "code"
resource = "file:src/**/*.rs"
kinds = ["function"]

[[scope]]
name = "model"
resource = "arch:model"
kinds = ["component"]
"#;

struct Fixture {
    dir: tempfile::TempDir,
    work: Shared,
    base: Shared,
    p: Providers,
    store: MemStore,
    git: hibi::git::Git,
    since: Since,
    /// Claims on `f`, `g`, `m` of src/a.rs, `h` of src/b.rs, `n` of
    /// docs/new.md, and one that only expired.
    f: ClaimId,
    g: ClaimId,
    m: ClaimId,
    h: ClaimId,
    n: ClaimId,
    expired: ClaimId,
}

fn fixture() -> Fixture {
    let dir = repo(&[
        ("src/a.rs", "fn f() {1}\nfn g() {1}\nfn m() {1}\n"),
        ("src/b.rs", "fn h() {1}\n"),
        ("docs/d.md", "One.\n"),
    ]);
    let base_commit = commit_all(dir.path(), "base");
    sh_git(dir.path(), &["branch", "other"]);
    // The branch: edit one function, drop another, add a section and an
    // untracked document. src/b.rs is not touched.
    write_files(
        dir.path(),
        &[
            ("src/a.rs", "fn f() {2}\nfn g() {1}\n"),
            ("docs/d.md", "One.\n\n## New\n\nNew a. New b.\n"),
            ("docs/new.md", "Brand new.\n"),
        ],
    );

    let work = file_world();
    let base = file_world();
    let mut p = Providers::new();
    p.register(&["file"], Box::new(Mem::new(&work)));
    p.declare_unregistered("arch");
    let store = MemStore::new();

    // Contents when the marks were made: g, h and n are older than the
    // base, so their claims are already stale at the base.
    set(
        &work,
        "file:src/a.rs",
        vec![
            sym_unit("f", "function", "fn f() {1}"),
            sym_unit("g", "function", "fn g() {0}"),
            sym_unit("m", "function", "fn m() {1}"),
        ],
    );
    set(
        &work,
        "file:src/b.rs",
        vec![sym_unit("h", "function", "fn h() {0}")],
    );
    set(
        &work,
        "file:docs/new.md",
        vec![sym_unit("n", "sentence", "Draft.")],
    );
    set(
        &work,
        "file:docs/d.md",
        vec![sym_unit("s1", "sentence", "One.")],
    );
    let mut claim = |statement: &str, subject: &str, recheck: Option<&str>| {
        let input = engine::ClaimInput {
            recheck: recheck.map(str::to_owned),
            ..claim_input(statement, subject, &[])
        };
        claim_at(&store, &mut p, input, "2026-01-01T00:00:00Z")
    };
    let f = claim("f returns 1", "file:src/a.rs#f", None);
    let g = claim("g returns 0", "file:src/a.rs#g", None);
    let m = claim("m returns 1", "file:src/a.rs#m", None);
    let h = claim("h returns 0", "file:src/b.rs#h", None);
    let n = claim("the draft", "file:docs/new.md#n", None);
    let expired = claim("one", "file:docs/d.md#s1", Some("P1D"));

    // The base commit.
    set(
        &base,
        "file:src/a.rs",
        vec![
            sym_unit("f", "function", "fn f() {1}"),
            sym_unit("g", "function", "fn g() {1}"),
            sym_unit("m", "function", "fn m() {1}"),
        ],
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
            sym_unit("f", "function", "fn f() {2}"),
            sym_unit("g", "function", "fn g() {1}"),
        ],
    );
    set(
        &work,
        "file:src/b.rs",
        vec![sym_unit("h", "function", "fn h() {1}")],
    );
    set(
        &work,
        "file:docs/d.md",
        vec![
            at(sym_unit("s1", "sentence", "One."), 0, 1),
            at(sym_unit("s2", "sentence", "New a."), 14, 5),
            at(sym_unit("s3", "sentence", "New b."), 21, 5),
        ],
    );
    set(
        &work,
        "file:docs/new.md",
        vec![sym_unit("n", "sentence", "Brand new.")],
    );

    // The claims of this fixture are older than the range: the base commit
    // holds their ids.
    let first_commit = base_commit;
    let base_commit = commit_claim_ids(dir.path(), &store);
    assert_ne!(base_commit, first_commit);
    sh_git(dir.path(), &["branch", "-f", "other", "HEAD"]);
    let git = real_git();
    let since = engine::resolve_since(&git, dir.path(), "HEAD").unwrap();
    assert_eq!(since.base.as_str(), base_commit);
    Fixture {
        dir,
        work,
        base,
        p,
        store,
        git,
        since,
        f,
        g,
        m,
        h,
        n,
        expired,
    }
}

impl Fixture {
    fn status(&mut self, query: &StatusQuery) -> Result<engine::StatusOutput, hibi::Error> {
        let mut base = Mem::new(&self.base);
        let run = SinceRun {
            since: &self.since,
            base: &mut base,
            git: &self.git,
            root: self.dir.path(),
        };
        engine::status(&self.store, &mut self.p, &no_git(), query, Some(run), now())
    }

    fn cover(&mut self, scopes: &[&str]) -> engine::CoverOutput {
        let config = RepoConfig::parse(CONFIG).unwrap();
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
            scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
            limit: 0,
        };
        engine::cover(&self.store, &mut self.p, &env, &query, Some(run)).unwrap()
    }
}

/// R19: `status --since` lists exactly the claims the range invalidated. A
/// claim that was already stale at the base, on another function of the
/// same file that the branch did not touch, is not listed.
#[test]
fn r19_status_since_lists_the_claims_the_range_invalidated() {
    let mut x = fixture();
    // Without --since every stale claim is listed, and the expired one.
    let all = engine::status(
        &x.store,
        &mut x.p,
        &no_git(),
        &StatusQuery::default(),
        None,
        now(),
    )
    .unwrap();
    assert_eq!(all.claims.len(), 6);

    let out = x.status(&StatusQuery::default()).unwrap();
    let mut want = vec![x.f.clone(), x.m.clone()];
    want.sort();
    let got: Vec<ClaimId> = out.claims.iter().map(|c| c.id.clone()).collect();
    // f: edited. m: resolved at the base, missing now.
    assert_eq!(got, want);
    // Not listed: g (stale before the range, same at base and now), h (its
    // file is not a changed path), n (no unit at the base), and the
    // expired claim.
    for id in [&x.g, &x.h, &x.n, &x.expired] {
        assert!(!got.contains(id));
    }
    assert!(out.claims.iter().all(|c| c.state == ClaimState::Stale));
    assert_eq!(
        serde_json::to_value(&out.since).unwrap(),
        serde_json::json!({"range": "HEAD", "base": x.since.base.as_str()})
    );
    // The summary still counts every claim.
    assert_eq!(out.summary, all.summary);
    assert_eq!(
        (out.summary.claims, out.summary.stale, out.summary.expired),
        (6, 5, 1)
    );

    // --loc filters further.
    let out = x
        .status(&StatusQuery {
            locs: vec!["file:docs/**".into()],
            ..Default::default()
        })
        .unwrap();
    assert!(out.claims.is_empty());

    // --since with --state or with ids is `usage`.
    let e = x
        .status(&StatusQuery {
            states: vec![ClaimState::Stale],
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(e.code, Code::Usage);
    let e = x
        .status(&StatusQuery {
            ids: vec![x.f.to_string()],
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(e.code, Code::Usage);
}

/// R20: `cover --since` lists exactly the units of the new section (and of
/// a new file); a unit with the same key and fingerprint at the base is
/// not kept; unchanged paths are not listed at all.
#[test]
fn r20_cover_since_keeps_only_new_or_changed_units() {
    let mut x = fixture();
    x.work.borrow_mut().log.clear();
    let out = x.cover(&[]);
    assert_eq!(
        serde_json::to_value(&out.since).unwrap(),
        serde_json::json!({"range": "HEAD", "base": x.since.base.as_str()})
    );
    let docs = &out.scopes[0];
    let items: Vec<(String, String)> = docs
        .items
        .iter()
        .map(|i| (i.loc.to_string(), i.unit.key.clone()))
        .collect();
    let want = [
        ("file:docs/d.md", "s2"),
        ("file:docs/d.md", "s3"),
        ("file:docs/new.md", "n"),
    ]
    .map(|(l, k)| (l.to_owned(), k.to_owned()));
    // The claim on docs/new.md#n covers n: it is counted, not listed.
    assert_eq!(items, want[..2]);
    assert_eq!((docs.units, docs.covered, docs.uncovered), (3, 1, 2));

    // src/a.rs: f changed (and is covered by its stale claim), g is the
    // same as at the base, m is gone.
    let code = &out.scopes[1];
    assert_eq!((code.units, code.covered, code.uncovered), (1, 1, 0));
    assert!(code.unavailable.is_empty());
    // src/b.rs is not a changed path: its units are never listed.
    assert!(!log(&x.work).contains(&"units file:src/b.rs".to_owned()));

    // External and repo: scopes are skipped and say so.
    let model = &out.scopes[2];
    assert_eq!(model.skipped, Some(Skipped::SinceUnsupported));
    assert_eq!((model.units, model.items.len()), (0, 0));
    assert!(model.unavailable.is_empty());
    assert_eq!(
        serde_json::to_value(model).unwrap()["skipped"],
        "since_unsupported"
    );
    assert_eq!(
        serde_json::to_value(out.summary).unwrap(),
        serde_json::json!({"units": 4, "covered": 2, "excluded": 0, "uncovered": 2, "excludedResources": 0})
    );
}

/// A base listing that fails with anything but `not_found` does not make
/// every unit of the resource look new: the resource is `unavailable`.
#[test]
fn cover_since_reports_a_base_resource_that_cannot_be_listed() {
    let mut x = fixture();
    set_err(&x.base, "file:docs/d.md", C::NotText);
    let out = x.cover(&["docs"]);
    let docs = &out.scopes[0];
    assert_eq!(docs.unavailable.len(), 1);
    assert_eq!(docs.unavailable[0].loc, "file:docs/d.md");
    assert_eq!(docs.unavailable[0].reason, Reason::NotText);
    // Only the new file's unit is counted.
    assert_eq!((docs.units, docs.covered, docs.uncovered), (1, 1, 0));
}

/// R21: only `A`, `A..`, `A..HEAD`, `A...`, `A...HEAD` are ranges.
#[test]
fn r21_ranges() {
    let x = fixture();
    let root = x.dir.path();
    for range in ["HEAD", "HEAD..", "HEAD..HEAD", "main...", "other...HEAD"] {
        let since = engine::resolve_since(&x.git, root, range).unwrap();
        assert_eq!(since.range, range);
        assert_eq!(since.base, x.since.base, "{range}");
    }
    for range in [
        "main..other",
        "main...other",
        "..HEAD",
        "no-such-rev",
        "HEAD:src/a.rs",
        "",
    ] {
        let e = engine::resolve_since(&x.git, root, range).unwrap_err();
        assert_eq!(e.code, Code::InvalidRange, "{range}");
    }
}
