//! Word-aligned unit matching.
//!
//! Compares what the user has typed against the target text and classifies
//! every unit reached so far. It is the single source of truth for both
//! display granularities in `PRD.md` Section 8.3: KEY mode renders
//! [`Outcome`]s, WORD mode renders [`WordOutcome`]s, and accuracy and error
//! metrics always come from the unit level, whichever mode is on screen.
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
//!
//! A layout that reorders or completes text (a reph struck after its
//! syllable, a half form completed by the next key) instead hands the matcher
//! a [`TextEdit`] through [`Matcher::apply`]: delete code points from the end,
//! then append. Some legitimate intermediate states are not prefixes of the
//! target unit (`य` on the way to `र्य`); an optional [`Composition`] hook,
//! implemented by the layout crate, tells the matcher to keep such a last
//! unit pending instead of scoring it. Without a hook nothing changes.
//!
//! # One algorithm, two units
//!
//! The matcher is generic over [`TextUnit`]: `char` for QWERTY and legacy
//! font streams, [`crate::akshar::Akshar`] for Unicode Hindi (RULES.MD
//! Rule 35). The typed buffer is kept as text and re-segmented on every
//! evaluation, so a cluster built over several keys is always judged as the
//! whole cluster. While the *last* typed unit is a strict prefix of the
//! expected one (क typed toward क्ष), it gets no outcome yet: the typist is
//! mid-cluster, not wrong. For `char` that rule never fires, so QWERTY
//! behaviour is exactly the per-character matcher it always was.

use std::ops::Range;
use std::sync::Arc;

use crate::input::{Composition, TextEdit};
use crate::unit::TextUnit;

/// How one typed or skipped unit compares with the target.
///
/// Exactly the four KEY-mode states from `PRD.md` Section 8.3. There is
/// deliberately no "not yet typed" variant: units the user has not reached
/// produce no outcome at all, so every outcome is something the user did (or
/// skipped), and accuracy is never diluted by untouched text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome<U> {
    /// The typed unit matched the target.
    Correct(U),
    /// A different unit was typed in this position.
    Wrong {
        /// What the target has here.
        expected: U,
        /// What was typed instead.
        typed: U,
    },
    /// A target unit skipped by pressing space in the middle of a word.
    Missing(U),
    /// A typed unit with no target slot: letters past the end of a word or of
    /// the text, or a stray space at the start of a word.
    Extra(U),
}

/// The per-character outcome, the original (QWERTY) form of [`Outcome`].
pub type CharOutcome = Outcome<char>;

impl<U> Outcome<U> {
    /// Whether this outcome counts as an error: anything but [`Outcome::Correct`].
    #[must_use]
    pub fn is_error(&self) -> bool {
        !matches!(self, Outcome::Correct(_))
    }

    /// The unit the user actually produced, or `None` for [`Outcome::Missing`]
    /// (nothing was typed there).
    #[must_use]
    pub fn typed_unit(&self) -> Option<&U> {
        match self {
            Outcome::Correct(u) | Outcome::Extra(u) => Some(u),
            Outcome::Wrong { typed, .. } => Some(typed),
            Outcome::Missing(_) => None,
        }
    }

