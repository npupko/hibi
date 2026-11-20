//! The only place that starts `git` (spec section 10.4).
//!
//! Every git process is built by one private function that adds the
//! hardening flags and environment of section 10.4, so no caller can start
//! git without them. A [`Git`] value exists only after the version check of
//! section 10.4 passed: [`GitStatus::probe`] runs `git --version` once and
//! yields either a usable [`Git`], or the reason none can be used. The
//! caller probes once per run and keeps the result.
//!
//! Every result is typed. Paths are returned as `String`s relative to the
//! directory git ran in; a path that is not UTF-8 or that contains a
//! character U+0000 to U+001F or U+007F is dropped, because no locator can
//! name it (section 5.1).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Code, Error};

/// The oldest git hibi runs (section 10.4).
pub const MIN_GIT: GitVersion = GitVersion {
    major: 2,
    minor: 45,
    patch: 0,
};

/// The flags every invocation carries after `git`, before `-C <dir>` is
/// followed by the configuration overrides (section 10.4).
const CONFIG_FLAGS: [&str; 8] = [
    "-c",
    "core.quotePath=false",
    "-c",
    "safe.bareRepository=explicit",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.untrackedCache=false",
];

/// The environment additions of every invocation (section 10.4).
pub const GIT_ENV: [(&str, &str); 4] = [
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("LC_ALL", "C"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_NO_LAZY_FETCH", "1"),
];

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/// A git version: major, minor, patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl fmt::Display for GitVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Parse the output of `git --version` by the rule of section 10.4: after
/// the prefix `git version `, the first three dot-separated integers are
/// major, minor and patch. A missing third integer counts as 0. `None` when
/// the output does not parse (which counts as too old).
///
/// `git version 2.54.0 (Apple Git-157)` is 2.54.0; `git version 2.45` is
/// 2.45.0; `git version 2.45.1.windows.1` is 2.45.1.
pub fn parse_version(output: &str) -> Option<GitVersion> {
    let rest = output.trim_end().strip_prefix("git version ")?;
    let word = rest.split_whitespace().next()?;
    let mut parts = word.split('.');
    let int = |s: Option<&str>| -> Option<u32> {
        let s = s?;
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    let major = int(parts.next())?;
    let minor = int(parts.next())?;
    let patch = int(parts.next()).unwrap_or(0);
    Some(GitVersion {
        major,
        minor,
        patch,
    })
}

/// The result of the once-per-run version check (section 10.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStatus {
    /// git started and is at least [`MIN_GIT`].
    Ready(Git),
    /// git could not be started.
    Missing { message: String },
    /// git started but is older than [`MIN_GIT`], or its version output did
    /// not parse (`found` is then `None` and `output` holds what it printed).
    TooOld {
        found: Option<GitVersion>,
        output: String,
    },
}

impl GitStatus {
    /// Run `git --version` with `git` from `PATH`.
    pub fn probe() -> GitStatus {
        GitStatus::probe_program("git")
    }

    /// Run `<program> --version`, with no other argument, and classify the
    /// result.
    pub fn probe_program(program: impl Into<OsString>) -> GitStatus {
        let program = program.into();
        let out = Command::new(&program)
            .arg("--version")
            .envs(GIT_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        match out {
            Err(e) => GitStatus::Missing {
                message: format!("git could not be started: {e}"),
            },
            Ok(out) => {
                GitStatus::from_version_output(program, &String::from_utf8_lossy(&out.stdout))
            }
        }
    }

    /// Classify the standard output of `git --version` of a git that
    /// started. Exposed so that the parsing rule can be tested with injected
    /// version strings.
    pub fn from_version_output(program: impl Into<OsString>, output: &str) -> GitStatus {
        match parse_version(output) {
            Some(v) if v >= MIN_GIT => GitStatus::Ready(Git {
                program: program.into(),
                version: v,
            }),
            found => GitStatus::TooOld {
                found,
                output: output.trim_end().to_owned(),
            },
        }
    }

    /// For a command that requires git (section 10.4, first row): the usable
    /// git, or `not_a_git_repository` when git cannot be started, or
    /// `git_too_old` naming the version found and the version required.
    pub fn require(&self) -> Result<&Git, Error> {
        match self {
            GitStatus::Ready(git) => Ok(git),
            GitStatus::Missing { message } => Err(Error::new(Code::NotAGitRepository, message)),
            GitStatus::TooOld { .. } => Err(Error::new(
                Code::GitTooOld,
                self.too_old_message().unwrap_or_default(),
            )),
        }
    }

    /// The usable git, if any. For a `repo:` side (section 10.4, second
    /// row) `None` means the side is `unavailable` with `repo_not_found`;
    /// when [`GitStatus::too_old_message`] is `Some`, the run also carries
    /// the warning `git_too_old` with that message, once.
    pub fn git(&self) -> Option<&Git> {
        match self {
            GitStatus::Ready(git) => Some(git),
            _ => None,
        }
    }

    /// The message of `git_too_old` (error or warning), naming the version
    /// found and the version required; `None` unless git is too old.
    pub fn too_old_message(&self) -> Option<String> {
        match self {
            GitStatus::TooOld { found: Some(v), .. } => Some(format!(
                "git {v} is installed; hibi requires git {MIN_GIT} or later"
            )),
            GitStatus::TooOld {
                found: None,
                output,
            } => Some(format!(
                "the git version {output:?} could not be read; hibi requires git {MIN_GIT} or later"
            )),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Typed values
// ---------------------------------------------------------------------------

/// A git object id: 40 (SHA-1) or 64 (SHA-256) lower-case hex digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(String);

impl ObjectId {
    /// Validate and wrap. Errors are `invalid_argument`.
    pub fn new(s: impl Into<String>) -> Result<ObjectId, Error> {
        let s = s.into();
        if (s.len() == 40 || s.len() == 64)
            && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            Ok(ObjectId(s))
        } else {
            Err(Error::invalid_argument(format!(
                "{s:?} is not a git object id"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The base of a `--since` range (section 10.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinceRange {
    /// `A`, `A..`, `A..HEAD`: the base is commit `A`.
    Commit(String),
    /// `A...`, `A...HEAD`: the base is `git merge-base A HEAD`.
    MergeBase(String),
}

/// Parse a `--since` argument (section 10.3). Only `A`, `A..`, `A..HEAD`,
/// `A...` and `A...HEAD` are accepted; anything else, including a right-hand
/// side other than `HEAD`, is `invalid_range`. `A` is checked later by
/// [`Git::since_base`].
pub fn parse_range(arg: &str) -> Result<SinceRange, Error> {
    let bad = || {
        Error::new(
            Code::InvalidRange,
            format!("{arg:?} is not a range hibi accepts"),
        )
    };
    let (a, merge_base) = if let Some(a) = arg.strip_suffix("...HEAD") {
        (a, true)
    } else if let Some(a) = arg.strip_suffix("...") {
        (a, true)
    } else if let Some(a) = arg.strip_suffix("..HEAD") {
        (a, false)
    } else if let Some(a) = arg.strip_suffix("..") {
        (a, false)
    } else {
        (arg, false)
    };
    if a.is_empty() || a.contains("..") || a.starts_with('-') {
        return Err(bad());
    }
    let a = a.to_owned();
    Ok(if merge_base {
        SinceRange::MergeBase(a)
    } else {
        SinceRange::Commit(a)
    })
}

/// The mode of a tree entry (`ls-tree`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeMode {
    /// `100644`
    Regular,
    /// `100755`
    Executable,
    /// `120000`
    Symlink,
    /// `160000`, a submodule commit.
    Gitlink,
    /// Any other mode, as printed.
    Other(String),
}

impl TreeMode {
    fn parse(s: &str) -> TreeMode {
        match s {
            "100644" => TreeMode::Regular,
            "100755" => TreeMode::Executable,
            "120000" => TreeMode::Symlink,
            "160000" => TreeMode::Gitlink,
            other => TreeMode::Other(other.to_owned()),
        }
    }

    /// The rule of section 6.8 for a revision source: the mode is `100644`
    /// or `100755`.
    pub fn is_regular_file(&self) -> bool {
        matches!(self, TreeMode::Regular | TreeMode::Executable)
    }
}

/// One entry of `ls-tree -r` at the base commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub mode: TreeMode,
    /// The object id: a blob for files and links, a commit for a gitlink.
    pub id: ObjectId,
}

/// One answer of `cat-file --batch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blob {
    /// The blob's bytes, exactly as stored (no filter, no textconv).
    Content(Vec<u8>),
    /// The blob is larger than the limit given; its bytes were not kept.
    TooLarge { size: u64 },
    /// The object is not a blob.
    NotBlob,
    /// The object does not exist.
    Missing,
}

/// The git blob id of `bytes`: hex SHA-1 of `"blob <len>\0"` followed by
/// the bytes (section 11.3).
#[cfg(any(feature = "file", feature = "cache"))]
pub fn blob_id(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", bytes.len()).as_bytes());
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(40);
    for b in digest.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

/// A git that passed the version check. Every method runs one git process
/// with the flags and environment of section 10.4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Git {
    program: OsString,
    version: GitVersion,
}

/// A finished git process.
struct Output {
    ok: bool,
    stdout: Vec<u8>,
    stderr: String,
}

impl Git {
    /// The version found by the check.
    pub fn version(&self) -> GitVersion {
        self.version
    }

    /// The command line of section 10.4: `git --no-pager -C <dir> -c ... <args>`,
    /// without a shell, standard input closed, the environment additions
    /// set. This is the only place a git `Command` is built after the
    /// version check.
    fn command(&self, dir: &Path, args: &[&OsStr]) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.arg("--no-pager")
            .arg("-C")
            .arg(dir)
            .args(CONFIG_FLAGS)
            .args(args)
            .envs(GIT_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    fn describe(args: &[&OsStr]) -> String {
        let parts: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        format!("git {}", parts.join(" "))
    }

    fn run(&self, dir: &Path, args: &[&OsStr]) -> Result<Output, Error> {
        let out = self.command(dir, args).output().map_err(|e| {
            Error::new(
                Code::GitFailed,
                format!("{} could not be started: {e}", Git::describe(args)),
            )
        })?;
        Ok(Output {
            ok: out.status.success(),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).trim_end().to_owned(),
        })
    }

    /// Run, and turn a non-zero exit into `git_failed` with git's standard
    /// error in the message.
    fn run_ok(&self, dir: &Path, args: &[&OsStr]) -> Result<Vec<u8>, Error> {
        let out = self.run(dir, args)?;
        if out.ok {
            Ok(out.stdout)
        } else {
            Err(Error::new(
                Code::GitFailed,
                format!(
                    "{} failed in {}: {}",
                    Git::describe(args),
                    dir.display(),
                    out.stderr
                ),
            ))
        }
    }

    /// `rev-parse --show-toplevel` in `dir`, canonicalized. `None` when git
    /// reports that `dir` is not in a work tree (or cannot enter it).
    pub fn toplevel(&self, dir: &Path) -> Result<Option<PathBuf>, Error> {
        let out = self.run(
            dir,
            &[OsStr::new("rev-parse"), OsStr::new("--show-toplevel")],
        )?;
        if !out.ok {
            return Ok(None);
        }
        let mut bytes = out.stdout;
        while bytes.last() == Some(&b'\n') || bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        if bytes.is_empty() {
            // A bare repository or a `.git` directory has no top level.
            return Ok(None);
        }
        let top = bytes_to_path(bytes);
        Ok(std::fs::canonicalize(&top).ok())
    }

    /// True when `dir`, canonicalized, is the canonicalized top level of a
    /// git work tree (section 10.4, first row). Used for the root and for a
    /// sibling repository (section 11.1 check 5).
    pub fn is_toplevel(&self, dir: &Path) -> Result<bool, Error> {
        let Ok(canon) = std::fs::canonicalize(dir) else {
            return Ok(false);
        };
        Ok(self.toplevel(&canon)?.as_deref() == Some(canon.as_path()))
    }

    /// The root must be a git top level; otherwise `not_a_git_repository`.
    pub fn check_root(&self, root: &Path) -> Result<(), Error> {
        if self.is_toplevel(root)? {
            Ok(())
        } else {
            Err(Error::new(
                Code::NotAGitRepository,
                format!("{} is not the top level of a git work tree", root.display()),
            ))
        }
    }

    /// The git top level of the working directory, canonicalized (for
    /// `init`). `not_a_git_repository` when there is none.
    pub fn find_toplevel(&self, cwd: &Path) -> Result<PathBuf, Error> {
        self.toplevel(cwd)?.ok_or_else(|| {
            Error::new(
                Code::NotAGitRepository,
                format!("{} is not inside a git work tree", cwd.display()),
            )
        })
    }

    /// `rev-parse --verify --quiet --end-of-options A^{commit}`. Failure is
    /// `invalid_range`.
    pub fn resolve_commit(&self, root: &Path, rev: &str) -> Result<ObjectId, Error> {
        let spec = format!("{rev}^{{commit}}");
        let out = self.run(
            root,
            &[
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("--quiet"),
                OsStr::new("--end-of-options"),
                OsStr::new(&spec),
            ],
        )?;
        let invalid = || {
            Error::new(
                Code::InvalidRange,
                format!("{rev:?} does not name a commit"),
            )
        };
        if !out.ok {
            return Err(invalid());
        }
        let text = String::from_utf8(out.stdout).map_err(|_| invalid())?;
        ObjectId::new(text.trim_end()).map_err(|_| invalid())
    }

    /// `merge-base <commit> HEAD`. Failure is `invalid_range`.
    pub fn merge_base(&self, root: &Path, commit: &ObjectId) -> Result<ObjectId, Error> {
        let out = self.run(
            root,
            &[
                OsStr::new("merge-base"),
                OsStr::new(commit.as_str()),
                OsStr::new("HEAD"),
            ],
        )?;
        let invalid = || {
            Error::new(
                Code::InvalidRange,
                format!("{commit} and HEAD have no merge base"),
            )
        };
        if !out.ok {
            return Err(invalid());
        }
        let text = String::from_utf8(out.stdout).map_err(|_| invalid())?;
        ObjectId::new(text.trim_end()).map_err(|_| invalid())
    }

    /// The base commit of a `--since` range (section 10.3). `A` is resolved
    /// with [`Git::resolve_commit`]; for `A...` the merge base of that
    /// commit and `HEAD` is taken.
    pub fn since_base(&self, root: &Path, range: &SinceRange) -> Result<ObjectId, Error> {
        match range {
            SinceRange::Commit(a) => self.resolve_commit(root, a),
            SinceRange::MergeBase(a) => {
                let a = self.resolve_commit(root, a)?;
                self.merge_base(root, &a)
            }
        }
    }

    /// The file list of section 10.4:
    /// `ls-files -z --cached --others --exclude-standard`, sorted, without
    /// duplicates, with every path that has a control character (or is not
    /// UTF-8) dropped.
    pub fn file_list(&self, dir: &Path) -> Result<Vec<String>, Error> {
        let out = self.run_ok(
            dir,
            &[
                OsStr::new("ls-files"),
                OsStr::new("-z"),
                OsStr::new("--cached"),
                OsStr::new("--others"),
                OsStr::new("--exclude-standard"),
            ],
        )?;
        let set: BTreeSet<String> = split_z(&out).filter_map(keep_path).collect();
        Ok(set.into_iter().collect())
    }

    /// The changed paths of section 10.3: `diff --name-only --no-renames
    /// --no-ext-diff --no-textconv -z <base> --` plus
    /// `ls-files -z --others --exclude-standard`.
    pub fn changed_paths(&self, root: &Path, base: &ObjectId) -> Result<BTreeSet<String>, Error> {
        let diff = self.run_ok(
            root,
            &[
                OsStr::new("diff"),
                OsStr::new("--name-only"),
                OsStr::new("--no-renames"),
                OsStr::new("--no-ext-diff"),
                OsStr::new("--no-textconv"),
                OsStr::new("-z"),
                OsStr::new(base.as_str()),
                OsStr::new("--"),
            ],
        )?;
        let others = self.run_ok(
            root,
            &[
                OsStr::new("ls-files"),
                OsStr::new("-z"),
                OsStr::new("--others"),
                OsStr::new("--exclude-standard"),
            ],
        )?;
        Ok(split_z(&diff)
            .chain(split_z(&others))
            .filter_map(keep_path)
            .collect())
    }

    /// `ls-tree -r -z <base>`, with no path arguments, filtered to `paths`
    /// (section 10.4, base entries).
    pub fn tree_entries(
        &self,
        root: &Path,
        base: &ObjectId,
        paths: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, TreeEntry>, Error> {
        let out = self.run_ok(
            root,
            &[
                OsStr::new("ls-tree"),
                OsStr::new("-r"),
                OsStr::new("-z"),
                OsStr::new(base.as_str()),
            ],
        )?;
        let mut entries = BTreeMap::new();
        for rec in split_z(&out) {
            let malformed = || {
                Error::new(
                    Code::GitFailed,
                    format!(
                        "git ls-tree printed an entry hibi cannot read: {:?}",
                        String::from_utf8_lossy(rec)
                    ),
                )
            };
            let tab = rec.iter().position(|&b| b == b'\t').ok_or_else(malformed)?;
            let (head, path) = (&rec[..tab], &rec[tab + 1..]);
            let Some(path) = keep_path(path) else {
                continue;
            };
            if !paths.contains(&path) {
                continue;
            }
            let head = std::str::from_utf8(head).map_err(|_| malformed())?;
            let mut fields = head.split(' ');
            let (Some(mode), Some(_kind), Some(id), None) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                return Err(malformed());
            };
            let id = ObjectId::new(id).map_err(|_| malformed())?;
            entries.insert(
                path,
                TreeEntry {
                    mode: TreeMode::parse(mode),
                    id,
                },
            );
        }
        Ok(entries)
    }

    /// `cat-file --batch` with `ids` on standard input, without `--filters`
    /// or `--textconv`. One answer per id, in order. A blob larger than
    /// `max_bytes` is [`Blob::TooLarge`] and its bytes are not kept.
    pub fn read_blobs(
        &self,
        root: &Path,
        ids: &[ObjectId],
        max_bytes: u64,
    ) -> Result<Vec<Blob>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let args = [OsStr::new("cat-file"), OsStr::new("--batch")];
        let failed = |what: String| {
            Error::new(
                Code::GitFailed,
                format!("{} in {}: {what}", Git::describe(&args), root.display()),
            )
        };
        let mut cmd = self.command(root, &args);
        cmd.stdin(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| failed(format!("could not be started: {e}")))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| failed("no standard input".into()))?;
        let mut input = String::new();
        for id in ids {
            input.push_str(id.as_str());
            input.push('\n');
        }
        // Write on another thread so that a full output pipe cannot block
        // the write.
        let writer = std::thread::spawn(move || {
            let r = stdin.write_all(input.as_bytes());
            drop(stdin);
            r
        });
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| failed("no standard output".into()))?;
        let mut reader = BufReader::new(stdout);
        let mut blobs = Vec::with_capacity(ids.len());
        let mut read_err = None;
        for id in ids {
            match read_batch_entry(&mut reader, id, max_bytes) {
                Ok(b) => blobs.push(b),
                Err(e) => {
                    read_err = Some(e);
                    break;
                }
            }
        }
        // Drain whatever is left so that git can exit, then collect it.
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
        let mut stderr = String::new();
        if let Some(mut s) = child.stderr.take() {
            let _ = s.read_to_string(&mut stderr);
        }
        let status = child
            .wait()
            .map_err(|e| failed(format!("could not be waited for: {e}")))?;
        let write_result = writer.join();
        if let Some(e) = read_err {
            return Err(failed(format!("{e}; {}", stderr.trim_end())));
        }
        if !status.success() {
            return Err(failed(stderr.trim_end().to_owned()));
        }
        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(failed(format!("writing standard input: {e}"))),
            Err(_) => return Err(failed("the writer thread panicked".into())),
        }
        Ok(blobs)
    }
}

/// Read one `cat-file --batch` answer: `<id> <type> <size>\n<bytes>\n` or
/// `<id> missing\n`.
fn read_batch_entry(
    reader: &mut impl BufRead,
    id: &ObjectId,
    max_bytes: u64,
) -> Result<Blob, String> {
    let mut header = Vec::new();
    reader
        .read_until(b'\n', &mut header)
        .map_err(|e| e.to_string())?;
    if header.pop() != Some(b'\n') {
        return Err("output ended early".into());
    }
    let header = String::from_utf8(header).map_err(|_| "header is not UTF-8".to_owned())?;
    let fields: Vec<&str> = header.split(' ').collect();
    match fields.as_slice() {
        [_, "missing"] | [_, "ambiguous"] => Ok(Blob::Missing),
        [got, kind, size] => {
            if *got != id.as_str() {
                return Err(format!("expected object {id}, got {got}"));
            }
            let size: u64 = size.parse().map_err(|_| format!("bad size {size:?}"))?;
            let keep = *kind == "blob" && size <= max_bytes;
            let mut content = Vec::new();
            let mut limited = reader.take(size);
            if keep {
                limited
                    .read_to_end(&mut content)
                    .map_err(|e| e.to_string())?;
            } else {
                std::io::copy(&mut limited, &mut std::io::sink()).map_err(|e| e.to_string())?;
            }
            let mut nl = [0u8; 1];
            reader.read_exact(&mut nl).map_err(|e| e.to_string())?;
            if nl[0] != b'\n' {
                return Err("object not followed by a newline".into());
            }
            Ok(if *kind != "blob" {
                Blob::NotBlob
            } else if !keep {
                Blob::TooLarge { size }
            } else if content.len() as u64 != size {
                return Err("object shorter than its size".into());
            } else {
                Blob::Content(content)
            })
        }
        _ => Err(format!("unexpected header {header:?}")),
    }
}

/// Split `-z` output into records, without the empty record after the last
/// NUL.
fn split_z(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split(|&b| b == 0).filter(|r| !r.is_empty())
}

/// A path git printed, as a `String`, or `None` when it is not UTF-8 or has
/// a character U+0000 to U+001F or U+007F (section 10.4, file list).
fn keep_path(bytes: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(bytes).ok()?;
    if s.chars()
        .any(|c| matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'))
    {
        return None;
    }
    Some(s.to_owned())
}

#[cfg(unix)]
fn bytes_to_path(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        let v = |s| parse_version(s);
        assert_eq!(
            v("git version 2.54.0 (Apple Git-157)\n"),
            Some(GitVersion {
                major: 2,
                minor: 54,
                patch: 0
            })
        );
        assert_eq!(
            v("git version 2.45"),
            Some(GitVersion {
                major: 2,
                minor: 45,
                patch: 0
            })
        );
        assert_eq!(
            v("git version 2.45.1.windows.1"),
            Some(GitVersion {
                major: 2,
                minor: 45,
                patch: 1
            })
        );
        assert_eq!(v("git version 2.46.0.rc0").map(|v| v.patch), Some(0));
        assert_eq!(v("git version 2"), None);
        assert_eq!(v("git version x.y.z"), None);
        assert_eq!(v("hub version 2.50.0"), None);
        assert_eq!(v(""), None);
    }

    #[test]
    fn version_classification() {
        let st = GitStatus::from_version_output("git", "git version 2.45.0\n");
        assert!(matches!(st, GitStatus::Ready(_)));
        let st = GitStatus::from_version_output("git", "git version 2.44.9\n");
        let e = st.require().unwrap_err();
        assert_eq!(e.code, Code::GitTooOld);
        assert!(e.message.contains("2.44.9") && e.message.contains("2.45.0"));
        assert!(st.git().is_none());
        let st = GitStatus::from_version_output("git", "garbage");
        assert_eq!(st.require().unwrap_err().code, Code::GitTooOld);
        let st = GitStatus::Missing {
            message: "x".into(),
        };
        assert_eq!(st.require().unwrap_err().code, Code::NotAGitRepository);
        assert!(st.too_old_message().is_none());
    }

    #[test]
    fn command_line_carries_the_hardening() {
        let git = Git {
            program: "git".into(),
            version: MIN_GIT,
        };
        let cmd = git.command(Path::new("/r"), &[OsStr::new("ls-files"), OsStr::new("-z")]);
        let args: Vec<&OsStr> = cmd.get_args().collect();
        let expected: Vec<&OsStr> = [
            "--no-pager",
            "-C",
            "/r",
            "-c",
            "core.quotePath=false",
            "-c",
            "safe.bareRepository=explicit",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "ls-files",
            "-z",
        ]
        .iter()
        .map(OsStr::new)
        .collect();
        assert_eq!(args, expected);
        let envs: BTreeMap<&OsStr, Option<&OsStr>> = cmd.get_envs().collect();
        for (k, v) in GIT_ENV {
            assert_eq!(envs.get(OsStr::new(k)), Some(&Some(OsStr::new(v))));
        }
    }

    #[test]
    fn ranges() {
        use SinceRange::*;
        assert_eq!(parse_range("main").unwrap(), Commit("main".into()));
        assert_eq!(parse_range("main..").unwrap(), Commit("main".into()));
        assert_eq!(parse_range("main..HEAD").unwrap(), Commit("main".into()));
        assert_eq!(parse_range("main...").unwrap(), MergeBase("main".into()));
        assert_eq!(
            parse_range("main...HEAD").unwrap(),
            MergeBase("main".into())
        );
        assert_eq!(parse_range("v1.2^").unwrap(), Commit("v1.2^".into()));
        for bad in [
            "",
            "..",
            "...",
            "..HEAD",
            "main..other",
            "main...other",
            "a..b..HEAD",
            "-x",
            "--all",
        ] {
            assert_eq!(
                parse_range(bad).unwrap_err().code,
                Code::InvalidRange,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn paths_with_controls_are_dropped() {
        assert_eq!(keep_path(b"a/b.md"), Some("a/b.md".into()));
        assert_eq!(keep_path(b"a\tb"), None);
        assert_eq!(keep_path(b"a\x7fb"), None);
        assert_eq!(keep_path(b"a\nb"), None);
        assert_eq!(keep_path(b"\xff"), None);
        assert_eq!(keep_path("é".as_bytes()), Some("é".into()));
    }

    #[cfg(any(feature = "file", feature = "cache"))]
    #[test]
    fn blob_ids() {
        // `git hash-object` of the empty file and of "hello\n".
        assert_eq!(blob_id(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(
            blob_id(b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
    }
}
