//! Per-key and per-finger statistics, in RAM only.
//!
//! Keyed by the *expected* unit (the key the typist was asked for), as Golu's
//! `StatisticsEngine` does: "you miss `b`" is the useful fact, not "you type
//! `x` a lot". An extra unit has no expected key, so it is charged to the
//! unit actually typed.
//!
//! Nothing here touches storage (RULES.MD Rule 14): the tallies live in the
//! session and are handed over once, with the completed result.
//!
//! Fingers come from the layout crate, which depends on this one, so the
//! finger lookup is passed in as a closure instead of being imported.

use std::collections::HashMap;
use std::time::Duration;

use crate::matcher::Outcome;
use crate::unit::TextUnit;

/// Latencies longer than this are pauses for thought, not key latency, and
/// are left out of the average (Golu `StatisticsEngine.MaxLatencyMs`).
pub const MAX_LATENCY: Duration = Duration::from_millis(2000);

/// Counts for one key (or one finger).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyTally {
    /// Times this key was asked for and answered (correctly or not).
    pub attempts: u32,
    /// How many of those were errors.
    pub errors: u32,
    /// Sum of the counted latencies.
    pub latency_total: Duration,
    /// How many latencies were counted.
    pub latency_samples: u32,
}

impl KeyTally {
    /// Mean counted latency, or `None` without samples.
    #[must_use]
    pub fn mean_latency(&self) -> Option<Duration> {
        (self.latency_samples > 0).then(|| self.latency_total / self.latency_samples)
    }

    fn add(&mut self, other: &KeyTally) {
        self.attempts = self.attempts.saturating_add(other.attempts);
        self.errors = self.errors.saturating_add(other.errors);
        self.latency_total = self.latency_total.saturating_add(other.latency_total);
        self.latency_samples = self.latency_samples.saturating_add(other.latency_samples);
    }
}

/// Per-key tallies for one session.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyStats<U: TextUnit> {
    tallies: HashMap<U, KeyTally>,
}

impl<U: TextUnit> Default for KeyStats<U> {
    fn default() -> Self {
        KeyStats {
            tallies: HashMap::new(),
        }
    }
}

impl<U: TextUnit> KeyStats<U> {
    /// Empty statistics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one outcome. `latency` is the time since the previous accepted
    /// key press; it is counted only when `0 < latency ≤` [`MAX_LATENCY`].
    /// Control characters (tab, newline) are ignored: no key row draws them.
    pub fn record(&mut self, outcome: &Outcome<U>, latency: Option<Duration>) {
        let key = match outcome {
            Outcome::Correct(u) | Outcome::Missing(u) | Outcome::Extra(u) => u,
            Outcome::Wrong { expected, .. } => expected,
        };
        if key.is_control() {
            return;
        }
        let tally = self.tallies.entry(key.clone()).or_default();
        tally.attempts = tally.attempts.saturating_add(1);
        if outcome.is_error() {
            tally.errors = tally.errors.saturating_add(1);
        }
        if let Some(l) = latency.filter(|l| !l.is_zero() && *l <= MAX_LATENCY) {
            tally.latency_total = tally.latency_total.saturating_add(l);
            tally.latency_samples = tally.latency_samples.saturating_add(1);
        }
    }

    /// The tally for `key`, if it was ever recorded.
    #[must_use]
    pub fn get(&self, key: &U) -> Option<&KeyTally> {
        self.tallies.get(key)
    }

    /// Every key with its tally, ordered by key text (stable for display and
    /// tests; a `HashMap` alone has no order).
    #[must_use]
    pub fn sorted(&self) -> Vec<(U, KeyTally)> {
        let mut rows: Vec<(U, KeyTally)> =
            self.tallies.iter().map(|(k, t)| (k.clone(), *t)).collect();
        rows.sort_by(|a, b| a.0.as_text().cmp(&b.0.as_text()));
        rows
    }

    /// Whether nothing was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tallies.is_empty()
    }

    /// Aggregate by finger (or by any grouping `group` returns). Keys `group`
    /// maps to `None` (not on the layout) are left out. Linear search over the
    /// groups keeps the key type free of `Hash`/`Ord` (ten fingers at most).
    pub fn by_finger<K, F>(&self, group: F) -> Vec<(K, KeyTally)>
    where
        K: Copy + Eq,
        F: Fn(&U) -> Option<K>,
    {
        let mut out: Vec<(K, KeyTally)> = Vec::new();
        for (key, tally) in self.sorted() {
            let Some(k) = group(&key) else {
                continue;
            };
            match out.iter_mut().find(|(g, _)| *g == k) {
                Some((_, sum)) => sum.add(&tally),
                None => out.push((k, tally)),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::CharOutcome;

    #[test]
    fn attempts_and_errors_are_separate() {
        let mut s = KeyStats::new();
        s.record(&CharOutcome::Correct('a'), None);
        s.record(
            &CharOutcome::Wrong {
                expected: 'a',
                typed: 'x',
            },
            None,
        );
        s.record(&CharOutcome::Correct('a'), None);
        let a = s.get(&'a').copied().unwrap_or_default();
        assert_eq!((a.attempts, a.errors), (3, 1));
        assert!(s.get(&'x').is_none(), "keyed by the expected key");
    }

    #[test]
    fn a_key_with_no_errors_still_gets_a_row() {
        let mut s = KeyStats::new();
        s.record(&CharOutcome::Correct('b'), None);
        assert_eq!(
            s.sorted(),
            [(
                'b',
                KeyTally {
                    attempts: 1,
                    ..KeyTally::default()
                }
            )]
        );
    }

    #[test]
    fn control_characters_are_skipped() {
        let mut s = KeyStats::new();
        for c in ['\t', '\n', '\0'] {
            s.record(&CharOutcome::Correct(c), None);
        }
        s.record(&CharOutcome::Correct('q'), None);
        let keys: Vec<char> = s.sorted().into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ['q']);
    }

    #[test]
    fn latency_is_capped_at_two_seconds() {
        let mut s = KeyStats::new();
        s.record(&CharOutcome::Correct('a'), Some(Duration::from_millis(200)));
        s.record(
            &CharOutcome::Correct('a'),
            Some(Duration::from_millis(2000)),
        );
        s.record(
            &CharOutcome::Correct('a'),
            Some(Duration::from_millis(2001)),
        );
        s.record(&CharOutcome::Correct('a'), Some(Duration::ZERO));
        let a = s.get(&'a').copied().unwrap_or_default();
        assert_eq!(a.latency_samples, 2);
        assert_eq!(a.mean_latency(), Some(Duration::from_millis(1100)));
    }

    #[test]
    fn by_finger_sums_keys_of_the_same_group() {
        let mut s = KeyStats::new();
        s.record(&CharOutcome::Correct('a'), None);
        s.record(&CharOutcome::Missing('q'), None);
        s.record(&CharOutcome::Correct('j'), None);
        let group = |c: &char| match c {
            'a' | 'q' => Some(1u8),
            'j' => Some(2u8),
            _ => None,
        };
        let fingers = s.by_finger(group);
        assert_eq!(fingers.len(), 2);
        let left = fingers.iter().find(|(g, _)| *g == 1).map(|(_, t)| *t);
        assert_eq!(left.map(|t| (t.attempts, t.errors)), Some((2, 1)));
    }
}
