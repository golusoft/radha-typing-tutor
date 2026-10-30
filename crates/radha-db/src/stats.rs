//! Read-side aggregates for the Analytics, Profile and Home surfaces (P08),
//! plus the few small writes those overlays make (profile name, per-profile
//! values such as the photo path and candidate info) and the database
//! backup.
//!
//! None of this runs while typing. The app calls it when a Result or an
//! overlay opens (`RULES.MD` Rule 33). Summaries come straight from the raw
//! `typing_sessions` / `session_metrics` rows; the daily series reads the
//! `analytics` roll-up that `save_session` maintains in the same
//! transaction, and the tests prove the two agree.

use std::path::Path;

use radha_analytics::{AnalyticsProvider, AnalyticsSummary, DayPoint, ProductAggregate};
use radha_core::{CoreError, Result};
use rusqlite::OptionalExtension;

use crate::Database;

fn sql(action: &str, error: rusqlite::Error) -> CoreError {
    CoreError::Database {
        detail: format!("{action}: {error}"),
    }
}

/// Longest profile display name accepted.
pub const MAX_NAME_CHARS: usize = 40;
/// Longest per-profile value accepted (paths, candidate fields).
pub const MAX_VALUE_CHARS: usize = 512;

/// Error totals across a profile's sessions (`session_metrics`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ErrorTotals {
    pub wrong: u64,
    pub extra: u64,
    pub missing: u64,
    /// Whitespace errors (a view across the three above).
    pub space: u64,
    pub backspaces: u64,
}

/// One key the profile misses most (all sessions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeakKeyTotal {
    pub physical_key: String,
    pub expected_char: String,
    pub occurrences: u64,
}

/// One completed session, for history lists, Quick Resume and CSV export.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionLine {
    pub session_id: i64,
    pub completed_at: String,
    pub product_slug: String,
    pub product_name: String,
    pub exercise_id: i64,
    pub exercise_number: i64,
    pub exercise_title: String,
    pub mode: Option<String>,
    pub gross_wpm: f64,
    pub net_wpm: f64,
    pub accuracy: f64,
    pub elapsed_secs: i64,
}

impl AnalyticsProvider for Database {
    fn summary(&self, user_id: i64) -> Result<AnalyticsSummary> {
        let c = self.connection();
        let mut s = c
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(s.elapsed_secs), 0),
                        AVG(m.gross_wpm), AVG(m.net_wpm), AVG(m.accuracy), AVG(m.consistency),
                        MAX(m.gross_wpm), MAX(m.net_wpm), MAX(m.accuracy)
                   FROM typing_sessions AS s
                   JOIN session_metrics AS m ON m.session_id = s.id
                  WHERE s.profile_id = ?1",
                [user_id],
                |r| {
                    Ok(AnalyticsSummary {
                        sessions: r.get::<_, i64>(0)?.clamp(0, i64::from(u32::MAX)) as u32,
                        total_practice_seconds: r.get::<_, i64>(1)?.max(0) as u64,
                        average_wpm: r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                        average_net_wpm: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                        average_accuracy: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                        average_consistency: r.get(5)?,
                        best_wpm: r.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                        best_net_wpm: r.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
                        best_accuracy: r.get::<_, Option<f64>>(8)?.unwrap_or(0.0),
                        ..AnalyticsSummary::default()
                    })
                },
            )
            .map_err(|e| sql("could not summarise sessions", e))?;
        let (completed, practised): (i64, i64) = c
            .query_row(
                "SELECT COALESCE(SUM(completed), 0), COUNT(*) FROM exercise_progress
                  WHERE profile_id = ?1 AND attempts > 0",
                [user_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| sql("could not count exercises", e))?;
        s.exercises_completed = completed.clamp(0, i64::from(u32::MAX)) as u32;
        s.exercises_practised = practised.clamp(0, i64::from(u32::MAX)) as u32;
        Ok(s)
    }
}

