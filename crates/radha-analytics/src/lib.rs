//! # Radha Analytics
//!
//! Aggregates completed sessions into the figures shown on the Analytics screen.
//!
//! ## Scope in Phase 0
//!
//! Scaffold only. Phase 12 implements the real aggregation. What is fixed now is
//! the boundary: analytics reads *completed, persisted* sessions and never
//! participates in the live typing path.
//!
//! ## Why this crate is separate
//!
//! Keeping aggregation out of `radha-core` protects the latency budget. Per
//! `RULES.MD` Rule 33 and `PRD.md` Section 2.3, no analytics work may run while
//! the user is typing — a separate crate with no dependency on the live session
//! loop makes that easy to verify by inspection.

/// Headline figures for the Analytics dashboard.
///
/// Field set will expand in Phase 12 to cover per-product and per-layout
/// breakdowns; these are the always-present aggregates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AnalyticsSummary {
    /// Mean gross WPM across all completed sessions.
    pub average_wpm: f64,
    /// Mean accuracy percentage across all completed sessions.
    pub average_accuracy: f64,
    /// Best gross WPM ever recorded.
    pub best_wpm: f64,
    /// Total practice time, in seconds.
    pub total_practice_seconds: u64,
    /// Count of exercises completed at least once.
    pub exercises_completed: u32,
}

/// Produces analytics from persisted session history.
pub trait AnalyticsProvider {
    /// Compute the dashboard summary for a user.
    ///
    /// # Errors
    ///
    /// Returns an error if session history cannot be read.
    fn summary(&self, user_id: i64) -> radha_core::Result<AnalyticsSummary>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_summary_is_all_zeroes() {
        // A brand-new profile must render a valid, empty dashboard rather than
        // requiring the UI to special-case "no data yet".
        let s = AnalyticsSummary::default();
        assert_eq!(s.average_wpm, 0.0);
        assert_eq!(s.best_wpm, 0.0);
        assert_eq!(s.exercises_completed, 0);
        assert_eq!(s.total_practice_seconds, 0);
    }
}
