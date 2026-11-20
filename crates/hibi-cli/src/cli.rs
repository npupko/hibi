//! The clap definition of the `hibi` command line (spec section 15). The
//! binary, the two generator examples and the tests all use this one
//! definition (spec section 2.7).

use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};

const ABOUT: &str =
    "Record that a statement was checked against evidence, and report what changed since";

const LONG_ABOUT: &str = "\
hibi records that a statement was checked against evidence. Later it reports \
which statements need a new check because their text or their evidence \
changed, and which parts of a declared scope have no claim at all.

hibi decides nothing about whether a statement is true. You check it, hibi \
remembers that you did and what the text and the evidence looked like then.";

const AFTER_HELP: &str = "\
Anchors:
  An anchor names one unit of a resource: a sentence, a heading, a function.
    file:docs/retry.md                     the whole file
    file:src/retry.rs#fetch_with_retry     the declaration with this symbol
    file:docs/retry.md::capped at 30 s     the unit that contains this quote
    file:src/retry.rs#retry::MAX = 5       a quote inside a symbol
  Use repo:<alias>//<path> for a file in a sibling repository. A path that
  contains '#' or '::', and any selector for an external scheme, is given as
  JSON on standard input (claim --stdin).

Output:
  The result is JSON when standard output is not a terminal, or with
  --format json: one object with \"ok\", \"command\" and the result keys, or
  with \"ok\": false and \"errors\". Otherwise the result is text.

Exit status:
  0 when the command ran, whatever it found. 1 on any error.

Environment:
  HIBI_BY            default for --by
  HIBI_CONFIG        path of the user config file
  HIBI_CACHE_DIR     directory of the cache database
  TYPESAFE_API_KEY   key for the judge commands (triage, suggest-evidence,
                     status --judge-order); nothing else uses the network
  HIBI_JEV_URL       address of the judge service, for tests
  NO_COLOR           turn colour off in help output";

/// The whole command line.
#[derive(Debug, Parser)]
#[command(
    name = "hibi",
    bin_name = "hibi",
    about = ABOUT,
    long_about = LONG_ABOUT,
    after_long_help = AFTER_HELP,
    disable_help_subcommand = true,
    subcommand_required = true,
    subcommand_value_name = "command",
    subcommand_help_heading = "Commands",
    max_term_width = 100
)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Command,
}

/// The options every command accepts (spec section 15.1).
#[derive(Debug, Clone, Default, Args)]
pub struct Global {
    /// Use this directory as the root (the directory that contains .hibi/)
    /// instead of searching upward from the working directory
    #[arg(
        long,
        global = true,
        help_heading = "Global options",
        value_name = "dir"
    )]
    pub root: Option<PathBuf>,

    /// Output format. Without this option the output is JSON when standard
    /// output is not a terminal, and text when it is
    #[arg(
        long,
        global = true,
        help_heading = "Global options",
        value_enum,
        value_name = "format"
    )]
    pub format: Option<Format>,

    /// Use this instant as the current time, for reproducible runs. RFC
    /// 3339 with any offset, for example 2026-11-20T18:04:11Z. A fraction of
    /// a second is dropped
    #[arg(
        long,
        global = true,
        help_heading = "Global options",
        value_name = "timestamp"
    )]
    pub now: Option<String>,

    /// Do not read or write the cache database. Results are the same with
    /// and without the cache
    #[arg(long, global = true, help_heading = "Global options")]
    pub no_cache: bool,
}

/// `--format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Text for a person
    Human,
    /// One JSON object
    Json,
}

/// `status --state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StateArg {
    /// Some mark matches the current content and is recent enough
    Fresh,
    /// No mark matches the current content
    Stale,
    /// A mark matches the content, but the recheck period has passed
    Expired,
    /// The claim was retired
    Retired,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create the .hibi/ store in this git repository
    #[command(long_about = "\
Create the .hibi/ store at the top level of the git repository that contains \
the working directory: hibi.toml, the claim and mark files, and the files \
that tell git how to merge them. Nothing is written outside .hibi/.

