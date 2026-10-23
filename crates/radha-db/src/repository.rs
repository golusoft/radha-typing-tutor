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

use radha_content::{
    Exercise, ExerciseProgress, ExerciseRepository, PersonalBest, ProductId, ProgressRepository,
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
                "SELECT e.id, e.number, e.title
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
                ))
            })
            .map_err(|e| query_error("could not list exercises", e))?;

        let mut exercises = Vec::new();
        for row in rows {
            let (id, number, title) =
                row.map_err(|e| query_error("could not read an exercise row", e))?;
            exercises.push(Exercise {
                id,
                product,
                number: narrow(number, "exercise number")?,
                title,
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
                "SELECT p.slug, e.number, e.title, c.content
                   FROM exercises             AS e
                   JOIN products              AS p ON p.id = e.product_id
                   LEFT JOIN exercise_content AS c ON c.exercise_id = e.id
                  WHERE e.id = ?1",
                [exercise_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| query_error("could not load the exercise", e))?;

        let Some((slug, number, title, content)) = row else {
            return Err(missing(format!("exercise:{exercise_id}")));
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
        (2, "Placeholder Exercise 2", "test exercise content — placeholder 2"),
        (1, "Placeholder Exercise 1", "test exercise content — placeholder 1"),
        (3, "Placeholder Exercise 3", "test exercise content — placeholder 3"),
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
            repository.exercise_best(1, 1).expect("query should succeed"),
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
            repository.product_best(1, ProductId::EnglishClassic).unwrap(),
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
    }
}
