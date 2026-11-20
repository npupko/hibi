//! `.hibi/hibi.toml` and the user config (spec sections 11.1 and 11.2), and
//! the sibling-repository checks of section 11.1.
//!
//! [`RepoConfig`] is the committed repository configuration: scopes,
//! declared external schemes and suggested sibling aliases. It never names a
//! command. [`UserConfig`] is the per-user file that registers provider
//! commands per scheme and allows sibling directories per repository.
//! [`allow_sibling`] runs the five ordered checks of section 11.1 before
//! anything is read from a sibling repository.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use schemars::{JsonSchema, Schema};
use serde::Deserialize;

use crate::build_info::STORE_VERSION;
use crate::error::{Code, Error};
use crate::model::Locator;

/// The largest `hibi.toml` or user config, in bytes (section 11.1).
pub const MAX_CONFIG_BYTES: u64 = 1_048_576;

/// Default provider timeout in milliseconds (section 11.2).
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;
/// Smallest allowed `timeout_ms`.
pub const MIN_TIMEOUT_MS: u64 = 100;
/// Largest allowed `timeout_ms`.
pub const MAX_TIMEOUT_MS: u64 = 600_000;

/// `.hibi/hibi.toml` relative to the root.
pub const REPO_CONFIG_PATH: &str = ".hibi/hibi.toml";

// ---------------------------------------------------------------------------
// Opening files without following a symbolic link
// ---------------------------------------------------------------------------

/// Opening store files without following a symbolic link in the last path
/// component (`O_NOFOLLOW` on Unix, sections 12.4 and 15.3 `init`).
///
/// `O_NOFOLLOW` guards only the last component; callers check `.hibi` and
/// the shard directories with `symlink_metadata` first. On Windows no flag is
/// set and the callers' `symlink_metadata` check is the only guard (not
/// tested).
pub(crate) mod nofollow {
    use std::fs::OpenOptions;
    use std::io;

    /// Add the no-follow flag to `opts`.
    pub(crate) fn apply(opts: &mut OpenOptions) -> &mut OpenOptions {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.custom_flags(libc::O_NOFOLLOW);
        }
        opts
    }

    /// True when `err` is what opening a symbolic link with the no-follow
    /// flag returns (`ELOOP`; some BSDs return `EMLINK`).
    pub(crate) fn is_link_error(err: &io::Error) -> bool {
        #[cfg(unix)]
        {
            matches!(err.raw_os_error(), Some(libc::ELOOP) | Some(libc::EMLINK))
        }
        #[cfg(not(unix))]
        {
            let _ = err;
            false
        }
    }
}

/// Read at most `cap` bytes from `file`; `None` when it holds more.
fn read_capped(mut file: File, cap: u64) -> std::io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    (&mut file).take(cap + 1).read_to_end(&mut buf)?;
    Ok(if buf.len() as u64 > cap {
        None
    } else {
        Some(buf)
    })
}

fn invalid_config(message: impl Into<String>) -> Error {
    Error::new(Code::InvalidConfig, message)
}

fn invalid_user_config(message: impl Into<String>) -> Error {
    Error::new(Code::InvalidUserConfig, message)
}

/// [`is_name`] as an ECMA 262 regular expression, for the JSON Schemas:
/// scope names and repository aliases.
pub const NAME_PATTERN: &str = "^[a-z0-9][a-z0-9-]*$";

/// [`is_scheme`] as an ECMA 262 regular expression, for the JSON Schemas.
/// That `file` and `repo` cannot be declared is a separate schema rule.
pub const SCHEME_PATTERN: &str = "^[a-z][a-z0-9-]*$";

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z' | '0'..='9'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

fn is_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

// ---------------------------------------------------------------------------
// Resource patterns (section 10.1)
// ---------------------------------------------------------------------------

/// A scope or exclusion `resource` (section 10.1).
#[derive(Debug, Clone)]
pub struct ResourcePattern {
    text: String,
    kind: PatternKind,
    matcher: Option<GlobMatcher>,
}

/// What a [`ResourcePattern`] matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternKind {
    /// `file:<glob>`: matched against the root's file list.
    File { glob: String },
    /// `repo:<alias>//<glob>`: matched against the sibling's file list.
    Repo { alias: String, glob: String },
    /// Any other scheme: one exact locator.
    Locator(Locator),
}

impl PartialEq for ResourcePattern {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}
impl Eq for ResourcePattern {}

