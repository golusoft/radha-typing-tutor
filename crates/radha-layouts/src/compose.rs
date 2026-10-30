//! The composer: one keystroke in, one [`TextEdit`] of the typed text out.
//!
//! Data-free: it reads the layout's own table and sequence rules.
//!
//! - **Direct mode** (QWERTY, Kruti Dev): a key inserts its output.
//! - **Devanagari mode** (GAIL, CBI): the key's role ([`TokenKind`]) decides
//!   how it changes the *tail akshar* (the last akshar of the last word).
//!   Committed text is NFC and in logical order. A pre-posed sign (GAIL `f`)
//!   is held as preedit until its consonant exists; a reph struck after its
//!   syllable and a completer after a half form are tail replacements.
//!
//! The composer keeps no history except the pending pre-posed sign, and
//! reads the session's typed text on every key, so backspace or a pause can
//! never desynchronise it.

use radha_core::akshar::{normalize, segment};
use radha_core::TextEdit;

use crate::keys::{Keystroke, Layer, SPACE};
use crate::script::{is_consonant, is_mark, starts_cluster, VIRAMA};
use crate::table::{KeyEntry, TokenKind};
use crate::KeyboardLayout;

/// A vowel or sign typed as a sequence of keys (`आ` = `अ` + `ा`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ligature {
    /// The precomposed result (one code point).
    pub result: &'static str,
    /// The parts, in typed order.
    pub parts: &'static [&'static str],
}

impl Ligature {
    /// A ligature.
    #[must_use]
    pub const fn new(result: &'static str, parts: &'static [&'static str]) -> Self {
        Ligature { result, parts }
    }

    /// The parts joined.
    #[must_use]
    pub fn joined(&self) -> String {
        self.parts.concat()
    }
}

/// A layout's Devanagari sequence rules.
#[derive(Debug, Clone, Copy)]
pub struct SequenceRules {
    ligatures: &'static [Ligature],
    /// Keystroke sequences that join only when struck back to back (the
    /// poonchh rule: `उ` then `फ` is `ऊ`). Never expanded or recomposed, so
    /// they cannot rewrite text typed earlier.
    sequences: &'static [Ligature],
}

impl SequenceRules {
    /// Rules from a ligature list.
    #[must_use]
    pub const fn new(ligatures: &'static [Ligature]) -> Self {
        SequenceRules {
            ligatures,
            sequences: &[],
        }
    }

    /// Rules from a ligature list plus back-to-back keystroke sequences.
    #[must_use]
    pub const fn with_sequences(
        ligatures: &'static [Ligature],
        sequences: &'static [Ligature],
    ) -> Self {
        SequenceRules {
            ligatures,
            sequences,
        }
    }

    /// The ligatures, in declaration order.
    #[must_use]
    pub fn ligatures(&self) -> &'static [Ligature] {
        self.ligatures
    }

    /// The back-to-back keystroke sequences, in declaration order.
    #[must_use]
    pub fn sequences(&self) -> &'static [Ligature] {
        self.sequences
    }

    /// Join the last two keystrokes' text when they form a ligature or a
    /// sequence. `from` is the byte offset where the previous keystroke's
    /// text starts: a join may not reach further back than that.
    fn join_last(&self, text: &str, from: usize) -> Option<String> {
        let tail = text.get(from..)?;
        let mut best: Option<(&Ligature, usize)> = None;
        // Allocation-free: this runs on every key of a Devanagari layout.
        for lig in self.ligatures.iter().chain(self.sequences.iter()) {
            let Some(len) = suffix_len(tail, lig.parts) else {
                continue;
            };
            if best.is_none_or(|(_, l)| len > l) {
                best = Some((lig, len));
            }
        }
        let (lig, len) = best?;
        let head = text.get(..text.len() - len)?;
        Some(normalize(&format!("{head}{}", lig.result)))
    }

    /// The ligatures longest first (code points of the joined parts), ties in
    /// declaration order.
    #[must_use]
    pub fn ordered(&self) -> Vec<&'static Ligature> {
        let mut all: Vec<&'static Ligature> = self.ligatures.iter().collect();
        all.sort_by_key(|l| std::cmp::Reverse(l.joined().chars().count()));
        all
    }

    /// Replace every ligature's parts with its result, longest first.
    #[must_use]
    pub fn apply(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for lig in self.ordered() {
            let parts = lig.joined();
            if out.contains(&parts) {
                out = out.replace(&parts, lig.result);
            }
        }
        out
    }

    /// Replace every ligature result with its parts.
    #[must_use]
    pub fn expand(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            let mut buf = [0u8; 4];
            let s: &str = c.encode_utf8(&mut buf);
            match self.ligatures.iter().find(|l| l.result == s) {
                Some(lig) => out.push_str(&lig.joined()),
                None => out.push(c),
            }
        }
        out
    }

    /// Base (consonants, vowels, viramas, nuktas) in order, then marks in
    /// order, then ligatures, then NFC.
    #[must_use]
    pub fn recompose(&self, text: &str) -> String {
        let (base, marks) = split(text);
        normalize(&self.apply(&format!("{base}{marks}")))
    }
}

