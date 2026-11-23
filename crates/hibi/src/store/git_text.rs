//! `GitTextStore`: the sharded JSON-lines store under `.hibi/` (spec
//! section 12).
//!
//! Layout (section 12.1): 16 claims shards and 16 marks shards, one
//! canonical JSON record per line. Readers take no lock and accept the
//! unsorted and duplicated lines a union merge produces (section 12.3).
//! [`GitTextStore::append`] follows the write protocol of section 12.4:
//! advisory lock, re-read, write each changed shard to an exclusive temp
//! file, `fsync`, rename, directory `fsync`, claims shards before marks
//! shards.
//!
//! Also here: root discovery and the `legacy_store` refusal (section 12.5),
//! and the store part of `hibi init` ([`init`], section 15.3).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;

use crate::config::nofollow;
use crate::error::{Code, Error};
use crate::model::{Claim, ClaimId, Locator, Mark, Retirement};
use crate::store::{Batch, Ledger, ORPHAN_RECORD, Store, StoreWarning, UNKNOWN_REPLACEMENT};

/// The largest shard file, in bytes (section 12.3).
pub const MAX_SHARD_BYTES: u64 = 67_108_864;

/// The largest total size of the 32 shard files, in bytes (256 MiB). A
/// loaded store takes several times its size on disk in memory, and git can
/// deliver every shard at [`MAX_SHARD_BYTES`]; this cap bounds what one
/// command reads. It is checked from the file sizes before any shard is
/// read.
pub const MAX_STORE_BYTES: u64 = 268_435_456;

/// How long `append` waits for the lock (section 12.4).
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a new temp-file name is chosen after the first one exists
/// (section 12.4 step 3d).
const TEMP_RETRIES: usize = 8;

/// The store directory under the root.
pub const HIBI_DIR: &str = ".hibi";

/// The directory of a hibi 0.x store, refused with `legacy_store`.
pub const LEGACY_DIR: &str = ".claims";

const SHARD_DIGITS: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];

/// `.hibi/.gitattributes` (section 12.1).
pub const GITATTRIBUTES: &str = "claims/*.jsonl text eol=lf merge=union linguist-generated\nmarks/*.jsonl text eol=lf merge=union linguist-generated\n";

/// `.hibi/.ignore` (section 12.1).
pub const IGNORE: &str = "claims/\nmarks/\n";

/// `.hibi/.gitignore` (section 12.1).
pub const GITIGNORE: &str = ".lock\n*.tmp-*\n";

/// The `hibi.toml` that `init` writes: `version = 1` and commented
/// examples.
pub const HIBI_TOML_TEMPLATE: &str = r#"# hibi configuration for this repository.
version = 1

# Sibling repositories that claims refer to as repo:<alias>//<path>.
# The value is only a suggested location. Each user allows the directory
# in their own user config with a [[repo]] entry.
# [repos]
# api = "../api"

# External schemes. The provider command for a scheme is registered in
# each user's own config, never here.
# [schemes.archctl]
# description = "Architecture facts served by archctl"

# Units that must be covered by a claim.
# [[scope]]
# name = "docs"
# resource = "file:docs/**/*.md"
# kinds = ["sentence", "list-item", "table-row"]
# exclude = [
#   { resource = "file:docs/changelog.md", reason = "release notes, not claims" },
# ]
"#;

// ---------------------------------------------------------------------------
// Shards
// ---------------------------------------------------------------------------

/// The shard of a claim (section 12.2): the high four bits of byte 0 of
/// BLAKE3 over the UTF-8 bytes of `subject.loc`, from 0 to 15.
pub fn shard_of(subject_loc: &Locator) -> u8 {
    blake3::hash(subject_loc.as_str().as_bytes()).as_bytes()[0] >> 4
}

/// The shard digit `0` to `f`.
pub fn shard_digit(shard: u8) -> char {
    SHARD_DIGITS[usize::from(shard & 0xf)]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ShardKind {
    Claims,
    Marks,
}

impl ShardKind {
    fn dir(self) -> &'static str {
        match self {
            ShardKind::Claims => "claims",
            ShardKind::Marks => "marks",
        }
    }
}

/// `claims/3.jsonl` and the like, relative to `.hibi`.
fn shard_rel(kind: ShardKind, shard: u8) -> String {
    format!("{}/{}.jsonl", kind.dir(), shard_digit(shard))
}

/// The 32 shard files in snapshot order: claims 0 to f, then marks 0 to f.
fn all_shards() -> impl Iterator<Item = (ShardKind, u8)> {
    [ShardKind::Claims, ShardKind::Marks]
        .into_iter()
        .flat_map(|k| (0u8..16).map(move |s| (k, s)))
}

fn corrupt(message: impl Into<String>) -> Error {
    Error::new(Code::StoreCorrupt, message)
}

// ---------------------------------------------------------------------------
// Fault injection (tests only)
// ---------------------------------------------------------------------------

/// The steps of the write protocol at which a test can simulate a crash.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Locked,
    TempCreated,
    TempWritten,
    TempSynced,
    Renamed,
    DirSynced,
}

#[cfg(test)]
pub(crate) mod fault {
    use super::Step;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    pub(crate) const CRASH: &str = "simulated crash";

    thread_local! {
        /// Crash at the `n`-th (0-based) time `step` is reached.
        static PLAN: RefCell<Option<(Step, usize)>> = const { RefCell::new(None) };
        /// Temp-file suffixes to use before random ones.
        static NAMES: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
    }

