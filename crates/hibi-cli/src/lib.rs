//! The hibi command line: argument parsing, wiring, output rendering and
//! process exit (spec section 15). The library target exposes the clap
//! command definition so that the binary, the generator examples and the
//! tests share it (spec section 2.7).

pub mod cli;
pub mod output;
pub mod reference;
pub mod schema;

mod app;
mod human;

use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use std::sync::Mutex;

use clap::error::ErrorKind;
use clap::{ArgAction, CommandFactory, FromArgMatches};
use hibi::Error;
use hibi::engine::ErrorEnvelope;

use crate::cli::{Cli, Format};
use crate::output::{Body, Reply};

/// The format named on the command line, read before clap parses it so
/// that a usage error is printed in the right format: the last
/// `--format <value>` or `--format=<value>`.
fn format_argument(args: &[OsString]) -> Option<Format> {
    let mut found = None;
    let mut args = args.iter().map(|a| a.to_string_lossy());
    while let Some(arg) = args.next() {
        let value = if arg == "--format" {
            args.next().map(|v| v.into_owned())
        } else {
            arg.strip_prefix("--format=").map(str::to_owned)
        };
        match value.as_deref() {
            Some("json") => found = Some(Format::Json),
            Some("human") => found = Some(Format::Human),
            _ => {}
        }
    }
    found
}

/// The output format of this run (spec section 15.1).
fn format_of(args: &[OsString]) -> Format {
    format_argument(args).unwrap_or(if std::io::stdout().is_terminal() {
        Format::Human
    } else {
        Format::Json
    })
}

/// The subcommand named on the command line, for the `command` of a usage
/// error: the first argument in subcommand position when it is a command
/// name, else `hibi`. The clap definition says which global options take a
/// value, so that a value equal to a command name (`--root status`) is not
/// taken for the command.
fn command_argument(args: &[OsString]) -> String {
    let definition = cli::command();
    let takes_value = |name: &str| {
        definition.get_arguments().any(|a| {
            a.get_long() == Some(name)
                && matches!(a.get_action(), ArgAction::Set | ArgAction::Append)
        })
    };
    let mut args = args.iter().skip(1).map(|a| a.to_string_lossy());
    while let Some(arg) = args.next() {
        match arg.strip_prefix("--") {
            // `--` ends the options: what follows is not a command.
            Some("") => break,
            Some(option) => {
                if !option.contains('=') && takes_value(option) {
                    args.next();
                }
            }
            None if arg.starts_with('-') && arg.len() > 1 => {}
            None => {
                return if definition.find_subcommand(arg.as_ref()).is_some() {
                    arg.into_owned()
                } else {
                    "hibi".to_owned()
                };
            }
        }
    }
    "hibi".to_owned()
}

/// clap's message for a usage error, without its `error:` prefix and its
/// closing pointer to `--help`.
fn usage_error(err: &clap::Error, command: &str) -> Error {
    let rendered = err.render().to_string();
    let message = rendered
        .lines()
        .filter(|l| !l.starts_with("For more information"))
        .collect::<Vec<_>>()
        .join("\n");
    let message = message.trim();
    let message = message.strip_prefix("error: ").unwrap_or(message);
    let help = if command == "hibi" {
        "hibi --help".to_owned()
    } else {
        format!("hibi {command} --help")
    };
    Error::usage(message).with_remediation(help)
}