    /// The target unit this outcome was judged against, or `None` for
    /// [`Outcome::Extra`] (it took no target slot).
    #[must_use]
    pub fn expected_unit(&self) -> Option<&U> {
        match self {
            Outcome::Correct(u) | Outcome::Missing(u) => Some(u),
            Outcome::Wrong { expected, .. } => Some(expected),
            Outcome::Extra(_) => None,
        }
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
    /// Every unit of the word was correct, with nothing extra.
    Correct,
    /// The word carries at least one wrong, missing or extra unit.
    Incorrect,
}

/// Everything one evaluation knows, for callers that need more than the
/// [`Matcher::evaluate`] triple (display states, word classification, cursor).
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation<U> {
    /// One outcome per unit reached, in typing order.
    pub outcomes: Vec<Outcome<U>>,
    /// One outcome per committed word.
    pub word_outcomes: Vec<WordOutcome>,
    /// The end of the target reached with a correct final unit.
    pub finished: bool,
    /// Target unit index the next typed unit will be compared with.
    pub cursor: usize,
    /// Index of the word being typed: the first word not yet committed.
    /// `None` once every word is committed. Space, not finishing a word's
    /// letters, moves it on, so the word stays highlighted until committed.
    pub current_word: Option<usize>,
    /// The last typed unit is a strict prefix of its target unit, or a state
    /// the [`Composition`] hook declares intermediate, and has no outcome yet
    /// (a Hindi cluster mid-build).
    pub pending: bool,
    /// `[start, end)` target-unit span of every word.
    pub word_spans: Vec<Range<usize>>,
    /// For every target unit, the outcome that consumed it (`Correct`,
    /// `Wrong` or `Missing`), or `None` while untouched.
    pub target_outcome: Vec<Option<usize>>,
    /// For every word, the outcome indices charged to it (its own units and
    /// any extras after it; the separator after it is not charged).
    pub word_charges: Vec<Vec<usize>>,
}

/// The target text and the typed buffer, classified against each other on
/// demand.
#[derive(Debug, Clone)]
pub struct Matcher<U: TextUnit = char> {
    /// The target, trimmed, as units.
    target: Vec<U>,
    /// `[start, end)` span of each target word, precomputed once because every
    /// evaluation needs them.
    words: Vec<(usize, usize)>,
    /// What the user has typed so far, as text. Kept as text, not units, so a
    /// multi-key Hindi cluster is re-segmented as a whole.
    typed: String,
    /// Optional layout hook for non-prefix intermediate states.
    composition: Option<Arc<dyn Composition<U>>>,
}

impl Matcher<char> {
    /// Create a per-character matcher for `target`.
    ///
    /// Leading and trailing whitespace is trimmed: it is invisible on screen,
    /// so requiring the user to type it would be an unfair trap.
    #[must_use]
    pub fn new(target: &str) -> Self {
        Self::from_target(target)
    }
}

impl<U: TextUnit> Matcher<U> {
    /// Create a matcher for `target` over unit type `U` (trimmed, as
    /// [`Matcher::new`]).
    #[must_use]
    pub fn from_target(target: &str) -> Self {
        let target: Vec<U> = U::segment(target.trim());
        let words = word_spans(&target);
        Matcher {
            target,
            words,
            typed: String::new(),
            composition: None,
        }
    }

    /// Attach a layout's [`Composition`] hook.
    #[must_use]
    pub fn with_composition(mut self, composition: Arc<dyn Composition<U>>) -> Self {
        self.composition = Some(composition);
        self
    }

