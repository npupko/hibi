//! `FileProvider` and the `UnitCache` trait (spec section 6).
//!
//! The file provider serves the schemes `file` and `repo`. It reads one of
//! two sources (section 6.8): the working tree, or the blobs of one commit
//! of the root repository. It answers `resolve`, `units` and `render` for
//! one locator at a time; the engine enumerates resources and changed paths
//! itself.

mod doc;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use crate::code::Class;
use crate::config::{self, RepoConfig, SiblingRefusal, UserConfig};
use crate::error::Error;
use crate::git::{self, Blob, GitStatus, ObjectId, TreeEntry};
use crate::model::{Anchor, Locator, LocatorParts, SelectorKey};
use crate::provider::{
    Capabilities, NameVersion, Provider, ProviderError, ProviderErrorCode, ProviderInfo, Rendered,
    Unit,
};

use doc::Doc;

/// The largest file the provider reads, in bytes (section 6.1). A longer
/// file is `too_large` and is not read.
pub const MAX_FILE_BYTES: u64 = 8_388_608;

/// The largest file of a code class the provider reads, in bytes (ruling
/// on section 6.1). Parsing costs far more memory per byte than the other
/// classes, so a longer code file is `too_large` and is neither read nor
/// parsed.
pub const MAX_CODE_FILE_BYTES: u64 = 2_097_152;

/// The size cap for a path: [`MAX_CODE_FILE_BYTES`] for a code class,
/// [`MAX_FILE_BYTES`] for markdown, prose and text.
fn byte_limit(path: &str) -> u64 {
    if Class::from_path(path).is_code() {
        MAX_CODE_FILE_BYTES
    } else {
        MAX_FILE_BYTES
    }
}

/// The largest cache row the provider writes, in bytes (ruling on 11.3).
pub const MAX_CACHE_ROW_BYTES: usize = 8_388_608;

pub use doc::MAX_UNITS_PER_RESOURCE;

/// The analysed documents a provider keeps between calls (ruling on
/// section 6.7): `render`, `units` and `resolve` of one run read a resource
/// once while its bytes are unchanged. A small bound keeps memory at a few
/// documents.
const MEMO_DOCS: usize = 4;

/// One analysed document, keyed by the exact bytes read and the class, so
/// a file changed within a run is analysed again, never served stale.
struct Memo {
    class: Class,
    bytes: Vec<u8>,
    doc: Rc<Doc>,
}

/// The provider name in [`ProviderInfo`].
pub const PROVIDER_NAME: &str = "hibi-file";

/// Every unit kind the file provider returns (section 6.2).
pub const KINDS: [&str; 22] = [
    "sentence",
    "heading",
    "list-item",
    "table-row",
    "fenced",
    "function",
    "method",
    "class",
    "struct",
    "interface",
    "trait",
    "enum",
    "type",
    "variable",
    "field",
    "variant",
    "module",
    "impl",
    "macro",
    "statement",
    "block",
    "file",
];

/// The cache form of a file that is not valid UTF-8 (section 11.3).
const NOT_TEXT: &str = "not_text";

/// A store for the unit list of a file's bytes, keyed by git blob id,
/// resource class and hibi version (section 11.3). The cache is never
/// authoritative: an implementation that loses a row, or fails, changes no
/// result. The version is the implementation's own (`cache::Cache` is opened
/// for one engine version), so it is not an argument.
pub trait UnitCache {
    /// The stored value for a blob read as `class`: a JSON array of units
    /// without `text`, or `not_text`. `None` when there is none or on error.
    fn get_units(&self, blob: &str, class: &str) -> Option<String>;
    /// Store the value for a blob read as `class`. Errors are ignored.
    fn put_units(&self, blob: &str, class: &str, units: &str);
}

impl<T: UnitCache + ?Sized> UnitCache for Rc<T> {
    fn get_units(&self, blob: &str, class: &str) -> Option<String> {
        (**self).get_units(blob, class)
    }
    fn put_units(&self, blob: &str, class: &str, units: &str) {
        (**self).put_units(blob, class, units)
    }
}

