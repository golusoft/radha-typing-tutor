//! Typing session lifecycle and live metrics.
//!
//! [`SessionState`] is the lifecycle. [`TypingSession`] drives it and turns the
//! matcher's outcomes into the numbers the typing screen shows.
//!
//! Everything lives in RAM. A session is handed to the persistence layer once,
//! on completion, which makes `RULES.MD` Rule 14 (no per-keystroke database
//! writes) structural rather than merely intended: nothing in this module can
//! write to a database, even if a caller wanted it to.
//!
//! # The clock seam
//!
//! Every public method has a `*_at` twin taking an explicit [`Instant`]. The
//! plain methods pass `Instant::now()`; tests and fixtures pass fixed instants,
//! so timing (including the 599.9 s vs 600 s countdown boundary) is checked
//! exactly without sleeping. Passing the instant *is* the injectable clock: no
//! trait object, no type parameter, nothing to fake.
//!
//! # Time-up
//!
//! A countdown is enforced lazily: every `*_at` call first checks the deadline
//! for the instant it was given, and [`TypingSession::tick_at`] lets a UI timer
//! end the session when no key is pressed. A session that expires is
//! completed *at the deadline* (not at the late instant that noticed it), so
//! elapsed time never exceeds the limit and a keystroke at or after the
//! deadline is ignored.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::error::{CoreError, Result};
use crate::input::{Composition, TextEdit};
use crate::matcher::{Evaluation, Matcher, Outcome, WordOutcome};
use crate::mistake::MistakeCounts;
use crate::scoring::{ErrorBreakdown, Score};
use crate::stats::KeyStats;
use crate::timer::{self, TimerMode};
use crate::unit::TextUnit;

/// Lifecycle of a typing session.
///
/// Only a completed session is eligible for persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Created but the user has not pressed the first key yet.
    Idle,
    /// Timer running, accepting keystrokes.
    Active,
    /// Timer paused by the user; keystrokes ignored.
    Paused,
    /// Finished — either the timer elapsed or the exercise was fully typed.
    /// This is the only state from which a session may be written to SQLite.
    Complete,
}

impl SessionState {
    /// Whether a session in this state may be flushed to the database.
    ///
    /// The persistence layer is expected to gate on this so that partial
    /// sessions can never be half-written.
    #[must_use]
    pub fn is_persistable(self) -> bool {
        matches!(self, SessionState::Complete)
    }

    /// Whether keystrokes should currently be processed.
    #[must_use]
    pub fn accepts_input(self) -> bool {
        matches!(self, SessionState::Active)
    }
}

/// Why a session completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionReason {
    /// The text was finished with a correct final unit.
    Finished,
    /// The countdown reached zero.
    TimeUp,
    /// The caller ended it ([`TypingSession::complete`]).
    Submitted,
}

/// How a session is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionConfig {
    /// Timer mode. Default: stopwatch (no limit).
    pub timer: TimerMode,
}

/// One typing attempt at one target text.
///
/// Owns the matcher and the clock. Driven by input events and polled for
/// [`SessionSnapshot`]s; it holds no database handle.
#[derive(Debug, Clone)]
pub struct TypingSession<U: TextUnit = char> {
    matcher: Matcher<U>,
    config: SessionConfig,
    state: SessionState,
    completion: Option<CompletionReason>,
    /// The first keystroke. `None` until typing starts, so reading time before
    /// the first key is never charged to the user's speed.
    started_at: Option<Instant>,
    /// When the current pause began; `None` unless paused.
    paused_at: Option<Instant>,
    /// Total time spent in completed pauses.
    paused_total: Duration,
    /// When the session ended; freezes the elapsed time.
    completed_at: Option<Instant>,
    /// Backspace presses. Corrected errors vanish from the outcomes, so this is
    /// the only lasting trace of a correction.
    backspaces: u32,
    /// Accepted presses: characters and effective backspaces.
    presses: u32,
    /// Active (pause-excluded) elapsed time of every accepted press, for the
    /// consistency intervals. RAM only.
    input_times: Vec<Duration>,
    /// Per-key tallies, updated as outcomes appear. Unlike the outcomes, they
    /// keep an error that was later corrected: the key was still missed.
    key_stats: KeyStats<U>,
    /// Outcome count and last outcome after the previous accepted input, so
    /// only new (or revised) outcomes are recorded in `key_stats`.
    seen_len: usize,
    seen_last: Option<Outcome<U>>,
}

