//! Fingerprints and freshness through the binary (spec sections 8 and 9;
//! requirements R12 to R15 and R18, the fourth verification item of the
//! plan).

mod support;

use serde_json::{Value, json};
use support::*;

/// One source file of a code class in four forms, with the symbol of the
/// anchored declaration.
struct Lang {
    /// Path of the source file relative to the root.
    path: &'static str,
    /// Symbol of the anchored declaration.
    symbol: &'static str,
    /// The unit kind the anchored declaration has.
    kind: &'static str,
    base: &'static str,
    /// `base` with whitespace, line breaks, comments and commas before a
    /// closing delimiter changed, and nothing else.
    reformatted: &'static str,
    /// `base` with one literal inside the anchored declaration changed.
    literal: &'static str,
    /// `base` with the literal of the other declaration changed.
    other: &'static str,
    /// The literal that `literal` changes, and what it becomes.
    change: (&'static str, &'static str),
}

const DOC: &str = "docs/retry.md";
const DOC_TEXT: &str = "The backoff adds one hundred to the delay. The attempt limit is five.\n";
const SUBJECT: &str = "file:docs/retry.md::The backoff adds one hundred to the delay.";

const TYPESCRIPT: Lang = Lang {
    path: "src/retry.ts",
    symbol: "backoff",
    kind: "function",
    base: "\
export const MAX_ATTEMPTS = 5;

export function backoff(attempt: number, base: number): number {
  const steps = [1, 2, 4];
  return base * steps[attempt % 3] + 100;
}
",
    reformatted: "\
// The attempt limit.
export const MAX_ATTEMPTS   =   5;


/* Computes the delay. */
export function backoff(
    attempt: number,
    base: number, // a comma before the closing parenthesis
): number {
    // the steps table
    const steps = [
        1,
        2, /* two */
        4,
    ];
    return base   *   steps[attempt % 3]
        + 100;
}
",
    literal: "\
export const MAX_ATTEMPTS = 5;

export function backoff(attempt: number, base: number): number {
  const steps = [1, 2, 4];
  return base * steps[attempt % 3] + 250;
}
",
    other: "\
export const MAX_ATTEMPTS = 6;

export function backoff(attempt: number, base: number): number {
  const steps = [1, 2, 4];
  return base * steps[attempt % 3] + 100;
}
",
    change: ("100", "250"),
};