    pub(crate) fn crash_at(step: Step, n: usize) {
        PLAN.with(|p| *p.borrow_mut() = Some((step, n)));
    }

    pub(crate) fn clear() {
        PLAN.with(|p| *p.borrow_mut() = None);
        NAMES.with(|n| n.borrow_mut().clear());
    }

    pub(crate) fn force_names(names: &[&str]) {
        NAMES.with(|n| n.borrow_mut().extend(names.iter().map(|s| s.to_string())));
    }

    pub(crate) fn next_name() -> Option<String> {
        NAMES.with(|n| n.borrow_mut().pop_front())
    }

    /// True when the plan says to crash here.
    pub(crate) fn hit(step: Step) -> bool {
        PLAN.with(|p| {
            let mut p = p.borrow_mut();
            match *p {
                Some((s, 0)) if s == step => {
                    *p = None;
                    true
                }
                Some((s, n)) if s == step => {
                    *p = Some((s, n - 1));
                    false
                }
                _ => false,
            }
        })
    }
}

/// Simulate a crash at this step: return at once, with no cleanup. The
/// lock is released as the operating system releases it for a killed
/// process.
macro_rules! crash_point {
    ($step:expr) => {
        #[cfg(test)]
        if fault::hit($step) {
            return Err(Error::internal(fault::CRASH));
        }
    };
}

// ---------------------------------------------------------------------------
// Root discovery and refusals (section 12.5)
// ---------------------------------------------------------------------------

fn legacy_error() -> Error {
    Error::new(
        Code::LegacyStore,
        "this repository has a hibi 0.x store in .claims/, which this version does not read",
    )
}

fn has_legacy(dir: &Path) -> bool {
    fs::metadata(dir.join(LEGACY_DIR)).is_ok_and(|m| m.is_dir())
}

/// Fail with `legacy_store` when `root` contains a directory `.claims/`
/// (section 12.5). Reads nothing else and changes nothing.
pub fn check_legacy(root: &Path) -> Result<(), Error> {
    if has_legacy(root) {
        Err(legacy_error())
    } else {
        Ok(())
    }
}

/// The root for a run started in `cwd` (section 12.5): the nearest ancestor
/// of `cwd`, itself included, that contains `.hibi/hibi.toml`. None found
/// is `not_initialized`.
///
/// A directory on the way up that contains `.claims/` is also a candidate
/// root, so that running in a hibi 0.x repository gives `legacy_store`
/// rather than `not_initialized`. A candidate root with `.claims/` fails
/// with `legacy_store`.
pub fn discover_root(cwd: &Path) -> Result<PathBuf, Error> {
    let start = std::path::absolute(cwd).map_err(|e| Error::io(cwd.display(), &e))?;
    for dir in start.ancestors() {
        let has_store = fs::symlink_metadata(dir.join(HIBI_DIR).join("hibi.toml")).is_ok();
        if has_legacy(dir) {
            return Err(legacy_error());
        }
        if has_store {
            return Ok(dir.to_path_buf());
        }
    }
    Err(Error::new(
        Code::NotInitialized,
        format!(
            "no .hibi/hibi.toml in {} or any directory above it",
            start.display()
        ),
    ))
}

/// The root given with `--root` (section 12.5): `legacy_store` when it
/// contains `.claims/`, `not_initialized` when it has no `.hibi/hibi.toml`.
pub fn check_root(root: &Path) -> Result<PathBuf, Error> {
    let root = std::path::absolute(root).map_err(|e| Error::io(root.display(), &e))?;
    check_legacy(&root)?;
    if fs::symlink_metadata(root.join(HIBI_DIR).join("hibi.toml")).is_err() {
        return Err(Error::new(
            Code::NotInitialized,
            format!("{} has no .hibi/hibi.toml", root.display()),
        ));
    }
    Ok(root)
}

// ---------------------------------------------------------------------------
// Symbolic-link checks and reading
// ---------------------------------------------------------------------------

/// What a path is, by `symlink_metadata`.
enum Entry {
    Missing,
    Link,
    Dir,
    File(u64),
    Other,
}

fn entry(path: &Path, rel: &str) -> Result<Entry, Error> {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            let t = m.file_type();
            Ok(if t.is_symlink() {
                Entry::Link
            } else if t.is_dir() {
                Entry::Dir
            } else if t.is_file() {
                Entry::File(m.len())
            } else {
                Entry::Other
            })
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Entry::Missing),
        Err(e) => Err(Error::io(format!("{HIBI_DIR}/{rel}"), &e)),
    }
}

fn link_error(rel: &str) -> Error {
    corrupt(format!("{rel} is a symbolic link"))
}

/// What [`scan`] saw besides the things it refuses.
struct Scan {
    /// `.hibi` is a directory.
    hibi_exists: bool,
    /// The first of `claims` and `marks` that does not exist.
    missing_dir: Option<ShardKind>,
    /// The sum of the sizes of the shard files, from their metadata.
    shard_bytes: u64,
}

