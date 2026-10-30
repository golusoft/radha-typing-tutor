//! The unit of comparison: what one "letter" of the target is.
//!
//! The matcher, scoring, classification and statistics are written once,
//! generic over [`TextUnit`], so English and Hindi share one alignment
//! algorithm instead of two matchers that could drift apart (RULES.MD Rule 36
//! spirit, applied to matching).
//!
//! - `char` is the unit for QWERTY and for legacy font-encoded streams (Kruti
//!   Dev), where one code point is one glyph slot.
//! - [`crate::akshar::Akshar`] is the unit for Unicode Hindi, where one
//!   visible letter can be several code points (RULES.MD Rule 35).
//!
//! For `char` every hook is the trivial identity (a `char` is never a proper
//! prefix of a different `char`), which is why switching the matcher to this
//! trait left every QWERTY result unchanged.

use std::borrow::Cow;
use std::fmt::Debug;
use std::hash::Hash;

/// One comparable unit of text.
pub trait TextUnit: Clone + Eq + Hash + Debug {
    /// Split `text` into units, in order. Must be lossless for `char`; for
    /// aksharas it is lossless over the NFC form of `text`.
    fn segment(text: &str) -> Vec<Self>;

    /// Whether this unit is a word separator.
    fn is_whitespace(&self) -> bool;

    /// Whether this unit is punctuation (see [`is_punctuation_char`]).
    fn is_punctuation(&self) -> bool;

    /// Whether `self` and `other` differ only in letter case.
    fn eq_ignore_case(&self, other: &Self) -> bool;

    /// Whether `self` is a strict prefix of `other`: typing can still turn it
    /// into `other`. Lets the matcher wait while a Hindi cluster is being built
    /// key by key (क → क् → क्ष) instead of flagging each step as wrong.
    fn is_proper_prefix_of(&self, other: &Self) -> bool;

    /// Code points in this unit. Speed is counted in code points, which is
    /// what Unicode keyboard layouts emit per key press (Golu WPM parity).
    fn code_points(&self) -> usize;

    /// Whether this unit is a control character (tab, newline, NUL). These
    /// never get a per-key statistics row: no keyboard key draws them.
    fn is_control(&self) -> bool;

    /// The unit as text, for display and fixture comparison.
    fn as_text(&self) -> Cow<'_, str>;
}

impl TextUnit for char {
    fn segment(text: &str) -> Vec<Self> {
        text.chars().collect()
    }

    fn is_whitespace(&self) -> bool {
        char::is_whitespace(*self)
    }

    fn is_punctuation(&self) -> bool {
        is_punctuation_char(*self)
    }

    fn eq_ignore_case(&self, other: &Self) -> bool {
        // Full Unicode lowercase, as Golu's `char.ToLowerInvariant` comparison.
        self.to_lowercase().eq(other.to_lowercase())
    }

    fn is_proper_prefix_of(&self, _other: &Self) -> bool {
        false
    }

    fn code_points(&self) -> usize {
        1
    }

    fn is_control(&self) -> bool {
        char::is_control(*self)
    }

    fn as_text(&self) -> Cow<'_, str> {
        Cow::Owned(self.to_string())
    }
}

/// Whether `c` is punctuation, for mistake classification.
///
/// A fixed subset of Unicode General Category P* (what .NET's
/// `char.IsPunctuation`, used by Golu's classifier, tests), kept as a table so
/// the rule is visible and std-only:
///
/// - ASCII `! " # % & ' ( ) * , - . / : ; ? @ [ \ ] _ { }`. The ASCII symbols
///   `$ + < = > ^ | ~` and the backtick are category S* (symbols), not
///   punctuation, exactly as in .NET.
/// - Latin-1 `¡ § « ¶ · » ¿`.
/// - General Punctuation U+2010–U+2027 and U+2030–U+205E (dashes, quotes,
///   ellipsis, per-mille …).
/// - Devanagari danda `।` (U+0964) and double danda `॥` (U+0965).
#[must_use]
pub fn is_punctuation_char(c: char) -> bool {
    matches!(
        c,
        '!' | '"'
            | '#'
            | '%'
            | '&'
            | '\''
            | '('
            | ')'
            | '*'
            | ','
            | '-'
            | '.'
            | '/'
            | ':'
            | ';'
            | '?'
            | '@'
            | '['
            | '\\'
            | ']'
            | '_'
            | '{'
            | '}'
            | '\u{00A1}'
            | '\u{00A7}'
            | '\u{00AB}'
            | '\u{00B6}'
            | '\u{00B7}'
            | '\u{00BB}'
            | '\u{00BF}'
            | '\u{2010}'..='\u{2027}'
            | '\u{2030}'..='\u{205E}'
            | '\u{0964}'
            | '\u{0965}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_units_are_code_points() {
        assert_eq!(<char as TextUnit>::segment("ab c"), ['a', 'b', ' ', 'c']);
        assert_eq!('a'.code_points(), 1);
        assert!(!'a'.is_proper_prefix_of(&'b'));
        assert!(!'a'.is_proper_prefix_of(&'a'));
    }

    #[test]
    fn case_comparison_is_unicode_aware() {
        assert!('a'.eq_ignore_case(&'A'));
        assert!('É'.eq_ignore_case(&'é'));
        assert!(!'a'.eq_ignore_case(&'b'));
    }

    #[test]
    fn punctuation_table_matches_the_dotnet_split() {
        for c in [
            '.', ',', '!', '?', ';', ':', '\'', '"', '-', '(', ')', '।', '॥', '…', '—',
        ] {
            assert!(is_punctuation_char(c), "{c:?} is punctuation");
        }
        for c in [
            '$', '+', '<', '=', '>', '^', '`', '|', '~', 'a', '1', ' ', 'क',
        ] {
            assert!(!is_punctuation_char(c), "{c:?} is not punctuation");
        }
    }

    #[test]
    fn control_characters_are_detected() {
        assert!('\t'.is_control());
        assert!('\n'.is_control());
        assert!('\0'.is_control());
        assert!(!' '.is_control());
    }
}
