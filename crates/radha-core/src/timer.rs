//! Timer settings and deadline maths (RULES.MD Rule 30).
//!
//! Three modes: off, stopwatch (counts up), countdown (time-up completes the
//! session). Ten presets plus a custom limit of 1 s – 60 min. The 45-minute
//! option of the v1 PRD timer table is superseded by RULES 30 and PRD v2
//! (17.1) and is deliberately not a preset.
//!
//! Everything here is pure: limits are validated values and the deadline test
//! works on [`Duration`] (integer nanoseconds), never `f64`, so 599.9 s vs
//! 600 s is decided exactly. The session owns the clock; this module only
//! answers "how long is left" and "has it expired" for a given elapsed time.

use std::time::Duration;

use crate::error::{CoreError, Result};

/// The ten fixed timer presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimerPreset {
    /// 30 seconds.
    Sec30,
    /// 1 minute.
    Min1,
    /// 2 minutes.
    Min2,
    /// 3 minutes.
    Min3,
    /// 5 minutes.
    Min5,
    /// 10 minutes.
    Min10,
    /// 15 minutes.
    Min15,
    /// 20 minutes.
    Min20,
    /// 25 minutes.
    Min25,
    /// 30 minutes.
    Min30,
}

impl TimerPreset {
    /// Every preset, shortest first (picker order).
    #[must_use]
    pub fn all() -> &'static [TimerPreset] {
        &[
            TimerPreset::Sec30,
            TimerPreset::Min1,
            TimerPreset::Min2,
            TimerPreset::Min3,
            TimerPreset::Min5,
            TimerPreset::Min10,
            TimerPreset::Min15,
            TimerPreset::Min20,
            TimerPreset::Min25,
            TimerPreset::Min30,
        ]
    }

    /// Length in seconds.
    #[must_use]
    pub fn seconds(self) -> u32 {
        match self {
            TimerPreset::Sec30 => 30,
            TimerPreset::Min1 => 60,
            TimerPreset::Min2 => 120,
            TimerPreset::Min3 => 180,
            TimerPreset::Min5 => 300,
            TimerPreset::Min10 => 600,
            TimerPreset::Min15 => 900,
            TimerPreset::Min20 => 1200,
            TimerPreset::Min25 => 1500,
            TimerPreset::Min30 => 1800,
        }
    }
}

/// A validated countdown length: whole seconds in `1..=3600`.
///
/// A newtype so an out-of-range limit cannot exist: construction is the only
/// place the range is checked, and it returns an error instead of clamping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimeLimit(u32);

impl TimeLimit {
    /// Shortest custom limit, in seconds.
    pub const MIN_SECONDS: u32 = 1;
    /// Longest custom limit, in seconds (60 minutes).
    pub const MAX_SECONDS: u32 = 3600;

    /// The limit of a preset (always valid).
    #[must_use]
    pub fn from_preset(preset: TimerPreset) -> Self {
        TimeLimit(preset.seconds())
    }

    /// A custom limit of `seconds`.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidSetting`] outside 1 s – 3600 s.
    pub fn custom(seconds: u32) -> Result<Self> {
        if (Self::MIN_SECONDS..=Self::MAX_SECONDS).contains(&seconds) {
            Ok(TimeLimit(seconds))
        } else {
            Err(invalid(format!(
                "{seconds} s is outside {} s – {} s",
                Self::MIN_SECONDS,
                Self::MAX_SECONDS
            )))
        }
    }

    /// Parse a custom limit typed as `MM:SS` (minutes 1–2 digits, seconds
    /// exactly 2 digits, `00`–`59`), e.g. `"1:05"`, `"10:00"`, `"60:00"`.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidSetting`] for any other shape, or a total outside
    /// 1 s – 60 min.
    pub fn parse_mm_ss(text: &str) -> Result<Self> {
        let shape = || invalid(format!("{text:?} is not MM:SS"));
        let (mm, ss) = text.split_once(':').ok_or_else(shape)?;
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if !digits(mm) || mm.len() > 2 || !digits(ss) || ss.len() != 2 {
            return Err(shape());
        }
        let minutes: u32 = mm.parse().map_err(|_| shape())?;
        let seconds: u32 = ss.parse().map_err(|_| shape())?;
        if seconds > 59 {
            return Err(invalid(format!("{text:?}: seconds must be 00–59")));
        }
        Self::custom(minutes * 60 + seconds)
    }