/// The symbolic-link check of section 12.3: `.hibi`, `hibi.toml`, `claims`,
/// `marks`, every shard file and `.lock` must not be links; the
/// directories must be directories and the files regular files. Reads
/// metadata only.
fn scan(hibi: &Path) -> Result<Scan, Error> {
    let mut found = Scan {
        hibi_exists: false,
        missing_dir: None,
        shard_bytes: 0,
    };
    match entry(hibi, "")? {
        Entry::Link => return Err(link_error(HIBI_DIR)),
        Entry::Missing => {}
        Entry::Dir => found.hibi_exists = true,
        _ => return Err(corrupt(format!("{HIBI_DIR} is not a directory"))),
    }
    for f in ["hibi.toml", ".lock"] {
        match entry(&hibi.join(f), f)? {
            Entry::Link => return Err(link_error(&format!("{HIBI_DIR}/{f}"))),
            Entry::Missing | Entry::File(_) => {}
            _ => return Err(corrupt(format!("{HIBI_DIR}/{f} is not a regular file"))),
        }
    }
    for kind in [ShardKind::Claims, ShardKind::Marks] {
        match entry(&hibi.join(kind.dir()), kind.dir())? {
            Entry::Link => return Err(link_error(&format!("{HIBI_DIR}/{}", kind.dir()))),
            Entry::Missing => {
                found.missing_dir.get_or_insert(kind);
                continue;
            }
            Entry::Dir => {}
            _ => {
                return Err(corrupt(format!(
                    "{HIBI_DIR}/{} is not a directory",
                    kind.dir()
                )));
            }
        }
        for s in 0..16 {
            let rel = shard_rel(kind, s);
            match entry(&hibi.join(&rel), &rel)? {
                Entry::Link => return Err(link_error(&format!("{HIBI_DIR}/{rel}"))),
                Entry::Missing => {}
                Entry::File(len) => found.shard_bytes = found.shard_bytes.saturating_add(len),
                _ => return Err(corrupt(format!("{HIBI_DIR}/{rel} is not a regular file"))),
            }
        }
    }
    Ok(found)
}

/// The checks of [`scan`], for `init`, which runs before the store exists.
fn check_links(hibi: &Path) -> Result<(), Error> {
    scan(hibi).map(|_| ())
}

/// The checks every operation on an existing store makes before it reads a
/// shard or takes the lock: the checks of [`scan`]; then, under an existing
/// `.hibi`, both shard directories exist; then the shard files together
/// hold at most [`MAX_STORE_BYTES`]. Reads metadata only.
///
/// A missing shard file is an empty shard (section 12.3). A missing shard
/// directory is refused: `append` could not write into it, and a store
/// whose records were removed with their directory would otherwise read as
/// a store without records.
fn check_store(hibi: &Path) -> Result<(), Error> {
    let found = scan(hibi)?;
    if found.hibi_exists
        && let Some(kind) = found.missing_dir
    {
        let dir = format!("{HIBI_DIR}/{}", kind.dir());
        return Err(
            corrupt(format!("the directory {dir} is missing")).with_remediation(format!(
                "Look first: git status -- {dir} shows what git knows of the directory. Restoring it with git checkout -- {dir} brings back the committed records only; records that were written and never committed are not in git and stay lost. To start that directory empty instead, create it again with mkdir {dir}."
            )),
        );
    }
    if found.shard_bytes > MAX_STORE_BYTES {
        let (claims, marks) = (ShardKind::Claims.dir(), ShardKind::Marks.dir());
        return Err(corrupt(format!(
            "the shard files under {HIBI_DIR}/{claims} and {HIBI_DIR}/{marks} hold {} bytes in total, more than the limit of {MAX_STORE_BYTES} bytes",
            found.shard_bytes
        ))
        .with_remediation(format!(
            "Look first: git diff --stat -- {HIBI_DIR}/{claims} {HIBI_DIR}/{marks} shows which shard files grew. Restoring them from a commit in which they are smaller, for example with git checkout <commit> -- {HIBI_DIR}/{claims} {HIBI_DIR}/{marks}, discards every record written since that commit, committed or not, so keep the lines you need before you do."
        )));
    }
    Ok(())
}

