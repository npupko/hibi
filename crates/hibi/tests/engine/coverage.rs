//! Coverage (spec section 10.2) through `cover`, on a real git file list
//! with scripted providers (R38, R50).

use hibi::Code;
use hibi::config::{RepoConfig, UserConfig};
use hibi::engine::{self, CoverEnv, CoverQuery, MarkInput, Reason, RetireInput};
use hibi::provider::{ProviderErrorCode as C, Providers};

use crate::support::*;

const CONFIG: &str = r#"
version = 1

[repos]
api = "../api"

[schemes.arch]
[schemes.ghost]

[[scope]]
name = "docs"
resource = "file:docs/**/*.md"
kinds = ["sentence", "heading"]
exclude = [
  { resource = "file:docs/b.md", reason = "generated" },
  { resource = "file:docs/a.md", unit = "s2", reason = "an example" },
  { resource = "file:docs/b*.md", reason = "never reached for b.md" },
]

[[scope]]
name = "model"
resource = "arch:model"
kinds = ["component"]

[[scope]]
name = "ghosts"
resource = "ghost:all"
kinds = ["thing"]

[[scope]]
name = "sibling"
resource = "repo:api//docs/*.md"
kinds = ["sentence"]

[[scope]]
name = "config"
resource = "file:**/*.toml"
kinds = ["block"]
"#;

struct Fixture {
    dir: tempfile::TempDir,
    files: Shared,
    arch: Shared,
    p: Providers,
    store: MemStore,
    config: RepoConfig,
    user: UserConfig,
    git: hibi::git::Git,
}

fn fixture() -> Fixture {
    let dir = repo(&[
        ("docs/a.md", "a"),
        ("docs/b.md", "b"),
        ("docs/deep/c.md", "c"),
        ("docs/notes.txt", "n"),
        (".hibi/hibi.toml", CONFIG),
        ("Cargo.toml", "[package]"),
    ]);
    let files = file_world();
    set(
        &files,
        "file:docs/a.md",
        vec![
            at(sym_unit("h", "heading", "Retries"), 0, 1),
            child(
                at(
                    sym_unit("s1", "sentence", "Retries are capped at 5."),
                    10,
                    3,
                ),
                "h",
            ),
            child(
                at(sym_unit("s2", "sentence", "For example, 3."), 40, 3),
                "h",
            ),
            child(at(sym_unit("f", "fenced", "let x = 1;"), 60, 5), "h"),
            at(sym_unit("s5", "sentence", "Nobody checked this."), 90, 9),
        ],
    );
    set(
        &files,
        "file:docs/b.md",
        vec![
            at(sym_unit("s3", "sentence", "Generated one."), 0, 1),
            at(sym_unit("s4", "sentence", "Generated two."), 20, 2),
        ],
    );
    set(&files, "file:docs/deep/c.md", vec![]);
    set(
        &files,
        "file:Cargo.toml",
        vec![sym_unit("b1", "block", "[package]")],
    );
    let arch = key_world();
    set(
        &arch,
        "arch:model",
        vec![
            unit("svc", "component", "services"),
            child(unit("auth", "component", "auth issues tokens"), "svc"),
            unit("d1", "decision", "use tokens"),
        ],
    );
    let mut p = Providers::new();
    p.register(&["file", "repo"], Box::new(Mem::new(&files)));
    p.register(&["arch"], Box::new(Mem::new(&arch)));
    p.declare_unregistered("ghost");
    Fixture {
        dir,
        files,
        arch,
        p,
        store: MemStore::new(),
        config: RepoConfig::parse(CONFIG).unwrap(),
        user: UserConfig::default(),
        git: real_git(),
    }
}

impl Fixture {
    fn cover(&mut self, scopes: &[&str], limit: usize) -> Result<engine::CoverOutput, hibi::Error> {
        let env = CoverEnv {
            config: &self.config,
            user: &self.user,
            root: self.dir.path(),
            git: &self.git,
        };
        let query = CoverQuery {
            scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
            limit,
        };
        engine::cover(&self.store, &mut self.p, &env, &query, None)
    }

    fn claim(&mut self, statement: &str, subject: &str) -> hibi::model::ClaimId {
        claim_at(
            &self.store,
            &mut self.p,
            claim_input(statement, subject, &[]),
            NOW,
        )
    }
}

