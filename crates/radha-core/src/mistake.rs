//! Mistake classification: what kind of slip each error is, and whether a
//! rule set would treat it as a half or a full mistake.
//!
//! Pure functions only. Classification is reported, never applied: default
//! scoring stays `PRD.md` Section 8.2 (every error counts once). Which scheme
//! an exam uses, and with what half-mistake weight, is exam policy that lives
//! in the exam profiles, not here.
//!
//! Two schemes are exposed because the sources define two:
//!
//! - **Unit level** ([`MistakeKind`]): Golu Expert AI's `MistakeClassifier`,
//!   reproduced check-for-check so its golden vectors match. Order matters:
//!   case, then whitespace, then punctuation, then wrong character.
//! - **Word level** ([`WordMistake`]): the court-style typing-test vocabulary
//!   (omission = full; case, spacing, spelling = half), applied to one
//!   committed word. "Wrong word substitution" and "word addition" need a
//!   similarity threshold no verified source defines, so they are not
//!   detected here (see `docs/golu-gap-log.md`).

use crate::matcher::{Evaluation, Outcome, WordOutcome};
use crate::unit::TextUnit;

/// How heavy a mistake is under a half/full rule set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Not a mistake.
    None,
    /// A half mistake.
    Half,
    /// A full mistake.
    Full,
}

/// What kind of slip one unit-level outcome is (Golu `MistakeKind`, plus
/// [`MistakeKind::Missing`] for Radha's skipped-letter outcome).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MistakeKind {
    /// Not a mistake.
    None,
    /// Same letter, wrong case. Half.
    WrongCase,
    /// A whitespace slip. Half.
    Spacing,
    /// A punctuation slip. Half.
    Punctuation,
    /// A genuinely different unit. Full.
    WrongCharacter,
    /// A unit typed with no target slot. Full.
    Extra,
    /// A target unit skipped with space. Full (Golu charges skipped slots as
    /// full mistakes).
    Missing,
}

impl MistakeKind {
    /// Half/full split, exactly Golu's `IsHalf` / `IsFull`.
    #[must_use]
    pub fn severity(self) -> Severity {
        match self {
            MistakeKind::None => Severity::None,
            MistakeKind::WrongCase | MistakeKind::Spacing | MistakeKind::Punctuation => {
                Severity::Half
            }
            MistakeKind::WrongCharacter | MistakeKind::Extra | MistakeKind::Missing => {
                Severity::Full
            }
        }
    }
}

/// Classify a typed unit against the unit expected in that slot.
///
/// Same order as Golu: exact → none; equal ignoring case → wrong case; either
/// whitespace → spacing; either punctuation → punctuation; else wrong
/// character. A punctuation mark in the wrong case resolves to the earlier
/// rule.
#[must_use]
pub fn classify_units<U: TextUnit>(expected: &U, typed: &U) -> MistakeKind {
    if expected == typed {
        MistakeKind::None
    } else if expected.eq_ignore_case(typed) {
        MistakeKind::WrongCase
    } else if expected.is_whitespace() || typed.is_whitespace() {
        MistakeKind::Spacing
    } else if expected.is_punctuation() || typed.is_punctuation() {
        MistakeKind::Punctuation
    } else {
        MistakeKind::WrongCharacter
    }
}

/// Classify one matcher outcome.
#[must_use]
pub fn classify_outcome<U: TextUnit>(outcome: &Outcome<U>) -> MistakeKind {
    match outcome {
        Outcome::Correct(_) => MistakeKind::None,
        Outcome::Wrong { expected, typed } => classify_units(expected, typed),
        Outcome::Missing(_) => MistakeKind::Missing,
        Outcome::Extra(_) => MistakeKind::Extra,
    }
}

/// What kind of mistake one committed word carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordMistake {
    /// The word is correct.
    None,
    /// The whole word was left out. Full.
    Omission,
    /// Only whitespace slips (e.g. a stray double space after it). Half.
    Spacing,
    /// Only letter-case slips. Half.
    Case,
    /// Any other mix of wrong, missing or extra letters. Half.
    Spelling,
}

