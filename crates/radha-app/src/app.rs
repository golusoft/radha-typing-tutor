//! Application startup and orchestration.
//!
//! Owns the startup sequence and the lifetime of the shared services (settings,
//! database). Keeping this out of `main.rs` leaves `main` as nothing but the
//! error boundary.
//!
//! # Startup sequence
//!
//! ```text
//! STAGE 1 — environment           (init_environment)
//!   1. Resolve platform paths     (paths.rs)
//!   2. Create directories
//!   3. Initialise logging         (logging.rs)
//!
//! STAGE 2 — services              (App::bootstrap_services)
//!   4. Load settings, or defaults (settings.rs)
//!   5. Open + migrate database    (radha-db)
//!
//! STAGE 3 — UI                    (ui.rs)
//!   6. Build and show the window
//!   7. Run the event loop
//!   8. Persist settings on exit
//! ```
//!
//! # Why two stages
//!
//! The split is what makes an in-window error state possible. A stage-2 failure —
//! most realistically a database that will not open — can be shown to the user in
//! a real window, because by then logging works and a window can be created.
//!
//! A stage-1 failure cannot: without resolved paths there is nowhere to log, and
//! reporting through a window would risk failing for the same underlying reason.
//! Those fall back to stderr, which is the honest limit of what is possible that
//! early.

use anyhow::Context;
use radha_content::ExerciseRepository;

use crate::paths::AppPaths;
use crate::settings::Settings;

/// Environment variable that forces a fatal error during startup.
///
/// Exists to demonstrate and test the top-level error boundary: it proves an
/// unrecoverable failure produces a clean logged message and a non-zero exit
/// rather than a panic backtrace. Honoured in debug builds only.
const SIMULATE_FATAL_ENV: &str = "RADHA_SIMULATE_FATAL";

/// Environment variable that forces the database open to fail.
///
/// Unlike [`SIMULATE_FATAL_ENV`], which bails out synthetically, this routes a
/// real failure through the real `radha-db` code path by pointing the database at
/// a directory rather than a file. That exercises the genuine error, message, and
/// in-window presentation rather than a stand-in. Debug builds only.
const SIMULATE_DB_FAILURE_ENV: &str = "RADHA_SIMULATE_DB_FAILURE";

/// Resolved environment: where things live, and a working logger.
pub struct Environment {
    pub paths: AppPaths,
    pub log_file: std::path::PathBuf,
}

/// Run stage 1: resolve paths, create directories, start logging.
///
/// # Errors
///
/// Returns an error if the platform directories cannot be resolved or created, or
/// if the logger cannot be installed. These can only be reported to stderr — see
/// the module docs.
pub fn init_environment() -> anyhow::Result<Environment> {
    let paths = AppPaths::resolve().context("could not determine where to store app data")?;
    paths
        .ensure_exist()
        .context("could not create application directories")?;

    let log_file = crate::logging::init(&paths).context("could not initialise logging")?;
    crate::logging::log_startup_banner(&paths, &log_file);

    Ok(Environment { paths, log_file })
}

/// Services shared across the application's lifetime.
///
/// The log path is deliberately not held here: `main` keeps it, because it is
/// needed precisely when this struct could not be built.
pub struct App {
    pub paths: AppPaths,
    pub settings: Settings,
    pub database: radha_db::Database,
    /// Read access to exercise content, over its own connection to the same
    /// database file (see `radha_db::repository` for why). Consumed by the
    /// Exercise screen in Phase 5.
    pub repository: radha_db::SqliteRepository,
}

impl App {
    /// Run stage 2: load settings and open the database.
    ///
    /// # Errors
    ///
    /// Returns a contextualised error if an unrecoverable step fails. Recoverable
    /// problems (missing or corrupt settings) are logged and defaulted instead.
    /// Callers are expected to surface a failure here in the UI rather than exiting
    /// silently.
    pub fn bootstrap_services(environment: Environment) -> anyhow::Result<Self> {
        let Environment { paths, .. } = environment;

        // Demo/test hook for the error boundary. Placed after logging so the
        // failure is visible in the log, which is the behaviour being verified.
        Self::check_simulated_fatal()?;

        // --- Settings -----------------------------------------------------
        // Infallible by design: a bad file degrades to defaults.
        let settings = Settings::load_or_default(paths.settings_file());
        log::info!(
            "Settings: theme={:?}, window={}x{}",
            settings.theme,
            settings.window.width,
            settings.window.height
        );

        // --- Database -----------------------------------------------------
        let database_path = Self::database_path(&paths);
        let database = radha_db::Database::open(&database_path).with_context(|| {
            format!(
                "could not open the Radha database at {}",
                database_path.display()
            )
        })?;
        log::info!(
            "Database ready at {} (schema version {})",
            database_path.display(),
            database
                .schema_version()
                .context("could not read schema version")?
        );

        // --- Content repository -------------------------------------------
        // A second connection to the same file, owned by the repository. Opening
        // it re-checks migrations, which is a no-op after the open above.
        let repository = radha_db::SqliteRepository::new(
            radha_db::Database::open(&database_path).with_context(|| {
                format!(
                    "could not open the content repository at {}",
                    database_path.display()
                )
            })?,
        );

        let app = App {
            paths,
            settings,
            database,
            repository,
        };
        app.log_content_summary()?;
        Ok(app)
    }

