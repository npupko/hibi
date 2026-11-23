//! The send rule of section 14.2 (R61): text from the file provider is
//! sent only when its path is in the file list (section 10.4) of the root,
//! or of a sibling repository that passes the checks of section 11.1.
//! Ignored files are not in a file list, so their text is never sent.

use std::collections::{HashMap, HashSet};

use crate::config::allow_sibling;
use crate::error::Error;
use crate::git::Git;
use crate::model::{Locator, LocatorParts};

use super::JudgeEnv;

/// File lists, read once per run and per repository.
pub(super) struct Gate<'a> {
    env: &'a JudgeEnv<'a>,
    git: &'a Git,
    root: Option<HashSet<String>>,
    /// Per alias: the sibling's file list, or `None` when it is not allowed.
    siblings: HashMap<String, Option<HashSet<String>>>,
}

impl<'a> Gate<'a> {
    pub fn new(env: &'a JudgeEnv<'a>, git: &'a Git) -> Self {
        Gate {
            env,
            git,
            root: None,
            siblings: HashMap::new(),
        }
    }

    /// The root's file list.
    pub fn root_files(&mut self) -> Result<&HashSet<String>, Error> {
        if self.root.is_none() {
            self.root = Some(self.git.file_list(self.env.root)?.into_iter().collect());
        }
        Ok(self.root.get_or_insert_with(HashSet::new))
    }

    /// The file list of sibling `alias`, or `None` when the alias fails
    /// the checks of section 11.1.
    pub fn sibling_files(&mut self, alias: &str) -> Result<Option<&HashSet<String>>, Error> {
        if !self.siblings.contains_key(alias) {
            let env = self.env;
            let git = self.git;
            let files = match allow_sibling(alias, env.config, env.user, env.root, |d| {
                git.is_toplevel(d)
            }) {
                Ok(dir) => Some(git.file_list(&dir)?.into_iter().collect()),
                Err(_) => None,
            };
            self.siblings.insert(alias.to_owned(), files);
        }
        Ok(self.siblings.get(alias).and_then(Option::as_ref))
    }

    /// Why sibling `alias` fails the checks of section 11.1, in the words
    /// the file provider uses for a `repo:` side; `None` when it passes.
    pub fn sibling_refusal(&self, alias: &str) -> Option<String> {
        let (env, git) = (self.env, self.git);
        allow_sibling(alias, env.config, env.user, env.root, |d| {
            git.is_toplevel(d)
        })
        .err()
        .map(|refusal| refusal.message().to_owned())
    }

    /// True when text of `loc` may be sent: an external locator (its text
    /// does not come from the file provider), or a `file:` or `repo:` path
    /// in the file list of its repository.
    pub fn allows(&mut self, loc: &Locator) -> Result<bool, Error> {
        match loc.parts() {
            LocatorParts::File { path } => Ok(self.root_files()?.contains(path)),
            LocatorParts::Repo { alias, path } => {
                Ok(self.sibling_files(alias)?.is_some_and(|f| f.contains(path)))
            }
            LocatorParts::External { .. } => Ok(true),
        }
    }
}
