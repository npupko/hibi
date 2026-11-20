//! The text form of each command's result (spec section 15.3, "Human").
//! Every function takes the same output object that is printed as JSON and
//! returns the lines for standard output.

use std::fmt::Write as _;

use hibi::engine::{
    ClaimOutput, ClaimState, ClaimView, CoverOutput, ListOutput, MarkOutput, NEXT_NOTE,
    ReanchorOutput, RetireOutput, Role, Side, SideState, StatusOutput, UnitRef, Warning,
};
use hibi::model::Anchor;
use hibi::{Code, Error};

use crate::output::{InitOutput, ProvidersReply, SchemaOutput, VersionOutput};

/// Text is cut to this many characters on one line of a listing.
const LINE_TEXT_CHARS: usize = 100;

/// One line of text: whitespace runs become one space, and a long text is
/// cut with an ellipsis.
fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `line 12` or `lines 12-14`; empty without a range.
fn lines_of(unit: &UnitRef) -> String {
    match (unit.line, unit.end_line) {
        (Some(a), Some(b)) if b > a => format!("lines {a}-{b}"),
        (Some(a), _) => format!("line {a}"),
        _ => String::new(),
    }
}

/// The unit as a person names it: its label (or key), kind and lines.
fn unit_name(unit: &UnitRef) -> String {
    let label = unit.label.as_deref().unwrap_or(&unit.key);
    let mut out = format!("{} \"{}\"", unit.kind, one_line(label, 60));
    let lines = lines_of(unit);
    if !lines.is_empty() {
        let _ = write!(out, ", {lines}");
    }
    out
}

fn role_name(side: &Side) -> String {
    match side.role {
        Role::Subject => "subject".to_owned(),
        Role::Evidence => format!("evidence {}", side.index),
    }
}

