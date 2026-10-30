//! Exam scoring policies (EXAM-MODE-AND-GOLU-REUSE-PLAN.md Section 5.4).
//!
//! The typing engine scores every session the same way (PRD 8.2). A policy
//! turns those figures into the exam's own measure and, when the profile's
//! certification allows it, a pass/fail verdict. Policies are chosen by data
//! (`scoring = "…"` in a profile); none is tied to an exam name.

use crate::field::Status;
use crate::profile::{ExamProfile, Qualification, TypingTest};
use crate::Certification;

/// Which policy a test is scored with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyKind {
    /// Metrics only, never a verdict.
    PracticeOnly,
    /// Correct words per minute against a threshold.
    CorrectWpmThreshold,
    /// Accuracy percent taken as marks out of `max_marks`.
    AccuracyAsMarks,
    /// Full and half mistakes, weighted.
    FullHalfMistake,
}

impl PolicyKind {
    /// The TOML spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyKind::PracticeOnly => "practice_only",
            PolicyKind::CorrectWpmThreshold => "correct_wpm_threshold",
            PolicyKind::AccuracyAsMarks => "accuracy_as_marks",
            PolicyKind::FullHalfMistake => "full_half_mistake",
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "practice_only" => Some(PolicyKind::PracticeOnly),
            "correct_wpm_threshold" => Some(PolicyKind::CorrectWpmThreshold),
            "accuracy_as_marks" => Some(PolicyKind::AccuracyAsMarks),
            "full_half_mistake" => Some(PolicyKind::FullHalfMistake),
            _ => None,
        }
    }
}

/// A finished session's figures, as the engine computed them.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TestInput {
    pub gross_wpm: f64,
    pub net_wpm: f64,
    /// Percent, 0..=100.
    pub accuracy: f64,
    pub words_typed: u32,
    pub words_correct: u32,
    pub full_mistakes: u32,
    pub half_mistakes: u32,
    /// Active typing time.
    pub elapsed_seconds: f64,
}

impl TestInput {
    fn minutes(&self) -> f64 {
        self.elapsed_seconds / 60.0
    }
}

/// Pass / fail under a policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Qualified,
    NotQualified,
}

