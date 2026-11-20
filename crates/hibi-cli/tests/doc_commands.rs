//! R43: every `hibi` command line shown in a code block of the user docs
//! parses with the real clap definition, `hibi_cli::cli::command()`.
//!
//! # Which files
//!
//! `README.md`, every `docs/**/*.mdx` and every `plugins/**/*.md`, except
//! the generated `plugins/hibi-cli/skills/hibi/references/cli-reference.md`,
//! whose usage lines such as `hibi [OPTIONS] <command>` are synopses, not
//! commands. Directories named `node_modules` or starting with `.` are not
//! read.
//!
//! # Which blocks
//!
//! Every fenced code block (```` ``` ```` or `~~~`, three or more, indented
//! or not, because MDX nests blocks inside components), whatever its
//! language, except a block marked as output.
//!
//! # Marking a block as output
//!
//! A block that shows output rather than commands is skipped when the word
//! `output` appears in its info string after the language, for example
//! ```` ```text output ```` or ```` ```json output ````. Mintlify shows that
//! text as the block title, so readers see "output". The marker must be a
//! word of its own after the language: ```` ```output ```` (no language) is
//! not the marker and the block is checked.
//!
//! # How a block is read
//!
//! The text of a block is read as POSIX shell:
//!
//! - backslash-newline joins lines, and a quoted string may span lines;
//! - single quotes, double quotes (where a backslash escapes `"`, `\`, `$`
//!   and a backtick) and a backslash outside quotes work as in the shell;
//! - a word that starts with `#` starts a comment, so a full-line comment
//!   is skipped, while `#` inside a word (`file:src/a.rs#name`) is text;
//! - a leading `$ ` prompt is removed;
//! - commands are split at `|`, `||`, `&&`, `;`, `&`, `(`, `)` and newline;
//! - leading `NAME=value` assignments are dropped;
//! - words after a redirection (`>`, `>>`, `<`, `2>`, `2>&1`, `&>`, ...)
//!   are ignored, and the body of a heredoc (`<<EOF`, `<<'EOF'`, `<<"EOF"`,
//!   `<<-EOF`, any delimiter word) is skipped.
//!
//! A command whose first word is `hibi` must parse: clap may succeed or stop
//! to show help or the version; any other clap error (unknown subcommand or
//! option, invalid value, missing or conflicting arguments) fails the test.
//! hibi's own checks after parsing, such as `--since` with `--state`, are
//! not run.
//!
//! # Command substitution
//!
//! `$(...)` or backticks in a `hibi` command, or a substitution that
//! contains a `hibi` command, is reported as a failure: the test cannot
//! know the words the substitution produces. Set a variable on a line of
//! its own instead, for example `BASE=$(git merge-base main HEAD)` and then
//! `hibi status --since "$BASE"`. A substitution in a line without hibi is
//! not reported.

use std::path::{Path, PathBuf};

use clap::error::ErrorKind;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The generated CLI reference, relative to the repository root.
const REFERENCE: &str = "plugins/hibi-cli/skills/hibi/references/cli-reference.md";

/// R43: every `hibi` command in the docs parses with the clap definition.
#[test]
fn r43_every_hibi_command_in_the_docs_parses() {
    let root = repo_root();
    let mut total = 0;
    let mut failures = Vec::new();
    for file in doc_files(&root) {
        let text =
            std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let name = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string();
        let checked = check_markdown(&name, &text);
        total += checked.commands;
        failures.extend(checked.failures);
    }
    assert!(
        failures.is_empty(),
        "{} hibi command(s) in the docs do not parse (mark a block that shows \
         output with ```<language> output):\n{}",
        failures.len(),
        failures.join("\n")
    );

    // An extractor that finds nothing must not pass: README.md shows hibi
    // commands, so at least one command must have been checked.
    let readme = std::fs::read_to_string(root.join("README.md")).expect("README.md");
    assert!(
        blocks(&readme).iter().any(|b| mentions_hibi(&b.text)),
        "README.md shows no hibi command in a code block"
    );
    assert!(total >= 1, "no hibi command was found in the docs");
}

