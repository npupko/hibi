//! An in-memory `Store` and a scripted in-memory `Provider` for the engine
//! tests, so that every row of spec section 9.2 is reachable.

#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use hibi::engine::{self, ClaimInput, ClaimView, StatusQuery};
use hibi::fingerprint::prose_v1;
use hibi::git::GitStatus;
use hibi::model::{Anchor, ClaimId, Fingerprint, Locator, Selector, SelectorKey, Timestamp};
use hibi::provider::{
    Capabilities, NameVersion, Provider, ProviderError, ProviderErrorCode, ProviderInfo, Providers,
    Range, Rendered, Unit,
};
use hibi::store::{Batch, Ledger, Store};
use hibi::{Code, Error};

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// A `Store` that keeps its ledger in memory and counts `append` calls.
#[derive(Default)]
pub struct MemStore {
    ledger: RefCell<Ledger>,
    pub appends: Cell<usize>,
}

impl MemStore {
    pub fn new() -> Self {
        MemStore::default()
    }

    pub fn ledger(&self) -> Ledger {
        self.ledger.borrow().clone()
    }

    /// Number of claims, marks and retirements.
    pub fn counts(&self) -> (usize, usize, usize) {
        let l = self.ledger.borrow();
        (l.claims.len(), l.marks.len(), l.retirements.len())
    }

    /// Put records in directly, without the checks of `append`.
    pub fn insert(&self, batch: Batch) {
        let mut l = self.ledger.borrow_mut();
        l.claims.extend(batch.claims);
        l.marks.extend(batch.marks);
        l.retirements.extend(batch.retirements);
        l.claims.sort_by(|a, b| a.id.cmp(&b.id));
        l.marks.sort_by(|a, b| a.id.cmp(&b.id));
        l.retirements.sort_by(|a, b| a.id.cmp(&b.id));
    }

    pub fn warn(&self, code: &str, message: &str) {
        self.ledger
            .borrow_mut()
            .warnings
            .push(hibi::store::StoreWarning::new(code, message));
    }
}

impl Store for MemStore {
    fn load(&self) -> Result<Ledger, Error> {
        Ok(self.ledger.borrow().clone())
    }

    fn append(&self, batch: &Batch) -> Result<(), Error> {
        self.appends.set(self.appends.get() + 1);
        batch.verify_ids()?;
        {
            let l = self.ledger.borrow();
            batch.verify_claims(l.claims.iter().map(|c| &c.id))?;
        }
        let mut fresh = Batch::default();
        {
            let l = self.ledger.borrow();
            for c in &batch.claims {
                if !l.claims.iter().any(|x| x.id == c.id) {
                    fresh.claims.push(c.clone());
                }
            }
            for m in &batch.marks {
                if !l.marks.iter().any(|x| x.id == m.id) {
                    fresh.marks.push(m.clone());
                }
            }
            for r in &batch.retirements {
                if !l.retirements.iter().any(|x| x.id == r.id) {
                    fresh.retirements.push(r.clone());
                }
            }
        }
        self.insert(fresh);
        Ok(())
    }

