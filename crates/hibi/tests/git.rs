//! Integration tests of the `git` module against a real `git` (spec section
//! 10.4; requirements R44, R60 and the hostile-repository checks).
//!
//! Every git process in these tests runs with `GIT_CONFIG_GLOBAL=/dev/null`
//! and `GIT_CONFIG_NOSYSTEM=1` (through a wrapper script for the module
//! under test), so the developer's own git configuration cannot hide or
//! cause a result.

#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use hibi::Code;
use hibi::git::{
    Blob, Git, GitStatus, MAX_BLOBS_PER_BATCH, MIN_GIT, SinceRange, TreeMode, parse_range,
};

/// Run the real git with an isolated configuration; panic on failure.
fn sh_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn write_script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A wrapper that records each call to `log`, then runs the real git with an
/// isolated configuration. A call is recorded as the line `CALL`, one line
/// `ARG <argument>` per argument, one `ENV` line with the four environment
/// variables of section 10.4, and one `STDIN` line: whether standard input
/// is a terminal, whether it is `/dev/null`, and, when it is `/dev/null`,
/// how many bytes could be read from it (`unread` otherwise, so that the
/// wrapper never takes the ids meant for `cat-file --batch` and never waits
/// on an open standard input).
fn recording_git(dir: &Path, log: &Path) -> PathBuf {
    let wrapper = dir.join("git-recorder");
    let real = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    write_script(
        &wrapper,
        &format!(
            r#"{{
  printf 'CALL\n'
  for a in "$@"; do printf 'ARG %s\n' "$a"; done
  printf 'ENV GIT_OPTIONAL_LOCKS=%s LC_ALL=%s GIT_TERMINAL_PROMPT=%s GIT_NO_LAZY_FETCH=%s\n' "$GIT_OPTIONAL_LOCKS" "$LC_ALL" "$GIT_TERMINAL_PROMPT" "$GIT_NO_LAZY_FETCH"
  if [ -t 0 ]; then tty=yes; else tty=no; fi
  if [ /dev/stdin -ef /dev/null ]; then null=yes; bytes=$(wc -c | tr -d ' '); else null=no; bytes=unread; fi
  printf 'STDIN tty=%s null=%s bytes=%s\n' "$tty" "$null" "$bytes"
}} >> '{}'
GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 exec '{real}' "$@"
"#,
            log.display()
        ),
    );
    wrapper
}

/// One call recorded by [`recording_git`].
#[derive(Debug)]
struct Call {
    args: Vec<String>,
    env: String,
    stdin: String,
}

fn recorded_calls(log: &Path) -> Vec<Call> {
    let mut calls: Vec<Call> = Vec::new();
    for line in fs::read_to_string(log).unwrap().lines() {
        if line == "CALL" {
            calls.push(Call {
                args: Vec::new(),
                env: String::new(),
                stdin: String::new(),
            });
            continue;
        }
        let call = calls.last_mut().expect("the log starts with CALL");
        if let Some(a) = line.strip_prefix("ARG ") {
            call.args.push(a.to_owned());
        } else if let Some(e) = line.strip_prefix("ENV ") {
            call.env = e.to_owned();
        } else if let Some(s) = line.strip_prefix("STDIN ") {
            call.stdin = s.to_owned();
        } else {
            panic!("unexpected log line {line:?}");
        }
    }
    calls
}

/// The fixed arguments of section 10.4 for a call in `dir`.
fn fixed_prefix(dir: &str) -> Vec<String> {
    [
        "--no-pager",
        "-C",
        dir,
        "-c",
        "core.quotePath=false",
        "-c",
        "safe.bareRepository=explicit",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.untrackedCache=false",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// What [`recording_git`] records for a standard input that is closed
/// (`/dev/null`): not a terminal, and end of file at once.
const STDIN_CLOSED: &str = "tty=no null=yes bytes=0";

/// Run `f` on another thread and return its result; panic when it has not
/// finished after `secs` seconds, so that a deadlock fails the test instead
/// of blocking the suite.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    use std::sync::mpsc::{RecvTimeoutError, channel};
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
        Ok(v) => v,
        Err(RecvTimeoutError::Timeout) => panic!("the call did not return within {secs} s"),
        Err(RecvTimeoutError::Disconnected) => panic!("the call panicked"),
    }
}

