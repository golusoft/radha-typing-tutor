//! Word-aligned character matching.
//!
//! Compares what the user has typed against the target text and classifies
//! every character reached so far. It is the single source of truth for both
//! display granularities in `PRD.md` Section 8.3: KEY mode renders
//! [`CharOutcome`]s, WORD mode renders [`WordOutcome`]s, and accuracy and error
//! metrics always come from the character level, whichever mode is on screen.
//!
//! # Word-aligned, not positional
//!
//! A purely positional comparison (`typed[i]` against `target[i]`) lets one
//! skipped or extra letter shift every later character, turning a single slip
//! into a paragraph of errors. Here a typed space realigns the cursor to the
//! next target word, and letters typed past the end of a word never consume a
//! target slot, so every error stays inside the word it happened in.
//!
//! # Stateless re-evaluation
//!
//! [`Matcher::evaluate`] recomputes everything from the typed buffer on every
//! call. Backspace is therefore just [`Matcher::pop`]: there is no undo log to
//! keep consistent, and the result can never drift from the buffer.
//!
//! # Logical characters, not keystrokes
//!
//! [`Matcher::push`] takes one output character. A key that produces several
//! characters (common in the Hindi layouts) is pushed once per character by the
//! caller, so the matcher never needs to know about keys.

/// How one typed or skipped character compares with the target.
///
/// Exactly the four KEY-mode states from `PRD.md` Section 8.3. There is
/// deliberately no "not yet typed" variant: characters the user has not reached
/// produce no outcome at all, so every outcome is something the user did (or
/// skipped), and accuracy is never diluted by untouched text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharOutcome {
    /// The typed character matched the target.
    Correct(char),
    /// A different character was typed in this position.
    Wrong {
        /// What the target has here.
        expected: char,
        /// What was typed instead.
        typed: char,
    },
    /// A target character skipped by pressing space in the middle of a word.
    Missing(char),
    /// A typed character with no target slot: letters past the end of a word
    /// or of the text, or a stray space at the start of a word.
    Extra(char),
}

impl CharOutcome {
    /// Whether this outcome counts as an error: anything but [`CharOutcome::Correct`].
    #[must_use]
    pub fn is_error(&self) -> bool {
        !matches!(self, CharOutcome::Correct(_))
    }
}

/// Whether a committed word was typed perfectly, for WORD mode.
///
/// Only committed words get an outcome: a word followed by a typed separator,
/// or the final word once the end of the text is reached. The word in progress
/// has none yet, so WORD mode never shows a word as wrong while it is still
/// being typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordOutcome {
    /// Every character of the word was correct, with nothing extra.
    Correct,
    /// The word carries at least one wrong, missing or extra character.
    Incorrect,
}

/// The target text and the typed buffer, classified against each other on
/// demand.
#[derive(Debug, Clone)]
pub struct Matcher {
    /// The target, trimmed, as characters.
    target: Vec<char>,
    /// `[start, end)` span of each target word, precomputed once because every
    /// evaluation needs them.
    words: Vec<(usize, usize)>,
    /// What the user has typed so far.
    typed: Vec<char>,
}

impl Matcher {
    /// Create a matcher for `target`.
    ///
    /// Leading and trailing whitespace is trimmed: it is invisible on screen,
    /// so requiring the user to type it would be an unfair trap.
    #[must_use]
    pub fn new(target: &str) -> Self {
        let target: Vec<char> = target.trim().chars().collect();
        let words = word_spans(&target);
        Matcher {
            target,
            words,
            typed: Vec::new(),
        }
    }

    /// Append one typed character.
    pub fn push(&mut self, c: char) {
        self.typed.push(c);
    }

    /// Remove the last typed character (backspace). Returns it, or `None` if
    /// nothing has been typed.
    pub fn pop(&mut self) -> Option<char> {
        self.typed.pop()
    }

