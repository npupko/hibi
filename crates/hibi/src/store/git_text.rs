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

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;

use crate::config::nofollow;
use crate::error::{Code, Error};
use crate::model::{Claim, ClaimId, Locator, Mark, Retirement};
use crate::store::{Batch, Ledger, ORPHAN_RECORD, Store, StoreWarning, UNKNOWN_REPLACEMENT};

/// The largest shard file, in bytes (section 12.3).
pub const MAX_SHARD_BYTES: u64 = 67_108_864;

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

/// The symbolic-link check of section 12.3: `.hibi`, `hibi.toml`, `claims`,
/// `marks`, every shard file and `.lock` must not be links; the
/// directories must be directories and the files regular files.
fn check_links(hibi: &Path) -> Result<(), Error> {
    match entry(hibi, "")? {
        Entry::Link => return Err(link_error(HIBI_DIR)),
        Entry::Missing | Entry::Dir => {}
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
            Entry::Missing => continue,
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
                Entry::Missing | Entry::File(_) => {}
                _ => return Err(corrupt(format!("{HIBI_DIR}/{rel} is not a regular file"))),
            }
        }
    }
    Ok(())
}

/// Read one shard: `None` when missing. A link, a non-file or a file longer
/// than [`MAX_SHARD_BYTES`] is `store_corrupt`. Opened without following a
/// link.
fn read_shard(hibi: &Path, rel: &str) -> Result<Option<Vec<u8>>, Error> {
    let path = hibi.join(rel);
    let shown = format!("{HIBI_DIR}/{rel}");
    let too_large = || corrupt(format!("{shown} is longer than {MAX_SHARD_BYTES} bytes"));
    match entry(&path, rel)? {
        Entry::Missing => return Ok(None),
        Entry::Link => return Err(link_error(&shown)),
        Entry::File(len) if len > MAX_SHARD_BYTES => return Err(too_large()),
        Entry::File(_) => {}
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
    let mut buf = Vec::new();
    file.take(MAX_SHARD_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| Error::io(&shown, &e))?;
    if buf.len() as u64 > MAX_SHARD_BYTES {
        return Err(too_large());
    }
    Ok(Some(buf))
}

/// A parsed shard line.
#[derive(Debug, Clone)]
enum Record {
    Claim(Claim),
    Mark(Mark),
    Retirement(Retirement),
}

impl Record {
    fn id(&self) -> &str {
        match self {
            Record::Claim(c) => c.id.as_str(),
            Record::Mark(m) => m.id.as_str(),
            Record::Retirement(r) => r.id.as_str(),
        }
    }

    fn canonical(&self) -> Result<Vec<u8>, Error> {
        match self {
            Record::Claim(c) => c.canonical(),
            Record::Mark(m) => m.canonical(),
            Record::Retirement(r) => r.canonical(),
        }
    }
}

/// One shard as read: its bytes and its records by id (duplicates merged).
#[derive(Debug, Default)]
struct ShardData {
    bytes: Vec<u8>,
    /// id to the canonical line (without the newline) and the record.
    lines: BTreeMap<String, (Vec<u8>, Record)>,
}

fn parse_as<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// Parse a shard by the rules of section 12.3, except the checks that need
/// other shards.
fn parse_shard(kind: ShardKind, rel: &str, bytes: Vec<u8>) -> Result<ShardData, Error> {
    let shown = format!("{HIBI_DIR}/{rel}");
    let mut lines = BTreeMap::new();
    if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
        let n = bytes.split(|&b| b == b'\n').count();
        return Err(corrupt(format!(
            "{shown} line {n} does not end with a newline"
        )));
    }
    let body = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    if !bytes.is_empty() {
        for (i, line) in body.split(|&b| b == b'\n').enumerate() {
            let n = i + 1;
            let bad = |why: &str| corrupt(format!("{shown} line {n}: {why}"));
            let value: serde_json::Value =
                serde_json::from_slice(line).map_err(|e| bad(&format!("not valid JSON: {e}")))?;
            let id = value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned();
            let record = match kind {
                ShardKind::Claims => Record::Claim(parse_as(value).map_err(|e| bad(&e))?),
                ShardKind::Marks if id.starts_with("m1-") => {
                    Record::Mark(parse_as(value).map_err(|e| bad(&e))?)
                }
                ShardKind::Marks if id.starts_with("r1-") => {
                    Record::Retirement(parse_as(value).map_err(|e| bad(&e))?)
                }
                ShardKind::Marks => {
                    return Err(bad(&format!(
                        "id {id:?} is neither a mark nor a retirement id"
                    )));
                }
            };
            let canonical = record.canonical()?;
            if canonical != line {
                return Err(bad("the line is not the canonical form of its record"));
            }
            lines.insert(record.id().to_owned(), (canonical, record));
        }
    }
    Ok(ShardData { bytes, lines })
}

/// The whole store as read, with every check of section 12.3 applied.
#[derive(Debug, Default)]
struct Snapshot {
    claims: [ShardData; 16],
    marks: [ShardData; 16],
    ledger: Ledger,
}

