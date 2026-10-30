//! Unicode Hindi -> Kruti Dev legacy key stream (import time).
//!
//! A Rust re-implementation of the algorithm in Golu
//! `tools/content-engine/krutidev-convert-core.js` `toKrutiDev`, over Kruti
//! Dev's own table only. Ordering rules (mapping doc §3):
//!
//! 1. the short i-sign `ि` moves to the front of its syllable;
//! 2. the reph `र्` opens a new group and is emitted at its end;
//! 3. a consonant + `्र` uses the full consonant + `z` (rakar), unless a
//!    dedicated conjunct key covers the run;
//! 4. a half form + the completer `k` gives the full consonant;
//! 5. whitespace closes groups.
//!
//! Input is NFC-normalised first. Characters with no key and no verified Alt
//! code are reported as failures and omitted, never guessed.

use std::collections::HashMap;
use std::sync::OnceLock;

use radha_core::akshar::normalize;

use super::alt_codes::{ALT_CODES, UNRESOLVED};
use super::rules::RULES;
use super::table::TABLE;
use crate::script::VIRAMA;
use crate::table::TokenKind;

/// Why a character could not be converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// Kruti Dev 010 has no glyph for it (mapping doc §6).
    Unresolved,
    /// No key, sequence or Alt code produces it.
    NoKey,
}

/// One character that was not converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvertFailure {
    /// Index of the character in the NFC input, in chars.
    pub char_index: usize,
    /// The character.
    pub ch: char,
    /// Why.
    pub reason: FailureReason,
}

/// How often each rule fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConvertStats {
    /// Pre-posed `ि` moved to the front of its syllable.
    pub preposed_moves: u32,
    /// Reph moved to the end of its syllable.
    pub reph_moves: u32,
    /// Full consonants typed as half form + completer.
    pub completer_uses: u32,
    /// Vowels/signs typed as a ligature sequence.
    pub ligature_uses: u32,
    /// Consonant + rakar substitutions.
    pub rakar_uses: u32,
    /// Devanagari digits typed as Latin digit keys.
    pub digit_substitutions: u32,
    /// Alt-code characters.
    pub alt_code_uses: u32,
}

/// The result of [`to_legacy`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Conversion {
    /// The legacy key stream.
    pub legacy: String,
    /// Characters not converted (the passage must not be stored if any).
    pub failures: Vec<ConvertFailure>,
    /// Rule counters.
    pub stats: ConvertStats,
}

/// How a reverse entry is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum How {
    Direct,
    Completed,
    Ligature,
    Digit,
    AltCode,
}

/// What a reverse entry is, for grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RevKind {
    Token(TokenKind),
    Ligature,
    AltCode(AltKind),
}

/// The composition role of an Alt-code character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AltKind {
    Matra,
    Consonant,
    Punct,
}

#[derive(Debug, Clone)]
struct RevEntry {
    unicode: Vec<char>,
    keys: String,
    kind: RevKind,
    how: How,
}

#[derive(Debug, Default)]
struct Reverse {
    /// Longest first.
    entries: Vec<RevEntry>,
    index: HashMap<String, usize>,
}

impl Reverse {
    fn get(&self, unicode: &str) -> Option<&RevEntry> {
        self.index.get(unicode).and_then(|&i| self.entries.get(i))
    }
}

/// The converter's own grouping set: exactly the JS `MARK_RANGES`
/// (U+0900-0903, U+093A-094F, U+0951-0957, U+0962-0963). Unlike the
/// composer's marks it includes nukta, avagraha and virama.
#[must_use]
pub fn is_grouping_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0900}'..='\u{0903}' | '\u{093A}'..='\u{094F}' | '\u{0951}'..='\u{0957}' | '\u{0962}'..='\u{0963}'
    )
}

/// The composition role of an Alt-code character's meaning.
pub(super) fn alt_kind(meaning: &str) -> AltKind {
    match meaning.chars().next() {
        Some('\u{0900}'..='\u{0903}' | '\u{093A}'..='\u{094F}') => AltKind::Matra,
        Some('\u{0904}'..='\u{0939}' | '\u{0958}'..='\u{0961}') => AltKind::Consonant,
        _ => AltKind::Punct,
    }
}