impl WordMistake {
    /// Half/full split of the word-level vocabulary.
    #[must_use]
    pub fn severity(self) -> Severity {
        match self {
            WordMistake::None => Severity::None,
            WordMistake::Omission => Severity::Full,
            WordMistake::Spacing | WordMistake::Case | WordMistake::Spelling => Severity::Half,
        }
    }
}

/// Classify a word from the outcomes charged to it.
///
/// No errors → none; every outcome missing → omission; every error involves
/// whitespace → spacing; every error is a case slip → case; otherwise
/// spelling.
///
/// The word-aligned matcher never skips a whole word (a double space is an
/// extra space, not a skip), so it never yields an omission today; the rule
/// is here so an exam profile that adds a skip gesture gets it for free.
#[must_use]
pub fn classify_word<'a, U: TextUnit + 'a>(
    outcomes: impl IntoIterator<Item = &'a Outcome<U>>,
) -> WordMistake {
    let mut any = false;
    let mut all_missing = true;
    let mut errors = 0usize;
    let mut spacing = 0usize;
    let mut case = 0usize;
    for o in outcomes {
        any = true;
        if !matches!(o, Outcome::Missing(_)) {
            all_missing = false;
        }
        if !o.is_error() {
            continue;
        }
        errors += 1;
        let spacey = o.typed_unit().is_some_and(TextUnit::is_whitespace)
            || o.expected_unit().is_some_and(TextUnit::is_whitespace);
        if spacey {
            spacing += 1;
        }
        if classify_outcome(o) == MistakeKind::WrongCase {
            case += 1;
        }
    }
    if errors == 0 {
        WordMistake::None
    } else if any && all_missing {
        WordMistake::Omission
    } else if spacing == errors {
        WordMistake::Spacing
    } else if case == errors {
        WordMistake::Case
    } else {
        WordMistake::Spelling
    }
}

/// The classification of every committed word of an evaluation, in order.
#[must_use]
pub fn word_mistakes<U: TextUnit>(evaluation: &Evaluation<U>) -> Vec<WordMistake> {
    evaluation
        .word_outcomes
        .iter()
        .enumerate()
        .map(|(w, outcome)| {
            if *outcome == WordOutcome::Correct {
                return WordMistake::None;
            }
            let charged = evaluation
                .word_charges
                .get(w)
                .map_or(&[][..], Vec::as_slice);
            classify_word(charged.iter().filter_map(|&i| evaluation.outcomes.get(i)))
        })
        .collect()
}

/// Counts of every mistake kind in one evaluation, under both schemes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MistakeCounts {
    /// Unit level: wrong case.
    pub wrong_case: usize,
    /// Unit level: spacing.
    pub spacing: usize,
    /// Unit level: punctuation.
    pub punctuation: usize,
    /// Unit level: wrong character.
    pub wrong_character: usize,
    /// Unit level: extra.
    pub extra: usize,
    /// Unit level: missing.
    pub missing: usize,
    /// Word level: omission.
    pub word_omission: usize,
    /// Word level: spacing.
    pub word_spacing: usize,
    /// Word level: case.
    pub word_case: usize,
    /// Word level: spelling.
    pub word_spelling: usize,
}

impl MistakeCounts {
    /// Count both schemes over `evaluation`.
    #[must_use]
    pub fn from_evaluation<U: TextUnit>(evaluation: &Evaluation<U>) -> Self {
        let mut c = MistakeCounts::default();
        for o in &evaluation.outcomes {
            match classify_outcome(o) {
                MistakeKind::None => {}
                MistakeKind::WrongCase => c.wrong_case += 1,
                MistakeKind::Spacing => c.spacing += 1,
                MistakeKind::Punctuation => c.punctuation += 1,
                MistakeKind::WrongCharacter => c.wrong_character += 1,
                MistakeKind::Extra => c.extra += 1,
                MistakeKind::Missing => c.missing += 1,
            }
        }
        for m in word_mistakes(evaluation) {
            match m {
                WordMistake::None => {}
                WordMistake::Omission => c.word_omission += 1,
                WordMistake::Spacing => c.word_spacing += 1,
                WordMistake::Case => c.word_case += 1,
                WordMistake::Spelling => c.word_spelling += 1,
            }
        }
        c
    }

