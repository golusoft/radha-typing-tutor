//! # Radha Analytics
//!
//! Aggregates completed sessions into the figures shown on the Analytics
//! overlay, the Profile overlay and the Home hero (P08).
//!
//! ## Boundary
//!
//! Analytics reads *completed, persisted* sessions and never participates in
//! the live typing path: the app computes these figures when a Result or an
//! overlay opens, never per key (`RULES.MD` Rule 33, `PRD.md` Section 2.3).
//! The SQL lives in `radha-db`; this crate holds the shared types and the
//! pure arithmetic over them (per-layout roll-up, day windows, streaks,
//! level, achievements), so each rule is one tested function.
//!
//! Nothing here invents a figure: an empty history yields empty / `None`
//! values and the UI shows an explicit empty state.

/// Headline figures for the Analytics dashboard, from raw sessions.
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
    /// Completed sessions recorded.
    pub sessions: u32,
    /// Mean net WPM.
    pub average_net_wpm: f64,
    /// Mean consistency, `None` when no session recorded one.
    pub average_consistency: Option<f64>,
    /// Best net WPM.
    pub best_net_wpm: f64,
    /// Best session accuracy.
    pub best_accuracy: f64,
    /// Distinct exercises practised at least once.
    pub exercises_practised: u32,
}

impl AnalyticsSummary {
    /// Whether any session has been recorded.
    #[must_use]
    pub fn has_data(&self) -> bool {
        self.sessions > 0
    }
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

/// One product's figures (`products` row joined with its sessions).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProductAggregate {
    pub product_slug: String,
    pub product_name: String,
    /// `LayoutId::as_str()` of the product's layout.
    pub layout_slug: String,
    pub layout_name: String,
    pub sessions: u32,
    pub practice_seconds: u64,
    /// `None` without sessions.
    pub average_net_wpm: Option<f64>,
    pub average_accuracy: Option<f64>,
    pub best_net_wpm: Option<f64>,
}

/// One layout's figures, rolled up from its products.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutAggregate {
    pub layout_slug: String,
    pub layout_name: String,
    pub sessions: u32,
    pub practice_seconds: u64,
    pub average_net_wpm: Option<f64>,
    pub average_accuracy: Option<f64>,
    pub best_net_wpm: Option<f64>,
}

/// Roll products up into layouts, in first-seen order. Averages are
/// weighted by session count, so they equal the mean over the layout's raw
/// sessions (the product mean times its count is the product sum).
#[must_use]
pub fn by_layout(products: &[ProductAggregate]) -> Vec<LayoutAggregate> {
    let mut out: Vec<(LayoutAggregate, f64, f64)> = Vec::new();
    for p in products {
        let idx = match out
            .iter()
            .position(|(l, _, _)| l.layout_slug == p.layout_slug)
        {
            Some(i) => i,
            None => {
                out.push((
                    LayoutAggregate {
                        layout_slug: p.layout_slug.clone(),
                        layout_name: p.layout_name.clone(),
                        ..LayoutAggregate::default()
                    },
                    0.0,
                    0.0,
                ));
                out.len() - 1
            }
        };
        let Some((l, net_sum, acc_sum)) = out.get_mut(idx) else {
            continue;
        };
        l.sessions = l.sessions.saturating_add(p.sessions);
        l.practice_seconds = l.practice_seconds.saturating_add(p.practice_seconds);
        let n = f64::from(p.sessions);
        if let Some(v) = p.average_net_wpm {
            *net_sum += v * n;
        }
        if let Some(v) = p.average_accuracy {
            *acc_sum += v * n;
        }
        if let Some(b) = p.best_net_wpm {
            l.best_net_wpm = Some(l.best_net_wpm.map_or(b, |x| x.max(b)));
        }
    }
    out.into_iter()
        .map(|(mut l, net_sum, acc_sum)| {
            if l.sessions > 0 {
                let n = f64::from(l.sessions);
                l.average_net_wpm = Some(net_sum / n);
                l.average_accuracy = Some(acc_sum / n);
            }
            l
        })
        .collect()
}

