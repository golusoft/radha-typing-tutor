//! # Radha DB
//!
//! SQLite persistence for Radha Typing Tutor, via `rusqlite` with the bundled
//! SQLite amalgamation (no system SQLite dependency, so the Windows installer
//! stays self-contained).
//!
//! ## Design constraints this crate is built around
//!
//! **RAM-first sessions.** `RULES.MD` Rule 14 forbids per-keystroke writes. This
//! crate therefore exposes no incremental "record keystroke" API at all. The
//! intended shape is a single batched, transactional write of a completed
//! session — see [`Database::transaction`]. The absence of a per-keystroke entry
//! point is the enforcement mechanism.
//!
//! **Synchronous.** No async runtime. Typing never blocks on I/O because typing
//! never touches I/O; everything else (loading an exercise, saving a result)
//! happens between screens where a few milliseconds are imperceptible.
//!
//! **Errors are typed.** `rusqlite::Error` is converted at this crate's boundary
//! so it never appears in another crate's public API.
//!
//! ## Contents
//!
//! - [`Database`]: connection management and the migration runner.
//! - [`migrations`]: the versioned schema. `0002_core_schema.sql` holds the
//!   product tables (profiles, exercises, sessions, bests, analytics, settings).
//! - [`repository`]: read-only implementations of the `radha-content` traits.

use std::path::{Path, PathBuf};

use radha_core::{CoreError, Result};
use rusqlite::Connection;

pub mod migrations;
pub mod repository;

pub use repository::SqliteRepository;

/// An open connection to the Radha database.
///
/// Holds a single `rusqlite::Connection`. One connection is sufficient and
/// preferred: the app is single-user and single-process, and a pool would add
/// memory overhead for no benefit on the 2GB-RAM target from `PRD.md` Section 3.
pub struct Database {
    connection: Connection,
    path: Option<PathBuf>,
}

impl Database {
    /// Open (or create) the database at `path` and bring the schema up to date.
    ///
    /// Applies any pending migrations before returning, so callers always
    /// receive a ready-to-use database.
    ///
    /// # Errors
    ///
    /// - [`CoreError::Database`] if the file cannot be opened.
    /// - [`CoreError::Migration`] if a migration fails to apply.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        // Make sure the parent directory exists — on first launch the app-data
        // folder may not have been created yet.
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
            }
        }

        let connection = Connection::open(path).map_err(|e| CoreError::Database {
            detail: format!("could not open {}: {e}", path.display()),
        })?;

        let mut db = Database {
            connection,
            path: Some(path.to_path_buf()),
        };
        db.configure()?;
        db.run_migrations()?;
        Ok(db)
    }

    /// Open a temporary in-memory database, fully migrated.
    ///
    /// For tests only — nothing is persisted.
    ///
    /// # Errors
    ///
    /// Mirrors [`Database::open`].
    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory().map_err(|e| CoreError::Database {
            detail: format!("could not open in-memory database: {e}"),
        })?;

        let mut db = Database {
            connection,
            path: None,
        };
        db.configure()?;
        db.run_migrations()?;
        Ok(db)
    }

    /// Apply connection-level settings.
    ///
    /// Chosen for the low-end-PC target:
    ///
    /// - `journal_mode = WAL` — readers don't block the writer, so saving a
    ///   result never stalls the UI thread.
    /// - `synchronous = NORMAL` — avoids an fsync per commit. Safe under WAL
    ///   (a crash can lose at most the last transaction, not corrupt the file),
    ///   and meaningfully faster on the slow spinning disks common on the target
    ///   hardware.
    /// - `foreign_keys = ON` — SQLite defaults this *off*; the schema relies on
    ///   referential integrity, so it must be enabled per connection.
    fn configure(&self) -> Result<()> {
        self.connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| CoreError::Database {
                detail: format!("could not enable WAL: {e}"),
            })?;

        self.connection
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| CoreError::Database {
                detail: format!("could not set synchronous mode: {e}"),
            })?;

        self.connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| CoreError::Database {
                detail: format!("could not enable foreign keys: {e}"),
            })?;

        Ok(())
    }

    /// Bring the schema up to the latest version.
    fn run_migrations(&mut self) -> Result<()> {
        let applied = migrations::run(&mut self.connection)?;
        log::info!(
            "Migrations applied: {applied} (schema version {})",
            self.schema_version()?
        );
        Ok(())
    }

    /// The highest applied migration version, or `0` on a fresh database.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Database`] if the tracking table cannot be read.
    pub fn schema_version(&self) -> Result<i64> {
        migrations::current_version(&self.connection)
    }

    /// Path this database was opened from, or `None` for in-memory databases.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Borrow the underlying connection for reads.
    ///
    /// Deliberately not public: keeping `rusqlite` types inside this crate means
    /// callers cannot accidentally run an unbatched write from the UI layer.
    ///
    /// Used by [`crate::repository`] for its read queries.
    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Run `f` inside a single transaction, committing if it succeeds.
    ///
    /// This is the **only** write path this crate offers, which is what makes
    /// the RAM-first rule structural: persisting a session is one call, one
    /// transaction, one commit — there is no API through which a caller could
    /// write incrementally while typing.
    ///
    /// # Errors
    ///
    /// Returns whatever `f` returns. On error the transaction is rolled back, so
    /// a failed save never leaves a partially written session.
    pub fn transaction<T, F>(&mut self, f: F) -> Result<T>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    {
        let tx = self
            .connection
            .transaction()
            .map_err(|e| CoreError::Database {
                detail: format!("could not begin transaction: {e}"),
            })?;

        let value = f(&tx)?;

        tx.commit().map_err(|e| CoreError::Database {
            detail: format!("could not commit transaction: {e}"),
        })?;

        Ok(value)
    }
}

