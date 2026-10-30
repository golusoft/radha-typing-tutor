//! The US touch-typing finger chart.
//!
//! A physical-keyboard fact, not a layout table: which finger strikes which
//! key does not depend on what the key produces. Each layout adopts it
//! explicitly in its own `finger_for` (RULES 9 "Finger mapping"), so one
//! layout can diverge later by changing only its own module.
//!
//! Transcribed from Golu `LayoutManager.cs` `FingerMap` (L18-29) plus Space =
//! right thumb (`ResolveKeyHint` whitespace branch).

use crate::keys::PhysicalKey;
use crate::{Finger, Hand};

/// Hand and finger for a physical key. Total: every key has an answer.
#[must_use]
pub fn us_touch_typing(key: PhysicalKey) -> (Hand, Finger) {
    match key.base_char() {
        '`' | '1' | 'q' | 'a' | 'z' => (Hand::Left, Finger::Pinky),
        '2' | 'w' | 's' | 'x' => (Hand::Left, Finger::Ring),
        '3' | 'e' | 'd' | 'c' => (Hand::Left, Finger::Middle),
        '4' | '5' | 'r' | 't' | 'f' | 'g' | 'v' | 'b' => (Hand::Left, Finger::Index),
        '6' | '7' | 'y' | 'u' | 'h' | 'j' | 'n' | 'm' => (Hand::Right, Finger::Index),
        '8' | 'i' | 'k' | ',' => (Hand::Right, Finger::Middle),
        '9' | 'o' | 'l' | '.' => (Hand::Right, Finger::Ring),
        '0' | 'p' | '-' | '=' | '[' | ']' | '\\' | ';' | '\'' | '/' => (Hand::Right, Finger::Pinky),
        // The space bar (and, defensively, any unchecked byte): right thumb.
        _ => (Hand::Right, Finger::Thumb),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::SPACE;

    #[test]
    fn spot_checks_against_the_golu_finger_map() {
        let f = |c| us_touch_typing(PhysicalKey::from_base(c).unwrap());
        assert_eq!(f('f'), (Hand::Left, Finger::Index));
        assert_eq!(f('j'), (Hand::Right, Finger::Index));
        assert_eq!(f('`'), (Hand::Left, Finger::Pinky));
        assert_eq!(f('='), (Hand::Right, Finger::Pinky));
        assert_eq!(f(','), (Hand::Right, Finger::Middle));
        assert_eq!(us_touch_typing(SPACE), (Hand::Right, Finger::Thumb));
    }

    #[test]
    fn no_printable_key_falls_to_the_thumb() {
        for key in PhysicalKey::all() {
            assert_ne!(us_touch_typing(key).1, Finger::Thumb, "{key:?}");
        }
    }
}
