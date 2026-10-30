//! GAIL sequence rules: vowels and signs typed as a sequence of keys.
//!
//! GAIL's own copy (RULES 9: each layout owns its sequence rules). Source:
//! book 01 GAIL-MAP-126..154 (SRC-04 p.16), which includes every ligature
//! of Golu's `DevanagariComposer.Ligatures` and adds `ऍ`, `ॉ`, `ँ` and the
//! poonchh rule.

use crate::compose::{Ligature, SequenceRules};

const LIGATURES: [Ligature; 12] = [
    Ligature::new("औ", &["अ", "\u{093E}", "\u{0948}"]),
    Ligature::new("ओ", &["अ", "\u{093E}", "\u{0947}"]),
    Ligature::new("ऑ", &["अ", "\u{093E}", "\u{0945}"]),
    Ligature::new("ई", &["इ", "र्"]),
    Ligature::new("आ", &["अ", "\u{093E}"]),
    Ligature::new("ऐ", &["ए", "\u{0947}"]),
    Ligature::new("ऍ", &["ए", "\u{0945}"]),
    Ligature::new("ऊ", &["उ", "\u{0941}"]),
    Ligature::new("\u{094C}", &["\u{093E}", "\u{0948}"]),
    Ligature::new("\u{094B}", &["\u{093E}", "\u{0947}"]),
    Ligature::new("\u{0901}", &["\u{0945}", "\u{0902}"]),
    Ligature::new("\u{0949}", &["\u{093E}", "\u{0945}"]),
];

/// The poonchh rule (Shift+Q, SRC-04 p.16): `फ` struck right after `उ` is
/// `ऊ`, right after `र` is `रु`. To type `उफ`, strike `उ`, Space, Backspace,
/// then Shift+Q: the Space breaks the sequence.
const POONCHH: [Ligature; 2] = [
    Ligature::new("ऊ", &["उ", "फ"]),
    Ligature::new("रु", &["र", "फ"]),
];

pub(super) static RULES: SequenceRules = SequenceRules::with_sequences(&LIGATURES, &POONCHH);