/// The files to read, in a stable order.
fn doc_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let readme = root.join("README.md");
    if readme.is_file() {
        files.push(readme);
    }
    walk(&root.join("docs"), "mdx", &mut files);
    let reference = root.join(REFERENCE);
    let mut plugins = Vec::new();
    walk(&root.join("plugins"), "md", &mut plugins);
    files.extend(plugins.into_iter().filter(|p| *p != reference));
    files
}

fn walk(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        entries => entries.unwrap_or_else(|e| panic!("{}: {e}", dir.display())),
    };
    let mut paths: Vec<PathBuf> = entries
        .map(|e| {
            e.unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
                .path()
        })
        .collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            walk(&path, extension, out);
        } else if path.is_file() && path.extension().is_some_and(|e| e == extension) {
            out.push(path);
        }
    }
}

/// What one file holds.
struct Checked {
    /// The number of `hibi` commands found.
    commands: usize,
    /// One line per command that does not parse or cannot be checked.
    failures: Vec<String>,
}

/// Check every `hibi` command in the code blocks of a markdown text. `path`
/// is only used in the failure lines.
fn check_markdown(path: &str, markdown: &str) -> Checked {
    let mut checked = Checked {
        commands: 0,
        failures: Vec::new(),
    };
    for simple in hibi_commands(markdown) {
        let is_hibi = simple.words.first().is_some_and(|w| w == "hibi");
        if is_hibi {
            checked.commands += 1;
        }
        let problem = if simple.substitutions.is_empty() {
            parse(&simple.words).err()
        } else {
            Some(
                "uses command substitution, which this test cannot check; \
                 set a variable on a line of its own and use it"
                    .to_owned(),
            )
        };
        if let Some(problem) = problem {
            let shown = if simple.words.is_empty() {
                simple.substitutions.join(" ")
            } else {
                shell_words(&simple.words)
            };
            checked
                .failures
                .push(format!("{path}:{}: {shown} -> {problem}", simple.line));
        }
    }
    checked
}

/// Parse the words with the clap definition. Help and version count as
/// parsed.
fn parse(words: &[String]) -> Result<(), String> {
    let Err(err) = hibi_cli::cli::command().try_get_matches_from(words) else {
        return Ok(());
    };
    match err.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => Ok(()),
        kind => {
            let rendered = err.render().to_string();
            let first = rendered
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or_default();
            let first = first.strip_prefix("error: ").unwrap_or(first);
            Err(format!("{kind:?}: {first}"))
        }
    }
}

