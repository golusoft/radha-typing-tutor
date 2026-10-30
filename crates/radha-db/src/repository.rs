//! SQLite-backed repositories.
//!
//! Implements the storage-agnostic traits from `radha-content` against the schema
//! created by `migrations/0002_core_schema.sql`.
//!
//! # The repository owns its own connection
//!
//! [`SqliteRepository`] owns a [`Database`] rather than borrowing the app's. The
//! app keeps its primary `Database` for writes and gives the repository a second
//! connection to the same file:
//!
//! - It lets `App` hold both side by side. A repository borrowing the app's
//!   `Database` would make `App` self-referential, which safe Rust cannot express.
//! - Under WAL (set by `Database::open`), readers never block the writer and the
//!   writer never blocks readers, so the two connections cannot contend, and reads
//!   see every committed write.
//! - Everything here is read-only — the traits have no write methods — so
//!   [`Database::transaction`] stays the single write path that `RULES.MD`
//!   Rule 14 relies on.
//!
//! In tests the repository simply owns an in-memory database.
//!
//! # List queries never read content
//!
//! `list_for_product` selects from `exercises` only and never touches
//! `exercise_content`, per the trait's contract, so each returned
//! `Exercise::content` is empty. That is unambiguous: stored content is
//! constrained non-empty by the schema, so an empty string always means "not
//! loaded". Call `load` for the text.

use std::path::PathBuf;

use radha_content::codec::{decompress_text, sha256_hex};
use radha_content::{
    ContentStatus, ExamPassageMeta, ExamPool, Exercise, ExerciseMeta, ExerciseProgress,
    ExerciseRepository, PersonalBest, ProductId, ProgressRepository,
};
use radha_core::{CoreError, Result};
use rusqlite::OptionalExtension;

use crate::Database;

/// Read access to exercise content, personal bests and progress in SQLite.
pub struct SqliteRepository {
    database: Database,
}

impl SqliteRepository {
    /// Wrap an open, migrated database. The repository takes ownership of it.
    #[must_use]
    pub fn new(database: Database) -> Self {
        SqliteRepository { database }
    }
}

/// Wrap a rusqlite error with what was being attempted.
fn query_error(action: &str, error: rusqlite::Error) -> CoreError {
    CoreError::Database {
        detail: format!("{action}: {error}"),
    }
}

/// A `ResourceMissing` error naming what was looked up.
fn missing(what: String) -> CoreError {
    CoreError::ResourceMissing {
        path: PathBuf::from(what),
    }
}

/// Decode a stored text column and check it against its sha256, so a
/// corrupt row is reported instead of being typed.
fn decode_checked(codec: &str, body: &[u8], sha: Option<&str>) -> Result<String> {
    let text = decompress_text(codec, body)?;
    if let Some(expected) = sha {
        if sha256_hex(text.as_bytes()) != expected {
            return Err(CoreError::Database {
                detail: "stored text does not match its sha256".to_owned(),
            });
        }
    }
    Ok(text)
}