/// One calendar day of the `analytics` roll-up (all products summed).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DayPoint {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub sessions: u32,
    pub practice_seconds: u64,
    /// Session-weighted mean net WPM of the day.
    pub average_net_wpm: f64,
    /// Session-weighted mean accuracy of the day.
    pub average_accuracy: f64,
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` date, `None` when malformed.
#[must_use]
pub fn day_number(day: &str) -> Option<i64> {
    let mut parts = day.get(..10)?.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// The `YYYY-MM-DD` of a day number (inverse of [`day_number`]).
#[must_use]
pub fn day_string(z: i64) -> String {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The first day of a `days`-long window ending on `today` (inclusive).
#[must_use]
pub fn window_start(today: &str, days: u32) -> Option<String> {
    let t = day_number(today)?;
    Some(day_string(t - i64::from(days.max(1)) + 1))
}

/// The points of `points` inside the `days`-long window ending `today`.
#[must_use]
pub fn in_window(points: &[DayPoint], today: &str, days: u32) -> Vec<DayPoint> {
    let (Some(end), Some(start)) = (
        day_number(today),
        window_start(today, days).and_then(|s| day_number(&s)),
    ) else {
        return Vec::new();
    };
    points
        .iter()
        .filter(|p| day_number(&p.day).is_some_and(|d| (start..=end).contains(&d)))
        .cloned()
        .collect()
}

/// Consecutive practice days ending today, or yesterday (a streak is not
/// broken before the day is over). `days` may be in any order and repeat.
#[must_use]
pub fn streak(days: &[String], today: &str) -> u32 {
    let Some(t) = day_number(today) else {
        return 0;
    };
    let mut nums: Vec<i64> = days.iter().filter_map(|d| day_number(d)).collect();
    nums.sort_unstable();
    nums.dedup();
    let mut expect = if nums.binary_search(&t).is_ok() {
        t
    } else {
        t - 1
    };
    let mut n = 0;
    while nums.binary_search(&expect).is_ok() {
        n += 1;
        expect -= 1;
    }
    n
}

/// Typing level from the best net WPM (practice guide bands, not an exam
/// grade). `None` until a session exists.
#[must_use]
pub fn level(best_net_wpm: f64, sessions: u32) -> Option<&'static str> {
    if sessions == 0 {
        return None;
    }
    Some(match best_net_wpm {
        w if w >= 60.0 => "Expert",
        w if w >= 40.0 => "Advanced",
        w if w >= 30.0 => "Intermediate",
        w if w >= 20.0 => "Developing",
        _ => "Beginner",
    })
}

/// What the achievements are judged on.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AchievementInput {
    pub summary: AnalyticsSummary,
    pub streak: u32,
    /// Best Typing Flow smoothness %, `None` without a scored Flow session.
    pub best_smoothness: Option<f64>,
}

/// One achievement and how far along it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Achievement {
    pub title: &'static str,
    /// "3 of 10 sessions".
    pub progress: String,
    pub earned: bool,
}

/// The fixed achievement list, judged on real history only.
#[must_use]
pub fn achievements(input: &AchievementInput) -> Vec<Achievement> {
    let s = &input.summary;
    let count = |title, have: f64, need: f64, unit: &str| Achievement {
        title,
        progress: format!("{:.0} of {need:.0} {unit}", have.min(need)),
        earned: have >= need,
    };
    vec![
        count("First session", f64::from(s.sessions), 1.0, "session"),
        count("Ten sessions", f64::from(s.sessions), 10.0, "sessions"),
        count(
            "One hour of practice",
            s.total_practice_seconds as f64 / 60.0,
            60.0,
            "minutes",
        ),
        count("30 net WPM", s.best_net_wpm.floor(), 30.0, "net WPM"),
        count("50 net WPM", s.best_net_wpm.floor(), 50.0, "net WPM"),
        count("98% accuracy", s.best_accuracy.floor(), 98.0, "%"),
        count("Three-day streak", f64::from(input.streak), 3.0, "days"),
        match input.best_smoothness {
            Some(v) => count("Smooth flow (90%)", v.floor(), 90.0, "% smoothness"),
            None => Achievement {
                title: "Smooth flow (90%)",
                progress: "no Typing Flow rhythm yet".to_owned(),
                earned: false,
            },
        },
    ]
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
        assert!(!s.has_data());
    }

    fn product(
        slug: &str,
        layout: &str,
        n: u32,
        net: f64,
        acc: f64,
        best: f64,
    ) -> ProductAggregate {
        ProductAggregate {
            product_slug: slug.into(),
            product_name: slug.into(),
            layout_slug: layout.into(),
            layout_name: layout.into(),
            sessions: n,
            practice_seconds: u64::from(n) * 60,
            average_net_wpm: (n > 0).then_some(net),
            average_accuracy: (n > 0).then_some(acc),
            best_net_wpm: (n > 0).then_some(best),
        }
    }

    #[test]
    fn layouts_are_session_weighted_rollups() {
        let rows = [
            product("english", "qwerty", 3, 40.0, 90.0, 50.0),
            product("flow", "qwerty", 1, 20.0, 100.0, 20.0),
            product("gail", "gail", 0, 0.0, 0.0, 0.0),
        ];
        let l = by_layout(&rows);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].sessions, 4);
        assert_eq!(l[0].average_net_wpm, Some(35.0), "(3*40 + 20) / 4");
        assert_eq!(l[0].average_accuracy, Some(92.5));
        assert_eq!(l[0].best_net_wpm, Some(50.0));
        assert_eq!(l[0].practice_seconds, 240);
        assert_eq!(l[1].average_net_wpm, None, "no sessions: no figure");
        assert_eq!(l[1].best_net_wpm, None);
    }

    #[test]
    fn day_arithmetic_round_trips() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("2026-10-25T10:00:00Z"), day_number("2026-10-25"));
        for z in [-1, 0, 59, 365, 20_000, 20_751] {
            assert_eq!(day_number(&day_string(z)), Some(z));
        }
        assert_eq!(
            day_string(day_number("2024-02-29").unwrap() + 1),
            "2024-03-01"
        );
        assert_eq!(day_number("2026-13-01"), None);
        assert_eq!(day_number("bad"), None);
        assert_eq!(window_start("2026-10-25", 7).as_deref(), Some("2026-10-19"));
    }

    #[test]
    fn windows_and_streaks_use_calendar_days() {
        let pts: Vec<DayPoint> = ["2026-10-01", "2026-10-19", "2026-10-25"]
            .iter()
            .map(|d| DayPoint {
                day: (*d).into(),
                sessions: 1,
                ..DayPoint::default()
            })
            .collect();
        assert_eq!(in_window(&pts, "2026-10-25", 7).len(), 2);
        assert_eq!(in_window(&pts, "2026-10-25", 30).len(), 3);
        assert!(in_window(&pts, "nonsense", 7).is_empty());

        let days: Vec<String> = ["2026-10-25", "2026-10-24", "2026-10-24", "2026-10-22"]
            .map(String::from)
            .to_vec();
        assert_eq!(streak(&days, "2026-10-25"), 2);
        assert_eq!(streak(&days, "2026-10-26"), 2, "yesterday keeps it alive");
        assert_eq!(streak(&days, "2026-10-27"), 0);
        assert_eq!(streak(&[], "2026-10-25"), 0);
    }

    #[test]
    fn level_and_achievements_follow_real_history() {
        assert_eq!(level(80.0, 0), None, "no sessions: no level");
        assert_eq!(level(12.0, 1), Some("Beginner"));
        assert_eq!(level(30.0, 1), Some("Intermediate"));
        assert_eq!(level(61.0, 4), Some("Expert"));

        let none = achievements(&AchievementInput::default());
        assert!(
            none.iter().all(|a| !a.earned),
            "nothing earned without history"
        );
        assert_eq!(none.len(), 8);

        let input = AchievementInput {
            summary: AnalyticsSummary {
                sessions: 12,
                total_practice_seconds: 1800,
                best_net_wpm: 31.6,
                best_accuracy: 97.9,
                ..AnalyticsSummary::default()
            },
            streak: 3,
            best_smoothness: Some(91.0),
        };
        let a = achievements(&input);
        let earned: Vec<&str> = a.iter().filter(|a| a.earned).map(|a| a.title).collect();
        assert_eq!(
            earned,
            [
                "First session",
                "Ten sessions",
                "30 net WPM",
                "Three-day streak",
                "Smooth flow (90%)"
            ]
        );
        assert_eq!(a[2].progress, "30 of 60 minutes");
        assert_eq!(a[5].progress, "97 of 98 %", "97.9% is not 98%");
    }
}