/// The size check of a shard (section 12.3), for a length taken from the
/// file's metadata and for the count of bytes read from it. This is the
/// only comparison with [`MAX_SHARD_BYTES`].
fn check_shard_len(shown: &str, len: u64) -> Result<(), Error> {
    if len > MAX_SHARD_BYTES {
        return Err(corrupt(format!(
            "{shown} is longer than {MAX_SHARD_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Open one shard for reading: `None` when missing. A link, a non-file or a
/// file longer than [`MAX_SHARD_BYTES`] is `store_corrupt`. Opened without
/// following a link. The reader yields at most `MAX_SHARD_BYTES` + 1 bytes.
fn open_shard(hibi: &Path, rel: &str) -> Result<Option<io::Take<File>>, Error> {
    let path = hibi.join(rel);
    let shown = format!("{HIBI_DIR}/{rel}");
    match entry(&path, rel)? {
        Entry::Missing => return Ok(None),
        Entry::Link => return Err(link_error(&shown)),
        Entry::File(len) => check_shard_len(&shown, len)?,
        _ => return Err(corrupt(format!("{shown} is not a regular file"))),
    }
    let file = match nofollow::apply(OpenOptions::new().read(true)).open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) if nofollow::is_link_error(&e) => return Err(link_error(&shown)),
        Err(e) => return Err(Error::io(&shown, &e)),
    };
    let meta = file.metadata().map_err(|e| Error::io(&shown, &e))?;
    if !meta.is_file() {
        return Err(corrupt(format!("{shown} is not a regular file")));
    }
    Ok(Some(file.take(MAX_SHARD_BYTES + 1)))
}

/// The bytes of one shard, empty when it is missing ([`open_shard`]).
fn read_shard(hibi: &Path, rel: &str) -> Result<Vec<u8>, Error> {
    let shown = format!("{HIBI_DIR}/{rel}");
    let mut buf = Vec::new();
    if let Some(mut file) = open_shard(hibi, rel)? {
        file.read_to_end(&mut buf)
            .map_err(|e| Error::io(&shown, &e))?;
        check_shard_len(&shown, buf.len() as u64)?;
    }
    Ok(buf)
}

/// A record kind that a shard holds one per line.
trait ShardRecord: Sized {
    fn id(&self) -> &str;

    fn canonical(&self) -> Result<Vec<u8>, Error>;

    /// The record of `line`, or `None` when the line does not parse.
    fn parse_direct(line: &[u8]) -> Option<Self>;

    /// The record of a line that parsed as JSON to `value`, whose `id`
    /// member is `id` (empty when there is none). The error is the text
    /// after "line n: ".
    fn parse_value(id: &str, value: serde_json::Value) -> Result<Self, String>;

    /// Give back the spare capacity of the record's lists. A list of one
    /// element is deserialized with room for four.
    fn shrink(&mut self);
}

fn parse_as<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

impl ShardRecord for Claim {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn canonical(&self) -> Result<Vec<u8>, Error> {
        Claim::canonical(self)
    }

    fn parse_direct(line: &[u8]) -> Option<Self> {
        serde_json::from_slice(line).ok()
    }

    fn parse_value(_id: &str, value: serde_json::Value) -> Result<Self, String> {
        parse_as(value)
    }

    fn shrink(&mut self) {
        self.evidence.shrink_to_fit();
    }
}

/// A line of a marks shard.
#[derive(Debug, Clone)]
enum MarkLine {
    Mark(Mark),
    Retirement(Retirement),
}

/// The `id` member of a line, read without building the other members.
#[derive(serde::Deserialize)]
struct IdOnly {
    id: Option<String>,
}

impl ShardRecord for MarkLine {
    fn id(&self) -> &str {
        match self {
            MarkLine::Mark(m) => m.id.as_str(),
            MarkLine::Retirement(r) => r.id.as_str(),
        }
    }

    fn canonical(&self) -> Result<Vec<u8>, Error> {
        match self {
            MarkLine::Mark(m) => m.canonical(),
            MarkLine::Retirement(r) => r.canonical(),
        }
    }

    fn parse_direct(line: &[u8]) -> Option<Self> {
        let id = serde_json::from_slice::<IdOnly>(line).ok()?.id?;
        if id.starts_with("m1-") {
            serde_json::from_slice(line).ok().map(MarkLine::Mark)
        } else if id.starts_with("r1-") {
            serde_json::from_slice(line).ok().map(MarkLine::Retirement)
        } else {
            None
        }
    }

    fn parse_value(id: &str, value: serde_json::Value) -> Result<Self, String> {
        if id.starts_with("m1-") {
            parse_as(value).map(MarkLine::Mark)
        } else if id.starts_with("r1-") {
            parse_as(value).map(MarkLine::Retirement)
        } else {
            Err(format!("id {id:?} is neither a mark nor a retirement id"))
        }
    }

    fn shrink(&mut self) {
        if let MarkLine::Mark(m) = self {
            m.sides.shrink_to_fit();
        }
    }
}

/// Parse one line (without its newline) by the rules of section 12.3.
/// `bad` makes the `store_corrupt` error that names the file and the line.
///
/// A line is accepted when it parses as its record type, which includes
/// the key set and the derived id, and its bytes are the canonical form of
/// the record. The first attempt builds the record straight from the bytes.
/// When that attempt does not end in an accepted record, the line is parsed
/// again by way of a JSON value, and that second parse alone decides
/// between accepting the line and the error to report. The second parse is
/// the definition; the first is the same decision for every line it
/// accepts, because such a line is the canonical form of the record it
/// produced.
fn parse_line<T: ShardRecord>(line: &[u8], bad: &dyn Fn(&str) -> Error) -> Result<T, Error> {
    if let Some(record) = T::parse_direct(line)
        && record.canonical().is_ok_and(|c| c == line)
    {
        return Ok(record);
    }
    let value: serde_json::Value =
        serde_json::from_slice(line).map_err(|e| bad(&format!("not valid JSON: {e}")))?;
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();
    let record = T::parse_value(&id, value).map_err(|e| bad(&e))?;
    if record.canonical()? != line {
        return Err(bad("the line is not the canonical form of its record"));
    }
    Ok(record)
}

/// One shard as read: its records sorted by id, one per id.
#[derive(Debug)]
struct ShardData<T> {
    /// The file held its lines in ascending id order with no id twice (or
    /// was empty or missing). Every line is canonical, so the file's bytes
    /// are then exactly what a writer renders from `records`.
    sorted: bool,
    records: Vec<T>,
}

impl<T> Default for ShardData<T> {
    fn default() -> Self {
        ShardData {
            sorted: true,
            records: Vec::new(),
        }
    }
}

/// Read and parse one shard by the rules of section 12.3, except the checks
/// that need other shards. The file is read one line at a time; neither its
/// whole text nor a second copy of its lines is kept.
fn read_records<T: ShardRecord>(hibi: &Path, rel: &str) -> Result<ShardData<T>, Error> {
    let shown = format!("{HIBI_DIR}/{rel}");
    let mut data = ShardData::default();
    let Some(file) = open_shard(hibi, rel)? else {
        return Ok(data);
    };
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut bytes = 0u64;
    let mut n = 0usize;
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| Error::io(&shown, &e))?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        check_shard_len(&shown, bytes)?;
        n += 1;
        if line.pop() != Some(b'\n') {
            return Err(corrupt(format!(
                "{shown} line {n} does not end with a newline"
            )));
        }
        let mut record: T = parse_line(&line, &|why| corrupt(format!("{shown} line {n}: {why}")))?;
        record.shrink();
        if data
            .records
            .last()
            .is_some_and(|prev| prev.id() >= record.id())
        {
            data.sorted = false;
        }
        data.records.push(record);
    }
    if !data.sorted {
        // Two lines with one id are the same bytes: the id is derived from
        // the body and both lines are canonical.
        data.records.sort_by(|a, b| a.id().cmp(b.id()));
        data.records.dedup_by(|b, a| a.id() == b.id());
    }
    Ok(data)
}

/// The whole store as read, with every check of section 12.3 applied.
#[derive(Debug, Default)]
struct Snapshot {
    claims: [ShardData<Claim>; 16],
    /// Every mark and retirement on disk, including those in `orphans`.
    marks: [ShardData<MarkLine>; 16],
    /// Ids of the marks and retirements whose claim is not in the store.
    orphans: BTreeSet<String>,
    warnings: Vec<StoreWarning>,
}

/// Read all 32 shards and apply the checks that need more than one. Marks
/// shards are read before claims shards: a writer renames claims shards
/// first, so a mark seen here has its claim on disk by the time the claims
/// shards are read, and a concurrent append cannot produce a spurious
/// `orphan_record`.
fn read_store(hibi: &Path) -> Result<Snapshot, Error> {
    check_store(hibi)?;
    let mut snap = Snapshot::default();
    for s in 0..16u8 {
        snap.marks[usize::from(s)] = read_records(hibi, &shard_rel(ShardKind::Marks, s))?;
    }
    for s in 0..16u8 {
        snap.claims[usize::from(s)] = read_records(hibi, &shard_rel(ShardKind::Claims, s))?;
    }
    let mut claims: HashMap<&str, &Claim> = HashMap::new();
    for (s, data) in snap.claims.iter().enumerate() {
        for c in &data.records {
            let want = shard_of(&c.subject.loc);
            if usize::from(want) != s {
                return Err(corrupt(format!(
                    "claim {} is in {HIBI_DIR}/{} but belongs in {HIBI_DIR}/{}",
                    c.id,
                    shard_rel(ShardKind::Claims, s as u8),
                    shard_rel(ShardKind::Claims, want)
                )));
            }
            claims.insert(c.id.as_str(), c);
        }
    }
    let mut orphans = BTreeSet::new();
    let mut warnings = Vec::new();
    for (s, data) in snap.marks.iter().enumerate() {
        let rel = shard_rel(ShardKind::Marks, s as u8);
        for rec in &data.records {
            let id = rec.id();
            let (claim_id, what) = match rec {
                MarkLine::Mark(m) => (m.claim.as_str(), "mark"),
                MarkLine::Retirement(r) => (r.retires.as_str(), "retirement"),
            };
            let Some(claim) = claims.get(claim_id) else {
                warnings.push(StoreWarning::new(
                    ORPHAN_RECORD,
                    format!(
                        "{what} {id} in {HIBI_DIR}/{rel} names claim {claim_id}, which is not in the store; the {what} is ignored"
                    ),
                ));
                orphans.insert(id.to_owned());
                continue;
            };
            let want = shard_of(&claim.subject.loc);
            if usize::from(want) != s {
                return Err(corrupt(format!(
                    "{what} {id} is in {HIBI_DIR}/{rel} but belongs in {HIBI_DIR}/{}",
                    shard_rel(ShardKind::Marks, want)
                )));
            }
            match rec {
                MarkLine::Mark(m) => {
                    m.check_against(claim)
                        .map_err(|e| corrupt(format!("{HIBI_DIR}/{rel}: {}", e.message)))?;
                }
                MarkLine::Retirement(r) => {
                    if let Some(rep) = &r.replaced_by
                        && !claims.contains_key(rep.as_str())
                    {
                        warnings.push(StoreWarning::new(
                            UNKNOWN_REPLACEMENT,
                            format!(
                                "retirement {id} names replacement claim {rep}, which is not in the store"
                            ),
                        ));
                    }
                }
            }
        }
    }
    drop(claims);
    snap.orphans = orphans;
    snap.warnings = warnings;
    Ok(snap)
}

impl Snapshot {
    /// The ledger: every claim, and every mark and retirement whose claim
    /// is in the store, each list sorted by id. The records are moved, not
    /// copied.
    fn into_ledger(self) -> Ledger {
        // Start from the largest shard's list so that its records stay
        // where they are.
        let mut parts: Vec<Vec<Claim>> = self.claims.into_iter().map(|d| d.records).collect();
        let largest = (0..parts.len())
            .max_by_key(|&i| parts[i].len())
            .unwrap_or(0);
        let mut claims = parts.swap_remove(largest);
        for part in parts {
            claims.extend(part);
        }
        claims.sort_unstable_by(|a, b| a.id.cmp(&b.id));

        let mut marks = Vec::new();
        let mut retirements = Vec::new();
        for data in self.marks {
            for rec in data.records {
                if self.orphans.contains(rec.id()) {
                    continue;
                }
                match rec {
                    MarkLine::Mark(m) => marks.push(m),
                    MarkLine::Retirement(r) => retirements.push(r),
                }
            }
        }
        marks.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        retirements.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        Ledger {
            claims,
            marks,
            retirements,
            warnings: self.warnings,
        }
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Open `.hibi/.lock` without following a link and take an exclusive
/// advisory lock, retrying for up to [`LOCK_TIMEOUT`] (section 12.4 step 1).
fn acquire_lock(hibi: &Path) -> Result<File, Error> {
    let path = hibi.join(".lock");
    let shown = format!("{HIBI_DIR}/.lock");
    let file = nofollow::apply(OpenOptions::new().read(true).write(true).create(true))
        .open(&path)
        .map_err(|e| {
            if nofollow::is_link_error(&e) {
                link_error(&shown)
            } else {
                Error::io(&shown, &e)
            }
        })?;
    let deadline = Instant::now() + LOCK_TIMEOUT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(Error::new(
                        Code::StoreLocked,
                        format!(
                            "another process held {shown} for {} seconds",
                            LOCK_TIMEOUT.as_secs()
                        ),
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(fs::TryLockError::Error(e)) => return Err(Error::io(&shown, &e)),
        }
    }
}

/// 16 random hex digits for a temp-file name.
fn random_suffix() -> String {
    #[cfg(test)]
    if let Some(name) = fault::next_name() {
        return name;
    }
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // `RandomState` is seeded from the operating system's random source.
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u32(std::process::id());
    if let Ok(d) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(d.as_nanos());
    }
    format!("{:016x}", h.finish())
}

#[cfg(unix)]
fn sync_dir(dir: &Path, shown: &str) -> Result<(), Error> {
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| Error::io(shown, &e))
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path, _shown: &str) -> Result<(), Error> {
    Ok(())
}

/// Section 12.4 step 3d: write `bytes` to a new exclusive temp file in the
/// shard's directory, `fsync`, rename over the shard, `fsync` the
/// directory.
fn replace_shard(hibi: &Path, kind: ShardKind, shard: u8, bytes: &[u8]) -> Result<(), Error> {
    let dir = hibi.join(kind.dir());
    let name = format!("{}.jsonl", shard_digit(shard));
    let shown = format!("{HIBI_DIR}/{}/{name}", kind.dir());
    let mut created = None;
    for _ in 0..=TEMP_RETRIES {
        let tmp = dir.join(format!("{name}.tmp-{}", random_suffix()));
        match nofollow::apply(OpenOptions::new().write(true).create_new(true)).open(&tmp) {
            Ok(f) => {
                created = Some((f, tmp));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists || nofollow::is_link_error(&e) => {
                continue;
            }
            Err(e) => return Err(Error::io(format!("{shown} temp file"), &e)),
        }
    }
    let Some((mut file, tmp)) = created else {
        return Err(Error::new(
            Code::IoError,
            format!(
                "{shown}: {} temp file names in a row already existed",
                TEMP_RETRIES + 1
            ),
        ));
    };
    crash_point!(Step::TempCreated);
    let cleanup = |e: io::Error| {
        let _ = fs::remove_file(&tmp);
        Error::io(&shown, &e)
    };
    file.write_all(bytes).map_err(cleanup)?;
    crash_point!(Step::TempWritten);
    file.sync_all().map_err(cleanup)?;
    crash_point!(Step::TempSynced);
    drop(file);
    fs::rename(&tmp, dir.join(&name)).map_err(cleanup)?;
    crash_point!(Step::Renamed);
    sync_dir(&dir, &format!("{HIBI_DIR}/{}", kind.dir()))?;
    crash_point!(Step::DirSynced);
    Ok(())
}

/// Section 12.4 step 3 for one shard: add the lines of `new` (id and
/// canonical line) whose id the shard does not hold, and write the shard
/// sorted by id, one canonical line each, each followed by a newline. A
/// record already in the shard is kept. Nothing is written when the result
/// would equal the file's current bytes.
fn merge_shard<T: ShardRecord>(
    current: &ShardData<T>,
    new: &[(String, Vec<u8>)],
) -> Result<Option<Vec<u8>>, Error> {
    let mut added: BTreeMap<&str, &[u8]> = BTreeMap::new();
    for (id, line) in new {
        let stored = current
            .records
            .binary_search_by(|r| r.id().cmp(id.as_str()))
            .is_ok();
        if !stored {
            added.entry(id.as_str()).or_insert(line.as_slice());
        }
    }
    if added.is_empty() && current.sorted {
        return Ok(None);
    }
    let mut out = Vec::new();
    let mut push = |line: &[u8]| {
        out.extend_from_slice(line);
        out.push(b'\n');
    };
    let mut added = added.into_iter().peekable();
    for record in &current.records {
        while let Some((_, line)) = added.next_if(|(id, _)| *id < record.id()) {
            push(line);
        }
        push(&record.canonical()?);
    }
    for (_, line) in added {
        push(line);
    }
    Ok(Some(out))
}

/// The size limits a write must keep, so that a write can never produce a
/// store that every later read refuses (section 12.3).
#[derive(Debug, Clone, Copy)]
struct Caps {
    shard: u64,
    store: u64,
}

const CAPS: Caps = Caps {
    shard: MAX_SHARD_BYTES,
    store: MAX_STORE_BYTES,
};

/// `store_full` when a shard to be written would be longer than the shard
/// limit, or the store after the write would be larger than the store
/// limit. `total` is the size of all shard files now; `writes` holds, per
/// shard to be written, its name, its size now and its size after.
fn check_room(caps: Caps, total: u64, writes: &[(String, u64, u64)]) -> Result<(), Error> {
    for (shown, _, after) in writes {
        if *after > caps.shard {
            return Err(Error::new(
                Code::StoreFull,
                format!(
                    "the write would make {shown} {after} bytes long, more than the limit of {} bytes for one shard",
                    caps.shard
                ),
            ));
        }
    }
    let before: u64 = writes.iter().map(|(_, now, _)| *now).sum();
    let after: u64 = writes.iter().map(|(_, _, after)| *after).sum();
    let new_total = total.saturating_sub(before).saturating_add(after);
    if new_total > caps.store {
        return Err(Error::new(
            Code::StoreFull,
            format!(
                "the write would make the shard files hold {new_total} bytes in total, more than the limit of {} bytes for the store (they hold {total} bytes now)",
                caps.store
            ),
        ));
    }
    Ok(())
}

/// Check every record of a batch before anything is locked or written:
/// each id is the derived id and each record keeps the rules of section 3.
/// Failure is `internal`.
fn verify_records(batch: &Batch) -> Result<(), Error> {
    batch.verify_ids()?;
    let internal = |e: Error| Error::internal(format!("append: {}", e.message));
    for c in &batch.claims {
        c.validate().map_err(internal)?;
    }
    for m in &batch.marks {
        m.validate().map_err(internal)?;
    }
    for r in &batch.retirements {
        r.validate().map_err(internal)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// The store of section 12 under `<root>/.hibi`.
#[derive(Debug, Clone)]
pub struct GitTextStore {
    root: PathBuf,
    hibi: PathBuf,
}

impl GitTextStore {
    /// The store of `root`. Fails with `legacy_store` when the root contains
    /// `.claims/`. Reads nothing else; every operation applies the
    /// symbolic-link checks of section 12.3 itself.
    pub fn open(root: &Path) -> Result<GitTextStore, Error> {
        check_legacy(root)?;
        Ok(GitTextStore {
            root: root.to_path_buf(),
            hibi: root.join(HIBI_DIR),
        })
    }

    /// The root this store belongs to.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Store for GitTextStore {
    fn load(&self) -> Result<Ledger, Error> {
        Ok(read_store(&self.hibi)?.into_ledger())
    }

    fn append(&self, batch: &Batch) -> Result<(), Error> {
        self.append_within(batch, CAPS)
    }

    fn snapshot_id(&self) -> Result<String, Error> {
        self.snapshot()
    }
}

impl GitTextStore {
    /// [`Store::append`] with the size limits given, so that the refusal
    /// can be tested without writing a store of the real limits.
    fn append_within(&self, batch: &Batch, caps: Caps) -> Result<(), Error> {
        verify_records(batch)?;
        // Step 1: the checks that read metadata only, then the lock.
        check_store(&self.hibi)?;
        let _lock = acquire_lock(&self.hibi)?;
        crash_point!(Step::Locked);
        // Re-read under the lock. Every shard passes section 12.3 or the
        // append fails before writing.
        let snap = read_store(&self.hibi)?;
        let stored: HashMap<&str, &Claim> = snap
            .claims
            .iter()
            .flat_map(|d| &d.records)
            .map(|c| (c.id.as_str(), c))
            .collect();
        batch.verify_claims(stored.values().map(|c| &c.id))?;
        let claim_of = |id: &ClaimId| -> Option<&Claim> {
            stored
                .get(id.as_str())
                .copied()
                .or_else(|| batch.claims.iter().find(|c| &c.id == id))
        };
        for m in &batch.marks {
            let claim = claim_of(&m.claim)
                .ok_or_else(|| Error::internal(format!("mark {} has no claim", m.id)))?;
            m.check_against(claim)
                .map_err(|e| Error::internal(format!("append: {}", e.message)))?;
        }

        // Step 2: group the batch by shard file.
        let mut new_claims: BTreeMap<u8, Vec<(String, Vec<u8>)>> = BTreeMap::new();
        let mut new_marks: BTreeMap<u8, Vec<(String, Vec<u8>)>> = BTreeMap::new();
        for c in &batch.claims {
            new_claims
                .entry(shard_of(&c.subject.loc))
                .or_default()
                .push((c.id.as_str().to_owned(), c.canonical()?));
        }
        for m in &batch.marks {
            let claim = claim_of(&m.claim)
                .ok_or_else(|| Error::internal(format!("mark {} has no claim", m.id)))?;
            new_marks
                .entry(shard_of(&claim.subject.loc))
                .or_default()
                .push((m.id.as_str().to_owned(), m.canonical()?));
        }
        for r in &batch.retirements {
            let claim = claim_of(&r.retires)
                .ok_or_else(|| Error::internal(format!("retirement {} has no claim", r.id)))?;
            new_marks
                .entry(shard_of(&claim.subject.loc))
                .or_default()
                .push((r.id.as_str().to_owned(), r.canonical()?));
        }

        // Step 3: render every shard that changes, claims shards in
        // ascending order, then marks shards. Nothing is written until
        // every one of them is known to fit.
        let mut writes: Vec<(ShardKind, u8, Vec<u8>)> = Vec::new();
        for (&shard, records) in &new_claims {
            let current = &snap.claims[usize::from(shard)];
            if let Some(bytes) = merge_shard(current, records)? {
                writes.push((ShardKind::Claims, shard, bytes));
            }
        }
        for (&shard, records) in &new_marks {
            let current = &snap.marks[usize::from(shard)];
            if let Some(bytes) = merge_shard(current, records)? {
                writes.push((ShardKind::Marks, shard, bytes));
            }
        }
        let total = scan(&self.hibi)?.shard_bytes;
        let mut sizes = Vec::with_capacity(writes.len());
        for (kind, shard, bytes) in &writes {
            let rel = shard_rel(*kind, *shard);
            let now = match fs::symlink_metadata(self.hibi.join(&rel)) {
                Ok(m) => m.len(),
                Err(_) => 0,
            };
            sizes.push((format!("{HIBI_DIR}/{rel}"), now, bytes.len() as u64));
        }
        check_room(caps, total, &sizes)?;
        for (kind, shard, bytes) in &writes {
            replace_shard(&self.hibi, *kind, *shard, bytes)?;
        }
        // Step 4: the lock is released when `_lock` is dropped.
        Ok(())
    }

    fn snapshot(&self) -> Result<String, Error> {
        check_store(&self.hibi)?;
        let mut h = blake3::Hasher::new();
        for (kind, s) in all_shards() {
            // One shard is in memory at a time.
            let bytes = read_shard(&self.hibi, &shard_rel(kind, s))?;
            h.update(&(bytes.len() as u64).to_be_bytes());
            h.update(&bytes);
        }
        Ok(format!("b3:{}", h.finalize().to_hex()))
    }
}

// ---------------------------------------------------------------------------
// init (section 15.3)
// ---------------------------------------------------------------------------

/// The files `init` creates, relative to the root, in the order they are
/// reported.
pub fn init_files() -> Vec<String> {
    let mut files = vec![
        format!("{HIBI_DIR}/hibi.toml"),
        format!("{HIBI_DIR}/.gitattributes"),
        format!("{HIBI_DIR}/.gitignore"),
        format!("{HIBI_DIR}/.ignore"),
    ];
    for (kind, s) in all_shards() {
        files.push(format!("{HIBI_DIR}/{}", shard_rel(kind, s)));
    }
    files
}

/// The store part of `hibi init` (section 15.3). `root` is the git top
/// level the caller found (`git::Git::find_toplevel`).
///
/// Fails, before writing anything, with `legacy_store` when the root holds
/// `.claims/`, `store_corrupt` when `.hibi` or anything under it that
/// section 12.3 names is a symbolic link, and `already_initialized` when
/// `.hibi/hibi.toml` exists (or any other file `init` would create exists).
/// Creates `.hibi/`, `claims/`, `marks/`, the three dot files and the 32
/// empty shards, then `hibi.toml` last, each exclusively and without
/// following a link. Writes nothing outside `.hibi/`. Returns the created
/// files relative to the root.
pub fn init(root: &Path) -> Result<Vec<String>, Error> {
    check_legacy(root)?;
    let hibi = root.join(HIBI_DIR);
    check_links(&hibi)?;
    if !matches!(entry(&hibi.join("hibi.toml"), "hibi.toml")?, Entry::Missing) {
        return Err(Error::new(
            Code::AlreadyInitialized,
            format!("{} already has {HIBI_DIR}/hibi.toml", root.display()),
        ));
    }
    let files = init_files();
    for f in &files {
        if fs::symlink_metadata(root.join(f)).is_ok() {
            return Err(Error::new(
                Code::AlreadyInitialized,
                format!("{f} already exists; remove it or restore .hibi/hibi.toml"),
            ));
        }
    }
    for dir in [hibi.clone(), hibi.join("claims"), hibi.join("marks")] {
        match fs::create_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                // Checked above to be a directory and not a link.
            }
            Err(e) => return Err(Error::io(dir.display(), &e)),
        }
    }
    let contents = |f: &str| -> &'static [u8] {
        match f.rsplit('/').next().unwrap_or("") {
            "hibi.toml" => HIBI_TOML_TEMPLATE.as_bytes(),
            ".gitattributes" => GITATTRIBUTES.as_bytes(),
            ".gitignore" => GITIGNORE.as_bytes(),
            ".ignore" => IGNORE.as_bytes(),
            _ => b"",
        }
    };
    // hibi.toml last: until it exists, root discovery does not see a store.
    let (toml, rest) = files.split_first().expect("init_files is not empty");
    for f in rest.iter().chain(std::iter::once(toml)) {
        let path = root.join(f);
        let mut file = nofollow::apply(OpenOptions::new().write(true).create_new(true))
            .open(&path)
            .map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists || nofollow::is_link_error(&e) {
                    Error::new(Code::AlreadyInitialized, format!("{f} already exists"))
                } else {
                    Error::io(f, &e)
                }
            })?;
        file.write_all(contents(f)).map_err(|e| Error::io(f, &e))?;
    }
    Ok(files)
}

/// Every shard file's path relative to the root, claims then marks.
pub fn shard_paths() -> Vec<String> {
    all_shards()
        .map(|(k, s)| format!("{HIBI_DIR}/{}", shard_rel(k, s)))
        .collect()
}

#[cfg(test)]
#[path = "git_text_tests.rs"]
mod tests;