/// R38: a unit is covered when a live claim side resolves to it or to a
/// unit nested in it, whatever the claim's state; every exclusion is
/// reported with its reason and match count.
#[test]
fn r38_covered_through_the_parent_chain_and_exclusions_with_counts() {
    let mut f = fixture();
    // s1 covers itself and its parent heading.
    f.claim("Capped.", "file:docs/a.md#s1");
    // `docs/b.md` is excluded as a whole: it is not read, and a claim on a
    // unit of it counts nowhere in the scope.
    f.claim("One.", "file:docs/b.md#s3");
    edit(&f.files, "file:docs/b.md", "s3", "Generated one, edited.");
    // A retired claim does not.
    let gone = f.claim("Two.", "file:docs/b.md#s4");
    engine::retire(
        &f.store,
        &RetireInput {
            claims: vec![gone.to_string()],
            ..RetireInput::default()
        },
        now(),
    )
    .unwrap();

    let out = f.cover(&["docs"], 200).unwrap();
    let json = serde_json::to_value(engine::Envelope::new("cover", &out)).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "ok": true, "command": "cover", "since": null,
            "summary": {"units": 4, "covered": 2, "excluded": 1, "uncovered": 1,
                        "excludedResources": 1},
            "scopes": [{
                "name": "docs", "resource": "file:docs/**/*.md",
                "kinds": ["sentence", "heading"],
                "units": 4, "covered": 2, "excluded": 1, "uncovered": 1,
                "excludedResources": 1,
                "truncated": false, "skipped": null,
                "items": [{
                    "loc": "file:docs/a.md",
                    "unit": {"key": "s5", "kind": "sentence", "label": "Nobody checked this.",
                             "line": 9, "endLine": 9},
                    "text": "Nobody checked this.",
                    "anchor": {"loc": "file:docs/a.md", "sel": {"symbol": "s5"}}
                }],
                "exclusions": [
                    // One resource, not one unit.
                    {"resource": "file:docs/b.md", "unit": null, "reason": "generated",
                     "matched": 1},
                    {"resource": "file:docs/a.md", "unit": "s2", "reason": "an example",
                     "matched": 1},
                    {"resource": "file:docs/b*.md", "unit": null,
                     "reason": "never reached for b.md", "matched": 0}
                ],
                "unavailable": []
            }],
            "warnings": []
        })
    );

    // The item's anchor can be claimed; the unit is then covered.
    let anchor = out.scopes[0].items[0].anchor.clone();
    let input = engine::ClaimInput {
        subject: anchor.into(),
        ..claim_input("Checked.", "file:docs/a.md", &[])
    };
    claim_at(&f.store, &mut f.p, input, NOW);
    let out = f.cover(&["docs"], 200).unwrap();
    assert_eq!(out.summary.uncovered, 0);
    assert_eq!(out.summary.covered, 3);
    // The excluded resource was never asked for its units.
    assert!(
        !log(&f.files).iter().any(|l| l == "units file:docs/b.md"),
        "{:?}",
        log(&f.files)
    );
}

#[test]
fn items_are_ordered_capped_and_cut() {
    let mut f = fixture();
    let long = "é".repeat(300);
    set(
        &f.files,
        "file:docs/deep/c.md",
        vec![
            at(sym_unit("z", "sentence", "second by offset"), 50, 2),
            at(sym_unit("b", "sentence", &long), 10, 1),
            at(sym_unit("a", "sentence", "same offset, smaller key"), 10, 1),
        ],
    );
    let out = f.cover(&["docs"], 0).unwrap();
    let keys: Vec<(String, String)> = out.scopes[0]
        .items
        .iter()
        .map(|i| (i.loc.to_string(), i.unit.key.clone()))
        .collect();
    let want = [
        ("file:docs/a.md", "h"),
        ("file:docs/a.md", "s1"),
        ("file:docs/a.md", "s5"),
        ("file:docs/deep/c.md", "a"),
        ("file:docs/deep/c.md", "b"),
        ("file:docs/deep/c.md", "z"),
    ]
    .map(|(l, k)| (l.to_owned(), k.to_owned()));
    assert_eq!(keys, want);
    assert!(!out.scopes[0].truncated);
    assert_eq!(
        out.scopes[0].items[4].text.as_deref(),
        Some("é".repeat(240).as_str())
    );

    // --limit caps items, never counts; text is fetched only for the
    // listed items.
    f.files.borrow_mut().log.clear();
    let out = f.cover(&["docs"], 2).unwrap();
    let s = &out.scopes[0];
    assert_eq!((s.items.len(), s.uncovered, s.truncated), (2, 6, true));
    assert_eq!(out.summary.uncovered, 6);
    let calls = |f: &Fixture, method: &str| {
        log(&f.files)
            .iter()
            .filter(|l| l.starts_with(method))
            .count()
    };
    // One `render` per listed item, and no `resolve`.
    assert_eq!((calls(&f, "render"), calls(&f, "resolve")), (2, 0));

    // A provider without `render` is asked with the item's selector.
    f.files.borrow_mut().render = false;
    f.files.borrow_mut().log.clear();
    let out = f.cover(&["docs"], 2).unwrap();
    assert!(out.scopes[0].items.iter().all(|i| i.text.is_some()));
    assert_eq!((calls(&f, "render"), calls(&f, "resolve")), (0, 2));
}