impl ResourcePattern {
    /// Parse a pattern. `file:` and `repo:` take a glob path (`*` within a
    /// segment, `**` across segments, `?`, `[...]`, `{a,b}`); any other
    /// scheme takes one locator. Errors are messages for `invalid_config`.
    pub fn parse(text: &str) -> Result<ResourcePattern, String> {
        let Some((scheme, rest)) = text.split_once(':') else {
            return Err(format!("{text:?} has no scheme"));
        };
        let glob = |g: &str| -> Result<GlobMatcher, String> {
            if g.is_empty() {
                return Err(format!("{text:?} has an empty path pattern"));
            }
            if g.chars()
                .any(|c| matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'))
            {
                return Err(format!("{text:?} contains a control character"));
            }
            GlobBuilder::new(g)
                .literal_separator(true)
                .backslash_escape(true)
                .build()
                .map(|g| g.compile_matcher())
                .map_err(|e| format!("{text:?} is not a valid glob: {e}"))
        };
        let (kind, matcher) = match scheme {
            "file" => (
                PatternKind::File {
                    glob: rest.to_owned(),
                },
                Some(glob(rest)?),
            ),
            "repo" => {
                let Some((alias, g)) = rest.split_once("//") else {
                    return Err(format!("{text:?} is not repo:<alias>//<pattern>"));
                };
                if !is_name(alias) {
                    return Err(format!("{text:?}: the alias must match [a-z0-9][a-z0-9-]*"));
                }
                (
                    PatternKind::Repo {
                        alias: alias.to_owned(),
                        glob: g.to_owned(),
                    },
                    Some(glob(g)?),
                )
            }
            _ => {
                let loc = Locator::new(text).map_err(|e| e.message)?;
                (PatternKind::Locator(loc), None)
            }
        };
        Ok(ResourcePattern {
            text: text.to_owned(),
            kind,
            matcher,
        })
    }

    /// The pattern as written.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The scheme: `file`, `repo` or an external one.
    pub fn scheme(&self) -> &str {
        self.text.split_once(':').map_or("", |(s, _)| s)
    }

    pub fn kind(&self) -> &PatternKind {
        &self.kind
    }

    /// For `file:` and `repo:` patterns: true when the glob matches `path`
    /// (a path of the file list). Always false for an exact locator.
    pub fn matches_path(&self, path: &str) -> bool {
        self.matcher.as_ref().is_some_and(|m| m.is_match(path))
    }

    /// True when the pattern matches the locator: the glob matches its path
    /// (same scheme, same alias), or it is the same locator.
    pub fn matches_locator(&self, loc: &Locator) -> bool {
        use crate::model::LocatorParts;
        match (&self.kind, loc.parts()) {
            (PatternKind::File { .. }, LocatorParts::File { path }) => self.matches_path(path),
            (PatternKind::Repo { alias, .. }, LocatorParts::Repo { alias: a, path }) => {
                alias == a && self.matches_path(path)
            }
            (PatternKind::Locator(l), _) => l == loc,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// .hibi/hibi.toml (section 11.1)
// ---------------------------------------------------------------------------

/// The repository configuration `.hibi/hibi.toml` (section 11.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoConfig {
    /// `[repos]`: alias to suggested directory. The value is only displayed.
    pub repos: BTreeMap<String, String>,
    /// `[schemes.<name>]`: declared external schemes.
    pub schemes: BTreeMap<String, SchemeDecl>,
    /// `[[scope]]` in file order.
    pub scopes: Vec<Scope>,
}

/// One `[schemes.<name>]` table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemeDecl {
    pub description: Option<String>,
}

/// One `[[scope]]` (section 10.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub name: String,
    pub resource: ResourcePattern,
    /// Non-empty. Not validated against any provider's kind list.
    pub kinds: Vec<String>,
    pub exclude: Vec<Exclusion>,
}

/// One exclusion of a scope (section 10.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exclusion {
    pub resource: ResourcePattern,
    /// An exact unit key, or every unit of matching resources when `None`.
    pub unit: Option<String>,
    pub reason: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawRepoConfig {
    /// The store format version. Must be 1.
    #[allow(dead_code)]
    #[schemars(extend("const" = STORE_VERSION))]
    version: i64,
    /// Alias to a suggested location, which is only ever displayed.
    #[serde(default)]
    #[schemars(extend(
        "propertyNames" = {"pattern": NAME_PATTERN},
        "additionalProperties" = {"type": "string", "minLength": 1}
    ))]
    repos: BTreeMap<String, String>,
    /// One table per external scheme. `file` and `repo` are built in.
    #[serde(default)]
    #[schemars(extend(
        "propertyNames" = {"pattern": SCHEME_PATTERN, "not": {"enum": ["file", "repo"]}}
    ))]
    schemes: BTreeMap<String, RawScheme>,
    #[serde(default)]
    scope: Vec<RawScope>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawScheme {
    description: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawScope {
    /// Unique among the scopes.
    #[schemars(extend("pattern" = NAME_PATTERN))]
    name: String,
    resource: String,
    #[schemars(length(min = 1), inner(length(min = 1)))]
    kinds: Vec<String>,
    #[serde(default)]
    exclude: Vec<RawExclusion>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawExclusion {
    resource: String,
    /// An exact unit key.
    #[schemars(length(min = 1))]
    unit: Option<String>,
    /// Shown in the output of `cover`. Not only whitespace.
    #[schemars(length(min = 1))]
    reason: String,
}

/// The JSON Schema of `.hibi/hibi.toml` read as a document (section 11.1),
/// generated from the shape the parser reads. Rules the parser checks after
/// reading (name patterns, `version = 1`, known schemes) are not in it.
pub fn repo_config_schema() -> Schema {
    schemars::schema_for!(RawRepoConfig)
}

/// The JSON Schema of the user config read as a document (section 11.2),
/// generated from the shape the parser reads.
pub fn user_config_schema() -> Schema {
    schemars::schema_for!(RawUserConfig)
}

impl RepoConfig {
    /// Parse the text of `hibi.toml`. Errors: `invalid_config` (does not
    /// parse, unknown key, a rule of section 11.1 broken, a `command` key in
    /// a scheme table), `unsupported_store_version` (`version` is an integer
    /// other than 1).
    pub fn parse(text: &str) -> Result<RepoConfig, Error> {
        if text.len() as u64 > MAX_CONFIG_BYTES {
            return Err(invalid_config(format!(
                "{REPO_CONFIG_PATH} is larger than {MAX_CONFIG_BYTES} bytes"
            )));
        }
        let table: toml::Table = toml::from_str(text)
            .map_err(|e| invalid_config(format!("{REPO_CONFIG_PATH} does not parse: {e}")))?;
        match table.get("version") {
            None => {
                return Err(invalid_config(format!(
                    "{REPO_CONFIG_PATH} has no version; write version = 1"
                )));
            }
            Some(toml::Value::Integer(1)) => {}
            Some(toml::Value::Integer(n)) => {
                return Err(Error::new(
                    Code::UnsupportedStoreVersion,
                    format!("{REPO_CONFIG_PATH} has version {n}; this hibi reads version 1"),
                ));
            }
            Some(v) => {
                return Err(invalid_config(format!(
                    "{REPO_CONFIG_PATH}: version must be the integer 1, not {v}"
                )));
            }
        }
        if let Some(toml::Value::Table(schemes)) = table.get("schemes") {
            for (name, t) in schemes {
                if let toml::Value::Table(t) = t
                    && t.contains_key("command")
                {
                    return Err(invalid_config(format!(
                        "{REPO_CONFIG_PATH}: [schemes.{name}] has a command key; a repository never names a command. Register the provider command in your user config under [providers.{name}]"
                    )));
                }
            }
        }
        let raw: RawRepoConfig =
            toml::from_str(text).map_err(|e| invalid_config(format!("{REPO_CONFIG_PATH}: {e}")))?;

        for (alias, suggested) in &raw.repos {
            if !is_name(alias) {
                return Err(invalid_config(format!(
                    "{REPO_CONFIG_PATH}: [repos] alias {alias:?} must match [a-z0-9][a-z0-9-]*"
                )));
            }
            if suggested.is_empty() {
                return Err(invalid_config(format!(
                    "{REPO_CONFIG_PATH}: [repos] {alias} has an empty suggested location"
                )));
            }
        }
        let mut schemes = BTreeMap::new();
        for (name, s) in raw.schemes {
            if !is_scheme(&name) {
                return Err(invalid_config(format!(
                    "{REPO_CONFIG_PATH}: scheme {name:?} must match [a-z][a-z0-9-]*"
                )));
            }
            if name == "file" || name == "repo" {
                return Err(invalid_config(format!(
                    "{REPO_CONFIG_PATH}: scheme {name} is built in and cannot be declared under [schemes]"
                )));
            }
            schemes.insert(
                name,
                SchemeDecl {
                    description: s.description,
                },
            );
        }
        let pattern = |text: &str, what: &str| -> Result<ResourcePattern, Error> {
            let p = ResourcePattern::parse(text)
                .map_err(|m| invalid_config(format!("{REPO_CONFIG_PATH}: {what}: {m}")))?;
            let scheme = p.scheme();
            if scheme != "file" && scheme != "repo" && !schemes.contains_key(scheme) {
                return Err(invalid_config(format!(
                    "{REPO_CONFIG_PATH}: {what}: scheme {scheme} is neither file, repo nor declared under [schemes]"
                )));
            }
            Ok(p)
        };
        let mut names = BTreeSet::new();
        let mut scopes = Vec::with_capacity(raw.scope.len());
        for s in raw.scope {
            if !is_name(&s.name) {
                return Err(invalid_config(format!(
                    "{REPO_CONFIG_PATH}: scope name {:?} must match [a-z0-9][a-z0-9-]*",
                    s.name
                )));
            }
            if !names.insert(s.name.clone()) {
                return Err(invalid_config(format!(
                    "{REPO_CONFIG_PATH}: scope name {} is used twice",
                    s.name
                )));
            }
            let what = format!("scope {}", s.name);
            let resource = pattern(&s.resource, &what)?;
            if s.kinds.is_empty() || s.kinds.iter().any(String::is_empty) {
                return Err(invalid_config(format!(
                    "{REPO_CONFIG_PATH}: {what}: kinds must be a non-empty list of non-empty names"
                )));
            }
            let mut exclude = Vec::with_capacity(s.exclude.len());
            for x in s.exclude {
                let resource = pattern(&x.resource, &format!("{what} exclusion"))?;
                if x.unit.as_deref() == Some("") {
                    return Err(invalid_config(format!(
                        "{REPO_CONFIG_PATH}: {what}: an exclusion unit must not be empty"
                    )));
                }
                if x.reason.trim().is_empty() {
                    return Err(invalid_config(format!(
                        "{REPO_CONFIG_PATH}: {what}: every exclusion needs a reason"
                    )));
                }
                exclude.push(Exclusion {
                    resource,
                    unit: x.unit,
                    reason: x.reason,
                });
            }
            scopes.push(Scope {
                name: s.name,
                resource,
                kinds: s.kinds,
                exclude,
            });
        }
        Ok(RepoConfig {
            repos: raw.repos,
            schemes,
            scopes,
        })
    }

    /// Read and parse `<root>/.hibi/hibi.toml`.
    ///
    /// `.hibi` or `hibi.toml` being a symbolic link is `store_corrupt`
    /// (section 12.3); the file is opened without following a link. A
    /// missing file is `not_initialized`. A `hibi.toml` that exists and is
    /// not a regular file (a directory, FIFO, socket or device) is
    /// `store_corrupt` (section 12.3) and is not opened. A file larger than
    /// [`MAX_CONFIG_BYTES`] is `invalid_config`.
    pub fn load(root: &Path) -> Result<RepoConfig, Error> {
        let hibi = root.join(".hibi");
        let path = root.join(REPO_CONFIG_PATH);
        let not_regular = || {
            Error::new(
                Code::StoreCorrupt,
                format!("{REPO_CONFIG_PATH} is not a regular file"),
            )
        };
        for (p, rel) in [(&hibi, ".hibi"), (&path, REPO_CONFIG_PATH)] {
            match std::fs::symlink_metadata(p) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(Error::new(
                        Code::StoreCorrupt,
                        format!("{rel} is a symbolic link"),
                    ));
                }
                // Checked before the file is opened: opening a FIFO for
                // reading waits for a writer.
                Ok(m) if rel == REPO_CONFIG_PATH && !m.is_file() => {
                    return Err(not_regular());
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(Error::new(
                        Code::NotInitialized,
                        format!("{} has no {REPO_CONFIG_PATH}", root.display()),
                    ));
                }
                Err(e) => return Err(Error::io(rel, &e)),
            }
        }
        let file = nofollow::apply(OpenOptions::new().read(true))
            .open(&path)
            .map_err(|e| {
                if nofollow::is_link_error(&e) {
                    Error::new(
                        Code::StoreCorrupt,
                        format!("{REPO_CONFIG_PATH} is a symbolic link"),
                    )
                } else {
                    Error::io(REPO_CONFIG_PATH, &e)
                }
            })?;
        let meta = file
            .metadata()
            .map_err(|e| Error::io(REPO_CONFIG_PATH, &e))?;
        // The file was replaced between the check above and the open.
        if !meta.is_file() {
            return Err(not_regular());
        }
        let bytes = read_capped(file, MAX_CONFIG_BYTES)
            .map_err(|e| Error::io(REPO_CONFIG_PATH, &e))?
            .ok_or_else(|| {
                invalid_config(format!(
                    "{REPO_CONFIG_PATH} is larger than {MAX_CONFIG_BYTES} bytes"
                ))
            })?;
        let text = String::from_utf8(bytes)
            .map_err(|_| invalid_config(format!("{REPO_CONFIG_PATH} is not UTF-8")))?;
        RepoConfig::parse(&text)
    }

    /// True when the scheme may be used in a locator: `file`, `repo`, or
    /// declared under `[schemes]`.
    pub fn knows_scheme(&self, scheme: &str) -> bool {
        scheme == "file" || scheme == "repo" || self.schemes.contains_key(scheme)
    }

    /// The scope with this name.
    pub fn scope(&self, name: &str) -> Option<&Scope> {
        self.scopes.iter().find(|s| s.name == name)
    }
}