impl SqliteRepository {
    /// List-card metadata for a product, ordered by number. Reads
    /// `exercises` only, never the text tables.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails or a row is corrupt.
    pub fn list_meta(&self, product: ProductId) -> Result<Vec<ExerciseMeta>> {
        let mut statement = self
            .database
            .connection()
            .prepare_cached(
                "SELECT e.id, e.number, e.title, e.category, e.difficulty, e.status, e.track,
                        e.word_count
                   FROM exercises AS e
                   JOIN products  AS p ON p.id = e.product_id
                  WHERE p.slug = ?1
                  ORDER BY e.number",
            )
            .map_err(|e| query_error("could not prepare the metadata list", e))?;
        let rows = statement
            .query_map([product.slug()], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                ))
            })
            .map_err(|e| query_error("could not list exercise metadata", e))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, number, title, category, difficulty, status, track, words) =
                row.map_err(|e| query_error("could not read a metadata row", e))?;
            out.push(ExerciseMeta {
                id,
                number: narrow(number, "exercise number")?,
                title,
                category,
                difficulty: difficulty.map(|d| narrow(d, "difficulty")).transpose()?,
                status: ContentStatus::parse(&status)?,
                track,
                word_count: words.map(|w| narrow(w, "word count")).transpose()?,
            });
        }
        Ok(out)
    }

    /// List-card metadata for an exam pool, ordered by number. Reads
    /// `exam_passages` only.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails or a row is corrupt.
    pub fn list_exam_pool(&self, pool: ExamPool) -> Result<Vec<ExamPassageMeta>> {
        let mut statement = self
            .database
            .connection()
            .prepare_cached(EXAM_LIST_SQL)
            .map_err(|e| query_error("could not prepare the exam pool list", e))?;
        let rows = statement
            .query_map([pool.slug()], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .map_err(|e| query_error("could not list the exam pool", e))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, number, title, category, difficulty, words, status) =
                row.map_err(|e| query_error("could not read an exam passage row", e))?;
            out.push(ExamPassageMeta {
                id,
                number: narrow(number, "passage number")?,
                title,
                category,
                difficulty: difficulty.map(|d| narrow(d, "difficulty")).transpose()?,
                word_count: narrow(words, "word count")?,
                status: ContentStatus::parse(&status)?,
            });
        }
        Ok(out)
    }

    /// The text of one exam passage: Unicode, or with `kruti_dev` its
    /// generated Kruti Dev legacy key stream.
    ///
    /// # Errors
    ///
    /// [`CoreError::ResourceMissing`] when the passage or that encoding does
    /// not exist; [`CoreError::Database`] for a corrupt row.
    pub fn load_exam_passage(&self, passage_id: i64, kruti_dev: bool) -> Result<String> {
        let encoding = if kruti_dev { "krutidev" } else { "unicode" };
        let row = self
            .database
            .connection()
            .query_row(
                "SELECT codec, body, sha256 FROM exam_passage_text
                  WHERE passage_id = ?1 AND encoding = ?2",
                rusqlite::params![passage_id, encoding],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| query_error("could not load the exam passage", e))?;
        let Some((codec, body, sha)) = row else {
            return Err(missing(format!("exam_passage:{passage_id}/{encoding}")));
        };
        decode_checked(&codec, &body, Some(&sha))
    }

    /// `EXPLAIN QUERY PLAN` of the exam pool list (tests check it never
    /// touches the text table).
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the plan cannot be read.
    pub fn explain_exam_list(&self) -> Result<String> {
        let mut statement = self
            .database
            .connection()
            .prepare(&format!("EXPLAIN QUERY PLAN {EXAM_LIST_SQL}"))
            .map_err(|e| query_error("could not explain the exam pool list", e))?;
        let rows = statement
            .query_map(["exam-pool-english"], |r| r.get::<_, String>(3))
            .map_err(|e| query_error("could not explain the exam pool list", e))?;
        let mut plan = Vec::new();
        for row in rows {
            plan.push(row.map_err(|e| query_error("could not read the plan", e))?);
        }
        Ok(plan.join("\n"))
    }
}

/// One exercise's progress, for an Exercise-screen card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardProgress {
    /// The exercise.
    pub exercise_id: i64,
    /// Best net WPM, when a best is recorded.
    pub best_net_wpm: Option<f64>,
    /// Best accuracy, when a best is recorded.
    pub best_accuracy: Option<f64>,
    /// Completed at least once.
    pub completed: bool,
}