/// R50 and section 10.2: the parent walk stops at a `null` parent, at a
/// key that is not in the unit list, and at a key already visited.
#[test]
fn r50_the_parent_walk_terminates() {
    let mut f = fixture();
    set(
        &f.arch,
        "arch:model",
        vec![
            child(unit("a", "component", "a"), "b"),
            child(unit("b", "component", "b"), "c"),
            child(unit("c", "component", "c"), "a"),
            child(unit("d", "component", "d"), "nowhere"),
            child(unit("e", "component", "e"), "d"),
            unit("lone", "component", "lone"),
        ],
    );
    let on = |key: &str| engine::ClaimInput {
        subject: json_anchor("arch:model", key_sel(key)),
        ..claim_input(&format!("on {key}"), "arch:model", &[])
    };
    claim_at(&f.store, &mut f.p, on("a"), NOW);
    claim_at(&f.store, &mut f.p, on("e"), NOW);
    let out = f.cover(&["model"], 0).unwrap();
    let s = &out.scopes[0];
    // a, b, c through the cycle; e and d, where the walk stops at the
    // unknown key.
    assert_eq!((s.units, s.covered, s.uncovered), (6, 5, 1));
    assert_eq!(s.items[0].unit.key, "lone");
    assert_eq!(s.items[0].unit.line, None);

    let units = vec![
        child(unit("x", "k", "x"), "y"),
        child(unit("y", "k", "y"), "x"),
        unit("z", "k", "z"),
    ];
    let covered = engine::covered_keys(&units, ["x", "not-a-unit"]);
    assert_eq!(covered.into_iter().collect::<Vec<_>>(), ["x", "y"]);
}

/// Section 10.2: a resource whose units cannot be listed is reported under
/// `unavailable` with the reason of section 9.2, never dropped and never
/// counted.
#[test]
fn resources_that_cannot_be_listed_are_unavailable() {
    let mut f = fixture();
    set_err(&f.files, "file:docs/a.md", C::NotText);
    // `docs/b.md` cannot be listed either, and the scope excludes it as a
    // whole: it is not read, so it is not reported.
    f.files.borrow_mut().resources.remove("file:docs/b.md");
    // Two units with one key.
    set(
        &f.files,
        "file:docs/deep/c.md",
        vec![
            sym_unit("s", "sentence", "one"),
            sym_unit("s", "sentence", "two"),
        ],
    );
    let out = f.cover(&["docs"], 0).unwrap();
    let s = &out.scopes[0];
    assert_eq!((s.units, s.covered, s.excluded, s.uncovered), (0, 0, 0, 0));
    assert_eq!(
        serde_json::to_value(&s.unavailable).unwrap(),
        serde_json::json!([
            {"loc": "file:docs/a.md", "reason": "not_text"},
            {"loc": "file:docs/deep/c.md", "reason": "provider_error"}
        ])
    );
    assert_eq!(s.exclusions[0].resource, "file:docs/b.md");
    assert_eq!(s.exclusions[0].matched, 1);
    // In the file list but not readable by section 6.8 (a symbolic link).
    f.files.borrow_mut().resources.remove("file:docs/deep/c.md");
    let out = f.cover(&["docs"], 0).unwrap();
    assert_eq!(
        serde_json::to_value(&out.scopes[0].unavailable[1]).unwrap(),
        serde_json::json!({"loc": "file:docs/deep/c.md", "reason": "resource_not_found"})
    );

    // An external scheme with no registered provider, a failing provider,
    // and a sibling alias the user config does not allow.
    f.arch.borrow_mut().info_error = Some(perr(C::Failed));
    let out = f.cover(&["model", "ghosts", "sibling"], 0).unwrap();
    let reasons: Vec<(String, String, Reason)> = out
        .scopes
        .iter()
        .map(|s| {
            assert_eq!(s.unavailable.len(), 1, "{}", s.name);
            assert_eq!(s.units, 0);
            (
                s.name.clone(),
                s.unavailable[0].loc.clone(),
                s.unavailable[0].reason,
            )
        })
        .collect();
    let want = [
        ("model", "arch:model", Reason::ProviderFailed),
        ("ghosts", "ghost:all", Reason::ProviderNotRegistered),
        ("sibling", "repo:api//docs/*.md", Reason::RepoNotAllowed),
    ]
    .map(|(n, l, r)| (n.to_owned(), l.to_owned(), r));
    assert_eq!(reasons, want);
}

