//! Shared harness for the end-to-end tests: a temporary git repository, and
//! the built `hibi` binary run in it with an isolated user config, cache
//! and environment.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use serde_json::Value;
use tempfile::TempDir;

/// The built `hibi` binary.
pub const HIBI: &str = env!("CARGO_BIN_EXE_hibi");

/// A fixed `--now` for runs that must be reproducible.
pub const NOW: &str = "2026-11-20T12:00:00Z";

/// The real `git`, found on the `PATH` of the test process.
pub fn real_git() -> PathBuf {
    static GIT: OnceLock<PathBuf> = OnceLock::new();
    GIT.get_or_init(|| find_on_path("git").expect("git is on PATH"))
        .clone()
}

pub fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}

/// The `hibi-provider-reference` binary, built on first use into the same
/// target directory as `hibi`.
pub fn reference_provider() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir = Path::new(HIBI).parent().expect("hibi has a directory");
        let mut build = Command::new(env!("CARGO"));
        build.args(["build", "--quiet", "-p", "hibi-provider-reference"]);
        if dir.ends_with("release") {
            build.arg("--release");
        }
        let status = build.status().expect("cargo runs");
        assert!(status.success(), "building hibi-provider-reference failed");
        let bin = dir.join(format!(
            "hibi-provider-reference{}",
            std::env::consts::EXE_SUFFIX
        ));
        assert!(bin.is_file(), "{} is missing", bin.display());
        bin
    })
    .clone()
}

/// The repository's `cases/provider/fixtures/notes.json`.
pub fn notes_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cases/provider/fixtures/notes.json")
}

/// The output of one run.
#[derive(Debug, Clone)]
pub struct Out {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    /// Standard output as the one JSON object of section 15.1.
    pub fn json(&self) -> Value {
        assert!(
            self.stdout.ends_with('\n') && self.stdout.matches('\n').count() == 1,
            "standard output is not one line: {:?}",
            self.stdout
        );
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("standard output is not JSON ({e}): {}", self.stdout))
    }

    /// The envelope of a run that succeeded: exit 0, `ok: true`, empty
    /// standard error.
    pub fn ok(&self) -> Value {
        assert_eq!(self.status, 0, "{self:?}");
        assert_eq!(self.stderr, "", "{self:?}");
        let v = self.json();
        assert_eq!(v["ok"], true, "{v}");
        v
    }

    /// The first error of a run that failed: exit 1, `ok: false`, a
    /// non-empty `errors`, empty standard error.
    pub fn err(&self) -> Value {
        self.errs()[0].clone()
    }

    /// Every error of a run that failed.
    pub fn errs(&self) -> Vec<Value> {
        assert_eq!(self.status, 1, "{self:?}");
        assert_eq!(self.stderr, "", "{self:?}");
        let v = self.json();
        assert_eq!(v["ok"], false, "{v}");
        let errors = v["errors"].as_array().expect("errors is an array").clone();
        assert!(!errors.is_empty(), "{v}");
        for e in &errors {
            assert!(e["code"].is_string() && e["message"].is_string(), "{v}");
            assert!(e.get("remediation").is_some(), "{v}");
        }
        errors
    }

    /// The code of the first error of a run that failed.
    pub fn code(&self) -> String {
        self.err()["code"].as_str().expect("a code").to_owned()
    }
}

/// One `hibi` invocation being built.
pub struct Cmd {
    command: Command,
    stdin: Option<String>,
}

impl Cmd {
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.command.args(args);
        self
    }

    pub fn env(mut self, key: &str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        self.command.env(key, value);
        self
    }

    pub fn env_remove(mut self, key: &str) -> Self {
        self.command.env_remove(key);
        self
    }

    /// Run in this directory instead of the repository root.
    pub fn cwd(mut self, dir: &Path) -> Self {
        self.command.current_dir(dir);
        self
    }

    /// Give this text on standard input.
    pub fn stdin(mut self, text: &str) -> Self {
        self.stdin = Some(text.to_owned());
        self
    }

    /// Start the process without waiting, for concurrency tests.
    pub fn spawn(mut self) -> std::process::Child {
        self.command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("hibi starts")
    }

    pub fn run(mut self) -> Out {
        self.command
            .stdin(if self.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = self.command.spawn().expect("hibi starts");
        if let Some(text) = &self.stdin {
            let mut pipe = child.stdin.take().expect("a pipe");
            // hibi may exit before reading its input (for example on
            // invalid_config); the exit status and output are still checked.
            if let Err(e) = pipe.write_all(text.as_bytes()) {
                assert_eq!(
                    e.kind(),
                    std::io::ErrorKind::BrokenPipe,
                    "stdin is written: {e}"
                );
            }
        }
        out_of(child.wait_with_output().expect("hibi exits"))
    }
}

