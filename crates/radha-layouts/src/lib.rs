//! # Radha Layouts
//!
//! Keyboard layout engine: QWERTY, Remington GAIL, Remington CBI, Kruti Dev
//! and Standard Hindi InScript.
//!
//! Three jobs, one table per layout:
//!
//! - **Compose** ([`compose`]): turn a physical keystroke into one
//!   [`radha_core::TextEdit`] of the typed text. Unicode layouts (GAIL, CBI)
//!   emit NFC text in logical order: GAIL's pre-posed `ि` is held as preedit
//!   until its consonant exists, the reph struck after its syllable and the
//!   half-form completer are tail replacements. QWERTY and Kruti Dev insert
//!   one character per key (Kruti Dev stays on the legacy `char` path).
//! - **Guide** ([`plan`], [`guide`]): plan the key sequence for each akshar
//!   of a target and say which key to press next, with which finger and
//!   modifiers ([`guide::KeyHint`]).
//! - **Convert** ([`krutidev::convert`], [`krutidev::reconstruct`]): Unicode
//!   Hindi to and from the Kruti Dev legacy key stream.
//!
//! [`input`] glues a composer to a `radha_core::TypingSession`.
//!
//! ## Hard rule: layouts are independent
//!
//! `RULES.MD` Rule 9 and `PRD.md` Section 4.2:
//!
//! ```text
//! GAIL  ≠  CBI  ≠  Kruti Dev
//! ```
//!
//! Each layout has its own module with its own `pub(super)` table, sequence
//! rules and `finger_for`. Shared modules hold only physical-keyboard facts
//! and data-free algorithms. `tests/independence.rs` scans the sources and
//! fails if any layout module names another layout, its table, or the
//! registry.

// Production code must not panic (RULES.MD: no unwrap/panic in production).
// Tests may; the policy test in radha-app enforces the same rule by grep.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented
    )
)]

use radha_core::CoreError;

pub mod cbi;
pub mod compose;
pub mod error;
pub mod finger;
pub mod gail;
pub mod guide;
pub mod input;
pub mod inscript;
pub mod keys;
pub mod krutidev;
pub mod plan;
pub mod qwerty;
pub mod registry;
pub mod script;
pub mod table;

pub use compose::{Composer, KeyOutput, Ligature, SequenceRules};
pub use error::LayoutError;
pub use guide::{Guide, KeyHint, NextAction};
pub use input::{
    press_alt_code, press_alt_numpad, press_backspace, press_enter, press_key, press_numpad_digit,
    KeyEffect,
};
pub use keys::{Keystroke, Layer, PhysicalKey, SPACE};
pub use plan::{Plan, PlanOutcome, PlanState, PlanStep, Planner};
pub use registry::load_layout;
pub use table::{AltCodeEntry, KeyEntry, LayoutTable, TableProfile, TokenKind};

/// Which hand a key is struck with. Drives the Hindi Guide's hand hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hand {
    Left,
    Right,
}

impl Hand {
    /// The other hand (the one that holds Shift).
    #[must_use]
    pub fn opposite(self) -> Hand {
        match self {
            Hand::Left => Hand::Right,
            Hand::Right => Hand::Left,
        }
    }
}

/// Which finger strikes a key. Drives the Hindi Guide's finger hint and the
/// virtual keyboard's finger colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Finger {
    Pinky,
    Ring,
    Middle,
    Index,
    Thumb,
}

/// Identifies one of Radha's five supported layouts.
///
/// Kept as a closed enum rather than a string so that adding a layout is a
/// deliberate, compiler-checked change across the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutId {
    /// English QWERTY — used by English Mastery, Typing Flow, Number + Punctuation.
    Qwerty,
    /// Remington GAIL (Unicode Devanagari).
    RemingtonGail,
    /// Remington CBI (Unicode Devanagari) — *not* a variant of GAIL.
    RemingtonCbi,
    /// Kruti Dev (legacy font encoding).
    KrutiDev,
    /// Standard Hindi InScript (BIS IS 13194, Windows KBDINDEV profile).
    Inscript,
}