    fn snapshot_id(&self) -> Result<String, Error> {
        let (c, m, r) = self.counts();
        Ok(format!("mem:{c}:{m}:{r}"))
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// What a scripted provider answers. Tests change it between runs.
pub struct World {
    /// Locator to its units, or to the error every request for it gets.
    pub resources: BTreeMap<String, Result<Vec<Unit>, ProviderError>>,
    /// Answers for exact anchors; they win over `resources`.
    pub overrides: HashMap<Anchor, Result<Vec<Unit>, ProviderError>>,
    pub info_error: Option<ProviderError>,
    pub selectors: Vec<SelectorKey>,
    /// The `kinds` of `info`. Empty, the default, means not stated.
    pub kinds: Vec<String>,
    pub render: bool,
    /// Answer `resolve` with one result too few.
    pub short_answers: bool,
    /// One line per call: `info`, `resolve <anchor json>` per anchor,
    /// `units <loc>`, `render <loc> <key>`.
    pub log: Vec<String>,
}

pub type Shared = Rc<RefCell<World>>;

pub fn world(selectors: &[SelectorKey]) -> Shared {
    Rc::new(RefCell::new(World {
        resources: BTreeMap::new(),
        overrides: HashMap::new(),
        info_error: None,
        selectors: selectors.to_vec(),
        kinds: Vec::new(),
        render: true,
        short_answers: false,
        log: Vec::new(),
    }))
}

/// A world for an external scheme: selectors `key` and `quote`.
pub fn key_world() -> Shared {
    world(&[SelectorKey::Key, SelectorKey::Quote])
}

/// A world shaped like the file provider: selectors `quote` and `symbol`.
pub fn file_world() -> Shared {
    world(&[SelectorKey::Quote, SelectorKey::Symbol])
}

pub struct Mem {
    world: Shared,
    info: ProviderInfo,
}

impl Mem {
    pub fn new(world: &Shared) -> Self {
        Mem {
            world: world.clone(),
            info: ProviderInfo {
                protocol: 1,
                provider: NameVersion {
                    name: "mem".into(),
                    version: "0".into(),
                },
                schemes: vec![],
                selectors: vec![],
                kinds: vec![],
                capabilities: Capabilities::default(),
            },
        }
    }
}

fn selects(sel: &Selector, u: &Unit) -> bool {
    sel.key.as_ref().is_none_or(|k| *k == u.key)
        && sel.symbol.as_ref().is_none_or(|s| *s == u.key)
        && sel
            .quote
            .as_ref()
            .is_none_or(|q| u.text.as_deref().is_some_and(|t| t.contains(q.as_str())))
}

impl Provider for Mem {
    fn info(&mut self) -> Result<&ProviderInfo, ProviderError> {
        let mut w = self.world.borrow_mut();
        w.log.push("info".into());
        if let Some(e) = &w.info_error {
            return Err(e.clone());
        }
        self.info.selectors = w.selectors.clone();
        self.info.kinds = w.kinds.clone();
        self.info.capabilities = Capabilities {
            units: true,
            render: w.render,
            batch: true,
        };
        Ok(&self.info)
    }

    fn resolve(&mut self, anchors: &[Anchor]) -> Vec<Result<Vec<Unit>, ProviderError>> {
        let mut w = self.world.borrow_mut();
        let mut out = Vec::new();
        for a in anchors {
            w.log
                .push(format!("resolve {}", serde_json::to_string(a).unwrap()));
            if let Some(answer) = w.overrides.get(a) {
                out.push(answer.clone());
                continue;
            }
            out.push(match w.resources.get(a.loc.as_str()) {
                None => Err(ProviderError::not_found("no such resource")),
                Some(Err(e)) => Err(e.clone()),
                Some(Ok(units)) => Ok(units
                    .iter()
                    .filter(|u| selects(&a.sel, u))
                    .cloned()
                    .collect()),
            });
        }
        if w.short_answers {
            out.pop();
        }
        out
    }

    fn units(&mut self, loc: &Locator, kind: Option<&str>) -> Result<Vec<Unit>, ProviderError> {
        let mut w = self.world.borrow_mut();
        w.log.push(format!("units {loc}"));
        match w.resources.get(loc.as_str()) {
            None => Err(ProviderError::not_found("no such resource")),
            Some(Err(e)) => Err(e.clone()),
            Some(Ok(units)) => Ok(units
                .iter()
                .filter(|u| kind.is_none_or(|k| k == u.kind))
                .map(|u| Unit {
                    text: None,
                    ..u.clone()
                })
                .collect()),
        }
    }

    fn render(&mut self, loc: &Locator, unit: &str) -> Result<Rendered, ProviderError> {
        let mut w = self.world.borrow_mut();
        w.log.push(format!("render {loc} {unit}"));
        let found = match w.resources.get(loc.as_str()) {
            Some(Ok(units)) => units.iter().find(|u| u.key == unit).cloned(),
            _ => None,
        };
        match found {
            Some(u) => Ok(Rendered {
                text: u.text.unwrap_or_default(),
                context: vec![],
            }),
            None => Err(ProviderError::new(
                ProviderErrorCode::UnknownUnit,
                "unknown",
            )),
        }
    }
}

/// A registry that serves `schemes` from `world`.
pub fn providers(world: &Shared, schemes: &[&str]) -> Providers {
    let mut p = Providers::new();
    p.register(schemes, Box::new(Mem::new(world)));
    p
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

pub fn loc(s: &str) -> Locator {
    Locator::new(s).unwrap()
}

pub fn key_sel(key: &str) -> Selector {
    Selector {
        key: Some(key.into()),
        ..Selector::default()
    }
}

pub fn symbol_sel(symbol: &str) -> Selector {
    Selector {
        symbol: Some(symbol.into()),
        ..Selector::default()
    }
}

pub fn anchor(l: &str, sel: Selector) -> Anchor {
    Anchor::new(loc(l), sel)
}

/// A unit whose completed selector is `{key}` and whose fingerprint is
/// `prose-v1` of its text.
pub fn unit(key: &str, kind: &str, text: &str) -> Unit {
    Unit {
        key: key.into(),
        kind: kind.into(),
        parent: None,
        label: Some(text.into()),
        range: None,
        fp: prose_v1(text),
        sel: key_sel(key),
        text: Some(text.into()),
        approx: false,
        identity: None,
        refuse: None,
    }
}

/// The same, with the completed selector `{symbol}`.
pub fn sym_unit(key: &str, kind: &str, text: &str) -> Unit {
    Unit {
        sel: symbol_sel(key),
        ..unit(key, kind, text)
    }
}

pub fn child(mut u: Unit, parent: &str) -> Unit {
    u.parent = Some(parent.into());
    u
}

pub fn at(mut u: Unit, start: usize, line: usize) -> Unit {
    u.range = Some(Range {
        start,
        end: start + 1,
        line,
        end_line: line,
    });
    u
}

pub fn fp(profile: &str, digit: char) -> Fingerprint {
    Fingerprint::new(format!("b3:{profile}:{}", digit.to_string().repeat(32))).unwrap()
}

pub fn perr(code: ProviderErrorCode) -> ProviderError {
    ProviderError::new(code, "scripted")
}

pub fn set(world: &Shared, l: &str, units: Vec<Unit>) {
    world.borrow_mut().resources.insert(l.into(), Ok(units));
}

pub fn set_err(world: &Shared, l: &str, code: ProviderErrorCode) {
    world
        .borrow_mut()
        .resources
        .insert(l.into(), Err(perr(code)));
}

pub fn set_err_message(world: &Shared, l: &str, code: ProviderErrorCode, message: &str) {
    world
        .borrow_mut()
        .resources
        .insert(l.into(), Err(ProviderError::new(code, message)));
}

/// Replace the text (and so the fingerprint) of one unit.
pub fn edit(world: &Shared, l: &str, key: &str, text: &str) {
    let mut w = world.borrow_mut();
    let units = w.resources.get_mut(l).unwrap().as_mut().unwrap();
    let u = units.iter_mut().find(|u| u.key == key).unwrap();
    u.text = Some(text.into());
    u.label = Some(text.into());
    u.fp = prose_v1(text);
}

pub fn log(world: &Shared) -> Vec<String> {
    world.borrow().log.clone()
}

pub fn ts(s: &str) -> Timestamp {
    Timestamp::parse(s).unwrap()
}

pub const NOW: &str = "2026-11-20T18:04:11Z";

pub fn now() -> Timestamp {
    ts(NOW)
}

/// No git: none of these tests has a `repo:` side unless it says so.
pub fn no_git() -> GitStatus {
    GitStatus::Missing {
        message: "git is not used in this test".into(),
    }
}

pub fn claim_input(statement: &str, subject: &str, evidence: &[&str]) -> ClaimInput {
    ClaimInput {
        statement: statement.into(),
        subject: subject.into(),
        evidence: evidence.iter().map(|e| (*e).into()).collect(),
        recheck: None,
        note: None,
        by: None,
    }
}

pub fn json_anchor(l: &str, sel: Selector) -> engine::AnchorInput {
    engine::AnchorInput::Anchor(anchor(l, sel))
}

/// Create one claim at `at` and return its id.
pub fn claim_at(store: &MemStore, p: &mut Providers, input: ClaimInput, at: &str) -> ClaimId {
    let out = engine::claim(store, p, &no_git(), &[input], false, ts(at)).unwrap();
    out.claims[0].id.clone()
}

/// The view of one claim from `status`, by id.
pub fn view_at(store: &MemStore, p: &mut Providers, id: &ClaimId, at: &str) -> ClaimView {
    let q = StatusQuery {
        ids: vec![id.to_string()],
        ..StatusQuery::default()
    };
    let out = engine::status(store, p, &no_git(), &q, None, ts(at)).unwrap();
    assert_eq!(out.claims.len(), 1);
    out.claims.into_iter().next().unwrap()
}

pub fn view(store: &MemStore, p: &mut Providers, id: &ClaimId) -> ClaimView {
    view_at(store, p, id, NOW)
}

pub fn codes(errors: &[Error]) -> Vec<(Code, Option<usize>)> {
    errors.iter().map(|e| (e.code, e.index)).collect()
}

// ---------------------------------------------------------------------------
// A real git repository, for coverage and `--since`
// ---------------------------------------------------------------------------

/// Run the real git with an isolated configuration; panic on failure.
pub fn sh_git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
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

/// A new repository holding `files` (path, content), not yet committed.
pub fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    sh_git(dir.path(), &["init", "-q"]);
    write_files(dir.path(), files);
    dir
}

pub fn write_files(root: &std::path::Path, files: &[(&str, &str)]) {
    for (path, content) in files {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
}

pub fn commit_all(root: &std::path::Path, message: &str) -> String {
    sh_git(root, &["add", "-A"]);
    sh_git(root, &["commit", "-q", "-m", message]);
    sh_git(root, &["rev-parse", "HEAD"]).trim().to_owned()
}

/// Commit the ids of `store`'s claims as the claims shard `.hibi/claims/0.jsonl`
/// on top of `HEAD`, and nothing else of the working tree. The engine tests
/// keep their records in memory; `status --since` asks the base commit which
/// claims existed there (section 10.3), so a fixture whose claims are meant
/// to be older than the range says so with this. Returns the new commit.
pub fn commit_claim_ids(root: &std::path::Path, store: &MemStore) -> String {
    let lines: String = store
        .ledger()
        .claims
        .iter()
        .map(|c| format!("{{\"id\":\"{}\"}}\n", c.id))
        .collect();
    let rel = ".hibi/claims/0.jsonl";
    write_files(root, &[(rel, &lines)]);
    sh_git(root, &["add", rel]);
    sh_git(root, &["commit", "-q", "-m", "claims", "--", rel]);
    sh_git(root, &["rev-parse", "HEAD"]).trim().to_owned()
}

/// The installed git, which the coverage and `--since` tests need.
pub fn real_git() -> hibi::git::Git {
    GitStatus::probe()
        .require()
        .expect("the engine coverage tests need git 2.45.0 or later")
        .clone()
}