After init, declare scopes in .hibi/hibi.toml to say which documents and \
source files must be covered by claims.")]
    Init,

    /// Record a statement, what it is about and its evidence, as checked now
    #[command(long_about = "\
Record a claim: a statement, the unit it is about (the subject) and the units \
that support it (the evidence). hibi stores the claim and a first mark with \
the fingerprint of every side as it is now. Run this after you checked that \
the statement is true.

Every anchor must match exactly one unit.

Running the same claim again adds a new mark and no second claim. A live \
claim is the same claim when it has the same statement, recheck period and \
note, and each of its anchors has the same locator, resolves to the same \
unit and has the same quote and symbol. Text edited near a quote does not \
make a new claim. The result says \"created\": false and gives the id of the \
existing claim.

With --stdin, standard input is a JSON array of claims, written all or not \
at all:
  [{\"statement\": \"...\", \"subject\": \"file:docs/a.md::quoted text\",
    \"evidence\": [\"file:src/a.rs#name\"], \"recheck\": \"P90D\",
    \"note\": \"...\", \"by\": \"...\"}]

An anchor is a compact string or an object {\"loc\": ..., \"sel\": {...}}. \
Only \"statement\" and \"subject\" are required. `hibi schema claim-input` \
prints the schema.")]
    Claim(ClaimArgs),

    /// Report which claims need a new check and why
    #[command(long_about = "\
Compare every claim with the current content. A claim is fresh when some \
mark matches the current fingerprint of every side, stale when none does, \
expired when a mark matches but the recheck period has passed, and retired \
when it was retired.

Without arguments every claim that needs attention is listed: the stale and \
expired ones, and fresh claims that drifted. A claim drifted when one of its \
anchors was found only approximately, because the text it quotes was \
reworded or its symbol was renamed: the claim still holds, and `hibi \
reanchor` brings the stored anchor up to date.

Each listed claim shows the sides that are not the same as at its mark, and \
the next command to run. Re-check a stale or expired statement before you \
run that command. The summary always counts every claim in the store.")]
    Status(StatusArgs),

    /// Record that claims were checked again and still hold
    #[command(long_about = "\
Add a mark to each claim: a record that you checked the statement again \
against the current content and it still holds. The claims become fresh.

Every side must resolve to one unit. If a side is missing or matches several \
units, use `hibi reanchor` to point it at the right unit. If the statement is \
no longer true, fix the text or use `hibi retire`.

With --stdin, standard input is a JSON array, written all or not at all. \
Each element is a claim id or {\"claim\": \"<id>\", \"note\": \"...\", \
\"by\": \"...\"}. `hibi schema mark-input` prints the schema.")]
    Mark(MarkArgs),

    /// Replace a claim with one that has a new statement, anchors or period
    #[command(long_about = "\
Build a new claim from an existing one and retire the old one, in one write. \
Claims are never edited in place: a change of the statement, an anchor, the \
recheck period or the note gives a new claim id. The new claim gets a mark, \
so run this only after you checked the statement.

Sides you do not replace are resolved again from their stored anchors, and \
every stored selector is refreshed from the unit it resolves to. Without any \
option this refreshes the selectors of a claim whose text moved or was \
reworded slightly.

Evidence numbers start at 1 and always refer to the old claim's list.")]
    Reanchor(ReanchorArgs),

    /// Retire claims that no longer apply
    #[command(long_about = "\
Retire each claim. A retired claim is kept in the store, is no longer \
checked and no longer counts as covering anything. Retirement is permanent.

Use this when a statement was removed or is no longer true. No provider is \
contacted.")]
    Retire(RetireArgs),

    /// List the units in the declared scopes that no claim covers
    #[command(long_about = "\
For every scope declared in .hibi/hibi.toml, count its units and list the \
ones no claim covers. A unit is covered when a side of a live claim resolves \
to it or to a unit nested in it, whatever the claim's state. Units matched \
by an exclusion are counted separately, with the reason given in hibi.toml.

Each listed unit comes with an anchor that can be passed to `hibi claim \
--stdin` as it is. This command needs git.")]
    Cover(CoverArgs),

    /// List stored claims, or show one claim with its history
    #[command(long_about = "\