    /// Apply a layout's tail edit. Returns the code points actually deleted
    /// (the deletion is clamped to what was typed).
    pub fn apply(&mut self, edit: &TextEdit) -> usize {
        edit.apply_to(&mut self.typed)
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

    /// Exactly what was typed, extra letters included and nothing invented.
    #[must_use]
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// The target as units.
    #[must_use]
    pub fn target_units(&self) -> &[U] {
        &self.target
    }

    /// Classify everything typed so far.
    ///
    /// Returns the unit outcomes, the outcomes of committed words, and
    /// whether the text has been finished correctly: the end of the target
    /// reached with the final outcome correct. That flag is what ends a session
    /// early. Reaching the end with a wrong final unit does not, so the user
    /// can still correct it.
    ///
    /// A single pass, O(target + typed), recomputed on every call.
    #[must_use]
    pub fn evaluate(&self) -> (Vec<Outcome<U>>, Vec<WordOutcome>, bool) {
        let e = self.evaluate_detailed();
        (e.outcomes, e.word_outcomes, e.finished)
    }

    /// [`Matcher::evaluate`] plus cursor, word and per-unit detail.
    #[must_use]
    pub fn evaluate_detailed(&self) -> Evaluation<U> {
        let typed_units = U::segment(&self.typed);
        let target = &self.target;
        let n = target.len();
        let words = &self.words;
        let mut outcomes: Vec<Outcome<U>> = Vec::with_capacity(typed_units.len());
        // Whether each word has had no error attributed to it so far.
        let mut word_clean = vec![true; words.len()];
        let mut word_charges: Vec<Vec<usize>> = vec![Vec::new(); words.len()];
        let mut target_outcome: Vec<Option<usize>> = vec![None; n];
        // Target cursor.
        let mut ti = 0;
        // Index of the first word ending after `ti`. It only moves forward,
        // which keeps the whole pass linear.
        let mut wi = 0;
        // The word the most recent Correct/Wrong/Missing belonged to. Extra
        // units are charged to it.
        let mut active_word: Option<usize> = None;
        let mut pending = false;
        let last_typed = typed_units.len().checked_sub(1);

        for (k, u) in typed_units.iter().enumerate() {
            while wi < words.len() && words[wi].1 <= ti {
                wi += 1;
            }
            // The word the cursor is inside, unless it sits on a separator or
            // past the end.
            let current = (wi < words.len() && words[wi].0 <= ti).then_some(wi);
            // The same word, but only when part of it has already been typed.
            let mid_word = current.filter(|&w| words[w].0 < ti);

            if u.is_whitespace() {
                if ti < n && target[ti].is_whitespace() {
                    // The expected separator.
                    target_outcome[ti] = Some(outcomes.len());
                    outcomes.push(Outcome::Correct(u.clone()));
                    ti += 1;
                } else if let Some(w) = mid_word {
                    // Space in the middle of a word: the rest of the word is
                    // missing, and the space realigns to the separator after it.
                    let end = words[w].1;
                    for slot in target_outcome.iter_mut().take(end).skip(ti) {
                        *slot = Some(outcomes.len());
                        word_charges[w].push(outcomes.len());
                        outcomes.push(Outcome::Missing(target[ti].clone()));
                        ti += 1;
                    }
                    word_clean[w] = false;
                    active_word = Some(w);
                    if ti < n {
                        target_outcome[ti] = Some(outcomes.len());
                        outcomes.push(Outcome::Correct(u.clone()));
                        ti += 1;
                    } else {
                        // The skipped word was the last one: no separator follows.
                        word_charges[w].push(outcomes.len());
                        outcomes.push(Outcome::Extra(u.clone()));
                    }
                } else {
                    // A stray space at the start of a word (a double space), or
                    // past the end. Deliberately not a word skip: the next word is
                    // still there to be typed.
                    if let Some(w) = active_word {
                        word_clean[w] = false;
                        word_charges[w].push(outcomes.len());
                    }
                    outcomes.push(Outcome::Extra(u.clone()));
                }
            } else if let Some(w) = current {
                let expected = &target[ti];
                if u == expected {
                    target_outcome[ti] = Some(outcomes.len());
                    word_charges[w].push(outcomes.len());
                    outcomes.push(Outcome::Correct(u.clone()));
                } else if Some(k) == last_typed
                    && (u.is_proper_prefix_of(expected)
                        || self
                            .composition
                            .as_ref()
                            .is_some_and(|c| c.is_intermediate(u, expected)))
                {
                    // Mid-cluster: judged once the cluster is complete.
                    pending = true;
                    break;
                } else {
                    target_outcome[ti] = Some(outcomes.len());
                    word_charges[w].push(outcomes.len());
                    outcomes.push(Outcome::Wrong {
                        expected: expected.clone(),
                        typed: u.clone(),
                    });
                    word_clean[w] = false;
                }
                active_word = Some(w);
                ti += 1;
            } else {
                // Past the end of the word or of the text. It takes no target
                // slot, so the words after it stay aligned.
                if let Some(w) = active_word {
                    word_clean[w] = false;
                    word_charges[w].push(outcomes.len());
                }
                outcomes.push(Outcome::Extra(u.clone()));
            }
        }

        // A word is committed once the cursor has passed its separator or, for
        // the last word, reached the end of the text. The cursor only moves
        // forward, so committed words always form a prefix.
        let last_word = words.len().checked_sub(1);
        let word_outcomes: Vec<WordOutcome> = words
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

        let finished = n > 0 && ti == n && matches!(outcomes.last(), Some(Outcome::Correct(_)));
        let current_word = (word_outcomes.len() < words.len()).then_some(word_outcomes.len());

        Evaluation {
            outcomes,
            word_outcomes,
            finished,
            cursor: ti,
            current_word,
            pending,
            word_spans: words.iter().map(|&(s, e)| s..e).collect(),
            target_outcome,
            word_charges,
        }
    }
}

/// `[start, end)` spans of the whitespace-separated words in `target`.
fn word_spans<U: TextUnit>(target: &[U]) -> Vec<(usize, usize)> {
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
        assert_eq!(
            outcomes,
            [C::Correct('c'), C::Correct('a'), C::Correct('t')]
        );
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
                C::Wrong {
                    expected: 'a',
                    typed: 'o'
                },
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
            [
                C::Correct('c'),
                C::Correct('a'),
                C::Missing('t'),
                C::Extra(' ')
            ]
        );
        assert_eq!(words, [W::Incorrect]);
        assert!(!finished);
    }