fn reverse() -> &'static Reverse {
    static REVERSE: OnceLock<Reverse> = OnceLock::new();
    REVERSE.get_or_init(build_reverse)
}

fn build_reverse() -> Reverse {
    let mut list: Vec<RevEntry> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut add = |list: &mut Vec<RevEntry>, uni: &str, keys: String, kind: RevKind, how: How| {
        if !seen.contains_key(uni) {
            seen.insert(uni.to_owned(), list.len());
            list.push(RevEntry {
                unicode: uni.chars().collect(),
                keys,
                kind,
                how,
            });
        }
    };
    for entry in &TABLE {
        add(
            &mut list,
            entry.text(),
            entry.output.to_owned(),
            RevKind::Token(entry.kind),
            How::Direct,
        );
    }
    for entry in &TABLE {
        let text = entry.text();
        if entry.kind == TokenKind::Half {
            if let Some(full) = text.strip_suffix(VIRAMA) {
                add(
                    &mut list,
                    full,
                    format!("{}k", entry.output),
                    RevKind::Token(TokenKind::Consonant),
                    How::Completed,
                );
            }
        }
    }
    for lig in RULES.ligatures() {
        let keys: Option<Vec<String>> = lig
            .parts
            .iter()
            .map(|p| {
                list.iter()
                    .find(|e| e.unicode.iter().copied().eq(p.chars()))
                    .map(|e| e.keys.clone())
            })
            .collect();
        if let Some(keys) = keys {
            add(
                &mut list,
                lig.result,
                keys.concat(),
                RevKind::Ligature,
                How::Ligature,
            );
        }
    }
    for (i, dev) in ('\u{0966}'..='\u{096F}').enumerate() {
        add(
            &mut list,
            &dev.to_string(),
            i.to_string(),
            RevKind::Token(TokenKind::Digit),
            How::Digit,
        );
    }
    // Book 04: `A` is the danda and `AA` the double danda.
    add(
        &mut list,
        "॥",
        "AA".to_owned(),
        RevKind::Token(TokenKind::Punctuation),
        How::Direct,
    );
    // Alt codes of one character (book 04 KD010-MAP-118..175). They spell a
    // character no key spells, and replace a spelling the book does not list
    // (a composer ligature such as `ऊ` = `mq`, or a Latin digit for a
    // Devanagari one). A key spelling the book verifies is kept (`भ` = `Hk`).
    // Multi-character glyphs (`िं`, `र्ि`) are typing shortcuts, not a
    // conversion target; the rakar glyph `ª` is placed by rule 3.
    for alt in ALT_CODES.iter().filter(|a| a.meaning.chars().count() == 1) {
        let entry = RevEntry {
            unicode: alt.meaning.chars().collect(),
            keys: alt.ansi.to_string(),
            kind: RevKind::AltCode(alt_kind(alt.meaning)),
            how: How::AltCode,
        };
        match list
            .iter()
            .position(|e| e.unicode.iter().copied().eq(alt.meaning.chars()))
        {
            Some(i) => {
                if let Some(slot) = list.get_mut(i) {
                    if matches!(slot.how, How::Ligature | How::Digit) {
                        *slot = entry;
                    }
                }
            }
            None => list.push(entry),
        }
    }
    list.sort_by_key(|e| std::cmp::Reverse(e.unicode.len()));
    let index = list
        .iter()
        .enumerate()
        .map(|(i, e)| (e.unicode.iter().collect::<String>(), i))
        .collect();
    Reverse {
        entries: list,
        index,
    }
}

/// One visual syllable being built.
#[derive(Debug, Default)]
struct Group {
    pre: String,
    keys: String,
    reph: String,
}

impl Group {
    fn render(&self) -> String {
        format!("{}{}{}", self.pre, self.keys, self.reph)
    }
}

/// Whether the char at `i` begins a new visual syllable.
fn starts_new(text: &[char], i: usize) -> bool {
    let Some(&c) = text.get(i) else {
        return true;
    };
    if is_grouping_mark(c) {
        return false;
    }
    !(i > 0 && text.get(i - 1) == Some(&VIRAMA))
}