Read the store and print claims as they are stored. Nothing is resolved and \
no state is computed, so this works when files or providers are missing.

Without an id: one line per live claim. With an id: the stored claim, its \
marks in time order and its retirement.")]
    List(ListArgs),

    /// Rank uncovered prose by how likely it states a checkable fact (judge)
    #[command(long_about = "\
Ask the judge model which uncovered sentences, list items, table rows and \
headings state a fact that could be confirmed by reading code or \
configuration, and where the evidence would be. The result is a ranking to \
read, not a finding: nothing is written to the store.

Needs TYPESAFE_API_KEY and git. The text of the listed units is sent to the \
judge service. Only files that git tracks, or that are untracked and not \
ignored, are sent.")]
    Triage(TriageArgs),

    /// Rank code units that may be evidence for a statement (judge)
    #[command(
        name = "suggest-evidence",
        long_about = "\
Ask the judge model which code units, among those matched by --in, decide \
whether a statement is true. The result is a list of units to read, never a \
finding: nothing is written to the store.

Needs TYPESAFE_API_KEY and git. The text of the candidate units is sent to \
the judge service. Only files that git tracks, or that are untracked and not \
ignored, are sent."
    )]
    SuggestEvidence(SuggestEvidenceArgs),

    /// Show the built-in file provider, sibling repositories and schemes
    #[command(long_about = "\
Show what can resolve locators here: the built-in file provider, every \
sibling repository alias of .hibi/hibi.toml with the directory your user \
config allows for it, and every external scheme with the command your user \
config registers for it. For anything that is not set up, the lines to add \
to your user config are printed.

Without --check no provider command is started.")]
    Providers(ProvidersArgs),

    /// Print the JSON Schema of a record, an input or a command's output
    #[command(long_about = "\
Without a name: list the schema names. With a name: print that JSON Schema \
(draft 2020-12) and nothing else, in both output formats.

Names: the stored records (claim, mark, retirement, anchor, selector), the \
configuration files (config, user-config), the standard-input documents \
(claim-input, mark-input), the error output (error), the provider protocol \
(unit, provider-request, provider-response, conformance-case), and \
<command>-output for each command with JSON output.")]
    Schema(SchemaArgs),

    /// Print a shell completion script
    #[command(long_about = "\
Print the completion script for the given shell to standard output. There is \
no JSON form.")]
    Completions(CompletionsArgs),

    /// Print the hibi version
    #[command(long_about = "\
Print the hibi version. The JSON form also gives the provider protocol \
versions, the store version, the fingerprint profiles, the grammar versions \
and the judge model.")]
    Version,
}

#[derive(Debug, Args)]
pub struct ClaimArgs {
    /// The statement that was checked
    #[arg(
        value_name = "statement",
        required_unless_present = "stdin",
        conflicts_with = "stdin"
    )]
    pub statement: Option<String>,

    /// What the statement is about: an anchor, usually the sentence in a
    /// document that makes the statement
    #[arg(
        long,
        value_name = "anchor",
        required_unless_present = "stdin",
        conflicts_with = "stdin"
    )]
    pub subject: Option<String>,

    /// A unit that supports the statement: an anchor, usually a function or
    /// another declaration. Repeat for several
    #[arg(long, value_name = "anchor", conflicts_with = "stdin")]
    pub evidence: Vec<String>,

    /// Check again after this long even if nothing changed: an ISO 8601
    /// duration with whole numbers, for example P90D or P6M
    #[arg(long, value_name = "duration", conflicts_with = "stdin")]
    pub recheck: Option<String>,

    /// A note stored with the claim: how to check it again, or where the
    /// truth lives
    #[arg(long, value_name = "text", conflicts_with = "stdin")]
    pub note: Option<String>,

    /// Who checked, stored with the mark for the record
    /// [default: the value of HIBI_BY]
    #[arg(long, value_name = "text", conflicts_with = "stdin")]
    pub by: Option<String>,

    /// Read a JSON array of claims from standard input. No other claim
    /// argument is allowed with it
    #[arg(long)]
    pub stdin: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Show exactly these claims, in any state. An id may be shortened to a
    /// unique prefix of at least 4 characters after "c1-"
    #[arg(value_name = "claim-id")]
    pub ids: Vec<String>,

    /// List only the stale claims that the changes since a commit caused.
    /// The range is A, A.. or A..HEAD (since commit A), or A... or A...HEAD
    /// (since the merge base of A and HEAD). The compared end is always the
    /// working tree. Cannot be combined with --state or with ids
    #[arg(long, value_name = "range", conflicts_with_all = ["ids", "state"])]
    pub since: Option<String>,

    /// Keep a claim only when the locator of one of its sides matches this
    /// glob, for example 'file:src/**'. Repeat for several
    #[arg(long, value_name = "glob")]
    pub loc: Vec<String>,

    /// List claims in this state. Repeat for several
    /// [default: stale, expired, and fresh claims that drifted]
    #[arg(long, value_enum, value_name = "state")]
    pub state: Vec<StateArg>,

    /// Include the current text of every side
    #[arg(long)]
    pub detail: bool,

    /// Order the stale claims by how likely each is broken, most likely
    /// first, as rated by the judge model. Needs TYPESAFE_API_KEY and git.
    /// The order is the only thing that changes
    #[arg(long)]
    pub judge_order: bool,
}