impl Database {
    /// Every product (in Home order) with the profile's figures on it.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn product_aggregates(&self, profile_id: i64) -> Result<Vec<ProductAggregate>> {
        let mut stmt = self
            .connection()
            .prepare(
                "SELECT p.slug, p.display_name, l.slug, l.display_name,
                        COUNT(s.id), COALESCE(SUM(s.elapsed_secs), 0),
                        AVG(m.net_wpm), AVG(m.accuracy), MAX(m.net_wpm)
                   FROM products AS p
                   JOIN layouts AS l ON l.id = p.layout_id
                   LEFT JOIN exercises AS e ON e.product_id = p.id
                   LEFT JOIN typing_sessions AS s
                          ON s.exercise_id = e.id AND s.profile_id = ?1
                   LEFT JOIN session_metrics AS m ON m.session_id = s.id
                  GROUP BY p.id
                  ORDER BY p.display_order",
            )
            .map_err(|e| sql("could not prepare the product aggregates", e))?;
        let rows = stmt
            .query_map([profile_id], |r| {
                Ok(ProductAggregate {
                    product_slug: r.get(0)?,
                    product_name: r.get(1)?,
                    layout_slug: r.get(2)?,
                    layout_name: r.get(3)?,
                    sessions: r.get::<_, i64>(4)?.clamp(0, i64::from(u32::MAX)) as u32,
                    practice_seconds: r.get::<_, i64>(5)?.max(0) as u64,
                    average_net_wpm: r.get(6)?,
                    average_accuracy: r.get(7)?,
                    best_net_wpm: r.get(8)?,
                })
            })
            .map_err(|e| sql("could not read the product aggregates", e))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| sql("could not read a product aggregate", e))
    }

    /// The daily roll-up from `since` (`YYYY-MM-DD`, inclusive), all
    /// products summed, oldest first.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn daily_points(&self, profile_id: i64, since: &str) -> Result<Vec<DayPoint>> {
        let mut stmt = self
            .connection()
            .prepare(
                "SELECT day, SUM(sessions), SUM(practice_secs),
                        SUM(COALESCE(avg_wpm, 0) * sessions) / SUM(sessions),
                        SUM(COALESCE(avg_accuracy, 0) * sessions) / SUM(sessions)
                   FROM analytics
                  WHERE profile_id = ?1 AND day >= ?2 AND sessions > 0
                  GROUP BY day
                  ORDER BY day",
            )
            .map_err(|e| sql("could not prepare the daily series", e))?;
        let rows = stmt
            .query_map(rusqlite::params![profile_id, since], |r| {
                Ok(DayPoint {
                    day: r.get(0)?,
                    sessions: r.get::<_, i64>(1)?.clamp(0, i64::from(u32::MAX)) as u32,
                    practice_seconds: r.get::<_, i64>(2)?.max(0) as u64,
                    average_net_wpm: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                    average_accuracy: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                })
            })
            .map_err(|e| sql("could not read the daily series", e))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| sql("could not read a daily point", e))
    }

    /// Distinct practice days, newest first (at most `limit`).
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn practice_days(&self, profile_id: i64, limit: u32) -> Result<Vec<String>> {
        let mut stmt = self
            .connection()
            .prepare(
                "SELECT DISTINCT day FROM analytics
                  WHERE profile_id = ?1 AND sessions > 0
                  ORDER BY day DESC LIMIT ?2",
            )
            .map_err(|e| sql("could not prepare the practice days", e))?;
        let rows = stmt
            .query_map(rusqlite::params![profile_id, limit], |r| r.get(0))
            .map_err(|e| sql("could not read the practice days", e))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| sql("could not read a practice day", e))
    }

    /// Error totals across the profile's sessions.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn error_totals(&self, profile_id: i64) -> Result<ErrorTotals> {
        self.connection()
            .query_row(
                "SELECT COALESCE(SUM(m.wrong_chars), 0), COALESCE(SUM(m.extra_chars), 0),
                        COALESCE(SUM(m.missing_chars), 0), COALESCE(SUM(m.space_errors), 0),
                        COALESCE(SUM(m.backspaces), 0)
                   FROM typing_sessions AS s
                   JOIN session_metrics AS m ON m.session_id = s.id
                  WHERE s.profile_id = ?1",
                [profile_id],
                |r| {
                    let n = |i| r.get::<_, i64>(i).map(|v| v.max(0) as u64);
                    Ok(ErrorTotals {
                        wrong: n(0)?,
                        extra: n(1)?,
                        missing: n(2)?,
                        space: n(3)?,
                        backspaces: n(4)?,
                    })
                },
            )
            .map_err(|e| sql("could not total errors", e))
    }

    /// The keys missed most across all sessions (whitespace excluded).
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn weak_key_totals(&self, profile_id: i64, limit: u32) -> Result<Vec<WeakKeyTotal>> {
        let mut stmt = self
            .connection()
            .prepare(
                "SELECT e.physical_key, e.expected_char, SUM(e.occurrences) AS n
                   FROM typing_errors AS e
                   JOIN typing_sessions AS s ON s.id = e.session_id
                  WHERE s.profile_id = ?1 AND e.error_kind != 'space'
                  GROUP BY e.physical_key, e.expected_char
                  ORDER BY n DESC, e.physical_key
                  LIMIT ?2",
            )
            .map_err(|e| sql("could not prepare the weak keys", e))?;
        let rows = stmt
            .query_map(rusqlite::params![profile_id, limit], |r| {
                Ok(WeakKeyTotal {
                    physical_key: r.get(0)?,
                    expected_char: r.get(1)?,
                    occurrences: r.get::<_, i64>(2)?.max(0) as u64,
                })
            })
            .map_err(|e| sql("could not read the weak keys", e))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| sql("could not read a weak key", e))
    }

    /// The newest `limit` sessions, newest first (ordered by id, so a wall
    /// clock set backwards cannot reorder history).
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn recent_sessions(&self, profile_id: i64, limit: u32) -> Result<Vec<SessionLine>> {
        let mut stmt = self
            .connection()
            .prepare(
                "SELECT s.id, s.completed_at, p.slug, p.display_name, e.id, e.number, e.title,
                        s.mode, m.gross_wpm, m.net_wpm, m.accuracy, s.elapsed_secs
                   FROM typing_sessions AS s
                   JOIN session_metrics AS m ON m.session_id = s.id
                   JOIN exercises AS e ON e.id = s.exercise_id
                   JOIN products AS p ON p.id = e.product_id
                  WHERE s.profile_id = ?1
                  ORDER BY s.id DESC LIMIT ?2",
            )
            .map_err(|e| sql("could not prepare the session history", e))?;
        let rows = stmt
            .query_map(rusqlite::params![profile_id, limit], |r| {
                Ok(SessionLine {
                    session_id: r.get(0)?,
                    completed_at: r.get(1)?,
                    product_slug: r.get(2)?,
                    product_name: r.get(3)?,
                    exercise_id: r.get(4)?,
                    exercise_number: r.get(5)?,
                    exercise_title: r.get(6)?,
                    mode: r.get(7)?,
                    gross_wpm: r.get(8)?,
                    net_wpm: r.get(9)?,
                    accuracy: r.get(10)?,
                    elapsed_secs: r.get(11)?,
                })
            })
            .map_err(|e| sql("could not read the session history", e))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| sql("could not read a session line", e))
    }

    /// The best Typing Flow smoothness ever scored.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn best_flow_smoothness(&self, profile_id: i64) -> Result<Option<f64>> {
        self.connection()
            .query_row(
                "SELECT MAX(f.smoothness) FROM flow_rhythm AS f
                   JOIN typing_sessions AS s ON s.id = f.session_id
                  WHERE s.profile_id = ?1",
                [profile_id],
                |r| r.get(0),
            )
            .map_err(|e| sql("could not read the best flow smoothness", e))
    }

    /// The profile's display name.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn profile_name(&self, profile_id: i64) -> Result<Option<String>> {
        self.connection()
            .query_row(
                "SELECT display_name FROM profiles WHERE id = ?1",
                [profile_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| sql("could not read the profile", e))
    }

    /// Rename the profile. The name is trimmed and must be 1–40 characters.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`]-style [`CoreError::Database`] detail for an
    /// invalid name; [`CoreError::Database`] if the write fails.
    pub fn set_profile_name(&mut self, profile_id: i64, name: &str) -> Result<String> {
        let name = name.trim();
        let chars = name.chars().count();
        if chars == 0 || chars > MAX_NAME_CHARS || name.chars().any(char::is_control) {
            return Err(CoreError::Database {
                detail: format!("a profile name needs 1 to {MAX_NAME_CHARS} characters"),
            });
        }
        let owned = name.to_owned();
        self.transaction(|tx| {
            tx.execute(
                "UPDATE profiles SET display_name = ?2 WHERE id = ?1",
                rusqlite::params![profile_id, owned],
            )
            .map_err(|e| sql("could not rename the profile", e))?;
            Ok(owned.clone())
        })
    }

    /// A per-profile value (`settings` table), `None` when unset.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn profile_value(&self, profile_id: i64, name: &str) -> Result<Option<String>> {
        self.connection()
            .query_row(
                "SELECT value FROM settings WHERE profile_id = ?1 AND name = ?2",
                rusqlite::params![profile_id, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| sql("could not read a profile value", e))
    }

    /// Set (or, for an empty value, remove) per-profile values in one
    /// transaction. Values are trimmed; over-long values are rejected.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if a value is too long or the write fails;
    /// nothing is kept then.
    pub fn set_profile_values(&mut self, profile_id: i64, values: &[(&str, &str)]) -> Result<()> {
        if let Some((name, _)) = values
            .iter()
            .find(|(_, v)| v.trim().chars().count() > MAX_VALUE_CHARS || v.contains('\0'))
        {
            return Err(CoreError::Database {
                detail: format!("{name} is longer than {MAX_VALUE_CHARS} characters"),
            });
        }
        self.transaction(|tx| {
            for (name, value) in values {
                let value = value.trim();
                if value.is_empty() {
                    tx.execute(
                        "DELETE FROM settings WHERE profile_id = ?1 AND name = ?2",
                        rusqlite::params![profile_id, name],
                    )
                    .map_err(|e| sql("could not clear a profile value", e))?;
                } else {
                    tx.execute(
                        "INSERT INTO settings (profile_id, name, value) VALUES (?1, ?2, ?3)
                         ON CONFLICT (profile_id, name) DO UPDATE SET
                             value = excluded.value, updated_at = datetime('now')",
                        rusqlite::params![profile_id, name, value],
                    )
                    .map_err(|e| sql("could not store a profile value", e))?;
                }
            }
            Ok(())
        })
    }

    /// Write a consistent copy of the whole database to `dest`
    /// (`VACUUM INTO`). `dest` must not exist yet.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if `dest` exists or the copy fails.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if dest.exists() {
            return Err(CoreError::Database {
                detail: format!("{} already exists", dest.display()),
            });
        }
        let target = dest.to_string_lossy().into_owned();
        self.connection()
            .execute("VACUUM INTO ?1", [target])
            .map(|_| ())
            .map_err(|e| sql("could not back up the database", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_import::import_bundle;
    use crate::sessions::{ErrorAggregate, ErrorKind, SessionMetricsRecord, SessionRecord};
    use radha_content::import::{ContentBundle, ImportedExercise, BUNDLE_FORMAT};
    use radha_content::ProductId;
    use radha_layouts::LayoutId;

    fn bundle(product: &str, text: &str) -> ContentBundle {
        ContentBundle {
            format: BUNDLE_FORMAT,
            product: product.into(),
            source: "test".into(),
            provisional: false,
            exercises: (1..=2)
                .map(|n| ImportedExercise {
                    number: n,
                    title: format!("Ex {n}"),
                    category: None,
                    difficulty: None,
                    content: format!("{text} {n}"),
                    source_id: String::new(),
                    ..ImportedExercise::default()
                })
                .collect(),
        }
    }

    fn seeded() -> (Database, i64) {
        let mut db = Database::open_in_memory().unwrap();
        import_bundle(&mut db, &bundle("english-classic", "the cat sat")).unwrap();
        import_bundle(&mut db, &bundle("typing-flow", "a steady beat")).unwrap();
        import_bundle(&mut db, &bundle("hindi-mastery-gail", "नमस्ते दुनिया")).unwrap();
        let profile = db.ensure_default_profile().unwrap();
        (db, profile)
    }

    fn exercise(db: &Database, product: &str, number: i64) -> i64 {
        db.connection()
            .query_row(
                "SELECT e.id FROM exercises e JOIN products p ON p.id = e.product_id
                  WHERE p.slug = ?1 AND e.number = ?2",
                rusqlite::params![product, number],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// A deterministic pseudo-random history (SplitMix64).
    fn history(db: &mut Database, profile: i64, n: usize) -> Vec<SessionRecord> {
        let mut state: u64 = 0x5EED_0008;
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let products = [
            (ProductId::EnglishClassic, "english-classic"),
            (ProductId::TypingFlow, "typing-flow"),
            (
                ProductId::HindiMastery(LayoutId::RemingtonGail),
                "hindi-mastery-gail",
            ),
        ];
        let mut out = Vec::new();
        for i in 0..n {
            let (product, slug) = products[(next() % 3) as usize];
            let ex = exercise(db, slug, (next() % 2) as i64 + 1);
            let day = format!("2026-10-{:02}", 10 + (next() % 15));
            let net = (next() % 6000) as f64 / 100.0;
            let r = SessionRecord {
                profile_id: profile,
                exercise_id: ex,
                product,
                started_at: format!("{day}T10:00:00Z"),
                completed_at: format!("{day}T10:0{}:00Z", i % 10),
                timer_secs: Some(60),
                elapsed_secs: 30 + (next() % 300) as u32,
                display_mode: "word",
                guide_mode: "off",
                mode: Some("test"),
                finished: next() % 2 == 0,
                day,
                metrics: SessionMetricsRecord {
                    gross_wpm: net + 3.0,
                    net_wpm: net,
                    accuracy: 80.0 + (next() % 2000) as f64 / 100.0,
                    consistency: Some(70.0),
                    wrong_chars: (next() % 5) as u32,
                    extra_chars: (next() % 3) as u32,
                    missing_chars: (next() % 2) as u32,
                    space_errors: 1,
                    backspaces: (next() % 7) as u32,
                    ..SessionMetricsRecord::default()
                },
                errors: vec![ErrorAggregate {
                    physical_key: ["S", "D", "K"][(next() % 3) as usize].into(),
                    expected_char: "s".into(),
                    kind: ErrorKind::Wrong,
                    occurrences: 1 + (next() % 3) as u32,
                }],
                flow: None,
            };
            db.save_session(&r).unwrap();
            out.push(r);
        }
        out
    }

    fn mean(v: &[f64]) -> f64 {
        v.iter().sum::<f64>() / v.len() as f64
    }

    #[test]
    fn an_empty_profile_has_empty_aggregates() {
        let (db, profile) = seeded();
        let s = db.summary(profile).unwrap();
        assert_eq!(s, AnalyticsSummary::default());
        let products = db.product_aggregates(profile).unwrap();
        // Nine Home products plus InScript Mastery (migration 0005).
        assert_eq!(products.len(), 10, "every product is listed");
        assert!(products
            .iter()
            .all(|p| p.sessions == 0 && p.average_net_wpm.is_none()));
        assert!(db.daily_points(profile, "2000-01-01").unwrap().is_empty());
        assert!(db.recent_sessions(profile, 5).unwrap().is_empty());
        assert_eq!(db.best_flow_smoothness(profile).unwrap(), None);
        assert_eq!(db.error_totals(profile).unwrap(), ErrorTotals::default());
    }

    #[test]
    fn aggregates_match_the_raw_sessions() {
        let (mut db, profile) = seeded();
        let records = history(&mut db, profile, 60);

        let s = db.summary(profile).unwrap();
        let nets: Vec<f64> = records.iter().map(|r| r.metrics.net_wpm).collect();
        assert_eq!(s.sessions, 60);
        assert!((s.average_net_wpm - mean(&nets)).abs() < 1e-9);
        assert_eq!(
            s.total_practice_seconds,
            records
                .iter()
                .map(|r| u64::from(r.elapsed_secs))
                .sum::<u64>()
        );
        assert_eq!(s.best_net_wpm, nets.iter().copied().fold(0.0, f64::max));
        let distinct: std::collections::BTreeSet<i64> =
            records.iter().map(|r| r.exercise_id).collect();
        assert_eq!(s.exercises_practised as usize, distinct.len());

        // Per product and per layout against the raw records.
        let products = db.product_aggregates(profile).unwrap();
        for p in &products {
            let mine: Vec<&SessionRecord> = records
                .iter()
                .filter(|r| r.product.slug() == p.product_slug)
                .collect();
            assert_eq!(p.sessions as usize, mine.len(), "{}", p.product_slug);
            if mine.is_empty() {
                assert_eq!(p.average_net_wpm, None);
                continue;
            }
            let n: Vec<f64> = mine.iter().map(|r| r.metrics.net_wpm).collect();
            assert!((p.average_net_wpm.unwrap() - mean(&n)).abs() < 1e-9);
        }
        let layouts = radha_analytics::by_layout(&products);
        let qwerty = layouts.iter().find(|l| l.layout_slug == "qwerty").unwrap();
        let q: Vec<f64> = records
            .iter()
            .filter(|r| r.product.layout() == LayoutId::Qwerty)
            .map(|r| r.metrics.net_wpm)
            .collect();
        assert_eq!(qwerty.sessions as usize, q.len());
        assert!((qwerty.average_net_wpm.unwrap() - mean(&q)).abs() < 1e-9);

        // The daily roll-up (maintained by save_session) equals the raw
        // sessions grouped by day.
        let days = db.daily_points(profile, "2026-10-01").unwrap();
        let mut total = 0;
        for d in &days {
            let mine: Vec<f64> = records
                .iter()
                .filter(|r| r.day == d.day)
                .map(|r| r.metrics.net_wpm)
                .collect();
            assert_eq!(d.sessions as usize, mine.len(), "{}", d.day);
            assert!((d.average_net_wpm - mean(&mine)).abs() < 1e-6, "{}", d.day);
            total += d.sessions;
        }
        assert_eq!(total, 60);
        assert!(days.windows(2).all(|w| w[0].day < w[1].day), "oldest first");
        assert_eq!(db.practice_days(profile, 400).unwrap().len(), days.len());

        let e = db.error_totals(profile).unwrap();
        assert_eq!(
            e.wrong,
            records
                .iter()
                .map(|r| u64::from(r.metrics.wrong_chars))
                .sum::<u64>()
        );
        assert_eq!(e.space, 60);
        let weak = db.weak_key_totals(profile, 8).unwrap();
        let occ: u64 = records
            .iter()
            .flat_map(|r| &r.errors)
            .map(|e| u64::from(e.occurrences))
            .sum();
        assert_eq!(weak.iter().map(|w| w.occurrences).sum::<u64>(), occ);
        assert!(weak
            .windows(2)
            .all(|w| w[0].occurrences >= w[1].occurrences));

        let recent = db.recent_sessions(profile, 3).unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(
            recent[0].net_wpm, records[59].metrics.net_wpm,
            "newest first"
        );
        // Another profile sees nothing.
        assert_eq!(db.summary(profile + 1).unwrap().sessions, 0);
    }

    #[test]
    fn a_failed_save_leaves_every_aggregate_unchanged() {
        let (mut db, profile) = seeded();
        let records = history(&mut db, profile, 5);
        let before = (
            db.summary(profile).unwrap(),
            db.daily_points(profile, "2000-01-01").unwrap(),
            db.product_aggregates(profile).unwrap(),
        );
        let mut bad = records[0].clone();
        bad.exercise_id = 9_999_999;
        assert!(db.save_session(&bad).is_err());
        let after = (
            db.summary(profile).unwrap(),
            db.daily_points(profile, "2000-01-01").unwrap(),
            db.product_aggregates(profile).unwrap(),
        );
        assert_eq!(before, after, "the save path is atomic");
    }

    #[test]
    fn profile_names_and_values_are_validated_and_stored() {
        let (mut db, profile) = seeded();
        assert_eq!(
            db.profile_name(profile).unwrap().as_deref(),
            Some("Learner")
        );
        assert_eq!(
            db.set_profile_name(profile, "  Golu Kumar ").unwrap(),
            "Golu Kumar"
        );
        assert_eq!(
            db.profile_name(profile).unwrap().as_deref(),
            Some("Golu Kumar")
        );
        assert!(db.set_profile_name(profile, "   ").is_err());
        assert!(db.set_profile_name(profile, &"x".repeat(41)).is_err());
        assert_eq!(
            db.profile_name(profile).unwrap().as_deref(),
            Some("Golu Kumar")
        );

        db.set_profile_values(
            profile,
            &[("candidate.name", "Asha"), ("candidate.roll", "R-12")],
        )
        .unwrap();
        assert_eq!(
            db.profile_value(profile, "candidate.roll")
                .unwrap()
                .as_deref(),
            Some("R-12")
        );
        db.set_profile_values(profile, &[("candidate.roll", " ")])
            .unwrap();
        assert_eq!(db.profile_value(profile, "candidate.roll").unwrap(), None);
        let long = "y".repeat(MAX_VALUE_CHARS + 1);
        assert!(db
            .set_profile_values(profile, &[("candidate.name", "B"), ("photo", &long)])
            .is_err());
        assert_eq!(
            db.profile_value(profile, "candidate.name")
                .unwrap()
                .as_deref(),
            Some("Asha"),
            "a rejected batch writes nothing"
        );
    }

    #[test]
    fn a_backup_is_a_complete_openable_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radha.sqlite");
        let mut db = Database::open(&path).unwrap();
        import_bundle(&mut db, &bundle("english-classic", "the cat sat")).unwrap();
        let profile = db.ensure_default_profile().unwrap();
        history_on_english(&mut db, profile);
        let dest = dir.path().join("backup.sqlite");
        db.backup_to(&dest).unwrap();
        assert!(db.backup_to(&dest).is_err(), "never overwrites");
        let copy = Database::open(&dest).unwrap();
        assert_eq!(copy.summary(profile).unwrap(), db.summary(profile).unwrap());
    }

    fn history_on_english(db: &mut Database, profile: i64) {
        let ex = exercise(db, "english-classic", 1);
        let r = SessionRecord {
            profile_id: profile,
            exercise_id: ex,
            product: ProductId::EnglishClassic,
            started_at: "2026-10-25T10:00:00Z".into(),
            completed_at: "2026-10-25T10:01:00Z".into(),
            timer_secs: None,
            elapsed_secs: 60,
            display_mode: "word",
            guide_mode: "off",
            mode: Some("learn"),
            finished: true,
            day: "2026-10-25".into(),
            metrics: SessionMetricsRecord {
                gross_wpm: 30.0,
                net_wpm: 28.0,
                accuracy: 97.0,
                ..SessionMetricsRecord::default()
            },
            errors: Vec::new(),
            flow: None,
        };
        db.save_session(&r).unwrap();
    }
}
