//! Kruti Dev 010 (legacy font encoding).
//!
//! Typed text is the raw ASCII key stream the font draws as Devanagari, in
//! visual order (`fd` draws `कि`). Sessions match it per `char` (the legacy
//! stream path); [`convert::to_legacy`] produces the target stream from
//! Unicode at import time, and [`reconstruct::to_unicode`] turns a stream
//! back into Unicode for round-trip checks and previews.

mod alt_codes;
pub mod convert;
pub mod reconstruct;
mod rules;
mod table;

use crate::finger::us_touch_typing;
use crate::keys::PhysicalKey;
use crate::table::{AltCodeEntry, LayoutTable, TableProfile};
use crate::{Encoding, Finger, Hand, KeyboardLayout, LayoutId};

pub use convert::{
    expected_round_trip, to_legacy, Conversion, ConvertFailure, ConvertStats, FailureReason,
};
pub use reconstruct::{to_unicode, Reconstruction};

static KRUTIDEV_TABLE: LayoutTable = LayoutTable::new(
    LayoutId::KrutiDev,
    TableProfile::LegacyIdentity,
    &table::TABLE,
);

/// The Kruti Dev layout.
#[derive(Debug, Default, Clone, Copy)]
pub struct KrutiDevLayout;

impl KrutiDevLayout {
    /// Create the layout.
    #[must_use]
    pub const fn new() -> Self {
        KrutiDevLayout
    }
}

/// The 58 verified Alt + numpad characters (Windows-1252, leading zero).
#[must_use]
pub fn alt_codes() -> &'static [AltCodeEntry] {
    &alt_codes::ALT_CODES
}

/// Characters Kruti Dev 010 cannot produce at all, with the reason.
#[must_use]
pub fn unresolved() -> &'static [(char, &'static str)] {
    &alt_codes::UNRESOLVED
}

impl KeyboardLayout for KrutiDevLayout {
    fn id(&self) -> LayoutId {
        LayoutId::KrutiDev
    }

    fn table(&self) -> &'static LayoutTable {
        &KRUTIDEV_TABLE
    }

    fn encoding(&self) -> Encoding {
        Encoding::Legacy
    }

    fn finger_for(&self, key: PhysicalKey) -> (Hand, Finger) {
        // An ASCII legacy font is typed on the US board with the US chart.
        us_touch_typing(key)
    }

    fn alt_code(&self, code: u16) -> Option<&'static AltCodeEntry> {
        alt_codes::ALT_CODES.iter().find(|a| a.code == code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_valid_with_93_slots_and_58_alt_codes() {
        assert_eq!(KrutiDevLayout::new().table().validate(), Ok(()));
        assert_eq!(KrutiDevLayout::new().table().entries().len(), 93);
        assert_eq!(alt_codes().len(), 58);
        assert!(alt_codes().iter().all(|a| a.leading_zero));
        assert!(KrutiDevLayout::new().alt_code(161).is_some());
        assert!(KrutiDevLayout::new().alt_code(162).is_none());
    }
}
