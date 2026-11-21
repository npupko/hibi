//! Wiring: turn parsed arguments into one call of a library function and
//! hand its output to the renderer. No behaviour of the specification lives
//! here (spec section 2.1).

use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use hibi::cache::{self, Cache};
use hibi::code::Class;
use hibi::config::{self, RepoConfig, UserConfig};
use hibi::engine::{
    self, Builtin, ClaimInput, ClaimState, CoverEnv, CoverQuery, ListQuery, MarkInput,
    ReanchorInput, RetireInput, Since, SinceRun, StatusQuery, Warning,
};
use hibi::git::GitStatus;
use hibi::judge::{self, Judge, JudgeEnv, SuggestQuery, TriageQuery};
use hibi::model::Timestamp;
use hibi::provider::Providers;
use hibi::provider::command::{CommandProvider, check_provider};
use hibi::provider::file::{FileOptions, FileProvider, UnitCache};
use hibi::store::git_text::{self, GitTextStore};
use hibi::{Code, Error, build_info, fingerprint};

use crate::cli::{
    ClaimArgs, Cli, Command, CompletionsArgs, CoverArgs, Global, ListArgs, MarkArgs, ProvidersArgs,
    ReanchorArgs, RetireArgs, SchemaArgs, StateArg, StatusArgs, SuggestEvidenceArgs, TriageArgs,
};
use crate::output::{InitOutput, ProvidersReply, Reply, SchemaOutput, VersionOutput};
use crate::{human, schema};

type Failure = Vec<Error>;

fn one(e: Error) -> Failure {
    vec![e]
}

/// Run the parsed command line.
pub fn run(cli: Cli) -> Result<Reply, Failure> {
    let global = &cli.global;
    // A malformed global option fails every command that accepts it,
    // whether or not the command reads the clock.
    if let Some(text) = &global.now {
        Timestamp::parse_instant(text).map_err(one)?;
    }
    match cli.command {
        Command::Version => version(),
        Command::Schema(args) => schema_command(&args),
        Command::Completions(args) => Ok(completions(&args)),
        Command::Init => init(global).map_err(one),
        Command::Claim(args) => claim(global, args),
        Command::Status(args) => status(global, &args).map_err(one),
        Command::Mark(args) => mark(global, args),
        Command::Reanchor(args) => reanchor(global, args).map_err(one),
        Command::Retire(args) => retire(global, args).map_err(one),
        Command::Cover(args) => cover(global, &args).map_err(one),
        Command::List(args) => list(global, args).map_err(one),
        Command::Triage(args) => triage(global, &args).map_err(one),
        Command::SuggestEvidence(args) => suggest_evidence(global, &args).map_err(one),
        Command::Providers(args) => providers(global, &args).map_err(one),
    }
}

// ---------------------------------------------------------------------------
// The pieces every store command needs
// ---------------------------------------------------------------------------

fn cwd() -> Result<PathBuf, Error> {
    std::env::current_dir().map_err(|e| Error::io("the working directory", &e))
}

/// `now`: the value of `--now`, or the wall clock.
fn now(global: &Global) -> Result<Timestamp, Error> {
    match &global.now {
        Some(text) => Timestamp::parse_instant(text),
        None => Timestamp::from_instant(jiff::Timestamp::now()),
    }
}

/// `--by`, or `HIBI_BY`.
fn by(arg: Option<String>) -> Option<String> {
    arg.or_else(|| std::env::var("HIBI_BY").ok())
}

/// The root (section 12.5) and its `hibi.toml`. Every command that reads
/// the store goes through here, so a legacy store, a missing store and a
/// bad `hibi.toml` stop the run before anything else happens.
struct Repo {
    root: PathBuf,
    config: RepoConfig,
    store: GitTextStore,
}

fn repo(global: &Global) -> Result<Repo, Error> {
    open_repo(find_root(global)?)
}