/// A program that answers `--version` as a usable git and otherwise runs
/// `body` (a shell fragment), with `$log` naming a file it may append to.
fn fake_git(dir: &Path, name: &str, body: &str) -> (PathBuf, PathBuf) {
    let program = dir.join(name);
    let log = dir.join(format!("{name}.log"));
    write_script(
        &program,
        &format!(
            "log='{}'\nif [ \"$1\" = --version ]; then echo 'git version {MIN_GIT}'; exit 0; fi\n{body}\n",
            log.display()
        ),
    );
    (program, log)
}

fn oid(n: usize) -> hibi::git::ObjectId {
    hibi::git::ObjectId::new(format!("{n:040x}")).unwrap()
}

fn ready(program: &Path) -> Git {
    match GitStatus::probe_program(program) {
        GitStatus::Ready(g) => g,
        other => panic!("git not usable: {other:?}"),
    }
}

/// A repository with one commit (`a.md`, `b.md`, a symbolic link, an
/// executable) and then working-tree changes.
fn sample_repo(dir: &Path) -> PathBuf {
    let root = dir.join("repo");
    fs::create_dir(&root).unwrap();
    sh_git(&root, &["init", "-q"]);
    fs::write(root.join("a.md"), "alpha\n").unwrap();
    fs::write(root.join("b.md"), "beta\n").unwrap();
    fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("a.md", root.join("link.md")).unwrap();
    fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
    sh_git(&root, &["add", "-A"]);
    sh_git(&root, &["commit", "-qm", "base"]);
    root
}

#[test]
fn r44_every_invocation_carries_the_hardening() {
    let tmp = tempfile::tempdir().unwrap();
    let root = sample_repo(tmp.path());
    let log = tmp.path().join("log");
    let git = ready(&recording_git(tmp.path(), &log));

    // Exercise every invocation of section 10.4.
    git.check_root(&root).unwrap();
    git.find_toplevel(&root.join("..").join("repo")).unwrap();
    git.file_list(&root).unwrap();
    let base = git
        .since_base(&root, &SinceRange::Commit("HEAD".into()))
        .unwrap();
    git.since_base(&root, &SinceRange::MergeBase("HEAD".into()))
        .unwrap();
    let changed = git.changed_paths(&root, &base).unwrap();
    let entries = git
        .tree_entries(&root, &base, &["a.md".to_owned()].into())
        .unwrap();
    git.read_blobs(&root, &[entries["a.md"].id.clone()], 1 << 20)
        .unwrap();
    let _ = changed;

    let calls = recorded_calls(&log);
    assert_eq!(
        calls[0].args,
        ["--version"],
        "the version check runs first and alone"
    );
    assert_eq!(calls[0].stdin, STDIN_CLOSED, "the version check");
    assert!(calls.len() >= 10, "{calls:?}");
    let mut subcommands = BTreeSet::new();
    for c in &calls[1..] {
        assert_eq!(
            c.args[..11],
            fixed_prefix(&c.args[2]),
            "the fixed arguments come first: {c:?}"
        );
        let sub = c.args[11].as_str();
        subcommands.insert(sub);
        // Standard input is closed for every call but `cat-file --batch`,
        // which gets the ids on a pipe.
        if sub == "cat-file" {
            assert_eq!(c.stdin, "tty=no null=no bytes=unread", "{c:?}");
        } else {
            assert_eq!(c.stdin, STDIN_CLOSED, "{c:?}");
        }
    }
    for c in &calls {
        assert_eq!(
            c.env, "GIT_OPTIONAL_LOCKS=0 LC_ALL=C GIT_TERMINAL_PROMPT=0 GIT_NO_LAZY_FETCH=1",
            "{c:?}"
        );
    }
    assert_eq!(
        subcommands,
        [
            "cat-file",
            "diff",
            "ls-files",
            "ls-tree",
            "merge-base",
            "rev-parse"
        ]
        .into()
    );
}

