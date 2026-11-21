//! The text form of each command's result (spec section 15.3, "Human").
//! Every function takes the same output object that is printed as JSON and
//! returns the lines for standard output.

use std::fmt::Write as _;

use hibi::engine::{
    Accepted, ClaimOutput, ClaimState, ClaimView, CoverOutput, ListOutput, MarkOutput,
    ReanchorOutput, RetireOutput, Role, Side, SideState, StatusOutput, UnitRef, Warning,
};
use hibi::judge::{SuggestOutput, TriageOutput};
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

/// `text` without control characters, except newline and tab. Text from a
/// provider, a file or the store is printed for a person and for an agent
/// that reads a terminal: an escape sequence in it must not move the
/// cursor, change colours or hide what follows. JSON output does not need
/// this, because a JSON string writes a control character as an escape.
pub fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

/// For each of `texts`, the line to show under a candidate. Usually the
/// text on one line, cut to [`LINE_TEXT_CHARS`]. Where that would be the
/// same for two candidates whose texts differ, the first line on which the
/// text differs from the other one is shown instead, with its number in
/// the unit, so that near-identical candidates can be told apart.
fn candidate_lines(texts: &[Option<&str>]) -> Vec<Option<String>> {
    let short: Vec<Option<String>> = texts
        .iter()
        .map(|t| t.map(|t| one_line(t, LINE_TEXT_CHARS)))
        .collect();
    texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let text = (*text)?;
            let twin = texts.iter().enumerate().find_map(|(j, other)| {
                let other = (*other)?;
                (j != i && short[j] == short[i] && other != text).then_some(other)
            });
            let Some(twin) = twin else {
                return short[i].clone();
            };
            let mut theirs = twin.lines();
            for (n, line) in text.lines().enumerate() {
                if theirs.next() != Some(line) {
                    return Some(format!(
                        "line {} of the unit: {}",
                        n + 1,
                        one_line(line, LINE_TEXT_CHARS)
                    ));
                }
            }
            // This text is the beginning of the other one.
            Some(format!(
                "ends after line {} of the unit",
                text.lines().count()
            ))
        })
        .collect()
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

/// The lines for one side that is not `same`.
fn side_lines(out: &mut String, side: &Side, detail: bool) {
    let mut line = format!("  {}  {}  {}", role_name(side), side.anchor.loc, side.state);
    if let Some(reason) = side.reason {
        let _ = write!(line, " ({reason})");
    }
    if let Some(unit) = &side.unit {
        let _ = write!(line, "  {}", unit_name(unit));
    }
    let _ = writeln!(out, "{line}");
    // A side that cannot be evaluated: what the provider or the registry
    // said, then the cause and what the author can do. (The remediation of
    // a resource that cannot be read now already holds what was said.)
    let quoted = side.reason == Some(hibi::engine::Reason::ResourceUnavailable);
    if let Some(said) = &side.detail
        && !quoted
    {
        let _ = writeln!(out, "    {}", one_line(said, 300));
    }
    if let Some(fix) = &side.remediation
        && fix != "hibi providers"
    {
        let _ = writeln!(out, "    {fix}");
    }
    // A side that no longer resolves to one unit: what the claim stored,
    // so that the reader knows what to look for.
    if matches!(side.state, SideState::Missing | SideState::Ambiguous) {
        let sel = &side.anchor.sel;
        let mut stored = Vec::new();
        if let Some(symbol) = &sel.symbol {
            stored.push(format!("symbol {symbol}"));
        }
        if let Some(quote) = &sel.quote {
            stored.push(format!("quote \"{}\"", one_line(quote, 80)));
        }
        if let Some(key) = &sel.key {
            stored.push(format!("key {key}"));
        }
        if !stored.is_empty() {
            let _ = writeln!(out, "    stored: {}", stored.join(", "));
        }
    }
    if let Some(candidates) = &side.candidates {
        // Under a missing side: units with the marked content, and units
        // the provider found by similarity. Under an ambiguous side: the
        // units the stored anchor matches.
        let guessed = side.state == SideState::Missing;
        let total = side.candidates_total.unwrap_or(candidates.len());
        let _ = writeln!(
            out,
            "    {}{}:",
            plural(total, "candidate", "candidates"),
            if guessed {
                "; each is a guess, read it before you anchor to it"
            } else {
                "; the stored anchor matches each of them"
            }
        );
        let texts: Vec<Option<&str>> = candidates.iter().map(|c| c.text.as_deref()).collect();
        let shown = candidate_lines(&texts);
        for (c, shown) in candidates.iter().zip(shown) {
            let label = c.label.as_deref().unwrap_or(&c.key);
            let line = c.line.map_or_else(String::new, |l| format!(", line {l}"));
            let same = if c.same_fingerprint {
                "; same content as at the reference mark"
            } else {
                ""
            };
            // The label as it is, in double quotes: only `"` and control
            // characters are escaped (`hibi::engine::quoted`).
            let _ = writeln!(
                out,
                "    - {} {}{line}{same}",
                c.kind,
                hibi::engine::quoted(&one_line(label, 60))
            );
            // With --detail the whole text the output carries, line by
            // line; otherwise one line.
            match (&c.text, shown) {
                (Some(text), _) if detail => {
                    for l in text.lines() {
                        let _ = writeln!(out, "{}", format!("      | {l}").trim_end());
                    }
                }
                (_, Some(line)) => {
                    let _ = writeln!(out, "      | {line}");
                }
                _ => {}
            }
            let _ = writeln!(out, "      anchor: {}", c.anchor);
        }
        if total > candidates.len() {
            let _ = writeln!(out, "    and {} more", total - candidates.len());
        }
    }
    if let Some(text) = &side.text {
        for l in text.lines() {
            let _ = writeln!(out, "{}", format!("    | {l}").trim_end());
        }
    }
}

