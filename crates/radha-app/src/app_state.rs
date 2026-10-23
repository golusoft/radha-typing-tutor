//! Application lifecycle state.
//!
//! Distinguishes "still starting up", "running", and "failed in a way the user
//! needs to be told about".
//!
//! # Why this exists
//!
//! Before this, an unrecoverable startup failure (a database that will not open)
//! reached only the log and stderr. A user launching from Explorer sees neither —
//! the release build has no console attached — so the app appeared to simply not
//! start. `RULES.MD` Rule 23 asks for graceful failure, and graceful has to mean
//! the user is told what happened, not just that the process avoided a panic.
//!
//! [`AppState::Error`] carries text written for the person on the other side of the
//! screen, with the raw cause kept separately for the log.

use std::fmt;

/// Where the application is in its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppState {
    /// Settings and the database are initialising.
    Loading,
    /// Running normally.
    Ready,
    /// Failed unrecoverably. Carries what to show the user.
    Error(AppError),
}

/// A user-facing description of an unrecoverable failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    /// One short line naming what failed.
    pub title: String,
    /// A sentence or two on what it means and what to try.
    pub detail: String,
    /// Path to the log file, so the user can find the technical detail.
    pub log_hint: String,
}

impl AppState {
    /// Slug the UI switches on. Slint cannot hold a Rust enum.
    #[must_use]
    pub fn slug(&self) -> &'static str {
        match self {
            AppState::Loading => "loading",
            AppState::Ready => "ready",
            AppState::Error(_) => "error",
        }
    }

    /// Whether the app is usable.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        matches!(self, AppState::Ready)
    }

    /// The error, if this is an error state.
    #[must_use]
    pub fn error(&self) -> Option<&AppError> {
        match self {
            AppState::Error(error) => Some(error),
            _ => None,
        }
    }

    /// Build an error state from a startup failure.
    ///
    /// Takes the `anyhow` cause chain and turns it into something worth reading.
    /// The root cause is used for the detail line because it is almost always the
    /// actionable part — "the file is locked" is useful, "could not start" is not.
    pub fn from_startup_failure(error: &anyhow::Error, log_path: &str) -> Self {
        let root_cause = error
            .chain()
            .last()
            .map_or_else(|| error.to_string(), ToString::to_string);

        AppState::Error(AppError {
            title: "Radha Typing Tutor could not start".to_owned(),
            detail: format!(
                "{error}.\n\nCause: {root_cause}\n\n\
                 If this keeps happening, closing any other copy of Radha that is \
                 already running usually resolves it."
            ),
            log_hint: log_path.to_owned(),
        })
    }
}

impl fmt::Display for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppState::Loading => write!(f, "loading"),
            AppState::Ready => write!(f, "ready"),
            AppState::Error(error) => write!(f, "error: {}", error.title),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_match_what_the_ui_switches_on() {
        assert_eq!(AppState::Loading.slug(), "loading");
        assert_eq!(AppState::Ready.slug(), "ready");
        assert_eq!(
            AppState::Error(AppError {
                title: "t".into(),
                detail: "d".into(),
                log_hint: "l".into(),
            })
            .slug(),
            "error"
        );
    }

    #[test]
    fn only_ready_is_usable() {
        assert!(AppState::Ready.is_ready());
        assert!(!AppState::Loading.is_ready());
    }

    #[test]
    fn simulated_database_failure_becomes_an_error_state_without_panicking() {
        // The Task 2.5 requirement: a DB-open failure must surface as an Error
        // state carrying a useful message, not a panic or a silent exit.
        let underlying = radha_core::CoreError::Database {
            detail: "unable to open database file".to_owned(),
        };
        let wrapped = anyhow::Error::new(underlying)
            .context("could not open the Radha database at C:/bad/path/radha.sqlite");

        let state = AppState::from_startup_failure(&wrapped, "C:/logs/radha.log");

        assert_eq!(state.slug(), "error");
        let error = state.error().expect("should carry an error");

        assert!(
            error.title.contains("could not start"),
            "title should be plain language, got: {}",
            error.title
        );
        assert!(
            error.detail.contains("unable to open database file"),
            "detail should surface the root cause, got: {}",
            error.detail
        );
        assert!(
            error.detail.contains("C:/bad/path/radha.sqlite"),
            "detail should name the path that failed, got: {}",
            error.detail
        );
        assert_eq!(error.log_hint, "C:/logs/radha.log");
    }

    #[test]
    fn ready_and_loading_carry_no_error() {
        assert!(AppState::Ready.error().is_none());
        assert!(AppState::Loading.error().is_none());
    }

    #[test]
    fn error_state_displays_its_title() {
        let state = AppState::Error(AppError {
            title: "Disk full".into(),
            detail: "d".into(),
            log_hint: "l".into(),
        });
        assert_eq!(state.to_string(), "error: Disk full");
    }
}
