//! KEY and WORD display modes (`PRD.md` Section 8.3).
//!
//! Both modes render the same evaluation; only the colouring rule differs.
//! Accuracy and error counts never depend on the mode, so switching mode
//! mid-passage recolours the text without changing a single number.
//!
//! - **KEY**: every target unit is coloured the moment it is reached: correct
//!   or wrong (a skipped unit is wrong). Immediate feedback for beginners.
//! - **WORD**: nothing turns red under the typist's fingers. A word is judged
//!   as one unit when it is committed (space, or the end of the text) and all
//!   its units take that verdict. Separators stay neutral. This is what lets
//!   an experienced typist keep going instead of stopping at every red letter.

use crate::matcher::{Evaluation, Outcome, WordOutcome};

/// Which colouring rule the typing screen uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DisplayMode {
    /// Per-unit feedback as each unit is typed.
    Key,
    /// Per-word feedback when each word is committed. The default.
    #[default]
    Word,
}

/// The display state of one target unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitState {
    /// Not judged yet.
    Pending,
    /// Shown as correct.
    Correct,
    /// Shown as wrong.
    Wrong,
}

/// The state of every target unit under `mode`, one entry per target unit.
#[must_use]
pub fn unit_states<U>(evaluation: &Evaluation<U>, mode: DisplayMode) -> Vec<UnitState> {
    match mode {
        DisplayMode::Key => evaluation
            .target_outcome
            .iter()
            .map(|slot| match slot.and_then(|i| evaluation.outcomes.get(i)) {
                None => UnitState::Pending,
                Some(Outcome::Correct(_)) => UnitState::Correct,
                Some(_) => UnitState::Wrong,
            })
            .collect(),
        DisplayMode::Word => {
            let mut states = vec![UnitState::Pending; evaluation.target_outcome.len()];
            for (span, outcome) in evaluation
                .word_spans
                .iter()
                .zip(evaluation.word_outcomes.iter())
            {
                let verdict = match outcome {
                    WordOutcome::Correct => UnitState::Correct,
                    WordOutcome::Incorrect => UnitState::Wrong,
                };
                for state in states.iter_mut().take(span.end).skip(span.start) {
                    *state = verdict;
                }
            }
            states
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::Matcher;

    fn states(target: &str, typed: &str, mode: DisplayMode) -> String {
        let mut m = Matcher::new(target);
        typed.chars().for_each(|c| m.push(c));
        unit_states(&m.evaluate_detailed(), mode)
            .iter()
            .map(|s| match s {
                UnitState::Pending => 'P',
                UnitState::Correct => 'C',
                UnitState::Wrong => 'W',
            })
            .collect()
    }

    #[test]
    fn word_mode_judges_a_whole_word_on_space() {
        assert_eq!(
            states("green apple", "gra", DisplayMode::Word),
            "PPPPPPPPPPP"
        );
        assert_eq!(
            states("green apple", "graan ", DisplayMode::Word),
            "WWWWWPPPPPP"
        );
        assert_eq!(
            states("green apple", "green", DisplayMode::Word),
            "PPPPPPPPPPP"
        );
        assert_eq!(
            states("green apple", "green ", DisplayMode::Word),
            "CCCCCPPPPPP"
        );
    }

    #[test]
    fn key_mode_colours_each_unit_at_once() {
        assert_eq!(
            states("green apple", "gra", DisplayMode::Key),
            "CCWPPPPPPPP"
        );
        // A skipped unit is wrong; the separator typed is correct.
        assert_eq!(
            states("visited every", "visite ", DisplayMode::Key),
            "CCCCCCWCPPPPP"
        );
    }

    #[test]
    fn word_mode_is_the_default() {
        assert_eq!(DisplayMode::default(), DisplayMode::Word);
    }
}