// ---------------------------------------------------------------------------
// User config (section 11.2)
// ---------------------------------------------------------------------------

/// The user config (section 11.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserConfig {
    /// `[providers.<scheme>]`.
    pub providers: BTreeMap<String, ProviderCommand>,
    /// `[[repo]]` entries in file order.
    pub repos: Vec<RepoEntry>,
}

/// One `[providers.<scheme>]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCommand {
    /// argv, at least one element.
    pub command: Vec<String>,
    /// 100 to 600000, default 10000.
    pub timeout_ms: u64,
}

/// One `[[repo]]` entry: the user allows directory `path` as alias `alias`
/// for the repository at `root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoEntry {
    /// Absolute, as written.
    pub root: PathBuf,
    pub alias: String,
    /// Absolute, as written.
    pub path: PathBuf,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawUserConfig {
    /// One table per external scheme.
    #[serde(default)]
    #[schemars(extend(
        "propertyNames" = {"pattern": SCHEME_PATTERN, "not": {"enum": ["file", "repo"]}}
    ))]
    providers: BTreeMap<String, RawProvider>,
    #[serde(default)]
    repo: Vec<RawRepoEntry>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawProvider {
    /// argv: the program, then its arguments.
    #[schemars(length(min = 1), extend("prefixItems" = [{"type": "string", "minLength": 1}]))]
    command: Vec<String>,
    /// Default 10000.
    #[schemars(range(min = MIN_TIMEOUT_MS, max = MAX_TIMEOUT_MS))]
    timeout_ms: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RawRepoEntry {
    /// Absolute: the repository whose `hibi.toml` declares the alias.
    root: String,
    #[schemars(extend("pattern" = NAME_PATTERN))]
    alias: String,
    /// Absolute: the directory to use.
    path: String,
}

/// The user-config location of section 11.2 from its inputs: `HIBI_CONFIG`
/// when set and non-empty, else `<config dir>/hibi/config.toml`, where the
/// config dir is `$XDG_CONFIG_HOME` or `~/.config` on Linux and macOS and
/// `%APPDATA%` on Windows. `None` when neither input is available.
pub fn user_config_path_from(
    hibi_config: Option<OsString>,
    config_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    match hibi_config {
        Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
        _ => config_dir.map(|d| d.join("hibi").join("config.toml")),
    }
}

/// The user-config location of section 11.2 for this process, from the
/// environment and `etcetera::choose_base_strategy`.
pub fn user_config_path() -> Option<PathBuf> {
    use etcetera::BaseStrategy;
    let dir = etcetera::choose_base_strategy()
        .ok()
        .map(|s| s.config_dir());
    user_config_path_from(std::env::var_os("HIBI_CONFIG"), dir)
}

impl UserConfig {
    /// Parse the text of a user config. Errors are `invalid_user_config`.
    pub fn parse(text: &str) -> Result<UserConfig, Error> {
        if text.len() as u64 > MAX_CONFIG_BYTES {
            return Err(invalid_user_config(format!(
                "the user config is larger than {MAX_CONFIG_BYTES} bytes"
            )));
        }
        let raw: RawUserConfig = toml::from_str(text)
            .map_err(|e| invalid_user_config(format!("the user config: {e}")))?;
        let mut providers = BTreeMap::new();
        for (scheme, p) in raw.providers {
            if !is_scheme(&scheme) || scheme == "file" || scheme == "repo" {
                return Err(invalid_user_config(format!(
                    "[providers.{scheme}]: the scheme must match [a-z][a-z0-9-]* and be neither file nor repo"
                )));
            }
            if p.command.first().is_none_or(String::is_empty) {
                return Err(invalid_user_config(format!(
                    "[providers.{scheme}]: command must be a list whose first element names a program"
                )));
            }
            let timeout_ms = p.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
            if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&timeout_ms) {
                return Err(invalid_user_config(format!(
                    "[providers.{scheme}]: timeout_ms must be from {MIN_TIMEOUT_MS} to {MAX_TIMEOUT_MS}"
                )));
            }
            providers.insert(
                scheme,
                ProviderCommand {
                    command: p.command,
                    timeout_ms,
                },
            );
        }
        let mut seen = BTreeSet::new();
        let mut repos = Vec::with_capacity(raw.repo.len());
        for r in raw.repo {
            let root = PathBuf::from(&r.root);
            let path = PathBuf::from(&r.path);
            if !root.is_absolute() || !path.is_absolute() {
                return Err(invalid_user_config(format!(
                    "[[repo]] alias {:?}: root and path must be absolute paths",
                    r.alias
                )));
            }
            if !is_name(&r.alias) {
                return Err(invalid_user_config(format!(
                    "[[repo]] alias {:?} must match [a-z0-9][a-z0-9-]*",
                    r.alias
                )));
            }
            if !seen.insert((r.root.clone(), r.alias.clone())) {
                return Err(invalid_user_config(format!(
                    "two [[repo]] entries have root {:?} and alias {:?}",
                    r.root, r.alias
                )));
            }
            repos.push(RepoEntry {
                root,
                alias: r.alias,
                path,
            });
        }
        Ok(UserConfig { providers, repos })
    }

    /// Read and parse the user config at `path` (following symbolic links:
    /// it is the user's own file). A missing file, or `None`, is the empty
    /// config. A file larger than [`MAX_CONFIG_BYTES`] is
    /// `invalid_user_config`.
    pub fn load(path: Option<&Path>) -> Result<UserConfig, Error> {
        let Some(path) = path else {
            return Ok(UserConfig::default());
        };
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(UserConfig::default());
            }
            Err(e) => return Err(Error::io(path.display(), &e)),
        };
        let bytes = read_capped(file, MAX_CONFIG_BYTES)
            .map_err(|e| Error::io(path.display(), &e))?
            .ok_or_else(|| {
                invalid_user_config(format!(
                    "{} is larger than {MAX_CONFIG_BYTES} bytes",
                    path.display()
                ))
            })?;
        let text = String::from_utf8(bytes)
            .map_err(|_| invalid_user_config(format!("{} is not UTF-8", path.display())))?;
        UserConfig::parse(&text).map_err(|e| {
            Error::new(
                Code::InvalidUserConfig,
                format!("{}: {}", path.display(), e.message),
            )
        })
    }
}

