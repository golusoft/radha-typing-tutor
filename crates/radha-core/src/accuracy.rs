//! Accuracy and error counting, per `PRD.md` Section 8.2.
//!
//! Free functions over matcher outcomes, kept separate from timing on purpose:
//! these depend only on *what* was typed, while WPM also depends on *when*
//! (see [`crate::speed`]).

use crate::matcher::{CharOutcome, WordOutcome};

/// Character accuracy: correct outcomes as a percentage of all outcomes.
///
/// 100.0 when there are no outcomes: nothing typed yet is not a mistake, and
/// the live display should not open at 0%.
#[must_use]
pub fn accuracy(outcomes: &[CharOutcome]) -> f64 {
    let correct = outcomes.iter().filter(|o| !o.is_error()).count();
    percentage(correct, outcomes.len())
}

/// Total errors: wrong + missing + extra characters, the `ERRORS` metric of
/// `PRD.md` Section 8.2. Every error counts once; there is no weighting.
#[must_use]
pub fn error_count(outcomes: &[CharOutcome]) -> usize {
    outcomes.iter().filter(|o| o.is_error()).count()
}

/// Word accuracy: correct words as a percentage of committed words, for the
/// WORD-mode result. 100.0 when no word has been committed yet.
#[must_use]
pub fn word_accuracy(words: &[WordOutcome]) -> f64 {
    let correct = words.iter().filter(|&&w| w == WordOutcome::Correct).count();
    percentage(correct, words.len())
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

    #[test]
    fn all_correct_is_one_hundred_percent() {
        assert_eq!(accuracy(&[CharOutcome::Correct('a'); 5]), 100.0);
    }

    #[test]
    fn nothing_typed_is_one_hundred_percent() {
        assert_eq!(accuracy(&[]), 100.0);
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
        assert_eq!(error_count(&[]), 0);
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
}