pub fn out_of(output: std::process::Output) -> Out {
    Out {
        status: output.status.code().expect("an exit status"),
        stdout: String::from_utf8(output.stdout).expect("UTF-8 output"),
        stderr: String::from_utf8(output.stderr).expect("UTF-8 output"),
    }
}

/// A temporary git repository with its own user config and cache
/// directory. Dropping it removes everything.
pub struct Repo {
    _dir: TempDir,
    /// The git top level (canonical path).
    pub root: PathBuf,
    /// A directory outside the repository for the user config and cache.
    pub home: PathBuf,
}

impl Repo {
    /// A git repository on branch `main` with no commit and no store.
    pub fn bare() -> Repo {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let base = dir.path().canonicalize().expect("a real path");
        let root = base.join("repo");
        let home = base.join("home");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let repo = Repo {
            _dir: dir,
            root,
            home,
        };
        repo.git(&["init", "-q", "-b", "main", "."]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    /// A git repository with `hibi init` run in it. Nothing is committed.
    pub fn new() -> Repo {
        let repo = Repo::bare();
        repo.json(&["init"]);
        repo
    }

    /// The user-config path that every run of this repository uses.
    pub fn user_config_path(&self) -> PathBuf {
        self.home.join("config.toml")
    }

    /// The cache directory that every run of this repository uses.
    pub fn cache_dir(&self) -> PathBuf {
        self.home.join("cache")
    }

    /// Write the user config.
    pub fn user_config(&self, text: &str) {
        std::fs::write(self.user_config_path(), text).unwrap();
    }

    /// Append to `.hibi/hibi.toml`.
    pub fn config(&self, text: &str) {
        let path = self.root.join(".hibi/hibi.toml");
        let mut all = std::fs::read_to_string(&path).unwrap();
        all.push('\n');
        all.push_str(text);
        std::fs::write(path, all).unwrap();
    }

    /// Write a file, creating its directories.
    pub fn write(&self, rel: &str, text: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }

    /// Run the real git in the repository, isolated from the user's and the
    /// system's git configuration. Panics when git fails. Returns its
    /// standard output, trimmed.
    pub fn git(&self, args: &[&str]) -> String {
        let out = Command::new(real_git())
            .current_dir(&self.root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// `git add -A` and commit. Returns the commit id.
    pub fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// A `hibi` invocation in the repository root with an isolated
    /// environment: this repository's user config and cache, no `HIBI_BY`,
    /// no judge key, and no global or system git configuration.
    pub fn hibi(&self) -> Cmd {
        let mut command = Command::new(HIBI);
        command
            .current_dir(&self.root)
            .env("HIBI_CONFIG", self.user_config_path())
            .env("HIBI_CACHE_DIR", self.cache_dir())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("HIBI_BY")
            .env_remove("HIBI_JEV_URL")
            .env_remove("TYPESAFE_API_KEY")
            .env_remove("NO_COLOR");
        Cmd {
            command,
            stdin: None,
        }
    }

    /// Run with `--format json`.
    pub fn run(&self, args: &[&str]) -> Out {
        self.hibi().args(["--format", "json"]).args(args).run()
    }

    /// Run with `--format json` and return the envelope of a success.
    pub fn json(&self, args: &[&str]) -> Value {
        self.run(args).ok()
    }

    /// Run with `--format json` and return the first error of a failure.
    pub fn fail(&self, args: &[&str]) -> Value {
        self.run(args).err()
    }

    /// Run with `--format human`.
    pub fn human(&self, args: &[&str]) -> Out {
        self.hibi().args(["--format", "human"]).args(args).run()
    }

    /// `hibi claim` with a fixed `--now`; returns the claim id.
    pub fn claim(&self, statement: &str, subject: &str, evidence: &[&str]) -> String {
        let mut args = vec!["--now", NOW, "claim", statement, "--subject", subject];
        for e in evidence {
            args.push("--evidence");
            args.push(e);
        }
        let v = self.json(&args);
        v["claims"][0]["id"].as_str().expect("an id").to_owned()
    }

    /// `hibi status <id>` and that claim's view.
    pub fn view(&self, id: &str) -> Value {
        let v = self.json(&["status", id]);
        assert_eq!(v["claims"].as_array().map(Vec::len), Some(1), "{v}");
        v["claims"][0].clone()
    }

    /// The state of one claim.
    pub fn state(&self, id: &str) -> String {
        self.view(id)["state"].as_str().expect("a state").to_owned()
    }

    /// Every file under `.hibi/` with its content, to prove that a run
    /// changed nothing.
    pub fn store_files(&self) -> BTreeMap<String, Vec<u8>> {
        tree(&self.root.join(".hibi"))
    }

    /// Every file of the repository outside `.git/`, with its content.
    pub fn files(&self) -> BTreeMap<String, Vec<u8>> {
        let mut all = tree(&self.root);
        all.retain(|k, _| !k.starts_with(".git/"));
        all
    }

    /// The parsed lines of one shard, for example `claims/3.jsonl`.
    pub fn shard(&self, rel: &str) -> Vec<Value> {
        self.read(&format!(".hibi/{rel}"))
            .lines()
            .map(|l| serde_json::from_str(l).expect("a JSON line"))
            .collect()
    }

    /// Every stored record of a kind: `claims` or `marks` (the marks shards
    /// also hold retirements).
    pub fn records(&self, kind: &str) -> Vec<Value> {
        "0123456789abcdef"
            .chars()
            .flat_map(|d| self.shard(&format!("{kind}/{d}.jsonl")))
            .collect()
    }
}

fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).expect("metadata");
            if meta.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(rel, std::fs::read(&path).unwrap_or_default());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

// ---------------------------------------------------------------------------
// A PATH whose programs record how they are called
// ---------------------------------------------------------------------------

/// A directory to put first on (or to use as the whole) `PATH`, holding
/// shell wrappers that append one line per call to a log file.
pub struct Bin {
    pub dir: PathBuf,
    pub log: PathBuf,
}

impl Bin {
    /// An empty directory under `home`.
    pub fn new(home: &Path, name: &str) -> Bin {
        let dir = home.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        Bin {
            log: dir.join("calls.log"),
            dir,
        }
    }

    fn script(&self, program: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.dir.join(program);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A `git` that logs `git <arguments>` and then runs the real git.
    pub fn recording_git(&self) {
        self.script(
            "git",
            &format!(
                "printf 'git %s\\n' \"$*\" >> '{}'\nexec '{}' \"$@\"",
                self.log.display(),
                real_git().display()
            ),
        );
    }

    /// A `git` that logs its call, answers `--version` with this text and
    /// fails for everything else.
    pub fn fake_git(&self, version_output: &str) {
        self.script(
            "git",
            &format!(
                "printf 'git %s\\n' \"$*\" >> '{}'\nif [ \"$1\" = --version ]; then echo '{version_output}'; exit 0; fi\nexit 1",
                self.log.display()
            ),
        );
    }

    /// A program that logs `<program> <arguments>` and exits 0 without
    /// output. Used to prove that a command is never started.
    pub fn recording(&self, program: &str) {
        self.script(
            program,
            &format!(
                "printf '{program} %s\\n' \"$*\" >> '{}'\nexit 0",
                self.log.display()
            ),
        );
    }

    /// The logged calls, one per line.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The value for `PATH` with this directory only. `/bin/sh` is started
    /// by absolute path, so the wrappers still run.
    pub fn only(&self) -> std::ffi::OsString {
        self.dir.clone().into_os_string()
    }
}