impl SqliteRepository {
    /// Bests and completion of every practised exercise of a product, in one
    /// query (the grid shows them on every card; one query per card would be
    /// 200+ round trips). Exercises without a best or progress row are
    /// omitted.
    ///
    /// # Errors
    ///
    /// [`CoreError::Database`] if the query fails.
    pub fn card_progress(&self, profile_id: i64, product: ProductId) -> Result<Vec<CardProgress>> {
        let mut statement = self
            .database
            .connection()
            .prepare_cached(
                "SELECT e.id, b.best_net_wpm, b.best_accuracy, COALESCE(pr.completed, 0)
                   FROM exercises AS e
                   JOIN products  AS p ON p.id = e.product_id
                   LEFT JOIN personal_bests    AS b
                          ON b.exercise_id = e.id AND b.profile_id = ?1
                   LEFT JOIN exercise_progress AS pr
                          ON pr.exercise_id = e.id AND pr.profile_id = ?1
                  WHERE p.slug = ?2
                    AND (b.exercise_id IS NOT NULL OR pr.exercise_id IS NOT NULL)",
            )
            .map_err(|e| query_error("could not prepare the card progress query", e))?;
        let rows = statement
            .query_map(rusqlite::params![profile_id, product.slug()], |r| {
                Ok(CardProgress {
                    exercise_id: r.get(0)?,
                    best_net_wpm: r.get(1)?,
                    best_accuracy: r.get(2)?,
                    completed: r.get(3)?,
                })
            })
            .map_err(|e| query_error("could not read card progress", e))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| query_error("could not read a card progress row", e))?);
        }
        Ok(out)
    }
}

const EXAM_LIST_SQL: &str = "SELECT e.id, e.number, e.title, e.category, e.difficulty,
        e.word_count, e.status
   FROM exam_passages AS e
   JOIN exam_pools    AS p ON p.id = e.pool_id
  WHERE p.slug = ?1
  ORDER BY e.number";

/// Narrow a stored integer, treating an out-of-range value as a corrupt row
/// rather than silently truncating it.
fn narrow<T: TryFrom<i64>>(value: i64, what: &str) -> Result<T> {
    T::try_from(value).map_err(|_| CoreError::Database {
        detail: format!("{what} out of range: {value}"),
    })
}