/// Byte length of `parts` joined, when `text` ends with them.
fn suffix_len(text: &str, parts: &[&str]) -> Option<usize> {
    let mut end = text.len();
    for part in parts.iter().rev() {
        let start = end.checked_sub(part.len())?;
        if text.get(start..end)? != *part {
            return None;
        }
        end = start;
    }
    Some(text.len() - end)
}

/// Split cluster text into its base and its marks, each in typed order.
fn split(text: &str) -> (String, String) {
    let base: String = text.chars().filter(|&c| !is_mark(c)).collect();
    let marks: String = text.chars().filter(|&c| is_mark(c)).collect();
    (base, marks)
}

/// What one key does to the committed text, plus the preedit to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyOutput {
    /// The edit to apply to the typed text.
    pub edit: TextEdit,
    /// Text shown but not committed (GAIL's pending pre-posed sign).
    pub preedit: Option<&'static str>,
}

/// A pre-posed sign waiting for its consonant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pending {
    /// The sign (`ि`).
    text: &'static str,
    /// Byte offset in the committed text: the sign belongs to the cluster
    /// that starts here.
    anchor: usize,
}

/// What was struck.
#[derive(Debug, Clone, Copy)]
enum Action {
    Space,
    Key(&'static KeyEntry),
}

/// The longest multi-key sequence the composer remembers (`v k s` = `ओ`).
pub const SEQUENCE_KEYS: usize = 3;

/// One remembered keystroke: the byte range of the committed text it
/// rewrote (`before` = length before the key, `after` = length after it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Struck {
    before: usize,
    after: usize,
}

/// The longest-match buffer: the last [`SEQUENCE_KEYS`] keystrokes struck
/// back to back. Fixed size and `Copy`, so the hot path never allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct SequenceBuffer {
    keys: [Struck; SEQUENCE_KEYS],
    len: usize,
}

impl SequenceBuffer {
    fn last(&self) -> Option<Struck> {
        self.len
            .checked_sub(1)
            .and_then(|i| self.keys.get(i).copied())
    }

    /// Whether the last remembered key ended exactly where the text ends now.
    fn continues(&self, committed_len: usize) -> bool {
        self.last().is_some_and(|s| s.after == committed_len)
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn push(&mut self, s: Struck) {
        if self.len == SEQUENCE_KEYS {
            self.keys.rotate_left(1);
            self.len -= 1;
        }
        if let Some(slot) = self.keys.get_mut(self.len) {
            *slot = s;
            self.len += 1;
        }
    }

    fn pop(&mut self) {
        self.len = self.len.saturating_sub(1);
    }

    /// Backspace rolled the text back to `new_len` bytes: forget every key
    /// whose text is entirely gone and trim the last one that survives.
    fn roll_back(&mut self, new_len: usize) {
        while self.last().is_some_and(|s| s.before >= new_len) {
            self.pop();
        }
        if let Some(i) = self.len.checked_sub(1) {
            if let Some(slot) = self.keys.get_mut(i) {
                slot.after = new_len;
            }
        }
    }
}

/// Turns keystrokes into edits for one layout.
#[derive(Clone)]
pub struct Composer {
    layout: &'static dyn KeyboardLayout,
    rules: Option<&'static SequenceRules>,
    pending: Option<Pending>,
    /// Esc was struck on a layout with a dead-key layer: the next key is
    /// read from that layer.
    dead: bool,
    seq: SequenceBuffer,
}

impl std::fmt::Debug for Composer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Composer")
            .field("layout", &self.layout.id())
            .field("pending", &self.pending)
            .field("dead", &self.dead)
            .finish()
    }
}