// The parentheses around the JSX expression are in every form: section 8.4
// says that adding or removing them changes the fingerprint.
const TSX: Lang = Lang {
    path: "src/Badge.tsx",
    symbol: "Badge",
    kind: "function",
    base: "\
export const LIMIT = 3;

export function Badge(props: { count: number; label: string }) {
  const shown = Math.min(props.count, 99);
  return (<span className=\"badge\">{shown}</span>);
}
",
    reformatted: "\
// The largest count.
export const LIMIT   =   3;


/* A count badge. */
export function Badge(
    props: { count: number; label: string }, // a comma before the closing parenthesis
) {
    // never more than two digits
    const shown = Math.min(
        props.count,
        99,
    );
    return (
        <span className=\"badge\">{shown}</span>
    );
}
",
    literal: "\
export const LIMIT = 3;

export function Badge(props: { count: number; label: string }) {
  const shown = Math.min(props.count, 50);
  return (<span className=\"badge\">{shown}</span>);
}
",
    other: "\
export const LIMIT = 4;

export function Badge(props: { count: number; label: string }) {
  const shown = Math.min(props.count, 99);
  return (<span className=\"badge\">{shown}</span>);
}
",
    change: ("99", "50"),
};

// Python line breaks and indentation are syntax, so the reformatted form
// only moves line breaks inside brackets.
const PYTHON: Lang = Lang {
    path: "src/retry.py",
    symbol: "backoff",
    kind: "function",
    base: "\
MAX_ATTEMPTS = 5


def backoff(attempt, base):
    steps = [1, 2, 4]
    return base * steps[attempt % 3] + 100
",
    reformatted: "\
# The attempt limit.
MAX_ATTEMPTS   =   5


# Computes the delay.
def backoff(
    attempt,
    base,  # a comma before the closing parenthesis
):
    # the steps table
    steps = [
        1,
        2,  # two
        4,
    ]
    return base   *   steps[attempt % 3] + 100
",
    literal: "\
MAX_ATTEMPTS = 5


def backoff(attempt, base):
    steps = [1, 2, 4]
    return base * steps[attempt % 3] + 250
",
    other: "\
MAX_ATTEMPTS = 6


def backoff(attempt, base):
    steps = [1, 2, 4]
    return base * steps[attempt % 3] + 100
",
    change: ("100", "250"),
};

const RUST: Lang = Lang {
    path: "src/retry.rs",
    symbol: "backoff",
    kind: "function",
    base: "\
pub const MAX_ATTEMPTS: u32 = 5;

pub fn backoff(attempt: u32, base: u32) -> u32 {
    let steps = [1, 2, 4];
    base * steps[(attempt % 3) as usize] + 100
}
",
    reformatted: "\
// The attempt limit.
pub const MAX_ATTEMPTS: u32   =   5;


/* Computes the delay. */
pub fn backoff(
        attempt: u32,
        base: u32, // a comma before the closing parenthesis
) -> u32 {
        // the steps table
        let steps = [
                1,
                2, /* two */
                4,
        ];
        base   *   steps[(attempt % 3) as usize]
                + 100
}
",
    literal: "\
pub const MAX_ATTEMPTS: u32 = 5;

pub fn backoff(attempt: u32, base: u32) -> u32 {
    let steps = [1, 2, 4];
    base * steps[(attempt % 3) as usize] + 250
}
",
    other: "\
pub const MAX_ATTEMPTS: u32 = 6;

pub fn backoff(attempt: u32, base: u32) -> u32 {
    let steps = [1, 2, 4];
    base * steps[(attempt % 3) as usize] + 100
}
",
    change: ("100", "250"),
};

const GO: Lang = Lang {
    path: "src/retry.go",
    symbol: "Backoff",
    kind: "function",
    base: "\
package retry

const MaxAttempts = 5

func Backoff(attempt int, base int) int {
\tsteps := []int{1, 2, 4}
\treturn base*steps[attempt%3] + 100
}
",
    reformatted: "\
package retry

// The attempt limit.
const MaxAttempts   =   5


/* Computes the delay. */
func Backoff(
    attempt int,
    base int, // a comma before the closing parenthesis
) int {
    // the steps table
    steps := []int{
        1,
        2, /* two */
        4,
    }
    return base * steps[attempt % 3] +
        100
}
",
    literal: "\
package retry

const MaxAttempts = 5

func Backoff(attempt int, base int) int {
\tsteps := []int{1, 2, 4}
\treturn base*steps[attempt%3] + 250
}
",
    other: "\
package retry

const MaxAttempts = 6

func Backoff(attempt int, base int) int {
\tsteps := []int{1, 2, 4}
\treturn base*steps[attempt%3] + 100
}
",
    change: ("100", "250"),
};

// Java allows a comma before the closing brace of an array initializer.
const JAVA: Lang = Lang {
    path: "src/Retry.java",
    symbol: "Retry.backoff",
    kind: "method",
    base: "\
class Retry {
    static final int MAX_ATTEMPTS = 5;

    static int backoff(int attempt, int base) {
        int[] steps = {1, 2, 4};
        return base * steps[attempt % 3] + 100;
    }
}
",
    reformatted: "\
class Retry {
  // The attempt limit.
  static final int MAX_ATTEMPTS   =   5;


  /* Computes the delay. */
  static int backoff(
      int attempt,
      int base // the second parameter
  ) {
    // the steps table
    int[] steps = {
      1,
      2, /* two */
      4,
    };
    return base   *   steps[attempt % 3]
        + 100;
  }
}
",
    literal: "\
class Retry {
    static final int MAX_ATTEMPTS = 5;

    static int backoff(int attempt, int base) {
        int[] steps = {1, 2, 4};
        return base * steps[attempt % 3] + 250;
    }
}
",
    other: "\
class Retry {
    static final int MAX_ATTEMPTS = 6;

    static int backoff(int attempt, int base) {
        int[] steps = {1, 2, 4};
        return base * steps[attempt % 3] + 100;
    }
}
",
    change: ("100", "250"),
};