fn anchor_json(anchor: &Anchor) -> String {
    serde_json::to_string(anchor).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Errors and warnings (standard error)
// ---------------------------------------------------------------------------

pub fn error(e: &Error) -> String {
    let mut out = String::from("error: ");
    if let Some(i) = e.index {
        let _ = write!(out, "element {i}: ");
    }
    let _ = writeln!(out, "{}: {}", e.code, e.message);
    if let Some(fix) = &e.remediation {
        let label = if e.code == Code::Usage { "help" } else { "fix" };
        let _ = writeln!(out, "  {label}: {fix}");
    }
    out
}

pub fn warning(w: &Warning) -> String {
    format!("warning: {}: {}\n", w.code, w.message)
}

// ---------------------------------------------------------------------------
// init, schema, version
// ---------------------------------------------------------------------------

pub fn init(o: &InitOutput) -> String {
    format!(
        "Created the hibi store in {}: {} under .hibi/.\n\
         Next: declare scopes in .hibi/hibi.toml, then run `hibi cover`.\n",
        o.root,
        plural(o.created.len(), "file", "files"),
    )
}

pub fn schema_names(o: &SchemaOutput) -> String {
    let mut out = String::new();
    for name in &o.schemas {
        let _ = writeln!(out, "{name}");
    }
    out
}

pub fn version(o: &VersionOutput) -> String {
    format!("hibi {}\n", o.version)
}

// ---------------------------------------------------------------------------
// Claim views
// ---------------------------------------------------------------------------

/// `subject file:docs/a.md (sentence "...", line 3)`.
fn side_place(side: &Side) -> String {
    let mut out = format!("{} {}", role_name(side), side.anchor.loc);
    if let Some(unit) = &side.unit {
        let _ = write!(out, " ({})", unit_name(unit));
    }
    out
}

/// The lines for one side that is not `same`, or that is approximate.
fn side_lines(out: &mut String, side: &Side) {
    let mut line = format!("  {}  {}  {}", role_name(side), side.anchor.loc, side.state);
    if let Some(reason) = side.reason {
        let _ = write!(line, " ({reason})");
    }
    if side.approx {
        line.push_str(", found approximately");
    }
    if let Some(unit) = &side.unit {
        let _ = write!(line, "  {}", unit_name(unit));
    }
    let _ = writeln!(out, "{line}");
    if let Some(candidates) = &side.candidates {
        for c in candidates {
            let _ = writeln!(out, "    candidate: {}", unit_name(c));
        }
    }
    if let Some(text) = &side.text {
        for l in text.lines() {
            let _ = writeln!(out, "    | {l}");
        }
    }
}

fn claim_block(out: &mut String, view: &ClaimView) {
    let _ = writeln!(
        out,
        "{}  {}  {}",
        view.id,
        view.state,
        one_line(&view.statement, LINE_TEXT_CHARS)
    );
    if view.state == ClaimState::Expired
        && let (Some(recheck), Some(at)) = (&view.recheck, view.expires_at)
    {
        let _ = writeln!(out, "  recheck {recheck} ran out at {at}");
    }
    for side in &view.sides {
        if side.state != SideState::Same || side.approx || side.text.is_some() {
            side_lines(out, side);
        }
    }
    if let Some(next) = &view.next {
        let _ = writeln!(out, "  next: {next}");
    }
}

/// True when a `next` writes a mark, so the statement must be re-checked
/// first (section 15.2).
fn writes_a_mark(view: &ClaimView) -> bool {
    view.next
        .as_deref()
        .is_some_and(|n| n.starts_with("hibi mark") || n.starts_with("hibi reanchor"))
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

/// `default_selection` is true when the caller gave no id, `--state`,
/// `--loc` or `--since`: only then an empty list means nothing is stale.
pub fn status(o: &StatusOutput, default_selection: bool) -> String {
    let s = &o.summary;
    let mut out = format!(
        "{}: {} fresh, {} stale, {} expired, {} retired\n",
        plural(s.claims, "claim", "claims"),
        s.fresh,
        s.stale,
        s.expired,
        s.retired
    );
    if let Some(since) = &o.since {
        let _ = writeln!(out, "since {} (base {})", since.range, since.base);
    }
    if o.claims.is_empty() {
        out.push_str(if default_selection {
            "nothing stale\n"
        } else {
            "no claim matches\n"
        });
        return out;
    }
    for view in &o.claims {
        out.push('\n');
        claim_block(&mut out, view);
    }
    if o.claims.iter().any(writes_a_mark) {
        let _ = writeln!(out, "\n{NEXT_NOTE}");
    }
    out
}

// ---------------------------------------------------------------------------
// claim, mark, reanchor, retire
// ---------------------------------------------------------------------------

pub fn claim(o: &ClaimOutput) -> String {
    let mut out = String::new();
    for c in &o.claims {
        let what = if c.created { "created" } else { "marked" };
        let sides: Vec<String> = c.claim.sides.iter().map(side_place).collect();
        let _ = writeln!(out, "{}  {what}  {}", c.id, sides.join("; "));
    }
    out
}

pub fn mark(o: &MarkOutput) -> String {
    let mut out = String::new();
    for m in &o.marks {
        let _ = writeln!(out, "{}  marked  {}", m.claim, m.mark);
        for side in m.claim_view.sides.iter().filter(|s| s.approx) {
            side_lines(&mut out, side);
        }
        if let Some(next) = &m.claim_view.next {
            let _ = writeln!(out, "  next: {next}");
        }
    }
    out
}

pub fn reanchor(o: &ReanchorOutput) -> String {
    let mut out = format!(
        "{} is retired and replaced by {}\n  mark {}, retirement {}\n",
        o.old, o.new, o.mark, o.retirement
    );
    for side in &o.claim.sides {
        let _ = writeln!(out, "  {}  {}", role_name(side), anchor_json(&side.anchor));
    }
    out
}

pub fn retire(o: &RetireOutput) -> String {
    let mut out = String::new();
    for r in &o.retired {
        match &r.retirement {
            Some(id) => {
                let _ = writeln!(out, "{}  retired  {id}", r.claim);
            }
            None => {
                let _ = writeln!(out, "{}  already retired", r.claim);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// cover
// ---------------------------------------------------------------------------

pub fn cover(o: &CoverOutput) -> String {
    let mut out = String::new();
    if let Some(since) = &o.since {
        let _ = writeln!(out, "since {} (base {})", since.range, since.base);
    }
    for scope in &o.scopes {
        let _ = writeln!(
            out,
            "{} ({}; {}): {}, {} covered, {} excluded, {} uncovered",
            scope.name,
            scope.resource,
            scope.kinds.join(", "),
            plural(scope.units, "unit", "units"),
            scope.covered,
            scope.excluded,
            scope.uncovered
        );
        if scope.skipped.is_some() {
            let _ = writeln!(
                out,
                "  not evaluated: --since works only for scopes on scheme \"file\""
            );
        }
        let mut current = None;
        for item in &scope.items {
            if current != Some(&item.loc) {
                let _ = writeln!(out, "  {}", item.loc);
                current = Some(&item.loc);
            }
            let line = item
                .unit
                .line
                .map_or_else(|| "-".to_owned(), |l| l.to_string());
            let text = item
                .text
                .as_deref()
                .or(item.unit.label.as_deref())
                .unwrap_or(&item.unit.key);
            let _ = writeln!(
                out,
                "    {line:>5}  {}  {}",
                item.unit.kind,
                one_line(text, LINE_TEXT_CHARS)
            );
        }
        if scope.truncated {
            let _ = writeln!(
                out,
                "  {} more not listed; raise --limit to see them",
                scope.uncovered - scope.items.len()
            );
        }
        for x in &scope.exclusions {
            let what = match &x.unit {
                Some(unit) => format!("{} unit {unit}", x.resource),
                None => x.resource.clone(),
            };
            let _ = writeln!(
                out,
                "  excluded: {what} ({}): {}",
                plural(x.matched, "unit", "units"),
                x.reason
            );
        }
        for u in &scope.unavailable {
            let _ = writeln!(out, "  unavailable: {} ({})", u.loc, u.reason);
        }
    }
    if o.scopes.len() > 1 {
        let s = &o.summary;
        let _ = writeln!(
            out,
            "total: {}, {} covered, {} excluded, {} uncovered",
            plural(s.units, "unit", "units"),
            s.covered,
            s.excluded,
            s.uncovered
        );
    }
    out
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

fn optional(label: &str, value: Option<&str>) -> String {
    value.map_or_else(String::new, |v| format!("  {label} {v}"))
}

pub fn list(o: &ListOutput) -> String {
    let mut out = String::new();
    match o {
        ListOutput::Claims { claims, .. } => {
            if claims.is_empty() {
                out.push_str("no claims\n");
            }
            for c in claims {
                let last = c.last_marked_at.map(|t| t.to_string());
                let _ = writeln!(
                    out,
                    "{}  {}  {}  {}{}{}",
                    c.id,
                    one_line(&c.statement, LINE_TEXT_CHARS),
                    c.subject,
                    plural(c.marks, "mark", "marks"),
                    optional("last", last.as_deref()),
                    if c.retired { "  retired" } else { "" }
                );
            }
        }
        ListOutput::Record {
            claim,
            marks,
            retirements,
            ..
        } => {
            let _ = writeln!(out, "{}", claim.id);
            let _ = writeln!(out, "  statement: {}", claim.statement);
            let _ = writeln!(out, "  subject: {}", anchor_json(&claim.subject));
            for (i, a) in claim.evidence.iter().enumerate() {
                let _ = writeln!(out, "  evidence {}: {}", i + 1, anchor_json(a));
            }
            if let Some(r) = &claim.recheck {
                let _ = writeln!(out, "  recheck: {r}");
            }
            if let Some(n) = &claim.note {
                let _ = writeln!(out, "  note: {n}");
            }
            let _ = writeln!(out, "{}:", plural(marks.len(), "mark", "marks"));
            for m in marks {
                let _ = writeln!(
                    out,
                    "  {}  {}{}{}",
                    m.id,
                    m.at,
                    optional("by", m.by.as_deref()),
                    optional("note:", m.note.as_deref())
                );
                for s in &m.sides {
                    let _ = writeln!(out, "    {}  {}", s.loc, s.fp);
                }
            }
            for r in retirements {
                let replaced = r.replaced_by.as_ref().map(|id| id.to_string());
                let _ = writeln!(
                    out,
                    "retired: {}  {}{}{}{}",
                    r.id,
                    r.at,
                    optional("by", r.by.as_deref()),
                    optional("replaced by", replaced.as_deref()),
                    optional("note:", r.note.as_deref())
                );
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// providers
// ---------------------------------------------------------------------------

pub fn providers(o: &ProvidersReply) -> String {
    let r = &o.report;
    let mut out = format!(
        "built in: schemes {}; classes {}; profiles {}\n",
        r.builtin.schemes.join(", "),
        r.builtin.classes.join(", "),
        r.builtin.profiles.join(", ")
    );
    let _ = writeln!(
        out,
        "user config: {}",
        r.user_config.as_deref().unwrap_or("no location found")
    );
    for repo in &r.repos {
        let state = match (&repo.error, &repo.path) {
            (None, Some(path)) => format!("allowed, {path}"),
            (Some(error), Some(path)) => format!("{error}, {path}"),
            (Some(error), None) => error.clone(),
            (None, None) => "allowed".to_owned(),
        };
        let _ = writeln!(
            out,
            "repo {}: {state} (hibi.toml suggests {})",
            repo.alias, repo.suggested
        );
        if let Some(lines) = &repo.remediation {
            for l in lines.lines() {
                let _ = writeln!(out, "    {l}");
            }
        }
    }
    for s in &r.schemes {
        match &s.command {
            Some(command) => {
                let _ = writeln!(
                    out,
                    "scheme {}: registered, {}",
                    s.scheme,
                    command.join(" ")
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "scheme {}: not registered. To register it, add to the user config:",
                    s.scheme
                );
                let _ = writeln!(out, "    [providers.{}]", s.scheme);
                let _ = writeln!(out, "    command = [\"<program>\", \"<argument>\"]");
            }
        }
    }
    if let Some(check) = &o.check {
        let c = &check.report;
        let _ = writeln!(
            out,
            "check {}: {} passed, {} failed, {} skipped",
            check.scheme, c.passed, c.failed, c.skipped
        );
        for case in &c.cases {
            let state = match (case.ok, case.skipped) {
                (_, true) => "skipped",
                (true, false) => "passed",
                (false, false) => "failed",
            };
            let detail = case
                .detail
                .as_deref()
                .map_or_else(String::new, |d| format!(": {d}"));
            let _ = writeln!(out, "  {state}  {}{detail}", case.name);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_flattens_and_cuts() {
        assert_eq!(one_line("a\n  b", 10), "a b");
        assert_eq!(one_line("abcdef", 4), "abc…");
    }

    #[test]
    fn errors_show_code_message_fix_and_index() {
        let e = Error::new(Code::NotInitialized, "no store").with_index(2);
        assert_eq!(
            error(&e),
            "error: element 2: not_initialized: no store\n  fix: hibi init\n"
        );
        let e = Error::new(Code::IoError, "x");
        assert_eq!(error(&e), "error: io_error: x\n");
    }
}
