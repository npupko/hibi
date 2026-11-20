//! Integration tests of `config` (spec sections 11.1 and 11.2; requirements
//! R29, R45 at the config layer, R48 for `hibi.toml`, R63).

use std::fs;
use std::path::{Path, PathBuf};

use hibi::Code;
use hibi::config::{
    MAX_CONFIG_BYTES, PatternKind, RepoConfig, SiblingRefusal, UserConfig, allow_sibling,
};

fn code(text: &str) -> Code {
    RepoConfig::parse(text).unwrap_err().code
}

#[test]
fn full_example_parses() {
    let cfg = RepoConfig::parse(
        r#"
version = 1

[repos]
api = "../api"

[schemes.archctl]
description = "Architecture facts served by archctl"

[schemes.bare]

[[scope]]
name = "docs"
resource = "file:docs/**/*.md"
kinds = ["sentence", "list-item", "table-row"]
exclude = [
  { resource = "file:docs/changelog.md", reason = "release notes, not claims" },
  { resource = "file:docs/api.md", unit = "fenced:0a1b2c3d4e5f", reason = "generated sample" },
]

[[scope]]
name = "arch"
resource = "archctl:system"
kinds = ["fact"]

[[scope]]
name = "api-src"
resource = "repo:api//src/**/*.rs"
kinds = ["function"]
"#,
    )
    .unwrap();
    assert_eq!(cfg.repos["api"], "../api");
    assert_eq!(
        cfg.schemes["archctl"].description.as_deref(),
        Some("Architecture facts served by archctl")
    );
    assert_eq!(cfg.schemes["bare"].description, None);
    assert!(cfg.knows_scheme("archctl") && cfg.knows_scheme("file") && !cfg.knows_scheme("x"));
    let docs = cfg.scope("docs").unwrap();
    assert_eq!(docs.kinds.len(), 3);
    assert_eq!(docs.exclude.len(), 2);
    assert_eq!(docs.exclude[1].unit.as_deref(), Some("fenced:0a1b2c3d4e5f"));
    assert!(docs.resource.matches_path("docs/guide/x.md"));
    assert!(docs.exclude[0].resource.matches_path("docs/changelog.md"));
    assert!(matches!(
        cfg.scope("arch").unwrap().resource.kind(),
        PatternKind::Locator(_)
    ));
    assert!(matches!(
        cfg.scope("api-src").unwrap().resource.kind(),
        PatternKind::Repo { alias, .. } if alias == "api"
    ));
}

#[test]
fn version_rules() {
    assert_eq!(code(""), Code::InvalidConfig);
    assert_eq!(code("version = 2"), Code::UnsupportedStoreVersion);
    assert_eq!(
        code("version = 2\nfuture_key = true"),
        Code::UnsupportedStoreVersion,
        "the version is checked before unknown keys"
    );
    assert_eq!(code("version = \"1\""), Code::InvalidConfig);
    assert_eq!(code("version = 1\nversion = 1"), Code::InvalidConfig);
    assert_eq!(code("not toml ["), Code::InvalidConfig);
}

/// R29: a `command` key in a scheme table is `invalid_config`.
#[test]
fn r29_command_key_is_rejected() {
    let e = RepoConfig::parse(
        "version = 1\n[schemes.archctl]\ncommand = [\"sh\", \"-c\", \"touch /tmp/pwned\"]\n",
    )
    .unwrap_err();
    assert_eq!(e.code, Code::InvalidConfig);
    assert!(e.message.contains("command"));
    assert_eq!(
        code("version = 1\n[schemes.archctl]\ndescription = \"x\"\nargs = []\n"),
        Code::InvalidConfig
    );
}

