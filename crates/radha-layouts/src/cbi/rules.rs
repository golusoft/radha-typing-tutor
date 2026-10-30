//! CBI sequence rules: vowels and signs typed as a sequence of keys.
//!
//! CBI's own copy (RULES 9: each layout owns its sequence rules; the data is
//! deliberately not shared with any other layout). Source: book 03
//! CBI-MAP-111..119 (`आ ओ औ ऑ ऐ ऍ ऊ रु ँ`), plus the matra ligatures and
//! `इ + र् = ई` of Golu's `DevanagariComposer.Ligatures` (golden coverage in
//! `RemingtonCbiGoldenTests`: `vk`, `dks`, `dkS`, `bZ`); without them CBI
//! could not type `ो ौ ॉ ई` at all.

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

/// The poonchh rule (Shift+Q): `फ` struck right after `उ` is `ऊ`
/// (CBI-MAP-117), right after `र` is `रु` (CBI-MAP-118).
const POONCHH: [Ligature; 2] = [
    Ligature::new("ऊ", &["उ", "फ"]),
    Ligature::new("रु", &["र", "फ"]),
];

pub(super) static RULES: SequenceRules = SequenceRules::with_sequences(&LIGATURES, &POONCHH);
