//! Layout errors.

use radha_core::CoreError;
use thiserror::Error;

/// What can go wrong when a layout is loaded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LayoutError {
    /// A table entry breaks a validation rule (`detail` names the rule id).
    #[error("invalid {layout} layout table at entry {index}: {detail}")]
    InvalidTable {
        /// The layout slug.
        layout: &'static str,
        /// Index of the offending entry.
        index: usize,
        /// Which rule failed and why.
        detail: String,
    },
}

impl From<LayoutError> for CoreError {
    fn from(err: LayoutError) -> Self {
        match err {
            LayoutError::InvalidTable {
                layout,
                index,
                detail,
            } => CoreError::malformed("layout table", layout, format!("entry {index}: {detail}")),
        }
    }
}