impl Composer {
    /// A composer for `layout`: Devanagari mode when the layout has sequence
    /// rules, Direct mode otherwise.
    #[must_use]
    pub fn new(layout: &'static dyn KeyboardLayout) -> Self {
        Composer {
            layout,
            rules: layout.rules(),
            pending: None,
            dead: false,
            seq: SequenceBuffer::default(),
        }
    }

    /// The pending preedit, if any.
    #[must_use]
    pub fn preedit(&self) -> Option<&'static str> {
        self.pending.map(|p| p.text)
    }

    /// Drop any pending state.
    pub fn reset(&mut self) {
        self.pending = None;
        self.dead = false;
        self.seq.clear();
    }

    /// Whether Esc armed the dead-key layer.
    #[must_use]
    pub fn dead_key_armed(&self) -> bool {
        self.dead
    }

    /// Esc: arm the dead-key layer. `false` (nothing armed) when the layout
    /// has no dead-key layer.
    pub fn arm_dead_key(&mut self) -> bool {
        self.dead = self.layout.dead_keys().is_some();
        self.dead
    }

    /// Disarm the dead-key layer. Returns whether it was armed.
    pub fn cancel_dead_key(&mut self) -> bool {
        std::mem::take(&mut self.dead)
    }

    /// Strike a key. `committed` is the session's typed text. `None` when the
    /// slot is unmapped on this layout (after Esc: on its dead-key layer).
    pub fn press(&mut self, stroke: Keystroke, committed: &str) -> Option<KeyOutput> {
        let dead = std::mem::take(&mut self.dead);
        let action = if stroke.key == SPACE {
            if stroke.layer != Layer::Base {
                if let Some(text) = self.layout.space_chord(stroke.layer) {
                    return Some(self.insert(text, committed));
                }
            }
            Action::Space
        } else if dead {
            // The book names the key ("Esc then V"); Shift or Caps Lock on a
            // letter still means that key.
            let entry = self.layout.dead_keys().and_then(|t| {
                t.get(stroke)
                    .or_else(|| t.get(Keystroke::new(stroke.key, Layer::Base)))
            });
            let Some(entry) = entry else {
                log::debug!("Esc then {stroke:?} is not mapped");
                return None;
            };
            Action::Key(entry)
        } else {
            Action::Key(self.layout.table().get(stroke)?)
        };
        let Some(rules) = self.rules else {
            let text = match action {
                Action::Space => " ",
                Action::Key(entry) => entry.output,
            };
            return Some(self.insert(text, committed));
        };
        let continues = self.seq.continues(committed.len());
        let from = self.seq.last().map_or(0, |s| s.before);
        let mut new = self.press_devanagari(action, committed, rules);
        if continues && matches!(action, Action::Key(_)) {
            if let Some(joined) = rules.join_last(&new, from) {
                new = joined;
            }
        }
        if !continues {
            self.seq.clear();
        }
        match action {
            Action::Space => self.seq.clear(),
            Action::Key(_) => self.seq.push(Struck {
                before: committed.len(),
                after: new.len(),
            }),
        }
        Some(KeyOutput {
            edit: diff(committed, &new),
            preedit: self.preedit(),
        })
    }

    /// Insert `text` as typed. Unicode layouts keep the last word NFC (a
    /// nukta after its consonant composes where Unicode composes it).
    fn insert(&mut self, text: &str, committed: &str) -> KeyOutput {
        self.seq.clear();
        if self.layout.encoding() != crate::Encoding::Unicode {
            return KeyOutput {
                edit: TextEdit::insert(text),
                preedit: None,
            };
        }
        let word_start = committed
            .char_indices()
            .rev()
            .find(|&(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8());
        let tail = committed.get(word_start..).unwrap_or("");
        KeyOutput {
            edit: diff(tail, &normalize(&format!("{tail}{text}"))),
            preedit: None,
        }
    }

    /// Backspace: cancels an armed dead key or a pending preedit first, else
    /// removes one code point and rolls the sequence buffer back. `None` when
    /// there is nothing to delete.
    pub fn backspace(&mut self, committed: &str) -> Option<KeyOutput> {
        if std::mem::take(&mut self.dead) {
            return Some(KeyOutput {
                edit: TextEdit::backspace(0),
                preedit: self.preedit(),
            });
        }
        if self.pending.take().is_some() {
            self.seq.pop();
            return Some(KeyOutput {
                edit: TextEdit::backspace(0),
                preedit: None,
            });
        }
        let last = committed.chars().next_back()?;
        if self.seq.continues(committed.len()) {
            self.seq.roll_back(committed.len() - last.len_utf8());
        } else {
            self.seq.clear();
        }
        Some(KeyOutput {
            edit: TextEdit::backspace(1),
            preedit: None,
        })
    }

    /// Enter: a line break on every layout.
    pub fn enter(&mut self, committed: &str) -> KeyOutput {
        self.pending = None;
        self.dead = false;
        self.insert("\n", committed)
    }

    /// A numpad digit key (`digit` 0..=9). `None` for a value out of range.
    pub fn numpad_digit(
        &mut self,
        digit: u8,
        caps_lock: bool,
        committed: &str,
    ) -> Option<KeyOutput> {
        let text = self.layout.numpad_digit(digit, caps_lock)?;
        self.dead = false;
        Some(self.insert(text, committed))
    }

    /// Ctrl+Shift+`digit`. `None` unless the layout maps the chord.
    pub fn ctrl_shift_digit(&mut self, digit: char, committed: &str) -> Option<KeyOutput> {
        let text = self.layout.ctrl_shift_digit(digit)?;
        Some(self.insert(text, committed))
    }

    /// An Alt + numpad code as typed (`"0188"`, `"33"`). A leading zero
    /// selects the Windows-1252 table, none the ASCII/OEM table; a code typed
    /// the wrong way is refused rather than inserting the wrong character.
    pub fn alt_numpad(&mut self, digits: &str, committed: &str) -> Option<KeyOutput> {
        if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let code: u16 = digits.parse().ok()?;
        let leading_zero = digits.len() > 1 && digits.starts_with('0');
        let entry = self.layout.alt_code(code)?;
        if entry.leading_zero != leading_zero {
            log::debug!("Alt+{digits} needs the form {}", entry.keys());
            return None;
        }
        self.alt_code(code, committed)
    }

    /// An Alt + numpad code. `None` unless the layout accepts the code.
    pub fn alt_code(&mut self, code: u16, committed: &str) -> Option<KeyOutput> {
        let Some(entry) = self.layout.alt_code(code) else {
            log::debug!(
                "alt code {code} is not mapped on {}",
                self.layout.id().as_str()
            );
            return None;
        };
        self.dead = false;
        let mut buf = [0u8; 4];
        let text: &str = entry.ansi.encode_utf8(&mut buf);
        Some(self.insert(text, committed))
    }

    fn press_devanagari(
        &mut self,
        action: Action,
        committed: &str,
        rules: &SequenceRules,
    ) -> String {
        let kind = effective_kind(action, committed, rules);
        if kind == Some(TokenKind::Preposed) {
            let mut text = committed.to_owned();
            if let Some(p) = self.pending.take() {
                if valid_anchor(p.anchor, &text) {
                    text = flush(p, &text, None, rules);
                }
            }
            let sign = match action {
                Action::Key(entry) => entry.output,
                Action::Space => "",
            };
            self.pending = Some(Pending {
                text: sign,
                anchor: text.len(),
            });
            return text;
        }
        let Some(p) = self.pending.take() else {
            return base_edit(committed, action, rules);
        };
        if !valid_anchor(p.anchor, committed) {
            log::debug!("pre-posed sign dropped: committed text changed under it");
            return base_edit(committed, action, rules);
        }
        if is_release(kind) {
            let new = base_edit(committed, action, rules);
            let start = common_prefix_bytes(committed, &new);
            let anchor = settle_anchor(&new, p.anchor.min(start));
            let x = new.get(anchor..).unwrap_or("");
            let head = new.get(..anchor).unwrap_or("");
            if releasable(x) {
                return format!("{head}{}", attach(x, p.text, rules));
            }
            self.pending = Some(Pending { anchor, ..p });
            return new;
        }
        flush(p, committed, Some(action), rules)
    }
}