fn match_at<'r>(rev: &'r Reverse, text: &[char], i: usize) -> Option<&'r RevEntry> {
    let rest = text.get(i..)?;
    rev.entries.iter().find(|e| rest.starts_with(&e.unicode))
}

/// Convert Unicode Hindi to the Kruti Dev key stream.
#[must_use]
pub fn to_legacy(unicode: &str) -> Conversion {
    let rev = reverse();
    let text: Vec<char> = normalize(unicode).chars().collect();
    let mut out = String::new();
    let mut failures = Vec::new();
    let mut stats = ConvertStats::default();
    let mut current: Option<Group> = None;

    fn flush(out: &mut String, current: &mut Option<Group>) {
        if let Some(g) = current.take() {
            out.push_str(&g.render());
        }
    }
    fn new_group<'g>(out: &mut String, current: &'g mut Option<Group>) -> &'g mut Group {
        flush(out, current);
        current.insert(Group::default())
    }
    fn open_group<'g>(
        out: &mut String,
        current: &'g mut Option<Group>,
        fresh: bool,
    ) -> &'g mut Group {
        if fresh || current.is_none() {
            return new_group(out, current);
        }
        current.get_or_insert_with(Group::default)
    }

    let mut i = 0;
    while let Some(&ch) = text.get(i) {
        if matches!(ch, ' ' | '\n' | '\t') {
            flush(&mut out, &mut current);
            out.push(ch);
            i += 1;
            continue;
        }
        if UNRESOLVED.iter().any(|&(c, _)| c == ch) {
            failures.push(ConvertFailure {
                char_index: i,
                ch,
                reason: FailureReason::Unresolved,
            });
            i += 1;
            continue;
        }
        let Some(m) = match_at(rev, &text, i) else {
            failures.push(ConvertFailure {
                char_index: i,
                ch,
                reason: FailureReason::NoKey,
            });
            i += 1;
            continue;
        };
        let len = m.unicode.len();

        // Rule 3: <consonant> ् र -> full consonant + 'z'.
        if m.unicode.last() == Some(&VIRAMA) && text.get(i + len) == Some(&'र') {
            let full: String = m.unicode.get(..len - 1).unwrap_or(&[]).iter().collect();
            if let Some(full) = rev.get(&full) {
                let fresh = starts_new(&text, i);
                let g = open_group(&mut out, &mut current, fresh);
                g.keys.push_str(&full.keys);
                // Book 04: under the rounded letters ट ठ ड ढ छ the rakar is
                // the `ª` glyph (Alt+0170), elsewhere `z`.
                let rounded = matches!(full.keys.as_str(), "V" | "B" | "M" | "<" | "N");
                g.keys.push(if rounded { '\u{00AA}' } else { 'z' });
                if full.how == How::Completed {
                    stats.completer_uses += 1;
                }
                stats.rakar_uses += 1;
                i += len + 1;
                continue;
            }
        }

        match m.how {
            How::Completed => stats.completer_uses += 1,
            How::Ligature => stats.ligature_uses += 1,
            How::Digit => stats.digit_substitutions += 1,
            How::Direct | How::AltCode => {}
        }

        match m.kind {
            RevKind::Token(TokenKind::Preposed) => {
                // Rule 1: the sign goes to the front of its syllable.
                let g = open_group(&mut out, &mut current, false);
                g.pre = m.keys.clone();
                stats.preposed_moves += 1;
            }
            RevKind::Token(TokenKind::Reph) => {
                // Rule 2: typed at the end of the syllable it rides.
                let g = new_group(&mut out, &mut current);
                g.reph = m.keys.clone();
                stats.reph_moves += 1;
            }
            RevKind::AltCode(kind) => {
                stats.alt_code_uses += 1;
                match kind {
                    AltKind::Matra => open_group(&mut out, &mut current, false)
                        .keys
                        .push_str(&m.keys),
                    AltKind::Consonant => {
                        let fresh = starts_new(&text, i);
                        open_group(&mut out, &mut current, fresh)
                            .keys
                            .push_str(&m.keys);
                    }
                    AltKind::Punct => {
                        flush(&mut out, &mut current);
                        out.push_str(&m.keys);
                    }
                }
            }
            RevKind::Token(TokenKind::Punctuation | TokenKind::Digit) => {
                flush(&mut out, &mut current);
                out.push_str(&m.keys);
            }
            RevKind::Token(TokenKind::Rakar) => {
                // Book 04: under ट ठ ड ढ छ the rakar is `ª` (Alt+0170).
                let g = open_group(&mut out, &mut current, false);
                let rounded = g.keys.ends_with(['V', 'B', 'M', '<', 'N']);
                if rounded {
                    g.keys.push('\u{00AA}');
                } else {
                    g.keys.push_str(&m.keys);
                }
            }
            RevKind::Token(_) | RevKind::Ligature => {
                let fresh = starts_new(&text, i);
                open_group(&mut out, &mut current, fresh)
                    .keys
                    .push_str(&m.keys);
            }
        }
        i += len;
    }
    flush(&mut out, &mut current);
    Conversion {
        legacy: out,
        failures,
        stats,
    }
}

