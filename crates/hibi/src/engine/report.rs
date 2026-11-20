//! The configuration report of `hibi providers` (spec section 15.3). It
//! starts no provider command.

use std::path::Path;

use schemars::JsonSchema;
use serde::Serialize;

use crate::config::{RepoConfig, UserConfig, allow_sibling};
use crate::git::GitStatus;

use super::types::{Nullable, Warning};

/// `builtin` in the output of `providers`: what the file provider serves.
/// The caller fills it from the file provider's constants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Builtin {
    pub schemes: Vec<String>,
    pub classes: Vec<String>,
    pub profiles: Vec<String>,
}

/// One `[repos]` alias with the result of the checks of section 11.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RepoReport {
    pub alias: String,
    /// The value in `hibi.toml`.
    pub suggested: String,
    /// The directory from the user config, as written there.
    #[schemars(with = "Nullable<String>")]
    pub path: Option<String>,
    /// `null`, `repo_not_found` or `repo_not_allowed`.
    #[schemars(with = "Nullable<String>")]
    pub error: Option<String>,
    /// For an alias the user config does not allow: the lines to add to
    /// the user config. `null` otherwise.
    #[schemars(with = "Nullable<String>")]
    pub remediation: Option<String>,
}

/// One scheme of `hibi.toml` with its registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SchemeReport {
    pub scheme: String,
    pub registered: bool,
    #[schemars(with = "Nullable<Vec<String>>")]
    pub command: Option<Vec<String>>,
    /// For a scheme without a registration: the lines to add to the user
    /// config, with the command to fill in. `null` otherwise.
    #[schemars(with = "Nullable<String>")]
    pub remediation: Option<String>,
}

/// The output of `providers` without `--check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersOutput {
    pub builtin: Builtin,
    pub repos: Vec<RepoReport>,
    pub schemes: Vec<SchemeReport>,
    /// The user-config location.
    #[schemars(with = "Nullable<String>")]
    pub user_config: Option<String>,
    pub warnings: Vec<Warning>,
}

/// Every `[repos]` alias, ordered by alias, with the checks of section 11.1.
/// Without a usable git every alias is `repo_not_found` (section 10.4).
pub fn repo_reports(
    config: &RepoConfig,
    user: &UserConfig,
    root: &Path,
    git: &GitStatus,
) -> Vec<RepoReport> {
    let canonical_root = std::fs::canonicalize(root).ok();
    config
        .repos
        .iter()
        .map(|(alias, suggested)| {
            let path = user
                .repos
                .iter()
                .find(|e| {
                    &e.alias == alias
                        && canonical_root.is_some()
                        && std::fs::canonicalize(&e.root).ok() == canonical_root
                })
                .map(|e| e.path.to_string_lossy().into_owned());
            let (error, remediation) = match git.git() {
                None => (Some("repo_not_found".to_owned()), None),
                Some(g) => match allow_sibling(alias, config, user, root, |d| g.is_toplevel(d)) {
                    Ok(_) => (None, None),
                    Err(refusal) => {
                        let remediation = match &refusal {
                            crate::config::SiblingRefusal::RepoNotAllowed {
                                remediation, ..
                            } => remediation.clone(),
                            crate::config::SiblingRefusal::RepoNotFound { .. } => None,
                        };
                        (Some(refusal.reason().to_owned()), remediation)
                    }
                },
            };
            RepoReport {
                alias: alias.clone(),
                suggested: suggested.clone(),
                path,
                error,
                remediation,
            }
        })
        .collect()
}

/// Every scheme declared in `hibi.toml`, ordered by name, with the command
/// the user config registers for it.
pub fn scheme_reports(config: &RepoConfig, user: &UserConfig) -> Vec<SchemeReport> {
    config
        .schemes
        .keys()
        .map(|scheme| {
            let command = user.providers.get(scheme).map(|p| p.command.clone());
            let remediation = command.is_none().then(|| {
                format!("[providers.{scheme}]\ncommand = [\"<program>\", \"<argument>\"]")
            });
            SchemeReport {
                scheme: scheme.clone(),
                registered: command.is_some(),
                command,
                remediation,
            }
        })
        .collect()
}

/// `hibi providers` without `--check`.
pub fn providers_report(
    builtin: Builtin,
    config: &RepoConfig,
    user: &UserConfig,
    root: &Path,
    git: &GitStatus,
    user_config: Option<&Path>,
) -> ProvidersOutput {
    ProvidersOutput {
        builtin,
        repos: repo_reports(config, user, root, git),
        schemes: scheme_reports(config, user),
        user_config: user_config.map(|p| p.to_string_lossy().into_owned()),
        warnings: Vec::new(),
    }
}