/// The live metrics of a session at one moment.
///
/// A plain value, recomputed from scratch by [`TypingSession::snapshot`] on
/// every call, so what the screen shows can never be stale relative to the
/// typed text. The completed session's snapshot *is* the result: the same
/// [`Score::compute`] builds both (RULES.MD Rule 36).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot<U = char> {
    /// Lifecycle state.
    pub state: SessionState,
    /// Typing time in seconds, excluding pauses.
    pub elapsed_seconds: f64,
    /// Gross words per minute.
    pub wpm: f64,
    /// Net words per minute.
    pub net_wpm: f64,
    /// Character accuracy, as a percentage.
    pub accuracy: f64,
    /// Wrong + missing + extra characters.
    pub error_count: usize,
    /// Backspace presses.
    pub backspaces: u32,
    /// One outcome per character reached, for KEY mode.
    pub outcomes: Vec<Outcome<U>>,
    /// One outcome per committed word, for WORD mode.
    pub word_outcomes: Vec<WordOutcome>,
    /// Correct committed words, as a percentage.
    pub word_accuracy: f64,
    /// Key depressions per hour.
    pub kdph: f64,
    /// Rhythm consistency, 0–100.
    pub consistency: f64,
    /// Code points typed (the WPM/KDPH numerator).
    pub characters_typed: usize,
    /// Errors split by kind (wrong / extra / missing / space).
    pub breakdown: ErrorBreakdown,
    /// Half/full classification counts (reported, not applied to net WPM).
    pub mistakes: MistakeCounts,
    /// Accepted presses (characters + effective backspaces).
    pub presses: u32,
    /// Target unit index the next unit is compared with.
    pub cursor: usize,
    /// Index of the word being typed, `None` once all are committed.
    pub current_word: Option<usize>,
    /// Countdown time left, in seconds; `None` without a countdown.
    pub remaining_seconds: Option<f64>,
    /// The timer mode.
    pub timer: TimerMode,
    /// Why the session completed, once it has.
    pub completion: Option<CompletionReason>,
}

impl TypingSession<char> {
    /// Start an idle per-character session for `target` with the default
    /// configuration (stopwatch). The clock starts on the first key.
    #[must_use]
    pub fn new(target: &str) -> Self {
        Self::configured(target, SessionConfig::default())
    }
}

impl<U: TextUnit> TypingSession<U> {
    /// Start an idle session for `target` over unit type `U`.
    #[must_use]
    pub fn configured(target: &str, config: SessionConfig) -> Self {
        TypingSession {
            matcher: Matcher::from_target(target),
            config,
            state: SessionState::Idle,
            completion: None,
            started_at: None,
            paused_at: None,
            paused_total: Duration::ZERO,
            completed_at: None,
            backspaces: 0,
            presses: 0,
            input_times: Vec::new(),
            key_stats: KeyStats::new(),
            seen_len: 0,
            seen_last: None,
        }
    }

    /// Current lifecycle state.
    #[must_use]
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// The configuration the session was started with.
    #[must_use]
    pub fn config(&self) -> SessionConfig {
        self.config
    }

    /// Per-key statistics so far (RAM only).
    #[must_use]
    pub fn key_stats(&self) -> &KeyStats<U> {
        &self.key_stats
    }

    /// The full evaluation of what is typed (display states, word detail).
    #[must_use]
    pub fn evaluation(&self) -> Evaluation<U> {
        self.matcher.evaluate_detailed()
    }

    /// The matcher, for read-only access to the typed text and target units.
    #[must_use]
    pub fn matcher(&self) -> &Matcher<U> {
        &self.matcher
    }

    /// Type one character.
    ///
    /// The first character starts the session and its clock. Ignored while
    /// paused or complete. Completes the session automatically when the text is
    /// finished correctly (see [`Matcher::evaluate`]).
    pub fn type_char(&mut self, c: char) {
        self.type_char_at(c, Instant::now());
    }

    /// Remove the last typed character. Ignored when nothing is typed, or while
    /// paused or complete.
    pub fn backspace(&mut self) {
        self.backspace_at(Instant::now());
    }

    /// Pause the clock.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidSessionState`] unless the session is active.
    pub fn pause(&mut self) -> Result<()> {
        self.pause_at(Instant::now())
    }