/// Section 10.4, "Changed paths": the two calls carry exactly these
/// arguments after the fixed ones, and standard input is closed for both.
#[test]
fn changed_paths_argv_and_closed_stdin() {
    let tmp = tempfile::tempdir().unwrap();
    let root = sample_repo(tmp.path());
    let log = tmp.path().join("log");
    let git = ready(&recording_git(tmp.path(), &log));
    let head = sh_git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    let base = hibi::git::ObjectId::new(head.clone()).unwrap();
    fs::write(root.join("a.md"), "alpha 2\n").unwrap();
    fs::write(root.join("new.md"), "new\n").unwrap();
    let changed = git.changed_paths(&root, &base).unwrap();
    assert_eq!(changed, ["a.md".to_owned(), "new.md".to_owned()].into());

    let calls = recorded_calls(&log);
    let with_prefix = |rest: &[&str]| -> Vec<String> {
        let mut v = fixed_prefix(root.to_str().unwrap());
        v.extend(rest.iter().map(|s| s.to_string()));
        v
    };
    let argv: Vec<Vec<String>> = calls.iter().map(|c| c.args.clone()).collect();
    assert_eq!(
        argv,
        [
            vec!["--version".to_owned()],
            with_prefix(&[
                "diff",
                "--name-only",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "-z",
                &head,
                "--",
            ]),
            with_prefix(&["ls-files", "-z", "--others", "--exclude-standard"]),
        ]
    );
    for c in &calls {
        assert_eq!(c.stdin, STDIN_CLOSED, "{c:?}");
    }
}

/// `cat-file --batch`: a git that writes several hundred KiB to standard
/// error before it reads an id, and more after each answer, does not block
/// the call: standard error is read while standard output is.
#[test]
fn read_blobs_does_not_block_on_standard_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (program, _) = fake_git(
        tmp.path(),
        "noisy",
        r#"head -c 524288 /dev/zero | tr '\0' 'e' >&2
while read -r id; do
  printf '%s blob 6\nalpha\n\n' "$id"
  head -c 131072 /dev/zero | tr '\0' 'w' >&2
done"#,
    );
    let git = ready(&program);
    let root = tmp.path().to_owned();
    let ids = vec![oid(1), oid(2), oid(3)];
    let blobs = within(20, move || git.read_blobs(&root, &ids, 1 << 20)).unwrap();
    assert_eq!(blobs, vec![Blob::Content(b"alpha\n".to_vec()); 3]);
}

/// A failing `cat-file --batch` that wrote several hundred KiB to standard
/// error is `git_failed` at once, and the message carries the end of what
/// git wrote, not all of it.
#[test]
fn read_blobs_failure_keeps_the_end_of_standard_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (program, _) = fake_git(
        tmp.path(),
        "failing",
        r#"head -c 524288 /dev/zero | tr '\0' 'e' >&2
echo 'fatal: the last words of the fake git' >&2
exit 3"#,
    );
    let git = ready(&program);
    let root = tmp.path().to_owned();
    let err = within(20, move || git.read_blobs(&root, &[oid(1)], 1 << 20)).unwrap_err();
    assert_eq!(err.code, Code::GitFailed);
    assert!(
        err.message
            .ends_with("fatal: the last words of the fake git"),
        "the message ends with {:?}",
        &err.message[err.message.len().saturating_sub(200)..]
    );
    assert!(
        err.message.len() < 16 * 1024,
        "the message holds {} bytes",
        err.message.len()
    );
}

