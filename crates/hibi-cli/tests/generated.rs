//! The generated deliverables equal what the binary and the generators
//! produce (spec section 2.7; requirements R6 and R59), and the commands
//! that need no root: `schema`, `completions`, `version`.

mod support;

use std::path::{Path, PathBuf};

use support::*;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// R6: `hibi schema <name>` equals `schemas/<name>.schema.json` for every
/// name, in both formats, and `schemas/` holds no other file.
#[test]
fn r6_every_schema_file_equals_the_command_output() {
    let repo = Repo::bare();
    let listed = repo.json(&["schema"]);
    let names: Vec<String> = listed["schemas"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, hibi_cli::schema::NAMES);
    for required in [
        "claim",
        "mark",
        "retirement",
        "anchor",
        "selector",
        "config",
        "user-config",
        "claim-input",
        "mark-input",
        "error",
        "unit",
        "provider-request",
        "provider-response",
        "conformance-case",
        "status-output",
        "cover-output",
    ] {
        assert!(names.iter().any(|n| n == required), "{required}");
    }

    let dir = repo_root().join("schemas");
    for name in &names {
        let file = std::fs::read_to_string(dir.join(format!("{name}.schema.json")))
            .unwrap_or_else(|e| panic!("schemas/{name}.schema.json: {e}"));
        for format in ["json", "human"] {
            let out = repo.hibi().args(["--format", format, "schema", name]).run();
            assert_eq!(out.status, 0, "{name}: {out:?}");
            assert_eq!(out.stderr, "");
            assert_eq!(out.stdout, file, "schema {name} in format {format}");
        }
        let parsed: serde_json::Value = serde_json::from_str(&file).unwrap();
        assert_eq!(
            parsed["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
    }
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    let mut expected: Vec<String> = names.iter().map(|n| format!("{n}.schema.json")).collect();
    expected.sort();
    assert_eq!(files, expected);
}

#[test]
fn an_unknown_schema_name_is_invalid_argument() {
    let repo = Repo::bare();
    let e = repo.fail(&["schema", "nope"]);
    assert_eq!(e["code"], "invalid_argument");
    assert_eq!(e["remediation"], "hibi schema");
}

/// R59: the committed CLI reference equals the generator's output.
#[test]
fn r59_the_cli_reference_equals_the_generator_output() {
    let file = std::fs::read_to_string(repo_root().join(hibi_cli::reference::PATH)).unwrap();
    assert_eq!(file, hibi_cli::reference::render());
}

/// Every command and every long option of the clap definition appears in
/// the reference under its command.
#[test]
fn the_reference_lists_every_option_under_its_command() {
    let text = hibi_cli::reference::render();
    let root = hibi_cli::cli::command();
    for sub in root.get_subcommands() {
        let heading = format!("## hibi {}\n", sub.get_name());
        let start = text.find(&heading).unwrap_or_else(|| panic!("{heading}"));
        let rest = &text[start + heading.len()..];
        let section = &rest[..rest.find("\n## hibi ").unwrap_or(rest.len())];
        for arg in sub.get_arguments() {
            let Some(long) = arg.get_long() else { continue };
            if arg.is_global_set() || long == "help" {
                continue;
            }
            assert!(
                section.contains(&format!("- `--{long}")),
                "{} --{long}",
                sub.get_name()
            );
            let help = arg.get_help().map(|h| h.to_string()).unwrap_or_default();
            assert!(!help.is_empty(), "{} --{long} has no help", sub.get_name());
        }
    }
    for global in [
        "--root <dir>",
        "--format <format>",
        "--now <timestamp>",
        "--no-cache",
    ] {
        assert!(text.contains(&format!("- `{global}`")), "{global}");
    }
    assert!(text.contains("Default: `200`"), "cover --limit default");
    assert!(text.contains("Default: `50`"), "triage --limit default");
    assert!(
        text.contains("Default: `10`"),
        "suggest-evidence --limit default"
    );
}

#[test]
fn version_needs_no_root_and_reports_the_build() {
    let repo = Repo::bare();
    let outside = repo.home.clone();
    let v = repo
        .hibi()
        .cwd(&outside)
        .args(["--format", "json", "version"])
        .run()
        .ok();
    assert_eq!(v["command"], "version");
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["protocol"], serde_json::json!([1]));
    assert_eq!(v["storeVersion"], 1);
    assert_eq!(v["profiles"], serde_json::json!(["prose-v1", "ast-v1"]));
    assert_eq!(v["judgeModel"], "jev-1.14.0");
    assert_eq!(
        v["grammars"],
        serde_json::json!({"typescript": "0.23.2", "tsx": "0.23.2", "python": "0.25.0",
            "rust": "0.24.2", "go": "0.25.0", "java": "0.23.5"})
    );
    assert_eq!(v["warnings"], serde_json::json!([]));

    let human = repo
        .hibi()
        .cwd(&outside)
        .args(["--format", "human", "version"])
        .run();
    assert_eq!(human.status, 0);
    assert_eq!(
        human.stdout,
        format!("hibi {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn completions_print_a_script_for_every_shell() {
    let repo = Repo::bare();
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let out = repo
            .hibi()
            .cwd(&repo.home)
            .args(["completions", shell])
            .run();
        assert_eq!(out.status, 0, "{shell}: {out:?}");
        assert_eq!(out.stderr, "");
        assert!(out.stdout.contains("hibi"), "{shell}");
        assert!(out.stdout.contains("suggest-evidence"), "{shell}");
        assert!(serde_json::from_str::<serde_json::Value>(&out.stdout).is_err());
    }
    let e = repo
        .hibi()
        .cwd(&repo.home)
        .args(["completions", "tcsh"])
        .run();
    assert_eq!(e.code(), "usage");
}