// ---------------------------------------------------------------------------
// Sibling repositories (section 11.1)
// ---------------------------------------------------------------------------

/// Why a sibling repository cannot be used (section 11.1). Nothing has been
/// read from any directory when this is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiblingRefusal {
    /// Check 1 or 3 failed: side reason `repo_not_found`.
    RepoNotFound { message: String },
    /// Check 2, 4 or 5 failed: side reason `repo_not_allowed`. For a failed
    /// check 2, `remediation` holds the exact lines to add to the user
    /// config.
    RepoNotAllowed {
        message: String,
        remediation: Option<String>,
    },
}

impl SiblingRefusal {
    /// The side reason code: `repo_not_found` or `repo_not_allowed`.
    pub fn reason(&self) -> &'static str {
        match self {
            SiblingRefusal::RepoNotFound { .. } => "repo_not_found",
            SiblingRefusal::RepoNotAllowed { .. } => "repo_not_allowed",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            SiblingRefusal::RepoNotFound { message }
            | SiblingRefusal::RepoNotAllowed { message, .. } => message,
        }
    }
}

/// The five ordered checks of section 11.1 for using sibling `alias` from
/// the repository at `root`. Returns the canonical directory `D` to read
/// from, or the refusal.
///
/// `is_git_toplevel` is check 5: it must be the first row of the table in
/// section 10.4 run in `D` (`git::Git::is_toplevel`). It is passed in
/// because `config` does not start git (section 2.3); it is called last,
/// only when checks 1 to 4 pass. An error from it refuses with
/// `repo_not_allowed`.
///
/// The caller handles the case of no usable git (section 10.4: every
/// `repo:` side is `repo_not_found`) before calling this.
pub fn allow_sibling(
    alias: &str,
    repo: &RepoConfig,
    user: &UserConfig,
    root: &Path,
    is_git_toplevel: impl FnOnce(&Path) -> Result<bool, Error>,
) -> Result<PathBuf, SiblingRefusal> {
    // 1. The alias is declared in [repos].
    let Some(suggested) = repo.repos.get(alias) else {
        return Err(SiblingRefusal::RepoNotFound {
            message: format!("alias {alias} is not declared under [repos] in {REPO_CONFIG_PATH}"),
        });
    };
    let root = std::fs::canonicalize(root).map_err(|e| SiblingRefusal::RepoNotFound {
        message: format!("the root {} cannot be resolved: {e}", root.display()),
    })?;
    // 2. A [[repo]] entry whose root canonicalizes to this root, with this
    //    alias. The first such entry in file order is used.
    let entry = user
        .repos
        .iter()
        .find(|e| e.alias == alias && std::fs::canonicalize(&e.root).ok().as_ref() == Some(&root));
    let Some(entry) = entry else {
        let path = if Path::new(suggested).is_absolute() {
            PathBuf::from(suggested)
        } else {
            lexical_join(&root, Path::new(suggested))
        };
        let q = |s: &str| toml::Value::String(s.to_owned()).to_string();
        let lines = format!(
            "[[repo]]\nroot = {}\nalias = {}\npath = {}",
            q(&root.to_string_lossy()),
            q(alias),
            q(&path.to_string_lossy())
        );
        return Err(SiblingRefusal::RepoNotAllowed {
            message: format!(
                "the user config does not allow alias {alias} for {}",
                root.display()
            ),
            remediation: Some(format!("Add these lines to the user config:\n{lines}")),
        });
    };
    // 3. The entry's path exists and canonicalizes to a directory D.
    let dir = match std::fs::canonicalize(&entry.path) {
        Ok(d) if d.is_dir() => d,
        _ => {
            return Err(SiblingRefusal::RepoNotFound {
                message: format!(
                    "the user config path {} for alias {alias} is not an existing directory",
                    entry.path.display()
                ),
            });
        }
    };
    // 4. D is not the root, not inside it, not an ancestor of it.
    if dir.starts_with(&root) || root.starts_with(&dir) {
        return Err(SiblingRefusal::RepoNotAllowed {
            message: format!(
                "the directory {} for alias {alias} is the root, inside it, or above it",
                dir.display()
            ),
            remediation: None,
        });
    }
    // 5. D is the top level of a git work tree.
    match is_git_toplevel(&dir) {
        Ok(true) => Ok(dir),
        Ok(false) => Err(SiblingRefusal::RepoNotAllowed {
            message: format!(
                "the directory {} for alias {alias} is not the top level of a git work tree",
                dir.display()
            ),
            remediation: None,
        }),
        Err(e) => Err(SiblingRefusal::RepoNotAllowed {
            message: format!(
                "the directory {} for alias {alias} could not be checked: {}",
                dir.display(),
                e.message
            ),
            remediation: None,
        }),
    }
}

