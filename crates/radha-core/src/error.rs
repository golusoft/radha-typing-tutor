//! Workspace-wide error foundation.
//!
//! # Strategy
//!
//! Radha follows the conventional split:
//!
//! - **Library crates** (`radha-core`, `radha-db`, `radha-content`,
//!   `radha-layouts`, `radha-analytics`) return typed errors built with
//!   `thiserror`. Callers can match on specific variants and recover.
//! - **The application boundary** (`radha-app`) uses `anyhow` to add context
//!   while bubbling errors up to a single top-level handler that logs and
//!   degrades gracefully.
//!
//! This exists from day one so no crate invents its own convention later.
//!
//! # Why a shared root error
//!
//! `RULES.MD` Rule 23 requires the application not to crash when an exercise is
//! missing, a layout is invalid, the database misbehaves, or a session is
//! interrupted. A single shared enum makes those cases *enumerable* — each one
//! is a variant a caller is forced to consider, rather than an opaque string.

use std::path::PathBuf;

/// Convenient alias for fallible operations in library crates.
pub type Result<T> = std::result::Result<T, CoreError>;

/// Root error type for Radha's library crates.
///
/// Sibling crates define their own focused error enums and convert into this
/// type at their public boundary, so `radha-app` only ever handles one shape.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// A required file or directory was not found on disk.
    ///
    /// Covers `RULES.MD` Rule 23's "exercise missing" case, and the missing
    /// config/asset cases exercised by Tasks 4 and 6.
    #[error("required resource not found: {path}")]
    ResourceMissing {
        /// Path that was expected to exist.
        path: PathBuf,
    },

    /// A platform directory (app data, config) could not be resolved.
    ///
    /// On Windows this would mean `%APPDATA%` is unavailable, which is rare but
    /// must not panic.
    #[error("could not resolve platform directory for {purpose}")]
    PlatformDirUnavailable {
        /// What the directory was needed for, e.g. "settings" or "database".
        purpose: &'static str,
    },

    /// Reading or writing a file failed.
    #[error("i/o error at {path}: {source}")]
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// A configuration or content file was malformed.
    #[error("malformed {kind} at {path}: {detail}")]
    Malformed {
        /// What kind of file it was, e.g. "settings", "exercise".
        kind: &'static str,
        /// Path to the offending file.
        path: PathBuf,
        /// Parser-supplied explanation.
        detail: String,
    },

    /// The database layer failed.
    ///
    /// `radha-db` converts `rusqlite` errors into this at its boundary so that
    /// `rusqlite` never appears in another crate's public API.
    #[error("database error: {detail}")]
    Database {
        /// Human-readable description of the failure.
        detail: String,
    },

    /// A schema migration failed to apply.
    #[error("migration {version} ({name}) failed: {detail}")]
    Migration {
        /// Numeric migration version.
        version: i64,
        /// Migration filename.
        name: String,
        /// Why it failed.
        detail: String,
    },

    /// A keyboard layout was requested that is not registered.
    ///
    /// Covers `RULES.MD` Rule 23's "invalid layout" case.
    #[error("unknown keyboard layout: {requested}")]
    UnknownLayout {
        /// The layout identifier that could not be resolved.
        requested: String,
    },

    /// An operation was attempted against a session in the wrong state.
    ///
    /// Covers `RULES.MD` Rule 23's "session interruption" case — for example an
    /// attempt to persist a session that never completed.
    #[error("invalid session operation: {detail}")]
    InvalidSessionState {
        /// What was attempted and why it was rejected.
        detail: String,
    },
}

impl CoreError {
    /// Build an [`CoreError::Io`] while attaching the path that failed.
    ///
    /// `std::io::Error` carries no path of its own, which makes bare I/O errors
    /// nearly useless in logs. This helper makes attaching one the easy path.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        CoreError::Io {
            path: path.into(),
            source,
        }
    }

    /// Build a [`CoreError::Malformed`] error.
    pub fn malformed(
        kind: &'static str,
        path: impl Into<PathBuf>,
        detail: impl Into<String>,
    ) -> Self {
        CoreError::Malformed {
            kind,
            path: path.into(),
            detail: detail.into(),
        }
    }

    /// Whether the application can reasonably continue after this error.
    ///
    /// Drives the top-level boundary in `radha-app`: recoverable errors are
    /// logged and the app carries on with defaults, while unrecoverable ones
    /// trigger a graceful shutdown with a clear message (never a panic
    /// backtrace in the user's face).
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        match self {
            // Missing/malformed config or content: fall back to defaults and
            // keep running. The user should not lose the whole app because one
            // file is bad.
            CoreError::ResourceMissing { .. }
            | CoreError::Malformed { .. }
            | CoreError::UnknownLayout { .. }
            | CoreError::InvalidSessionState { .. } => true,

            // Storage-level failures: the app cannot persist progress, which
            // is severe enough to stop rather than silently lose user data.
            CoreError::PlatformDirUnavailable { .. }
            | CoreError::Io { .. }
            | CoreError::Database { .. }
            | CoreError::Migration { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_helper_retains_path() {
        let err = CoreError::io(
            "C:/radha/settings.toml",
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        );
        let rendered = err.to_string();
        assert!(
            rendered.contains("settings.toml"),
            "path must appear in the message, got: {rendered}"
        );
    }

    #[test]
    fn malformed_helper_reports_kind_and_detail() {
        let err = CoreError::malformed("settings", "C:/radha/settings.toml", "bad key `thme`");
        let rendered = err.to_string();
        assert!(rendered.contains("settings"));
        assert!(rendered.contains("bad key"));
    }

    #[test]
    fn config_problems_are_recoverable() {
        // A bad settings file must not take the app down — it falls back to
        // defaults (exercised end to end by Task 4).
        assert!(CoreError::malformed("settings", "s.toml", "bad").is_recoverable());
        assert!(CoreError::ResourceMissing {
            path: PathBuf::from("missing.toml")
        }
        .is_recoverable());
    }

    #[test]
    fn storage_failures_are_not_recoverable() {
        assert!(!CoreError::Database {
            detail: "disk full".into()
        }
        .is_recoverable());
        assert!(!CoreError::Migration {
            version: 1,
            name: "0001_init.sql".into(),
            detail: "syntax error".into()
        }
        .is_recoverable());
    }

    #[test]
    fn unknown_layout_names_the_layout() {
        let err = CoreError::UnknownLayout {
            requested: "remington-xyz".into(),
        };
        assert!(err.to_string().contains("remington-xyz"));
    }
}