/// A kind of a scope on an external scheme that the provider's `hello`
/// does not list is reported as `unknown_kind`. A provider that states no
/// kinds, one that does not start and one that is not registered give no
/// warning.
#[test]
fn a_scope_kind_the_external_provider_does_not_list_is_a_warning() {
    let mut f = fixture();
    let warnings = |f: &mut Fixture| {
        let out = f.cover(&["model", "ghosts", "docs"], 0).unwrap();
        serde_json::to_value(&out.warnings).unwrap()
    };
    // `kinds` is informational and may be empty: unknown, no warning.
    assert_eq!(warnings(&mut f), serde_json::json!([]));
    // Every kind of the scope is listed.
    f.arch.borrow_mut().kinds = vec!["component".to_owned(), "decision".to_owned()];
    assert_eq!(warnings(&mut f), serde_json::json!([]));
    // The scope asks for `component`; the provider lists other kinds.
    f.arch.borrow_mut().kinds = vec!["decision".to_owned(), "service".to_owned()];
    assert_eq!(
        warnings(&mut f),
        serde_json::json!([{
            "code": "unknown_kind",
            "message": "scope model: the arch provider lists no unit kind \"component\", so it selects nothing; its kinds are decision, service"
        }])
    );
    // The units are still listed and counted as before.
    let out = f.cover(&["model"], 0).unwrap();
    assert_eq!(out.scopes[0].units, 2);
    // A provider that does not start gives no kinds to compare with.
    f.arch.borrow_mut().info_error = Some(perr(C::Failed));
    assert_eq!(warnings(&mut f), serde_json::json!([]));
}

#[test]
fn scopes_are_selected_by_name_in_config_order_and_hibi_is_never_listed() {
    let mut f = fixture();
    let out = f.cover(&["config", "model"], 0).unwrap();
    let names: Vec<&str> = out.scopes.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["model", "config"]);
    // `.hibi/hibi.toml` matches `**/*.toml` and is not a resource.
    let config = &out.scopes[1];
    assert_eq!((config.units, config.uncovered), (1, 1));
    assert_eq!(config.items[0].loc, "file:Cargo.toml");
    assert!(config.unavailable.is_empty());
    // Kinds select: `decision` is not in the model scope.
    assert_eq!(out.scopes[0].units, 2);
    assert_eq!(out.summary.units, 3);

    assert_eq!(f.cover(&[], 0).unwrap().scopes.len(), 5);
    assert_eq!(f.cover(&["nope"], 0).unwrap_err().code, Code::UnknownScope);
    f.config = RepoConfig::parse("version = 1\n").unwrap();
    assert_eq!(f.cover(&[], 0).unwrap_err().code, Code::NoScopes);
}

/// Section 10.1: paths under `.hibi/` are never in the file list, in a
/// sibling repository as in the root.
#[test]
fn hibi_paths_of_a_sibling_repository_are_never_listed() {
    let mut f = fixture();
    let sibling = repo(&[
        ("docs/x.md", "x"),
        (".hibi/hibi.toml", "version = 1\n"),
        (".hibi/notes.md", "n"),
        ("sub/.hibi/kept.md", "k"),
    ]);
    f.config = RepoConfig::parse(
        "version = 1\n[repos]\napi = \"../api\"\n[[scope]]\nname = \"sibling\"\nresource = \"repo:api//**/*.md\"\nkinds = [\"sentence\"]\n",
    )
    .unwrap();
    f.user = UserConfig {
        repos: vec![hibi::config::RepoEntry {
            root: f.dir.path().to_path_buf(),
            alias: "api".into(),
            path: sibling.path().to_path_buf(),
        }],
        ..UserConfig::default()
    };
    for loc in [
        "repo:api//docs/x.md",
        "repo:api//.hibi/notes.md",
        "repo:api//sub/.hibi/kept.md",
    ] {
        set(
            &f.files,
            loc,
            vec![sym_unit("s", "sentence", "A sentence.")],
        );
    }
    let out = f.cover(&["sibling"], 0).unwrap();
    let scope = &out.scopes[0];
    assert!(scope.unavailable.is_empty(), "{:?}", scope.unavailable);
    let listed: Vec<&str> = scope.items.iter().map(|i| i.loc.as_str()).collect();
    // Only a top-level `.hibi/` is the store; `sub/.hibi/` is a plain
    // directory.
    assert_eq!(
        listed,
        ["repo:api//docs/x.md", "repo:api//sub/.hibi/kept.md"]
    );
    assert_eq!((scope.units, scope.uncovered), (2, 2));
}

