//! Persisting a completed typing session — once, in one transaction.
//!
//! `RULES.MD` Rule 14: a session lives in RAM while it is typed and reaches
//! SQLite exactly once, when it is complete. [`Database::save_session`] is
//! that one write. It inserts the session, its metrics and its aggregated
//! error rows, and upserts progress, personal bests (per exercise and per
//! product) and the daily analytics roll-up, all inside a single
//! [`Database::transaction`]: a failure anywhere rolls every table back.
//!
//! There is still no per-keystroke entry point. The caller builds a
//! [`SessionRecord`] from the finished session's snapshot and hands it over.

use radha_content::ProductId;
use radha_core::{CoreError, Result};
use rusqlite::OptionalExtension;

use crate::Database;

/// Error categories stored in `typing_errors.error_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorKind {
    /// A different unit was typed.
    Wrong,
    /// A unit was typed where none was expected.
    Extra,
    /// An expected unit was skipped.
    Missing,
    /// A space error (missing or extra separator).
    Space,
}

impl ErrorKind {
    /// The column value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Wrong => "wrong",
            ErrorKind::Extra => "extra",
            ErrorKind::Missing => "missing",
            ErrorKind::Space => "space",
        }
    }
}

/// One aggregated error row: a key, what was expected, how often.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorAggregate {
    /// The key the error is attributed to (keycap legend, or the unit text).
    pub physical_key: String,
    /// The expected character(s); empty for an extra.
    pub expected_char: String,
    /// Category.
    pub kind: ErrorKind,
    /// Occurrences (at least 1).
    pub occurrences: u32,
}

/// The final figures of a session (`session_metrics`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionMetricsRecord {
    pub gross_wpm: f64,
    pub net_wpm: f64,
    pub accuracy: f64,
    pub consistency: Option<f64>,
    pub word_accuracy: Option<f64>,
    pub correct_chars: u32,
    pub wrong_chars: u32,
    pub extra_chars: u32,
    pub missing_chars: u32,
    pub backspaces: u32,
    pub space_errors: u32,
    pub avg_latency_ms: Option<f64>,
}

/// The Typing Flow rhythm analysis of a session (`flow_rhythm`, P07).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlowRhythmRecord {
    /// `FLOW_CONFIG_SCHEMA` of the thresholds used.
    pub engine_schema: u32,
    /// Smoothness %, `None` when too little was typed to score a rhythm.
    pub smoothness: Option<f64>,
    /// Coefficient of variation of the scored intervals.
    pub cadence_cv: Option<f64>,
    /// Mean scored interval (ms).
    pub mean_interval_ms: Option<f64>,
    pub scored_intervals: u64,
    pub hesitations: u32,
    pub pauses: u32,
    pub corrections: u32,
    /// Weak keys, worst first, as typed characters.
    pub weak_keys: String,
}

/// A completed session, ready to be written.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    pub profile_id: i64,
    pub exercise_id: i64,
    pub product: ProductId,
    /// ISO-8601 UTC, e.g. `2026-10-25T10:00:00Z`.
    pub started_at: String,
    /// ISO-8601 UTC.
    pub completed_at: String,
    /// Selected countdown, `None` for stopwatch/off.
    pub timer_secs: Option<u32>,
    pub elapsed_secs: u32,
    /// `"key"` or `"word"`.
    pub display_mode: &'static str,
    /// `"full"`, `"smart"`, `"minimal"` or `"off"`.
    pub guide_mode: &'static str,
    /// Session mode (`typing_sessions.mode`): `"learn"`, `"test"`,
    /// `"beginner"`, `"classic"`, `"flow"` or `"exam"`; `None` when unknown.
    pub mode: Option<&'static str>,
    /// The whole text was typed (as opposed to time-up or submitted).
    pub finished: bool,
    /// Local calendar day for the analytics roll-up, `YYYY-MM-DD`.
    pub day: String,
    pub metrics: SessionMetricsRecord,
    pub errors: Vec<ErrorAggregate>,
    /// Typing Flow sessions: the rhythm analysis, written in the same
    /// transaction. `None` for every other product.
    pub flow: Option<FlowRhythmRecord>,
}

/// What [`Database::save_session`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedSession {
    /// The new `typing_sessions.id`.
    pub session_id: i64,
    /// The session set a new best net WPM on its exercise.
    pub new_exercise_best: bool,
}

