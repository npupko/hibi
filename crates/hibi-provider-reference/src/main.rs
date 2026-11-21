//! `hibi-provider-reference --fixture <file.json>`: serves scheme `ref` from a
//! fixture file and holds no other state (spec section 13.5).
//!
//! It is the worked example for provider authors: read one request per line
//! from standard input, write one response per line to standard output,
//! exit at end of input. The fixture looks like this:
//!
//! ```json
//! {"resources":{"ref:notes":{"units":[
//!   {"key":"n1","kind":"note","parent":null,"text":"Retries are capped at 5."},
//!   {"key":"n2","kind":"note","parent":"n1","text":"The cap is configurable."}]}}}
//! ```
//!
//! - A unit's fingerprint is `prose-v1` over its text, its selector is
//!   `{"key": <key>}`, its label is its text, and it has no range.
//! - `resolve`: with `key`, the unit with that key; with `quote`, every unit
//!   whose text contains the quote, in fixture order (`prefix`, `suffix` and
//!   `pos` are ignored); with both, the units that satisfy both; with `{}`,
//!   every unit; with `symbol`, `invalid_selector`.
//! - `render`: the unit's text, and as context the texts of its ancestors,
//!   outermost first. A parent chain that repeats a unit stops there.
//! - A locator that is not in the fixture is `not_found`.

use std::collections::{BTreeMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use hibi::fingerprint::prose_v1;
use hibi::model::{Selector, SelectorKey};
use hibi::provider::{
    BatchResult, Call, Capabilities, InnerResponse, NameVersion, PROTOCOL_VERSION, ProviderError,
    ProviderErrorCode, ProviderInfo, Rendered, Response, Unit, UnitsResult, parse_request_line,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Parser)]
#[command(
    version,
    about = "The hibi reference provider: serves scheme `ref` from a fixture file"
)]
struct Args {
    /// The fixture file: {"resources": {"<locator>": {"units": [...]}}}.
    #[arg(long)]
    fixture: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    resources: BTreeMap<String, Resource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Resource {
    units: Vec<FixtureUnit>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureUnit {
    key: String,
    kind: String,
    #[serde(default)]
    parent: Option<String>,
    text: String,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let fixture = match load(&args.fixture) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("hibi-provider-reference: {}: {e}", args.fixture.display());
            return ExitCode::from(2);
        }
    };
    match serve(&fixture) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hibi-provider-reference: {e}");
            ExitCode::FAILURE
        }
    }
}

fn load(path: &PathBuf) -> Result<Fixture, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let fixture: Fixture = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    for (loc, resource) in &fixture.resources {
        let mut keys = HashSet::new();
        if let Some(u) = resource
            .units
            .iter()
            .find(|u| u.key.is_empty() || !keys.insert(&u.key))
        {
            return Err(format!("{loc}: unit key {:?} is empty or repeated", u.key));
        }
    }
    Ok(fixture)
}

/// Answer every line of standard input until it ends.
fn serve(fixture: &Fixture) -> io::Result<()> {
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().split(b'\n') {
        let response = match String::from_utf8(line?) {
            Err(_) => Response::err(
                None,
                ProviderError::invalid_request("the line is not UTF-8"),
            ),
            Ok(line) => match parse_request_line(&line) {
                Ok((id, call)) => Response {
                    id: Some(id),
                    outcome: handle(fixture, call),
                },
                Err(e) => Response::err(e.id, e.error),
            },
        };
        writeln!(out, "{}", response.to_line())?;
        out.flush()?;
    }
    Ok(())
}