/// The words as a shell line, quoting a word that is empty or has a blank.
fn shell_words(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            if w.is_empty() || w.contains(char::is_whitespace) {
                format!("'{w}'")
            } else {
                w.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when the text has the word `hibi`, not as part of a longer word
/// such as `hibi-cli`.
fn mentions_hibi(text: &str) -> bool {
    text.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .any(|w| w == "hibi")
}

/// Every command to check in the markdown: those whose first word is
/// `hibi`, and those with a command substitution that mentions hibi.
fn hibi_commands(markdown: &str) -> Vec<Simple> {
    blocks(markdown)
        .into_iter()
        .filter(|b| !b.is_output())
        .flat_map(|b| lex(&b.text, b.first_line))
        .filter(|s| {
            s.words.first().is_some_and(|w| w == "hibi")
                || s.substitutions.iter().any(|t| mentions_hibi(t))
        })
        .collect()
}

/// A fenced code block.
#[derive(Debug)]
struct Block {
    /// The text after the language in the info string.
    meta: String,
    /// The line of the file of the first line of the text.
    first_line: usize,
    /// The lines between the fences.
    text: String,
}

impl Block {
    /// True when the block is marked as output: the word `output` after
    /// the language.
    fn is_output(&self) -> bool {
        self.meta.split_whitespace().any(|w| w == "output")
    }
}

/// A fence: its character, its length and the text after it.
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start();
    let c = trimmed.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = trimmed.chars().take_while(|&x| x == c).count();
    // The fence character is ASCII, so n characters are n bytes.
    (n >= 3).then(|| (c, n, &trimmed[n..]))
}

/// The fenced code blocks of a markdown text. A block that is not closed
/// runs to the end of the text.
fn blocks(markdown: &str) -> Vec<Block> {
    let mut found = Vec::new();
    let mut open: Option<(char, usize, Block)> = None;
    for (index, line) in markdown.lines().enumerate() {
        let number = index + 1;
        match open.take() {
            None => {
                if let Some((c, n, info)) = fence(line) {
                    if c == '`' && info.contains('`') {
                        continue;
                    }
                    let info = info.trim();
                    let meta = info
                        .split_once(char::is_whitespace)
                        .map_or("", |(_, rest)| rest)
                        .to_owned();
                    let block = Block {
                        meta,
                        first_line: number + 1,
                        text: String::new(),
                    };
                    open = Some((c, n, block));
                }
            }
            Some((c, n, mut block)) => {
                let closes = fence(line)
                    .is_some_and(|(d, m, rest)| d == c && m >= n && rest.trim().is_empty());
                if closes {
                    found.push(block);
                } else {
                    block.text.push_str(line);
                    block.text.push('\n');
                    open = Some((c, n, block));
                }
            }
        }
    }
    if let Some((_, _, block)) = open {
        found.push(block);
    }
    found
}

/// A simple command of a code block.
#[derive(Debug, Default)]
struct Simple {
    /// The line of the file where the command starts.
    line: usize,
    /// Its words after quote removal, without leading assignments and
    /// without anything from the first redirection on.
    words: Vec<String>,
    /// The text of each command substitution in it.
    substitutions: Vec<String>,
}

/// A word as the lexer read it.
struct Word {
    text: String,
    /// An unquoted number right before `<` or `>`: a file descriptor.
    fd: bool,
    /// A `NAME=value` assignment.
    assignment: bool,
    substitutions: Vec<String>,
}

/// Split a block text, whose first line is line `first_line` of the file,
/// into simple commands.
fn lex(text: &str, first_line: usize) -> Vec<Simple> {
    let mut lexer = Lexer {
        chars: text.chars().collect(),
        i: 0,
        line: first_line,
        commands: Vec::new(),
        current: Simple::default(),
        redirected: false,
        heredocs: Vec::new(),
        line_start: true,
    };
    lexer.run();
    lexer.commands
}

struct Lexer {
    chars: Vec<char>,
    i: usize,
    line: usize,
    commands: Vec<Simple>,
    current: Simple,
    /// A redirection was read in the current command.
    redirected: bool,
    /// Heredoc delimiters whose bodies start after the next newline.
    heredocs: Vec<String>,
    /// Only blanks since the last newline.
    line_start: bool,
}

impl Lexer {
    fn peek(&self, n: usize) -> Option<char> {
        self.chars.get(self.i + n).copied()
    }

    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            match c {
                '\n' => {
                    self.i += 1;
                    self.line += 1;
                    self.finish();
                    self.skip_heredocs();
                    self.line_start = true;
                }
                ' ' | '\t' | '\r' => self.i += 1,
                '\\' if self.peek(1) == Some('\n') => {
                    self.i += 2;
                    self.line += 1;
                }
                '$' if self.line_start && self.peek(1) == Some(' ') => {
                    self.i += 2;
                    self.line_start = false;
                }
                '#' => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                '<' | '>' => {
                    self.line_start = false;
                    self.redirect();
                }
                '&' if self.peek(1) == Some('>') => {
                    self.line_start = false;
                    self.i += 1;
                    self.redirect();
                }
                '|' | '&' | ';' | '(' | ')' => {
                    self.line_start = false;
                    self.i += 1;
                    self.finish();
                }
                _ => {
                    self.line_start = false;
                    let line = self.line;
                    let word = self.word();
                    if self.current.words.is_empty() && self.current.substitutions.is_empty() {
                        self.current.line = line;
                    }
                    self.current.substitutions.extend(word.substitutions);
                    if word.fd {
                        self.redirect();
                    } else if !self.redirected
                        && !(self.current.words.is_empty() && word.assignment)
                    {
                        self.current.words.push(word.text);
                    }
                }
            }
        }
        self.finish();
    }

    /// End the current simple command.
    fn finish(&mut self) {
        let done = std::mem::take(&mut self.current);
        self.redirected = false;
        if !done.words.is_empty() || !done.substitutions.is_empty() {
            self.commands.push(done);
        }
    }

    /// Read a redirection operator at `<` or `>` and its target word.
    fn redirect(&mut self) {
        self.redirected = true;
        let heredoc =
            self.peek(0) == Some('<') && self.peek(1) == Some('<') && self.peek(2) != Some('<');
        if heredoc {
            self.i += 2;
            if self.peek(0) == Some('-') {
                self.i += 1;
            }
        } else {
            while matches!(self.peek(0), Some('<' | '>' | '&' | '|')) {
                self.i += 1;
            }
        }
        while matches!(self.peek(0), Some(' ' | '\t')) {
            self.i += 1;
        }
        let starts_word = self
            .peek(0)
            .is_some_and(|c| !matches!(c, '\n' | '\r' | '|' | '&' | ';' | '<' | '>' | '(' | ')'));
        if starts_word {
            let target = self.word();
            self.current.substitutions.extend(target.substitutions);
            if heredoc {
                self.heredocs.push(target.text);
            }
        }
    }

    /// Skip the bodies of the pending heredocs, each up to the line that
    /// is its delimiter.
    fn skip_heredocs(&mut self) {
        for delimiter in std::mem::take(&mut self.heredocs) {
            while self.i < self.chars.len() {
                let end = self.chars[self.i..]
                    .iter()
                    .position(|&c| c == '\n')
                    .map_or(self.chars.len(), |p| self.i + p);
                let line: String = self.chars[self.i..end].iter().collect();
                if end < self.chars.len() {
                    self.line += 1;
                    self.i = end + 1;
                } else {
                    self.i = end;
                }
                if line.trim() == delimiter {
                    break;
                }
            }
        }
    }

    /// Read one word, removing quotes and backslashes.
    fn word(&mut self) -> Word {
        let start = self.i;
        let mut text = String::new();
        let mut quoted = false;
        let mut substitutions = Vec::new();
        while let Some(c) = self.peek(0) {
            match c {
                ' ' | '\t' | '\r' | '\n' | '|' | '&' | ';' | '<' | '>' | '(' | ')' => break,
                '\\' => match self.peek(1) {
                    Some('\n') => {
                        self.i += 2;
                        self.line += 1;
                    }
                    Some(next) => {
                        quoted = true;
                        text.push(next);
                        self.i += 2;
                    }
                    None => self.i += 1,
                },
                '\'' => {
                    quoted = true;
                    self.i += 1;
                    while let Some(c) = self.peek(0) {
                        self.i += 1;
                        if c == '\'' {
                            break;
                        }
                        if c == '\n' {
                            self.line += 1;
                        }
                        text.push(c);
                    }
                }
                '"' => {
                    quoted = true;
                    self.i += 1;
                    self.double_quoted(&mut text, &mut substitutions);
                }
                '$' if self.peek(1) == Some('(') => {
                    let s = self.substitution();
                    text.push_str(&s);
                    substitutions.push(s);
                }
                '`' => {
                    let s = self.backticks();
                    text.push_str(&s);
                    substitutions.push(s);
                }
                c => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        let raw: String = self.chars[start..self.i].iter().collect();
        let fd = !quoted
            && !text.is_empty()
            && text.chars().all(|c| c.is_ascii_digit())
            && matches!(self.peek(0), Some('<' | '>'));
        Word {
            text,
            fd,
            assignment: is_assignment(&raw),
            substitutions,
        }
    }

    /// Read the rest of a double-quoted string, after its opening quote.
    fn double_quoted(&mut self, text: &mut String, substitutions: &mut Vec<String>) {
        while let Some(c) = self.peek(0) {
            match c {
                '"' => {
                    self.i += 1;
                    return;
                }
                '\\' => match self.peek(1) {
                    Some('\n') => {
                        self.i += 2;
                        self.line += 1;
                    }
                    Some(next @ ('"' | '\\' | '$' | '`')) => {
                        text.push(next);
                        self.i += 2;
                    }
                    _ => {
                        text.push('\\');
                        self.i += 1;
                    }
                },
                '$' if self.peek(1) == Some('(') => {
                    let s = self.substitution();
                    text.push_str(&s);
                    substitutions.push(s);
                }
                '`' => {
                    let s = self.backticks();
                    text.push_str(&s);
                    substitutions.push(s);
                }
                c => {
                    if c == '\n' {
                        self.line += 1;
                    }
                    text.push(c);
                    self.i += 1;
                }
            }
        }
    }

    /// Read `$(...)` up to its matching parenthesis and return its text.
    fn substitution(&mut self) -> String {
        let start = self.i;
        self.i += 2;
        let mut depth = 1_usize;
        while let Some(c) = self.peek(0) {
            self.i += 1;
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                '\n' => self.line += 1,
                _ => {}
            }
        }
        self.chars[start..self.i].iter().collect()
    }

    /// Read `` `...` `` and return its text.
    fn backticks(&mut self) -> String {
        let start = self.i;
        self.i += 1;
        while let Some(c) = self.peek(0) {
            self.i += 1;
            if c == '\n' {
                self.line += 1;
            }
            if c == '`' {
                break;
            }
        }
        self.chars[start..self.i].iter().collect()
    }
}