impl ExerciseRepository for SqliteRepository {
    fn list_for_product(&self, product: ProductId) -> Result<Vec<Exercise>> {
        let mut statement = self
            .database
            .connection()
            .prepare(
                "SELECT e.id, e.number, e.title, e.category, e.difficulty
                   FROM exercises AS e
                   JOIN products  AS p ON p.id = e.product_id
                  WHERE p.slug = ?1
                  ORDER BY e.number",
            )
            .map_err(|e| query_error("could not prepare the exercise list query", e))?;

        let rows = statement
            .query_map([product.slug()], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(|e| query_error("could not list exercises", e))?;

        let mut exercises = Vec::new();
        for row in rows {
            let (id, number, title, category, difficulty) =
                row.map_err(|e| query_error("could not read an exercise row", e))?;
            exercises.push(Exercise {
                id,
                product,
                number: narrow(number, "exercise number")?,
                title,
                category,
                difficulty: difficulty.map(|d| narrow(d, "difficulty")).transpose()?,
                // Deliberately not loaded; see the module docs.
                content: String::new(),
            });
        }

        Ok(exercises)
    }

    fn load(&self, exercise_id: i64) -> Result<Exercise> {
        let row = self
            .database
            .connection()
            .query_row(
                "SELECT p.slug, e.number, e.title, c.content, e.category, e.difficulty,
                        t.codec, t.body, t.sha256
                   FROM exercises             AS e
                   JOIN products              AS p ON p.id = e.product_id
                   LEFT JOIN exercise_text    AS t ON t.exercise_id = e.id
                   LEFT JOIN exercise_content AS c ON c.exercise_id = e.id
                  WHERE e.id = ?1",
                [exercise_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, Option<Vec<u8>>>(7)?,
                        r.get::<_, Option<String>>(8)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| query_error("could not load the exercise", e))?;

        let Some((slug, number, title, plain, category, difficulty, codec, body, sha)) = row else {
            return Err(missing(format!("exercise:{exercise_id}")));
        };
        // Compressed text (P04) first; the plain 0002 column for rows not
        // re-imported yet.
        let content = match (codec, body) {
            (Some(codec), Some(body)) => Some(decode_checked(&codec, &body, sha.as_deref())?),
            _ => plain,
        };

        // Metadata without text cannot be typed, so it is as missing as no row at
        // all. It is named separately because it points at an import problem
        // rather than a bad id.
        let Some(content) = content else {
            return Err(missing(format!("exercise:{exercise_id}/content")));
        };

        Ok(Exercise {
            id: exercise_id,
            product: ProductId::from_slug(&slug)?,
            number: narrow(number, "exercise number")?,
            title,
            category,
            difficulty: difficulty.map(|d| narrow(d, "difficulty")).transpose()?,
            content,
        })
    }
}

impl ProgressRepository for SqliteRepository {
    fn exercise_best(&self, profile_id: i64, exercise_id: i64) -> Result<Option<PersonalBest>> {
        self.database
            .connection()
            .query_row(
                "SELECT best_wpm, best_net_wpm, best_accuracy, best_consistency, best_score
                   FROM personal_bests
                  WHERE profile_id = ?1 AND exercise_id = ?2",
                [profile_id, exercise_id],
                read_personal_best,
            )
            .optional()
            .map_err(|e| query_error("could not read the exercise best", e))
    }

    fn product_best(&self, profile_id: i64, product: ProductId) -> Result<Option<PersonalBest>> {
        self.database
            .connection()
            .query_row(
                "SELECT b.best_wpm, b.best_net_wpm, b.best_accuracy,
                        b.best_consistency, b.best_score
                   FROM personal_bests AS b
                   JOIN products       AS p ON p.id = b.product_id
                  WHERE b.profile_id = ?1 AND p.slug = ?2 AND b.exercise_id IS NULL",
                rusqlite::params![profile_id, product.slug()],
                read_personal_best,
            )
            .optional()
            .map_err(|e| query_error("could not read the product best", e))
    }

    fn exercise_progress(
        &self,
        profile_id: i64,
        exercise_id: i64,
    ) -> Result<Option<ExerciseProgress>> {
        let row = self
            .database
            .connection()
            .query_row(
                "SELECT attempts, completed, total_practice_secs
                   FROM exercise_progress
                  WHERE profile_id = ?1 AND exercise_id = ?2",
                [profile_id, exercise_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, bool>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| query_error("could not read exercise progress", e))?;

        let Some((attempts, completed, practice_secs)) = row else {
            return Ok(None);
        };

        Ok(Some(ExerciseProgress {
            attempts: narrow(attempts, "attempt count")?,
            completed,
            total_practice_seconds: narrow(practice_secs, "practice time")?,
        }))
    }
}

/// Map a personal-best row, in the column order both best queries select.
fn read_personal_best(r: &rusqlite::Row<'_>) -> rusqlite::Result<PersonalBest> {
    Ok(PersonalBest {
        wpm: r.get(0)?,
        net_wpm: r.get(1)?,
        accuracy: r.get(2)?,
        consistency: r.get(3)?,
        score: r.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convert a rusqlite error for use inside `Database::transaction`.
    fn sql(error: rusqlite::Error) -> CoreError {
        CoreError::Database {
            detail: error.to_string(),
        }
    }

    /// Synthetic placeholder rows, clearly not real curriculum content. Inserted
    /// out of order so the list test proves ORDER BY number, not insertion order.
    const PLACEHOLDERS: [(u32, &str, &str); 3] = [
        (
            2,
            "Placeholder Exercise 2",
            "test exercise content — placeholder 2",
        ),
        (
            1,
            "Placeholder Exercise 1",
            "test exercise content — placeholder 1",
        ),
        (
            3,
            "Placeholder Exercise 3",
            "test exercise content — placeholder 3",
        ),
    ];

    /// An in-memory repository holding the placeholder exercises under English
    /// Classic.
    fn seeded() -> SqliteRepository {
        let mut database = Database::open_in_memory().expect("in-memory database");
        database
            .transaction(|tx| {
                for (number, title, content) in PLACEHOLDERS {
                    tx.execute(
                        "INSERT INTO exercises (product_id, number, title)
                         VALUES ((SELECT id FROM products WHERE slug = ?1), ?2, ?3)",
                        rusqlite::params![ProductId::EnglishClassic.slug(), number, title],
                    )
                    .map_err(sql)?;
                    tx.execute(
                        "INSERT INTO exercise_content (exercise_id, content) VALUES (?1, ?2)",
                        rusqlite::params![tx.last_insert_rowid(), content],
                    )
                    .map_err(sql)?;
                }
                Ok(())
            })
            .expect("placeholder rows should insert");
        SqliteRepository::new(database)
    }

    #[test]
    fn list_returns_the_products_exercises_in_number_order() {
        let exercises = seeded()
            .list_for_product(ProductId::EnglishClassic)
            .expect("list should succeed");

        let numbers: Vec<u32> = exercises.iter().map(|e| e.number).collect();
        assert_eq!(numbers, [1, 2, 3], "ordered by number, not insertion order");
        assert_eq!(exercises[0].title, "Placeholder Exercise 1");
        assert!(exercises
            .iter()
            .all(|e| e.product == ProductId::EnglishClassic));
        assert!(
            exercises.iter().all(|e| e.content.is_empty()),
            "the list query must not load exercise content"
        );
    }

    #[test]
    fn list_for_a_product_without_exercises_is_empty_not_an_error() {
        // The real state of every product until content is imported.
        let exercises = seeded()
            .list_for_product(ProductId::TypingFlow)
            .expect("list should succeed");
        assert!(exercises.is_empty());
    }

    #[test]
    fn load_returns_the_full_content() {
        let repository = seeded();
        let second = repository
            .list_for_product(ProductId::EnglishClassic)
            .unwrap()
            .into_iter()
            .find(|e| e.number == 2)
            .expect("placeholder exercise 2 exists");

        let loaded = repository.load(second.id).expect("load should succeed");
        assert_eq!(loaded.id, second.id);
        assert_eq!(loaded.product, ProductId::EnglishClassic);
        assert_eq!(loaded.number, 2);
        assert_eq!(loaded.title, "Placeholder Exercise 2");
        assert_eq!(loaded.content, "test exercise content — placeholder 2");
    }

    #[test]
    fn load_with_an_unknown_id_is_resource_missing() {
        let error = seeded().load(9_999).unwrap_err();
        assert!(
            matches!(error, CoreError::ResourceMissing { .. }),
            "got: {error}"
        );
        assert!(error.is_recoverable(), "a bad id must be survivable");
    }

    #[test]
    fn load_with_metadata_but_no_content_is_resource_missing() {
        let mut database = Database::open_in_memory().unwrap();
        let id = database
            .transaction(|tx| {
                tx.execute(
                    "INSERT INTO exercises (product_id, number, title)
                     VALUES ((SELECT id FROM products WHERE slug = ?1), 1, 'Placeholder without content')",
                    [ProductId::TypingFlow.slug()],
                )
                .map_err(sql)?;
                Ok(tx.last_insert_rowid())
            })
            .unwrap();

        let error = SqliteRepository::new(database).load(id).unwrap_err();
        assert!(
            matches!(error, CoreError::ResourceMissing { .. }),
            "metadata without content should be missing, got: {error}"
        );
    }

    #[test]
    fn progress_queries_on_a_fresh_database_return_none_not_an_error() {
        // The common case on a new install: no sessions yet. "No best yet" is a
        // normal state, not a failure.
        let repository = SqliteRepository::new(Database::open_in_memory().unwrap());

        assert_eq!(
            repository
                .exercise_best(1, 1)
                .expect("query should succeed"),
            None
        );
        assert_eq!(
            repository
                .product_best(1, ProductId::EnglishClassic)
                .expect("query should succeed"),
            None
        );
        assert_eq!(
            repository
                .exercise_progress(1, 1)
                .expect("query should succeed"),
            None
        );
    }

    #[test]
    fn progress_queries_return_stored_rows() {
        // Guards against an implementation that always returns None, which would
        // pass the fresh-database test above. All rows are synthetic fixtures.
        let mut database = Database::open_in_memory().unwrap();
        let exercise_id = database
            .transaction(|tx| {
                tx.execute(
                    "INSERT INTO profiles (id, display_name) VALUES (1, 'Test Profile')",
                    [],
                )
                .map_err(sql)?;
                tx.execute(
                    "INSERT INTO exercises (product_id, number, title)
                     VALUES ((SELECT id FROM products WHERE slug = ?1), 1, 'Placeholder Exercise 1')",
                    [ProductId::EnglishClassic.slug()],
                )
                .map_err(sql)?;
                let exercise_id = tx.last_insert_rowid();

                // Per-exercise best.
                tx.execute(
                    "INSERT INTO personal_bests
                         (profile_id, product_id, exercise_id, best_wpm, best_net_wpm, best_accuracy)
                     VALUES (1, (SELECT product_id FROM exercises WHERE id = ?1), ?1, 52.5, 50.0, 97.5)",
                    [exercise_id],
                )
                .map_err(sql)?;
                // Product-level best: exercise_id is NULL.
                tx.execute(
                    "INSERT INTO personal_bests
                         (profile_id, product_id, exercise_id, best_wpm, best_net_wpm, best_accuracy)
                     VALUES (1, (SELECT id FROM products WHERE slug = ?1), NULL, 60.0, 58.0, 98.0)",
                    [ProductId::EnglishClassic.slug()],
                )
                .map_err(sql)?;
                tx.execute(
                    "INSERT INTO exercise_progress
                         (profile_id, exercise_id, attempts, completed, total_practice_secs)
                     VALUES (1, ?1, 3, 1, 360)",
                    [exercise_id],
                )
                .map_err(sql)?;
                Ok(exercise_id)
            })
            .expect("fixture rows should insert");
        let repository = SqliteRepository::new(database);

        assert_eq!(
            repository.exercise_best(1, exercise_id).unwrap(),
            Some(PersonalBest {
                wpm: 52.5,
                net_wpm: 50.0,
                accuracy: 97.5,
                consistency: None,
                score: None,
            })
        );
        assert_eq!(
            repository
                .product_best(1, ProductId::EnglishClassic)
                .unwrap(),
            Some(PersonalBest {
                wpm: 60.0,
                net_wpm: 58.0,
                accuracy: 98.0,
                consistency: None,
                score: None,
            })
        );
        assert_eq!(
            repository.exercise_progress(1, exercise_id).unwrap(),
            Some(ExerciseProgress {
                attempts: 3,
                completed: true,
                total_practice_seconds: 360,
            })
        );

        // Scoped correctly: another product and another profile have nothing.
        assert_eq!(
            repository.product_best(1, ProductId::TypingFlow).unwrap(),
            None
        );
        assert_eq!(repository.exercise_best(2, exercise_id).unwrap(), None);

        // The grid's batch query sees the same rows, scoped the same way.
        assert_eq!(
            repository
                .card_progress(1, ProductId::EnglishClassic)
                .unwrap(),
            vec![CardProgress {
                exercise_id,
                best_net_wpm: Some(50.0),
                best_accuracy: Some(97.5),
                completed: true,
            }]
        );
        assert!(repository
            .card_progress(2, ProductId::EnglishClassic)
            .unwrap()
            .is_empty());
        assert!(repository
            .card_progress(1, ProductId::TypingFlow)
            .unwrap()
            .is_empty());
    }
}