fn sql(action: &str, error: rusqlite::Error) -> CoreError {
    CoreError::Database {
        detail: format!("{action}: {error}"),
    }
}

/// Keep a percentage inside the schema's `0..=100` check.
fn pct(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// Keep a rate non-negative and finite.
fn rate(v: f64) -> f64 {
    if v.is_finite() {
        v.max(0.0)
    } else {
        0.0
    }
}

/// Default display name for the profile created on first run.
pub const DEFAULT_PROFILE_NAME: &str = "Learner";

impl Database {
    /// The first profile's id, creating a default profile if none exists.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the profile cannot be read or created.
    pub fn ensure_default_profile(&mut self) -> Result<i64> {
        self.transaction(|tx| {
            let existing: Option<i64> = tx
                .query_row("SELECT id FROM profiles ORDER BY id LIMIT 1", [], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(|e| sql("could not read profiles", e))?;
            if let Some(id) = existing {
                return Ok(id);
            }
            tx.execute(
                "INSERT INTO profiles (display_name) VALUES (?1)",
                [DEFAULT_PROFILE_NAME],
            )
            .map_err(|e| sql("could not create the default profile", e))?;
            Ok(tx.last_insert_rowid())
        })
    }

    /// Write a completed session and everything derived from it, in one
    /// transaction.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if any write fails; nothing is kept then.
    pub fn save_session(&mut self, record: &SessionRecord) -> Result<SavedSession> {
        let m = &record.metrics;
        let gross = rate(m.gross_wpm);
        let net = rate(m.net_wpm);
        let accuracy = pct(m.accuracy);
        let consistency = m.consistency.map(pct);
        let word_accuracy = m.word_accuracy.map(pct);
        self.transaction(|tx| {
            let product_id: i64 = tx
                .query_row(
                    "SELECT id FROM products WHERE slug = ?1",
                    [record.product.slug()],
                    |r| r.get(0),
                )
                .map_err(|e| sql("unknown product", e))?;

            tx.execute(
                "INSERT INTO typing_sessions
                     (profile_id, exercise_id, started_at, completed_at, timer_secs,
                      elapsed_secs, display_mode, guide_mode, mode)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    record.profile_id,
                    record.exercise_id,
                    record.started_at,
                    record.completed_at,
                    record.timer_secs,
                    record.elapsed_secs,
                    record.display_mode,
                    record.guide_mode,
                    record.mode,
                ],
            )
            .map_err(|e| sql("could not insert the session", e))?;
            let session_id = tx.last_insert_rowid();

            tx.execute(
                "INSERT INTO session_metrics
                     (session_id, gross_wpm, net_wpm, accuracy, consistency, word_accuracy,
                      correct_chars, wrong_chars, extra_chars, missing_chars, backspaces,
                      space_errors, avg_latency_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    session_id,
                    gross,
                    net,
                    accuracy,
                    consistency,
                    word_accuracy,
                    m.correct_chars,
                    m.wrong_chars,
                    m.extra_chars,
                    m.missing_chars,
                    m.backspaces,
                    m.space_errors,
                    m.avg_latency_ms.map(rate),
                ],
            )
            .map_err(|e| sql("could not insert session metrics", e))?;

            if let Some(f) = &record.flow {
                tx.execute(
                    "INSERT INTO flow_rhythm
                         (session_id, engine_schema, smoothness, cadence_cv, mean_interval_ms,
                          scored_intervals, hesitations, pauses, corrections, weak_keys)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        session_id,
                        f.engine_schema.max(1),
                        f.smoothness.filter(|v| v.is_finite()).map(pct),
                        f.cadence_cv.filter(|v| v.is_finite()).map(rate),
                        f.mean_interval_ms.filter(|v| v.is_finite()).map(rate),
                        i64::try_from(f.scored_intervals).unwrap_or(i64::MAX),
                        f.hesitations,
                        f.pauses,
                        f.corrections,
                        f.weak_keys,
                    ],
                )
                .map_err(|e| sql("could not insert the flow rhythm", e))?;
            }

            for err in record.errors.iter().filter(|e| e.occurrences > 0) {
                tx.execute(
                    "INSERT INTO typing_errors
                         (session_id, physical_key, expected_char, error_kind, occurrences)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (session_id, physical_key, expected_char, error_kind)
                     DO UPDATE SET occurrences = occurrences + excluded.occurrences",
                    rusqlite::params![
                        session_id,
                        err.physical_key,
                        err.expected_char,
                        err.kind.as_str(),
                        err.occurrences,
                    ],
                )
                .map_err(|e| sql("could not insert an error row", e))?;
            }

            tx.execute(
                "INSERT INTO exercise_progress
                     (profile_id, exercise_id, attempts, completed, total_practice_secs,
                      first_attempt_at, last_attempt_at)
                 VALUES (?1, ?2, 1, ?3, ?4, ?5, ?5)
                 ON CONFLICT (profile_id, exercise_id) DO UPDATE SET
                     attempts = attempts + 1,
                     completed = MAX(completed, excluded.completed),
                     total_practice_secs = total_practice_secs + excluded.total_practice_secs,
                     last_attempt_at = excluded.last_attempt_at",
                rusqlite::params![
                    record.profile_id,
                    record.exercise_id,
                    record.finished,
                    record.elapsed_secs,
                    record.completed_at,
                ],
            )
            .map_err(|e| sql("could not update exercise progress", e))?;

            let previous_best: Option<f64> = tx
                .query_row(
                    "SELECT best_net_wpm FROM personal_bests
                      WHERE profile_id = ?1 AND exercise_id = ?2",
                    [record.profile_id, record.exercise_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| sql("could not read the exercise best", e))?;

            upsert_best(
                tx,
                record.profile_id,
                product_id,
                Some(record.exercise_id),
                (gross, net, accuracy, consistency),
            )?;
            upsert_best(
                tx,
                record.profile_id,
                product_id,
                None,
                (gross, net, accuracy, consistency),
            )?;

            tx.execute(
                "INSERT INTO analytics
                     (profile_id, product_id, day, sessions, practice_secs, avg_wpm,
                      avg_accuracy, best_wpm)
                 VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?5)
                 ON CONFLICT (profile_id, day, product_id) DO UPDATE SET
                     avg_wpm = (COALESCE(avg_wpm, 0) * sessions + excluded.avg_wpm) / (sessions + 1),
                     avg_accuracy = (COALESCE(avg_accuracy, 0) * sessions + excluded.avg_accuracy)
                                    / (sessions + 1),
                     best_wpm = MAX(COALESCE(best_wpm, 0), excluded.best_wpm),
                     practice_secs = practice_secs + excluded.practice_secs,
                     sessions = sessions + 1",
                rusqlite::params![
                    record.profile_id,
                    product_id,
                    record.day,
                    record.elapsed_secs,
                    net,
                    accuracy,
                ],
            )
            .map_err(|e| sql("could not update analytics", e))?;

            Ok(SavedSession {
                session_id,
                new_exercise_best: previous_best.is_none_or(|b| net > b),
            })
        })
    }
}

