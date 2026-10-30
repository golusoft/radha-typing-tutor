//! Accuracy and error counting, per `PRD.md` Section 8.2.
//!
//! Free functions over matcher outcomes, kept separate from timing on purpose:
//! these depend only on *what* was typed, while WPM also depends on *when*
//! (see [`super::speed`]).

use crate::matcher::{Outcome, WordOutcome};
use crate::unit::TextUnit;

/// Character accuracy: correct outcomes as a percentage of all outcomes.
///
/// 100.0 when there are no outcomes: nothing typed yet is not a mistake, and
/// the live display should not open at 0%.
#[must_use]
pub fn accuracy<U>(outcomes: &[Outcome<U>]) -> f64 {
    let correct = outcomes.iter().filter(|o| !o.is_error()).count();
    percentage(correct, outcomes.len())
}

/// Total errors: wrong + missing + extra units, the `ERRORS` metric of
/// `PRD.md` Section 8.2. Every error counts once; there is no weighting.
#[must_use]
pub fn error_count<U>(outcomes: &[Outcome<U>]) -> usize {
    outcomes.iter().filter(|o| o.is_error()).count()
}

/// Word accuracy: correct words as a percentage of committed words, for the
/// WORD-mode result. 100.0 when no word has been committed yet.
#[must_use]
pub fn word_accuracy(words: &[WordOutcome]) -> f64 {
    let correct = words.iter().filter(|&&w| w == WordOutcome::Correct).count();
    percentage(correct, words.len())
}

/// Characters typed, for WPM and KDPH: the code points of every unit the user
/// actually produced.
///
/// Missing units were never typed, so skipping letters with space must not
/// inflate speed (unlike Golu, which counts each skipped slot as a
/// keystroke). For `char` this is exactly "outcomes minus Missing"; for
/// aksharas it counts code points, which is what a Unicode layout emits per
/// key press.
#[must_use]
pub fn characters_typed<U: TextUnit>(outcomes: &[Outcome<U>]) -> usize {
    outcomes
        .iter()
        .filter_map(Outcome::typed_unit)
        .map(TextUnit::code_points)
        .sum()
}

/// The error total split by kind, for the WORD/KEY result tables.
///
/// `wrong + extra + missing` always equals [`error_count`]. `space` is a
/// *view* across them (errors whose expected or typed unit is whitespace: a
/// stray double space, a space where a letter belongs), not a fourth bucket,
/// so it is never added to the total.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ErrorBreakdown {
    /// A different unit typed in a slot.
    pub wrong: usize,
    /// Units typed with no target slot.
    pub extra: usize,
    /// Target units skipped with space.
    pub missing: usize,
    /// Errors involving whitespace (subset of the three above).
    pub space: usize,
}

impl ErrorBreakdown {
    /// Count the errors in `outcomes`.
    #[must_use]
    pub fn from_outcomes<U: TextUnit>(outcomes: &[Outcome<U>]) -> Self {
        let mut b = ErrorBreakdown::default();
        for o in outcomes {
            match o {
                Outcome::Correct(_) => continue,
                Outcome::Wrong { .. } => b.wrong += 1,
                Outcome::Extra(_) => b.extra += 1,
                Outcome::Missing(_) => b.missing += 1,
            }
            let spacey = o.typed_unit().is_some_and(TextUnit::is_whitespace)
                || o.expected_unit().is_some_and(TextUnit::is_whitespace);
            if spacey {
                b.space += 1;
            }
        }
        b
    }

    /// `wrong + extra + missing`.
    #[must_use]
    pub fn total(&self) -> usize {
        self.wrong + self.extra + self.missing
    }
}

/// `part` as a percentage of `total`, with an empty total counting as 100%.
fn percentage(part: usize, total: usize) -> f64 {
    if total == 0 {
        return 100.0;
    }
    // Multiplying first keeps whole-number results exact (8 of 10 is 80.0).
    part as f64 * 100.0 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::CharOutcome;

    #[test]
    fn all_correct_is_one_hundred_percent() {
        assert_eq!(accuracy(&[CharOutcome::Correct('a'); 5]), 100.0);
    }

    #[test]
    fn nothing_typed_is_one_hundred_percent() {
        assert_eq!(accuracy::<char>(&[]), 100.0);
    }

    #[test]
    fn two_wrong_out_of_ten_is_eighty_percent() {
        let mut outcomes = vec![CharOutcome::Correct('a'); 8];
        outcomes.extend(
            [CharOutcome::Wrong {
                expected: 'a',
                typed: 'b',
            }; 2],
        );
        assert_eq!(accuracy(&outcomes), 80.0);
    }

    #[test]
    fn error_count_is_wrong_plus_missing_plus_extra() {
        let outcomes = [
            CharOutcome::Correct('a'),
            CharOutcome::Wrong {
                expected: 'b',
                typed: 'x',
            },
            CharOutcome::Missing('c'),
            CharOutcome::Missing('d'),
            CharOutcome::Extra('e'),
            CharOutcome::Correct('f'),
        ];
        assert_eq!(error_count(&outcomes), 4);
        assert_eq!(error_count::<char>(&[]), 0);
    }

    #[test]
    fn word_accuracy_counts_committed_words() {
        assert_eq!(word_accuracy(&[]), 100.0);
        assert_eq!(
            word_accuracy(&[
                WordOutcome::Correct,
                WordOutcome::Incorrect,
                WordOutcome::Correct,
                WordOutcome::Correct
            ]),
            75.0
        );
        assert_eq!(word_accuracy(&[WordOutcome::Incorrect; 3]), 0.0);
    }

    #[test]
    fn characters_typed_excludes_skipped_letters() {
        let outcomes = [
            CharOutcome::Correct('o'),
            CharOutcome::Missing('n'),
            CharOutcome::Extra(' '),
            CharOutcome::Wrong {
                expected: 'a',
                typed: 'b',
            },
        ];
        assert_eq!(characters_typed(&outcomes), 3);
    }

    #[test]
    fn breakdown_reconciles_with_error_count() {
        let outcomes = [
            CharOutcome::Correct('a'),
            CharOutcome::Wrong {
                expected: 'b',
                typed: 'x',
            },
            CharOutcome::Missing('c'),
            CharOutcome::Extra(' '),
            CharOutcome::Extra('e'),
        ];
        let b = ErrorBreakdown::from_outcomes(&outcomes);
        assert_eq!((b.wrong, b.extra, b.missing, b.space), (1, 2, 1, 1));
        assert_eq!(b.total(), error_count(&outcomes));
    }
}
