//! English QWERTY layout.
//!
//! Covers every printable ASCII character a US keyboard produces: all four
//! rows, their shifted symbols, and the space bar (`TASK.MD` Section 14).
//!
//! Fingers follow the standard touch-typing chart. A shifted character uses the
//! same finger as its base key (`!` is struck by the finger that strikes `1`),
//! because Shift is held by the other hand and does not change which key is
//! pressed. Guidance for the Shift key itself is left to the guide.

use crate::{Finger, Hand, KeyGuidance, KeyboardLayout, LayoutId};

/// The English QWERTY layout.
#[derive(Debug, Default, Clone, Copy)]
pub struct QwertyLayout;

impl QwertyLayout {
    /// Create the layout.
    #[must_use]
    pub const fn new() -> Self {
        QwertyLayout
    }
}

impl KeyboardLayout for QwertyLayout {
    fn id(&self) -> LayoutId {
        LayoutId::Qwerty
    }

    fn guidance_for(&self, physical_key: char) -> Option<KeyGuidance> {
        let base = base_key(physical_key);
        let (hand, finger) = finger_for(base)?;

        Some(KeyGuidance {
            // Labelled by the key cap, so '!' and '1' both point at "1".
            physical_key: base.to_ascii_uppercase().to_string(),
            // For QWERTY the output is the character itself, unlike the Hindi
            // layouts, where key and output differ.
            output: physical_key.to_string(),
            hand,
            finger,
        })
    }
}

/// The unshifted key that produces `c`: a shifted symbol maps to the key it is
/// printed on, a capital letter to its lowercase key. Anything else is returned
/// unchanged, and [`finger_for`] decides whether QWERTY has it.
fn base_key(c: char) -> char {
    match c {
        '~' => '`',
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        ')' => '0',
        '_' => '-',
        '+' => '=',
        '{' => '[',
        '}' => ']',
        '|' => '\\',
        ':' => ';',
        '"' => '\'',
        '<' => ',',
        '>' => '.',
        '?' => '/',
        other => other.to_ascii_lowercase(),
    }
}

/// Hand and finger for an unshifted key, per the standard touch-typing chart.
/// `None` for anything that is not on a US keyboard.
fn finger_for(base: char) -> Option<(Hand, Finger)> {
    let assignment = match base {
        '`' | '1' | 'q' | 'a' | 'z' => (Hand::Left, Finger::Pinky),
        '2' | 'w' | 's' | 'x' => (Hand::Left, Finger::Ring),
        '3' | 'e' | 'd' | 'c' => (Hand::Left, Finger::Middle),
        '4' | '5' | 'r' | 't' | 'f' | 'g' | 'v' | 'b' => (Hand::Left, Finger::Index),
        '6' | '7' | 'y' | 'u' | 'h' | 'j' | 'n' | 'm' => (Hand::Right, Finger::Index),
        '8' | 'i' | 'k' | ',' => (Hand::Right, Finger::Middle),
        '9' | 'o' | 'l' | '.' => (Hand::Right, Finger::Ring),
        '0' | 'p' | '-' | '=' | '[' | ']' | '\\' | ';' | '\'' | '/' => {
            (Hand::Right, Finger::Pinky)
        }
        ' ' => (Hand::Right, Finger::Thumb),
        _ => return None,
    };
    Some(assignment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_its_own_id() {
        assert_eq!(QwertyLayout::new().id(), LayoutId::Qwerty);
    }

    #[test]
    fn home_row_index_fingers_are_correct() {
        let layout = QwertyLayout::new();

        let f = layout.guidance_for('f').expect("f is on the home row");
        assert_eq!(f.hand, Hand::Left);
        assert_eq!(f.finger, Finger::Index);

        let j = layout.guidance_for('j').expect("j is on the home row");
        assert_eq!(j.hand, Hand::Right);
        assert_eq!(j.finger, Finger::Index);
    }

    #[test]
    fn pinkies_anchor_the_home_row() {
        let layout = QwertyLayout::new();
        assert_eq!(layout.guidance_for('a').unwrap().finger, Finger::Pinky);
        assert_eq!(layout.guidance_for(';').unwrap().finger, Finger::Pinky);
    }

    #[test]
    fn space_is_a_thumb_key() {
        let layout = QwertyLayout::new();
        assert_eq!(layout.guidance_for(' ').unwrap().finger, Finger::Thumb);
    }

    #[test]
    fn unmapped_keys_return_none_rather_than_guessing() {
        // A Devanagari letter is not on a US keyboard.
        assert!(QwertyLayout::new().guidance_for('क').is_none());
    }

    #[test]
    fn qwerty_output_equals_the_key_pressed() {
        // Distinguishes QWERTY from the Hindi layouts, where key != output.
        let g = QwertyLayout::new().guidance_for('d').unwrap();
        assert_eq!(g.output, "d");
        assert_eq!(g.physical_key, "D");
    }

    #[test]
    fn number_row_key_is_mapped() {
        let g = QwertyLayout::new()
            .guidance_for('5')
            .expect("5 is on the number row");
        assert_eq!((g.hand, g.finger), (Hand::Left, Finger::Index));
        assert_eq!(g.physical_key, "5");
    }

    #[test]
    fn top_row_keys_are_mapped() {
        let layout = QwertyLayout::new();
        let q = layout.guidance_for('q').expect("q is on the top row");
        assert_eq!((q.hand, q.finger), (Hand::Left, Finger::Pinky));
        let p = layout.guidance_for('p').expect("p is on the top row");
        assert_eq!((p.hand, p.finger), (Hand::Right, Finger::Pinky));
    }

    #[test]
    fn bottom_row_key_is_mapped() {
        let g = QwertyLayout::new()
            .guidance_for('m')
            .expect("m is on the bottom row");
        assert_eq!((g.hand, g.finger), (Hand::Right, Finger::Index));
    }

    #[test]
    fn punctuation_is_mapped() {
        let layout = QwertyLayout::new();
        let comma = layout.guidance_for(',').expect(", is mapped");
        assert_eq!((comma.hand, comma.finger), (Hand::Right, Finger::Middle));
        let slash = layout.guidance_for('/').expect("/ is mapped");
        assert_eq!((slash.hand, slash.finger), (Hand::Right, Finger::Pinky));
    }

    #[test]
    fn a_shifted_symbol_uses_its_base_key() {
        let layout = QwertyLayout::new();
        let bang = layout.guidance_for('!').expect("! is mapped");
        let one = layout.guidance_for('1').expect("1 is mapped");
        assert_eq!(bang.physical_key, one.physical_key);
        assert_eq!((bang.hand, bang.finger), (one.hand, one.finger));
        assert_eq!(bang.output, "!", "the output is the character itself");
    }

    #[test]
    fn a_capital_letter_uses_its_lowercase_key() {
        let upper = QwertyLayout::new().guidance_for('Q').expect("Q is mapped");
        assert_eq!(upper.physical_key, "Q");
        assert_eq!(upper.output, "Q");
        assert_eq!((upper.hand, upper.finger), (Hand::Left, Finger::Pinky));
    }

    #[test]
    fn every_printable_ascii_character_is_mapped() {
        let layout = QwertyLayout::new();
        for c in ' '..='~' {
            assert!(layout.guidance_for(c).is_some(), "{c:?} should be mapped");
        }
    }
}