    /// Unit-level half mistakes.
    #[must_use]
    pub fn unit_half(&self) -> usize {
        self.wrong_case + self.spacing + self.punctuation
    }

    /// Unit-level full mistakes.
    #[must_use]
    pub fn unit_full(&self) -> usize {
        self.wrong_character + self.extra + self.missing
    }

    /// Word-level half mistakes.
    #[must_use]
    pub fn word_half(&self) -> usize {
        self.word_spacing + self.word_case + self.word_spelling
    }

    /// Word-level full mistakes.
    #[must_use]
    pub fn word_full(&self) -> usize {
        self.word_omission
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::{CharOutcome, Matcher};

    #[test]
    fn golu_classifier_rows_keep_the_half_full_split() {
        let rows = [
            ('a', 'A', MistakeKind::WrongCase, Severity::Half),
            ('A', 'a', MistakeKind::WrongCase, Severity::Half),
            (' ', 'x', MistakeKind::Spacing, Severity::Half),
            ('x', ' ', MistakeKind::Spacing, Severity::Half),
            ('.', 'x', MistakeKind::Punctuation, Severity::Half),
            ('x', ',', MistakeKind::Punctuation, Severity::Half),
            ('a', 'b', MistakeKind::WrongCharacter, Severity::Full),
            ('a', 'a', MistakeKind::None, Severity::None),
        ];
        for (expected, typed, kind, severity) in rows {
            let got = classify_units(&expected, &typed);
            assert_eq!(got, kind, "{expected:?} vs {typed:?}");
            assert_eq!(got.severity(), severity);
        }
    }

    #[test]
    fn missing_and_extra_are_full() {
        assert_eq!(
            classify_outcome(&CharOutcome::Missing('a')).severity(),
            Severity::Full
        );
        assert_eq!(
            classify_outcome(&CharOutcome::Extra('a')).severity(),
            Severity::Full
        );
        assert_eq!(
            classify_outcome(&CharOutcome::Correct('a')),
            MistakeKind::None
        );
    }

    fn words(target: &str, typed: &str) -> Vec<WordMistake> {
        let mut m = Matcher::new(target);
        typed.chars().for_each(|c| m.push(c));
        word_mistakes(&m.evaluate_detailed())
    }

    #[test]
    fn word_level_kinds() {
        assert_eq!(
            words("asdf jkl;", "asdf Jkl;"),
            [WordMistake::None, WordMistake::Case]
        );
        assert_eq!(
            words("asdf jkl;", "asdf jxl;"),
            [WordMistake::None, WordMistake::Spelling]
        );
        assert_eq!(words("visited every", "visite "), [WordMistake::Spelling]);
        assert_eq!(words("cat sat", "cat  "), [WordMistake::Spacing]);
        assert_eq!(WordMistake::Spelling.severity(), Severity::Half);
    }

    #[test]
    fn omission_is_all_missing() {
        let all_missing = [CharOutcome::Missing('a'), CharOutcome::Missing('b')];
        assert_eq!(classify_word(&all_missing), WordMistake::Omission);
        assert_eq!(WordMistake::Omission.severity(), Severity::Full);
        assert_eq!(classify_word(&[] as &[CharOutcome]), WordMistake::None);
    }

    #[test]
    fn counts_cover_both_schemes() {
        let mut m = Matcher::new("Go now x");
        "go nw ".chars().for_each(|c| m.push(c));
        let c = MistakeCounts::from_evaluation(&m.evaluate_detailed());
        assert_eq!(c.wrong_case, 1);
        assert_eq!(c.wrong_character, 1, "'w' for 'o'");
        assert_eq!(c.missing, 1, "'w' skipped");
        assert_eq!((c.unit_half(), c.unit_full()), (1, 2));
        assert_eq!((c.word_case, c.word_spelling), (1, 1));
        assert_eq!((c.word_half(), c.word_full()), (2, 0));
    }
}