/// The key's role for this tail: a completer with no half form to complete
/// acts as a plain sign. `None` for Space.
fn effective_kind(action: Action, committed: &str, rules: &SequenceRules) -> Option<TokenKind> {
    let Action::Key(entry) = action else {
        return None;
    };
    if entry.kind == TokenKind::Completer {
        let (_, tail) = split_tail(committed);
        let completes = starts_cluster(tail) && split(&rules.expand(tail)).0.ends_with(VIRAMA);
        if !completes {
            return Some(TokenKind::Matra);
        }
    }
    Some(entry.kind)
}

/// Kinds that can give a waiting pre-posed sign its consonant.
fn is_release(kind: Option<TokenKind>) -> bool {
    matches!(
        kind,
        Some(
            TokenKind::Consonant
                | TokenKind::Half
                | TokenKind::Conjunct
                | TokenKind::Halant
                | TokenKind::Nukta
                | TokenKind::Rakar
                | TokenKind::Completer
                | TokenKind::Reph
        )
    )
}

/// A consonant cluster whose last code point is not a virama.
fn releasable(x: &str) -> bool {
    x.chars().next().is_some_and(is_consonant) && !x.ends_with(VIRAMA)
}

fn valid_anchor(anchor: usize, text: &str) -> bool {
    anchor <= text.len() && text.is_char_boundary(anchor)
}

