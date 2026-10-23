//! Speed metrics, per `PRD.md` Section 8.2.
//!
//! Pure functions of counts and time with no session state, so the formulas
//! can be checked directly against the PRD's worked examples.

/// Gross words per minute: `(characters_typed / 5) / minutes`, using the
/// standard five-character word.
///
/// 0.0 when no time has elapsed (or the time is negative or NaN): speed cannot
/// be measured before the clock has run, and dividing would report infinity on
/// the very first keystroke.
#[must_use]
pub fn wpm(characters_typed: usize, elapsed_seconds: f64) -> f64 {
    // Equivalent to `!(elapsed_seconds > 0.0)`, NaN included; spelled this way
    // because clippy rejects negated comparisons on floats.
    if elapsed_seconds.is_nan() || elapsed_seconds <= 0.0 {
        return 0.0;
    }
    (characters_typed as f64 / 5.0) / (elapsed_seconds / 60.0)
}

/// Net words per minute: gross WPM minus errors per minute, never below zero
/// (`PRD.md` Section 8.2: "Cannot be negative").
///
/// 0.0 when no time has elapsed, for the same reason as [`wpm`].
#[must_use]
pub fn net_wpm(wpm: f64, errors: usize, elapsed_minutes: f64) -> f64 {
    if elapsed_minutes.is_nan() || elapsed_minutes <= 0.0 {
        return 0.0;
    }
    (wpm - (errors as f64 / elapsed_minutes)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prd_example_300_characters_in_two_minutes_is_30_wpm() {
        assert_eq!(wpm(300, 120.0), 30.0);
    }

    #[test]
    fn prd_example_45_wpm_with_10_errors_in_two_minutes_is_40_net() {
        assert_eq!(net_wpm(45.0, 10, 2.0), 40.0);
    }

    #[test]
    fn zero_negative_or_nan_elapsed_time_is_zero_wpm() {
        assert_eq!(wpm(300, 0.0), 0.0);
        assert_eq!(wpm(300, -1.0), 0.0);
        assert_eq!(wpm(300, f64::NAN), 0.0);
        assert_eq!(net_wpm(45.0, 10, 0.0), 0.0);
    }

    #[test]
    fn net_wpm_is_clamped_at_zero() {
        assert_eq!(net_wpm(10.0, 100, 1.0), 0.0);
    }
}
