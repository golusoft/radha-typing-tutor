//! Remington CBI (Unicode Devanagari).
//!
//! Not a variant of GAIL in behaviour: the short i-sign `ि` (key `f`) is
//! typed after its consonant or complete conjunct (logical order; `d f` =
//! `कि`). The Normal/Shift slots match the GAIL book key for key; CBI has no
//! AltGr layer. The reph (Shift+`z`) is struck after its syllable and moved
//! before the base. Esc arms a dead-key layer; numpad digits follow Caps
//! Lock.

mod rules;
mod table;

use crate::compose::SequenceRules;
use crate::finger::us_touch_typing;
use crate::keys::PhysicalKey;
use crate::table::{AltCodeEntry, LayoutTable, TableProfile};
use crate::{Encoding, Finger, Hand, KeyboardLayout, LayoutId, ASCII_DIGITS, DEVANAGARI_DIGITS};

static CBI_TABLE: LayoutTable =
    LayoutTable::new(LayoutId::RemingtonCbi, TableProfile::Unicode, &table::TABLE);

static CBI_DEAD: LayoutTable =
    LayoutTable::new(LayoutId::RemingtonCbi, TableProfile::Unicode, &table::DEAD);

/// The Remington CBI layout.
#[derive(Debug, Default, Clone, Copy)]
pub struct CbiLayout;

impl CbiLayout {
    /// Create the layout.
    #[must_use]
    pub const fn new() -> Self {
        CbiLayout
    }
}

impl KeyboardLayout for CbiLayout {
    fn id(&self) -> LayoutId {
        LayoutId::RemingtonCbi
    }

    fn table(&self) -> &'static LayoutTable {
        &CBI_TABLE
    }

    fn encoding(&self) -> Encoding {
        Encoding::Unicode
    }

    fn finger_for(&self, key: PhysicalKey) -> (Hand, Finger) {
        // CBI adopts the US touch-typing chart; no reference gives a
        // CBI-specific finger for any key.
        us_touch_typing(key)
    }

    fn rules(&self) -> Option<&'static SequenceRules> {
        Some(&rules::RULES)
    }

    fn alt_code(&self, code: u16) -> Option<&'static AltCodeEntry> {
        table::ALT_CODES.iter().find(|a| a.code == code)
    }

    fn dead_keys(&self) -> Option<&'static LayoutTable> {
        Some(&CBI_DEAD)
    }

    fn numpad_digit(&self, digit: u8, caps_lock: bool) -> Option<&'static str> {
        // SRC-04 p.18: Caps Lock on gives 0-9, off gives ०-९.
        let digits = if caps_lock {
            &ASCII_DIGITS
        } else {
            &DEVANAGARI_DIGITS
        };
        digits.get(usize::from(digit)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_valid_with_94_slots_and_13_dead_keys() {
        assert_eq!(CbiLayout::new().table().validate(), Ok(()));
        assert_eq!(CbiLayout::new().table().entries().len(), 94);
        assert_eq!(CBI_DEAD.validate(), Ok(()));
        assert_eq!(CBI_DEAD.entries().len(), 13);
    }
}
