//! # Radha Core
//!
//! Foundational crate for Radha Typing Tutor: the shared error type and the
//! typing engine.
//!
//! ## Modules
//!
//! - `error`: the workspace-wide error type every crate converts into.
//! - `input`: layout-agnostic key effects (`TextEdit`) and the `Composition`
//!   hook a layout implements.
//! - `unit`: what one comparable "letter" is (`char`, or an akshar).
//! - `akshar`: grapheme-aware Hindi segmentation (RULES.MD Rule 35).
//! - `matcher`: word-aligned comparison of typed text against the target.
//! - `mode`: KEY and WORD display states.
//! - `mistake`: half/full mistake classification as pure functions.
//! - `scoring`: the one scoring module, one formula per metric (Rule 36).
//! - `stats`: per-key and per-finger tallies, RAM only.
//! - `timer`: timer modes, presets and deadline maths (Rule 30).
//! - `session`: the session lifecycle.
//! - `flow`: the Typing Flow rhythm engine (PRD 17.8).
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

// Production code must not panic (RULES.MD: no unwrap/panic in production).
// Tests may; the policy test in radha-app enforces the same rule by grep.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented
    )
)]

pub mod akshar;
pub mod error;
pub mod flow;
pub mod input;
pub mod matcher;
pub mod mistake;
pub mod mode;
pub mod scoring;
pub mod session;
pub mod stats;
pub mod timer;
pub mod unit;

pub use akshar::Akshar;
pub use error::{CoreError, Result};
pub use input::{Composition, TextEdit};
pub use matcher::{CharOutcome, Evaluation, Matcher, Outcome, WordOutcome};
pub use mistake::{
    classify_outcome, classify_units, classify_word, word_mistakes, MistakeCounts, MistakeKind,
    Severity, WordMistake,
};
pub use mode::{unit_states, DisplayMode, UnitState};
pub use scoring::{
    accuracy, characters_typed, consistency, error_count, kdph, net_wpm, word_accuracy, wpm,
    ErrorBreakdown, Score,
};
pub use session::{CompletionReason, SessionConfig, SessionSnapshot, SessionState, TypingSession};
pub use stats::{KeyStats, KeyTally};
pub use timer::{TimeLimit, TimerMode, TimerPreset};
pub use unit::TextUnit;

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
