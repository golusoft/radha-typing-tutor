//! Devanagari code point classes used by the composer and table validation.
//!
//! Script facts, not layout data. The Kruti Dev converter keeps its own,
//! different grouping set (`krutidev::convert::is_grouping_mark`) because it
//! answers a different question (visual grouping, not cluster structure).

/// The virama (halant), U+094D.
pub const VIRAMA: char = '\u{094D}';
/// The nukta, U+093C.
pub const NUKTA: char = '\u{093C}';
/// The short-i vowel sign, U+093F (pre-posed on typewriter layouts).
pub const SIGN_I: char = '\u{093F}';

/// A consonant letter (U+0915-0939, U+0958-095F).
#[must_use]
pub fn is_consonant(c: char) -> bool {
    matches!(c, '\u{0915}'..='\u{0939}' | '\u{0958}'..='\u{095F}')
}

/// An independent vowel (U+0904-0914, U+0950 `ॐ`, U+0960-0961).
#[must_use]
pub fn is_independent_vowel(c: char) -> bool {
    matches!(c, '\u{0904}'..='\u{0914}' | '\u{0950}' | '\u{0960}'..='\u{0961}')
}

/// A dependent sign: vowel signs and `ँ ं ः`. Nukta and virama are their own
/// classes and are not marks here.
#[must_use]
pub fn is_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0900}'..='\u{0903}'
            | '\u{093A}'..='\u{093B}'
            | '\u{093E}'..='\u{094C}'
            | '\u{094E}'..='\u{094F}'
            | '\u{0955}'..='\u{0957}'
            | '\u{0962}'..='\u{0963}'
    )
}

/// Whether `c` belongs to a cluster's base (consonants, independent vowels,
/// viramas, nuktas) rather than to its marks.
#[must_use]
pub fn is_base_part(c: char) -> bool {
    !is_mark(c)
}

/// Whether `text` starts a cluster: a consonant or an independent vowel.
#[must_use]
pub fn starts_cluster(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|c| is_consonant(c) || is_independent_vowel(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_are_disjoint_where_it_matters() {
        assert!(is_consonant('क') && !is_mark('क'));
        assert!(is_independent_vowel('ॐ'));
        assert!(is_mark('ि') && is_mark('ं') && is_mark('ॉ'));
        assert!(!is_mark(VIRAMA) && !is_mark(NUKTA));
        assert!(starts_cluster("क्ष") && starts_cluster("आ") && !starts_cluster("ि"));
    }
}