/// The stored marks of one claim, from `hibi list <id>`.
fn marks_of(repo: &Repo, id: &str) -> Vec<Value> {
    let v = repo.json(&["list", id]);
    assert_eq!(v["claim"]["id"], id, "{v}");
    v["marks"].as_array().expect("marks is an array").clone()
}

/// `(state, reason)` of every side of a view, in order.
fn sides(view: &Value) -> Vec<(String, Value)> {
    view["sides"]
        .as_array()
        .expect("sides is an array")
        .iter()
        .map(|s| {
            (
                s["state"].as_str().expect("a side state").to_owned(),
                s["reason"].clone(),
            )
        })
        .collect()
}

fn same() -> (String, Value) {
    ("same".to_owned(), Value::Null)
}

/// Asserts that the claim is fresh with both sides `same`, not approximate,
/// and nothing to do next.
fn assert_fresh(repo: &Repo, id: &str, what: &str) {
    let view = repo.view(id);
    assert_eq!(view["id"], id, "{what}: {view}");
    assert_eq!(view["state"], "fresh", "{what}: {view}");
    assert_eq!(sides(&view), [same(), same()], "{what}: {view}");
    for side in view["sides"].as_array().unwrap() {
        assert_eq!(side["approx"], false, "{what}: {view}");
        assert_eq!(side["fp"], side["markedFp"], "{what}: {view}");
    }
    assert_eq!(view["next"], Value::Null, "{what}: {view}");
}

/// The counts of `hibi status` for the whole store.
fn summary(repo: &Repo) -> Value {
    repo.json(&["status"])["summary"].clone()
}

/// R12, R13 and R14 for one code class.
fn check_class(lang: &Lang) {
    let repo = Repo::new();
    repo.write(DOC, DOC_TEXT);
    repo.write(lang.path, lang.base);
    let evidence = format!("file:{}#{}", lang.path, lang.symbol);
    let id = repo.claim("The backoff adds one hundred.", SUBJECT, &[&evidence]);
    assert!(id.starts_with("c1-"), "{id}");

    // The claim is anchored where the test means it to be.
    let view = repo.view(&id);
    assert_eq!(view["sides"][0]["role"], "subject");
    assert_eq!(view["sides"][0]["unit"]["kind"], "sentence");
    assert_eq!(view["sides"][1]["role"], "evidence");
    assert_eq!(
        view["sides"][1]["anchor"]["loc"],
        format!("file:{}", lang.path)
    );
    assert_eq!(view["sides"][1]["anchor"]["sel"]["symbol"], lang.symbol);
    assert_eq!(view["sides"][1]["unit"]["kind"], lang.kind);
    assert_eq!(view["sides"][1]["unit"]["label"], lang.symbol);
    let marked_fp = view["sides"][1]["fp"].as_str().unwrap().to_owned();
    assert!(marked_fp.starts_with("b3:ast-v1:"), "{marked_fp}");
    assert_fresh(&repo, &id, "base");
    let marks = marks_of(&repo, &id);
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0]["id"], view["mark"]);
    let all_fresh =
        json!({"claims": 1, "fresh": 1, "drifted": 0, "stale": 0, "expired": 0, "retired": 0});
    let one_stale =
        json!({"claims": 1, "fresh": 0, "drifted": 0, "stale": 1, "expired": 0, "retired": 0});
    assert_eq!(summary(&repo), all_fresh);

    // R12: whitespace, line breaks, comments, commas before a closing
    // delimiter.
    assert_ne!(lang.reformatted, lang.base);
    repo.write(lang.path, lang.reformatted);
    assert_fresh(&repo, &id, "reformatted");
    assert_eq!(repo.view(&id)["sides"][1]["fp"], marked_fp.as_str());
    assert_eq!(summary(&repo), all_fresh);

    // A change to the other declaration.
    repo.write(lang.path, lang.other);
    assert_fresh(&repo, &id, "other declaration changed");
    assert_eq!(summary(&repo), all_fresh);

    // R13: a literal inside the anchored declaration.
    repo.write(lang.path, lang.literal);
    let view = repo.view(&id);
    assert_eq!(view["id"], id);
    assert_eq!(view["state"], "stale", "{view}");
    assert_eq!(
        sides(&view),
        [same(), ("changed".to_owned(), json!("fingerprint_differs"))],
        "{view}"
    );
    assert_eq!(view["sides"][1]["approx"], false, "{view}");
    assert_eq!(view["sides"][1]["markedFp"], marked_fp.as_str(), "{view}");
    assert_ne!(view["sides"][1]["fp"], marked_fp.as_str(), "{view}");
    assert!(
        view["sides"][1]["fp"]
            .as_str()
            .unwrap()
            .starts_with("b3:ast-v1:"),
        "{view}"
    );
    assert_eq!(view["next"], format!("hibi mark {id}"), "{view}");
    assert_eq!(view["mark"], marks[0]["id"], "{view}");
    assert_eq!(summary(&repo), one_stale);
    // The default selection of `status` lists exactly this claim.
    let listed = repo.json(&["status"]);
    assert_eq!(listed["claims"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["claims"][0]["id"], id);
    assert_eq!(listed["claims"][0]["state"], "stale");

    // R14: the earlier content is fresh again, and no mark was written.
    repo.write(lang.path, lang.base);
    assert_fresh(&repo, &id, "restored");
    assert_eq!(marks_of(&repo, &id), marks);
    assert_eq!(repo.records("marks").len(), 1);
    assert_eq!(repo.records("claims").len(), 1);
    assert_eq!(summary(&repo), all_fresh);

    // The reformatted form of the changed literal is still stale: the
    // reformatting does not hide the change.
    let (from, to) = lang.change;
    assert_eq!(lang.base.replacen(from, to, 1), lang.literal);
    let reformatted_literal = lang.reformatted.replacen(from, to, 1);
    assert_ne!(reformatted_literal, lang.reformatted);
    repo.write(lang.path, &reformatted_literal);
    assert_eq!(repo.state(&id), "stale");
    repo.write(lang.path, lang.reformatted);
    assert_fresh(&repo, &id, "reformatted again");
    assert_eq!(marks_of(&repo, &id), marks);
}