/// `base` joined with `rel`, with `.` and `..` resolved by text only.
fn lexical_join(base: &Path, rel: &Path) -> PathBuf {
    let mut out = base.to_path_buf();
    for c in rel.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    fn keys(schema: &Schema, pointer: &str) -> Vec<String> {
        let v = serde_json::to_value(schema).unwrap();
        let mut keys: Vec<String> = v
            .pointer(pointer)
            .and_then(|p| p.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        keys
    }

    #[test]
    fn the_schemas_list_the_keys_the_parsers_read() {
        let repo = repo_config_schema();
        assert_eq!(
            keys(&repo, "/properties"),
            ["repos", "schemes", "scope", "version"]
        );
        assert_eq!(
            keys(&repo, "/$defs/RawScope/properties"),
            ["exclude", "kinds", "name", "resource"]
        );
        assert_eq!(keys(&repo, "/$defs/RawScheme/properties"), ["description"]);
        let v = serde_json::to_value(&repo).unwrap();
        assert_eq!(v["additionalProperties"], serde_json::json!(false));
        assert_eq!(v["required"], serde_json::json!(["version"]));

        let user = user_config_schema();
        assert_eq!(keys(&user, "/properties"), ["providers", "repo"]);
        assert_eq!(
            keys(&user, "/$defs/RawProvider/properties"),
            ["command", "timeout_ms"]
        );
        assert_eq!(
            keys(&user, "/$defs/RawRepoEntry/properties"),
            ["alias", "path", "root"]
        );
    }

    /// The schemas carry the rules the parsers check after reading, with
    /// the parsers' own constants.
    #[test]
    fn the_schemas_carry_the_rules_the_parsers_check() {
        use serde_json::json;
        let repo = serde_json::to_value(repo_config_schema()).unwrap();
        let at = |v: &serde_json::Value, p: &str| {
            v.pointer(p)
                .unwrap_or_else(|| panic!("no {p} in {v}"))
                .clone()
        };
        assert_eq!(at(&repo, "/properties/version/const"), STORE_VERSION);
        assert_eq!(
            at(&repo, "/properties/repos/propertyNames"),
            json!({"pattern": NAME_PATTERN})
        );
        assert_eq!(
            at(&repo, "/properties/repos/additionalProperties"),
            json!({"type": "string", "minLength": 1})
        );
        let schemes = json!({"pattern": SCHEME_PATTERN, "not": {"enum": ["file", "repo"]}});
        assert_eq!(at(&repo, "/properties/schemes/propertyNames"), schemes);
        let scope = "/$defs/RawScope/properties";
        assert_eq!(at(&repo, &format!("{scope}/name/pattern")), NAME_PATTERN);
        assert_eq!(at(&repo, &format!("{scope}/kinds/minItems")), 1);
        assert_eq!(at(&repo, &format!("{scope}/kinds/items/minLength")), 1);
        let exclusion = "/$defs/RawExclusion/properties";
        assert_eq!(at(&repo, &format!("{exclusion}/unit/minLength")), 1);
        assert_eq!(at(&repo, &format!("{exclusion}/reason/minLength")), 1);

        let user = serde_json::to_value(user_config_schema()).unwrap();
        assert_eq!(at(&user, "/properties/providers/propertyNames"), schemes);
        let provider = "/$defs/RawProvider/properties";
        assert_eq!(at(&user, &format!("{provider}/command/minItems")), 1);
        assert_eq!(
            at(&user, &format!("{provider}/command/prefixItems")),
            json!([{"type": "string", "minLength": 1}])
        );
        assert_eq!(
            at(&user, &format!("{provider}/timeout_ms/minimum")),
            MIN_TIMEOUT_MS
        );
        assert_eq!(
            at(&user, &format!("{provider}/timeout_ms/maximum")),
            MAX_TIMEOUT_MS
        );
        assert_eq!(
            at(&user, "/$defs/RawRepoEntry/properties/alias/pattern"),
            NAME_PATTERN
        );
    }

    /// The patterns name the same strings as the parsers' own checks. No
    /// regular-expression engine is linked, so the patterns are read here
    /// for their fixed shape and the functions are run on samples.
    #[test]
    fn the_name_patterns_describe_the_parser_checks() {
        assert_eq!(NAME_PATTERN, "^[a-z0-9][a-z0-9-]*$");
        assert_eq!(SCHEME_PATTERN, "^[a-z][a-z0-9-]*$");
        for (s, name, scheme) in [
            ("docs", true, true),
            ("0docs", true, false),
            ("a-b-9", true, true),
            ("-a", false, false),
            ("", false, false),
            ("Docs", false, false),
            ("a_b", false, false),
            ("a.b", false, false),
        ] {
            assert_eq!(is_name(s), name, "{s:?}");
            assert_eq!(is_scheme(s), scheme, "{s:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical() {
        assert_eq!(
            lexical_join(Path::new("/w/shop"), Path::new("../api")),
            PathBuf::from("/w/api")
        );
        assert_eq!(
            lexical_join(Path::new("/w/shop"), Path::new("./x/../y")),
            PathBuf::from("/w/shop/y")
        );
    }

    #[test]
    fn config_path_order() {
        assert_eq!(
            user_config_path_from(Some("/a/b.toml".into()), Some("/c".into())),
            Some(PathBuf::from("/a/b.toml"))
        );
        assert_eq!(
            user_config_path_from(Some("".into()), Some("/c".into())),
            Some(PathBuf::from("/c/hibi/config.toml"))
        );
        assert_eq!(
            user_config_path_from(None, Some("/c".into())),
            Some(PathBuf::from("/c/hibi/config.toml"))
        );
        assert_eq!(user_config_path_from(None, None), None);
    }

    #[test]
    fn globs() {
        let p = ResourcePattern::parse("file:docs/**/*.md").unwrap();
        assert!(p.matches_path("docs/a.md"));
        assert!(p.matches_path("docs/x/y/a.md"));
        assert!(!p.matches_path("docs/a.txt"));
        let p = ResourcePattern::parse("file:docs/*.md").unwrap();
        assert!(!p.matches_path("docs/x/a.md"));
        let p = ResourcePattern::parse("file:{a,b}/?.rs").unwrap();
        assert!(p.matches_path("a/x.rs") && p.matches_path("b/y.rs") && !p.matches_path("c/x.rs"));
        let p = ResourcePattern::parse("repo:api//src/**").unwrap();
        assert!(p.matches_locator(&Locator::new("repo:api//src/a.rs").unwrap()));
        assert!(!p.matches_locator(&Locator::new("repo:web//src/a.rs").unwrap()));
        assert!(!p.matches_locator(&Locator::new("file:src/a.rs").unwrap()));
        let p = ResourcePattern::parse("archctl:system/a").unwrap();
        assert!(p.matches_locator(&Locator::new("archctl:system/a").unwrap()));
        assert!(ResourcePattern::parse("file:").is_err());
        assert!(ResourcePattern::parse("file:[").is_err());
        assert!(ResourcePattern::parse("nocolon").is_err());
    }
}