/// One call answers at most `MAX_BLOBS_PER_BATCH` ids, so the bytes it holds
/// are bounded by that count times the size limit per blob. A longer list is
/// refused before any git process is started; the caller passes it in
/// batches.
#[test]
fn read_blobs_takes_a_bounded_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let (program, log) = fake_git(
        tmp.path(),
        "counting",
        r#"echo call >> "$log"
while read -r id; do printf '%s blob 6\nalpha\n\n' "$id"; done"#,
    );
    let git = ready(&program);
    let full: Vec<_> = (0..MAX_BLOBS_PER_BATCH).map(oid).collect();
    let blobs = git.read_blobs(tmp.path(), &full, 1 << 20).unwrap();
    assert_eq!(blobs.len(), MAX_BLOBS_PER_BATCH);
    assert_eq!(fs::read_to_string(&log).unwrap(), "call\n");

    let over: Vec<_> = (0..=MAX_BLOBS_PER_BATCH).map(oid).collect();
    let err = git.read_blobs(tmp.path(), &over, 1 << 20).unwrap_err();
    assert_eq!(err.code, Code::Internal);
    assert!(
        err.message.contains(&MAX_BLOBS_PER_BATCH.to_string()),
        "{}",
        err.message
    );
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "call\n",
        "no git process was started for the refused list"
    );

    let mut all = Vec::new();
    for batch in over.chunks(MAX_BLOBS_PER_BATCH) {
        all.extend(git.read_blobs(tmp.path(), batch, 1 << 20).unwrap());
    }
    assert_eq!(all.len(), MAX_BLOBS_PER_BATCH + 1);
}

#[test]
fn hostile_repository_config_runs_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = sample_repo(tmp.path());
    let marker_dir = tmp.path().join("markers");
    fs::create_dir(&marker_dir).unwrap();
    let hook = tmp.path().join("hook.sh");
    write_script(
        &hook,
        &format!("touch '{}/ran-$$'\nexit 1\n", marker_dir.display()),
    );
    let markers = || fs::read_dir(&marker_dir).unwrap().count();

    // A bare-repository layout committed inside the tree, whose config names
    // the hook as its fsmonitor and the outer directory as its work tree.
    let evil = root.join("evil");
    fs::create_dir_all(evil.join("objects")).unwrap();
    fs::create_dir_all(evil.join("refs/heads")).unwrap();
    fs::write(evil.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(
        evil.join("config"),
        format!(
            "[core]\n\trepositoryformatversion = 0\n\tbare = false\n\tworktree = ..\n\tfsmonitor = {}\n",
            hook.display()
        ),
    )
    .unwrap();
    sh_git(&root, &["add", "-A"]);
    sh_git(&root, &["commit", "-qm", "evil"]);

    // Control: plain git in that directory runs the hook.
    let out = Command::new("git")
        .args(["-C"])
        .arg(&evil)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(markers() > 0, "control: unhardened git runs the hook");
    for e in fs::read_dir(&marker_dir).unwrap() {
        fs::remove_file(e.unwrap().path()).unwrap();
    }

    // The root repository's own config names the hook too (the root's
    // config is trusted, but core.fsmonitor=false must still win).
    sh_git(&root, &["config", "core.fsmonitor", hook.to_str().unwrap()]);
    fs::write(root.join("a.md"), "alpha changed\n").unwrap();

    let log = tmp.path().join("log");
    let git = ready(&recording_git(tmp.path(), &log));
    assert!(
        !git.is_toplevel(&evil).unwrap(),
        "evil is not a work tree top level"
    );
    assert_eq!(git.toplevel(&evil).unwrap(), None);
    assert_eq!(git.file_list(&evil).unwrap_err().code, Code::GitFailed);
    git.check_root(&root).unwrap();
    let files = git.file_list(&root).unwrap();
    assert!(files.contains(&"evil/config".to_owned()));
    let base = git.resolve_commit(&root, "HEAD").unwrap();
    let changed = git.changed_paths(&root, &base).unwrap();
    assert!(changed.contains("a.md"));
    assert_eq!(markers(), 0, "no hardened git command ran the hook");
}