/// What a policy made of a session.
#[derive(Debug, Clone, PartialEq)]
pub struct TestOutcome {
    pub certification: Certification,
    /// The policy that produced the figures (PracticeOnly whenever the
    /// certification is).
    pub policy: PolicyKind,
    /// The exam measure ("marks", "correct WPM" …) and its value, rounded
    /// to two decimals; the verdict compares exactly this value.
    pub measure: Option<(&'static str, f64)>,
    pub threshold: Option<f64>,
    /// `None` unless the certification gives a verdict.
    pub verdict: Option<Verdict>,
    pub formula: String,
    /// The formula with this session's numbers.
    pub worked: String,
    /// Unverified parts of the rules that shaped this outcome.
    pub notes: Vec<String>,
}

/// One scoring policy.
pub trait ScoringPolicy {
    /// Which policy this is.
    fn kind(&self) -> PolicyKind;
    /// The formula, as written for `test`.
    fn formula(&self, test: &TypingTest) -> String;
    /// The exam measure for `input` (unrounded) and the worked formula.
    fn measure(&self, input: &TestInput, test: &TypingTest) -> Option<(&'static str, f64, String)>;
}

/// Round half away from zero to two decimals (the value shown is the value
/// compared).
#[must_use]
pub fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

struct PracticeOnlyPolicy;
struct CorrectWpmThresholdPolicy;
struct AccuracyAsMarksPolicy;
struct FullHalfMistakePolicy;

impl ScoringPolicy for PracticeOnlyPolicy {
    fn kind(&self) -> PolicyKind {
        PolicyKind::PracticeOnly
    }
    fn formula(&self, _test: &TypingTest) -> String {
        "No exam formula: this exam's rules are not verified, so speed and accuracy are \
         practice figures (Net WPM per PRD 8.2) and there is no pass or fail."
            .to_owned()
    }
    fn measure(&self, _: &TestInput, _: &TypingTest) -> Option<(&'static str, f64, String)> {
        None
    }
}

impl ScoringPolicy for CorrectWpmThresholdPolicy {
    fn kind(&self) -> PolicyKind {
        PolicyKind::CorrectWpmThreshold
    }
    fn formula(&self, _test: &TypingTest) -> String {
        "Correct WPM = correct words ÷ minutes (Radha's practice definition; the notice's \
         definition is not verified)"
            .to_owned()
    }
    fn measure(&self, input: &TestInput, _: &TypingTest) -> Option<(&'static str, f64, String)> {
        let minutes = input.minutes();
        if !(minutes > 0.0 && minutes.is_finite()) {
            return Some(("correct WPM", 0.0, "No active typing time.".to_owned()));
        }
        let v = f64::from(input.words_correct) / minutes;
        Some((
            "correct WPM",
            v,
            format!(
                "{} ÷ {minutes:.2} = {:.2} correct WPM",
                input.words_correct,
                round2(v)
            ),
        ))
    }
}

impl ScoringPolicy for AccuracyAsMarksPolicy {
    fn kind(&self) -> PolicyKind {
        PolicyKind::AccuracyAsMarks
    }
    fn formula(&self, test: &TypingTest) -> String {
        let max = test.max_marks.effective().copied().unwrap_or(100);
        format!(
            "Marks = accuracy % × {max} ÷ 100 (how accuracy is counted is {}; Radha's \
             accuracy of the typed text is used)",
            test.error_policy.status().label()
        )
    }
    fn measure(&self, input: &TestInput, test: &TypingTest) -> Option<(&'static str, f64, String)> {
        let max = f64::from(test.max_marks.effective().copied().unwrap_or(100));
        let acc = input.accuracy.clamp(0.0, 100.0);
        let marks = acc * max / 100.0;
        Some((
            "marks",
            marks,
            format!("{acc:.2}% × {max:.0} ÷ 100 = {:.2} marks", round2(marks)),
        ))
    }
}

impl FullHalfMistakePolicy {
    fn weights(test: &TypingTest) -> (f64, f64) {
        test.mistake_weights
            .effective()
            .map_or((1.0, 0.5), |w| (w.full, w.half))
    }
}

impl ScoringPolicy for FullHalfMistakePolicy {
    fn kind(&self) -> PolicyKind {
        PolicyKind::FullHalfMistake
    }
    fn formula(&self, test: &TypingTest) -> String {
        let (f, h) = Self::weights(test);
        let base = format!("mistakes = full × {f} + half × {h}");
        match test.qualification.effective() {
            Some(Qualification::MinNetWpm(_)) => {
                format!("{base}; net WPM = max(0, words typed − mistakes) ÷ minutes")
            }
            _ => format!("{base}; accuracy = max(0, 100 × (1 − mistakes ÷ words typed))"),
        }
    }
    fn measure(&self, input: &TestInput, test: &TypingTest) -> Option<(&'static str, f64, String)> {
        let (wf, wh) = Self::weights(test);
        let weighted = f64::from(input.full_mistakes) * wf + f64::from(input.half_mistakes) * wh;
        let words = f64::from(input.words_typed);
        match test.qualification.effective() {
            Some(Qualification::MinNetWpm(_)) => {
                let minutes = input.minutes();
                if !(minutes > 0.0 && minutes.is_finite()) {
                    return Some(("net WPM", 0.0, "No active typing time.".to_owned()));
                }
                let v = (words - weighted).max(0.0) / minutes;
                Some((
                    "net WPM",
                    v,
                    format!(
                        "max(0, {words:.0} − {weighted:.1}) ÷ {minutes:.2} = {:.2} net WPM",
                        round2(v)
                    ),
                ))
            }
            _ => {
                let v = if words > 0.0 {
                    (100.0 * (1.0 - weighted / words)).max(0.0)
                } else {
                    100.0
                };
                Some((
                    "accuracy %",
                    v,
                    format!("100 × (1 − {weighted:.1} ÷ {words:.0}) = {:.2}%", round2(v)),
                ))
            }
        }
    }
}

/// The policy implementation of `kind`.
#[must_use]
pub fn policy(kind: PolicyKind) -> &'static dyn ScoringPolicy {
    match kind {
        PolicyKind::PracticeOnly => &PracticeOnlyPolicy,
        PolicyKind::CorrectWpmThreshold => &CorrectWpmThresholdPolicy,
        PolicyKind::AccuracyAsMarks => &AccuracyAsMarksPolicy,
        PolicyKind::FullHalfMistake => &FullHalfMistakePolicy,
    }
}

/// Score `input` under `test` of `profile`.
#[must_use]
pub fn evaluate(profile: &ExamProfile, test: &TypingTest, input: &TestInput) -> TestOutcome {
    let certification = profile.test_certification(test);
    let kind = if certification.gives_verdict() {
        test.scoring
    } else {
        PolicyKind::PracticeOnly
    };
    let p = policy(kind);
    let measured = p.measure(input, test);
    let threshold = test.qualification.known().map(|q| q.threshold());
    let measure = measured.as_ref().map(|(label, v, _)| (*label, round2(*v)));
    let verdict = match (certification.gives_verdict(), measure, threshold) {
        (true, Some((_, v)), Some(t)) => Some(if v >= t {
            Verdict::Qualified
        } else {
            Verdict::NotQualified
        }),
        _ => None,
    };
    let worked = measured.map_or_else(
        || {
            format!(
                "Practice figures: {:.1} gross WPM, {:.1} net WPM, {:.1}% accuracy.",
                input.gross_wpm, input.net_wpm, input.accuracy
            )
        },
        |(_, _, w)| w,
    );
    let mut notes = Vec::new();
    let mut note = |what: &str, status: Status, extra: Option<&str>| {
        if status != Status::Verified {
            let mut s = format!("{what}: {}", status.label());
            if let Some(e) = extra {
                s.push_str(" (");
                s.push_str(e);
                s.push(')');
            }
            notes.push(s);
        }
    };
    note(
        "Threshold",
        test.qualification.status(),
        test.qualification.note(),
    );
    note(
        "Duration",
        test.duration_seconds.status(),
        test.duration_seconds.note(),
    );
    note(
        "Error policy",
        test.error_policy.status(),
        test.error_policy.note(),
    );
    note("Backspace", test.backspace.status(), test.backspace.note());
    if kind == PolicyKind::FullHalfMistake {
        note(
            "Mistake weights",
            test.mistake_weights.status(),
            test.mistake_weights.note(),
        );
    }
    TestOutcome {
        certification,
        policy: kind,
        measure,
        threshold,
        verdict,
        formula: p.formula(test),
        worked,
        notes,
    }
}

/// A test's policy must fit its qualification (checked at load).
pub(crate) fn check_policy(test: &TypingTest) -> Result<(), String> {
    let q = test.qualification.effective().copied();
    let fits = match (test.scoring, q) {
        (_, None) => true,
        (PolicyKind::PracticeOnly, Some(_)) => test.qualification.known().is_none(),
        (PolicyKind::CorrectWpmThreshold, Some(q)) => {
            matches!(q, Qualification::MinCorrectWpm(_))
        }
        (PolicyKind::AccuracyAsMarks, Some(q)) => matches!(q, Qualification::MinMarks(_)),
        (PolicyKind::FullHalfMistake, Some(q)) => {
            matches!(
                q,
                Qualification::MinAccuracy(_) | Qualification::MinNetWpm(_)
            )
        }
    };
    if !fits {
        return Err(format!(
            "policy {} cannot judge a {} threshold",
            test.scoring.as_str(),
            q.map_or("missing", Qualification::measure)
        ));
    }
    if test.scoring == PolicyKind::AccuracyAsMarks
        && test.qualification.known().is_some()
        && test.max_marks.known().is_none()
    {
        return Err("accuracy_as_marks with a known threshold needs known max_marks".to_owned());
    }
    if test.scoring == PolicyKind::FullHalfMistake && test.mistake_weights.effective().is_none() {
        return Err(
            "full_half_mistake needs mistake_weights (a configurable default is fine)".to_owned(),
        );
    }
    Ok(())
}
