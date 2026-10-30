//! Countdown boundary tests (RULES.MD Rule 30), driven through the clock seam
//! with fixed instants: no sleeping, exact to the nanosecond.

use std::time::{Duration, Instant};

use radha_core::{
    CompletionReason, SessionConfig, SessionState, TimeLimit, TimerMode, TimerPreset, TypingSession,
};

fn ms(t0: Instant, millis: u64) -> Instant {
    t0 + Duration::from_millis(millis)
}

fn countdown(target: &str, preset: TimerPreset) -> TypingSession {
    TypingSession::<char>::configured(
        target,
        SessionConfig {
            timer: TimerMode::Countdown(TimeLimit::from_preset(preset)),
        },
    )
}

const TEXT: &str = "the quick brown fox jumps over the lazy dog";

#[test]
fn ten_minutes_is_still_running_at_599_9_s_and_over_at_600_s() {
    let t0 = Instant::now();
    let mut s = countdown(TEXT, TimerPreset::Min10);
    s.type_char_at('t', t0);

    assert!(!s.tick_at(ms(t0, 599_900)));
    let before = s.snapshot_at(ms(t0, 599_900));
    assert_eq!(before.state, SessionState::Active);
    assert!((before.remaining_seconds.unwrap_or(-1.0) - 0.1).abs() < 1e-9);

    assert!(s.tick_at(ms(t0, 600_000)), "this tick ends the session");
    let after = s.snapshot_at(ms(t0, 600_000));
    assert_eq!(after.state, SessionState::Complete);
    assert_eq!(after.completion, Some(CompletionReason::TimeUp));
    assert_eq!(after.elapsed_seconds, 600.0);
    assert_eq!(after.remaining_seconds, Some(0.0));
    assert!(!s.tick_at(ms(t0, 600_001)), "a second tick does nothing");
}

#[test]
fn a_key_just_before_the_deadline_counts_and_one_at_it_does_not() {
    let t0 = Instant::now();
    let mut s = countdown(TEXT, TimerPreset::Min10);
    s.type_char_at('t', t0);
    s.type_char_at('h', ms(t0, 599_999));
    assert_eq!(s.snapshot_at(ms(t0, 599_999)).outcomes.len(), 2);

    s.type_char_at('e', ms(t0, 600_000));
    let snap = s.snapshot_at(ms(t0, 600_000));
    assert_eq!(snap.outcomes.len(), 2, "the key at the deadline is ignored");
    assert_eq!(snap.state, SessionState::Complete);
    assert_eq!(snap.completion, Some(CompletionReason::TimeUp));
}

#[test]
fn a_late_check_never_adds_time() {
    let t0 = Instant::now();
    let mut s = countdown(TEXT, TimerPreset::Min10);
    s.type_char_at('t', t0);
    // Nobody ticked; the next event arrives 50 s late.
    let unticked = s.snapshot_at(ms(t0, 650_000));
    assert_eq!(
        unticked.elapsed_seconds, 600.0,
        "a snapshot clamps to the limit"
    );
    assert!(s.tick_at(ms(t0, 650_000)));
    assert_eq!(s.snapshot_at(ms(t0, 700_000)).elapsed_seconds, 600.0);
}

#[test]
fn paused_time_is_excluded_from_the_countdown() {
    let t0 = Instant::now();
    let mut s = countdown(TEXT, TimerPreset::Min10);
    s.type_char_at('t', t0);
    assert!(s.pause_at(ms(t0, 10_000)).is_ok());
    // A paused session never expires, however long the pause.
    assert!(!s.tick_at(ms(t0, 900_000)));
    assert_eq!(s.state(), SessionState::Paused);
    assert!(s.resume_at(ms(t0, 70_000 + 900_000)).is_ok());
    // 960 s paused: the deadline moved from 600 s to 1560 s.
    assert!(!s.tick_at(ms(t0, 1_559_900)));
    assert!(s.tick_at(ms(t0, 1_560_000)));
    let snap = s.snapshot_at(ms(t0, 2_000_000));
    assert_eq!(snap.elapsed_seconds, 600.0);
    assert_eq!(snap.completion, Some(CompletionReason::TimeUp));
}