#[test]
fn r60_old_missing_or_unparsable_git() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("calls");
    let fake = |name: &str, version: &str| {
        let p = tmp.path().join(name);
        write_script(
            &p,
            &format!("echo \"$*\" >> '{}'\necho '{version}'\n", log.display()),
        );
        p
    };

    let old = GitStatus::probe_program(fake("old", "git version 2.44.2"));
    let err = old.require().unwrap_err();
    assert_eq!(err.code, Code::GitTooOld);
    assert!(err.message.contains("2.44.2") && err.message.contains(&MIN_GIT.to_string()));
    assert!(old.git().is_none());
    assert!(
        old.too_old_message().is_some(),
        "repo: sides carry the warning"
    );

    let garbage = GitStatus::probe_program(fake("garbage", "not git at all"));
    assert_eq!(garbage.require().unwrap_err().code, Code::GitTooOld);

    let apple = GitStatus::probe_program(fake("apple", "git version 2.54.0 (Apple Git-157)"));
    assert_eq!(apple.require().unwrap().version().to_string(), "2.54.0");
    let short = GitStatus::probe_program(fake("short", "git version 2.45"));
    assert_eq!(short.require().unwrap().version().to_string(), "2.45.0");

    // Only `--version` was ever run.
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.lines().all(|l| l == "--version"), "{calls}");
    assert_eq!(calls.lines().count(), 4);

    let missing = GitStatus::probe_program(tmp.path().join("no-such-git"));
    assert!(matches!(missing, GitStatus::Missing { .. }));
    assert_eq!(missing.require().unwrap_err().code, Code::NotAGitRepository);
    assert!(missing.too_old_message().is_none());
}

#[test]
fn file_list_changed_paths_tree_and_blobs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = sample_repo(tmp.path());
    let git = ready(&recording_git(tmp.path(), &tmp.path().join("log")));
    let base = git.resolve_commit(&root, "HEAD").unwrap();

    // Working-tree changes: modify, add untracked, add ignored, add a path
    // with a control character, delete one.
    fs::write(root.join("a.md"), "alpha 2\n").unwrap();
    fs::write(root.join("new.md"), "new\n").unwrap();
    fs::write(root.join("ignored.txt"), "secret\n").unwrap();
    fs::write(root.join("tab\there.md"), "x\n").unwrap();
    fs::write(root.join("é.md"), "x\n").unwrap();
    fs::remove_file(root.join("b.md")).unwrap();

    let files = git.file_list(&root).unwrap();
    assert!(files.contains(&"new.md".to_owned()));
    assert!(
        files.contains(&"é.md".to_owned()),
        "core.quotePath=false keeps UTF-8"
    );
    assert!(!files.contains(&"ignored.txt".to_owned()));
    assert!(
        files.iter().all(|f| !f.contains('\t')),
        "control characters are dropped"
    );
    let mut sorted = files.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(files, sorted);

    let changed = git.changed_paths(&root, &base).unwrap();
    let want: BTreeSet<String> = ["a.md", "b.md", "new.md", "é.md"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(changed, want);

    let paths: BTreeSet<String> = ["a.md", "link.md", "run.sh", "new.md"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let entries = git.tree_entries(&root, &base, &paths).unwrap();
    assert_eq!(entries.len(), 3, "new.md is absent at the base");
    assert_eq!(entries["a.md"].mode, TreeMode::Regular);
    assert_eq!(entries["run.sh"].mode, TreeMode::Executable);
    assert_eq!(entries["link.md"].mode, TreeMode::Symlink);
    assert!(!entries["link.md"].mode.is_regular_file());

    let tree =
        hibi::git::ObjectId::new(sh_git(&root, &["rev-parse", "HEAD^{tree}"]).trim()).unwrap();
    let missing = hibi::git::ObjectId::new("0".repeat(40)).unwrap();
    let blobs = git
        .read_blobs(
            &root,
            &[
                entries["a.md"].id.clone(),
                tree,
                missing,
                entries["run.sh"].id.clone(),
            ],
            6,
        )
        .unwrap();
    assert_eq!(blobs[0], Blob::Content(b"alpha\n".to_vec()));
    assert_eq!(blobs[1], Blob::NotBlob);
    assert_eq!(blobs[2], Blob::Missing);
    assert_eq!(blobs[3], Blob::TooLarge { size: 10 });
    #[cfg(any(feature = "file", feature = "cache"))]
    assert_eq!(
        hibi::git::blob_id(b"alpha\n"),
        entries["a.md"].id.as_str(),
        "blob ids match git's"
    );
}

#[test]
fn ranges_resolve_against_git() {
    let tmp = tempfile::tempdir().unwrap();
    let root = sample_repo(tmp.path());
    let git = ready(&recording_git(tmp.path(), &tmp.path().join("log")));
    let first = sh_git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    sh_git(&root, &["checkout", "-qb", "side"]);
    fs::write(root.join("side.md"), "s\n").unwrap();
    sh_git(&root, &["add", "-A"]);
    sh_git(&root, &["commit", "-qm", "side"]);
    sh_git(&root, &["checkout", "-q", "main"]);
    fs::write(root.join("main.md"), "m\n").unwrap();
    sh_git(&root, &["add", "-A"]);
    sh_git(&root, &["commit", "-qm", "main"]);

    let base = |arg: &str| {
        let r = parse_range(arg)?;
        git.since_base(&root, &r).map(|o| o.as_str().to_owned())
    };
    let side = sh_git(&root, &["rev-parse", "side"]).trim().to_owned();
    assert_eq!(base("side").unwrap(), side);
    assert_eq!(base("side..").unwrap(), side);
    assert_eq!(base("side..HEAD").unwrap(), side);
    assert_eq!(base("side...").unwrap(), first);
    assert_eq!(base("side...HEAD").unwrap(), first);
    assert_eq!(base("nope").unwrap_err().code, Code::InvalidRange);
    assert_eq!(base("side..main").unwrap_err().code, Code::InvalidRange);
    assert_eq!(base("HEAD^{tree}").unwrap_err().code, Code::InvalidRange);
}

#[test]
fn root_check_canonicalizes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = sample_repo(tmp.path());
    let git = ready(&recording_git(tmp.path(), &tmp.path().join("log")));
    let link = tmp.path().join("via-link");
    std::os::unix::fs::symlink(&root, &link).unwrap();
    git.check_root(&link).unwrap();
    git.check_root(&root).unwrap();
    let sub = root.join("sub");
    fs::create_dir(&sub).unwrap();
    assert_eq!(
        git.check_root(&sub).unwrap_err().code,
        Code::NotAGitRepository
    );
    let outside = tmp.path().join("plain");
    fs::create_dir(&outside).unwrap();
    assert_eq!(
        git.check_root(&outside).unwrap_err().code,
        Code::NotAGitRepository
    );
    assert_eq!(
        git.find_toplevel(&outside).unwrap_err().code,
        Code::NotAGitRepository
    );
    assert_eq!(
        git.find_toplevel(&sub).unwrap(),
        fs::canonicalize(&root).unwrap()
    );
}