/// True when the raw word starts with `NAME=`.
fn is_assignment(raw: &str) -> bool {
    let Some((name, _)) = raw.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// Tests of the extractor itself.

/// The `hibi` commands of a markdown text, as shell lines.
fn commands_in(markdown: &str) -> Vec<String> {
    hibi_commands(markdown)
        .iter()
        .map(|s| shell_words(&s.words))
        .collect()
}

fn failures_in(markdown: &str) -> Vec<String> {
    check_markdown("doc.md", markdown).failures
}

#[test]
fn extract_skips_a_block_marked_output() {
    let md = "```text output\nhibi frobnicate\n```\n\n~~~json output\nhibi frobnicate\n~~~\n";
    assert!(commands_in(md).is_empty());
    assert!(failures_in(md).is_empty());
    // Without the marker the same block is checked.
    let md = "```text\nhibi frobnicate\n```\n";
    assert_eq!(commands_in(md), ["hibi frobnicate"]);
}

#[test]
fn extract_reads_an_indented_fence_inside_mdx() {
    let md = "<Steps>\n  <Step title=\"Check\">\n    ```bash\n    hibi status --detail\n    ```\n  </Step>\n</Steps>\n";
    let found = hibi_commands(md);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].words, ["hibi", "status", "--detail"]);
    assert_eq!(found[0].line, 4);
    assert!(failures_in(md).is_empty());
}