#[test]
fn r12_r13_r14_typescript() {
    check_class(&TYPESCRIPT);
}

#[test]
fn r12_r13_r14_tsx() {
    check_class(&TSX);
}

#[test]
fn r12_r13_r14_python() {
    check_class(&PYTHON);
}

#[test]
fn r12_r13_r14_rust() {
    check_class(&RUST);
}

#[test]
fn r12_r13_r14_go() {
    check_class(&GO);
}

#[test]
fn r12_r13_r14_java() {
    check_class(&JAVA);
}

/// Section 8.4: parentheses are not formatting. Wrapping a TSX expression
/// in parentheses changes the fingerprint.
#[test]
fn tsx_parentheses_around_jsx_change_the_fingerprint() {
    let repo = Repo::new();
    repo.write(DOC, DOC_TEXT);
    let bare = TSX.base.replace(
        "return (<span className=\"badge\">{shown}</span>);",
        "return <span className=\"badge\">{shown}</span>;",
    );
    assert_ne!(bare, TSX.base);
    repo.write(TSX.path, &bare);
    let id = repo.claim(
        "The backoff adds one hundred.",
        SUBJECT,
        &["file:src/Badge.tsx#Badge"],
    );
    assert_fresh(&repo, &id, "without parentheses");
    repo.write(TSX.path, TSX.base);
    let view = repo.view(&id);
    assert_eq!(view["state"], "stale", "{view}");
    assert_eq!(
        sides(&view),
        [same(), ("changed".to_owned(), json!("fingerprint_differs"))]
    );
    assert_eq!(view["next"], format!("hibi mark {id}"));
}

