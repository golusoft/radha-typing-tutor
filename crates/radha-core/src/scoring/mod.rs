//! The one scoring module (RULES.MD Rule 36).
//!
//! Every metric has exactly one formula, and it lives here:
//!
//! | Metric | Formula | File |
//! |---|---|---|
//! | Gross WPM | `(chars / 5) / minutes` | [`speed::wpm`] |
//! | Net WPM | `max(0, gross − errors / minutes)` | [`speed::net_wpm`] |
//! | KDPH | `chars × 3600 / seconds` | [`speed::kdph`] |
//! | Accuracy | `correct × 100 / outcomes` (100 when empty) | [`accuracy::accuracy`] |
//! | Errors | `wrong + missing + extra`, unweighted | [`accuracy::error_count`] |
//! | Word accuracy | `correct words × 100 / committed words` | [`accuracy::word_accuracy`] |
//! | Characters typed | code points of typed units (Missing excluded) | [`accuracy::characters_typed`] |
//! | Consistency | `100 × (1 − min(CV, 1))` of key intervals | [`consistency::consistency`] |
//!
//! [`Score::compute`] is the only aggregator. The live typing snapshot and the
//! final result are both built from it, so the number on screen while typing
//! and the number on the result sheet can never come from different maths.
//!
//! Exam-specific formulas (half-mistake weights, deduction rules,
//! qualification thresholds) are deliberately absent: they are exam policy
//! and belong to the exam profiles, not the engine. Default scoring is
//! `PRD.md` Section 8.2, where every error counts once.

pub mod accuracy;
pub mod consistency;
pub mod speed;

use std::time::Duration;

use crate::matcher::{Outcome, WordOutcome};
use crate::unit::TextUnit;

pub use accuracy::{accuracy, characters_typed, error_count, word_accuracy, ErrorBreakdown};
pub use consistency::consistency;
pub use speed::{kdph, net_wpm, wpm};

/// Every scored number for one moment of one session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    /// Gross words per minute.
    pub wpm: f64,
    /// Net words per minute (PRD 8.2, unweighted errors).
    pub net_wpm: f64,
    /// Unit accuracy, as a percentage.
    pub accuracy: f64,
    /// Wrong + missing + extra units.
    pub error_count: usize,
    /// The error total split by kind.
    pub breakdown: ErrorBreakdown,
    /// Code points the user produced.
    pub characters_typed: usize,
    /// Key depressions per hour.
    pub kdph: f64,
    /// Correct committed words, as a percentage.
    pub word_accuracy: f64,
    /// Rhythm consistency, 0–100.
    pub consistency: f64,
}

impl Score {
    /// Score `outcomes` typed over `elapsed_seconds` of active time, with
    /// `intervals` between accepted key presses for consistency.
    #[must_use]
    pub fn compute<U: TextUnit>(
        outcomes: &[Outcome<U>],
        word_outcomes: &[WordOutcome],
        elapsed_seconds: f64,
        intervals: &[Duration],
    ) -> Self {
        let chars = characters_typed(outcomes);
        let errors = error_count(outcomes);
        let gross = wpm(chars, elapsed_seconds);
        Score {
            wpm: gross,
            net_wpm: net_wpm(gross, errors, elapsed_seconds / 60.0),
            accuracy: accuracy(outcomes),
            error_count: errors,
            breakdown: ErrorBreakdown::from_outcomes(outcomes),
            characters_typed: chars,
            kdph: kdph(chars, elapsed_seconds),
            word_accuracy: word_accuracy(word_outcomes),
            consistency: consistency(intervals),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::CharOutcome;

    #[test]
    fn compute_uses_the_single_formulas() {
        let mut outcomes = vec![CharOutcome::Correct('a'); 294];
        outcomes.extend(
            [CharOutcome::Wrong {
                expected: 'a',
                typed: 'b',
            }; 6],
        );
        let s = Score::compute(&outcomes, &[], 60.0, &[]);
        assert_eq!(s.wpm, 60.0);
        assert_eq!(s.net_wpm, 54.0, "every error counts once (PRD 8.2)");
        assert_eq!(s.accuracy, 98.0);
        assert_eq!(s.kdph, 18_000.0);
        assert_eq!(s.error_count, 6);
        assert_eq!(s.breakdown.total(), 6);
        assert_eq!(s.consistency, 100.0);
    }

    #[test]
    fn an_empty_score_is_neutral() {
        let s = Score::compute::<char>(&[], &[], 0.0, &[]);
        assert_eq!((s.wpm, s.net_wpm, s.kdph), (0.0, 0.0, 0.0));
        assert_eq!(
            (s.accuracy, s.word_accuracy, s.consistency),
            (100.0, 100.0, 100.0)
        );
    }
}