impl<T: UnitCache + ?Sized> UnitCache for Arc<T> {
    fn get_units(&self, blob: &str, class: &str) -> Option<String> {
        (**self).get_units(blob, class)
    }
    fn put_units(&self, blob: &str, class: &str, units: &str) {
        (**self).put_units(blob, class, units)
    }
}

impl<T: UnitCache + ?Sized> UnitCache for &T {
    fn get_units(&self, blob: &str, class: &str) -> Option<String> {
        (**self).get_units(blob, class)
    }
    fn put_units(&self, blob: &str, class: &str, units: &str) {
        (**self).put_units(blob, class, units)
    }
}

/// What a [`FileProvider`] needs besides its root.
pub struct FileOptions {
    /// `.hibi/hibi.toml`: the `[repos]` aliases.
    pub repo: RepoConfig,
    /// The user config: the `[[repo]]` entries that allow an alias.
    pub user: UserConfig,
    /// The once-per-run git check. Without a usable git every `repo:`
    /// locator is `repo_not_found` (section 10.4).
    pub git: GitStatus,
    /// The unit cache, or `None` to compute everything.
    pub cache: Option<Box<dyn UnitCache>>,
}

/// One sibling repository that passed the checks of section 11.1, with its
/// file list (section 10.4).
struct Sibling {
    dir: PathBuf,
    files: HashSet<String>,
}

enum Source {
    WorkingTree {
        /// Per alias: the sibling, or the error every locator of it gets.
        siblings: HashMap<String, Result<Rc<Sibling>, ProviderError>>,
        /// The root's file list, fetched once: `None` until first use;
        /// `Some(Ok(None))` when git is not usable for the root (the
        /// working-tree rule alone applies); `Some(Err(_))` when listing
        /// failed.
        root_files: Option<Result<Option<Rc<HashSet<String>>>, ProviderError>>,
    },
    Revision {
        base: ObjectId,
        /// Per path: its tree entry at `base`, or `None` when it has none.
        entries: HashMap<String, Option<TreeEntry>>,
    },
}

/// The built-in provider for `file:` and `repo:` locators (section 6).
pub struct FileProvider {
    root: PathBuf,
    source: Source,
    repo: RepoConfig,
    user: UserConfig,
    git: GitStatus,
    cache: Option<Box<dyn UnitCache>>,
    info: ProviderInfo,
    /// The last analysed documents, most recent first, at most `MEMO_DOCS`.
    memo: VecDeque<Memo>,
}

fn err(code: ProviderErrorCode, message: impl Into<String>) -> ProviderError {
    ProviderError::new(code, message)
}