/// R15: reflowing a paragraph keeps the claims on its sentences fresh; a
/// changed word makes exactly the claim on that sentence stale.
#[test]
fn r15_reflow_keeps_sentences_fresh_and_a_changed_word_stales_one() {
    let repo = Repo::new();
    let base = "\
# Retry

The client retries a failed call five times. Each retry waits twice as long as the one before. After the last retry the error is returned to the caller.
";
    let reflowed = "\
# Retry

The client retries a failed
call   five times.
Each retry waits twice as
long as the one before.   After the last retry
the error is returned
to the caller.
";
    let reworded = "\
# Retry

The client retries a failed call five times. Each retry waits thrice as long as the one before. After the last retry the error is returned to the caller.
";
    repo.write("docs/retry.md", base);
    let first = repo.claim(
        "Five retries.",
        "file:docs/retry.md::The client retries a failed call five times.",
        &[],
    );
    let second = repo.claim(
        "The wait doubles.",
        "file:docs/retry.md::Each retry waits twice as long as the one before.",
        &[],
    );
    assert_ne!(first, second);
    let ids = {
        let mut ids = vec![first.clone(), second.clone()];
        ids.sort();
        ids
    };
    let fresh_view = |id: &str, what: &str| {
        let view = repo.view(id);
        assert_eq!(view["id"], id, "{what}: {view}");
        assert_eq!(view["state"], "fresh", "{what}: {view}");
        assert_eq!(sides(&view), [same()], "{what}: {view}");
        assert_eq!(view["sides"][0]["unit"]["kind"], "sentence", "{what}");
        assert_eq!(
            view["sides"][0]["fp"], view["sides"][0]["markedFp"],
            "{what}: {view}"
        );
        view
    };
    let first_fp = fresh_view(&first, "base")["sides"][0]["fp"].clone();
    let second_fp = fresh_view(&second, "base")["sides"][0]["fp"].clone();
    assert!(first_fp.as_str().unwrap().starts_with("b3:prose-v1:"));
    assert_ne!(first_fp, second_fp);

    // Line breaks and spaces only.
    repo.write("docs/retry.md", reflowed);
    assert_eq!(fresh_view(&first, "reflowed")["sides"][0]["fp"], first_fp);
    assert_eq!(fresh_view(&second, "reflowed")["sides"][0]["fp"], second_fp);
    assert_eq!(
        summary(&repo),
        // Both stay fresh and neither drifted: a quote that differs from
        // the text only in whitespace is an exact match.
        json!({"claims": 2, "fresh": 2, "drifted": 0, "stale": 0, "expired": 0, "retired": 0})
    );

    // One word of the second sentence.
    repo.write("docs/retry.md", reworded);
    fresh_view(&first, "reworded");
    let listed = repo.json(&["status"]);
    assert_eq!(
        listed["summary"],
        json!({"claims": 2, "fresh": 1, "drifted": 0, "stale": 1, "expired": 0, "retired": 0})
    );
    assert_eq!(listed["claims"].as_array().unwrap().len(), 1, "{listed}");
    let view = &listed["claims"][0];
    assert_eq!(view["id"], second.as_str(), "{view}");
    assert_eq!(view["state"], "stale", "{view}");
    assert_eq!(view["sides"].as_array().unwrap().len(), 1, "{view}");
    assert_eq!(view["sides"][0]["state"], "changed", "{view}");
    assert_eq!(view["sides"][0]["markedFp"], second_fp, "{view}");
    assert_eq!(view["next"], format!("hibi mark {second}"), "{view}");
    // Asked for by id, the two claims come back in id order with these
    // states.
    let both = repo.json(&["status", &first, &second]);
    let states: Vec<(String, String)> = both["claims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["id"].as_str().unwrap().to_owned(),
                c["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let expected: Vec<(String, String)> = ids
        .iter()
        .map(|id| {
            let state = if *id == second { "stale" } else { "fresh" };
            (id.clone(), state.to_owned())
        })
        .collect();
    assert_eq!(states, expected);

    // The reflowed form of the reworded paragraph is stale as well, and the
    // original text is fresh again with one mark per claim.
    repo.write("docs/retry.md", &reflowed.replace("twice", "thrice"));
    fresh_view(&first, "reworded and reflowed");
    assert_eq!(repo.state(&second), "stale");
    repo.write("docs/retry.md", base);
    fresh_view(&first, "restored");
    fresh_view(&second, "restored");
    assert_eq!(marks_of(&repo, &first).len(), 1);
    assert_eq!(marks_of(&repo, &second).len(), 1);
    assert_eq!(repo.records("marks").len(), 2);
}

/// `hibi --now <now> status <id>`: the envelope and the one view.
fn status_at(repo: &Repo, now: &str, id: &str) -> (Value, Value) {
    let v = repo.json(&["--now", now, "status", id]);
    assert_eq!(v["claims"].as_array().map(Vec::len), Some(1), "{v}");
    let view = v["claims"][0].clone();
    (v, view)
}

/// R18: fresh one second before `add(mark.at, recheck)`, expired at that
/// instant.
#[test]
fn r18_recheck_boundary_under_now() {
    let repo = Repo::new();
    repo.write(DOC, DOC_TEXT);
    repo.write(RUST.path, RUST.base);
    let made = repo.json(&[
        "--now",
        "2026-01-01T00:00:00Z",
        "claim",
        "The backoff adds one hundred.",
        "--subject",
        SUBJECT,
        "--evidence",
        "file:src/retry.rs#backoff",
        "--recheck",
        "P90D",
    ]);
    assert_eq!(made["claims"].as_array().unwrap().len(), 1, "{made}");
    let id = made["claims"][0]["id"].as_str().unwrap().to_owned();
    let mark = made["claims"][0]["mark"].as_str().unwrap().to_owned();
    assert_eq!(made["claims"][0]["created"], true);
    assert_eq!(made["claims"][0]["claim"]["state"], "fresh");
    assert_eq!(made["claims"][0]["claim"]["recheck"], "P90D");
    assert_eq!(
        made["claims"][0]["claim"]["markedAt"],
        "2026-01-01T00:00:00Z"
    );
    // 2026-01-01 plus 90 days: 30 left in January, 28 in February, 31 in
    // March, then 1 April.
    let expires = "2026-04-01T00:00:00Z";
    assert_eq!(made["claims"][0]["claim"]["expiresAt"], expires);
    let marks = marks_of(&repo, &id);
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0]["id"], mark.as_str());
    assert_eq!(marks[0]["at"], "2026-01-01T00:00:00Z");

    // One second before.
    let (envelope, view) = status_at(&repo, "2026-03-31T23:59:59Z", &id);
    assert_eq!(envelope["now"], "2026-03-31T23:59:59Z");
    assert_eq!(
        envelope["summary"],
        json!({"claims": 1, "fresh": 1, "drifted": 0, "stale": 0, "expired": 0, "retired": 0})
    );
    assert_eq!(view["id"], id.as_str());
    assert_eq!(view["state"], "fresh", "{view}");
    assert_eq!(view["mark"], mark.as_str());
    assert_eq!(view["markedAt"], "2026-01-01T00:00:00Z");
    assert_eq!(view["expiresAt"], expires);
    assert_eq!(sides(&view), [same(), same()]);
    assert_eq!(view["next"], Value::Null);

    // At the instant: the boundary is inclusive.
    let (envelope, view) = status_at(&repo, expires, &id);
    assert_eq!(envelope["now"], expires);
    assert_eq!(
        envelope["summary"],
        json!({"claims": 1, "fresh": 0, "drifted": 0, "stale": 0, "expired": 1, "retired": 0})
    );
    assert_eq!(view["id"], id.as_str());
    assert_eq!(view["state"], "expired", "{view}");
    assert_eq!(view["mark"], mark.as_str());
    assert_eq!(view["markedAt"], "2026-01-01T00:00:00Z");
    assert_eq!(view["expiresAt"], expires);
    assert_eq!(sides(&view), [same(), same()]);
    assert_eq!(view["next"], format!("hibi mark {id}"));

    // One second after, and long after.
    for now in ["2026-04-01T00:00:01Z", "2031-07-09T10:11:12Z"] {
        let (_, view) = status_at(&repo, now, &id);
        assert_eq!(view["state"], "expired", "{now}: {view}");
        assert_eq!(view["expiresAt"], expires);
        assert_eq!(view["next"], format!("hibi mark {id}"));
    }

    // The default selection lists an expired claim and not a fresh one.
    let listed = repo.json(&["--now", expires, "status"]);
    assert_eq!(listed["claims"].as_array().unwrap().len(), 1);
    assert_eq!(listed["claims"][0]["id"], id.as_str());
    let listed = repo.json(&["--now", "2026-03-31T23:59:59Z", "status"]);
    assert_eq!(listed["claims"], json!([]));

    // Nothing above wrote a mark.
    assert_eq!(marks_of(&repo, &id), marks);
    assert_eq!(repo.records("marks").len(), 1);
}

