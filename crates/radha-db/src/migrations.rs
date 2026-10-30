//! Versioned schema migrations.
//!
//! ## How it works
//!
//! Each migration is a `.sql` file in `migrations/` at the workspace root, named
//! `NNNN_description.sql`. The numeric prefix is its version. On open, the runner
//! applies every migration whose version exceeds the highest already recorded in
//! `schema_migrations`, in ascending order, each inside its own transaction.
//!
//! ## Why migrations are embedded, not read from disk
//!
//! [`MIGRATIONS`] uses `include_str!`, so the SQL is compiled into the binary.
//! Two reasons:
//!
//! 1. **Shipping.** The Windows installer stays a single executable — there is no
//!    `migrations/` folder to ship, and no way for a user to delete or edit the
//!    SQL and break their database.
//! 2. **Correctness.** A migration can never be missing at runtime. Adding a
//!    file without registering it is a compile-time-visible omission rather than
//!    a first-launch failure on a user's machine.
//!
//! The tradeoff is that adding a migration requires editing this list. That is
//! intentional friction: registering a migration should be a deliberate act.

use radha_core::{CoreError, Result};
use rusqlite::Connection;

/// A single migration: version, name, and SQL.
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

/// All known migrations, ascending by version.
///
/// **When adding a migration:** create `migrations/NNNN_name.sql`, then append an
/// entry here. Never edit or renumber a migration that has shipped — users will
/// already have it recorded as applied, so a change would silently never run.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_init.sql",
        sql: include_str!("../../../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "0002_core_schema.sql",
        sql: include_str!("../../../migrations/0002_core_schema.sql"),
    },
    Migration {
        version: 3,
        name: "0003_content_v2.sql",
        sql: include_str!("../../../migrations/0003_content_v2.sql"),
    },
    Migration {
        version: 4,
        name: "0004_flow_rhythm.sql",
        sql: include_str!("../../../migrations/0004_flow_rhythm.sql"),
    },
    Migration {
        version: 5,
        name: "0005_inscript.sql",
        sql: include_str!("../../../migrations/0005_inscript.sql"),
    },
];

/// The highest applied migration version, or `0` on a fresh database.
///
/// # Errors
///
/// Returns [`CoreError::Database`] if the tracking table exists but cannot be
/// queried.
pub fn current_version(connection: &Connection) -> Result<i64> {
    // A fresh database has no tracking table yet, which is version 0 rather than
    // an error.
    let table_exists: bool = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(|e| CoreError::Database {
            detail: format!("could not inspect schema: {e}"),
        })?;

    if !table_exists {
        return Ok(0);
    }

    connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .map_err(|e| CoreError::Database {
            detail: format!("could not read schema version: {e}"),
        })
}

/// Apply all pending migrations.
///
/// Returns how many were applied — `0` means the schema was already current.
///
/// # Errors
///
/// Returns [`CoreError::Migration`] naming the migration that failed. Each
/// migration runs in its own transaction, so a failure leaves the database at the
/// last successfully applied version rather than in a half-migrated state.
pub fn run(connection: &mut Connection) -> Result<usize> {
    let starting_version = current_version(connection)?;
    let mut applied = 0usize;

    for migration in MIGRATIONS {
        if migration.version <= starting_version {
            continue;
        }

        log::info!(
            "Applying migration {} ({})",
            migration.version,
            migration.name
        );

        let tx = connection.transaction().map_err(|e| CoreError::Migration {
            version: migration.version,
            name: migration.name.to_owned(),
            detail: format!("could not begin transaction: {e}"),
        })?;

        tx.execute_batch(migration.sql)
            .map_err(|e| CoreError::Migration {
                version: migration.version,
                name: migration.name.to_owned(),
                detail: e.to_string(),
            })?;

        // Migration 0001 creates the tracking table itself, so this insert is
        // valid from the very first migration onward.
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations (version, name) VALUES (?1, ?2)",
            rusqlite::params![migration.version, migration.name],
        )
        .map_err(|e| CoreError::Migration {
            version: migration.version,
            name: migration.name.to_owned(),
            detail: format!("could not record migration: {e}"),
        })?;

        tx.commit().map_err(|e| CoreError::Migration {
            version: migration.version,
            name: migration.name.to_owned(),
            detail: format!("could not commit: {e}"),
        })?;

        applied += 1;
    }

    Ok(applied)
}

