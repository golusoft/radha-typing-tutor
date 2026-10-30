//! Layout table types, lookups and validation.
//!
//! Each layout owns one `const` array of [`KeyEntry`] in its own module
//! (`src/<layout>/table.rs`, `pub(super)`), so no layout can name another's
//! table. This module holds only the types and the data-free rules every
//! table must satisfy.

use radha_core::akshar::normalize;

use crate::error::LayoutError;
use crate::keys::{k, Keystroke, Layer, PhysicalKey, SPACE};
use crate::script::{is_consonant, is_independent_vowel, is_mark, NUKTA, VIRAMA};
use crate::LayoutId;

/// What role a key's output plays when it is composed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Plain text (QWERTY).
    Literal,
    /// One full consonant.
    Consonant,
    /// A half form: consonant(s) ending in virama.
    Half,
    /// A ready-made conjunct with no final virama (`त्र`, `श्र`, `रू`).
    Conjunct,
    /// An independent vowel.
    Vowel,
    /// A dependent sign typed after its consonant (incl. `ं ः ँ`).
    Matra,
    /// The short-i sign struck before its consonant (typewriter order).
    Preposed,
    /// The aa-sign key that also completes a preceding half form.
    Completer,
    /// `र्` struck after the syllable it rides.
    Reph,
    /// `्र` struck after its consonant.
    Rakar,
    /// A bare virama.
    Halant,
    /// A bare nukta.
    Nukta,
    /// A digit.
    Digit,
    /// Punctuation or a symbol.
    Punctuation,
}

/// One (key, layer) slot of a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEntry {
    /// The physical key.
    pub key: PhysicalKey,
    /// The modifier layer.
    pub layer: Layer,
    /// What the key emits: Unicode text, or for Kruti Dev the raw ASCII code.
    pub output: &'static str,
    /// The output's composition role.
    pub kind: TokenKind,
    /// Kruti Dev only: the Devanagari the font draws for `output`.
    pub meaning: Option<&'static str>,
}

impl KeyEntry {
    /// A Unicode or Latin entry.
    #[must_use]
    pub const fn new(
        key: PhysicalKey,
        layer: Layer,
        output: &'static str,
        kind: TokenKind,
    ) -> Self {
        KeyEntry {
            key,
            layer,
            output,
            kind,
            meaning: None,
        }
    }

    /// A legacy-font entry: raw `output` code plus the glyph's `meaning`.
    #[must_use]
    pub const fn legacy(
        key: PhysicalKey,
        layer: Layer,
        output: &'static str,
        meaning: &'static str,
        kind: TokenKind,
    ) -> Self {
        KeyEntry {
            key,
            layer,
            output,
            kind,
            meaning: Some(meaning),
        }
    }

    /// The keystroke for this slot.
    #[must_use]
    pub fn keystroke(&self) -> Keystroke {
        Keystroke::new(self.key, self.layer)
    }

    /// What the slot means as Devanagari/text: `meaning` when present.
    #[must_use]
    pub fn text(&self) -> &'static str {
        self.meaning.unwrap_or(self.output)
    }
}

/// A character reached with Alt + a numpad code instead of a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AltCodeEntry {
    /// The Windows code typed on the numpad (e.g. 161 for Alt+0161).
    pub code: u16,
    /// The character Windows inserts for it (Windows-1252 for a code typed
    /// with a leading zero, ASCII otherwise).
    pub ansi: char,
    /// The Devanagari (or punctuation) the font draws for it.
    pub meaning: &'static str,
    /// Whether the code must be typed with a leading zero (`Alt+0188`). Without
    /// it Windows uses the OEM (CP437) table and inserts a different character.
    pub leading_zero: bool,
}

impl AltCodeEntry {
    /// A Windows-1252 code typed with a leading zero (Kruti Dev).
    #[must_use]
    pub const fn ansi(code: u16, ansi: char, meaning: &'static str) -> Self {
        AltCodeEntry {
            code,
            ansi,
            meaning,
            leading_zero: true,
        }
    }

    /// An ASCII code typed without a leading zero (`Alt+33` = `!`).
    #[must_use]
    pub const fn ascii(code: u16, ascii: char, meaning: &'static str) -> Self {
        AltCodeEntry {
            code,
            ansi: ascii,
            meaning,
            leading_zero: false,
        }
    }

