//! Remington GAIL (Unicode Devanagari, typewriter order).
//!
//! The short i-sign `ि` (key `f`) is pre-posed: struck before its consonant
//! or whole conjunct and held as preedit by the composer until the cluster
//! exists. The reph (Shift+`z`) is struck after its syllable and moved
//! before the base. `k` and Shift+`a` give `ा` and complete a preceding half
//! form. Esc arms a dead-key layer; numpad digits follow Caps Lock.

mod rules;
mod table;

use crate::compose::SequenceRules;
use crate::finger::us_touch_typing;
use crate::keys::PhysicalKey;
use crate::table::{AltCodeEntry, LayoutTable, TableProfile};
use crate::{Encoding, Finger, Hand, KeyboardLayout, LayoutId, ASCII_DIGITS, DEVANAGARI_DIGITS};

static GAIL_TABLE: LayoutTable = LayoutTable::new(
    LayoutId::RemingtonGail,
    TableProfile::Unicode,
    &table::TABLE,
);

static GAIL_DEAD: LayoutTable =
    LayoutTable::new(LayoutId::RemingtonGail, TableProfile::Unicode, &table::DEAD);

/// The Remington GAIL layout.
#[derive(Debug, Default, Clone, Copy)]
pub struct GailLayout;

impl GailLayout {
    /// Create the layout.
    #[must_use]
    pub const fn new() -> Self {
        GailLayout
    }
}

impl KeyboardLayout for GailLayout {
    fn id(&self) -> LayoutId {
        LayoutId::RemingtonGail
    }

    fn table(&self) -> &'static LayoutTable {
        &GAIL_TABLE
    }

    fn encoding(&self) -> Encoding {
        Encoding::Unicode
    }

    fn finger_for(&self, key: PhysicalKey) -> (Hand, Finger) {
        // GAIL adopts the US touch-typing chart (gail-character-chart.md
        // gives every row the finger of its physical key).
        us_touch_typing(key)
    }

    fn rules(&self) -> Option<&'static SequenceRules> {
        Some(&rules::RULES)
    }

    fn alt_code(&self, code: u16) -> Option<&'static AltCodeEntry> {
        table::ALT_CODES.iter().find(|a| a.code == code)
    }

    fn dead_keys(&self) -> Option<&'static LayoutTable> {
        Some(&GAIL_DEAD)
    }

    fn numpad_digit(&self, digit: u8, caps_lock: bool) -> Option<&'static str> {
        // SRC-04 p.16: Caps Lock on gives 0-9, off gives ०-९.
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
    fn the_table_is_valid_with_112_slots_and_13_dead_keys() {
        assert_eq!(GailLayout::new().table().validate(), Ok(()));
        assert_eq!(GailLayout::new().table().entries().len(), 112);
        assert_eq!(GAIL_DEAD.validate(), Ok(()));
        assert_eq!(GAIL_DEAD.entries().len(), 13);
    }
}