/// The root (section 12.5), from `--root` or by discovery. A legacy store
/// and a missing store are found here; nothing of the store is read.
fn find_root(global: &Global) -> Result<PathBuf, Error> {
    match &global.root {
        Some(dir) => git_text::check_root(dir),
        None => git_text::discover_root(&cwd()?),
    }
}

/// Read `hibi.toml` of this root and open its store.
fn open_repo(root: PathBuf) -> Result<Repo, Error> {
    let config = RepoConfig::load(&root)?;
    let store = GitTextStore::open(&root)?;
    Ok(Repo {
        root,
        config,
        store,
    })
}

/// The root and store of a judge command (section 14.1). After the key,
/// a judge command checks git before anything else: the root is found,
/// then git must be usable and the root a git top level
/// (`not_a_git_repository`, `git_too_old`), and only then are `hibi.toml`
/// and the store read. The git status is returned so that git is probed
/// once per run.
fn judge_repo(global: &Global) -> Result<(Repo, GitStatus), Error> {
    let root = find_root(global)?;
    let git = GitStatus::probe();
    git.require()?.check_root(&root)?;
    Ok((open_repo(root)?, git))
}

/// What a command that resolves anchors needs besides the store.
struct Wired {
    user: UserConfig,
    git: GitStatus,
    cache: Option<Rc<Cache>>,
    /// `cache_unavailable`, when the cache could not be opened.
    cache_warning: Option<Warning>,
    providers: Providers,
}

fn open_cache(global: &Global) -> (Option<Rc<Cache>>, Option<Warning>) {
    if global.no_cache {
        return (None, None);
    }
    let Some(path) = cache::default_path() else {
        return (None, None);
    };
    match Cache::open(&path) {
        Ok(c) => (Some(Rc::new(c)), None),
        Err(unavailable) => (None, Some(Warning::from(&unavailable))),
    }
}

fn file_options(
    config: &RepoConfig,
    user: &UserConfig,
    git: &GitStatus,
    cache: &Option<Rc<Cache>>,
) -> FileOptions {
    FileOptions {
        repo: config.clone(),
        user: user.clone(),
        git: git.clone(),
        cache: cache.clone().map(|c| Box::new(c) as Box<dyn UnitCache>),
    }
}

/// Build the provider registry: the working-tree file provider for `file`
/// and `repo`, one command provider per scheme that `hibi.toml` lists and
/// the user config registers (started on first use), and a declaration for
/// each listed scheme without a registration (nothing is started for it).
fn wire(repo: &Repo, global: &Global) -> Result<Wired, Error> {
    wire_with(repo, global, None)
}

/// [`wire`] with the git status of a run that already probed git (a judge
/// command, [`judge_repo`]).
fn wire_with(repo: &Repo, global: &Global, probed: Option<GitStatus>) -> Result<Wired, Error> {
    let user = UserConfig::load(config::user_config_path().as_deref())?;
    let git = probed.unwrap_or_else(GitStatus::probe);
    let (cache, cache_warning) = open_cache(global);

    let mut providers = Providers::new();
    providers.register(
        &["file", "repo"],
        Box::new(FileProvider::working_tree(
            &repo.root,
            file_options(&repo.config, &user, &git, &cache),
        )),
    );
    for scheme in repo.config.schemes.keys() {
        match user.providers.get(scheme) {
            Some(registered) => providers.register(
                &[scheme],
                Box::new(CommandProvider::new(
                    scheme.clone(),
                    registered.command.clone(),
                    &repo.root,
                    Duration::from_millis(registered.timeout_ms),
                )),
            ),
            None => providers.declare_unregistered(scheme),
        }
    }
    Ok(Wired {
        user,
        git,
        cache,
        cache_warning,
        providers,
    })
}

