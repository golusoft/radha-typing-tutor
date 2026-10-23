//! # Radha Layouts
//!
//! Keyboard layout engine. Resolves a physical key press into the character (or
//! character sequence) a given layout produces, plus the hand/finger guidance
//! the Hindi Guide needs.
//!
//! ## Scope in Phase 0
//!
//! Scaffold only. Per the plan, this crate ships the [`KeyboardLayout`] trait
//! and a minimal QWERTY implementation to prove the seam compiles. The real
//! mappings land in Phase 7–8.
//!
//! ## Hard rule: layouts are independent
//!
//! `RULES.MD` Rule 9 and `PRD.md` Section 4.2:
//!
//! ```text
//! GAIL  ≠  CBI  ≠  Kruti Dev
//! ```
//!
//! Each layout gets its own module, its own mapping table, and its own tests.
//! The trait below exists so they share an *interface*, never a mapping. No
//! layout may `use` another layout's tables — reviewers should treat any such
//! import as a defect.

use radha_core::CoreError;

pub mod qwerty;

/// Which hand a key is struck with. Drives the Hindi Guide's hand hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    Left,
    Right,
}

/// Which finger strikes a key. Drives the Hindi Guide's finger hint and the
/// virtual keyboard's finger colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finger {
    Pinky,
    Ring,
    Middle,
    Index,
    Thumb,
}

/// Identifies one of Radha's four supported layouts.
///
/// Kept as a closed enum rather than a string so that adding a layout is a
/// deliberate, compiler-checked change across the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutId {
    /// English QWERTY — used by English Mastery, Typing Flow, Number + Punctuation.
    Qwerty,
    /// Remington GAIL (Unicode Devanagari).
    RemingtonGail,
    /// Remington CBI (Unicode Devanagari) — *not* a variant of GAIL.
    RemingtonCbi,
    /// Kruti Dev (legacy font encoding).
    KrutiDev,
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
        }
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
        ]
    }
}

/// Guidance for a single key, as surfaced by the Hindi Guide panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyGuidance {
    /// The physical key to press, e.g. `"C"`.
    pub physical_key: String,
    /// What the layout produces, e.g. `"क"`.
    pub output: String,
    /// Which hand to use.
    pub hand: Hand,
    /// Which finger to use.
    pub finger: Finger,
}

/// A keyboard layout: maps physical keys to output characters and guidance.
///
/// Implementors must be self-contained. See the crate-level note on layout
/// independence.
pub trait KeyboardLayout {
    /// Which layout this is.
    fn id(&self) -> LayoutId;

    /// Resolve a physical key into its guidance, or `None` if the layout does
    /// not map that key.
    fn guidance_for(&self, physical_key: char) -> Option<KeyGuidance>;
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
}