// ---------------------------------------------------------------------------
// Inherited environment
//
// These tests need variables in the environment of the process that runs
// the module, which a test cannot set in-process without `unsafe`. The test
// binary therefore starts itself again with the variables set, and
// `child_role` runs the module there.
// ---------------------------------------------------------------------------

const CHILD_ENV: &str = "HIBI_GIT_TEST_CHILD";

/// Start this test binary as a child running `child_role` with `role`,
/// `root`, `arg` and the extra environment; true when it passed.
fn run_child(role: &str, root: &Path, arg: &str, env: &[(&str, String)]) -> bool {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "child_role", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, format!("{role}\n{}\n{arg}", root.display()))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.status().unwrap().success()
}

/// The child side. Does nothing unless started by `run_child`.
#[test]
fn child_role() {
    let Ok(spec) = std::env::var(CHILD_ENV) else {
        return;
    };
    let mut parts = spec.splitn(3, '\n');
    let (role, root, arg) = (
        parts.next().unwrap(),
        PathBuf::from(parts.next().unwrap()),
        parts.next().unwrap(),
    );
    let git = ready(Path::new("git"));
    match role {
        // `arg` is the marker directory; it must stay empty.
        "no-hook" => {
            git.check_root(&root).unwrap();
            git.file_list(&root).unwrap();
            let base = git.resolve_commit(&root, "HEAD").unwrap();
            git.changed_paths(&root, &base).unwrap();
            assert_eq!(
                fs::read_dir(arg).unwrap().count(),
                0,
                "the hook ran despite core.fsmonitor=false"
            );
        }
        // `arg` is the root's HEAD commit.
        "own-repo" => {
            git.check_root(&root).unwrap();
            let files = git.file_list(&root).unwrap();
            assert!(files.contains(&"a.md".to_owned()), "{files:?}");
            assert!(!files.contains(&"other-only.md".to_owned()), "{files:?}");
            let head = git.resolve_commit(&root, "HEAD").unwrap();
            assert_eq!(head.as_str(), arg);
            let changed = git.changed_paths(&root, &head).unwrap();
            assert!(!changed.contains("other-only.md"), "{changed:?}");
            let entries = git
                .tree_entries(&root, &head, &["a.md".to_owned()].into())
                .unwrap();
            assert_eq!(
                git.read_blobs(&root, &[entries["a.md"].id.clone()], 1 << 20)
                    .unwrap(),
                vec![Blob::Content(b"alpha\n".to_vec())]
            );
        }
        other => panic!("unknown role {other}"),
    }
}

