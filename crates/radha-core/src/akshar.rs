//! Akshar (Hindi letter) segmentation for grapheme-aware matching.
//!
//! RULES.MD Rule 35: no Hindi comparison may run on UTF-16 units, bytes or
//! code points. One visible akshar such as `क्षे` is four code points; scoring
//! it per code point would turn one slip into several errors and show a
//! half-built conjunct as wrong while the typist is still building it.
//!
//! # The rule set (decided in `docs/hindi-grapheme-decision.md`)
//!
//! 1. **NFC** both target and typed text, so the two spellings of a nukta
//!    letter (`U+0958` and `U+0915 U+093C`) are the same akshar.
//! 2. **Extended grapheme clusters** from `unicode-segmentation` 1.13.3
//!    (Unicode 17). Its GB9c rule keeps a conjunct (consonant + virama +
//!    consonant, with optional ZWJ) in one cluster, which older grapheme
//!    definitions split.
//! 3. **R1, baseless marks.** UAX #29 lets a combining mark attach to *any*
//!    preceding character, so `" ं"` (space + anusvara) is one extended
//!    cluster. In an isolated-matra drill that would make one keystroke look
//!    like it did nothing. When a cluster starts with a non-letter (space,
//!    punctuation, digit, symbol) or with a mark, and the rest are all
//!    combining marks, the first code point is split off. Ported from Golu's
//!    `IndicGraphemeSegmenter` (defect 1).
//!
//! Golu's second fix (a virama must not swallow a following space) is not
//! needed: GB9c only joins a following *consonant*.

use std::borrow::Cow;

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use crate::unit::{is_punctuation_char, TextUnit};

/// One akshar: an NFC grapheme cluster after the Indic post-rule.
///
/// Boxed text rather than `String`: units are created once per evaluation and
/// never grow, so the smaller handle is the honest type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Akshar(Box<str>);

impl Akshar {
    /// The akshar's text (NFC).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `text` in Unicode Normalization Form C.
#[must_use]
pub fn normalize(text: &str) -> String {
    text.nfc().collect()
}

/// Split `text` into aksharas, as strings over its NFC form.
///
/// Lossless: the concatenation of the result equals `normalize(text)`.
#[must_use]
pub fn segment(text: &str) -> Vec<String> {
    let nfc = normalize(text);
    let mut out = Vec::new();
    for cluster in nfc.graphemes(true) {
        let mut chars = cluster.chars();
        let Some(first) = chars.next() else {
            continue;
        };
        let rest = chars.as_str();
        if !rest.is_empty() && !is_base(first) && rest.chars().all(is_combining_mark) {
            // R1: a baseless mark is its own akshar, not part of the space,
            // digit or punctuation before it.
            out.push(first.to_string());
            out.push(rest.to_string());
        } else {
            out.push(cluster.to_string());
        }
    }
    out
}

/// Whether `c` can carry a mark: a letter that is not itself a mark. Matches
/// Golu's `char.IsLetter` test (matras are Alphabetic in Unicode but are
/// combining marks, so they are not bases).
fn is_base(c: char) -> bool {
    c.is_alphabetic() && !is_combining_mark(c)
}

impl TextUnit for Akshar {
    fn segment(text: &str) -> Vec<Self> {
        segment(text)
            .into_iter()
            .map(|s| Akshar(s.into_boxed_str()))
            .collect()
    }

    fn is_whitespace(&self) -> bool {
        !self.0.is_empty() && self.0.chars().all(char::is_whitespace)
    }

    fn is_punctuation(&self) -> bool {
        !self.0.is_empty() && self.0.chars().all(is_punctuation_char)
    }

    fn eq_ignore_case(&self, other: &Self) -> bool {
        self.0.to_lowercase() == other.0.to_lowercase()
    }

    fn is_proper_prefix_of(&self, other: &Self) -> bool {
        other.0.len() > self.0.len() && other.0.starts_with(&*self.0)
    }

    fn code_points(&self) -> usize {
        self.0.chars().count()
    }

    fn is_control(&self) -> bool {
        !self.0.is_empty() && self.0.chars().all(char::is_control)
    }

    fn as_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conjuncts_stay_in_one_akshar() {
        assert_eq!(segment("क्षेत्र"), ["क्षे", "त्र"]);
        assert_eq!(segment("नमस्ते"), ["न", "म", "स्ते"]);
    }

    #[test]
    fn a_baseless_mark_is_split_off_the_space_before_it() {
        assert_eq!(segment("ं ं"), ["ं", " ", "ं"]);
    }

    #[test]
    fn both_nukta_spellings_converge() {
        assert_eq!(segment("\u{0958}"), segment("\u{0915}\u{093C}"));
    }

    #[test]
    fn prefixes_are_strict() {
        let units = <Akshar as TextUnit>::segment("क्ष");
        let ka = <Akshar as TextUnit>::segment("क");
        assert!(ka[0].is_proper_prefix_of(&units[0]));
        assert!(!units[0].is_proper_prefix_of(&units[0]));
        assert!(!units[0].is_proper_prefix_of(&ka[0]));
    }

    #[test]
    fn segmentation_is_lossless() {
        for text in ["राष्ट्रीय विद्यालय", "ं े ि", "a b", "\r\n"]
        {
            assert_eq!(segment(text).concat(), normalize(text));
        }
    }
}