/// Put `sign` first among `x`'s marks and recompose.
fn attach(x: &str, sign: &str, rules: &SequenceRules) -> String {
    let (base, marks) = split(&rules.expand(x));
    normalize(&rules.apply(&format!("{base}{sign}{marks}")))
}

/// Resolve a pending sign before a key that cannot carry it, then apply
/// the key (`action == None` only applies the resolution).
fn flush(p: Pending, text: &str, action: Option<Action>, rules: &SequenceRules) -> String {
    let anchor = p.anchor.min(text.len());
    let x = text.get(anchor..).unwrap_or("");
    let head = text.get(..anchor).unwrap_or("");
    let apply = |t: &str| match action {
        Some(a) => base_edit(t, a, rules),
        None => t.to_owned(),
    };
    // (a) A cluster is still waiting (a half form): the sign joins it.
    if starts_cluster(x) {
        return apply(&format!("{head}{}", attach(x, p.text, rules)));
    }
    if x.is_empty() {
        // (b) A vowel starts the cluster: the sign follows it (Golu parity).
        if let Some(Action::Key(entry)) = action {
            if entry.kind == TokenKind::Vowel {
                return normalize(&format!("{}{}", apply(text), p.text));
            }
        }
        // (c) At text start or after a space the bare sign is its own akshar.
        if head.is_empty() || head.ends_with(char::is_whitespace) {
            return apply(&format!("{text}{}", p.text));
        }
    }
    // (d) After a complete akshar: discarding keeps that akshar correct.
    log::trace!("pre-posed sign discarded: no cluster to attach to");
    apply(text)
}