fn claim_block(out: &mut String, view: &ClaimView, detail: bool) {
    let _ = writeln!(
        out,
        "{}  {}  {}",
        view.id,
        view.state,
        one_line(&view.statement, LINE_TEXT_CHARS)
    );
    marked_line(out, view);
    for side in &view.sides {
        if side.state != SideState::Same || side.text.is_some() {
            side_lines(out, side, detail);
        }
    }
    if let Some(next) = &view.next {
        let _ = writeln!(out, "  next: {next}");
    }
}

/// `  marked <at> (<mark id>)`, with the recheck period and when it runs
/// or ran out; `  never marked` for a claim without a mark.
fn marked_line(out: &mut String, view: &ClaimView) {
    if view.state == ClaimState::Retired {
        return;
    }
    let (Some(mark), Some(at)) = (&view.mark, view.marked_at) else {
        let _ = writeln!(out, "  never marked");
        return;
    };
    let mut line = format!("  marked {at} ({mark})");
    if let (Some(recheck), Some(end)) = (&view.recheck, view.expires_at) {
        let verb = if view.state == ClaimState::Expired {
            "ran out"
        } else {
            "runs out"
        };
        let _ = write!(line, ", recheck {recheck} {verb} at {end}");
    }
    let _ = writeln!(out, "{line}");
}

/// `  was stale: evidence 1 changed (fingerprint_differs); now marked`: what
/// a write took over for a claim that was not fresh.
fn accepted_line(out: &mut String, accepted: &Accepted) {
    if accepted.was == ClaimState::Fresh {
        return;
    }
    let sides: Vec<String> = accepted
        .sides
        .iter()
        .map(|s| {
            let role = match s.role {
                Role::Subject => "subject".to_owned(),
                Role::Evidence => format!("evidence {}", s.index),
            };
            match s.reason {
                Some(reason) => format!("{role} {} {} ({reason})", s.loc, s.state),
                None => format!("{role} {} {}", s.loc, s.state),
            }
        })
        .collect();
    let what = if sides.is_empty() {
        String::new()
    } else {
        format!(": {}", sides.join("; "))
    };
    // The write marked sides that were not as the last check left them:
    // say so, and that the check is the caller's.
    let checked = match sides.len() {
        0 => "",
        1 => ": make sure you re-checked it",
        _ => ": make sure you re-checked them",
    };
    let _ = writeln!(out, "  was {}{what}; now marked{checked}", accepted.was);
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
/// `detail` is `--detail`: a candidate is then printed with all of its
/// text.
pub fn status(o: &StatusOutput, default_selection: bool, detail: bool) -> String {
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
        claim_block(&mut out, view, detail);
    }
    if o.claims.iter().any(writes_a_mark) {
        let _ = writeln!(
            out,
            "\nRe-check each statement before running its next command. If it no longer holds, fix the text or the code, or retire the claim with hibi retire."
        );
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
        let _ = writeln!(out, "{}  {what}  {}  {}", c.id, c.mark, sides.join("; "));
        if let Some(accepted) = &c.accepted {
            accepted_line(&mut out, accepted);
        }
        if let Some(next) = &c.claim.next {
            let _ = writeln!(out, "  next: {next}");
        }
    }
    out
}