impl Wired {
    /// For `--since`: the base commit and a file provider that reads it.
    /// Needs a usable git and a root that is a git top level.
    fn since(&self, repo: &Repo, range: &str) -> Result<(Since, FileProvider), Error> {
        let git = self.git.require()?;
        git.check_root(&repo.root)?;
        let since = engine::resolve_since(git, &repo.root, range)?;
        let base = FileProvider::revision(
            &repo.root,
            since.base.clone(),
            file_options(&repo.config, &self.user, &self.git, &self.cache),
        )?;
        Ok((since, base))
    }

    /// Add `cache_unavailable` to the warnings of a result.
    fn add_warnings(&self, warnings: &mut Vec<Warning>) {
        warnings.extend(self.cache_warning.iter().cloned());
    }
}

fn read_stdin() -> Result<String, Error> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("standard input", &e))?;
    String::from_utf8(bytes)
        .map_err(|_| Error::new(Code::InvalidJson, "standard input is not valid UTF-8"))
}

// ---------------------------------------------------------------------------
// Commands that need no root
// ---------------------------------------------------------------------------

fn version() -> Result<Reply, Failure> {
    let output = VersionOutput {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol: vec![hibi::provider::PROTOCOL_VERSION],
        store_version: build_info::STORE_VERSION,
        profiles: vec![
            fingerprint::PROSE_V1.to_owned(),
            fingerprint::AST_V1.to_owned(),
        ],
        grammars: build_info::GRAMMARS
            .iter()
            .map(|(class, _, version)| ((*class).to_owned(), (*version).to_owned()))
            .collect(),
        judge_model: judge::MODEL.to_owned(),
        warnings: Vec::new(),
    };
    Reply::new("version", &output, &[], human::version(&output)).map_err(one)
}

fn schema_command(args: &SchemaArgs) -> Result<Reply, Failure> {
    match &args.name {
        None => {
            let output = SchemaOutput {
                schemas: schema::NAMES.iter().map(|n| (*n).to_owned()).collect(),
                warnings: Vec::new(),
            };
            Reply::new("schema", &output, &[], human::schema_names(&output)).map_err(one)
        }
        Some(name) => schema::text(name).map(Reply::raw).ok_or_else(|| {
            one(
                Error::invalid_argument(format!("{name:?} is not a schema name"))
                    .with_remediation("hibi schema"),
            )
        }),
    }
}