/// What a Unicode passage should look like after a round trip: NFC, with
/// Since book 04 every character round-trips as itself: Devanagari digits
/// have their own Alt codes (KD010-MAP-118..126, 170), so the result is just
/// the NFC form.
#[must_use]
pub fn expected_round_trip(unicode: &str) -> String {
    normalize(unicode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping_mark_boundaries() {
        for c in [
            '\u{0900}', '\u{0903}', '\u{093A}', '\u{094F}', '\u{0951}', '\u{0957}', '\u{0962}',
            '\u{0963}',
        ] {
            assert!(is_grouping_mark(c), "{c:?}");
        }
        for c in [
            '\u{08FF}', '\u{0904}', '\u{0939}', '\u{0950}', '\u{0958}', '\u{0961}', '\u{0964}',
        ] {
            assert!(!is_grouping_mark(c), "{c:?}");
        }
    }

    #[test]
    fn rules_fire_individually() {
        assert_eq!(to_legacy("कि").legacy, "fd");
        assert_eq!(to_legacy("अर्थ").legacy, "vFkZ");
        assert_eq!(to_legacy("क्रम").legacy, "dze");
        assert_eq!(to_legacy("भारत").legacy, "Hkkjr");
        // Book 04 KD010-MAP-122: ५ is Alt+0135 (`‡`); the `5` key draws an
        // Arabic 5.
        assert_eq!(to_legacy("५").legacy, "\u{2021}");
        assert_eq!(to_legacy("5").legacy, "5");
        let c = to_legacy("क्रिया");
        assert_eq!(c.legacy, "fdz;k");
        assert_eq!((c.stats.rakar_uses, c.stats.preposed_moves), (1, 1));
    }

    #[test]
    fn book_rules_for_rounded_rakar_double_danda_and_alt_letters() {
        // Rakar under ट ठ ड ढ छ is ª (Alt+0170).
        assert_eq!(to_legacy("ट्र").legacy, "V\u{00AA}");
        assert_eq!(to_legacy("ड्रम").legacy, "M\u{00AA}e");
        assert_eq!(to_legacy("क्र").legacy, "dz");
        // AA is ॥; ऊ is Alt+0197 (no key spells it).
        assert_eq!(to_legacy("॥").legacy, "AA");
        assert_eq!(to_legacy("ऊ").legacy, "\u{00C5}");
        assert_eq!(to_legacy("ङ").legacy, "\u{00B3}");
        // A key spelling the book verifies wins over an Alt glyph (भ = Hk).
        assert_eq!(to_legacy("भ").legacy, "Hk");
        // The rakar glyph stays in the base when ि is stored before it.
        assert_eq!(to_legacy("ट्रि").legacy, "fV\u{00AA}");
        assert_eq!(super::super::to_unicode("fV\u{00AA}").text, "ट्रि");
    }

    #[test]
    fn refusals_are_reported_not_guessed() {
        for s in ["ॐ", "*", "a", "\r"] {
            let c = to_legacy(s);
            assert_eq!(c.failures.len(), 1, "{s:?}");
            assert!(c.legacy.is_empty());
        }
        assert_eq!(to_legacy("ॐ").failures[0].reason, FailureReason::Unresolved);
    }
}