fn handle(fixture: &Fixture, call: Call) -> Result<Value, ProviderError> {
    match call {
        Call::Hello(p) => {
            if !p.protocol.contains(&PROTOCOL_VERSION) {
                return Err(ProviderError::new(
                    ProviderErrorCode::UnsupportedProtocol,
                    format!("this provider speaks protocol {PROTOCOL_VERSION} only"),
                ));
            }
            to_value(hello(fixture))
        }
        Call::Resolve(p) => {
            let units = resolve(resource(fixture, p.loc.as_str())?, &p.sel)?;
            to_value(UnitsResult { units })
        }
        Call::Units(p) => {
            let units = resource(fixture, p.loc.as_str())?
                .units
                .iter()
                .filter(|u| p.kind.as_ref().is_none_or(|k| *k == u.kind))
                .map(|u| unit(u, false))
                .collect();
            to_value(UnitsResult { units })
        }
        Call::Render(p) => to_value(render(resource(fixture, p.loc.as_str())?, &p.unit)?),
        Call::Batch(b) => {
            let responses = b
                .requests
                .iter()
                .map(|raw| InnerResponse(raw.parse_inner().and_then(|call| handle(fixture, call))))
                .collect();
            to_value(BatchResult { responses })
        }
    }
}

fn hello(fixture: &Fixture) -> ProviderInfo {
    let mut kinds: Vec<String> = Vec::new();
    for u in fixture.resources.values().flat_map(|r| &r.units) {
        if !kinds.contains(&u.kind) {
            kinds.push(u.kind.clone());
        }
    }
    ProviderInfo {
        protocol: PROTOCOL_VERSION,
        provider: NameVersion {
            name: "hibi-provider-reference".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        schemes: vec!["ref".into()],
        selectors: vec![SelectorKey::Key, SelectorKey::Quote],
        kinds,
        capabilities: Capabilities {
            units: true,
            render: true,
            batch: true,
        },
    }
}

fn resource<'f>(fixture: &'f Fixture, loc: &str) -> Result<&'f Resource, ProviderError> {
    fixture
        .resources
        .get(loc)
        .ok_or_else(|| ProviderError::not_found(format!("{loc} is not in the fixture")))
}

fn resolve(resource: &Resource, sel: &Selector) -> Result<Vec<Unit>, ProviderError> {
    if sel.symbol.is_some() {
        return Err(ProviderError::new(
            ProviderErrorCode::InvalidSelector,
            "this provider accepts the selector keys key and quote",
        ));
    }
    Ok(resource
        .units
        .iter()
        .filter(|u| sel.key.as_ref().is_none_or(|k| *k == u.key))
        .filter(|u| {
            sel.quote
                .as_ref()
                .is_none_or(|q| u.text.contains(q.as_str()))
        })
        .map(|u| unit(u, true))
        .collect())
}

fn render(resource: &Resource, key: &str) -> Result<Rendered, ProviderError> {
    let find = |key: &str| resource.units.iter().find(|u| u.key == key);
    let Some(target) = find(key) else {
        return Err(ProviderError::new(
            ProviderErrorCode::UnknownUnit,
            format!("no unit has key {key:?}"),
        ));
    };
    let mut seen = HashSet::from([target.key.as_str()]);
    let mut context = Vec::new();
    let mut parent = target.parent.as_deref();
    while let Some(u) = parent.and_then(find) {
        if !seen.insert(u.key.as_str()) {
            break;
        }
        context.push(u.text.clone());
        parent = u.parent.as_deref();
    }
    context.reverse();
    Ok(Rendered {
        text: target.text.clone(),
        context,
    })
}

/// The unit object of a fixture unit; `text` only for `resolve`.
fn unit(u: &FixtureUnit, with_text: bool) -> Unit {
    Unit {
        key: u.key.clone(),
        kind: u.kind.clone(),
        parent: u.parent.clone(),
        label: Some(u.text.clone()),
        range: None,
        fp: prose_v1(&u.text),
        sel: Selector {
            key: Some(u.key.clone()),
            ..Selector::default()
        },
        text: with_text.then(|| u.text.clone()),
        approx: false,
        identity: None,
        refuse: None,
    }
}

fn to_value(v: impl serde::Serialize) -> Result<Value, ProviderError> {
    serde_json::to_value(v)
        .map_err(|e| ProviderError::new(ProviderErrorCode::Internal, e.to_string()))
}