fn info() -> ProviderInfo {
    ProviderInfo {
        protocol: 1,
        provider: NameVersion {
            name: PROVIDER_NAME.into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        schemes: vec!["file".into(), "repo".into()],
        selectors: vec![SelectorKey::Quote, SelectorKey::Symbol],
        kinds: KINDS.iter().map(|k| (*k).to_owned()).collect(),
        capabilities: Capabilities {
            units: true,
            render: true,
            batch: true,
        },
    }
}

impl FileProvider {
    /// A provider that reads the working tree under `root`, and sibling
    /// repositories for `repo:` locators.
    pub fn working_tree(root: impl Into<PathBuf>, options: FileOptions) -> FileProvider {
        FileProvider::new(
            root.into(),
            Source::WorkingTree {
                siblings: HashMap::new(),
                root_files: None,
            },
            options,
        )
    }

    /// A provider that reads the blobs of commit `base` of the repository
    /// at `root` (sections 6.8 and 10.4). `repo:` locators are not available
    /// in it (`invalid_locator`). Fails with `not_a_git_repository` or
    /// `git_too_old` when git is not usable.
    pub fn revision(
        root: impl Into<PathBuf>,
        base: ObjectId,
        options: FileOptions,
    ) -> Result<FileProvider, Error> {
        options.git.require()?;
        Ok(FileProvider::new(
            root.into(),
            Source::Revision {
                base,
                entries: HashMap::new(),
            },
            options,
        ))
    }

    fn new(root: PathBuf, source: Source, options: FileOptions) -> FileProvider {
        FileProvider {
            root,
            source,
            repo: options.repo,
            user: options.user,
            git: options.git,
            cache: options.cache,
            info: info(),
            memo: VecDeque::new(),
        }
    }

    /// The sibling repository of `alias`, checked once per provider.
    fn sibling(&mut self, alias: &str) -> Result<Rc<Sibling>, ProviderError> {
        let Source::WorkingTree { siblings, .. } = &mut self.source else {
            return Err(err(
                ProviderErrorCode::InvalidLocator,
                "repo: locators are not available at a base commit",
            ));
        };
        if let Some(s) = siblings.get(alias) {
            return s.clone();
        }
        let result = (|| {
            let Some(git) = self.git.git() else {
                let why = self
                    .git
                    .too_old_message()
                    .unwrap_or_else(|| "git cannot be started".into());
                return Err(err(
                    ProviderErrorCode::RepoNotFound,
                    format!("repo:{alias} needs git: {why}"),
                ));
            };
            let dir = config::allow_sibling(alias, &self.repo, &self.user, &self.root, |d| {
                git.is_toplevel(d)
            })
            .map_err(|r| match r {
                SiblingRefusal::RepoNotFound { message } => {
                    err(ProviderErrorCode::RepoNotFound, message)
                }
                SiblingRefusal::RepoNotAllowed {
                    message,
                    remediation,
                } => err(
                    ProviderErrorCode::RepoNotAllowed,
                    match remediation {
                        Some(r) => format!("{message}. {r}"),
                        None => message,
                    },
                ),
            })?;
            let files = git.file_list(&dir).map_err(|e| {
                err(
                    ProviderErrorCode::Unavailable,
                    format!(
                        "the file list of repo:{alias} cannot be read ({}); check that {} is a git repository that git can read",
                        detail(&e.message),
                        dir.display()
                    ),
                )
            })?;
            Ok(Rc::new(Sibling {
                dir,
                files: files.into_iter().collect(),
            }))
        })();
        siblings.insert(alias.to_owned(), result.clone());
        result
    }

    /// Look up the tree entries of `paths` at the base commit that are not
    /// known yet, with one `ls-tree`.
    fn prefetch(&mut self, paths: &BTreeSet<String>) -> Result<(), ProviderError> {
        let Source::Revision { base, entries } = &mut self.source else {
            return Ok(());
        };
        let missing: BTreeSet<String> = paths
            .iter()
            .filter(|p| !entries.contains_key(*p))
            .cloned()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let git = self.git.git().ok_or_else(git_not_usable)?;
        let found: BTreeMap<String, TreeEntry> = git
            .tree_entries(&self.root, base, &missing)
            .map_err(|e| git_cannot_read(base, &e.message))?;
        for p in missing {
            let e = found.get(&p).cloned();
            entries.insert(p, e);
        }
        Ok(())
    }

    /// The bytes of a resource, after the path rule of section 6.8 and the
    /// size cap of section 6.1. Returns the bytes and the locator path.
    fn read_bytes(&mut self, loc: &Locator) -> Result<(Vec<u8>, String), ProviderError> {
        match loc.parts() {
            LocatorParts::File { path } => {
                let path = path.to_owned();
                if is_private_path(&path) {
                    return Err(ProviderError::not_found(format!(
                        "{loc}: paths under .git/ and .hibi/ are never read"
                    )));
                }
                if matches!(self.source, Source::Revision { .. }) {
                    let bytes = self.read_revision(&path)?;
                    return Ok((bytes, path));
                }
                // When git is usable for the root, only the paths of its file
                // list are read (ruling on section 6.8), so an ignored file
                // such as `.env` is never opened.
                if let Some(files) = self.root_files()?
                    && !files.contains(&path)
                {
                    return Err(ProviderError::not_found(format!(
                        "{loc} is not in git's file list of the root (tracked, or untracked and not ignored)"
                    )));
                }
                let bytes = read_tree_file(&self.root, &path, loc)?;
                Ok((bytes, path))
            }
            LocatorParts::Repo { alias, path } => {
                let sibling = self.sibling(alias)?;
                // Nothing outside the sibling's file list is ever examined.
                if !sibling.files.contains(path) {
                    return Err(ProviderError::not_found(format!(
                        "{loc} is not in the file list of repo:{alias}"
                    )));
                }
                let bytes = read_tree_file(&sibling.dir, path, loc)?;
                Ok((bytes, path.to_owned()))
            }
            LocatorParts::External { scheme, .. } => Err(err(
                ProviderErrorCode::InvalidLocator,
                format!("the file provider does not serve scheme {scheme}"),
            )),
        }
    }

    /// The root's file list (section 10.4) when git is usable for the root:
    /// git passed the version check and the root is a git top level.
    /// `None` otherwise, when the working-tree rule alone applies. Fetched
    /// once per provider. A failing list is `unavailable`.
    fn root_files(&mut self) -> Result<Option<Rc<HashSet<String>>>, ProviderError> {
        let Source::WorkingTree { root_files, .. } = &mut self.source else {
            return Ok(None);
        };
        if let Some(r) = root_files {
            return r.clone();
        }
        let result = match self.git.git() {
            Some(git) if git.is_toplevel(&self.root).unwrap_or(false) => git
                .file_list(&self.root)
                .map(|files| Some(Rc::new(files.into_iter().collect())))
                .map_err(|e| {
                    err(
                        ProviderErrorCode::Unavailable,
                        format!(
                            "the root's file list cannot be read ({}); check that the root is a git repository that git can read",
                            detail(&e.message)
                        ),
                    )
                }),
            _ => Ok(None),
        };
        *root_files = Some(result.clone());
        result
    }

    fn read_revision(&mut self, path: &str) -> Result<Vec<u8>, ProviderError> {
        self.prefetch(&BTreeSet::from([path.to_owned()]))?;
        let Source::Revision { base, entries } = &self.source else {
            unreachable!("read_revision is called for a revision source");
        };
        let not_found =
            || ProviderError::not_found(format!("file:{path} does not exist at {base}"));
        let entry = match entries.get(path) {
            Some(Some(e)) if e.mode.is_regular_file() => e.clone(),
            _ => return Err(not_found()),
        };
        let git = self.git.git().ok_or_else(git_not_usable)?;
        let limit = byte_limit(path);
        let blobs = git
            .read_blobs(&self.root, std::slice::from_ref(&entry.id), limit)
            .map_err(|e| git_cannot_read(base, &e.message))?;
        match blobs.into_iter().next() {
            Some(Blob::Content(bytes)) => Ok(bytes),
            Some(Blob::TooLarge { size }) => Err(too_large(&format!("file:{path}"), size, limit)),
            Some(Blob::NotBlob) => Err(not_found()),
            Some(Blob::Missing) | None => Err(err(
                ProviderErrorCode::Unavailable,
                format!(
                    "the object {} of file:{path} at {base} is not in the repository; fetch it, for example with git fetch when the clone is shallow or partial",
                    entry.id
                ),
            )),
        }
    }

    /// Read and analyze a resource. The bytes are read on every call; a
    /// document analysed from the same bytes and class in an earlier call is
    /// reused.
    fn doc(&mut self, loc: &Locator) -> Result<Rc<Doc>, ProviderError> {
        let (bytes, path) = self.read_bytes(loc)?;
        let class = Class::from_path(&path);
        if let Some(k) = self
            .memo
            .iter()
            .position(|m| m.class == class && m.bytes == bytes)
        {
            let m = self.memo.remove(k).expect("found above");
            let doc = Rc::clone(&m.doc);
            self.memo.push_front(m);
            return Ok(doc);
        }
        let doc = Rc::new(self.analyse(loc, bytes.clone(), class)?);
        self.memo.push_front(Memo {
            class,
            bytes,
            doc: Rc::clone(&doc),
        });
        self.memo.truncate(MEMO_DOCS);
        Ok(doc)
    }

    /// Decode and analyse the bytes of a resource, through the unit cache.
    fn analyse(&self, loc: &Locator, bytes: Vec<u8>, class: Class) -> Result<Doc, ProviderError> {
        let cache = self.cache.as_deref();
        let blob = cache.map(|_| git::blob_id(&bytes));
        let row = match (cache, &blob) {
            (Some(c), Some(b)) => c.get_units(b, class.name()),
            _ => None,
        };
        let not_text = || {
            err(
                ProviderErrorCode::NotText,
                format!("{loc} is not valid UTF-8"),
            )
        };
        // A `not_text` row is not believed: the bytes decide (ruling on
        // section 11.3). A row that says `not_text` for valid UTF-8 is a
        // miss and is rewritten below.
        let text = match String::from_utf8(bytes) {
            Ok(t) => t,
            Err(_) => {
                if let (Some(c), Some(b)) = (cache, &blob) {
                    c.put_units(b, class.name(), NOT_TEXT);
                }
                return Err(not_text());
            }
        };
        let text = if text.contains("\r\n") {
            text.replace("\r\n", "\n")
        } else {
            text
        };
        if let Some(d) = row.and_then(|r| Doc::from_cache(text.clone(), class, &r)) {
            return Ok(d);
        }
        let d = Doc::build(text, class)?;
        if let (Some(c), Some(b)) = (cache, &blob) {
            let row = d.to_cache();
            // A row larger than the cap is not written (ruling on 11.3).
            if row.len() <= MAX_CACHE_ROW_BYTES {
                c.put_units(b, class.name(), &row);
            }
        }
        Ok(d)
    }
}

/// A `file:` path under `.git/` (any segment) or `.hibi/` (the first
/// segment), compared without ASCII case so that a case-insensitive file
/// system cannot reach them through `.GIT` (ruling on section 6.8).
fn is_private_path(path: &str) -> bool {
    let mut segments = path.split('/');
    let first = segments.next().unwrap_or("");
    let is = |s: &str, name: &str| s.eq_ignore_ascii_case(name);
    is(first, ".git") || is(first, ".hibi") || segments.any(|s| is(s, ".git"))
}

/// Characters of a git or system message quoted in an `unavailable`
/// message. The engine cuts the whole message to 500 characters before it
/// adds what to do after the fix, so the quoted part is cut here and the
/// fix at the end is never lost.
const DETAIL_CHARS: usize = 200;

/// `message` cut to [`DETAIL_CHARS`] characters, without a final `.`.
fn detail(message: &str) -> String {
    let m = message.trim().trim_end_matches('.');
    match m.char_indices().nth(DETAIL_CHARS) {
        Some((i, _)) => format!("{}...", &m[..i]),
        None => m.to_owned(),
    }
}

/// `unavailable` messages state the cause and the fix and end there: the
/// engine adds that the command can be run again (ruling on section 9.2).
fn git_not_usable() -> ProviderError {
    err(
        ProviderErrorCode::Unavailable,
        "git is needed to read the base commit and is not usable; install git or repair its installation",
    )
}

fn git_cannot_read(base: &ObjectId, message: &str) -> ProviderError {
    err(
        ProviderErrorCode::Unavailable,
        format!(
            "git cannot read the base commit {base} ({}); check that the commit exists in this repository",
            detail(message)
        ),
    )
}

fn too_large(what: &str, size: u64, limit: u64) -> ProviderError {
    let of = if limit == MAX_CODE_FILE_BYTES {
        " bytes for a code file"
    } else {
        " bytes"
    };
    err(
        ProviderErrorCode::TooLarge,
        format!("{what} has {size} bytes, more than the limit of {limit}{of}"),
    )
}

/// Read `path` under `base` by the rule of section 6.8: every component
/// below `base` is checked without following links, every directory
/// component is a directory, the last is a regular file, and none is a
/// symbolic link. Anything else is `not_found`.
fn read_tree_file(base: &Path, path: &str, loc: &Locator) -> Result<Vec<u8>, ProviderError> {
    let not_found = |why: &str| ProviderError::not_found(format!("{loc}: {why}"));
    let io_err = |e: &std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
            not_found("no such file")
        }
        _ => err(
            ProviderErrorCode::Unavailable,
            format!(
                "{loc} cannot be read ({}); make it readable by this user",
                detail(&e.to_string())
            ),
        ),
    };
    let segments: Vec<&str> = path.split('/').collect();
    let mut cur = base.to_path_buf();
    let mut size = 0;
    for (i, seg) in segments.iter().enumerate() {
        // A segment must be one plain name on this platform (a `\` or a
        // drive prefix on Windows would leave `base`).
        let mut comps = Path::new(seg).components();
        if !matches!((comps.next(), comps.next()), (Some(Component::Normal(c)), None) if c == *seg)
        {
            return Err(not_found("the path is not a plain relative path here"));
        }
        cur.push(seg);
        let md = std::fs::symlink_metadata(&cur).map_err(|e| io_err(&e))?;
        if md.file_type().is_symlink() {
            return Err(not_found("a path component is a symbolic link"));
        }
        let last = i + 1 == segments.len();
        if last {
            if !md.is_file() {
                return Err(not_found("not a regular file"));
            }
            size = md.len();
        } else if !md.is_dir() {
            return Err(not_found("a path component is not a directory"));
        }
    }
    let limit = byte_limit(path);
    if size > limit {
        return Err(too_large(loc.as_str(), size, limit));
    }
    let mut opts = OpenOptions::new();
    opts.read(true);
    config::nofollow::apply(&mut opts);
    let file = opts.open(&cur).map_err(|e| {
        if config::nofollow::is_link_error(&e) {
            not_found("a path component is a symbolic link")
        } else {
            io_err(&e)
        }
    })?;
    let md = file.metadata().map_err(|e| io_err(&e))?;
    if !md.is_file() {
        return Err(not_found("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_err(&e))?;
    if bytes.len() as u64 > limit {
        return Err(too_large(
            loc.as_str(),
            md.len().max(bytes.len() as u64),
            limit,
        ));
    }
    Ok(bytes)
}

impl Provider for FileProvider {
    fn info(&mut self) -> Result<&ProviderInfo, ProviderError> {
        Ok(&self.info)
    }

    fn resolve(&mut self, anchors: &[Anchor]) -> Vec<Result<Vec<Unit>, ProviderError>> {
        // One ls-tree for every path of the batch at a base commit.
        if matches!(self.source, Source::Revision { .. }) {
            let paths: BTreeSet<String> = anchors
                .iter()
                .filter_map(|a| match a.loc.parts() {
                    LocatorParts::File { path } => Some(path.to_owned()),
                    _ => None,
                })
                .collect();
            if let Err(e) = self.prefetch(&paths) {
                return anchors.iter().map(|_| Err(e.clone())).collect();
            }
        }
        // Each resource is read once per batch.
        let mut docs: HashMap<&Locator, Result<Rc<Doc>, ProviderError>> = HashMap::new();
        anchors
            .iter()
            .map(|a| {
                let doc = match docs.get(&a.loc) {
                    Some(d) => d.clone(),
                    None => {
                        let d = self.doc(&a.loc);
                        docs.insert(&a.loc, d.clone());
                        d
                    }
                }?;
                if a.sel.key.is_some() {
                    return Err(err(
                        ProviderErrorCode::InvalidSelector,
                        "the file provider does not accept the selector key `key`",
                    ));
                }
                a.sel
                    .validate()
                    .map_err(|e| ProviderError::invalid_request(e.message))?;
                // Error 8 for a symbol; error 9 for a symbol or the empty
                // selector on code that does not parse (section 6.7).
                if a.sel.symbol.is_some() {
                    doc.check_symbol(a.loc.as_str())?;
                } else if a.sel.quote.is_none() {
                    doc.check_parses(a.loc.as_str())?;
                }
                doc.resolve(&a.sel)
            })
            .collect()
    }

    fn units(&mut self, loc: &Locator, kind: Option<&str>) -> Result<Vec<Unit>, ProviderError> {
        Ok(self.doc(loc)?.units(kind))
    }

    fn render(&mut self, loc: &Locator, unit: &str) -> Result<Rendered, ProviderError> {
        self.doc(loc)?.render(unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_details_leave_room_for_the_fix() {
        assert_eq!(detail("  fatal: bad index.\n"), "fatal: bad index");
        let long = "é".repeat(1000);
        let cut = detail(&long);
        assert_eq!(cut.chars().count(), DETAIL_CHARS + 3);
        assert!(cut.ends_with("..."));
        // The longest message still ends with its fix within the 500
        // characters the engine keeps.
        let e = git_cannot_read(&ObjectId::new("a".repeat(40)).unwrap(), &long);
        assert!(e.message.chars().count() <= 500, "{}", e.message);
        assert!(e.message.ends_with("exists in this repository"));
    }
}