fn completions(args: &CompletionsArgs) -> Reply {
    let mut out = Vec::new();
    let mut command = crate::cli::command();
    clap_complete::generate(args.shell, &mut command, "hibi", &mut out);
    Reply::raw(String::from_utf8_lossy(&out).into_owned())
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

fn init(global: &Global) -> Result<Reply, Error> {
    let start = match &global.root {
        Some(dir) => dir.clone(),
        None => cwd()?,
    };
    let git = GitStatus::probe();
    let top = git.require()?.find_toplevel(&start)?;
    let created = git_text::init(&top)?;
    let output = InitOutput {
        root: top.to_string_lossy().into_owned(),
        created,
        warnings: Vec::new(),
    };
    Reply::new("init", &output, &[], human::init(&output))
}

// ---------------------------------------------------------------------------
// Write commands
// ---------------------------------------------------------------------------

fn claim(global: &Global, args: ClaimArgs) -> Result<Reply, Failure> {
    let repo = repo(global).map_err(one)?;
    let now = now(global).map_err(one)?;
    let default_by = by(None);
    let elements = if args.stdin {
        let mut elements = engine::read_claim_stdin(&read_stdin().map_err(one)?)?;
        for input in elements.iter_mut().flatten() {
            if input.by.is_none() {
                input.by.clone_from(&default_by);
            }
        }
        elements
    } else {
        let anchor = |text: &String| engine::compact_anchor(text);
        let input = ClaimInput {
            statement: args.statement.unwrap_or_default(),
            subject: anchor(&args.subject.unwrap_or_default()).map_err(one)?,
            evidence: args
                .evidence
                .iter()
                .map(anchor)
                .collect::<Result<_, _>>()
                .map_err(one)?,
            recheck: args.recheck,
            note: args.note,
            by: args.by.or(default_by),
        };
        vec![Ok(input)]
    };
    let mut wired = wire(&repo, global).map_err(one)?;
    let mut output = if args.stdin {
        engine::claim_batch(
            &repo.store,
            &mut wired.providers,
            &wired.git,
            &elements,
            now,
        )?
    } else {
        let inputs: Vec<ClaimInput> = elements.into_iter().flatten().collect();
        engine::claim(
            &repo.store,
            &mut wired.providers,
            &wired.git,
            &inputs,
            false,
            now,
        )?
    };
    wired.add_warnings(&mut output.warnings);
    Reply::new("claim", &output, &output.warnings, human::claim(&output)).map_err(one)
}

fn mark(global: &Global, args: MarkArgs) -> Result<Reply, Failure> {
    let repo = repo(global).map_err(one)?;
    let now = now(global).map_err(one)?;
    let default_by = by(None);
    let elements = if args.stdin {
        let mut elements = engine::read_mark_stdin(&read_stdin().map_err(one)?)?;
        for input in elements.iter_mut().flatten() {
            if input.by.is_none() {
                input.by.clone_from(&default_by);
            }
        }
        elements
    } else {
        let by = args.by.or(default_by);
        let inputs: Vec<Result<MarkInput, Error>> = args
            .ids
            .into_iter()
            .map(|claim| MarkInput {
                claim,
                note: args.note.clone(),
                by: by.clone(),
            })
            .map(Ok)
            .collect();
        inputs
    };
    let mut wired = wire(&repo, global).map_err(one)?;
    let mut output = if args.stdin {
        engine::mark_batch(
            &repo.store,
            &mut wired.providers,
            &wired.git,
            &elements,
            now,
        )?
    } else {
        let inputs: Vec<MarkInput> = elements.into_iter().flatten().collect();
        engine::mark(
            &repo.store,
            &mut wired.providers,
            &wired.git,
            &inputs,
            false,
            now,
        )?
    };
    wired.add_warnings(&mut output.warnings);
    Reply::new("mark", &output, &output.warnings, human::mark(&output)).map_err(one)
}

/// `--evidence <n>=<anchor>`: the number and the anchor text.
fn numbered_anchor(arg: &str) -> Result<(usize, &str), Error> {
    arg.split_once('=')
        .and_then(|(n, anchor)| Some((n.parse::<usize>().ok()?, anchor)))
        .ok_or_else(|| {
            Error::usage(format!(
                "--evidence takes <n>=<anchor>, for example 1=file:src/a.rs#name; got {arg:?}"
            ))
            .with_remediation("hibi reanchor --help")
        })
}

fn reanchor(global: &Global, args: ReanchorArgs) -> Result<Reply, Error> {
    let repo = repo(global)?;
    let now = now(global)?;
    let mut evidence = Vec::with_capacity(args.evidence.len());
    for arg in &args.evidence {
        let (n, anchor) = numbered_anchor(arg)?;
        evidence.push((n, engine::compact_anchor(anchor)?));
    }
    let input = ReanchorInput {
        claim: args.id,
        statement: args.statement,
        subject: args
            .subject
            .as_deref()
            .map(engine::compact_anchor)
            .transpose()?,
        evidence,
        add_evidence: args
            .add_evidence
            .iter()
            .map(|a| engine::compact_anchor(a))
            .collect::<Result<_, _>>()?,
        drop_evidence: args.drop_evidence,
        recheck: args.recheck,
        note: args.note,
        by: by(args.by),
    };
    let mut wired = wire(&repo, global)?;
    let mut output = engine::reanchor(&repo.store, &mut wired.providers, &wired.git, &input, now)?;
    wired.add_warnings(&mut output.warnings);
    Reply::new(
        "reanchor",
        &output,
        &output.warnings,
        human::reanchor(&output),
    )
}

fn retire(global: &Global, args: RetireArgs) -> Result<Reply, Error> {
    let repo = repo(global)?;
    let now = now(global)?;
    let input = RetireInput {
        claims: args.ids,
        replaced_by: args.replaced_by,
        note: args.note,
        by: by(args.by),
    };
    let output = engine::retire(&repo.store, &input, now)?;
    Reply::new("retire", &output, &output.warnings, human::retire(&output))
}

// ---------------------------------------------------------------------------
// Read commands
// ---------------------------------------------------------------------------

fn state(arg: StateArg) -> ClaimState {
    match arg {
        StateArg::Fresh => ClaimState::Fresh,
        StateArg::Stale => ClaimState::Stale,
        StateArg::Expired => ClaimState::Expired,
        StateArg::Retired => ClaimState::Retired,
    }
}

fn status(global: &Global, args: &StatusArgs) -> Result<Reply, Error> {
    // `--judge-order` asks for the judge: its key is checked before any
    // other work (section 14.1).
    let client = if args.judge_order {
        Some(judge_client()?)
    } else {
        None
    };
    // With the judge, git is checked next, before `hibi.toml` is read.
    let (repo, probed) = if client.is_some() {
        let (repo, git) = judge_repo(global)?;
        (repo, Some(git))
    } else {
        (repo(global)?, None)
    };
    let now = now(global)?;
    let query = StatusQuery {
        ids: args.ids.clone(),
        states: args.state.iter().copied().map(state).collect(),
        locs: args.loc.clone(),
        detail: args.detail,
    };
    let mut wired = wire_with(&repo, global, probed)?;
    let mut since = match &args.since {
        Some(range) => Some(wired.since(&repo, range)?),
        None => None,
    };
    let run = match &mut since {
        Some((since, base)) => Some(SinceRun {
            since,
            base,
            git: wired.git.require()?,
            root: &repo.root,
        }),
        None => None,
    };
    let mut output = match &client {
        None => engine::status(
            &repo.store,
            &mut wired.providers,
            &wired.git,
            &query,
            run,
            now,
        )?,
        Some(client) => {
            let env = JudgeEnv {
                root: &repo.root,
                config: &repo.config,
                user: &wired.user,
                git: &wired.git,
                cache: wired.cache.as_deref(),
                now,
            };
            judge::judge_order(client, &repo.store, &mut wired.providers, &env, &query, run)?
        }
    };
    wired.add_warnings(&mut output.warnings);
    let default_selection =
        args.ids.is_empty() && args.state.is_empty() && args.loc.is_empty() && args.since.is_none();
    Reply::new(
        "status",
        &output,
        &output.warnings,
        human::status(&output, default_selection, args.detail),
    )
}

fn cover(global: &Global, args: &CoverArgs) -> Result<Reply, Error> {
    let repo = repo(global)?;
    let mut wired = wire(&repo, global)?;
    let git = wired.git.require()?;
    git.check_root(&repo.root)?;
    let mut since = match &args.since {
        Some(range) => Some(wired.since(&repo, range)?),
        None => None,
    };
    let run = match &mut since {
        Some((since, base)) => Some(SinceRun {
            since,
            base,
            git,
            root: &repo.root,
        }),
        None => None,
    };
    let env = CoverEnv {
        config: &repo.config,
        user: &wired.user,
        root: &repo.root,
        git,
    };
    let query = CoverQuery {
        scopes: args.scope.clone(),
        limit: args.limit,
    };
    let mut output = engine::cover(&repo.store, &mut wired.providers, &env, &query, run)?;
    output.warnings.extend(wired.cache_warning.iter().cloned());
    Reply::new("cover", &output, &output.warnings, human::cover(&output))
}

fn list(global: &Global, args: ListArgs) -> Result<Reply, Error> {
    let repo = repo(global)?;
    let query = ListQuery {
        id: args.id,
        locs: args.loc,
        text: args.text,
        retired: args.retired,
    };
    let output = engine::list(&repo.store, &query)?;
    let warnings = match &output {
        engine::ListOutput::Claims { warnings, .. }
        | engine::ListOutput::Record { warnings, .. } => warnings.clone(),
    };
    Reply::new("list", &output, &warnings, human::list(&output))
}

fn providers(global: &Global, args: &ProvidersArgs) -> Result<Reply, Error> {
    let repo = repo(global)?;
    let user_path = config::user_config_path();
    let user = UserConfig::load(user_path.as_deref())?;
    let git = GitStatus::probe();
    let builtin = Builtin {
        schemes: vec!["file".to_owned(), "repo".to_owned()],
        classes: Class::ALL.iter().map(|c| c.name().to_owned()).collect(),
        profiles: vec![
            fingerprint::PROSE_V1.to_owned(),
            fingerprint::AST_V1.to_owned(),
        ],
    };
    let check = match &args.check {
        Some(scheme) => Some(check_provider(&repo.config, &user, &repo.root, scheme)?),
        None => None,
    };
    let report = engine::providers_report(
        builtin,
        &repo.config,
        &user,
        &repo.root,
        &git,
        user_path.as_deref(),
    );
    let output = ProvidersReply { report, check };
    Reply::new(
        "providers",
        &output,
        &output.report.warnings,
        human::providers(&output),
    )
}

// ---------------------------------------------------------------------------
// Judge commands (section 14)
// ---------------------------------------------------------------------------

/// The first step of every judge command (section 14.1): without
/// `TYPESAFE_API_KEY` this fails with `judge_unavailable` before any other
/// work, including root discovery. The library never reads the
/// environment; both values are passed in.
fn judge_client() -> Result<Judge, Error> {
    Judge::new(
        std::env::var(judge::KEY_VAR).ok().as_deref(),
        std::env::var(judge::ENDPOINT_VAR).ok().as_deref(),
    )
}

fn triage(global: &Global, args: &TriageArgs) -> Result<Reply, Error> {
    let client = judge_client()?;
    let (repo, probed) = judge_repo(global)?;
    let now = now(global)?;
    let mut wired = wire_with(&repo, global, Some(probed))?;
    let git = wired.git.require()?;
    let mut since = match &args.since {
        Some(range) => Some(wired.since(&repo, range)?),
        None => None,
    };
    let run = match &mut since {
        Some((since, base)) => Some(SinceRun {
            since,
            base,
            git,
            root: &repo.root,
        }),
        None => None,
    };
    let env = JudgeEnv {
        root: &repo.root,
        config: &repo.config,
        user: &wired.user,
        git: &wired.git,
        cache: wired.cache.as_deref(),
        now,
    };
    let query = TriageQuery {
        scopes: args.scope.clone(),
        limit: args.limit,
    };
    let mut output = judge::triage(
        &client,
        &repo.store,
        &mut wired.providers,
        &env,
        &query,
        run,
    )?;
    output.warnings.extend(wired.cache_warning.iter().cloned());
    Reply::new("triage", &output, &output.warnings, human::triage(&output))
}

fn suggest_evidence(global: &Global, args: &SuggestEvidenceArgs) -> Result<Reply, Error> {
    let client = judge_client()?;
    let (repo, probed) = judge_repo(global)?;
    let now = now(global)?;
    let mut wired = wire_with(&repo, global, Some(probed))?;
    let env = JudgeEnv {
        root: &repo.root,
        config: &repo.config,
        user: &wired.user,
        git: &wired.git,
        cache: wired.cache.as_deref(),
        now,
    };
    let query = SuggestQuery {
        statement: args.statement.clone(),
        within: args.r#in.clone(),
        kinds: args.kind.clone(),
        limit: args.limit,
    };
    let mut output = judge::suggest_evidence(&client, &mut wired.providers, &env, &query)?;
    output.warnings.extend(wired.cache_warning.iter().cloned());
    Reply::new(
        "suggest-evidence",
        &output,
        &output.warnings,
        human::suggest_evidence(&output),
    )
}