/// The latest version this build knows about.
#[must_use]
pub fn latest_version() -> i64 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use radha_content::ProductId;
    use radha_layouts::LayoutId;

    /// The thirteen tables TASK.MD Section 5 specifies, created by 0002.
    const CORE_TABLES: [&str; 13] = [
        "profiles",
        "products",
        "layouts",
        "exercises",
        "exercise_content",
        "exercise_progress",
        "typing_sessions",
        "session_metrics",
        "typing_errors",
        "personal_bests",
        "analytics",
        "settings",
        "guide_settings",
    ];

    fn fresh() -> Connection {
        Connection::open_in_memory().expect("in-memory connection")
    }

    #[test]
    fn migration_list_is_ascending_and_unique() {
        // Guards against a copy-paste mistake when registering a new migration.
        let mut previous = 0i64;
        for m in MIGRATIONS {
            assert!(
                m.version > previous,
                "migration versions must ascend and not repeat; {} followed {}",
                m.version,
                previous
            );
            previous = m.version;
        }
    }

    #[test]
    fn migrations_are_not_empty() {
        // Catches a broken include_str! path, which would otherwise only show up
        // as a mysteriously missing table at runtime.
        for m in MIGRATIONS {
            assert!(
                !m.sql.trim().is_empty(),
                "migration {} ({}) has empty SQL",
                m.version,
                m.name
            );
        }
    }

    #[test]
    fn fresh_database_reports_version_zero() {
        let conn = fresh();
        assert_eq!(current_version(&conn).unwrap(), 0);
    }

    #[test]
    fn run_applies_all_migrations_and_creates_tracking_table() {
        let mut conn = fresh();
        let applied = run(&mut conn).expect("migrations should apply");

        assert_eq!(applied, MIGRATIONS.len(), "all migrations should run");
        assert_eq!(current_version(&conn).unwrap(), latest_version());

        // The Task 5 demo criterion: schema_migrations exists and is populated.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .expect("tracking table should exist");
        assert_eq!(count as usize, MIGRATIONS.len());
    }

    #[test]
    fn migration_records_its_name() {
        let mut conn = fresh();
        run(&mut conn).unwrap();

        let name: String = conn
            .query_row(
                "SELECT name FROM schema_migrations WHERE version = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "0001_init.sql");
    }

    #[test]
    fn run_is_idempotent() {
        let mut conn = fresh();
        let first = run(&mut conn).unwrap();
        let second = run(&mut conn).unwrap();

        assert_eq!(first, MIGRATIONS.len());
        assert_eq!(
            second, 0,
            "a second run must apply nothing — reopening the app should never \
             re-run migrations"
        );
    }

    #[test]
    fn applied_at_is_recorded() {
        let mut conn = fresh();
        run(&mut conn).unwrap();

        let applied_at: String = conn
            .query_row(
                "SELECT applied_at FROM schema_migrations WHERE version = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !applied_at.is_empty(),
            "applied_at should be defaulted by SQLite"
        );
    }

    #[test]
    fn fresh_database_applies_every_migration_and_ends_at_the_latest_version() {
        let mut conn = fresh();
        run(&mut conn).unwrap();

        let mut statement = conn
            .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
            .unwrap();
        let recorded: Vec<(i64, String)> = statement
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();

        assert_eq!(
            recorded,
            vec![
                (1, "0001_init.sql".to_owned()),
                (2, "0002_core_schema.sql".to_owned()),
                (3, "0003_content_v2.sql".to_owned()),
                (4, "0004_flow_rhythm.sql".to_owned()),
                (5, "0005_inscript.sql".to_owned()),
            ]
        );
        assert_eq!(current_version(&conn).unwrap(), 5);
    }

    /// Apply migrations up to `version` only (simulates an older install).
    fn run_up_to(conn: &mut Connection, version: i64) {
        for m in MIGRATIONS.iter().filter(|m| m.version <= version) {
            let tx = conn.transaction().unwrap();
            tx.execute_batch(m.sql).unwrap();
            tx.execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                rusqlite::params![m.version, m.name],
            )
            .unwrap();
            tx.commit().unwrap();
        }
    }

    #[test]
    fn content_v2_upgrades_a_version_two_database_without_losing_rows() {
        let mut conn = fresh();
        run_up_to(&mut conn, 2);
        conn.execute_batch(
            "INSERT INTO profiles (id, display_name) VALUES (1, 'Test Profile');
             INSERT INTO exercises (id, product_id, number, title)
                 VALUES (7, (SELECT id FROM products WHERE slug = 'english-classic'), 1, 'Old');
             INSERT INTO exercise_content (exercise_id, content) VALUES (7, 'old text');
             INSERT INTO typing_sessions (profile_id, exercise_id, started_at, completed_at,
                                          elapsed_secs)
                 VALUES (1, 7, '2026-01-01T00:00:00Z', '2026-01-01T00:01:00Z', 60);",
        )
        .unwrap();

        assert_eq!(
            run(&mut conn).unwrap(),
            3,
            "only 0003, 0004 and 0005 are pending"
        );
        assert_eq!(current_version(&conn).unwrap(), 5);

        // Old rows survive with honest defaults.
        let (status, track, content): (String, Option<String>, String) = conn
            .query_row(
                "SELECT e.status, e.track, c.content FROM exercises e
                   JOIN exercise_content c ON c.exercise_id = e.id WHERE e.id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "provisional");
        assert_eq!(track, None);
        assert_eq!(content, "old text");
        let mode: Option<String> = conn
            .query_row("SELECT mode FROM typing_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, None, "pre-0003 sessions have no mode");

        // New tables exist; the two pools are seeded.
        for table in [
            "exercise_text",
            "exam_pools",
            "exam_passages",
            "exam_passage_text",
            "exam_profile_pools",
            "content_provenance",
            "content_imports",
        ] {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "{table}");
        }
        let pools: i64 = conn
            .query_row("SELECT COUNT(*) FROM exam_pools", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pools, 2);
        // The new constraints hold.
        assert!(conn
            .execute("UPDATE exercises SET status = 'official' WHERE id = 7", [])
            .is_err());
        assert!(conn
            .execute("UPDATE typing_sessions SET mode = 'race'", [])
            .is_err());
        conn.execute("UPDATE typing_sessions SET mode = 'test'", [])
            .unwrap();
    }

    #[test]
    fn flow_rhythm_upgrades_a_version_three_database_without_losing_rows() {
        let mut conn = fresh();
        run_up_to(&mut conn, 3);
        conn.execute_batch(
            "INSERT INTO profiles (id, display_name) VALUES (1, 'Test Profile');
             INSERT INTO exercises (id, product_id, number, title)
                 VALUES (7, (SELECT id FROM products WHERE slug = 'typing-flow'), 1, 'Old');
             INSERT INTO typing_sessions (id, profile_id, exercise_id, started_at, completed_at,
                                          elapsed_secs, mode)
                 VALUES (3, 1, 7, '2026-01-01T00:00:00Z', '2026-01-01T00:01:00Z', 60, 'flow');",
        )
        .unwrap();
        assert_eq!(run(&mut conn).unwrap(), 2, "only 0004 and 0005 are pending");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        assert_eq!(current_version(&conn).unwrap(), 5);
        let sessions: i64 = conn
            .query_row("SELECT COUNT(*) FROM typing_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sessions, 1, "old sessions survive");
        // Old sessions simply have no rhythm row; the constraints hold.
        conn.execute(
            "INSERT INTO flow_rhythm (session_id, engine_schema, smoothness) VALUES (3, 1, 94.5)",
            [],
        )
        .unwrap();
        assert!(conn
            .execute(
                "UPDATE flow_rhythm SET smoothness = 120 WHERE session_id = 3",
                []
            )
            .is_err());
        assert!(conn
            .execute(
                "INSERT INTO flow_rhythm (session_id, engine_schema) VALUES (99, 1)",
                []
            )
            .is_err());
        conn.execute(
            "INSERT INTO flow_rhythm (session_id, engine_schema) VALUES (3, 1)
             ON CONFLICT (session_id) DO NOTHING",
            [],
        )
        .unwrap();
    }

    #[test]
    fn core_schema_creates_all_thirteen_tables() {
        let mut conn = fresh();
        run(&mut conn).unwrap();

        for table in CORE_TABLES {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "table `{table}` should exist after migration");
        }
    }

    #[test]
    fn layout_seed_matches_the_rust_layout_enum() {
        let mut conn = fresh();
        run(&mut conn).unwrap();

        for &layout in LayoutId::all() {
            let display_name: String = conn
                .query_row(
                    "SELECT display_name FROM layouts WHERE slug = ?1",
                    [layout.as_str()],
                    |r| r.get(0),
                )
                .unwrap_or_else(|e| panic!("layout `{}` should be seeded: {e}", layout.as_str()));
            assert_eq!(display_name, layout.display_name());
        }

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM layouts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count as usize,
            LayoutId::all().len(),
            "no layouts beyond the Rust enum"
        );
    }

    #[test]
    fn product_seed_matches_the_rust_product_enum() {
        let mut conn = fresh();
        run(&mut conn).unwrap();

        // The nine Home products in Home order, then InScript Mastery (0005,
        // no Home card) as display_order 10.
        for (index, product) in ProductId::every().into_iter().enumerate() {
            let slug = product.slug();
            let (display_name, layout_slug, is_beginner, display_order) = conn
                .query_row(
                    "SELECT p.display_name, l.slug, p.is_beginner, p.display_order
                       FROM products AS p
                       JOIN layouts  AS l ON l.id = p.layout_id
                      WHERE p.slug = ?1",
                    [slug],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, bool>(2)?,
                            r.get::<_, i64>(3)?,
                        ))
                    },
                )
                .unwrap_or_else(|e| panic!("product `{slug}` should be seeded: {e}"));

            assert_eq!(
                display_name,
                product.display_name(),
                "display name of {slug}"
            );
            assert_eq!(layout_slug, product.layout().as_str(), "layout of {slug}");
            assert_eq!(
                is_beginner,
                product.is_beginner(),
                "beginner flag of {slug}"
            );
            assert_eq!(display_order as usize, index + 1, "Home order of {slug}");
        }

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM products", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count as usize,
            ProductId::every().len(),
            "no products beyond the Rust enum"
        );
    }
}