    #[test]
    fn characters_past_the_end_of_the_text_are_extra() {
        let (outcomes, words, finished) = eval("cat", "cats");
        assert_eq!(
            outcomes,
            [
                C::Correct('c'),
                C::Correct('a'),
                C::Correct('t'),
                C::Extra('s')
            ]
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
                C::Wrong {
                    expected: 'h',
                    typed: 'g'
                },
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
        assert!(
            eval("cat", "cxt").2,
            "an earlier error does not block finishing"
        );
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
        assert_eq!(
            outcomes,
            [C::Correct('c'), C::Correct('a'), C::Correct('t')]
        );
        assert!(finished);
    }

    #[test]
    fn only_correct_is_not_an_error() {
        assert!(!C::Correct('a').is_error());
        assert!(C::Wrong {
            expected: 'a',
            typed: 'b'
        }
        .is_error());
        assert!(C::Missing('a').is_error());
        assert!(C::Extra('a').is_error());
    }

    // ---- P02 additions: detail, cursor, akshar units ----------------------

    use super::Outcome;
    use crate::akshar::Akshar;

    fn akshar_eval(target: &str, typed: &str) -> super::Evaluation<Akshar> {
        let mut matcher = Matcher::<Akshar>::from_target(target);
        for c in typed.chars() {
            matcher.push(c);
        }
        matcher.evaluate_detailed()
    }

    #[test]
    fn the_current_word_moves_on_space_not_on_the_last_letter() {
        let mut m = Matcher::new("the dog");
        for c in "the".chars() {
            m.push(c);
        }
        let e = m.evaluate_detailed();
        assert_eq!(e.cursor, 3);
        assert_eq!(e.current_word, Some(0));
        m.push(' ');
        assert_eq!(m.evaluate_detailed().current_word, Some(1));
        assert_eq!(m.typed(), "the ");
    }

    #[test]
    fn a_correct_hindi_word_is_one_outcome_per_akshar() {
        let e = akshar_eval("क्षेत्र", "क्षेत्र");
        assert_eq!(e.outcomes.len(), 2);
        assert!(e.finished);
        assert!(e.outcomes.iter().all(|o| !o.is_error()));
    }

    #[test]
    fn a_one_akshar_typo_is_exactly_one_error() {
        // क्षेत्र typed with त्र replaced by त्रि.
        let e = akshar_eval("क्षेत्र नया", "क्षेत्रि नया");
        assert_eq!(e.outcomes.iter().filter(|o| o.is_error()).count(), 1);
    }

    #[test]
    fn a_cluster_being_built_is_pending_not_wrong() {
        for typed in ["क", "क्", "क्ष"] {
            let e = akshar_eval("क्षे", typed);
            assert!(e.pending, "{typed} is mid-cluster");
            assert!(e.outcomes.is_empty());
        }
        let e = akshar_eval("क्षे", "क्षे");
        assert!(!e.pending);
        assert!(e.finished);
    }

    #[test]
    fn a_wrong_cluster_is_wrong_even_if_it_starts_like_the_target() {
        let e = akshar_eval("कि", "कु");
        assert!(matches!(e.outcomes[..], [Outcome::Wrong { .. }]));
    }

    // ---- P03 additions: tail edits and the composition hook ---------------

    use crate::input::{Composition, TextEdit};
    use std::sync::Arc;

    /// A fake layout oracle: `य` is a legitimate state toward `र्य`.
    #[derive(Debug)]
    struct RephLast;

    impl Composition<Akshar> for RephLast {
        fn is_intermediate(&self, typed: &Akshar, expected: &Akshar) -> bool {
            typed.as_str() == "य" && expected.as_str() == "र्य"
        }
    }

    #[test]
    fn apply_replaces_the_tail_and_clamps() {
        let mut m = Matcher::new("ab");
        m.push('a');
        assert_eq!(m.apply(&TextEdit::replace(3, "b")), 1);
        assert_eq!(m.typed(), "b");
    }

    #[test]
    fn the_hook_keeps_a_non_prefix_intermediate_pending() {
        let mut plain = Matcher::<Akshar>::from_target("र्य");
        plain.apply(&TextEdit::insert("य"));
        assert!(matches!(
            plain.evaluate_detailed().outcomes[..],
            [Outcome::Wrong { .. }]
        ));

        let mut hooked = Matcher::<Akshar>::from_target("र्य").with_composition(Arc::new(RephLast));
        hooked.apply(&TextEdit::insert("य"));
        let e = hooked.evaluate_detailed();
        assert!(e.pending);
        assert!(e.outcomes.is_empty());
        hooked.apply(&TextEdit::replace(1, "र्य"));
        assert!(hooked.evaluate_detailed().finished);
    }

    #[test]
    fn the_hook_only_applies_to_the_last_unit() {
        let mut m = Matcher::<Akshar>::from_target("र्य क").with_composition(Arc::new(RephLast));
        m.apply(&TextEdit::insert("य क"));
        let e = m.evaluate_detailed();
        assert!(matches!(e.outcomes[0], Outcome::Wrong { .. }));
    }

    #[test]
    fn nfc_makes_both_nukta_spellings_correct() {
        assert!(akshar_eval("\u{0958}", "\u{0915}\u{093C}").finished);
        assert!(akshar_eval("\u{0915}\u{093C}", "\u{0958}").finished);
    }
}