/// Move `anchor` back to the start of its akshar when the text after it does
/// not start a cluster (an edit rewrote text before the anchor).
fn settle_anchor(text: &str, anchor: usize) -> usize {
    let anchor = anchor.min(text.len());
    let x = text.get(anchor..).unwrap_or("");
    if x.is_empty() || starts_cluster(x) {
        return anchor;
    }
    let head = text.get(..anchor).unwrap_or("");
    let word_start = head
        .char_indices()
        .rev()
        .find(|&(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let word = text.get(word_start..).unwrap_or("");
    let segs = segment(word);
    if segs.concat() != word {
        return anchor;
    }
    let mut at = word_start;
    let mut best = word_start;
    for s in &segs {
        if at > anchor {
            break;
        }
        best = at;
        at += s.len();
    }
    best
}

/// `(prefix, tail)`: the tail is the last akshar of the last word.
fn split_tail(text: &str) -> (&str, &str) {
    let word_start = text
        .char_indices()
        .rev()
        .find(|&(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let word = text.get(word_start..).unwrap_or("");
    let segs = segment(word);
    if segs.concat() != word {
        return (text, "");
    }
    let tail_len = segs.last().map_or(0, String::len);
    let cut = text.len().saturating_sub(tail_len);
    match (text.get(..cut), text.get(cut..)) {
        (Some(prefix), Some(tail)) => (prefix, tail),
        _ => (text, ""),
    }
}

/// The key's own edit, as if nothing were pending.
fn base_edit(text: &str, action: Action, rules: &SequenceRules) -> String {
    let entry = match action {
        Action::Space => return format!("{text} "),
        Action::Key(entry) => entry,
    };
    let out = entry.output;
    let (prefix, tail) = split_tail(text);
    let cluster = starts_cluster(tail);
    let expanded = rules.expand(tail);
    let base_virama = cluster && split(&expanded).0.ends_with(VIRAMA);
    let join = |s: String| format!("{prefix}{}", rules.recompose(&s));
    match entry.kind {
        TokenKind::Completer if base_virama => {
            // Drop the base's final virama: the half form becomes full.
            let (mut base, marks) = split(&expanded);
            base.pop();
            join(format!("{base}{marks}"))
        }
        TokenKind::Completer
        | TokenKind::Matra
        | TokenKind::Nukta
        | TokenKind::Rakar
        | TokenKind::Halant
            if cluster =>
        {
            join(format!("{expanded}{out}"))
        }
        TokenKind::Reph if cluster => {
            if expanded.chars().next().is_some_and(is_consonant) {
                join(format!("{out}{expanded}"))
            } else {
                join(format!("{expanded}{out}"))
            }
        }
        TokenKind::Consonant | TokenKind::Half | TokenKind::Conjunct if base_virama => {
            join(format!("{expanded}{out}"))
        }
        _ => normalize(&format!("{text}{out}")),
    }
}

/// Byte length of the common prefix, on a char boundary.
fn common_prefix_bytes(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|&((_, ca), cb)| ca != cb)
        .map_or_else(|| a.len().min(b.len()), |((i, _), _)| i)
}

/// The single tail edit that turns `old` into `new`.
#[must_use]
pub(crate) fn diff(old: &str, new: &str) -> TextEdit {
    let prefix = common_prefix_bytes(old, new);
    let delete = old.get(prefix..).map_or(0, |s| s.chars().count());
    TextEdit::replace(delete, new.get(prefix..).unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_is_the_minimal_tail_edit() {
        assert_eq!(diff("क्", "क"), TextEdit::replace(1, ""));
        assert_eq!(diff("काय", "कार्य"), TextEdit::replace(1, "र्य"));
        assert_eq!(diff("", "a"), TextEdit::insert("a"));
    }

    #[test]
    fn split_tail_takes_the_last_akshar_of_the_last_word() {
        assert_eq!(split_tail("नम स्ते"), ("नम ", "स्ते"));
        assert_eq!(split_tail("कार"), ("का", "र"));
        assert_eq!(split_tail("क "), ("क ", ""));
        assert_eq!(split_tail(""), ("", ""));
    }

    #[test]
    fn ligature_order_is_longest_first() {
        static L: [Ligature; 3] = [
            Ligature::new("\u{094C}", &["\u{093E}", "\u{0948}"]),
            Ligature::new("औ", &["अ", "\u{093E}", "\u{0948}"]),
            Ligature::new("आ", &["अ", "\u{093E}"]),
        ];
        let rules = SequenceRules::new(&L);
        let order: Vec<&str> = rules.ordered().iter().map(|l| l.result).collect();
        assert_eq!(order, ["औ", "\u{094C}", "आ"]);
        assert_eq!(rules.apply("अ\u{093E}\u{0948}"), "औ");
        assert_eq!(rules.expand("औ"), "अ\u{093E}\u{0948}");
    }
}