#[test]
fn extract_takes_the_hibi_side_of_a_pipe() {
    let md = "```bash\necho '[{\"statement\": \"a | b; c\"}]' | hibi claim --stdin\n```\n";
    assert_eq!(commands_in(md), ["hibi claim --stdin"]);
    assert!(failures_in(md).is_empty());
}

#[test]
fn extract_skips_a_heredoc_body() {
    let md = "```bash\nhibi claim --stdin <<'EOF'\n[\n  {\"statement\": \"x\", \"subject\": \"file:a.md\"}\n]\nEOF\nhibi mark --stdin <<-END\n\thibi frobnicate\n\tEND\nhibi status\n```\n";
    assert_eq!(
        commands_in(md),
        ["hibi claim --stdin", "hibi mark --stdin", "hibi status"]
    );
    assert_eq!(hibi_commands(md)[2].line, 10);
    assert!(failures_in(md).is_empty());
}

#[test]
fn extract_joins_continuation_lines() {
    let md = "```bash\nhibi claim \"Retries stop\" \\\n  --subject file:docs/a.md \\\n  --evidence file:src/retry.rs#MAX_ATTEMPTS\nhibi status\n```\n";
    let found = hibi_commands(md);
    assert_eq!(found.len(), 2);
    assert_eq!(
        found[0].words,
        [
            "hibi",
            "claim",
            "Retries stop",
            "--subject",
            "file:docs/a.md",
            "--evidence",
            "file:src/retry.rs#MAX_ATTEMPTS"
        ]
    );
    assert_eq!(found[0].line, 2);
    assert_eq!(found[1].line, 5);
    assert!(failures_in(md).is_empty());
}

