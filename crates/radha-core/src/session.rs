//! Typing session lifecycle and live metrics.
//!
//! [`SessionState`] is the lifecycle. [`TypingSession`] drives it and turns the
//! matcher's outcomes into the numbers the typing screen shows.
//!
//! Everything lives in RAM. A session is handed to the persistence layer once,
//! on completion, which makes `RULES.MD` Rule 14 (no per-keystroke database
//! writes) structural rather than merely intended: nothing in this module can
//! write to a database, even if a caller wanted it to.

use std::time::{Duration, Instant};

use crate::accuracy::{accuracy, error_count, word_accuracy};
use crate::error::{CoreError, Result};
use crate::matcher::{CharOutcome, Matcher, WordOutcome};
use crate::speed::{net_wpm, wpm};

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

/// One typing attempt at one target text.
///
/// Owns the matcher and the clock. Driven by input events and polled for
/// [`SessionSnapshot`]s; it holds no database handle.
///
/// Every public method delegates to a private `*_at` twin that takes an
/// explicit [`Instant`]. The public methods pass `Instant::now()`; tests pass
/// fixed instants, so timing is checked exactly without sleeping.
#[derive(Debug, Clone)]
pub struct TypingSession {
    matcher: Matcher,
    state: SessionState,
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
}

/// The live metrics of a session at one moment.
///
/// A plain value, recomputed from scratch by [`TypingSession::snapshot`] on
/// every call, so what the screen shows can never be stale relative to the
/// typed text.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
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
    pub outcomes: Vec<CharOutcome>,
    /// One outcome per committed word, for WORD mode.
    pub word_outcomes: Vec<WordOutcome>,
    /// Correct committed words, as a percentage.
    pub word_accuracy: f64,
}

impl TypingSession {
    /// Start an idle session for `target`. The clock starts on the first key.
    #[must_use]
    pub fn new(target: &str) -> Self {
        TypingSession {
            matcher: Matcher::new(target),
            state: SessionState::Idle,
            started_at: None,
            paused_at: None,
            paused_total: Duration::ZERO,
            completed_at: None,
            backspaces: 0,
        }
    }

    /// Current lifecycle state.
    #[must_use]
    pub fn state(&self) -> SessionState {
        self.state
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

    /// End the session, for example when the timer runs out.
    ///
    /// Infallible and idempotent: the timer and a final keystroke may both try
    /// to end the session, and ending it twice must be harmless. Completing a
    /// paused session freezes its time at the pause.
    pub fn complete(&mut self) {
        self.complete_at(Instant::now());
    }

    /// The live metrics right now.
    #[must_use]
    pub fn snapshot(&self) -> SessionSnapshot {
        self.snapshot_at(Instant::now())
    }

    fn type_char_at(&mut self, c: char, now: Instant) {
        if self.state == SessionState::Idle {
            self.state = SessionState::Active;
            self.started_at = Some(now);
        }
        if !self.state.accepts_input() {
            return;
        }
        self.matcher.push(c);
        if self.matcher.evaluate().2 {
            self.complete_at(now);
        }
    }

    // `_now` is unused today; kept so every method has the same `*_at` shape.
    fn backspace_at(&mut self, _now: Instant) {
        if !self.state.accepts_input() {
            return;
        }
        if self.matcher.pop().is_some() {
            self.backspaces = self.backspaces.saturating_add(1);
        }
    }

    fn pause_at(&mut self, now: Instant) -> Result<()> {
        if self.state != SessionState::Active {
            return Err(CoreError::InvalidSessionState {
                detail: format!("cannot pause a session that is {:?}", self.state),
            });
        }
        self.state = SessionState::Paused;
        self.paused_at = Some(now);
        Ok(())
    }

    fn resume_at(&mut self, now: Instant) -> Result<()> {
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

    fn complete_at(&mut self, now: Instant) {
        if self.state == SessionState::Complete {
            return;
        }
        self.completed_at = if self.state == SessionState::Paused {
            self.paused_at
        } else {
            Some(now)
        };
        self.state = SessionState::Complete;
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

    fn snapshot_at(&self, now: Instant) -> SessionSnapshot {
        let elapsed_seconds = self.elapsed_at(now).as_secs_f64();
        let (outcomes, word_outcomes, _) = self.matcher.evaluate();

        // Every outcome except Missing corresponds to exactly one typed
        // character, so this is the typed length. Skipped letters were never
        // typed and must not inflate speed.
        let characters_typed = outcomes
            .iter()
            .filter(|o| !matches!(o, CharOutcome::Missing(_)))
            .count();
        let errors = error_count(&outcomes);
        let gross = wpm(characters_typed, elapsed_seconds);

        SessionSnapshot {
            state: self.state,
            elapsed_seconds,
            wpm: gross,
            net_wpm: net_wpm(gross, errors, elapsed_seconds / 60.0),
            accuracy: accuracy(&outcomes),
            error_count: errors,
            backspaces: self.backspaces,
            word_accuracy: word_accuracy(&word_outcomes),
            outcomes,
            word_outcomes,
        }
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
        assert!(session.resume_at(t0).is_err(), "cannot resume an active session");
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
}