fn print(out: &mut dyn Write, text: &str) {
    // A closed pipe is not an error of the command.
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

fn succeed(format: Format, reply: Reply) -> ExitCode {
    let stdout = &mut std::io::stdout().lock();
    match (reply.body, format) {
        (Body::Raw(text), _) => print(stdout, &text),
        (Body::Result { json, .. }, Format::Json) => print(stdout, &format!("{json}\n")),
        (Body::Result { human, .. }, Format::Human) => {
            let stderr = &mut std::io::stderr().lock();
            for w in &reply.warnings {
                print(stderr, &human::warning(w));
            }
            print(stdout, &human);
        }
    }
    ExitCode::SUCCESS
}

fn fail(format: Format, command: &str, errors: Vec<Error>) -> ExitCode {
    match format {
        Format::Json => {
            let envelope = ErrorEnvelope::new(command, errors);
            let json = serde_json::to_string(&envelope).unwrap_or_else(|_| {
                format!(
                    r#"{{"ok":false,"command":"{command}","errors":[{{"code":"internal","message":"the error does not serialize","remediation":null}}]}}"#
                )
            });
            print(&mut std::io::stdout().lock(), &format!("{json}\n"));
        }
        Format::Human => {
            let stderr = &mut std::io::stderr().lock();
            for e in &errors {
                print(stderr, &human::error(e));
            }
        }
    }
    ExitCode::FAILURE
}

/// Run `hibi` with these arguments (the first is the program name) and
/// return the exit status: 0 when the command ran, 1 on any error (spec
/// section 15.1).
pub fn run(args: &[OsString]) -> ExitCode {
    // For the test of the panic report only: there is no other way to make
    // a correct binary panic. Not in the help or the reference.
    if std::env::var_os(PANIC_TRIGGER).is_some_and(|v| v == "1") {
        panic!("{PANIC_TRIGGER} is 1, which asks for this panic to test how one is reported");
    }
    let format = format_of(args);
    let parsed = Cli::command()
        .try_get_matches_from(args)
        .and_then(|m| Cli::from_arg_matches(&m));
    let cli = match parsed {
        Ok(cli) => cli,
        Err(err) => {
            return match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    let _ = err.print();
                    ExitCode::SUCCESS
                }
                _ => {
                    let command = command_argument(args);
                    let error = usage_error(&err, &command);
                    fail(format, &command, vec![error])
                }
            };
        }
    };
    let command = cli.command.name();
    match app::run(cli) {
        Ok(reply) => succeed(format, reply),
        Err(errors) => {
            // A usage error found after parsing points at the help of the
            // command that was run, as one found by clap does.
            let general = hibi::Code::Usage.remediation();
            let errors = errors
                .into_iter()
                .map(|e| {
                    if e.code == hibi::Code::Usage && e.remediation.as_deref() == general {
                        e.with_remediation(format!("hibi {command} --help"))
                    } else {
                        e
                    }
                })
                .collect();
            fail(format, command, errors)
        }
    }
}

/// The environment variable that makes [`run`] panic when its value is `1`.
/// Only the test of the panic report sets it.
const PANIC_TRIGGER: &str = "HIBI_TEST_PANIC";

/// The text and place of the first panic of this process, kept by the
/// panic hook of [`main`] for the error it is reported as.
static PANIC: Mutex<Option<String>> = Mutex::new(None);

/// The entry point of the binary. A panic is reported as `internal` with
/// exit status 1 in the format of the run, so that no run exits with
/// another status. The standard panic message is not printed: in JSON
/// format standard error stays empty (spec section 15.1), and in human
/// format the error is printed once, in the form of every other error.
pub fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let text = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("a panic without a message");
        let place = info
            .location()
            .map(|l| format!(" at {}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        if let Ok(mut first) = PANIC.lock() {
            first.get_or_insert(format!("{text}{place}"));
        }
    }));
    match std::panic::catch_unwind(|| run(&args)) {
        Ok(code) => code,
        Err(_) => {
            let what = PANIC
                .lock()
                .ok()
                .and_then(|mut first| first.take())
                .unwrap_or_else(|| "a panic that left no message".to_owned());
            fail(
                format_of(&args),
                &command_argument(&args),
                vec![Error::internal(format!(
                    "hibi stopped on an unexpected condition: {what}"
                ))],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_format_is_read_from_the_raw_arguments() {
        assert_eq!(format_argument(&args(&["hibi", "status"])), None);
        assert_eq!(
            format_argument(&args(&["hibi", "--format", "json", "nope"])),
            Some(Format::Json)
        );
        assert_eq!(
            format_argument(&args(&["hibi", "x", "--format=human"])),
            Some(Format::Human)
        );
    }

    #[test]
    fn the_command_of_a_usage_error_is_the_subcommand_or_hibi() {
        assert_eq!(command_argument(&args(&["hibi", "--bogus"])), "hibi");
        // A command name that is the value of an option is not the command.
        assert_eq!(
            command_argument(&args(&["hibi", "--root", "status", "--bogus"])),
            "hibi"
        );
        assert_eq!(
            command_argument(&args(&["hibi", "--root=status", "list", "--bogus"])),
            "list"
        );
        assert_eq!(
            command_argument(&args(&["hibi", "--no-cache", "mark", "claim"])),
            "mark"
        );
        assert_eq!(command_argument(&args(&["hibi", "nope", "status"])), "hibi");
        assert_eq!(command_argument(&args(&["hibi", "--", "status"])), "hibi");
        assert_eq!(
            command_argument(&args(&["hibi", "--root", "x", "claim", "--bogus"])),
            "claim"
        );
    }
}