impl std::fmt::Debug for Database {
    // Hand-written because rusqlite::Connection isn't Debug.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field(
                "path",
                &self
                    .path
                    .as_ref()
                    .map_or("<in-memory>".to_string(), |p| p.display().to_string()),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_database_opens_and_migrates() {
        let db = Database::open_in_memory().expect("in-memory db should open");
        assert!(
            db.schema_version().expect("version readable") >= 1,
            "migrations should have been applied on open"
        );
        assert!(db.path().is_none(), "in-memory db has no path");
    }

    #[test]
    fn foreign_keys_are_enabled() {
        // SQLite defaults this off; the schema depends on it being on.
        let db = Database::open_in_memory().unwrap();
        let enabled: i64 = db
            .connection()
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(enabled, 1, "foreign key enforcement must be on");
    }

    #[test]
    fn opens_a_file_and_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        // Deliberately nest so create_dir_all is exercised.
        let path = dir.path().join("nested").join("radha.sqlite");

        let db = Database::open(&path).expect("file db should open");
        assert_eq!(db.path(), Some(path.as_path()));
        assert!(path.exists(), "database file should have been created");
    }

    #[test]
    fn reopening_a_database_does_not_reapply_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radha.sqlite");

        let first = Database::open(&path).unwrap();
        let version_after_first = first.schema_version().unwrap();
        drop(first);

        let second = Database::open(&path).unwrap();
        assert_eq!(
            second.schema_version().unwrap(),
            version_after_first,
            "schema version must be stable across reopens"
        );
    }

    #[test]
    fn transaction_commits_on_success() {
        let mut db = Database::open_in_memory().unwrap();

        db.transaction(|tx| {
            tx.execute_batch("CREATE TABLE t (v INTEGER); INSERT INTO t VALUES (7);")
                .map_err(|e| CoreError::Database {
                    detail: e.to_string(),
                })
        })
        .expect("transaction should succeed");

        let v: i64 = db
            .connection()
            .query_row("SELECT v FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 7);
    }

    #[test]
    fn transaction_rolls_back_on_error() {
        let mut db = Database::open_in_memory().unwrap();

        db.transaction(|tx| {
            tx.execute_batch("CREATE TABLE t (v INTEGER);")
                .map_err(|e| CoreError::Database {
                    detail: e.to_string(),
                })
        })
        .unwrap();

        // Insert then fail: the insert must not survive.
        let outcome: Result<()> = db.transaction(|tx| {
            tx.execute("INSERT INTO t VALUES (99)", [])
                .map_err(|e| CoreError::Database {
                    detail: e.to_string(),
                })?;
            Err(CoreError::Database {
                detail: "simulated failure after write".into(),
            })
        });

        assert!(outcome.is_err(), "transaction should have failed");

        let count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 0,
            "a failed transaction must not leave partial writes — this is what \
             protects a session save from being half-persisted"
        );
    }
}