/// Section 15.1: `--now` takes any offset, and a fraction of a second is
/// cut off.
#[test]
fn r18_now_accepts_an_offset_and_a_fraction() {
    let repo = Repo::new();
    repo.write(DOC, DOC_TEXT);
    let made = repo.json(&[
        "--now",
        "2026-01-01T00:00:00Z",
        "claim",
        "The backoff adds one hundred.",
        "--subject",
        SUBJECT,
        "--recheck",
        "P90D",
    ]);
    let id = made["claims"][0]["id"].as_str().unwrap().to_owned();
    let expires = "2026-04-01T00:00:00Z";

    // 01:59:59.999 at +02:00 is 23:59:59 UTC the day before.
    let (envelope, view) = status_at(&repo, "2026-04-01T01:59:59.999+02:00", &id);
    assert_eq!(envelope["now"], "2026-03-31T23:59:59Z");
    assert_eq!(view["state"], "fresh", "{view}");
    assert_eq!(view["expiresAt"], expires);
    assert_eq!(view["next"], Value::Null);

    // 02:00:00.25 at +02:00 is the instant of expiry.
    let (envelope, view) = status_at(&repo, "2026-04-01T02:00:00.25+02:00", &id);
    assert_eq!(envelope["now"], expires);
    assert_eq!(view["state"], "expired", "{view}");
    assert_eq!(view["expiresAt"], expires);
    assert_eq!(view["next"], format!("hibi mark {id}"));

    // A negative offset.
    let (envelope, view) = status_at(&repo, "2026-03-31T18:29:59.5-05:30", &id);
    assert_eq!(envelope["now"], "2026-03-31T23:59:59Z");
    assert_eq!(view["state"], "fresh", "{view}");
    let (envelope, view) = status_at(&repo, "2026-03-31T18:30:00-05:30", &id);
    assert_eq!(envelope["now"], expires);
    assert_eq!(view["state"], "expired", "{view}");

    // A claim made with a fraction and an offset is marked at the whole
    // second in UTC.
    let made = repo.json(&[
        "--now",
        "2026-06-01T09:30:15.987+09:00",
        "claim",
        "The limit is five.",
        "--subject",
        "file:docs/retry.md::The attempt limit is five.",
        "--recheck",
        "PT1H",
    ]);
    let second = made["claims"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        made["claims"][0]["claim"]["markedAt"],
        "2026-06-01T00:30:15Z"
    );
    assert_eq!(
        made["claims"][0]["claim"]["expiresAt"],
        "2026-06-01T01:30:15Z"
    );
    let marks = marks_of(&repo, &second);
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0]["at"], "2026-06-01T00:30:15Z");
    let (_, view) = status_at(&repo, "2026-06-01T01:30:14Z", &second);
    assert_eq!(view["state"], "fresh", "{view}");
    let (_, view) = status_at(&repo, "2026-06-01T01:30:15Z", &second);
    assert_eq!(view["state"], "expired", "{view}");
    assert_eq!(view["next"], format!("hibi mark {second}"));
}