impl LayoutId {
    /// Stable string identifier, used as the SQLite `layout_type` value and in
    /// settings files.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            LayoutId::Qwerty => "qwerty",
            LayoutId::RemingtonGail => "gail",
            LayoutId::RemingtonCbi => "cbi",
            LayoutId::KrutiDev => "krutidev",
            LayoutId::Inscript => "inscript",
        }
    }

    /// Display name shown in the UI.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            LayoutId::Qwerty => "English QWERTY",
            LayoutId::RemingtonGail => "Remington GAIL",
            LayoutId::RemingtonCbi => "Remington CBI",
            LayoutId::KrutiDev => "Kruti Dev",
            LayoutId::Inscript => "InScript",
        }
    }

    /// Whether the layout types Devanagari as Unicode (GAIL, CBI, InScript).
    #[must_use]
    pub fn is_unicode_hindi(self) -> bool {
        matches!(
            self,
            LayoutId::RemingtonGail | LayoutId::RemingtonCbi | LayoutId::Inscript
        )
    }

    /// Parse a stable identifier back into a [`LayoutId`].
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::UnknownLayout`] for unrecognised input, so a
    /// corrupt settings file degrades gracefully instead of panicking
    /// (`RULES.MD` Rule 23).
    pub fn parse(raw: &str) -> radha_core::Result<Self> {
        match raw {
            "qwerty" => Ok(LayoutId::Qwerty),
            "gail" => Ok(LayoutId::RemingtonGail),
            "cbi" => Ok(LayoutId::RemingtonCbi),
            "krutidev" => Ok(LayoutId::KrutiDev),
            "inscript" => Ok(LayoutId::Inscript),
            other => Err(CoreError::UnknownLayout {
                requested: other.to_owned(),
            }),
        }
    }

    /// Every layout, in Home-screen presentation order.
    #[must_use]
    pub fn all() -> &'static [LayoutId] {
        &[
            LayoutId::Qwerty,
            LayoutId::RemingtonGail,
            LayoutId::RemingtonCbi,
            LayoutId::KrutiDev,
            LayoutId::Inscript,
        ]
    }
}

/// ASCII digits `0`..`9`, indexed by value.
pub const ASCII_DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// Devanagari digits `०`..`९`, indexed by value.
pub const DEVANAGARI_DIGITS: [&str; 10] = ["०", "१", "२", "३", "४", "५", "६", "७", "८", "९"];

/// Guidance for a single key, as surfaced by the Hindi Guide panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyGuidance {
    /// The physical key to press (its keycap legend), e.g. `"D"`.
    pub physical_key: String,
    /// What the layout produces, e.g. `"क"`.
    pub output: String,
    /// Which hand to use.
    pub hand: Hand,
    /// Which finger to use.
    pub finger: Finger,
    /// Which modifier layer the key is struck on.
    pub layer: Layer,
}

/// How a layout's typed text is encoded, which decides the session's unit
/// type: `char` for Latin and Legacy, `Akshar` for Unicode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// English text, one key one character.
    Latin,
    /// Unicode Devanagari in logical order, matched per akshar.
    Unicode,
    /// A legacy-font ASCII stream (Kruti Dev), matched per `char`.
    Legacy,
}

/// A keyboard layout: its table, sequence rules and finger mapping.
///
/// Implementors must be self-contained. See the crate-level note on layout
/// independence.
pub trait KeyboardLayout: Send + Sync {
    /// Which layout this is.
    fn id(&self) -> LayoutId;

    /// The layout's own key table.
    fn table(&self) -> &'static LayoutTable;

    /// How typed text is encoded.
    fn encoding(&self) -> Encoding;

    /// Hand and finger for a physical key. No default body: each layout
    /// states its own finger mapping (RULES 9).
    fn finger_for(&self, key: PhysicalKey) -> (Hand, Finger);

    /// Devanagari sequence rules (ligatures), for layouts that compose.
    fn rules(&self) -> Option<&'static SequenceRules> {
        None
    }

    /// An Alt + numpad code the layout accepts.
    fn alt_code(&self, _code: u16) -> Option<&'static AltCodeEntry> {
        None
    }

    /// The Esc dead-key layer (Esc, then a key), for layouts that have one.
    fn dead_keys(&self) -> Option<&'static LayoutTable> {
        None
    }

    /// What a numpad digit key types (`digit` 0..=9). The default is the
    /// ASCII digit; GAIL/CBI follow Caps Lock (on `0-9`, off `०-९`).
    fn numpad_digit(&self, digit: u8, _caps_lock: bool) -> Option<&'static str> {
        ASCII_DIGITS.get(usize::from(digit)).copied()
    }

    /// What Space types on a modifier layer (AltGr+Space). `None`: a plain
    /// space.
    fn space_chord(&self, _layer: Layer) -> Option<&'static str> {
        None
    }

    /// What Ctrl+Shift+`digit` types (InScript joiners), if anything.
    fn ctrl_shift_digit(&self, _digit: char) -> Option<&'static str> {
        None
    }

    /// Guidance for the single key that produces `output`, or `None` if no
    /// one key produces it. Space is the right thumb on every layout.
    fn guidance_for(&self, output: char) -> Option<KeyGuidance> {
        let (key, layer) = if output == ' ' {
            (SPACE, Layer::Base)
        } else {
            let mut buf = [0u8; 4];
            let wanted: &str = output.encode_utf8(&mut buf);
            let entry = self.table().producing(wanted).into_iter().next()?;
            (entry.key, entry.layer)
        };
        let (hand, finger) = self.finger_for(key);
        Some(KeyGuidance {
            physical_key: key.legend().to_owned(),
            output: output.to_string(),
            hand,
            finger,
            layer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_ids_round_trip() {
        for &id in LayoutId::all() {
            let parsed = LayoutId::parse(id.as_str()).expect("known id must parse");
            assert_eq!(parsed, id);
        }
    }

    #[test]
    fn unknown_layout_is_an_error_not_a_panic() {
        let err = LayoutId::parse("remington-nonsense").unwrap_err();
        assert!(matches!(err, CoreError::UnknownLayout { .. }));
        assert!(err.is_recoverable(), "a bad layout id must be survivable");
    }

    #[test]
    fn the_three_hindi_layouts_are_distinct_identities() {
        // Guards the GAIL != CBI != Kruti Dev rule at the type level.
        assert_ne!(LayoutId::RemingtonGail, LayoutId::RemingtonCbi);
        assert_ne!(LayoutId::RemingtonCbi, LayoutId::KrutiDev);
        assert_ne!(LayoutId::RemingtonGail, LayoutId::KrutiDev);

        assert_ne!(
            LayoutId::RemingtonGail.as_str(),
            LayoutId::RemingtonCbi.as_str()
        );
    }

    #[test]
    fn inscript_is_the_fifth_layout() {
        assert_eq!(LayoutId::all().len(), 5);
        assert_eq!(LayoutId::parse("inscript").ok(), Some(LayoutId::Inscript));
        assert!(LayoutId::Inscript.is_unicode_hindi());
        assert!(!LayoutId::KrutiDev.is_unicode_hindi());
    }
}
