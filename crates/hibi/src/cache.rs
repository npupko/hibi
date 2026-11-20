//! The SQLite cache (spec section 11.3).
//!
//! The cache is never authoritative: every caller holds an
//! `Option<&Cache>`, and `None` (no cache, `--no-cache`, or the database
//! could not be opened) must change no result. Opening never fails the run:
//! [`Cache::open`] returns [`Unavailable`] and the caller reports the
//! warning [`CACHE_UNAVAILABLE`] with its message.
//!
//! Two tables: `units` (the unit list of a file's bytes, keyed by git blob
//! id, resource class and hibi version) and `judge` (judge answers, keyed by
//! the key of section 14.6). This module stores the text it is given; the
//! file provider and the judge own the JSON inside it.
//!
//! Concurrency: the database stays in SQLite's default rollback journal
//! (`DELETE`); hibi never switches it to WAL. Every lock this module takes
//! is taken from outside any transaction (`BEGIN IMMEDIATE` when opening,
//! single autocommit statements afterwards), which is the case where SQLite
//! runs the busy handler instead of returning `SQLITE_BUSY` at once
//! (<https://www.sqlite.org/c3ref/busy_handler.html>: the handler is skipped
//! only when waiting could deadlock, which needs a read transaction that
//! tries to become a write transaction). WAL mode has documented cases that
//! return `SQLITE_BUSY` regardless of the busy handler, among them opening
//! the database while its last connection is closing
//! (<https://www.sqlite.org/wal.html>, section 9), so with WAL two hibi
//! processes opening the cache at once could not both be guaranteed to
//! succeed (section 11.3).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::error::{Code, Error};

/// The warning code reported when the cache cannot be used (section 15.1).
pub const CACHE_UNAVAILABLE: &str = "cache_unavailable";

/// The database file name.
pub const FILE_NAME: &str = "cache.sqlite3";

/// The `user_version` this hibi writes and reads.
pub const SCHEMA_VERSION: i64 = 1;

/// The engine string of the running hibi: its version (section 11.3).
pub const ENGINE: &str = env!("CARGO_PKG_VERSION");

/// The busy timeout of section 11.3.
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

const SCHEMA: &str = "
CREATE TABLE units (
  blob    TEXT NOT NULL,
  class   TEXT NOT NULL,
  engine  TEXT NOT NULL,
  units   TEXT NOT NULL,
  PRIMARY KEY (blob, class, engine)
) WITHOUT ROWID;

CREATE TABLE judge (
  key      TEXT PRIMARY KEY,
  model    TEXT NOT NULL,
  answer   TEXT NOT NULL,
  at       TEXT NOT NULL
) WITHOUT ROWID;

PRAGMA user_version = 1;
";

/// The cache could not be opened; the run continues without it and reports
/// warning [`CACHE_UNAVAILABLE`] with `message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable {
    pub message: String,
}

/// One row of the `judge` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeEntry {
    /// The model id the answer came from.
    pub model: String,
    /// The answer object as JSON text.
    pub answer: String,
    /// When it was stored: a timestamp of section 3.1.
    pub at: String,
}

/// The cache location of section 11.3 from its inputs: `$HIBI_CACHE_DIR`
/// when set and non-empty, else `<cache dir>/hibi`, where the cache dir is
/// `$XDG_CACHE_HOME` or `~/.cache` on Linux and macOS and `%LOCALAPPDATA%`
/// on Windows. The file is [`FILE_NAME`] in that directory. `None` when
/// neither input is available.
pub fn path_from(hibi_cache_dir: Option<OsString>, cache_dir: Option<PathBuf>) -> Option<PathBuf> {
    match hibi_cache_dir {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d).join(FILE_NAME)),
        _ => cache_dir.map(|d| d.join("hibi").join(FILE_NAME)),
    }
}

/// The cache location for this process, from the environment and
/// `etcetera::choose_base_strategy`.
pub fn default_path() -> Option<PathBuf> {
    use etcetera::BaseStrategy;
    let dir = etcetera::choose_base_strategy().ok().map(|s| s.cache_dir());
    path_from(std::env::var_os("HIBI_CACHE_DIR"), dir)
}

/// An open cache database.
#[derive(Debug)]
pub struct Cache {
    conn: Connection,
    engine: String,
}

fn cache_error(e: rusqlite::Error) -> Error {
    Error::new(Code::IoError, format!("cache database: {e}"))
}

impl Cache {
    /// Open the cache of the running hibi version (section 11.3).
    pub fn open(path: &Path) -> Result<Cache, Unavailable> {
        Cache::open_as(path, ENGINE)
    }