    /// Log how much exercise content is available.
    ///
    /// Doubles as a startup check on the repository: every product's exercise
    /// list is queried once, so a broken schema is reported through the startup
    /// error window now, rather than surfacing later when the user opens a
    /// product.
    fn log_content_summary(&self) -> anyhow::Result<()> {
        let products = radha_content::ProductId::all();
        let mut total = 0;
        for &product in &products {
            total += self
                .repository
                .list_for_product(product)
                .with_context(|| format!("could not list exercises for {}", product.slug()))?
                .len();
        }
        log::info!(
            "Content repository ready: {total} exercises across {} products",
            products.len()
        );
        Ok(())
    }

    /// Where the database should be opened from.
    ///
    /// Normally [`AppPaths::database_file`]. When [`SIMULATE_DB_FAILURE_ENV`] is
    /// set in a debug build, returns the data *directory* instead — SQLite cannot
    /// open a directory as a database, which produces a genuine open failure.
    fn database_path(paths: &AppPaths) -> std::path::PathBuf {
        if cfg!(debug_assertions) && std::env::var_os(SIMULATE_DB_FAILURE_ENV).is_some() {
            log::warn!(
                "{SIMULATE_DB_FAILURE_ENV} is set — pointing the database at a \
                 directory to force an open failure"
            );
            return paths.data_dir().to_path_buf();
        }
        paths.database_file()
    }

    /// Persist state that should survive a restart.
    ///
    /// Called on clean shutdown. Failures are logged rather than propagated —
    /// losing a window-size preference is not worth failing an exit over, and
    /// reporting an error after the window has closed would have nowhere to go.
    pub fn shutdown(&mut self) {
        // Clear the first-launch flag now that a launch has completed.
        self.settings.first_launch = false;

        if let Err(err) = self.settings.save(self.paths.settings_file()) {
            log::warn!("Could not save settings on exit: {err}");
        }

        // Record the schema version the session ran against. Cheap, and it makes
        // "which schema was this user on when it broke?" answerable from a log
        // alone.
        match self.database.schema_version() {
            Ok(version) => log::debug!("Closing database at schema version {version}"),
            Err(err) => log::warn!("Could not read schema version on exit: {err}"),
        }

        log::info!("{} exited cleanly", radha_core::PRODUCT_NAME);
    }

    /// Honour [`SIMULATE_FATAL_ENV`] in debug builds.
    fn check_simulated_fatal() -> anyhow::Result<()> {
        if cfg!(debug_assertions) && std::env::var_os(SIMULATE_FATAL_ENV).is_some() {
            log::error!("{SIMULATE_FATAL_ENV} is set — simulating an unrecoverable startup failure");
            anyhow::bail!(
                "simulated fatal error ({SIMULATE_FATAL_ENV} was set). \
                 This verifies the top-level error boundary."
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_services_constructs_the_content_repository() {
        // Task 3.3: the real startup path, not a hand-assembled App, builds a
        // working repository against a freshly migrated database.
        if std::env::var_os(SIMULATE_FATAL_ENV).is_some()
            || std::env::var_os(SIMULATE_DB_FAILURE_ENV).is_some()
        {
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted_at(temp.path());
        paths.ensure_exist().unwrap();

        let app = App::bootstrap_services(Environment {
            log_file: paths.log_file(),
            paths,
        })
        .expect("services should start against a temp root");

        for product in radha_content::ProductId::all() {
            let exercises = app
                .repository
                .list_for_product(product)
                .expect("the repository should answer queries");
            assert!(exercises.is_empty(), "no content has been imported yet");
        }
    }

    #[test]
    fn simulated_fatal_is_off_by_default() {
        // Guards against the demo hook accidentally firing in normal runs.
        if std::env::var_os(SIMULATE_FATAL_ENV).is_none() {
            assert!(App::check_simulated_fatal().is_ok());
        }
    }

    #[test]
    fn database_path_is_the_normal_file_by_default() {
        if std::env::var_os(SIMULATE_DB_FAILURE_ENV).is_none() {
            let paths = AppPaths::rooted_at("C:/tmp/radha");
            assert_eq!(App::database_path(&paths), paths.database_file());
        }
    }

    #[test]
    fn pointing_the_database_at_a_directory_really_fails() {
        // Underpins the SIMULATE_DB_FAILURE hook: confirms the technique produces a
        // genuine error rather than silently succeeding, so the demo exercises the
        // real path.
        let temp = tempfile::tempdir().unwrap();
        let outcome = radha_db::Database::open(temp.path());
        assert!(
            outcome.is_err(),
            "opening a directory as a database should fail"
        );
    }

    #[test]
    fn bootstrap_steps_work_against_a_temp_root() {
        // Exercises settings and database setup without touching the real user
        // profile or installing a global logger (only one test may do that).
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted_at(temp.path());

        paths.ensure_exist().expect("directories should be created");

        let settings = Settings::load_or_default(paths.settings_file());
        assert!(settings.first_launch, "temp root implies a first launch");

        let database =
            radha_db::Database::open(paths.database_file()).expect("database should open");
        assert!(
            database.schema_version().unwrap() >= 1,
            "migrations should have run"
        );
        assert!(
            paths.database_file().exists(),
            "database file should exist on disk after open"
        );
    }

    #[test]
    fn shutdown_clears_the_first_launch_flag() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted_at(temp.path());
        paths.ensure_exist().unwrap();

        let mut app = App {
            settings: Settings::first_run(),
            database: radha_db::Database::open(paths.database_file()).unwrap(),
            repository: radha_db::SqliteRepository::new(
                radha_db::Database::open(paths.database_file()).unwrap(),
            ),
            paths: paths.clone(),
        };
        assert!(app.settings.first_launch);

        app.shutdown();

        let reloaded = Settings::load(paths.settings_file()).expect("settings should be saved");
        assert!(
            !reloaded.first_launch,
            "after one completed launch the flag must be cleared"
        );
    }
}