    /// Resume after a pause.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidSessionState`] unless the session is paused.
    pub fn resume(&mut self) -> Result<()> {
        self.resume_at(Instant::now())
    }

    /// End the session, for example when the user submits.
    ///
    /// Infallible and idempotent: the timer and a final keystroke may both try
    /// to end the session, and ending it twice must be harmless. Completing a
    /// paused session freezes its time at the pause.
    pub fn complete(&mut self) {
        self.complete_at(Instant::now());
    }

    /// Let the countdown end the session if its time is up. Call from the UI
    /// timer. Returns `true` when this call completed the session.
    pub fn tick(&mut self) -> bool {
        self.tick_at(Instant::now())
    }

    /// The live metrics right now.
    #[must_use]
    pub fn snapshot(&self) -> SessionSnapshot<U> {
        self.snapshot_at(Instant::now())
    }

    /// [`TypingSession::type_char`] at `now`.
    pub fn type_char_at(&mut self, c: char, now: Instant) {
        self.enforce_deadline(now);
        if self.state == SessionState::Idle {
            self.state = SessionState::Active;
            self.started_at = Some(now);
        }
        if !self.state.accepts_input() {
            return;
        }
        self.matcher.push(c);
        let latency = self.note_press(now);
        let evaluation = self.matcher.evaluate_detailed();
        self.record_new_outcomes(&evaluation.outcomes, latency);
        if evaluation.finished {
            self.finish(now, CompletionReason::Finished);
        }
    }

    /// Attach a layout's [`Composition`] hook to the matcher, so legitimate
    /// non-prefix intermediate states stay pending (see [`Matcher::apply`]).
    #[must_use]
    pub fn with_composition(mut self, composition: Arc<dyn Composition<U>>) -> Self {
        self.matcher = self.matcher.with_composition(composition);
        self
    }

    /// Apply one key press expressed as a [`TextEdit`].
    pub fn apply_edit(&mut self, edit: &TextEdit) {
        self.apply_edit_at(edit, Instant::now());
    }

    /// [`TypingSession::apply_edit`] at `now`.
    ///
    /// A non-backspace edit follows [`TypingSession::type_char_at`]: it starts
    /// an idle session, counts one press (even when it changes no text, such
    /// as a preedit-only key) and is ignored unless active. A backspace edit
    /// follows [`TypingSession::backspace_at`]: it never starts a session and
    /// counts one backspace when it removed something or when it only
    /// cancelled a preedit (`delete == 0`).
    pub fn apply_edit_at(&mut self, edit: &TextEdit, now: Instant) {
        self.enforce_deadline(now);
        if edit.backspace {
            if !self.state.accepts_input() {
                return;
            }
            let removed = self.matcher.apply(edit);
            if removed > 0 || edit.delete == 0 {
                self.backspaces = self.backspaces.saturating_add(1);
                self.note_press(now);
                let (outcomes, _, _) = self.matcher.evaluate();
                self.seen_len = outcomes.len();
                self.seen_last = outcomes.last().cloned();
            }
            return;
        }
        if self.state == SessionState::Idle {
            self.state = SessionState::Active;
            self.started_at = Some(now);
        }
        if !self.state.accepts_input() {
            return;
        }
        self.matcher.apply(edit);
        let latency = self.note_press(now);
        let evaluation = self.matcher.evaluate_detailed();
        self.record_new_outcomes(&evaluation.outcomes, latency);
        if evaluation.finished {
            self.finish(now, CompletionReason::Finished);
        }
    }

    /// [`TypingSession::backspace`] at `now`.
    pub fn backspace_at(&mut self, now: Instant) {
        self.enforce_deadline(now);
        if !self.state.accepts_input() {
            return;
        }
        if self.matcher.pop().is_some() {
            self.backspaces = self.backspaces.saturating_add(1);
            self.note_press(now);
            let (outcomes, _, _) = self.matcher.evaluate();
            self.seen_len = outcomes.len();
            self.seen_last = outcomes.last().cloned();
        }
    }

    /// [`TypingSession::pause`] at `now`.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidSessionState`] unless the session is active
    /// (including when its countdown already ran out at `now`).
    pub fn pause_at(&mut self, now: Instant) -> Result<()> {
        self.enforce_deadline(now);
        if self.state != SessionState::Active {
            return Err(CoreError::InvalidSessionState {
                detail: format!("cannot pause a session that is {:?}", self.state),
            });
        }
        self.state = SessionState::Paused;
        self.paused_at = Some(now);
        Ok(())
    }

