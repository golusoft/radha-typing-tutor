//! Kruti Dev sequence rules, used by the legacy-to-Unicode reconstruction.
//!
//! Kruti Dev's own list. It adds `ॉ = ा + ॅ` (render-confirmed `dkW` ->
//! `कॉ`, mapping doc §5b), placed after the three-part `ऑ` so `vkW` still
//! gives `ऑ`. Source: Golu `krutidev-keymap.js` `LIGATURES`.

use crate::compose::{Ligature, SequenceRules};

const LIGATURES: [Ligature; 10] = [
    Ligature::new("औ", &["अ", "\u{093E}", "\u{0948}"]),
    Ligature::new("ओ", &["अ", "\u{093E}", "\u{0947}"]),
    Ligature::new("ऑ", &["अ", "\u{093E}", "\u{0945}"]),
    Ligature::new("ई", &["इ", "र्"]),
    Ligature::new("\u{0949}", &["\u{093E}", "\u{0945}"]),
    Ligature::new("आ", &["अ", "\u{093E}"]),
    Ligature::new("ऐ", &["ए", "\u{0947}"]),
    Ligature::new("ऊ", &["उ", "\u{0941}"]),
    Ligature::new("\u{094C}", &["\u{093E}", "\u{0948}"]),
    Ligature::new("\u{094B}", &["\u{093E}", "\u{0947}"]),
];

pub(super) static RULES: SequenceRules = SequenceRules::new(&LIGATURES);