/// The hardening `-c core.fsmonitor=false` wins over the user's
/// `GIT_CONFIG_COUNT`/`KEY`/`VALUE` and `GIT_CONFIG_PARAMETERS`, which are
/// passed through unchanged.
#[test]
fn hardening_flags_win_over_config_environment() {
    let tmp = tempfile::tempdir().unwrap();
    let root = sample_repo(tmp.path());
    fs::write(root.join("a.md"), "alpha changed\n").unwrap();
    let marker_dir = tmp.path().join("markers");
    fs::create_dir(&marker_dir).unwrap();
    let hook = tmp.path().join("hook.sh");
    write_script(
        &hook,
        &format!("touch '{}/ran-$$'\nexit 1\n", marker_dir.display()),
    );
    let hook = hook.display().to_string();
    let variants: [Vec<(&str, String)>; 2] = [
        vec![
            ("GIT_CONFIG_COUNT", "1".into()),
            ("GIT_CONFIG_KEY_0", "core.fsmonitor".into()),
            ("GIT_CONFIG_VALUE_0", hook.clone()),
        ],
        vec![(
            "GIT_CONFIG_PARAMETERS",
            format!("'core.fsmonitor'='{hook}'"),
        )],
    ];
    for env in &variants {
        // Control: plain git with this environment runs the hook.
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["ls-files", "--cached", "--others", "--exclude-standard"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(
            fs::read_dir(&marker_dir).unwrap().count() > 0,
            "control: {env:?} makes plain git run the hook"
        );
        for e in fs::read_dir(&marker_dir).unwrap() {
            fs::remove_file(e.unwrap().path()).unwrap();
        }
        assert!(
            run_child("no-hook", &root, &marker_dir.display().to_string(), env),
            "with {env:?} the module ran the hook or failed"
        );
    }
}

/// Inherited repository-location variables are removed: with `GIT_DIR`,
/// `GIT_WORK_TREE` and `GIT_INDEX_FILE` naming another repository, every
/// call still reads the root's repository.
#[test]
fn inherited_git_dir_does_not_redirect() {
    let tmp = tempfile::tempdir().unwrap();
    let root = sample_repo(tmp.path());
    let head = sh_git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    let other = tmp.path().join("other");
    fs::create_dir(&other).unwrap();
    sh_git(&other, &["init", "-q"]);
    fs::write(other.join("other-only.md"), "x\n").unwrap();
    sh_git(&other, &["add", "-A"]);
    sh_git(&other, &["commit", "-qm", "other"]);
    let env = vec![
        ("GIT_DIR", other.join(".git").display().to_string()),
        ("GIT_WORK_TREE", other.display().to_string()),
        (
            "GIT_INDEX_FILE",
            other.join(".git/index").display().to_string(),
        ),
    ];

    // Control: plain git with this environment reads the other repository.
    let out = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["ls-files"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "other-only.md\n");

    assert!(run_child("own-repo", &root, &head, &env));
}
