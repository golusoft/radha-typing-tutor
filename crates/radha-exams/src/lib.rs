//! # Radha Exams
//!
//! Data-driven exam profiles: duration, passage size, layouts, font,
//! qualification, scoring policy, error and backspace policy, sources and
//! the derived certification level of each supported exam
//! (EXAM-MODE-AND-GOLU-REUSE-PLAN.md Section 5, PRD 17.9).
//!
//! ## Why this crate is separate
//!
//! Per `RULES.MD` Rules 31–33, exam names and exam-specific rules are allowed
//! **only here** (and in the app's UI data). The engine crates (`radha-core`,
//! `radha-layouts`, `radha-content`, `radha-db`, `radha-analytics`) must stay
//! exam-agnostic; a policy test in `radha-app` greps them for exam names.
//!
//! ## Honesty
//!
//! Every rule value is a [`Field`]: Verified (primary document, cited),
//! UserAsserted (Golu's statement), Configurable (unverified default) or
//! Unknown. [`Certification`] is derived from those statuses, never read
//! from a file. Figures seen only in the design mock-up or on coaching sites
//! are [`ReferenceClaim`]s: shown as unverified, never used as rules.
//!
//! ## Files
//!
//! `profiles/*.toml`, embedded with `include_str!` ([`registry::EMBEDDED`]).
//! A result stores `profile_id@profile_version` ([`ExamProfile::key`]) so an
//! old result is read under the rules it was scored with.

use serde::Deserialize;

pub mod card;
pub mod certification;
pub mod field;
pub mod profile;
pub mod registry;
pub mod scoring;

pub use card::{card_summary, CardSummary};
pub use certification::Certification;
pub use field::{Field, Status};
pub use profile::{
    parse_profile, Backspace, Cycle, ExamProfile, Language, MistakeWeights, Notice, Post,
    Qualification, ReferenceClaim, Source, SourceKind, TypingTest,
};
pub use registry::{parse_key, LoadReport, Registry};
pub use scoring::{evaluate, policy, PolicyKind, ScoringPolicy, TestInput, TestOutcome, Verdict};

/// Schema version written at the top of every exam profile file.
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

/// The header shared by every exam profile TOML file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProfileFileHeader {
    /// Profile schema version; must equal [`PROFILE_SCHEMA_VERSION`] to load.
    pub schema_version: u32,
}

/// Parse the header of an exam profile TOML document.
///
/// # Errors
///
/// Returns the TOML deserialisation error if the text is not valid TOML or
/// `schema_version` is missing / not an unsigned integer.
pub fn parse_header(text: &str) -> Result<ProfileFileHeader, toml::de::Error> {
    toml::from_str(text)
}

/// The schema version alone, the error as text.
pub(crate) fn parse_header_lenient(text: &str) -> Result<u32, String> {
    parse_header(text)
        .map(|h| h.schema_version)
        .map_err(|e| e.to_string())
}

/// Why a profile did not load. Never a panic (RULES.MD Rule 23).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// Not valid TOML, or a field of the wrong type.
    Toml { file: String, message: String },
    /// Another schema version: skipped, not an error for the app.
    UnsupportedSchema { file: String, found: u32 },
    /// An id that is empty or not `[a-z0-9-]`.
    InvalidId { id: String },
    /// `status` is not one of the four.
    InvalidStatus {
        profile: String,
        field: String,
        status: String,
    },
    /// A Verified value with no source id.
    MissingSource { profile: String, field: String },
    /// A source id the profile does not list.
    UnknownSource {
        profile: String,
        field: String,
        source_id: String,
    },
    /// `value` / `default` / `sources` do not fit the status.
    FieldShape {
        profile: String,
        field: String,
        detail: String,
    },
    /// A value out of range or inconsistent.
    InvalidValue {
        profile: String,
        field: String,
        detail: String,
    },
    /// The scoring policy cannot judge the qualification.
    PolicyMismatch {
        profile: String,
        test: String,
        detail: String,
    },
    /// The same `id@version` twice.
    DuplicateProfile { key: String },
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::Toml { file, message } => write!(f, "{file}: {message}"),
            ProfileError::UnsupportedSchema { file, found } => write!(
                f,
                "{file}: schema_version {found} is not supported (expected {PROFILE_SCHEMA_VERSION})"
            ),
            ProfileError::InvalidId { id } => write!(f, "invalid profile id '{id}'"),
            ProfileError::InvalidStatus {
                profile,
                field,
                status,
            } => write!(f, "{profile}: {field}: unknown status '{status}'"),
            ProfileError::MissingSource { profile, field } => {
                write!(f, "{profile}: {field}: a verified value needs a source id")
            }
            ProfileError::UnknownSource {
                profile,
                field,
                source_id,
            } => write!(f, "{profile}: {field}: unknown source '{source_id}'"),
            ProfileError::FieldShape {
                profile,
                field,
                detail,
            }
            | ProfileError::InvalidValue {
                profile,
                field,
                detail,
            } => write!(f, "{profile}: {field}: {detail}"),
            ProfileError::PolicyMismatch {
                profile,
                test,
                detail,
            } => write!(f, "{profile}: {test} test: {detail}"),
            ProfileError::DuplicateProfile { key } => write!(f, "duplicate profile {key}"),
        }
    }
}

impl std::error::Error for ProfileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_schema_version_header() {
        let header = parse_header("schema_version = 1\n").expect("valid header");
        assert_eq!(header.schema_version, 1);
        assert_eq!(header.schema_version, PROFILE_SCHEMA_VERSION);
    }
}
