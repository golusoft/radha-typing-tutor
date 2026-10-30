//! Per-finger error aggregation: radha-core's RAM key tallies grouped through
//! the QWERTY layout's finger guidance. Lives here because the layout crate
//! depends on the core, not the other way round.

use std::time::Instant;

use radha_core::TypingSession;
use radha_layouts::qwerty::QwertyLayout;
use radha_layouts::{Finger, Hand, KeyboardLayout};

#[test]
fn key_errors_roll_up_to_the_finger_that_strikes_the_key() {
    let layout = QwertyLayout::new();
    let t0 = Instant::now();
    let mut s = TypingSession::new("asdf jkl;");
    // 'a' and 'q' are the left pinky; type 'q' for 'a' and 'x' for 'l'.
    for c in "qsdf jkx;".chars() {
        s.type_char_at(c, t0);
    }

    let fingers = s
        .key_stats()
        .by_finger(|c: &char| layout.guidance_for(*c).map(|g| (g.hand, g.finger)));

    let tally = |hand: Hand, finger: Finger| {
        fingers
            .iter()
            .find(|(k, _)| *k == (hand, finger))
            .map(|(_, t)| (t.attempts, t.errors))
    };
    assert_eq!(tally(Hand::Left, Finger::Pinky), Some((1, 1)), "a missed");
    assert_eq!(tally(Hand::Right, Finger::Ring), Some((1, 1)), "l missed");
    assert_eq!(tally(Hand::Left, Finger::Index), Some((1, 0)), "f clean");
    assert_eq!(
        tally(Hand::Right, Finger::Thumb),
        Some((1, 0)),
        "space clean"
    );
    assert_eq!(tally(Hand::Right, Finger::Pinky), Some((1, 0)), "; clean");
    let total: u32 = fingers.iter().map(|(_, t)| t.attempts).sum();
    assert_eq!(total, 9, "every key press lands on exactly one finger");
}