/// Section 15.1: a `--now` that does not parse is `invalid_time`, in every
/// command, and writes nothing.
#[test]
fn r18_a_bad_now_is_invalid_time() {
    let repo = Repo::new();
    repo.write(DOC, DOC_TEXT);
    let id = repo.claim("The backoff adds one hundred.", SUBJECT, &[]);
    let before = repo.store_files();
    for bad in [
        "yesterday",
        "",
        "2026-01-01",
        "2026-01-01T00:00:00",
        "2026-13-01T00:00:00Z",
        "2026-02-30T00:00:00Z",
        "1767225600",
    ] {
        let out = repo.run(&["--now", bad, "status"]);
        assert_eq!(out.code(), "invalid_time", "{bad:?}: {out:?}");
        assert_eq!(out.json()["command"], "status", "{bad:?}");
        assert_eq!(out.errs().len(), 1, "{bad:?}");
    }
    let out = repo.run(&["--now", "yesterday", "mark", &id]);
    assert_eq!(out.code(), "invalid_time", "{out:?}");
    let out = repo.run(&[
        "--now",
        "yesterday",
        "claim",
        "The limit is five.",
        "--subject",
        "file:docs/retry.md::The attempt limit is five.",
    ]);
    assert_eq!(out.code(), "invalid_time", "{out:?}");
    assert_eq!(repo.store_files(), before);
    assert_eq!(repo.state(&id), "fresh");
}
