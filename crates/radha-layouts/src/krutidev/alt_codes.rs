//! Kruti Dev Alt + numpad characters and characters no key or code reaches.
//!
//! Source: RADHA production keyboard book 04 KD010-MAP-118..175 (SRC-07):
//! Windows-1252 codes, typed with the leading zero (Alt+188 without it uses
//! CP437 and inserts a different character). 0224/0230/0233 have
//! disagreeing third-party sources; the book keeps these values as
//! verified. Nothing here is guessed: a character absent from both lists has
//! no Kruti Dev spelling and the converter refuses it.

use crate::table::AltCodeEntry;

const fn a(code: u16, ansi: char, meaning: &'static str) -> AltCodeEntry {
    AltCodeEntry::ansi(code, ansi, meaning)
}

pub(super) const ALT_CODES: [AltCodeEntry; 58] = [
    a(131, '\u{0192}', "१"),
    a(132, '\u{201E}', "२"),
    a(133, '\u{2026}', "३"),
    a(134, '\u{2020}', "४"),
    a(135, '\u{2021}', "५"),
    a(136, '\u{02C6}', "६"),
    a(137, '\u{2030}', "७"),
    a(138, '\u{0160}', "८"),
    a(139, '\u{2039}', "९"),
    a(150, '\u{2013}', "दृ"),
    a(151, '\u{2014}', "कृ"),
    a(153, '\u{2122}', "न्न्"),
    a(161, '\u{00A1}', "\u{0901}"),
    a(165, '\u{00A5}', "ञ"),
    a(170, '\u{00AA}', "\u{094D}\u{0930}"),
    a(171, '\u{00AB}', "त्र्"),
    a(179, '\u{00B3}', "ङ"),
    a(182, '\u{00B6}', "फ्"),
    a(183, '\u{00B7}', "ऽ"),
    a(184, '\u{00B8}', "य्"),
    a(188, '\u{00BC}', "("),
    a(189, '\u{00BD}', ")"),
    a(190, '\u{00BE}', "="),
    a(191, '\u{00BF}', "{"),
    a(192, '\u{00C0}', "}"),
    a(193, '\u{00C1}', "प्र"),
    a(197, '\u{00C5}', "ऊ"),
    a(198, '\u{00C6}', "र्\u{093F}"),
    a(199, '\u{00C7}', "\u{093F}\u{0902}"),
    a(200, '\u{00C8}', "\u{0940}\u{0902}"),
    a(201, '\u{00C9}', "र्\u{093F}\u{0902}"),
    a(202, '\u{00CA}', "\u{0940}र्"),
    a(203, '\u{00CB}', "ध्"),
    a(204, '\u{00CC}', "द्द"),
    a(205, '\u{00CD}', "ट्ट"),
    a(206, '\u{00CE}', "ट्ठ"),
    a(207, '\u{00CF}', "ड्ड"),
    a(209, '\u{00D1}', "कृ"),
    a(210, '\u{00D2}', "भ"),
    a(211, '\u{00D3}', "\u{094D}य"),
    a(212, '\u{00D4}', "ड्ढ"),
    a(214, '\u{00D6}', "झ्"),
    a(216, '\u{00D8}', "क्र"),
    a(217, '\u{00D9}', "त्त्"),
    a(221, '\u{00DD}', "फ्र"),
    a(222, '\u{00DE}', "\u{201C}"),
    a(223, '\u{00DF}', "\u{201D}"),
    a(224, '\u{00E0}', "ह्न"),
    a(225, '\u{00E1}', "ह्य"),
    a(226, '\u{00E2}', "हृ"),
    a(227, '\u{00E3}', "ह्म"),
    a(228, '\u{00E4}', "क्त"),
    a(229, '\u{00E5}', "०"),
    a(230, '\u{00E6}', "द्र"),
    a(231, '\u{00E7}', "प्र"),
    a(233, '\u{00E9}', "न्न"),
    a(241, '\u{00F1}', "॰"),
    a(244, '\u{00F4}', "क्क"),
];

/// Characters with no key and no verified Alt code in Kruti Dev 010
/// (book 04: `ॐ` and `₹` are not supported).
pub(super) const UNRESOLVED: [(char, &str); 3] = [
    ('ॐ', "om: not supported by Kruti Dev 010 (book 04)"),
    ('₹', "rupee sign: not supported by Kruti Dev 010 (book 04)"),
    (
        '*',
        "asterisk: not found in the Alt+0128..0255 sweep of Kruti Dev 010",
    ),
];