impl Database {
    /// The smoothness % of the profile's most recent Typing Flow session
    /// that scored a rhythm, or `None` when there is none yet (the Home
    /// card then shows "—", never an invented figure). Ordered by session
    /// id, so a wall clock set backwards cannot reorder history.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn last_flow_smoothness(&self, profile_id: i64) -> Result<Option<f64>> {
        self.connection()
            .query_row(
                "SELECT f.smoothness FROM flow_rhythm AS f
                   JOIN typing_sessions AS s ON s.id = f.session_id
                  WHERE s.profile_id = ?1 AND f.smoothness IS NOT NULL
                  ORDER BY s.id DESC LIMIT 1",
                [profile_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| sql("could not read the last flow smoothness", e))
    }

    /// Per exercise, the profile's Typing Flow rhythm history: the last and
    /// the best smoothness scored, and how many sessions scored one.
    /// Exercises without a scored rhythm are absent (shown as "—"). Read
    /// when the Flow list opens or a session ends, never per key.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn flow_rhythm_by_exercise(&self, profile_id: i64) -> Result<Vec<FlowExerciseRhythm>> {
        let mut stmt = self
            .connection()
            .prepare(
                "SELECT s.exercise_id, MAX(f.smoothness), COUNT(*),
                        (SELECT f2.smoothness FROM flow_rhythm AS f2
                           JOIN typing_sessions AS s2 ON s2.id = f2.session_id
                          WHERE s2.profile_id = ?1 AND s2.exercise_id = s.exercise_id
                            AND f2.smoothness IS NOT NULL
                          ORDER BY s2.id DESC LIMIT 1)
                   FROM flow_rhythm AS f
                   JOIN typing_sessions AS s ON s.id = f.session_id
                  WHERE s.profile_id = ?1 AND f.smoothness IS NOT NULL
                  GROUP BY s.exercise_id
                  ORDER BY s.exercise_id",
            )
            .map_err(|e| sql("could not prepare the flow history", e))?;
        let rows = stmt
            .query_map([profile_id], |r| {
                let sessions: i64 = r.get(2)?;
                Ok(FlowExerciseRhythm {
                    exercise_id: r.get(0)?,
                    best: r.get(1)?,
                    sessions: u32::try_from(sessions).unwrap_or(u32::MAX),
                    last: r.get(3)?,
                })
            })
            .map_err(|e| sql("could not read the flow history", e))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| sql("could not read a flow history row", e))
    }
}

