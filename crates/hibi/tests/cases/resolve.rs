//! `resolve.json` (spec sections 6.7 and 16.3; requirement R53).
//!
//! Each case writes its fixture into an empty working tree and resolves one
//! anchor through `FileProvider` and the `Provider` trait. The same anchor
//! is resolved again without a cache, then twice through a cache (cold, then
//! warm), and every answer must be the same (R40, R53).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use hibi::config::{RepoConfig, UserConfig};
use hibi::git::GitStatus;
use hibi::model::{Anchor, Locator, Selector};
use hibi::provider::Provider;
use hibi::provider::file::{FileOptions, FileProvider, UnitCache};
use serde_json::{Value, json};

use crate::harness::{Case, cases_dir, str_field};

/// An in-memory unit cache.
#[derive(Default)]
struct Memory(RefCell<HashMap<(String, String), String>>);

impl UnitCache for Memory {
    fn get_units(&self, blob: &str, class: &str) -> Option<String> {
        self.0
            .borrow()
            .get(&(blob.to_owned(), class.to_owned()))
            .cloned()
    }
    fn put_units(&self, blob: &str, class: &str, units: &str) {
        self.0
            .borrow_mut()
            .insert((blob.to_owned(), class.to_owned()), units.to_owned());
    }
}

fn provider(root: &std::path::Path, cache: Option<Box<dyn UnitCache>>) -> FileProvider {
    FileProvider::working_tree(
        root,
        FileOptions {
            repo: RepoConfig::default(),
            user: UserConfig::default(),
            git: GitStatus::Missing {
                message: "not used".into(),
            },
            cache,
        },
    )
}

/// A content key `<kind>:<12 hex>` with an optional `~n` matches the
/// expected `<kind>:*` with the same suffix.
fn key_matches(got: &str, want: &str) -> bool {
    let Some((kind, rest)) = want.split_once(":*") else {
        return got == want;
    };
    let Some(after) = got.strip_prefix(kind).and_then(|g| g.strip_prefix(':')) else {
        return false;
    };
    after.len() >= 12
        && after[..12]
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && &after[12..] == rest
}

/// `input: {path, fixture | text | neither, sel}`; `expected: {units:
/// [{key, approx, refuse, sel}]}` or `{error: <code>}`. Without `fixture`
/// and `text` the resource does not exist.
pub fn resolve(case: &Case) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = str_field(&case.input, "path");
    let content: Option<Vec<u8>> = if let Some(f) = case.input["fixture"].as_str() {
        Some(std::fs::read(cases_dir().join(f)).map_err(|e| format!("{f}: {e}"))?)
    } else {
        case.input["text"].as_str().map(|t| t.as_bytes().to_vec())
    };
    if let Some(bytes) = content {
        let file = dir.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    }
    let sel: Selector =
        serde_json::from_value(case.input["sel"].clone()).map_err(|e| format!("input.sel: {e}"))?;
    let anchor = Anchor::new(
        Locator::new(format!("file:{path}")).map_err(|e| e.message)?,
        sel,
    );

    let mut plain = provider(dir.path(), None);
    let first = plain.resolve(std::slice::from_ref(&anchor)).remove(0);
    let again = plain.resolve(std::slice::from_ref(&anchor)).remove(0);
    let cache = Rc::new(Memory::default());
    let mut cached = provider(dir.path(), Some(Box::new(cache.clone())));
    let cold = cached.resolve(std::slice::from_ref(&anchor)).remove(0);
    let warm = cached.resolve(std::slice::from_ref(&anchor)).remove(0);
    if again != first || cold != first || warm != first {
        return Err("the answer differs between runs or with a cache (R40, R53)".into());
    }

    let got: Value = match &first {
        Err(e) => json!({ "error": e.code.as_str() }),
        Ok(units) => json!({ "units": units.iter().map(|u| json!({
            "key": u.key,
            "approx": u.approx,
            "refuse": u.refuse.map(|r| r.as_str()),
            "sel": serde_json::to_value(&u.sel).unwrap(),
        })).collect::<Vec<_>>() }),
    };
    let mut got = got;
    if let (Some(g), Some(w)) = (
        got.get_mut("units").and_then(Value::as_array_mut),
        case.expected.get("units").and_then(Value::as_array),
    ) {
        for (gu, wu) in g.iter_mut().zip(w) {
            if let (Some(gk), Some(wk)) = (gu["key"].as_str(), wu["key"].as_str())
                && key_matches(gk, wk)
            {
                gu["key"] = Value::from(wk);
            }
        }
    }
    if got == case.expected {
        Ok(())
    } else {
        Err(format!(
            "\n   got {}\n  want {}",
            serde_json::to_string_pretty(&got).unwrap(),
            serde_json::to_string_pretty(&case.expected).unwrap()
        ))
    }
}