/// Read all 32 shards and build the ledger. Marks shards are read before
/// claims shards: a writer renames claims shards first, so a mark seen here
/// has its claim on disk by the time the claims shards are read, and a
/// concurrent append cannot produce a spurious `orphan_record`.
fn read_store(hibi: &Path) -> Result<Snapshot, Error> {
    check_links(hibi)?;
    let mut snap = Snapshot::default();
    for kind in [ShardKind::Marks, ShardKind::Claims] {
        for s in 0..16u8 {
            let rel = shard_rel(kind, s);
            let bytes = read_shard(hibi, &rel)?.unwrap_or_default();
            let data = parse_shard(kind, &rel, bytes)?;
            match kind {
                ShardKind::Claims => snap.claims[usize::from(s)] = data,
                ShardKind::Marks => snap.marks[usize::from(s)] = data,
            }
        }
    }
    let mut claims: BTreeMap<String, Claim> = BTreeMap::new();
    for (s, data) in snap.claims.iter().enumerate() {
        for (id, (_, rec)) in &data.lines {
            let Record::Claim(c) = rec else { continue };
            let want = shard_of(&c.subject.loc);
            if usize::from(want) != s {
                return Err(corrupt(format!(
                    "claim {id} is in {HIBI_DIR}/{} but belongs in {HIBI_DIR}/{}",
                    shard_rel(ShardKind::Claims, s as u8),
                    shard_rel(ShardKind::Claims, want)
                )));
            }
            claims.insert(id.clone(), c.clone());
        }
    }
    let mut marks: BTreeMap<String, Mark> = BTreeMap::new();
    let mut retirements: BTreeMap<String, Retirement> = BTreeMap::new();
    let mut warnings = Vec::new();
    for (s, data) in snap.marks.iter().enumerate() {
        let rel = shard_rel(ShardKind::Marks, s as u8);
        for (id, (_, rec)) in &data.lines {
            let (claim_id, what) = match rec {
                Record::Mark(m) => (m.claim.as_str(), "mark"),
                Record::Retirement(r) => (r.retires.as_str(), "retirement"),
                Record::Claim(_) => continue,
            };
            let Some(claim) = claims.get(claim_id) else {
                warnings.push(StoreWarning::new(
                    ORPHAN_RECORD,
                    format!(
                        "{what} {id} in {HIBI_DIR}/{rel} names claim {claim_id}, which is not in the store; the {what} is ignored"
                    ),
                ));
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
                Record::Mark(m) => {
                    m.check_against(claim)
                        .map_err(|e| corrupt(format!("{HIBI_DIR}/{rel}: {}", e.message)))?;
                    marks.insert(id.clone(), m.clone());
                }
                Record::Retirement(r) => {
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
                    retirements.insert(id.clone(), r.clone());
                }
                Record::Claim(_) => {}
            }
        }
    }
    snap.ledger = Ledger {
        claims: claims.into_values().collect(),
        marks: marks.into_values().collect(),
        retirements: retirements.into_values().collect(),
        warnings,
    };
    Ok(snap)
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

/// The bytes of a shard holding `lines`: sorted by id, one canonical line
/// each, each followed by a newline.
fn render(lines: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::with_capacity(lines.values().map(|l| l.len() + 1).sum());
    for line in lines.values() {
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    out
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
        Ok(read_store(&self.hibi)?.ledger)
    }

    fn append(&self, batch: &Batch) -> Result<(), Error> {
        verify_records(batch)?;
        // Step 1: the link check, then the lock.
        check_links(&self.hibi)?;
        let _lock = acquire_lock(&self.hibi)?;
        crash_point!(Step::Locked);
        // Re-read under the lock. Every shard passes section 12.3 or the
        // append fails before writing.
        let snap = read_store(&self.hibi)?;
        let stored: BTreeMap<&str, &Claim> = snap
            .claims
            .iter()
            .flat_map(|d| d.lines.values())
            .filter_map(|(_, r)| match r {
                Record::Claim(c) => Some((c.id.as_str(), c)),
                _ => None,
            })
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

        // Step 3: claims shards in ascending order, then marks shards.
        for (kind, groups, current) in [
            (ShardKind::Claims, &new_claims, &snap.claims),
            (ShardKind::Marks, &new_marks, &snap.marks),
        ] {
            for (&shard, records) in groups {
                let data = &current[usize::from(shard)];
                let mut lines: BTreeMap<String, Vec<u8>> = data
                    .lines
                    .iter()
                    .map(|(id, (line, _))| (id.clone(), line.clone()))
                    .collect();
                for (id, line) in records {
                    lines.entry(id.clone()).or_insert_with(|| line.clone());
                }
                let bytes = render(&lines);
                if bytes == data.bytes {
                    continue;
                }
                replace_shard(&self.hibi, kind, shard, &bytes)?;
            }
        }
        // Step 4: the lock is released when `_lock` is dropped.
        Ok(())
    }

    fn snapshot_id(&self) -> Result<String, Error> {
        check_links(&self.hibi)?;
        let mut h = blake3::Hasher::new();
        for (kind, s) in all_shards() {
            let bytes = read_shard(&self.hibi, &shard_rel(kind, s))?.unwrap_or_default();
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
