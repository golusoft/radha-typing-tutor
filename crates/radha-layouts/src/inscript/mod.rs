//! Standard Hindi InScript (Unicode Devanagari, logical phonetic order).
//!
//! A direct OS-driver style layout: every key inserts its text, nothing is
//! reordered. A matra follows its consonant (`k f` = `कि`), a conjunct is
//! consonant + `d` (halant) + consonant (`k d Shift+,` = `क्ष`), the reph is
//! `j d` or Shift+4 struck before its consonant. The last word is kept NFC.
//! Joiners: Ctrl+Shift+1 (or AltGr+Shift+Space) is ZWJ, Ctrl+Shift+2 (or
//! AltGr+Space) is ZWNJ (INSC-MAP-115..116). No Alt codes are needed.

mod table;

use crate::finger::us_touch_typing;
use crate::keys::{Layer, PhysicalKey};
use crate::table::{LayoutTable, TableProfile};
use crate::{Encoding, Finger, Hand, KeyboardLayout, LayoutId};

/// Zero width joiner.
const ZWJ: &str = "\u{200D}";
/// Zero width non-joiner.
const ZWNJ: &str = "\u{200C}";

static INSCRIPT_TABLE: LayoutTable =
    LayoutTable::new(LayoutId::Inscript, TableProfile::Unicode, &table::TABLE);

/// The Standard Hindi InScript layout (KBDINDEV default profile).
#[derive(Debug, Default, Clone, Copy)]
pub struct InscriptLayout;

impl InscriptLayout {
    /// Create the layout.
    #[must_use]
    pub const fn new() -> Self {
        InscriptLayout
    }
}

impl KeyboardLayout for InscriptLayout {
    fn id(&self) -> LayoutId {
        LayoutId::Inscript
    }

    fn table(&self) -> &'static LayoutTable {
        &INSCRIPT_TABLE
    }

    fn encoding(&self) -> Encoding {
        Encoding::Unicode
    }

    fn finger_for(&self, key: PhysicalKey) -> (Hand, Finger) {
        // InScript is typed on the US board with the US touch-typing chart.
        us_touch_typing(key)
    }

    fn space_chord(&self, layer: Layer) -> Option<&'static str> {
        match layer {
            Layer::AltGr => Some(ZWNJ),
            Layer::ShiftAltGr => Some(ZWJ),
            Layer::Base | Layer::Shift => None,
        }
    }

    fn ctrl_shift_digit(&self, digit: char) -> Option<&'static str> {
        match digit {
            '1' => Some(ZWJ),
            '2' => Some(ZWNJ),
            _ => None,
        }
    }
}

/// Whether `text` can be typed on InScript key by key: every character is
/// some key's whole output. Used to serve shared Unicode Hindi content to
/// InScript honestly (a passage with `?` or `“` is refused, not altered).
#[must_use]
pub fn can_type(text: &str) -> bool {
    // NFC first: U+0958..095F (क़ ..) become consonant + nukta, both keys.
    INSCRIPT_TABLE.covers_directly(&radha_core::akshar::normalize(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_valid_with_114_slots() {
        assert_eq!(InscriptLayout::new().table().validate(), Ok(()));
        assert_eq!(InscriptLayout::new().table().entries().len(), 114);
    }

    #[test]
    fn typability_is_per_character() {
        assert!(can_type("विद्या भारत।"));
        assert!(!can_type("क्या?"));
        assert!(can_type("\u{0958}"), "precomposed क़ is क + nukta");
    }
}
