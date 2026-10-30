//! Golden vectors extracted from Golu Expert AI's tests (and the PRD worked
//! examples), run against radha-core.
//!
//! Every fixture in `tests/fixtures/*.json` records the source test file and
//! its sha256 at extraction time. Every case is tagged:
//!
//! - `must_match`: Radha produces what Golu's test asserts (for the asserted
//!   fields).
//! - `intentional_divergence`: Radha deliberately differs; the case carries a
//!   written `reason` and is listed in `docs/golu-gap-log.md`.
//!
//! In both cases only `radha_expected` is asserted ("pass" = Radha meets its
//! own expected value). `golu_expected` is documentation. Unknown keys in
//! `radha_expected` fail the run, so a typo can never silently skip a check.
//! All failures are collected and reported together.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use radha_core::akshar;
use radha_core::{
    classify_outcome, consistency, net_wpm, unit_states, word_mistakes, Akshar, CharOutcome,
    DisplayMode, Outcome, Score, SessionConfig, TextUnit, TimeLimit, TimerMode, TypingSession,
    UnitState, WordOutcome,
};
use serde::Deserialize;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: u32,
    source: Source,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Source {
    origin: String,
    path: String,
    sha256: String,
    extracted: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Case {
    name: String,
    kind: String,
    tag: String,
    #[serde(default)]
    reason: String,
    input: Value,
    #[serde(default)]
    golu_expected: Value,
    radha_expected: Value,
}

/// `Some(None)` for an explicit JSON `null`, `None` when the key is absent.
fn nullable<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Expected {
    state: Option<String>,
    completion: Option<String>,
    elapsed_seconds: Option<f64>,
    remaining_seconds: Option<f64>,
    wpm: Option<f64>,
    net_wpm: Option<f64>,
    accuracy: Option<f64>,
    kdph: Option<f64>,
    consistency: Option<f64>,
    word_accuracy: Option<f64>,
    error_count: Option<usize>,
    wrong: Option<usize>,
    extra: Option<usize>,
    missing: Option<usize>,
    space_errors: Option<usize>,
    backspaces: Option<u32>,
    characters_typed: Option<usize>,
    outcomes: Option<String>,
    word_outcomes: Option<String>,
    cursor: Option<usize>,
    #[serde(default, deserialize_with = "nullable")]
    current_word: Option<Option<String>>,
    current_word_always_set: Option<bool>,
    typed_text: Option<String>,
    key_states: Option<String>,
    word_states: Option<String>,
    key_errors: Option<BTreeMap<String, u32>>,
    key_attempts: Option<BTreeMap<String, u32>>,
    finished: Option<bool>,
    pause_error: Option<bool>,
    mistake_kind: Option<String>,
    severity: Option<String>,
    error_kinds: Option<Vec<String>>,
    word_mistakes: Option<Vec<String>>,
    unit_half: Option<usize>,
    unit_full: Option<usize>,
    clusters: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreInput {
    #[serde(default)]
    correct: usize,
    #[serde(default)]
    errors: usize,
    elapsed_ms: u64,
    /// When set, only `net_wpm(wpm, errors, minutes)` is evaluated.
    wpm: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TimerInput {
    mode: String,
    #[serde(default)]
    seconds: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    at_ms: u64,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
    #[serde(default = "one")]
    count: u32,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionInput {
    unit: String,
    target: String,
    timer: Option<TimerInput>,
    #[serde(default)]
    events: Vec<Event>,
    snapshot_at_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassifyInput {
    expected: Option<String>,
    typed: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SegmentInput {
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConsistencyInput {
    intervals_ms: Vec<f64>,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn load_all() -> Vec<(String, Fixture)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixture_dir())
        .unwrap_or_else(|e| panic!("cannot read fixtures dir: {e}"))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = fs::read_to_string(&p).unwrap_or_else(|e| panic!("{name}: {e}"));
            let fixture: Fixture =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            (name, fixture)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Comparison helpers
// ---------------------------------------------------------------------------

struct Check<'a> {
    id: &'a str,
    failures: &'a mut Vec<String>,
}

impl Check<'_> {
    fn eq<T: PartialEq + std::fmt::Debug>(&mut self, field: &str, want: Option<T>, got: T) {
        if let Some(want) = want {
            if want != got {
                self.failures.push(format!(
                    "{}: {field}: expected {want:?}, got {got:?}",
                    self.id
                ));
            }
        }
    }

    fn float(&mut self, field: &str, want: Option<f64>, got: f64) {
        if let Some(want) = want {
            if (want - got).abs() > 1e-6 {
                self.failures
                    .push(format!("{}: {field}: expected {want}, got {got}", self.id));
            }
        }
    }

    fn fail(&mut self, detail: String) {
        self.failures.push(format!("{}: {detail}", self.id));
    }
}

fn outcome_codes<U>(outcomes: &[Outcome<U>]) -> String {
    outcomes
        .iter()
        .map(|o| match o {
            Outcome::Correct(_) => 'c',
            Outcome::Wrong { .. } => 'w',
            Outcome::Missing(_) => 'm',
            Outcome::Extra(_) => 'x',
        })
        .collect()
}

fn word_codes(words: &[WordOutcome]) -> String {
    words
        .iter()
        .map(|w| match w {
            WordOutcome::Correct => 'C',
            WordOutcome::Incorrect => 'I',
        })
        .collect()
}

fn state_codes(states: &[UnitState]) -> String {
    states
        .iter()
        .map(|s| match s {
            UnitState::Pending => 'P',
            UnitState::Correct => 'C',
            UnitState::Wrong => 'W',
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Runners
// ---------------------------------------------------------------------------

fn run_score(input: &Value, want: &Expected, check: &mut Check) {
    let input: ScoreInput = match serde_json::from_value(input.clone()) {
        Ok(i) => i,
        Err(e) => return check.fail(format!("bad score input: {e}")),
    };
    let secs = input.elapsed_ms as f64 / 1000.0;
    if let Some(gross) = input.wpm {
        check.float(
            "net_wpm",
            want.net_wpm,
            net_wpm(gross, input.errors, secs / 60.0),
        );
        return;
    }
    let mut outcomes = vec![CharOutcome::Correct('a'); input.correct];
    outcomes.extend(std::iter::repeat_n(
        CharOutcome::Wrong {
            expected: 'a',
            typed: 'b',
        },
        input.errors,
    ));
    let s = Score::compute(&outcomes, &[], secs, &[]);
    check.float("wpm", want.wpm, s.wpm);
    check.float("net_wpm", want.net_wpm, s.net_wpm);
    check.float("accuracy", want.accuracy, s.accuracy);
    check.float("kdph", want.kdph, s.kdph);
    check.eq("error_count", want.error_count, s.error_count);
    check.eq(
        "characters_typed",
        want.characters_typed,
        s.characters_typed,
    );
}

fn timer_mode(t: Option<&TimerInput>, check: &mut Check) -> Option<TimerMode> {
    let Some(t) = t else {
        return Some(TimerMode::Stopwatch);
    };
    match t.mode.as_str() {
        "off" => Some(TimerMode::Off),
        "stopwatch" => Some(TimerMode::Stopwatch),
        "countdown" => match TimeLimit::custom(t.seconds) {
            Ok(l) => Some(TimerMode::Countdown(l)),
            Err(e) => {
                check.fail(format!("bad timer: {e}"));
                None
            }
        },
        other => {
            check.fail(format!("unknown timer mode {other}"));
            None
        }
    }
}

fn run_session<U: TextUnit>(input: &SessionInput, want: &Expected, check: &mut Check) {
    let Some(timer) = timer_mode(input.timer.as_ref(), check) else {
        return;
    };
    let mut s = TypingSession::<U>::configured(&input.target, SessionConfig { timer });
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let mut pause_error = false;
    for ev in &input.events {
        let now = at(ev.at_ms);
        match ev.kind.as_str() {
            "text" => {
                for c in ev.text.chars() {
                    s.type_char_at(c, now);
                    if want.current_word_always_set == Some(true)
                        && s.state() != radha_core::SessionState::Complete
                        && s.evaluation().current_word.is_none()
                    {
                        check.fail(format!(
                            "no current word after typing {:?}",
                            s.matcher().typed()
                        ));
                    }
                }
            }
            "backspace" => (0..ev.count).for_each(|_| s.backspace_at(now)),
            "pause" => pause_error |= s.pause_at(now).is_err(),
            "resume" => pause_error |= s.resume_at(now).is_err(),
            "complete" => s.complete_at(now),
            "tick" => {
                s.tick_at(now);
            }
            other => return check.fail(format!("unknown event {other}")),
        }
    }

    let snap = s.snapshot_at(at(input.snapshot_at_ms));
    let eval = s.evaluation();
    check.eq("state", want.state.clone(), format!("{:?}", snap.state));
    check.eq(
        "completion",
        want.completion.clone(),
        snap.completion
            .map_or("None".to_string(), |c| format!("{c:?}")),
    );
    check.float(
        "elapsed_seconds",
        want.elapsed_seconds,
        snap.elapsed_seconds,
    );
    if let Some(r) = want.remaining_seconds {
        match snap.remaining_seconds {
            Some(got) => check.float("remaining_seconds", Some(r), got),
            None => check.fail("remaining_seconds: expected a value, got None".into()),
        }
    }
    check.float("wpm", want.wpm, snap.wpm);
    check.float("net_wpm", want.net_wpm, snap.net_wpm);
    check.float("accuracy", want.accuracy, snap.accuracy);
    check.float("kdph", want.kdph, snap.kdph);
    check.float("consistency", want.consistency, snap.consistency);
    check.float("word_accuracy", want.word_accuracy, snap.word_accuracy);
    check.eq("error_count", want.error_count, snap.error_count);
    check.eq("wrong", want.wrong, snap.breakdown.wrong);
    check.eq("extra", want.extra, snap.breakdown.extra);
    check.eq("missing", want.missing, snap.breakdown.missing);
    check.eq("space_errors", want.space_errors, snap.breakdown.space);
    check.eq("backspaces", want.backspaces, snap.backspaces);
    check.eq(
        "characters_typed",
        want.characters_typed,
        snap.characters_typed,
    );
    check.eq(
        "outcomes",
        want.outcomes.clone(),
        outcome_codes(&snap.outcomes),
    );
    check.eq(
        "word_outcomes",
        want.word_outcomes.clone(),
        word_codes(&snap.word_outcomes),
    );
    check.eq("cursor", want.cursor, snap.cursor);
    let units = s.matcher().target_units();
    let current = snap
        .current_word
        .and_then(|w| eval.word_spans.get(w))
        .map(|span| {
            units[span.clone()]
                .iter()
                .map(|u| u.as_text().into_owned())
                .collect::<String>()
        });
    check.eq("current_word", want.current_word.clone(), current);
    check.eq(
        "typed_text",
        want.typed_text.clone(),
        s.matcher().typed().to_string(),
    );
    check.eq(
        "key_states",
        want.key_states.clone(),
        state_codes(&unit_states(&eval, DisplayMode::Key)),
    );
    check.eq(
        "word_states",
        want.word_states.clone(),
        state_codes(&unit_states(&eval, DisplayMode::Word)),
    );
    let rows = s.key_stats().sorted();
    let errors: BTreeMap<String, u32> = rows
        .iter()
        .filter(|(_, t)| t.errors > 0)
        .map(|(k, t)| (k.as_text().into_owned(), t.errors))
        .collect();
    let attempts: BTreeMap<String, u32> = rows
        .iter()
        .map(|(k, t)| (k.as_text().into_owned(), t.attempts))
        .collect();
    check.eq("key_errors", want.key_errors.clone(), errors);
    check.eq("key_attempts", want.key_attempts.clone(), attempts);
    check.eq("finished", want.finished, eval.finished);
    check.eq("pause_error", want.pause_error, pause_error);
    let kinds: Vec<String> = snap
        .outcomes
        .iter()
        .filter(|o| o.is_error())
        .map(|o| format!("{:?}", classify_outcome(o)))
        .collect();
    check.eq("error_kinds", want.error_kinds.clone(), kinds);
    let words: Vec<String> = word_mistakes(&eval)
        .iter()
        .map(|m| format!("{m:?}"))
        .collect();
    check.eq("word_mistakes", want.word_mistakes.clone(), words);
    check.eq("unit_half", want.unit_half, snap.mistakes.unit_half());
    check.eq("unit_full", want.unit_full, snap.mistakes.unit_full());
}

fn single_char(s: &Option<String>, check: &mut Check) -> Option<Option<char>> {
    match s {
        None => Some(None),
        Some(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(Some(c)),
                _ => {
                    check.fail(format!("classify input {text:?} is not one char"));
                    None
                }
            }
        }
    }
}

fn run_classify(input: &Value, want: &Expected, check: &mut Check) {
    let input: ClassifyInput = match serde_json::from_value(input.clone()) {
        Ok(i) => i,
        Err(e) => return check.fail(format!("bad classify input: {e}")),
    };
    let (Some(expected), Some(typed)) = (
        single_char(&input.expected, check),
        single_char(&input.typed, check),
    ) else {
        return;
    };
    let outcome = match (expected, typed) {
        (Some(e), Some(t)) if e == t => CharOutcome::Correct(t),
        (Some(e), Some(t)) => CharOutcome::Wrong {
            expected: e,
            typed: t,
        },
        (Some(e), None) => CharOutcome::Missing(e),
        (None, Some(t)) => CharOutcome::Extra(t),
        (None, None) => return check.fail("classify needs expected or typed".into()),
    };
    let kind = classify_outcome(&outcome);
    check.eq(
        "mistake_kind",
        want.mistake_kind.clone(),
        format!("{kind:?}"),
    );
    check.eq(
        "severity",
        want.severity.clone(),
        format!("{:?}", kind.severity()),
    );
}

fn run_case(case: &Case, check: &mut Check) {
    let want: Expected = match serde_json::from_value(case.radha_expected.clone()) {
        Ok(w) => w,
        Err(e) => return check.fail(format!("bad radha_expected: {e}")),
    };
    match case.kind.as_str() {
        "score" => run_score(&case.input, &want, check),
        "session" => match serde_json::from_value::<SessionInput>(case.input.clone()) {
            Ok(input) => match input.unit.as_str() {
                "char" => run_session::<char>(&input, &want, check),
                "akshar" => run_session::<Akshar>(&input, &want, check),
                other => check.fail(format!("unknown unit {other}")),
            },
            Err(e) => check.fail(format!("bad session input: {e}")),
        },
        "classify" => run_classify(&case.input, &want, check),
        "segment" => match serde_json::from_value::<SegmentInput>(case.input.clone()) {
            Ok(input) => check.eq(
                "clusters",
                want.clusters.clone(),
                akshar::segment(&input.text),
            ),
            Err(e) => check.fail(format!("bad segment input: {e}")),
        },
        "consistency" => match serde_json::from_value::<ConsistencyInput>(case.input.clone()) {
            Ok(input) => {
                let intervals: Vec<Duration> = input
                    .intervals_ms
                    .iter()
                    .map(|&ms| Duration::from_secs_f64(ms / 1000.0))
                    .collect();
                check.float("consistency", want.consistency, consistency(&intervals));
            }
            Err(e) => check.fail(format!("bad consistency input: {e}")),
        },
        other => check.fail(format!("unknown kind {other}")),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn every_golden_vector_meets_radhas_expected_value() {
    let mut failures = Vec::new();
    for (file, fixture) in load_all() {
        for case in &fixture.cases {
            let id = format!("{file}::{}", case.name);
            run_case(
                case,
                &mut Check {
                    id: &id,
                    failures: &mut failures,
                },
            );
        }
    }
    assert!(
        failures.is_empty(),
        "{} golden-vector failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_fixture_has_provenance() {
    let mut problems = Vec::new();
    let fixtures = load_all();
    assert!(
        fixtures.len() >= 14,
        "expected 14 fixture files, found {}",
        fixtures.len()
    );
    for (file, f) in &fixtures {
        if f.schema_version != 1 {
            problems.push(format!("{file}: schema_version {}", f.schema_version));
        }
        if f.source.path.trim().is_empty() {
            problems.push(format!("{file}: empty source path"));
        }
        let hex = &f.source.sha256;
        if hex.len() != 64
            || !hex
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        {
            problems.push(format!("{file}: sha256 is not 64 lowercase hex"));
        }
        if !matches!(f.source.origin.as_str(), "golu-expert-ai" | "radha-prd") {
            problems.push(format!("{file}: unknown origin {}", f.source.origin));
        }
        let mut names = HashSet::new();
        for c in &f.cases {
            if !names.insert(c.name.as_str()) {
                problems.push(format!("{file}: duplicate case {}", c.name));
            }
            match c.tag.as_str() {
                "must_match" => {}
                "intentional_divergence" if !c.reason.trim().is_empty() => {}
                "intentional_divergence" => {
                    problems.push(format!("{file}::{}: divergence without reason", c.name))
                }
                other => problems.push(format!("{file}::{}: unknown tag {other}", c.name)),
            }
            if c.radha_expected.as_object().is_none_or(|o| o.is_empty()) {
                problems.push(format!("{file}::{}: empty radha_expected", c.name));
            }
            if c.golu_expected.is_null() && f.source.origin == "golu-expert-ai" {
                problems.push(format!("{file}::{}: missing golu_expected", c.name));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "fixture problems:\n{}",
        problems.join("\n")
    );
}

#[test]
fn golden_vector_counts() {
    let fixtures = load_all();
    let all: Vec<&Case> = fixtures.iter().flat_map(|(_, f)| f.cases.iter()).collect();
    let must = all.iter().filter(|c| c.tag == "must_match").count();
    let div = all
        .iter()
        .filter(|c| c.tag == "intentional_divergence")
        .count();
    println!(
        "golden vectors: {} total, {must} must_match, {div} intentional_divergence ({} files)",
        all.len(),
        fixtures.len()
    );
    assert!(all.len() >= 90, "only {} golden vectors", all.len());
}