    /// Classify everything typed so far.
    ///
    /// Returns the character outcomes, the outcomes of committed words, and
    /// whether the text has been finished correctly: the end of the target
    /// reached with the final outcome correct. That flag is what ends a session
    /// early. Reaching the end with a wrong final character does not, so the
    /// user can still correct it.
    ///
    /// A single pass, O(target + typed), recomputed on every call.
    #[must_use]
    pub fn evaluate(&self) -> (Vec<CharOutcome>, Vec<WordOutcome>, bool) {
        let target = &self.target;
        let n = target.len();
        let mut outcomes = Vec::with_capacity(self.typed.len());
        // Whether each word has had no error attributed to it so far.
        let mut word_clean = vec![true; self.words.len()];
        // Target cursor.
        let mut ti = 0;
        // Index of the first word ending after `ti`. It only moves forward,
        // which keeps the whole pass linear.
        let mut wi = 0;
        // The word the most recent Correct/Wrong/Missing belonged to. Extra
        // characters are charged to it.
        let mut active_word: Option<usize> = None;

        for &c in &self.typed {
            while wi < self.words.len() && self.words[wi].1 <= ti {
                wi += 1;
            }
            // The word the cursor is inside, unless it sits on a separator or
            // past the end.
            let current = (wi < self.words.len() && self.words[wi].0 <= ti).then_some(wi);
            // The same word, but only when part of it has already been typed.
            let mid_word = current.filter(|&w| self.words[w].0 < ti);

            if c.is_whitespace() {
                if ti < n && target[ti].is_whitespace() {
                    // The expected separator.
                    outcomes.push(CharOutcome::Correct(c));
                    ti += 1;
                } else if let Some(w) = mid_word {
                    // Space in the middle of a word: the rest of the word is
                    // missing, and the space realigns to the separator after it.
                    let end = self.words[w].1;
                    outcomes.extend(target[ti..end].iter().copied().map(CharOutcome::Missing));
                    word_clean[w] = false;
                    active_word = Some(w);
                    ti = end;
                    if ti < n {
                        outcomes.push(CharOutcome::Correct(c));
                        ti += 1;
                    } else {
                        // The skipped word was the last one: no separator follows.
                        outcomes.push(CharOutcome::Extra(c));
                    }
                } else {
                    // A stray space at the start of a word (a double space), or
                    // past the end. Deliberately not a word skip: the next word is
                    // still there to be typed.
                    outcomes.push(CharOutcome::Extra(c));
                    if let Some(w) = active_word {
                        word_clean[w] = false;
                    }
                }
            } else if let Some(w) = current {
                let expected = target[ti];
                if c == expected {
                    outcomes.push(CharOutcome::Correct(c));
                } else {
                    outcomes.push(CharOutcome::Wrong { expected, typed: c });
                    word_clean[w] = false;
                }
                active_word = Some(w);
                ti += 1;
            } else {
                // Past the end of the word or of the text. It takes no target
                // slot, so the words after it stay aligned.
                outcomes.push(CharOutcome::Extra(c));
                if let Some(w) = active_word {
                    word_clean[w] = false;
                }
            }
        }

        // A word is committed once the cursor has passed its separator or, for
        // the last word, reached the end of the text. The cursor only moves
        // forward, so committed words always form a prefix.
        let last_word = self.words.len().checked_sub(1);
        let word_outcomes = self
            .words
            .iter()
            .enumerate()
            .take_while(|&(w, &(_, end))| ti > end || (Some(w) == last_word && ti == n))
            .map(|(w, _)| {
                if word_clean[w] {
                    WordOutcome::Correct
                } else {
                    WordOutcome::Incorrect
                }
            })
            .collect();

        let finished_correctly =
            n > 0 && ti == n && matches!(outcomes.last(), Some(CharOutcome::Correct(_)));

        (outcomes, word_outcomes, finished_correctly)
    }
}