#[test]
fn rule_violations_are_invalid_config() {
    for text in [
        "version = 1\nunknown = 1",
        "version = 1\n[repos]\nApi = \"../api\"",
        "version = 1\n[schemes.file]",
        "version = 1\n[schemes.Bad]",
        "version = 1\n[[scope]]\nname = \"d\"\nresource = \"file:**\"\nkinds = []",
        "version = 1\n[[scope]]\nname = \"d\"\nresource = \"file:**\"",
        "version = 1\n[[scope]]\nname = \"D\"\nresource = \"file:**\"\nkinds = [\"x\"]",
        "version = 1\n[[scope]]\nname = \"d\"\nresource = \"file:**\"\nkinds = [\"x\"]\n[[scope]]\nname = \"d\"\nresource = \"file:*\"\nkinds = [\"x\"]",
        "version = 1\n[[scope]]\nname = \"d\"\nresource = \"archctl:x\"\nkinds = [\"x\"]",
        "version = 1\n[[scope]]\nname = \"d\"\nresource = \"file:[\"\nkinds = [\"x\"]",
        "version = 1\n[[scope]]\nname = \"d\"\nresource = \"file:**\"\nkinds = [\"x\"]\nexclude = [{ resource = \"file:a\" }]",
        "version = 1\n[[scope]]\nname = \"d\"\nresource = \"file:**\"\nkinds = [\"x\"]\nexclude = [{ resource = \"file:a\", reason = \" \" }]",
        "version = 1\n[[scope]]\nname = \"d\"\nresource = \"file:**\"\nkinds = [\"x\"]\nexclude = [{ resource = \"file:a\", reason = \"r\", extra = 1 }]",
        "version = 1\n[[scope]]\nname = \"d\"\nresource = \"file:**\"\nkinds = [\"x\"]\nexclude = [{ resource = \"nope:a\", reason = \"r\" }]",
    ] {
        assert_eq!(code(text), Code::InvalidConfig, "{text}");
    }
}

fn write_repo_config(root: &Path, text: &str) {
    fs::create_dir_all(root.join(".hibi")).unwrap();
    fs::write(root.join(".hibi/hibi.toml"), text).unwrap();
}

/// R63: a `hibi.toml` longer than `MAX_CONFIG_BYTES` is `invalid_config`.
#[test]
fn r63_oversized_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut text = String::from("version = 1\n");
    while (text.len() as u64) <= MAX_CONFIG_BYTES {
        text.push_str("# padding padding padding padding padding padding padding\n");
    }
    write_repo_config(dir.path(), &text);
    assert_eq!(
        RepoConfig::load(dir.path()).unwrap_err().code,
        Code::InvalidConfig
    );
    assert_eq!(
        RepoConfig::parse(&text).unwrap_err().code,
        Code::InvalidConfig
    );

    let user = dir.path().join("user.toml");
    fs::write(&user, &text).unwrap();
    assert_eq!(
        UserConfig::load(Some(&user)).unwrap_err().code,
        Code::InvalidUserConfig
    );
}

#[test]
fn repo_config_load() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        RepoConfig::load(dir.path()).unwrap_err().code,
        Code::NotInitialized
    );
    write_repo_config(dir.path(), "version = 1\n");
    assert_eq!(RepoConfig::load(dir.path()).unwrap(), RepoConfig::default());
}

/// R48 for `hibi.toml`: a symbolic link (at `.hibi` or at the file) is
/// `store_corrupt`.
#[cfg(unix)]
#[test]
fn r48_linked_config_is_refused() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.toml");
    fs::write(&real, "version = 1\n").unwrap();
    let root = dir.path().join("r");
    fs::create_dir_all(root.join(".hibi")).unwrap();
    symlink(&real, root.join(".hibi/hibi.toml")).unwrap();
    assert_eq!(
        RepoConfig::load(&root).unwrap_err().code,
        Code::StoreCorrupt
    );

    let root2 = dir.path().join("r2");
    fs::create_dir(&root2).unwrap();
    symlink(root.join(".hibi"), root2.join(".hibi")).unwrap();
    assert_eq!(
        RepoConfig::load(&root2).unwrap_err().code,
        Code::StoreCorrupt
    );
}

