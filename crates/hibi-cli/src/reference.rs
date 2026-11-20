//! The generated CLI reference,
//! `plugins/hibi-cli/skills/hibi/references/cli-reference.md` (spec section
//! 2.7): every command, option, default and help text, taken from the clap
//! definition. It lives inside the skill's directory because a skill ships
//! only the files under it.

use std::fmt::Write as _;

use clap::{Arg, ArgAction, Command};

/// Where the reference lives, relative to the repository root.
pub const PATH: &str = "plugins/hibi-cli/skills/hibi/references/cli-reference.md";

/// Help text as one paragraph.
fn paragraph(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn takes_value(arg: &Arg) -> bool {
    matches!(arg.get_action(), ArgAction::Set | ArgAction::Append)
}

/// `--name <value>` for an option, `<value>` for a positional.
fn signature(arg: &Arg) -> String {
    let value = arg
        .get_value_names()
        .map(|names| {
            names
                .iter()
                .map(|n| format!("<{n}>"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    let Some(long) = arg.get_long() else {
        return value;
    };
    let mut out = String::new();
    if let Some(short) = arg.get_short() {
        let _ = write!(out, "-{short}, ");
    }
    let _ = write!(out, "--{long}");
    if takes_value(arg) && !value.is_empty() {
        let _ = write!(out, " {value}");
    }
    out
}

fn argument(out: &mut String, arg: &Arg) {
    let help = arg
        .get_long_help()
        .or_else(|| arg.get_help())
        .map(|h| paragraph(&h.to_string()))
        .unwrap_or_default();
    let _ = write!(out, "- `{}`: {help}", signature(arg));
    if !help.ends_with('.') && !help.ends_with(']') {
        out.push('.');
    }
    if takes_value(arg) {
        if arg.is_required_set() {
            out.push_str(" Required.");
        }
        if matches!(arg.get_action(), ArgAction::Append) && arg.is_positional() {
            out.push_str(" May be given several times.");
        }
        let defaults: Vec<String> = arg
            .get_default_values()
            .iter()
            .map(|v| format!("`{}`", v.to_string_lossy()))
            .collect();
        if !defaults.is_empty() {
            let _ = write!(out, " Default: {}.", defaults.join(", "));
        }
        let values: Vec<String> = arg
            .get_possible_values()
            .iter()
            .filter(|v| !v.is_hide_set())
            .map(|v| match v.get_help() {
                Some(h) => format!("`{}` ({})", v.get_name(), paragraph(&h.to_string())),
                None => format!("`{}`", v.get_name()),
            })
            .collect();
        if !values.is_empty() {
            let _ = write!(out, " Values: {}.", values.join(", "));
        }
    }
    out.push('\n');
}

/// Long help text as Markdown: paragraphs, and indented examples as code.
fn block(out: &mut String, text: &str) {
    for part in text.split("\n\n") {
        if part.lines().skip(1).any(|l| l.starts_with("  ")) {
            let mut lines = part.lines();
            let _ = writeln!(out, "{}\n", lines.next().unwrap_or_default());
            out.push_str("```\n");
            for l in lines {
                let _ = writeln!(out, "{}", l.strip_prefix("  ").unwrap_or(l));
            }
            out.push_str("```\n\n");
        } else {
            let _ = writeln!(out, "{}\n", paragraph(part));
        }
    }
}

fn usage(command: &Command) -> String {
    let text = command.clone().render_usage().to_string();
    text.strip_prefix("Usage: ").unwrap_or(&text).to_owned()
}

fn about(command: &Command) -> String {
    command
        .get_about()
        .map(|a| a.to_string())
        .unwrap_or_default()
}

/// The whole reference as Markdown.
pub fn render() -> String {
    let mut root = crate::cli::command();
    root.build();
    let mut out = String::from(
        "# hibi command reference\n\n\
         This file is generated from the command definition by\n\
         `cargo run -p hibi-cli --example cli-reference`. Do not edit it by hand.\n\n",
    );
    block(
        &mut out,
        &root
            .get_long_about()
            .map_or_else(|| about(&root), |a| a.to_string()),
    );
    let _ = writeln!(out, "```\n{}\n```\n", usage(&root));

    out.push_str("## Global options\n\nEvery command accepts these.\n\n");
    for arg in root.get_arguments().filter(|a| a.is_global_set()) {
        argument(&mut out, arg);
    }
    out.push_str("- `-h, --help`: Print help and exit with status 0.\n\n");

    if let Some(after) = root.get_after_long_help() {
        for section in after.to_string().split("\n\n") {
            let mut lines = section.lines();
            let title = lines.next().unwrap_or_default().trim_end_matches(':');
            let _ = writeln!(out, "## {title}\n\n```");
            for l in lines {
                let _ = writeln!(out, "{}", l.strip_prefix("  ").unwrap_or(l));
            }
            out.push_str("```\n\n");
        }
    }

    out.push_str("## Commands\n\n");
    for sub in root.get_subcommands() {
        let name = sub.get_name();
        let _ = writeln!(out, "- [`hibi {name}`](#hibi-{name}): {}", about(sub));
    }
    out.push('\n');

    for sub in root.get_subcommands() {
        let _ = writeln!(out, "## hibi {}\n", sub.get_name());
        let _ = writeln!(out, "{}.\n", about(sub));
        let _ = writeln!(out, "```\n{}\n```\n", usage(sub));
        if let Some(long) = sub.get_long_about() {
            block(&mut out, &long.to_string());
        }
        let (positionals, options): (Vec<&Arg>, Vec<&Arg>) = sub
            .get_arguments()
            .filter(|a| !a.is_global_set() && a.get_id() != "help")
            .partition(|a| a.is_positional());
        if !positionals.is_empty() {
            out.push_str("Arguments:\n\n");
            for arg in positionals {
                argument(&mut out, arg);
            }
            out.push('\n');
        }
        if !options.is_empty() {
            out.push_str("Options:\n\n");
            for arg in options {
                argument(&mut out, arg);
            }
            out.push('\n');
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_names_every_command_and_option() {
        let text = render();
        let root = crate::cli::command();
        for sub in root.get_subcommands() {
            assert!(
                text.contains(&format!("## hibi {}\n", sub.get_name())),
                "{}",
                sub.get_name()
            );
            for arg in sub.get_arguments() {
                if let Some(long) = arg.get_long() {
                    assert!(text.contains(&format!("`--{long}")), "--{long}");
                }
            }
        }
    }

    #[test]
    fn leftover_terms_do_not_appear() {
        let text = render().to_lowercase();
        for term in ["attest", "--fail-on", "bunx", "supersede", "archive", "v4"] {
            assert!(!text.contains(term), "{term}");
        }
    }
}