/// The work of `cover` on a resource of many identical units is bounded by
/// the number of listed items: the text of an item is read with one
/// `render` of its key, never with a `resolve` of its selector, which
/// matches every one of the identical units. A resolution keeps at most
/// `MAX_CANDIDATES` of the units a selector matches.
#[test]
fn cover_reads_item_text_by_key_and_keeps_few_of_many_identical_units() {
    const MANY: usize = 500;
    let mut f = fixture();
    let same = hibi::model::Selector::quote("function f(){}");
    let twins: Vec<hibi::provider::Unit> = (0..MANY)
        .map(|n| {
            let mut u = at(
                unit(&format!("f{n:04}"), "sentence", "function f(){}"),
                n * 20,
                n + 1,
            );
            u.sel = same.clone();
            u
        })
        .collect();
    set(&f.files, "file:docs/deep/c.md", twins);
    f.files.borrow_mut().log.clear();

    let out = f.cover(&["docs"], 3).unwrap();
    let scope = &out.scopes[0];
    assert!(scope.uncovered >= MANY, "{}", scope.uncovered);
    assert_eq!(scope.items.len(), 3);
    for item in &scope.items {
        assert!(item.text.is_some(), "{item:?}");
    }
    let log = log(&f.files);
    let renders = log.iter().filter(|l| l.starts_with("render ")).count();
    assert_eq!(renders, 3, "{log:?}");
    // No selector of a listed unit was resolved.
    assert!(
        !log.iter()
            .any(|l| l.starts_with("resolve ") && l.contains("function f")),
        "{log:?}"
    );

    // An anchor that matches all of them: the count is exact, and twenty
    // units are kept.
    let all = anchor("file:docs/deep/c.md", same);
    match &engine::resolve_anchors(&mut f.p, &[&all])[0] {
        engine::Resolved::Several(c) => {
            assert_eq!(c.total, MANY);
            assert_eq!(c.units.len(), engine::MAX_CANDIDATES);
            assert_eq!(c.anchors.len(), engine::MAX_CANDIDATES);
        }
        other => panic!("{other:?}"),
    }
}

/// For the judge: `coverage` gives every uncovered unit, and `unit_texts`
/// their full text; `render_sides` gives the text of a claim's sides.
#[test]
fn the_judge_can_read_uncovered_units_and_rendered_sides() {
    let mut f = fixture();
    let long = "x".repeat(500);
    set(
        &f.files,
        "file:docs/deep/c.md",
        vec![sym_unit("long", "sentence", &long)],
    );
    let env = CoverEnv {
        config: &f.config,
        user: &f.user,
        root: f.dir.path(),
        git: &f.git,
    };
    let cov = engine::coverage(&f.store, &mut f.p, &env, &["docs".to_owned()], None).unwrap();
    let units: Vec<&engine::UncoveredUnit> = cov.scopes[0].uncovered.iter().collect();
    assert_eq!(units.len(), 4);
    assert_eq!(units[0].unit.kind, "heading");
    let texts = engine::unit_texts(&mut f.p, &units);
    assert_eq!(texts[3].as_deref(), Some(long.as_str()));

    let id = f.claim("Capped.", "file:docs/a.md#s1");
    edit(&f.files, "file:docs/a.md", "s1", "Retries are capped at 9.");
    let v = view(&f.store, &mut f.p, &id);
    assert_eq!(
        engine::render_sides(&mut f.p, &v),
        Some(vec!["Retries are capped at 9.".to_owned()])
    );
    f.files.borrow_mut().render = false;
    assert_eq!(engine::render_sides(&mut f.p, &v), None);
    f.files.borrow_mut().render = true;
    f.files.borrow_mut().resources.remove("file:docs/a.md");
    let v = view(&f.store, &mut f.p, &id);
    assert_eq!(engine::render_sides(&mut f.p, &v), None);
    // `mark` is not involved: nothing was written by the reads above.
    let before = f.store.counts();
    let _ = engine::mark(
        &f.store,
        &mut f.p,
        &no_git(),
        &[MarkInput::new(id.as_str())],
        false,
        now(),
    )
    .unwrap_err();
    assert_eq!(f.store.counts(), before);
}