pub fn mark(o: &MarkOutput) -> String {
    let mut out = String::new();
    for m in &o.marks {
        let _ = writeln!(out, "{}  marked  {}", m.claim, m.mark);
        accepted_line(&mut out, &m.accepted);
        if let Some(next) = &m.claim_view.next {
            let _ = writeln!(out, "  next: {next}");
        }
    }
    out
}

pub fn reanchor(o: &ReanchorOutput) -> String {
    let mut out = if o.merged {
        format!(
            "{} is retired and merged into the existing claim {}\n  mark {}, retirement {}\n",
            o.old, o.new, o.mark, o.retirement
        )
    } else {
        format!(
            "{} is retired and replaced by the new claim {}\n  mark {}, retirement {}\n",
            o.old, o.new, o.mark, o.retirement
        )
    };
    accepted_line(&mut out, &o.accepted);
    for side in &o.claim.sides {
        let _ = writeln!(out, "  {}", side_place(side));
        let _ = writeln!(out, "    {}", anchor_json(&side.anchor));
    }
    if let Some(next) = &o.claim.next {
        let _ = writeln!(out, "  next: {next}");
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
                for l in s.remediation.as_deref().unwrap_or_default().lines() {
                    let _ = writeln!(out, "    {l}");
                }
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

// ---------------------------------------------------------------------------
// triage, suggest-evidence
// ---------------------------------------------------------------------------

/// The JSON name of a label, such as a band.
fn label<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

pub fn triage(o: &TriageOutput) -> String {
    let mut out = format!(
        "{} ranked by {}, {} listed, {} not sent\n",
        plural(o.considered, "unit", "units"),
        o.model,
        o.items.len(),
        o.skipped
    );
    for item in &o.items {
        let line = item.unit.line.map_or_else(String::new, |l| format!(":{l}"));
        let _ = writeln!(
            out,
            "{:.2}  {}  evidence {}  {}{line}  {}",
            item.checkable,
            label(&item.band),
            label(&item.evidence),
            item.loc,
            one_line(&item.text, LINE_TEXT_CHARS)
        );
    }
    out.push_str("This is a ranking to read, not a finding.\n");
    out
}

pub fn suggest_evidence(o: &SuggestOutput) -> String {
    let mut out = format!(
        "{} ranked by {}, {} listed\n",
        plural(o.considered, "candidate", "candidates"),
        o.model,
        o.candidates.len()
    );
    for c in &o.candidates {
        let _ = writeln!(
            out,
            "{:.2}  {}  {}  {}",
            c.p,
            if c.fits { "fits" } else { "-" },
            c.loc,
            unit_name(&c.unit)
        );
    }
    if o.none_fits {
        out.push_str("None of the candidates fits the statement.\n");
    }
    out.push_str("These are units to read, not findings.\n");
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
    fn plain_drops_control_characters_but_newline_and_tab() {
        assert_eq!(
            plain("a\u{1b}[31mred\u{1b}[0m\tb\r\nc\u{7}\u{0}\u{9b}d é"),
            "a[31mred[0m\tb\nc".to_owned() + "d é"
        );
    }

    #[test]
    fn near_identical_candidates_show_the_line_where_they_differ() {
        let head = "x".repeat(150);
        let a = format!("{head}\nsecond line\nreturn 1");
        let b = format!("{head}\nsecond line\nreturn 2");
        let c = format!("{head}\nsecond line");
        let lines = candidate_lines(&[Some(&a), Some(&b), Some("short"), None]);
        assert_eq!(lines[0].as_deref(), Some("line 3 of the unit: return 1"));
        assert_eq!(lines[1].as_deref(), Some("line 3 of the unit: return 2"));
        assert_eq!(lines[2].as_deref(), Some("short"));
        assert_eq!(lines[3], None);
        let lines = candidate_lines(&[Some(&c), Some(&a)]);
        assert_eq!(lines[0].as_deref(), Some("ends after line 2 of the unit"));
        // The same text twice is printed as it is.
        let lines = candidate_lines(&[Some("same"), Some("same")]);
        assert_eq!(lines[0].as_deref(), Some("same"));
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

    /// The lines under a side that no run of the built-in providers
    /// reaches together: what a provider said and what to do, a candidate
    /// label with a quote and a backslash in it, and the count of the
    /// candidates that are not shown.
    #[test]
    fn side_lines_show_detail_remediation_candidates_and_the_rest() {
        let side = |index: usize, rest: serde_json::Value| {
            let mut side = serde_json::json!({
                "index": index,
                "role": if index == 0 { "subject" } else { "evidence" },
                "anchor": {"loc": "ext:notes", "sel": {"key": "n1"}},
                "unit": null, "fp": null, "markedFp": null,
                "remediation": null, "detail": null
            });
            side.as_object_mut()
                .unwrap()
                .extend(rest.as_object().unwrap().clone());
            side
        };
        let view: ClaimView = serde_json::from_value(serde_json::json!({
            "id": "c1-0000000000000000zzzz",
            "statement": "Retries are capped.",
            "recheck": null, "note": null, "state": "stale",
            "mark": null, "markedAt": null, "expiresAt": null,
            "next": "hibi reanchor c1-0000000000000000zzzz --subject <anchor>",
            "sides": [
                side(0, serde_json::json!({
                    "state": "ambiguous", "reason": "several_units",
                    "candidatesTotal": 23,
                    "candidates": [
                        {"key": "n1", "kind": "note", "label": "say \"hi\" in C:\\dir",
                         "line": 4, "endLine": 4, "text": "say \"hi\"\nin C:\\dir",
                         "anchor": "ext:notes#n1", "sameFingerprint": true},
                        {"key": "n2", "kind": "note", "label": null,
                         "line": null, "endLine": null, "text": null,
                         "anchor": "{\"loc\":\"ext:notes\",\"sel\":{\"key\":\"n2\"}}",
                         "sameFingerprint": false}
                    ]
                })),
                side(1, serde_json::json!({
                    "state": "unavailable", "reason": "provider_failed",
                    "detail": "the provider command exited with status 3:\nno such table",
                    "remediation": "hibi providers"
                })),
                side(2, serde_json::json!({
                    "state": "unavailable", "reason": "scheme_not_declared",
                    "detail": "the scheme ext is not declared",
                    "remediation": "Declare the scheme under [schemes] in .hibi/hibi.toml, or retire the claim."
                }))
            ]
        }))
        .unwrap();
        let mut out = String::new();
        claim_block(&mut out, &view, false);
        assert_eq!(
            out,
            r#"c1-0000000000000000zzzz  stale  Retries are capped.
  never marked
  subject  ext:notes  ambiguous (several_units)
    stored: key n1
    23 candidates; the stored anchor matches each of them:
    - note "say \"hi\" in C:\dir", line 4; same content as at the reference mark
      | say "hi" in C:\dir
      anchor: ext:notes#n1
    - note "n2"
      anchor: {"loc":"ext:notes","sel":{"key":"n2"}}
    and 21 more
  evidence 1  ext:notes  unavailable (provider_failed)
    the provider command exited with status 3: no such table
  evidence 2  ext:notes  unavailable (scheme_not_declared)
    the scheme ext is not declared
    Declare the scheme under [schemes] in .hibi/hibi.toml, or retire the claim.
  next: hibi reanchor c1-0000000000000000zzzz --subject <anchor>
"#
        );
    }
}