    /// Open the cache as hibi version `engine`. Creates the parent
    /// directory and the file when missing; sets the busy timeout; in one
    /// `BEGIN IMMEDIATE` transaction checks `user_version`, creates the
    /// tables when it is 0 and deletes every `units` row of another engine.
    /// A `user_version` other than 0 or 1 leaves the file untouched.
    pub fn open_as(path: &Path, engine: &str) -> Result<Cache, Unavailable> {
        let unavailable = |what: String| Unavailable {
            message: format!("the cache {} {what}; running without it", path.display()),
        };
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| unavailable(format!("cannot be created: {e}")))?;
        }
        let mut conn =
            Connection::open(path).map_err(|e| unavailable(format!("cannot be opened: {e}")))?;
        conn.busy_timeout(BUSY_TIMEOUT)
            .map_err(|e| unavailable(format!("cannot be opened: {e}")))?;
        // The schema is created, and other engines' rows deleted, in one
        // write transaction started with BEGIN IMMEDIATE, so racing openers
        // wait in the busy handler and exactly one of them creates the
        // tables. The journal mode is left at SQLite's default (see the
        // module documentation).
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| unavailable(format!("cannot be opened: {e}")))?;
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| unavailable(format!("cannot be read: {e}")))?;
        match version {
            0 => tx
                .execute_batch(SCHEMA)
                .map_err(|e| unavailable(format!("cannot be created: {e}")))?,
            SCHEMA_VERSION => {}
            other => {
                // Dropping the transaction rolls it back: nothing changed.
                return Err(unavailable(format!(
                    "has user_version {other}, written by another hibi version"
                )));
            }
        }
        tx.execute("DELETE FROM units WHERE engine <> ?1", params![engine])
            .map_err(|e| unavailable(format!("cannot be written: {e}")))?;
        tx.commit()
            .map_err(|e| unavailable(format!("cannot be written: {e}")))?;
        Ok(Cache {
            conn,
            engine: engine.to_owned(),
        })
    }

    /// The engine string rows are written and read with.
    pub fn engine(&self) -> &str {
        &self.engine
    }

    /// The cached unit list (JSON text, or the string `not_text`) of a blob
    /// read as `class`, for this engine.
    pub fn units(&self, blob: &str, class: &str) -> Result<Option<String>, Error> {
        self.conn
            .query_row(
                "SELECT units FROM units WHERE blob = ?1 AND class = ?2 AND engine = ?3",
                params![blob, class, self.engine],
                |r| r.get(0),
            )
            .optional()
            .map_err(cache_error)
    }

    /// Store the unit list of a blob read as `class`, for this engine.
    pub fn put_units(&self, blob: &str, class: &str, units: &str) -> Result<(), Error> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO units (blob, class, engine, units) VALUES (?1, ?2, ?3, ?4)",
                params![blob, class, self.engine, units],
            )
            .map(|_| ())
            .map_err(cache_error)
    }

    /// The judge answer stored under `key` (section 14.6).
    pub fn judge(&self, key: &str) -> Result<Option<JudgeEntry>, Error> {
        self.conn
            .query_row(
                "SELECT model, answer, at FROM judge WHERE key = ?1",
                params![key],
                |r| {
                    Ok(JudgeEntry {
                        model: r.get(0)?,
                        answer: r.get(1)?,
                        at: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(cache_error)
    }

    /// Store a judge answer under `key`.
    pub fn put_judge(&self, key: &str, entry: &JudgeEntry) -> Result<(), Error> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO judge (key, model, answer, at) VALUES (?1, ?2, ?3, ?4)",
                params![key, entry.model, entry.answer, entry.at],
            )
            .map(|_| ())
            .map_err(cache_error)
    }
}

/// The unit cache of the file provider (section 11.3). A failing read is a
/// miss and a failing write is dropped: the cache never changes a result.
#[cfg(feature = "file")]
impl crate::provider::file::UnitCache for Cache {
    fn get_units(&self, blob: &str, class: &str) -> Option<String> {
        self.units(blob, class).ok().flatten()
    }

    fn put_units(&self, blob: &str, class: &str, units: &str) {
        let _ = Cache::put_units(self, blob, class, units);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_order() {
        assert_eq!(
            path_from(Some("/h".into()), Some("/x".into())),
            Some(PathBuf::from("/h/cache.sqlite3"))
        );
        assert_eq!(
            path_from(Some("".into()), Some("/x".into())),
            Some(PathBuf::from("/x/hibi/cache.sqlite3"))
        );
        assert_eq!(
            path_from(None, Some("/x".into())),
            Some(PathBuf::from("/x/hibi/cache.sqlite3"))
        );
        assert_eq!(path_from(None, None), None);
    }

    fn user_version(path: &Path) -> i64 {
        let c = Connection::open(path).unwrap();
        c.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn schema_version_and_rollback_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        drop(Cache::open(&path).unwrap());
        assert_eq!(user_version(&path), SCHEMA_VERSION);
        let c = Connection::open(&path).unwrap();
        let mode: String = c
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "delete", "never WAL: see the module documentation");
    }

    #[test]
    fn other_engines_rows_are_purged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let old = Cache::open_as(&path, "4.9.9").unwrap();
        old.put_units("b", "markdown", "[1]").unwrap();
        old.put_judge(
            "k",
            &JudgeEntry {
                model: "m".into(),
                answer: "{}".into(),
                at: "2026-11-20T18:04:11Z".into(),
            },
        )
        .unwrap();
        drop(old);
        let new = Cache::open_as(&path, "5.0.0").unwrap();
        assert_eq!(new.units("b", "markdown").unwrap(), None);
        assert!(new.judge("k").unwrap().is_some(), "judge rows are kept");
        drop(new);
        let c = Connection::open(&path).unwrap();
        let n: i64 = c
            .query_row("SELECT count(*) FROM units", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "the old engine's rows are deleted, not only hidden");
    }

    #[test]
    fn unknown_user_version_leaves_the_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE other (x); PRAGMA user_version = 7;")
                .unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        let err = Cache::open(&path).unwrap_err();
        assert!(err.message.contains('7'), "{}", err.message);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(user_version(&path), 7);
    }
}