    /// Length in whole seconds.
    #[must_use]
    pub fn seconds(self) -> u32 {
        self.0
    }

    /// Length as a [`Duration`].
    #[must_use]
    pub fn as_duration(self) -> Duration {
        Duration::from_secs(u64::from(self.0))
    }
}

fn invalid(detail: String) -> CoreError {
    CoreError::InvalidSetting {
        setting: "time limit",
        detail,
    }
}

/// How a session is timed.
///
/// In every mode the session still measures active typing time, because WPM
/// needs it; the mode decides what the screen shows and whether time runs out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TimerMode {
    /// No visible timer and no time limit.
    Off,
    /// Counts up, no time limit. The default: today's behaviour.
    #[default]
    Stopwatch,
    /// Counts down; reaching zero completes the session.
    Countdown(TimeLimit),
}

impl TimerMode {
    /// The time limit, if this mode has one.
    #[must_use]
    pub fn limit(self) -> Option<TimeLimit> {
        match self {
            TimerMode::Countdown(limit) => Some(limit),
            TimerMode::Off | TimerMode::Stopwatch => None,
        }
    }
}

/// Time left on `limit` after `elapsed`, never negative.
#[must_use]
pub fn remaining(limit: TimeLimit, elapsed: Duration) -> Duration {
    limit.as_duration().saturating_sub(elapsed)
}

/// Whether `elapsed` has reached `limit`. Exactly at the limit counts as
/// expired: a 600 s test is over at 600.000 s, not one tick later.
#[must_use]
pub fn is_expired(limit: TimeLimit, elapsed: Duration) -> bool {
    elapsed >= limit.as_duration()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_exactly_the_rules_30_list() {
        let secs: Vec<u32> = TimerPreset::all().iter().map(|p| p.seconds()).collect();
        assert_eq!(secs, [30, 60, 120, 180, 300, 600, 900, 1200, 1500, 1800]);
        assert!(!secs.contains(&2700), "45 min is not a preset (RULES 30)");
    }

    #[test]
    fn custom_limits_are_validated_not_clamped() {
        assert!(matches!(
            TimeLimit::custom(0),
            Err(CoreError::InvalidSetting { .. })
        ));
        assert_eq!(TimeLimit::custom(1).map(TimeLimit::seconds).ok(), Some(1));
        assert_eq!(
            TimeLimit::custom(3600).map(TimeLimit::seconds).ok(),
            Some(3600)
        );
        assert!(matches!(
            TimeLimit::custom(3601),
            Err(CoreError::InvalidSetting { .. })
        ));
    }

    #[test]
    fn mm_ss_parsing() {
        let ok = |s: &str| TimeLimit::parse_mm_ss(s).map(TimeLimit::seconds).ok();
        assert_eq!(ok("00:01"), Some(1));
        assert_eq!(ok("60:00"), Some(3600));
        assert_eq!(ok("1:05"), Some(65));
        assert_eq!(ok("10:00"), Some(600));
        for bad in [
            "00:00", "60:01", "10:60", "", "ab:cd", "1:5", "-1:00", "100:00", ":30", "5",
        ] {
            assert!(
                matches!(
                    TimeLimit::parse_mm_ss(bad),
                    Err(CoreError::InvalidSetting { .. })
                ),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn deadline_maths_is_exact_at_the_boundary() {
        let ten_min = TimeLimit::from_preset(TimerPreset::Min10);
        let just_before = Duration::from_millis(599_900);
        let at = Duration::from_secs(600);
        assert!(!is_expired(ten_min, just_before));
        assert_eq!(remaining(ten_min, just_before), Duration::from_millis(100));
        assert!(is_expired(ten_min, at));
        assert_eq!(remaining(ten_min, at), Duration::ZERO);
        assert_eq!(remaining(ten_min, Duration::from_secs(700)), Duration::ZERO);
    }

    #[test]
    fn only_countdown_has_a_limit() {
        assert_eq!(TimerMode::default(), TimerMode::Stopwatch);
        assert_eq!(TimerMode::Off.limit(), None);
        let l = TimeLimit::from_preset(TimerPreset::Sec30);
        assert_eq!(TimerMode::Countdown(l).limit(), Some(l));
    }
}