#[test]
fn a_short_pause_shifts_the_deadline_by_exactly_its_length() {
    let t0 = Instant::now();
    let mut s = countdown(TEXT, TimerPreset::Min10);
    s.type_char_at('t', t0);
    assert!(s.pause_at(ms(t0, 10_000)).is_ok());
    assert!(s.resume_at(ms(t0, 70_000)).is_ok());
    assert!(!s.tick_at(ms(t0, 659_900)));
    assert!(s.tick_at(ms(t0, 660_000)));
    assert_eq!(s.snapshot_at(ms(t0, 660_000)).elapsed_seconds, 600.0);
}

#[test]
fn the_countdown_starts_on_the_first_keystroke() {
    let t0 = Instant::now();
    let mut s = countdown(TEXT, TimerPreset::Sec30);
    assert!(
        !s.tick_at(ms(t0, 3_600_000)),
        "an idle session never expires"
    );
    let idle = s.snapshot_at(ms(t0, 3_600_000));
    assert_eq!(idle.state, SessionState::Idle);
    assert_eq!(idle.remaining_seconds, Some(30.0));

    let first_key = ms(t0, 3_600_000);
    s.type_char_at('t', first_key);
    assert!(!s.tick_at(first_key + Duration::from_millis(29_999)));
    assert!(s.tick_at(first_key + Duration::from_secs(30)));
}

#[test]
fn stopwatch_and_off_never_expire() {
    let t0 = Instant::now();
    for timer in [TimerMode::Stopwatch, TimerMode::Off] {
        let mut s = TypingSession::<char>::configured(TEXT, SessionConfig { timer });
        s.type_char_at('t', t0);
        assert!(!s.tick_at(ms(t0, 10_000_000)));
        let snap = s.snapshot_at(ms(t0, 10_000_000));
        assert_eq!(snap.state, SessionState::Active);
        assert_eq!(snap.remaining_seconds, None);
        assert_eq!(snap.elapsed_seconds, 10_000.0);
    }
}

#[test]
fn submitting_before_the_deadline_is_submitted_not_time_up() {
    let t0 = Instant::now();
    let mut s = countdown(TEXT, TimerPreset::Min1);
    s.type_char_at('t', t0);
    s.complete_at(ms(t0, 59_999));
    let snap = s.snapshot_at(ms(t0, 120_000));
    assert_eq!(snap.completion, Some(CompletionReason::Submitted));
    assert_eq!(snap.elapsed_seconds, 59.999);
}

#[test]
fn submitting_after_the_deadline_is_time_up_at_the_deadline() {
    let t0 = Instant::now();
    let mut s = countdown(TEXT, TimerPreset::Min1);
    s.type_char_at('t', t0);
    s.complete_at(ms(t0, 90_000));
    let snap = s.snapshot_at(ms(t0, 90_000));
    assert_eq!(snap.completion, Some(CompletionReason::TimeUp));
    assert_eq!(snap.elapsed_seconds, 60.0);
}

#[test]
fn pausing_at_the_deadline_is_an_error_because_time_is_up() {
    let t0 = Instant::now();
    let mut s = countdown(TEXT, TimerPreset::Min1);
    s.type_char_at('t', t0);
    assert!(s.pause_at(ms(t0, 60_000)).is_err());
    assert_eq!(s.state(), SessionState::Complete);
}

#[test]
fn custom_limits_run_to_the_exact_second() {
    let t0 = Instant::now();
    for secs in [1_u32, 3600] {
        let limit = TimeLimit::custom(secs).unwrap_or_else(|e| panic!("{secs}: {e}"));
        let mut s = TypingSession::<char>::configured(
            TEXT,
            SessionConfig {
                timer: TimerMode::Countdown(limit),
            },
        );
        s.type_char_at('t', t0);
        let end = t0 + Duration::from_secs(u64::from(secs));
        assert!(!s.tick_at(end - Duration::from_millis(1)));
        assert!(s.tick_at(end));
    }
}
