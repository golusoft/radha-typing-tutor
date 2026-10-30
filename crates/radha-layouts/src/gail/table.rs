//! Remington GAIL key table: 47 base + 47 Shift + 13 AltGr + 5 AltGr+Shift
//! slots, the Esc dead-key layer and the ASCII Alt codes.
//!
//! Source of truth: RADHA production keyboard book 01 (Remington GAIL,
//! Microsoft Hindi Indic Input 3 profile, 162 records), which agrees with
//! Golu `docs/gail-keyboard-diagram.md` on every Normal/Shift slot. Digits
//! are ASCII on the number row (GAIL-MAP-002..011).

use crate::keys::Layer::{AltGr as G, Base as B, Shift as S, ShiftAltGr as GS};
use crate::table::TokenKind::{
    Completer, Conjunct as Cj, Consonant as C, Digit as D, Halant, Half as H, Matra as M, Nukta,
    Preposed, Punctuation as P, Rakar, Reph, Vowel as V,
};
use crate::table::{e, AltCodeEntry, KeyEntry};

pub(super) const TABLE: [KeyEntry; 112] = [
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
    e(b'f', B, "\u{093F}", Preposed),
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
    // AltGr / AltGr+Shift (Book 01 GAIL-MAP-108..125, Keyman SIL v1.1.1
    // layer; it does not collide with the Indic Input 3 Normal/Shift slots).
    e(b'`', G, "\u{200C}", P),
    e(b'`', GS, "\u{200D}", P),
    e(b'1', GS, "ङ", C),
    e(b'-', G, "ञ", C),
    e(b'q', G, "फ्", H),
    e(b'w', G, "\u{0944}", M),
    e(b'r', GS, "त्त्", H),
    e(b'y', G, "द्द", Cj),
    e(b'y', GS, "द्भ", Cj),
    e(b's', G, "भ", C),
    e(b's', GS, "ष्ट", Cj),
    e(b'd', G, "ध", C),
    e(b'd', GS, "ष्ठ", Cj),
    e(b'f', G, "य्", H),
    e(b'b', G, "ई", V),
    e(b'm', G, "ऊ", V),
    e(b'm', GS, "ड्ड", Cj),
    e(b',', GS, "ढ्ढ", Cj),
];

/// The Esc dead-key layer: Esc, then the key (Book 01 GAIL-MAP-095..107,
/// SRC-04 p.15). Esc then `[` or `]` is unverified (GAIL-MAP-162) and stays
/// unmapped. Nukta letters are stored decomposed (consonant + U+093C), the
/// NFC form.
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

/// Windows ASCII Alt codes (Book 01 GAIL-MAP-155..158, SRC-11).
pub(super) const ALT_CODES: [AltCodeEntry; 4] = [
    AltCodeEntry::ascii(33, '!', "!"),
    AltCodeEntry::ascii(43, '+', "+"),
    AltCodeEntry::ascii(61, '=', "="),
    AltCodeEntry::ascii(63, '?', "?"),
];