#[derive(Debug, Args)]
pub struct MarkArgs {
    /// The claims that were checked again. An id may be shortened to a
    /// unique prefix
    #[arg(
        value_name = "claim-id",
        required_unless_present = "stdin",
        conflicts_with = "stdin"
    )]
    pub ids: Vec<String>,

    /// A note stored with each mark
    #[arg(long, value_name = "text", conflicts_with = "stdin")]
    pub note: Option<String>,

    /// Who checked, stored with each mark for the record
    /// [default: the value of HIBI_BY]
    #[arg(long, value_name = "text", conflicts_with = "stdin")]
    pub by: Option<String>,

    /// Read a JSON array of claim ids or objects from standard input. No
    /// other mark argument is allowed with it
    #[arg(long)]
    pub stdin: bool,
}

#[derive(Debug, Args)]
pub struct ReanchorArgs {
    /// The claim to replace. May be shortened to a unique prefix
    #[arg(value_name = "claim-id")]
    pub id: String,

    /// The new statement
    #[arg(long, value_name = "text")]
    pub statement: Option<String>,

    /// A new anchor for the subject
    #[arg(long, value_name = "anchor")]
    pub subject: Option<String>,

    /// A new anchor for evidence number n, for example 1=file:src/a.rs#name.
    /// Repeat for several
    #[arg(long, value_name = "n=anchor")]
    pub evidence: Vec<String>,

    /// Add an evidence anchor at the end of the list. Repeat for several
    #[arg(long, value_name = "anchor")]
    pub add_evidence: Vec<String>,

    /// Remove evidence number n. Repeat for several
    #[arg(long, value_name = "n")]
    pub drop_evidence: Vec<usize>,

    /// A new recheck period, or "none" to remove it
    #[arg(long, value_name = "duration|none")]
    pub recheck: Option<String>,

    /// A new note, or "none" to remove it
    #[arg(long, value_name = "text|none")]
    pub note: Option<String>,

    /// Who checked, stored with the mark and the retirement
    /// [default: the value of HIBI_BY]
    #[arg(long, value_name = "text")]
    pub by: Option<String>,
}

#[derive(Debug, Args)]
pub struct RetireArgs {
    /// The claims to retire. An id may be shortened to a unique prefix
    #[arg(value_name = "claim-id", required = true)]
    pub ids: Vec<String>,

    /// The claim that takes the place of the retired ones. It must exist
    /// and must not be one of them
    #[arg(long, value_name = "claim-id")]
    pub replaced_by: Option<String>,

    /// A note stored with each retirement, for example why
    #[arg(long, value_name = "text")]
    pub note: Option<String>,

    /// Who retired, stored for the record [default: the value of HIBI_BY]
    #[arg(long, value_name = "text")]
    pub by: Option<String>,
}

#[derive(Debug, Args)]
pub struct CoverArgs {
    /// Report only this scope. Repeat for several [default: every scope]
    #[arg(long, value_name = "name")]
    pub scope: Vec<String>,