    /// The keys the typist presses: `Alt+0161` or `Alt+33`.
    #[must_use]
    pub fn keys(&self) -> String {
        if self.leading_zero {
            format!("Alt+{:04}", self.code)
        } else {
            format!("Alt+{}", self.code)
        }
    }
}

/// Short constructor for the const tables.
pub(crate) const fn e(key: u8, layer: Layer, output: &'static str, kind: TokenKind) -> KeyEntry {
    KeyEntry::new(k(key), layer, output, kind)
}

/// Short legacy constructor for the const tables.
pub(crate) const fn lg(
    key: u8,
    layer: Layer,
    output: &'static str,
    meaning: &'static str,
    kind: TokenKind,
) -> KeyEntry {
    KeyEntry::legacy(k(key), layer, output, meaning, kind)
}

/// Which validation profile a table follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableProfile {
    /// Output is the US character of the slot (QWERTY).
    Latin,
    /// Output is NFC Unicode (GAIL, CBI).
    Unicode,
    /// Output is the US character of the slot, with a Devanagari meaning
    /// (Kruti Dev).
    LegacyIdentity,
}

/// A layout's table: its id, validation profile and entries.
#[derive(Debug, Clone, Copy)]
pub struct LayoutTable {
    id: LayoutId,
    profile: TableProfile,
    entries: &'static [KeyEntry],
}

impl LayoutTable {
    /// Wrap a const table.
    #[must_use]
    pub const fn new(id: LayoutId, profile: TableProfile, entries: &'static [KeyEntry]) -> Self {
        LayoutTable {
            id,
            profile,
            entries,
        }
    }

    /// The layout this table belongs to.
    #[must_use]
    pub fn id(&self) -> LayoutId {
        self.id
    }

    /// The validation profile.
    #[must_use]
    pub fn profile(&self) -> TableProfile {
        self.profile
    }

    /// Every slot.
    #[must_use]
    pub fn entries(&self) -> &'static [KeyEntry] {
        self.entries
    }

    /// The entry for a keystroke, or `None` if the slot is unmapped.
    #[must_use]
    pub fn get(&self, stroke: Keystroke) -> Option<&'static KeyEntry> {
        self.entries
            .iter()
            .find(|e| e.key == stroke.key && e.layer == stroke.layer)
    }

    /// Entries whose output is exactly `output`, cheapest first (fewest
    /// modifiers, then board order).
    #[must_use]
    pub fn producing(&self, output: &str) -> Vec<&'static KeyEntry> {
        let mut found: Vec<&'static KeyEntry> =
            self.entries.iter().filter(|e| e.output == output).collect();
        found.sort_by_key(|e| cheapness(e));
        found
    }

    /// Every entry, cheapest first.
    #[must_use]
    pub fn cheapest_first(&self) -> Vec<&'static KeyEntry> {
        let mut all: Vec<&'static KeyEntry> = self.entries.iter().collect();
        all.sort_by_key(|e| cheapness(e));
        all
    }

    /// The keycap a virtual keyboard shows for a key on a layer (derived from
    /// the table, never kept as a second table). For Kruti Dev this is the
    /// glyph meaning.
    #[must_use]
    pub fn keycap(&self, key: PhysicalKey, layer: Layer) -> Option<&'static str> {
        self.get(Keystroke::new(key, layer)).map(KeyEntry::text)
    }

    /// For a direct-input layout (one key inserts its output, no
    /// composition): whether every character of `text` is some slot's whole
    /// output, so the text can be typed key by key. Whitespace is always
    /// typeable.
    #[must_use]
    pub fn covers_directly(&self, text: &str) -> bool {
        text.chars().all(|c| {
            c.is_whitespace()
                || self
                    .entries
                    .iter()
                    .any(|e| e.output.chars().eq(std::iter::once(c)))
        })
    }

    /// Check every rule in the module docs. The first broken rule is reported
    /// with the entry index and the rule id.
    ///
    /// # Errors
    ///
    /// [`LayoutError::InvalidTable`] naming the entry and the rule.
    pub fn validate(&self) -> Result<(), LayoutError> {
        let fail = |index: usize, detail: String| LayoutError::InvalidTable {
            layout: self.id.as_str(),
            index,
            detail,
        };
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.key == SPACE || entry.key.index().is_none() {
                return Err(fail(i, format!("S1 unknown key {:?}", entry.key)));
            }
            if self.entries[..i]
                .iter()
                .any(|p| p.key == entry.key && p.layer == entry.layer)
            {
                return Err(fail(i, "S2 duplicate slot".to_owned()));
            }
            if entry.output.is_empty() {
                return Err(fail(i, "S3 empty output".to_owned()));
            }
            let us_char = match entry.layer {
                Layer::Base => Some(entry.key.base_char()),
                Layer::Shift => Some(entry.key.shifted_char()),
                Layer::AltGr | Layer::ShiftAltGr => None,
            };
            let identity = us_char.is_some_and(|c| entry.output.chars().eq(std::iter::once(c)));
            match self.profile {
                TableProfile::Latin => {
                    if !identity || entry.kind != TokenKind::Literal || entry.meaning.is_some() {
                        return Err(fail(i, "P1 latin slot must be its US character".to_owned()));
                    }
                }
                TableProfile::LegacyIdentity => {
                    if !identity {
                        return Err(fail(
                            i,
                            "P2 legacy slot must be its US character".to_owned(),
                        ));
                    }
                    match entry.meaning {
                        Some(m) if !m.is_empty() => {
                            kind_rule(entry.kind, m).map_err(|rule| fail(i, rule.to_owned()))?
                        }
                        _ => return Err(fail(i, "P2 legacy slot needs a meaning".to_owned())),
                    }
                }
                TableProfile::Unicode => {
                    if normalize(entry.output) != entry.output {
                        return Err(fail(i, "P3 output must be NFC".to_owned()));
                    }
                    if entry.meaning.is_some() {
                        return Err(fail(i, "P3 unicode slot layer/meaning".to_owned()));
                    }
                    kind_rule(entry.kind, entry.output).map_err(|rule| fail(i, rule.to_owned()))?;
                }
            }
        }
        Ok(())
    }
}