    /// [`TypingSession::resume`] at `now`.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidSessionState`] unless the session is paused.
    pub fn resume_at(&mut self, now: Instant) -> Result<()> {
        if self.state != SessionState::Paused {
            return Err(CoreError::InvalidSessionState {
                detail: format!("cannot resume a session that is {:?}", self.state),
            });
        }
        if let Some(paused_at) = self.paused_at.take() {
            self.paused_total += now.saturating_duration_since(paused_at);
        }
        self.state = SessionState::Active;
        Ok(())
    }

    /// [`TypingSession::complete`] at `now`.
    pub fn complete_at(&mut self, now: Instant) {
        if self.enforce_deadline(now) {
            return;
        }
        self.finish(now, CompletionReason::Submitted);
    }

    /// [`TypingSession::tick`] at `now`.
    pub fn tick_at(&mut self, now: Instant) -> bool {
        self.enforce_deadline(now)
    }

    /// [`TypingSession::snapshot`] at `now`.
    #[must_use]
    pub fn snapshot_at(&self, now: Instant) -> SessionSnapshot<U> {
        let (mut snapshot, evaluation) = self.live_at(now);
        snapshot.outcomes = evaluation.outcomes;
        snapshot.word_outcomes = evaluation.word_outcomes;
        snapshot
    }

    /// The snapshot at `now` and the evaluation it was computed from, in a
    /// single matcher pass (a live screen needs both on every key). The
    /// snapshot's `outcomes` and `word_outcomes` are left empty: read them
    /// from the evaluation.
    #[must_use]
    pub fn live_at(&self, now: Instant) -> (SessionSnapshot<U>, Evaluation<U>) {
        let limit = self.config.timer.limit();
        let mut elapsed = self.elapsed_at(now);
        if let Some(limit) = limit {
            // A countdown nobody ticked yet still never reports more than its
            // limit.
            elapsed = elapsed.min(limit.as_duration());
        }
        let elapsed_seconds = elapsed.as_secs_f64();
        let evaluation = self.matcher.evaluate_detailed();
        let intervals = self.intervals();
        let score = Score::compute(
            &evaluation.outcomes,
            &evaluation.word_outcomes,
            elapsed_seconds,
            &intervals,
        );

        let snapshot = SessionSnapshot {
            state: self.state,
            elapsed_seconds,
            wpm: score.wpm,
            net_wpm: score.net_wpm,
            accuracy: score.accuracy,
            error_count: score.error_count,
            backspaces: self.backspaces,
            word_accuracy: score.word_accuracy,
            kdph: score.kdph,
            consistency: score.consistency,
            characters_typed: score.characters_typed,
            breakdown: score.breakdown,
            mistakes: MistakeCounts::from_evaluation(&evaluation),
            presses: self.presses,
            cursor: evaluation.cursor,
            current_word: evaluation.current_word,
            remaining_seconds: limit.map(|l| timer::remaining(l, elapsed).as_secs_f64()),
            timer: self.config.timer,
            completion: self.completion,
            outcomes: Vec::new(),
            word_outcomes: Vec::new(),
        };
        (snapshot, evaluation)
    }

    /// Complete with `reason` unless already complete.
    fn finish(&mut self, now: Instant, reason: CompletionReason) {
        if self.state == SessionState::Complete {
            return;
        }
        self.completed_at = if self.state == SessionState::Paused {
            self.paused_at
        } else {
            Some(now)
        };
        self.state = SessionState::Complete;
        self.completion = Some(reason);
    }

    /// End an active countdown whose time is up at `now`. Returns `true` when
    /// it did. The session is completed at the deadline itself, so a late
    /// check never adds time. Paused sessions never expire: their clock is
    /// stopped.
    fn enforce_deadline(&mut self, now: Instant) -> bool {
        let Some(limit) = self.config.timer.limit() else {
            return false;
        };
        if self.state != SessionState::Active || !timer::is_expired(limit, self.elapsed_at(now)) {
            return false;
        }
        let deadline = self
            .started_at
            .and_then(|s| s.checked_add(self.paused_total))
            .and_then(|s| s.checked_add(limit.as_duration()))
            .unwrap_or(now);
        self.completed_at = Some(deadline);
        self.state = SessionState::Complete;
        self.completion = Some(CompletionReason::TimeUp);
        true
    }

