//! # Radha Core
//!
//! Foundational crate for Radha Typing Tutor: the shared error type and the
//! typing engine.
//!
//! ## Modules
//!
//! - `error`: the workspace-wide error type every crate converts into.
//! - `matcher`: word-aligned comparison of typed text against the target.
//! - `accuracy`: accuracy and error counts over matcher outcomes.
//! - `speed`: WPM and net WPM.
//! - `session`: the session lifecycle.
//!
//! The engine is pure computation over in-memory state. It knows nothing about
//! input devices, the UI or the database; later phases drive it with real
//! keystrokes and render its snapshots.
//!
//! ## Architecture note: RAM-first sessions
//!
//! Per `RULES.MD` Rule 14, a typing session lives entirely in RAM and is
//! flushed to SQLite exactly once, on completion. No type in this crate holds a
//! database handle; `radha-db` accepts a completed session as a single batch
//! write.

pub mod accuracy;
pub mod error;
pub mod matcher;
pub mod session;
pub mod speed;

pub use accuracy::{accuracy, error_count, word_accuracy};
pub use error::{CoreError, Result};
pub use matcher::{CharOutcome, Matcher, WordOutcome};
pub use session::{SessionSnapshot, TypingSession};
pub use speed::{net_wpm, wpm};

/// Human-readable product name, used for window titles and log headers.
pub const PRODUCT_NAME: &str = "Radha Typing Tutor";

/// Crate version, surfaced in logs and the eventual About panel.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_name_is_set() {
        assert_eq!(PRODUCT_NAME, "Radha Typing Tutor");
    }

    #[test]
    fn version_is_not_empty() {
        assert!(!VERSION.is_empty());
    }
}