/// One Typing Flow exercise's rhythm history ([`Database::flow_rhythm_by_exercise`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlowExerciseRhythm {
    pub exercise_id: i64,
    /// Smoothness % of the most recent session that scored one.
    pub last: f64,
    /// Best smoothness % ever scored on it.
    pub best: f64,
    /// Sessions that scored a smoothness.
    pub sessions: u32,
}

/// Raise a personal-best row to the new figures (each metric independently).
fn upsert_best(
    tx: &rusqlite::Transaction<'_>,
    profile_id: i64,
    product_id: i64,
    exercise_id: Option<i64>,
    (gross, net, accuracy, consistency): (f64, f64, f64, Option<f64>),
) -> Result<()> {
    let updated = tx
        .execute(
            "UPDATE personal_bests SET
                 best_wpm = MAX(best_wpm, ?4),
                 best_net_wpm = MAX(best_net_wpm, ?5),
                 best_accuracy = MAX(best_accuracy, ?6),
                 best_consistency = CASE
                     WHEN ?7 IS NULL THEN best_consistency
                     ELSE MAX(COALESCE(best_consistency, 0), ?7) END,
                 updated_at = datetime('now')
             WHERE profile_id = ?1 AND product_id = ?2 AND exercise_id IS ?3",
            rusqlite::params![
                profile_id,
                product_id,
                exercise_id,
                gross,
                net,
                accuracy,
                consistency
            ],
        )
        .map_err(|e| sql("could not update a personal best", e))?;
    if updated == 0 {
        tx.execute(
            "INSERT INTO personal_bests
                 (profile_id, product_id, exercise_id, best_wpm, best_net_wpm, best_accuracy,
                  best_consistency)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                profile_id,
                product_id,
                exercise_id,
                gross,
                net,
                accuracy,
                consistency
            ],
        )
        .map_err(|e| sql("could not insert a personal best", e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_import::import_bundle;
    use crate::SqliteRepository;
    use radha_content::import::{ContentBundle, ImportedExercise, BUNDLE_FORMAT};
    use radha_content::{ExerciseRepository, ProgressRepository};

    fn seeded() -> (Database, i64, i64) {
        let mut db = Database::open_in_memory().unwrap();
        import_bundle(
            &mut db,
            &ContentBundle {
                format: BUNDLE_FORMAT,
                product: "english-classic".into(),
                source: "test".into(),
                provisional: false,
                exercises: vec![ImportedExercise {
                    number: 1,
                    title: "Placeholder".into(),
                    category: None,
                    difficulty: None,
                    content: "the cat sat".into(),
                    source_id: String::new(),
                    ..ImportedExercise::default()
                }],
            },
        )
        .unwrap();
        let profile = db.ensure_default_profile().unwrap();
        let id: i64 = db
            .connection()
            .query_row("SELECT id FROM exercises", [], |r| r.get(0))
            .unwrap();
        (db, profile, id)
    }

    fn record(profile_id: i64, exercise_id: i64, net: f64) -> SessionRecord {
        SessionRecord {
            profile_id,
            exercise_id,
            product: ProductId::EnglishClassic,
            started_at: "2026-10-25T10:00:00Z".into(),
            completed_at: "2026-10-25T10:01:00Z".into(),
            timer_secs: Some(60),
            elapsed_secs: 60,
            display_mode: "word",
            guide_mode: "off",
            mode: Some("test"),
            finished: false,
            day: "2026-10-25".into(),
            metrics: SessionMetricsRecord {
                gross_wpm: net + 2.0,
                net_wpm: net,
                accuracy: 96.5,
                consistency: Some(80.0),
                word_accuracy: Some(90.0),
                correct_chars: 50,
                wrong_chars: 2,
                backspaces: 3,
                ..SessionMetricsRecord::default()
            },
            errors: vec![
                ErrorAggregate {
                    physical_key: "S".into(),
                    expected_char: "s".into(),
                    kind: ErrorKind::Wrong,
                    occurrences: 2,
                },
                ErrorAggregate {
                    physical_key: "X".into(),
                    expected_char: String::new(),
                    kind: ErrorKind::Extra,
                    occurrences: 1,
                },
            ],
            flow: None,
        }
    }

    fn count(db: &Database, table: &str) -> i64 {
        db.connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    const TABLES: [&str; 6] = [
        "typing_sessions",
        "session_metrics",
        "typing_errors",
        "exercise_progress",
        "personal_bests",
        "analytics",
    ];

    #[test]
    fn the_default_profile_is_created_once() {
        let mut db = Database::open_in_memory().unwrap();
        let a = db.ensure_default_profile().unwrap();
        let b = db.ensure_default_profile().unwrap();
        assert_eq!(a, b);
        assert_eq!(count(&db, "profiles"), 1);
    }

    #[test]
    fn nothing_is_written_before_save_and_everything_after() {
        let (mut db, profile, exercise) = seeded();
        for t in TABLES {
            assert_eq!(count(&db, t), 0, "{t} before the session is saved");
        }
        let saved = db.save_session(&record(profile, exercise, 40.0)).unwrap();
        assert!(saved.new_exercise_best);
        assert_eq!(count(&db, "typing_sessions"), 1);
        let mode: Option<String> = db
            .connection()
            .query_row("SELECT mode FROM typing_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.as_deref(), Some("test"), "mode is recorded (0003)");
        assert_eq!(count(&db, "session_metrics"), 1);
        assert_eq!(
            count(&db, "typing_errors"),
            2,
            "aggregated, one row per key"
        );
        assert_eq!(count(&db, "exercise_progress"), 1);
        assert_eq!(count(&db, "personal_bests"), 2, "exercise and product");
        assert_eq!(count(&db, "analytics"), 1);

        let repo_db = db;
        let repo = SqliteRepository::new(repo_db);
        let best = repo.exercise_best(profile, exercise).unwrap().unwrap();
        assert_eq!((best.net_wpm, best.wpm, best.accuracy), (40.0, 42.0, 96.5));
        let progress = repo.exercise_progress(profile, exercise).unwrap().unwrap();
        assert_eq!((progress.attempts, progress.completed), (1, false));
        assert_eq!(progress.total_practice_seconds, 60);
        assert!(repo
            .product_best(profile, ProductId::EnglishClassic)
            .unwrap()
            .is_some());
        assert_eq!(
            repo.list_for_product(ProductId::EnglishClassic)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn bests_only_rise_and_progress_accumulates() {
        let (mut db, profile, exercise) = seeded();
        db.save_session(&record(profile, exercise, 40.0)).unwrap();
        let mut slower = record(profile, exercise, 30.0);
        slower.finished = true;
        let saved = db.save_session(&slower).unwrap();
        assert!(!saved.new_exercise_best);
        let best: f64 = db
            .connection()
            .query_row(
                "SELECT best_net_wpm FROM personal_bests WHERE exercise_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(best, 40.0, "a slower session never lowers the best");
        let (attempts, completed, secs): (i64, bool, i64) = db
            .connection()
            .query_row(
                "SELECT attempts, completed, total_practice_secs FROM exercise_progress",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((attempts, completed, secs), (2, true, 120));
        let (sessions, avg): (i64, f64) = db
            .connection()
            .query_row("SELECT sessions, avg_wpm FROM analytics", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((sessions, avg), (2, 35.0));
    }

    #[test]
    fn a_failing_save_rolls_every_table_back() {
        let (mut db, profile, _) = seeded();
        // An exercise that does not exist violates the foreign key midway.
        assert!(db.save_session(&record(profile, 9_999, 40.0)).is_err());
        for t in TABLES {
            assert_eq!(count(&db, t), 0, "{t} after a failed save");
        }
    }

    #[test]
    fn a_flow_session_saves_its_rhythm_in_the_same_transaction() {
        let (mut db, profile, exercise) = seeded();
        assert_eq!(
            db.last_flow_smoothness(profile).unwrap(),
            None,
            "no session yet"
        );
        let mut r = record(profile, exercise, 40.0);
        r.flow = Some(FlowRhythmRecord {
            engine_schema: 1,
            smoothness: Some(93.5),
            cadence_cv: Some(0.21),
            mean_interval_ms: Some(212.0),
            scored_intervals: 480,
            hesitations: 3,
            pauses: 1,
            corrections: 4,
            weak_keys: "q;".into(),
        });
        db.save_session(&r).unwrap();
        assert_eq!(count(&db, "flow_rhythm"), 1);
        assert_eq!(db.last_flow_smoothness(profile).unwrap(), Some(93.5));
        // A too-short session stores NULL and does not replace the last
        // real smoothness.
        let mut short = record(profile, exercise, 10.0);
        short.flow = Some(FlowRhythmRecord {
            engine_schema: 1,
            smoothness: None,
            ..FlowRhythmRecord::default()
        });
        db.save_session(&short).unwrap();
        assert_eq!(count(&db, "flow_rhythm"), 2);
        assert_eq!(db.last_flow_smoothness(profile).unwrap(), Some(93.5));
        // Out-of-range figures are clamped, NaN stored as NULL.
        let mut odd = record(profile, exercise, 10.0);
        odd.flow = Some(FlowRhythmRecord {
            engine_schema: 0,
            smoothness: Some(140.0),
            cadence_cv: Some(f64::NAN),
            ..FlowRhythmRecord::default()
        });
        db.save_session(&odd).unwrap();
        assert_eq!(db.last_flow_smoothness(profile).unwrap(), Some(100.0));
        // Other products never write a rhythm row; another profile has none.
        db.save_session(&record(profile, exercise, 41.0)).unwrap();
        assert_eq!(count(&db, "flow_rhythm"), 3);
        assert_eq!(db.last_flow_smoothness(profile + 1).unwrap(), None);
        // Per exercise: last 100 (the clamped one), best 100, two scored
        // sessions (the NULL one does not count).
        assert_eq!(
            db.flow_rhythm_by_exercise(profile).unwrap(),
            vec![FlowExerciseRhythm {
                exercise_id: exercise,
                last: 100.0,
                best: 100.0,
                sessions: 2,
            }]
        );
        assert!(db.flow_rhythm_by_exercise(profile + 1).unwrap().is_empty());
    }

    #[test]
    fn the_flow_history_keeps_the_last_and_the_best_apart() {
        let (mut db, profile, exercise) = seeded();
        for s in [91.0, 97.0, 88.5] {
            let mut r = record(profile, exercise, 30.0);
            r.flow = Some(FlowRhythmRecord {
                engine_schema: 1,
                smoothness: Some(s),
                ..FlowRhythmRecord::default()
            });
            db.save_session(&r).unwrap();
        }
        let h = db.flow_rhythm_by_exercise(profile).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].last, h[0].best, h[0].sessions), (88.5, 97.0, 3));
    }

    #[test]
    fn a_failing_flow_save_leaves_no_rhythm_row() {
        let (mut db, profile, _) = seeded();
        let mut r = record(profile, 9_999, 40.0);
        r.flow = Some(FlowRhythmRecord {
            engine_schema: 1,
            smoothness: Some(90.0),
            ..FlowRhythmRecord::default()
        });
        assert!(db.save_session(&r).is_err());
        assert_eq!(count(&db, "flow_rhythm"), 0);
    }

    #[test]
    fn out_of_range_figures_are_clamped_not_rejected() {
        let (mut db, profile, exercise) = seeded();
        let mut r = record(profile, exercise, -3.0);
        r.metrics.accuracy = 120.0;
        r.metrics.gross_wpm = f64::NAN;
        db.save_session(&r).unwrap();
        let (net, acc, gross): (f64, f64, f64) = db
            .connection()
            .query_row(
                "SELECT net_wpm, accuracy, gross_wpm FROM session_metrics",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((net, acc, gross), (0.0, 100.0, 0.0));
    }
}