#[test]
fn user_config_rules() {
    let cfg = UserConfig::parse(
        r#"
[providers.archctl]
command = ["archctl", "hibi-provider"]

[providers.slow]
command = ["slow"]
timeout_ms = 600000

[[repo]]
root = "/home/u/work/shop"
alias = "api"
path = "/home/u/work/api"

[[repo]]
root = "/home/u/work/other"
alias = "api"
path = "/home/u/work/api"
"#,
    )
    .unwrap();
    assert_eq!(
        cfg.providers["archctl"].command,
        ["archctl", "hibi-provider"]
    );
    assert_eq!(cfg.providers["archctl"].timeout_ms, 10_000);
    assert_eq!(cfg.providers["slow"].timeout_ms, 600_000);
    assert_eq!(cfg.repos.len(), 2);

    let bad = |t: &str| UserConfig::parse(t).unwrap_err().code;
    for t in [
        "unknown = 1",
        "[providers.x]",
        "[providers.x]\ncommand = []",
        "[providers.x]\ncommand = [\"\"]",
        "[providers.x]\ncommand = [\"a\"]\ntimeout_ms = 99",
        "[providers.x]\ncommand = [\"a\"]\ntimeout_ms = 600001",
        "[providers.x]\ncommand = [\"a\"]\nenv = {}",
        "[providers.file]\ncommand = [\"a\"]",
        "[[repo]]\nroot = \"rel\"\nalias = \"a\"\npath = \"/p\"",
        "[[repo]]\nroot = \"/r\"\nalias = \"a\"\npath = \"~/p\"",
        "[[repo]]\nroot = \"/r\"\nalias = \"A\"\npath = \"/p\"",
        "[[repo]]\nroot = \"/r\"\nalias = \"a\"",
        "[[repo]]\nroot = \"/r\"\nalias = \"a\"\npath = \"/p\"\n[[repo]]\nroot = \"/r\"\nalias = \"a\"\npath = \"/q\"",
    ] {
        assert_eq!(bad(t), Code::InvalidUserConfig, "{t}");
    }
}

#[test]
fn user_config_load_missing_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        UserConfig::load(Some(&dir.path().join("none.toml"))).unwrap(),
        UserConfig::default()
    );
    assert_eq!(UserConfig::load(None).unwrap(), UserConfig::default());
    let p = dir.path().join("c.toml");
    fs::write(&p, "bogus = true").unwrap();
    let e = UserConfig::load(Some(&p)).unwrap_err();
    assert_eq!(e.code, Code::InvalidUserConfig);
    assert!(e.message.contains("c.toml"));
}

// ---------------------------------------------------------------------------
// The five sibling checks of section 11.1
// ---------------------------------------------------------------------------

struct Layout {
    _dir: tempfile::TempDir,
    root: PathBuf,
    api: PathBuf,
}

fn layout() -> Layout {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let root = base.join("shop");
    let api = base.join("api");
    fs::create_dir_all(root.join("inner")).unwrap();
    fs::create_dir_all(&api).unwrap();
    Layout {
        _dir: dir,
        root,
        api,
    }
}

fn repo_cfg() -> RepoConfig {
    RepoConfig::parse("version = 1\n[repos]\napi = \"../api\"\n").unwrap()
}

fn user_cfg(root: &Path, path: &Path) -> UserConfig {
    let q = |p: &Path| toml_string(&p.to_string_lossy());
    UserConfig::parse(&format!(
        "[[repo]]\nroot = {}\nalias = \"api\"\npath = {}\n",
        q(root),
        q(path)
    ))
    .unwrap()
}

