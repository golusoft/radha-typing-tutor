//! Remington CBI key table: 47 base + 47 Shift slots, the Esc dead-key
//! layer and the ASCII Alt codes.
//!
//! Source of truth: RADHA production keyboard book 03 (Remington CBI,
//! Microsoft Hindi Indic Input 2/3 CBI profile, 120 records). Its Normal and
//! Shift table is the same key for key as the GAIL book's; the defining
//! difference is that the short i-sign `ि` (key `f`) is typed after its
//! consonant or conjunct (CBI-MAP-108..110). CBI has no AltGr layer. The
//! legacy standalone CBI vendor keys (CBI-MAP-120) are a registered
//! conflict and are not implemented.

use crate::keys::Layer::{Base as B, Shift as S};
use crate::table::TokenKind::{
    Completer, Conjunct as Cj, Consonant as C, Digit as D, Halant, Half as H, Matra as M, Nukta,
    Punctuation as P, Rakar, Reph, Vowel as V,
};
use crate::table::{e, AltCodeEntry, KeyEntry};

pub(super) const TABLE: [KeyEntry; 94] = [
    // Number row, base.
    e(b'`', B, "\u{093C}", Nukta),
    e(b'1', B, "1", D),
    e(b'2', B, "2", D),
    e(b'3', B, "3", D),
    e(b'4', B, "4", D),
    e(b'5', B, "5", D),
    e(b'6', B, "6", D),
    e(b'7', B, "7", D),
    e(b'8', B, "8", D),
    e(b'9', B, "9", D),
    e(b'0', B, "0", D),
    e(b'-', B, ";", P),
    e(b'=', B, "\u{0943}", M),
    // Number row, Shift.
    e(b'`', S, "द्य", Cj),
    e(b'1', S, "।", P),
    e(b'2', S, "/", P),
    e(b'3', S, "\u{0903}", M),
    e(b'4', S, "*", P),
    e(b'5', S, "-", P),
    e(b'6', S, "\u{2018}", P),
    e(b'7', S, "\u{2019}", P),
    e(b'8', S, "द्ध", Cj),
    e(b'9', S, "त्र", Cj),
    e(b'0', S, "ऋ", V),
    e(b'-', S, ".", P),
    e(b'=', S, "\u{094D}", Halant),
    // Top row, base.
    e(b'q', B, "\u{0941}", M),
    e(b'w', B, "\u{0942}", M),
    e(b'e', B, "म", C),
    e(b'r', B, "त", C),
    e(b't', B, "ज", C),
    e(b'y', B, "ल", C),
    e(b'u', B, "न", C),
    e(b'i', B, "प", C),
    e(b'o', B, "व", C),
    e(b'p', B, "च", C),
    e(b'[', B, "ख्", H),
    e(b']', B, ",", P),
    e(b'\\', B, "(", P),
    // Top row, Shift.
    e(b'q', S, "फ", C),
    e(b'w', S, "\u{0945}", M),
    e(b'e', S, "म्", H),
    e(b'r', S, "त्", H),
    e(b't', S, "ज्", H),
    e(b'y', S, "ल्", H),
    e(b'u', S, "न्", H),
    e(b'i', S, "प्", H),
    e(b'o', S, "व्", H),
    e(b'p', S, "च्", H),
    e(b'[', S, "क्ष्", H),
    e(b']', S, "द्व", Cj),
    e(b'\\', S, ")", P),
    // Home row, base.
    e(b'a', B, "\u{0902}", M),
    e(b's', B, "\u{0947}", M),
    e(b'd', B, "क", C),
    e(b'f', B, "\u{093F}", M),
    e(b'g', B, "ह", C),
    e(b'h', B, "\u{0940}", M),
    e(b'j', B, "र", C),
    e(b'k', B, "\u{093E}", Completer),
    e(b'l', B, "स", C),
    e(b';', B, "य", C),
    e(b'\'', B, "श्", H),
    // Home row, Shift.
    e(b'a', S, "\u{093E}", Completer),
    e(b's', S, "\u{0948}", M),
    e(b'd', S, "क्", H),
    e(b'f', S, "थ्", H),
    e(b'g', S, "ळ", C),
    e(b'h', S, "भ्", H),
    e(b'j', S, "श्र", Cj),
    e(b'k', S, "ज्ञ", Cj),
    e(b'l', S, "स्", H),
    e(b';', S, "रू", Cj),
    e(b'\'', S, "ष्", H),
    // Bottom row, base.
    e(b'z', B, "\u{094D}\u{0930}", Rakar),
    e(b'x', B, "ग", C),
    e(b'c', B, "ब", C),
    e(b'v', B, "अ", V),
    e(b'b', B, "इ", V),
    e(b'n', B, "द", C),
    e(b'm', B, "उ", V),
    e(b',', B, "ए", V),
    e(b'.', B, "ण्", H),
    e(b'/', B, "ध्", H),
    // Bottom row, Shift.
    e(b'z', S, "र्", Reph),
    e(b'x', S, "ग्", H),
    e(b'c', S, "ब्", H),
    e(b'v', S, "ट", C),
    e(b'b', S, "ठ", C),
    e(b'n', S, "छ", C),
    e(b'm', S, "ड", C),
    e(b',', S, "ढ", C),
    e(b'.', S, "झ", C),
    e(b'/', S, "घ्", H),
];

/// The Esc dead-key layer: Esc, then the key (Book 03 CBI-MAP-095..107,
/// SRC-04 p.17). Nukta letters are stored decomposed, the NFC form.
pub(super) const DEAD: [KeyEntry; 13] = [
    e(b'3', B, "-", P),
    e(b'4', B, "ऋ", V),
    e(b'5', B, "\u{0943}", M),
    e(b'7', B, "\u{00F7}", P),
    e(b'8', B, "\u{00D7}", P),
    e(b'i', B, "\u{092B}\u{093C}", Cj),
    e(b'p', B, "\u{0922}\u{093C}", Cj),
    e(b'a', B, "\u{0916}\u{093C}", Cj),
    e(b'x', B, "\u{0917}\u{093C}", Cj),
    e(b'c', B, "\u{091C}\u{093C}", Cj),
    e(b'v', B, "!", P),
    e(b'b', B, "*", P),
    e(b'm', B, "\u{0921}\u{093C}", Cj),
];

/// Windows ASCII Alt codes (Book 03 chapter 17, SRC-11).
pub(super) const ALT_CODES: [AltCodeEntry; 4] = [
    AltCodeEntry::ascii(33, '!', "!"),
    AltCodeEntry::ascii(43, '+', "+"),
    AltCodeEntry::ascii(61, '=', "="),
    AltCodeEntry::ascii(63, '?', "?"),
];