/// Sort key for "fewest modifiers first, then board order".
fn cheapness(entry: &KeyEntry) -> (u8, usize) {
    (entry.layer.cost(), entry.key.index().unwrap_or(usize::MAX))
}

/// Kind rules K1-K9 on a slot's Devanagari text.
fn kind_rule(kind: TokenKind, text: &str) -> Result<(), &'static str> {
    let cps: Vec<char> = text.chars().collect();
    let first = cps.first().copied();
    let last = cps.last().copied();
    let single = |pred: fn(char) -> bool| cps.len() == 1 && first.is_some_and(pred);
    let ok = match kind {
        TokenKind::Half => first.is_some_and(is_consonant) && last == Some(VIRAMA) && cps.len() > 1,
        TokenKind::Reph => first.is_some_and(is_consonant) && last == Some(VIRAMA) && cps.len() > 1,
        TokenKind::Rakar => cps == ['\u{094D}', '\u{0930}'],
        TokenKind::Halant => cps == [VIRAMA],
        TokenKind::Nukta => cps == [NUKTA],
        TokenKind::Preposed | TokenKind::Completer | TokenKind::Matra => single(is_mark),
        TokenKind::Consonant => single(is_consonant),
        TokenKind::Conjunct => {
            first.is_some_and(is_consonant) && cps.len() > 1 && last != Some(VIRAMA)
        }
        TokenKind::Vowel => single(is_independent_vowel),
        TokenKind::Digit | TokenKind::Punctuation | TokenKind::Literal => !cps.is_empty(),
    };
    if ok {
        return Ok(());
    }
    Err(match kind {
        TokenKind::Half => "K1 half must be consonant(s) + virama",
        TokenKind::Reph => "K2 reph must be consonant + virama",
        TokenKind::Rakar => "K3 rakar must be exactly U+094D U+0930",
        TokenKind::Halant => "K4 halant must be exactly U+094D",
        TokenKind::Nukta => "K5 nukta must be exactly U+093C",
        TokenKind::Preposed | TokenKind::Completer | TokenKind::Matra => {
            "K6 sign must be exactly one mark"
        }
        TokenKind::Consonant => "K7 consonant must be exactly one consonant",
        TokenKind::Conjunct => "K8 conjunct must start with a consonant, >1 cp, no final virama",
        TokenKind::Vowel => "K9 vowel must be exactly one independent vowel",
        TokenKind::Digit | TokenKind::Punctuation | TokenKind::Literal => "S3 empty output",
    })
}