fn toml_string(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

#[test]
fn sibling_checks_in_order() {
    let l = layout();
    let calls = std::cell::Cell::new(0);
    let yes = |_: &Path| {
        calls.set(calls.get() + 1);
        Ok(true)
    };

    // All five hold.
    let d = allow_sibling("api", &repo_cfg(), &user_cfg(&l.root, &l.api), &l.root, yes).unwrap();
    assert_eq!(d, l.api);
    assert_eq!(calls.get(), 1);

    // 1. Not declared: repo_not_found, git not asked.
    let r = allow_sibling(
        "web",
        &repo_cfg(),
        &user_cfg(&l.root, &l.api),
        &l.root,
        |_: &Path| -> Result<bool, hibi::Error> { panic!("check 5 must not run") },
    )
    .unwrap_err();
    assert_eq!(r.reason(), "repo_not_found");

    // 2. No user entry: repo_not_allowed with the exact lines to add.
    let r = allow_sibling(
        "api",
        &repo_cfg(),
        &UserConfig::default(),
        &l.root,
        |_: &Path| -> Result<bool, hibi::Error> { panic!("check 5 must not run") },
    )
    .unwrap_err();
    assert_eq!(r.reason(), "repo_not_allowed");
    let SiblingRefusal::RepoNotAllowed {
        remediation: Some(rem),
        ..
    } = &r
    else {
        panic!("{r:?}")
    };
    let lines = rem.split_once('\n').unwrap().1;
    let parsed = UserConfig::parse(lines).expect("the remediation lines are a valid user config");
    assert_eq!(parsed.repos[0].root, l.root);
    assert_eq!(
        parsed.repos[0].path, l.api,
        "the suggested location, made absolute"
    );
    // An entry for another root does not count.
    let other = l.root.parent().unwrap().join("api");
    let r = allow_sibling(
        "api",
        &repo_cfg(),
        &user_cfg(&other, &l.api),
        &l.root,
        |_: &Path| -> Result<bool, hibi::Error> { panic!() },
    )
    .unwrap_err();
    assert_eq!(r.reason(), "repo_not_allowed");

    // 3. Path missing: repo_not_found.
    let r = allow_sibling(
        "api",
        &repo_cfg(),
        &user_cfg(&l.root, &l.root.parent().unwrap().join("missing")),
        &l.root,
        |_: &Path| -> Result<bool, hibi::Error> { panic!() },
    )
    .unwrap_err();
    assert_eq!(r.reason(), "repo_not_found");

    // 4. The root, inside it, or above it: repo_not_allowed.
    for d in [
        l.root.clone(),
        l.root.join("inner"),
        l.root.parent().unwrap().to_path_buf(),
    ] {
        let r = allow_sibling(
            "api",
            &repo_cfg(),
            &user_cfg(&l.root, &d),
            &l.root,
            |_: &Path| -> Result<bool, hibi::Error> { panic!() },
        )
        .unwrap_err();
        assert_eq!(r.reason(), "repo_not_allowed", "{}", d.display());
    }

    // 5. Not a git top level: repo_not_allowed.
    let r = allow_sibling(
        "api",
        &repo_cfg(),
        &user_cfg(&l.root, &l.api),
        &l.root,
        |_: &Path| Ok(false),
    )
    .unwrap_err();
    assert_eq!(r.reason(), "repo_not_allowed");
}

#[cfg(unix)]
#[test]
fn sibling_paths_are_compared_canonicalized() {
    let l = layout();
    let link = l.root.parent().unwrap().join("api-link");
    std::os::unix::fs::symlink(&l.api, &link).unwrap();
    let root_link = l.root.parent().unwrap().join("shop-link");
    std::os::unix::fs::symlink(&l.root, &root_link).unwrap();
    let d = allow_sibling(
        "api",
        &repo_cfg(),
        &user_cfg(&root_link, &link),
        &l.root,
        |_: &Path| Ok(true),
    )
    .unwrap();
    assert_eq!(d, l.api);
    // A link that leads back into the root is refused by check 4.
    let inner_link = l.root.parent().unwrap().join("inner-link");
    std::os::unix::fs::symlink(l.root.join("inner"), &inner_link).unwrap();
    let r = allow_sibling(
        "api",
        &repo_cfg(),
        &user_cfg(&l.root, &inner_link),
        &l.root,
        |_: &Path| Ok(true),
    )
    .unwrap_err();
    assert_eq!(r.reason(), "repo_not_allowed");
}
