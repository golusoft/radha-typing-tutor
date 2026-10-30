//! The physical keyboard: keys, layers, and how modifiers select a layer.
//!
//! Every layout in Radha assumes the US ANSI board that every Golu reference
//! assumes. A [`PhysicalKey`] is identified by the character the key prints
//! with no modifier held (`d`, `;`, `` ` ``), never by what a layout makes it
//! produce, so a keystroke means the same thing on all four layouts.

/// The 47 printable keys in board order (number, top, home, bottom row).
const BASE: &str = "`1234567890-=qwertyuiop[]\\asdfghjkl;'zxcvbnm,./";
/// The keycap legend of each key in [`BASE`], same order.
const LEGEND: &str = "`1234567890-=QWERTYUIOP[]\\ASDFGHJKL;'ZXCVBNM,./";
/// The US character each key in [`BASE`] produces with Shift, same order.
const SHIFTED: &str = "~!@#$%^&*()_+QWERTYUIOP{}|ASDFGHJKL:\"ZXCVBNM<>?";

/// One physical key, by its unshifted US character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PhysicalKey(u8);

/// The space bar. It is the same key with the same output on every layout,
/// so it never appears in a layout table.
pub const SPACE: PhysicalKey = PhysicalKey(b' ');

impl PhysicalKey {
    /// The key whose unshifted US character is `c` (`' '` is the space bar).
    #[must_use]
    pub fn from_base(c: char) -> Option<Self> {
        if c == ' ' {
            return Some(SPACE);
        }
        let byte = u8::try_from(c).ok()?;
        BASE.as_bytes().contains(&byte).then_some(PhysicalKey(byte))
    }

    /// A key from its byte without checking it. Used by the const layout
    /// tables; [`crate::LayoutTable::validate`] re-checks every key.
    #[must_use]
    pub const fn new_unchecked(byte: u8) -> Self {
        PhysicalKey(byte)
    }

    /// Board index of the key (0..47), or `None` for the space bar or an
    /// unchecked byte that is not a key.
    #[must_use]
    pub fn index(self) -> Option<usize> {
        BASE.bytes().position(|b| b == self.0)
    }

    /// Whether this is one of the 47 printable keys or the space bar.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self == SPACE || self.index().is_some()
    }

    /// The unshifted US character.
    #[must_use]
    pub fn base_char(self) -> char {
        char::from(self.0)
    }

    /// The US character with Shift (`'!'` for `1`, `'A'` for `a`); the space
    /// bar gives `' '`.
    #[must_use]
    pub fn shifted_char(self) -> char {
        if self == SPACE {
            return ' ';
        }
        self.index()
            .and_then(|i| SHIFTED.as_bytes().get(i).copied())
            .map_or(self.base_char(), char::from)
    }

    /// The keycap legend: capital letters, the base symbol otherwise,
    /// `"Space"` for the space bar.
    #[must_use]
    pub fn legend(self) -> &'static str {
        if self == SPACE {
            return "Space";
        }
        self.index()
            .and_then(|i| LEGEND.get(i..i + 1))
            .unwrap_or("")
    }

    /// Whether the key is a letter (Caps Lock applies to letters only).
    #[must_use]
    pub fn is_letter(self) -> bool {
        self.0.is_ascii_lowercase()
    }

    /// Which board row the key is on (0 number, 1 top, 2 home, 3 bottom),
    /// `None` for the space bar.
    #[must_use]
    pub fn row(self) -> Option<usize> {
        let i = self.index()?;
        Some(match i {
            0..=12 => 0,
            13..=25 => 1,
            26..=36 => 2,
            _ => 3,
        })
    }

    /// All 47 printable keys in board order.
    pub fn all() -> impl Iterator<Item = PhysicalKey> {
        BASE.bytes().map(PhysicalKey)
    }
}

/// `PhysicalKey::new_unchecked`, short form for the const tables.
pub(crate) const fn k(byte: u8) -> PhysicalKey {
    PhysicalKey::new_unchecked(byte)
}

/// The board as four rows of keys, the shape every keycap grid uses
/// (13, 13, 11, 10 keys).
#[must_use]
pub fn rows() -> [Vec<PhysicalKey>; 4] {
    let all: Vec<PhysicalKey> = PhysicalKey::all().collect();
    let take =
        |r: std::ops::Range<usize>| all.get(r).map(<[PhysicalKey]>::to_vec).unwrap_or_default();
    [take(0..13), take(13..26), take(26..37), take(37..47)]
}

/// Which modifier layer of a key is struck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    /// No modifier.
    Base,
    /// Shift (or Caps Lock on a letter).
    Shift,
    /// Right Alt (AltGr).
    AltGr,
    /// Shift + AltGr. No Radha layout maps it; it exists so the combination
    /// resolves to an explicit unmapped slot instead of falling back.
    ShiftAltGr,
}

impl Layer {
    /// Modifier cost for "fewest modifiers first" ordering.
    #[must_use]
    pub fn cost(self) -> u8 {
        match self {
            Layer::Base => 0,
            Layer::Shift => 1,
            Layer::AltGr => 2,
            Layer::ShiftAltGr => 3,
        }
    }
}