/// `[start, end)` spans of the whitespace-separated words in `target`.
fn word_spans(target: &[char]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (i, c) in target.iter().enumerate() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                spans.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        spans.push((s, target.len()));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::{CharOutcome as C, Matcher, WordOutcome as W};

    /// Evaluate `typed` against `target`.
    fn eval(target: &str, typed: &str) -> (Vec<C>, Vec<W>, bool) {
        let mut matcher = Matcher::new(target);
        for c in typed.chars() {
            matcher.push(c);
        }
        matcher.evaluate()
    }

    #[test]
    fn exact_match_is_all_correct_and_finished() {
        let (outcomes, words, finished) = eval("cat", "cat");
        assert_eq!(outcomes, [C::Correct('c'), C::Correct('a'), C::Correct('t')]);
        assert_eq!(words, [W::Correct]);
        assert!(finished);
    }

    #[test]
    fn one_wrong_character_is_wrong_and_the_rest_stay_correct() {
        let (outcomes, words, _) = eval("cat", "cot");
        assert_eq!(
            outcomes,
            [
                C::Correct('c'),
                C::Wrong { expected: 'a', typed: 'o' },
                C::Correct('t')
            ]
        );
        assert_eq!(words, [W::Incorrect]);
    }

    #[test]
    fn space_mid_word_marks_the_rest_of_the_word_missing() {
        let (outcomes, words, finished) = eval("the cat sat", "th cat");
        assert_eq!(
            outcomes,
            [
                C::Correct('t'),
                C::Correct('h'),
                C::Missing('e'),
                C::Correct(' '),
                C::Correct('c'),
                C::Correct('a'),
                C::Correct('t')
            ]
        );
        assert_eq!(words, [W::Incorrect], "\"cat\" is still in progress");
        assert!(!finished);
    }

    #[test]
    fn space_mid_final_word_is_missing_then_extra() {
        let (outcomes, words, finished) = eval("cat", "ca ");
        assert_eq!(
            outcomes,
            [C::Correct('c'), C::Correct('a'), C::Missing('t'), C::Extra(' ')]
        );
        assert_eq!(words, [W::Incorrect]);
        assert!(!finished);
    }

    #[test]
    fn characters_past_the_end_of_the_text_are_extra() {
        let (outcomes, words, finished) = eval("cat", "cats");
        assert_eq!(
            outcomes,
            [C::Correct('c'), C::Correct('a'), C::Correct('t'), C::Extra('s')]
        );
        assert_eq!(words, [W::Incorrect]);
        assert!(!finished);
    }

    #[test]
    fn a_wrong_character_does_not_cascade() {
        let (outcomes, words, _) = eval("the cat", "tge cat");
        assert_eq!(
            outcomes,
            [
                C::Correct('t'),
                C::Wrong { expected: 'h', typed: 'g' },
                C::Correct('e'),
                C::Correct(' '),
                C::Correct('c'),
                C::Correct('a'),
                C::Correct('t')
            ]
        );
        assert_eq!(words, [W::Incorrect, W::Correct]);
    }

    #[test]
    fn an_overrun_word_does_not_shift_the_next_word() {
        let (outcomes, words, _) = eval("the cat", "thee cat");
        assert_eq!(
            outcomes,
            [
                C::Correct('t'),
                C::Correct('h'),
                C::Correct('e'),
                C::Extra('e'),
                C::Correct(' '),
                C::Correct('c'),
                C::Correct('a'),
                C::Correct('t')
            ]
        );
        assert_eq!(words, [W::Incorrect, W::Correct]);
    }

    #[test]
    fn nothing_typed_yields_no_outcomes() {
        let (outcomes, words, finished) = eval("cat", "");
        assert!(outcomes.is_empty());
        assert!(words.is_empty());
        assert!(!finished);
    }

    #[test]
    fn an_empty_target_never_finishes() {
        let (outcomes, words, finished) = eval("   ", "x");
        assert_eq!(outcomes, [C::Extra('x')]);
        assert!(words.is_empty());
        assert!(!finished);
    }

    #[test]
    fn a_double_space_is_extra_and_does_not_skip_the_next_word() {
        let (outcomes, words, finished) = eval("cat sat", "cat  sat");
        assert_eq!(
            outcomes,
            [
                C::Correct('c'),
                C::Correct('a'),
                C::Correct('t'),
                C::Correct(' '),
                C::Extra(' '),
                C::Correct('s'),
                C::Correct('a'),
                C::Correct('t')
            ]
        );
        assert_eq!(words, [W::Incorrect, W::Correct]);
        assert!(finished);
    }

    #[test]
    fn a_leading_space_is_extra_and_charged_to_no_word() {
        let (outcomes, words, finished) = eval("cat", " cat");
        assert_eq!(outcomes[0], C::Extra(' '));
        assert_eq!(words, [W::Correct]);
        assert!(finished);
    }

    #[test]
    fn words_are_committed_only_after_their_separator() {
        assert!(eval("cat sat", "cat").1.is_empty());
        assert_eq!(eval("cat sat", "cat ").1, [W::Correct]);
        assert_eq!(eval("cat sat", "cat sa").1, [W::Correct]);
        assert_eq!(eval("cat sat", "cat sat").1, [W::Correct, W::Correct]);
        assert_eq!(eval("cat sat", "cat sax").1, [W::Correct, W::Incorrect]);
    }

    #[test]
    fn finishing_requires_the_end_reached_with_a_correct_final_character() {
        assert!(eval("cat", "cat").2);
        assert!(eval("cat", "cxt").2, "an earlier error does not block finishing");
        assert!(!eval("cat", "ca").2, "not at the end yet");
        assert!(!eval("cat", "cax").2, "wrong final character");
        assert!(!eval("cat", "cats").2, "typed past the end");
    }

    #[test]
    fn surrounding_whitespace_in_the_target_is_trimmed() {
        assert!(eval("  cat \n", "cat").2);
    }

    #[test]
    fn pop_is_backspace() {
        let mut matcher = Matcher::new("cat");
        assert_eq!(matcher.pop(), None);
        matcher.push('c');
        matcher.push('x');
        assert_eq!(matcher.pop(), Some('x'));
        matcher.push('a');
        matcher.push('t');
        let (outcomes, _, finished) = matcher.evaluate();
        assert_eq!(outcomes, [C::Correct('c'), C::Correct('a'), C::Correct('t')]);
        assert!(finished);
    }

    #[test]
    fn only_correct_is_not_an_error() {
        assert!(!C::Correct('a').is_error());
        assert!(C::Wrong { expected: 'a', typed: 'b' }.is_error());
        assert!(C::Missing('a').is_error());
        assert!(C::Extra('a').is_error());
    }
}