    /// Count an accepted press and log its active time. Returns the interval
    /// since the previous press, for key latency.
    fn note_press(&mut self, now: Instant) -> Option<Duration> {
        self.presses = self.presses.saturating_add(1);
        let t = self.elapsed_at(now);
        let latency = self.input_times.last().map(|&prev| t.saturating_sub(prev));
        self.input_times.push(t);
        latency
    }

    /// Record outcomes that appeared (or whose last unit was revised by a
    /// combining key) since the previous accepted input.
    fn record_new_outcomes(&mut self, outcomes: &[Outcome<U>], latency: Option<Duration>) {
        let mut from = self.seen_len.min(outcomes.len());
        if from > 0 && from == self.seen_len && outcomes.get(from - 1) != self.seen_last.as_ref() {
            from -= 1;
        }
        for o in &outcomes[from..] {
            self.key_stats.record(o, latency);
        }
        self.seen_len = outcomes.len();
        self.seen_last = outcomes.last().cloned();
    }

    /// Gaps between consecutive accepted presses on the active clock.
    fn intervals(&self) -> Vec<Duration> {
        self.input_times
            .windows(2)
            .map(|w| w[1].saturating_sub(w[0]))
            .collect()
    }

    /// Typing time: from the first key to completion, the current pause, or
    /// `now`, minus completed pauses.
    fn elapsed_at(&self, now: Instant) -> Duration {
        let Some(started_at) = self.started_at else {
            return Duration::ZERO;
        };
        let end = self.completed_at.or(self.paused_at).unwrap_or(now);
        end.saturating_duration_since(started_at)
            .saturating_sub(self.paused_total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `secs` seconds after `start`.
    fn at(start: Instant, secs: u64) -> Instant {
        start + Duration::from_secs(secs)
    }

    #[test]
    fn only_complete_sessions_are_persistable() {
        assert!(SessionState::Complete.is_persistable());
        assert!(!SessionState::Idle.is_persistable());
        assert!(!SessionState::Active.is_persistable());
        assert!(!SessionState::Paused.is_persistable());
    }

    #[test]
    fn only_active_sessions_accept_input() {
        assert!(SessionState::Active.accepts_input());
        assert!(!SessionState::Idle.accepts_input());
        assert!(!SessionState::Paused.accepts_input());
        assert!(!SessionState::Complete.accepts_input());
    }

    #[test]
    fn a_correct_session_reports_live_metrics_and_completes_on_its_own() {
        let t0 = Instant::now();
        let mut session = TypingSession::new("cats sleep");

        for c in "cats ".chars() {
            session.type_char_at(c, t0);
        }
        assert_eq!(session.state(), SessionState::Active);

        let mid = session.snapshot_at(at(t0, 60));
        assert_eq!(mid.state, SessionState::Active);
        assert_eq!(mid.elapsed_seconds, 60.0);
        assert_eq!(mid.wpm, 1.0, "5 characters in one minute");
        assert_eq!(mid.accuracy, 100.0);
        assert_eq!(mid.error_count, 0);
        assert_eq!(mid.word_outcomes, [WordOutcome::Correct]);

        for c in "sleep".chars() {
            session.type_char_at(c, at(t0, 120));
        }
        assert_eq!(
            session.state(),
            SessionState::Complete,
            "finishing correctly ends the session"
        );

        let done = session.snapshot_at(at(t0, 600));
        assert_eq!(done.elapsed_seconds, 120.0, "the clock stops at completion");
        assert_eq!(done.wpm, 1.0, "10 characters in two minutes");
        assert_eq!(done.net_wpm, 1.0);
        assert_eq!(
            done.word_outcomes,
            [WordOutcome::Correct, WordOutcome::Correct]
        );
        assert_eq!(done.word_accuracy, 100.0);
    }

    #[test]
    fn backspace_removes_a_wrong_character_before_completion() {
        let t0 = Instant::now();
        let mut session = TypingSession::new("cat");
        session.type_char_at('c', t0);
        session.type_char_at('x', t0);
        assert_eq!(session.snapshot_at(t0).error_count, 1);

        session.backspace_at(t0);
        session.type_char_at('a', t0);
        session.type_char_at('t', at(t0, 30));

        let snap = session.snapshot_at(at(t0, 30));
        assert_eq!(snap.state, SessionState::Complete);
        assert_eq!(snap.error_count, 0, "a corrected error is gone");
        assert_eq!(snap.accuracy, 100.0);
        assert_eq!(snap.backspaces, 1, "but the correction is still counted");
    }

    #[test]
    fn a_paused_session_ignores_input_and_excludes_paused_time() {
        let t0 = Instant::now();
        let mut session = TypingSession::new("cat");
        session.type_char_at('c', t0);
        session
            .pause_at(at(t0, 10))
            .expect("an active session can pause");

        session.type_char_at('a', at(t0, 20));
        session.backspace_at(at(t0, 20));
        let paused = session.snapshot_at(at(t0, 50));
        assert_eq!(paused.state, SessionState::Paused);
        assert_eq!(paused.outcomes.len(), 1, "typing is ignored while paused");
        assert_eq!(paused.backspaces, 0, "so is backspace");
        assert_eq!(paused.elapsed_seconds, 10.0, "the clock stops while paused");

        session
            .resume_at(at(t0, 70))
            .expect("a paused session can resume");
        session.type_char_at('a', at(t0, 90));
        session.type_char_at('t', at(t0, 90));

        let done = session.snapshot_at(at(t0, 90));
        assert_eq!(done.state, SessionState::Complete);
        assert_eq!(done.elapsed_seconds, 30.0, "90s minus 60s paused");
    }

    #[test]
    fn invalid_transitions_are_errors_not_panics() {
        let t0 = Instant::now();
        let mut session = TypingSession::new("cat");
        assert!(
            matches!(
                session.pause_at(t0),
                Err(CoreError::InvalidSessionState { .. })
            ),
            "cannot pause before typing"
        );
        assert!(matches!(
            session.resume_at(t0),
            Err(CoreError::InvalidSessionState { .. })
        ));

        session.type_char_at('c', t0);
        assert!(
            session.resume_at(t0).is_err(),
            "cannot resume an active session"
        );
        session.pause_at(t0).unwrap();
        assert!(session.pause_at(t0).is_err(), "cannot pause twice");

        session.complete_at(t0);
        assert!(session.pause_at(t0).is_err());
        assert!(session.resume_at(t0).is_err());
        assert_eq!(session.state(), SessionState::Complete);
    }

    #[test]
    fn a_session_that_never_finishes_can_be_completed_explicitly() {
        let t0 = Instant::now();
        let mut session = TypingSession::new("cat sat");
        for c in "cat".chars() {
            session.type_char_at(c, t0);
        }
        assert_eq!(session.state(), SessionState::Active, "not finished yet");

        session.complete_at(at(t0, 30));
        assert_eq!(session.state(), SessionState::Complete);

        session.type_char_at(' ', at(t0, 40));
        session.complete_at(at(t0, 90));
        let snap = session.snapshot_at(at(t0, 120));
        assert_eq!(snap.outcomes.len(), 3, "typing after completion is ignored");
        assert_eq!(
            snap.elapsed_seconds, 30.0,
            "completing twice keeps the first time"
        );
    }

    #[test]
    fn a_wrong_final_character_does_not_complete_the_session() {
        let t0 = Instant::now();
        let mut session = TypingSession::new("cat");
        for c in "cax".chars() {
            session.type_char_at(c, t0);
        }
        assert_eq!(
            session.state(),
            SessionState::Active,
            "the user can still correct it"
        );
    }

    #[test]
    fn completing_while_paused_freezes_time_at_the_pause() {
        let t0 = Instant::now();
        let mut session = TypingSession::new("cat");
        session.type_char_at('c', t0);
        session.pause_at(at(t0, 40)).unwrap();
        session.complete_at(at(t0, 500));

        assert_eq!(session.snapshot_at(at(t0, 1000)).elapsed_seconds, 40.0);
    }

    #[test]
    fn an_idle_snapshot_has_neutral_defaults() {
        let snap = TypingSession::new("cat").snapshot();
        assert_eq!(snap.state, SessionState::Idle);
        assert_eq!(snap.elapsed_seconds, 0.0);
        assert_eq!(snap.wpm, 0.0);
        assert_eq!(snap.net_wpm, 0.0);
        assert_eq!(snap.accuracy, 100.0);
        assert_eq!(snap.error_count, 0);
        assert_eq!(snap.backspaces, 0);
        assert!(snap.outcomes.is_empty());
        assert!(snap.word_outcomes.is_empty());
        assert_eq!(snap.word_accuracy, 100.0);
    }

    #[test]
    fn public_methods_drive_the_same_lifecycle() {
        let mut session = TypingSession::new("ab");
        session.type_char('a');
        assert_eq!(session.state(), SessionState::Active);
        session.pause().expect("pause");
        assert_eq!(session.state(), SessionState::Paused);
        session.resume().expect("resume");
        session.backspace();
        assert_eq!(session.snapshot().backspaces, 1);
        session.type_char('a');
        session.type_char('b');
        assert_eq!(session.state(), SessionState::Complete);
        session.complete();
        assert_eq!(session.state(), SessionState::Complete);
    }

    // ---- P02 additions ------------------------------------------------------

    #[test]
    fn completion_reasons_are_recorded() {
        let t0 = Instant::now();
        let mut finished = TypingSession::new("ab");
        finished.type_char_at('a', t0);
        finished.type_char_at('b', t0);
        assert_eq!(
            finished.snapshot_at(t0).completion,
            Some(CompletionReason::Finished)
        );

        let mut submitted = TypingSession::new("ab");
        submitted.type_char_at('a', t0);
        submitted.complete_at(t0);
        assert_eq!(
            submitted.snapshot_at(t0).completion,
            Some(CompletionReason::Submitted)
        );
    }

    #[test]
    fn key_stats_keep_a_corrected_error() {
        let t0 = Instant::now();
        let mut s = TypingSession::new("ab");
        s.type_char_at('a', t0);
        s.type_char_at('X', t0);
        s.backspace_at(t0);
        s.type_char_at('b', t0);
        let b = s.key_stats().get(&'b').copied().unwrap_or_default();
        assert_eq!((b.attempts, b.errors), (2, 1));
        assert_eq!(s.snapshot_at(t0).error_count, 0);
    }

    // ---- P03 additions: tail edits ---------------------------------------

    #[test]
    fn a_preedit_only_edit_starts_the_clock_and_counts_a_press() {
        let t0 = Instant::now();
        let mut s = TypingSession::new("ab");
        s.apply_edit_at(&TextEdit::none(), t0);
        assert_eq!(s.state(), SessionState::Active);
        assert_eq!(s.snapshot_at(t0).presses, 1);
        assert_eq!(s.matcher().typed(), "");
    }

    #[test]
    fn backspace_edits_count_like_backspace() {
        let t0 = Instant::now();
        let mut s = TypingSession::new("ab");
        s.apply_edit_at(&TextEdit::backspace(1), t0);
        assert_eq!(s.state(), SessionState::Idle, "backspace never starts");
        s.apply_edit_at(&TextEdit::insert("x"), t0);
        s.apply_edit_at(&TextEdit::backspace(1), t0);
        s.apply_edit_at(&TextEdit::backspace(0), t0);
        let snap = s.snapshot_at(t0);
        assert_eq!(snap.backspaces, 2, "a removed char and a cancelled preedit");
        s.apply_edit_at(&TextEdit::backspace(1), t0);
        assert_eq!(s.snapshot_at(t0).backspaces, 2, "nothing left to remove");
    }

    #[test]
    fn edits_are_ignored_while_paused_and_finish_the_text() {
        let t0 = Instant::now();
        let mut s = TypingSession::new("ab");
        s.apply_edit_at(&TextEdit::insert("a"), t0);
        s.pause_at(t0).unwrap();
        s.apply_edit_at(&TextEdit::insert("b"), t0);
        assert_eq!(s.matcher().typed(), "a");
        s.resume_at(t0).unwrap();
        s.apply_edit_at(&TextEdit::replace(1, "ab"), t0);
        assert_eq!(s.state(), SessionState::Complete);
    }

    #[test]
    fn consistency_comes_from_press_intervals() {
        let t0 = Instant::now();
        let mut s = TypingSession::new("abcd");
        for (i, c) in "abc".chars().enumerate() {
            s.type_char_at(c, t0 + Duration::from_millis(200 * i as u64));
        }
        let snap = s.snapshot_at(t0 + Duration::from_millis(400));
        assert_eq!(snap.consistency, 100.0);
        assert_eq!(snap.presses, 3);
    }
}