/// One key press: a physical key on a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Keystroke {
    /// The key.
    pub key: PhysicalKey,
    /// The layer selected by the modifiers.
    pub layer: Layer,
}

impl Keystroke {
    /// A key on a layer.
    #[must_use]
    pub const fn new(key: PhysicalKey, layer: Layer) -> Self {
        Keystroke { key, layer }
    }

    /// Resolve the held modifiers. For letter keys the Shift layer is
    /// `shift XOR caps_lock`; Caps Lock does not affect any other key
    /// (Windows behaviour).
    #[must_use]
    pub fn from_modifiers(key: PhysicalKey, shift: bool, caps_lock: bool, altgr: bool) -> Self {
        let shifted = if key.is_letter() {
            shift != caps_lock
        } else {
            shift
        };
        let layer = match (shifted, altgr) {
            (false, false) => Layer::Base,
            (true, false) => Layer::Shift,
            (false, true) => Layer::AltGr,
            (true, true) => Layer::ShiftAltGr,
        };
        Keystroke { key, layer }
    }

    /// The keystroke that makes a US board produce `c` (`'!'` is Shift+`1`,
    /// `'A'` is Shift+`a`, `' '` is the space bar). `None` for anything not
    /// on a US board.
    #[must_use]
    pub fn from_us_char(c: char, altgr: bool) -> Option<Self> {
        if c == ' ' {
            // AltGr+Space is a chord on some layouts (InScript ZWNJ); every
            // composer treats any other Space layer as a plain space.
            let layer = if altgr { Layer::AltGr } else { Layer::Base };
            return Some(Keystroke::new(SPACE, layer));
        }
        let byte = u8::try_from(c).ok()?;
        let (index, shifted) = if let Some(i) = BASE.bytes().position(|b| b == byte) {
            (i, false)
        } else {
            (SHIFTED.bytes().position(|b| b == byte)?, true)
        };
        let key = PhysicalKey(*BASE.as_bytes().get(index)?);
        Some(Keystroke::from_modifiers(key, shifted, false, altgr))
    }

    /// What the typist must physically hold for this keystroke:
    /// `(shift, altgr)`. With Caps Lock on, a Shift-layer letter needs no
    /// Shift and a Base-layer letter does.
    #[must_use]
    pub fn hold_modifiers(self, caps_lock: bool) -> (bool, bool) {
        let shift_layer = matches!(self.layer, Layer::Shift | Layer::ShiftAltGr);
        let altgr = matches!(self.layer, Layer::AltGr | Layer::ShiftAltGr);
        let shift = if self.key.is_letter() && caps_lock {
            !shift_layer
        } else {
            shift_layer
        };
        (shift, altgr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_board_has_47_keys_in_four_rows() {
        let rows = rows();
        assert_eq!(
            rows.iter().map(Vec::len).collect::<Vec<_>>(),
            [13, 13, 11, 10]
        );
        assert_eq!(PhysicalKey::all().count(), 47);
    }

    #[test]
    fn every_printable_ascii_character_has_a_keystroke() {
        for c in ' '..='~' {
            let ks = Keystroke::from_us_char(c, false).unwrap();
            let produced = if ks.layer == Layer::Shift {
                ks.key.shifted_char()
            } else {
                ks.key.base_char()
            };
            assert_eq!(produced, c);
        }
        assert!(Keystroke::from_us_char('क', false).is_none());
    }

    #[test]
    fn caps_lock_flips_letters_only() {
        let a = PhysicalKey::from_base('a').unwrap();
        let one = PhysicalKey::from_base('1').unwrap();
        assert_eq!(
            Keystroke::from_modifiers(a, false, true, false).layer,
            Layer::Shift
        );
        assert_eq!(
            Keystroke::from_modifiers(a, true, true, false).layer,
            Layer::Base
        );
        assert_eq!(
            Keystroke::from_modifiers(one, false, true, false).layer,
            Layer::Base
        );
        assert_eq!(
            Keystroke::from_modifiers(a, true, false, true).layer,
            Layer::ShiftAltGr
        );
        assert_eq!(
            Keystroke::new(a, Layer::Shift).hold_modifiers(true),
            (false, false)
        );
        assert_eq!(
            Keystroke::new(a, Layer::Base).hold_modifiers(true),
            (true, false)
        );
        assert_eq!(
            Keystroke::new(one, Layer::Shift).hold_modifiers(true),
            (true, false)
        );
    }

    #[test]
    fn legends_and_shifted_characters() {
        let k = |c| PhysicalKey::from_base(c).unwrap();
        assert_eq!(k('a').legend(), "A");
        assert_eq!(k(';').legend(), ";");
        assert_eq!(SPACE.legend(), "Space");
        assert_eq!(k('1').shifted_char(), '!');
        assert_eq!(k('\'').shifted_char(), '"');
        assert_eq!(k('/').row(), Some(3));
        assert!(PhysicalKey::from_base('A').is_none());
    }
}