    /// Report only units that were added or changed since a commit. The
    /// range is written as for `hibi status --since`. Only scopes on scheme
    /// "file" are evaluated
    #[arg(long, value_name = "range")]
    pub since: Option<String>,

    /// List at most this many uncovered units per scope; 0 lists all. The
    /// counts are never cut
    #[arg(long, value_name = "n", default_value_t = 200)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Show this claim as stored, with its marks and retirement. May be
    /// shortened to a unique prefix
    #[arg(value_name = "claim-id")]
    pub id: Option<String>,

    /// Keep a claim only when the locator of one of its sides matches this
    /// glob. Repeat for several
    #[arg(long, value_name = "glob")]
    pub loc: Vec<String>,

    /// Keep a claim only when its statement contains this text. The match
    /// is case-sensitive
    #[arg(long, value_name = "substring")]
    pub text: Option<String>,

    /// Include retired claims
    #[arg(long)]
    pub retired: bool,
}

#[derive(Debug, Args)]
pub struct TriageArgs {
    /// Consider only this scope. Repeat for several [default: every scope]
    #[arg(long, value_name = "name")]
    pub scope: Vec<String>,

    /// Consider only units that were added or changed since a commit. The
    /// range is written as for `hibi status --since`
    #[arg(long, value_name = "range")]
    pub since: Option<String>,

    /// Print at most this many units; 0 prints all
    #[arg(long, value_name = "n", default_value_t = 50)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct SuggestEvidenceArgs {
    /// The statement to find evidence for
    #[arg(value_name = "statement")]
    pub statement: String,

    /// Where to look: a locator glob on scheme "file" or "repo", for
    /// example 'file:src/**/*.rs'. Repeat for several
    #[arg(long = "in", value_name = "glob", required = true)]
    pub r#in: Vec<String>,

    /// Consider only units of this kind. Repeat for several [default:
    /// function, method, class, struct, interface, trait, enum, type,
    /// variable]
    #[arg(long, value_name = "kind")]
    pub kind: Vec<String>,

    /// Print at most this many candidates
    #[arg(long, value_name = "n", default_value_t = 10)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct ProvidersArgs {
    /// Start the provider registered for this scheme and run the protocol
    /// conformance cases against it. A failing case is reported and does
    /// not make the command fail
    #[arg(long, value_name = "scheme")]
    pub check: Option<String>,
}

#[derive(Debug, Args)]
pub struct SchemaArgs {
    /// The schema to print [default: list the names]
    #[arg(value_name = "name")]
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// The shell to print the script for
    #[arg(value_enum, value_name = "shell")]
    pub shell: clap_complete::Shell,
}

impl Command {
    /// The command name, as in the `command` key of the JSON envelope.
    pub fn name(&self) -> &'static str {
        match self {
            Command::Init => "init",
            Command::Claim(_) => "claim",
            Command::Status(_) => "status",
            Command::Mark(_) => "mark",
            Command::Reanchor(_) => "reanchor",
            Command::Retire(_) => "retire",
            Command::Cover(_) => "cover",
            Command::List(_) => "list",
            Command::Triage(_) => "triage",
            Command::SuggestEvidence(_) => "suggest-evidence",
            Command::Providers(_) => "providers",
            Command::Schema(_) => "schema",
            Command::Completions(_) => "completions",
            Command::Version => "version",
        }
    }
}

/// The clap command definition, as the binary uses it.
pub fn command() -> clap::Command {
    Cli::command()
}

/// The name of every subcommand, in help order.
pub fn command_names() -> Vec<String> {
    command()
        .get_subcommands()
        .map(|c| c.get_name().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_definition_is_consistent() {
        command().debug_assert();
    }

    #[test]
    fn the_commands_are_those_of_the_specification() {
        assert_eq!(
            command_names(),
            [
                "init",
                "claim",
                "status",
                "mark",
                "reanchor",
                "retire",
                "cover",
                "list",
                "triage",
                "suggest-evidence",
                "providers",
                "schema",
                "completions",
                "version"
            ]
        );
    }
}