#[test]
fn extract_drops_environment_assignments() {
    let md = "```bash\nHIBI_BY=agent hibi mark c1-abcd\n```\n";
    assert_eq!(commands_in(md), ["hibi mark c1-abcd"]);
    assert!(failures_in(md).is_empty());
}

#[test]
fn extract_drops_prompts_comments_and_redirections() {
    let md = "```console\n$ hibi status --format json > out.json # save it\n# hibi frobnicate\nhibi list 2>&1 | jq .\nhibi version && hibi frobnicate\n```\n";
    assert_eq!(
        commands_in(md),
        [
            "hibi status --format json",
            "hibi list",
            "hibi version",
            "hibi frobnicate"
        ]
    );
    let failures = failures_in(md);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].starts_with("doc.md:5: hibi frobnicate -> "));
}

#[test]
fn extract_reports_command_substitution_with_hibi() {
    let md = "```bash\nBASE=$(git merge-base main HEAD)\nhibi status --since \"$BASE\"\nhibi status --since $(git merge-base main HEAD)\nids=$(hibi status --format json)\n```\n";
    let failures = failures_in(md);
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(failures[0].starts_with("doc.md:4: hibi status --since "));
    assert!(failures[0].contains("command substitution"));
    assert!(failures[1].starts_with("doc.md:5: $(hibi status --format json) -> "));
}

#[test]
fn a_bad_option_fails() {
    let failures = failures_in("```bash\nhibi status --bogus\n```\n");
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].starts_with("doc.md:2: hibi status --bogus -> UnknownArgument: "),
        "{failures:?}"
    );
}

#[test]
fn an_unknown_subcommand_fails() {
    let failures = failures_in("```bash\nhibi frobnicate\n```\n");
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].contains("-> InvalidSubcommand: "),
        "{failures:?}"
    );
}

#[test]
fn hibi_version_and_help_parse() {
    let md = "```bash\nhibi version\nhibi --help\nhibi claim --help\n```\n";
    assert_eq!(commands_in(md).len(), 3);
    assert!(failures_in(md).is_empty());
}

#[test]
fn a_quoted_statement_with_spaces_parses() {
    let md = "```bash\nhibi claim \"Retries stop after 5 attempts\" --subject 'file:docs/a.md::stop after 5' --evidence file:src/retry.rs#MAX_ATTEMPTS\n```\n";
    let found = hibi_commands(md);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].words,
        [
            "hibi",
            "claim",
            "Retries stop after 5 attempts",
            "--subject",
            "file:docs/a.md::stop after 5",
            "--evidence",
            "file:src/retry.rs#MAX_ATTEMPTS"
        ]
    );
    assert!(failures_in(md).is_empty(), "{:?}", failures_in(md));
}
