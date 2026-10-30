//! Layout-agnostic input primitives.
//!
//! A keyboard layout turns one key press into one [`TextEdit`]: delete some
//! code points from the end of the typed text, then append some text. That is
//! the same shape as Golu's `ReplacePreviousCount` + `ProducedText`, and it is
//! all a Hindi typewriter layout needs to reorder a reph or complete a half
//! form, while a plain layout only ever inserts.
//!
//! [`Composition`] is the one hook the matcher needs from a layout: "is this
//! typed unit a legitimate state on the way to the expected unit?". A layout
//! answers it from its keystroke plans; the core never learns what a layout
//! is, so `radha-core` stays free of any dependency on `radha-layouts`.

use std::fmt::Debug;

/// One key press, expressed as an edit of the tail of the typed text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextEdit {
    /// Unicode scalar values removed from the end of the typed text (clamped
    /// to what exists).
    pub delete: usize,
    /// Text appended after the deletion.
    pub insert: String,
    /// The press was Backspace (counted as a backspace, never starts a
    /// session).
    pub backspace: bool,
}

impl TextEdit {
    /// Append `text`.
    #[must_use]
    pub fn insert(text: impl Into<String>) -> Self {
        TextEdit {
            delete: 0,
            insert: text.into(),
            backspace: false,
        }
    }

    /// Delete `delete` code points, then append `text`.
    #[must_use]
    pub fn replace(delete: usize, text: impl Into<String>) -> Self {
        TextEdit {
            delete,
            insert: text.into(),
            backspace: false,
        }
    }

    /// A Backspace press removing `delete` code points (`0` when it only
    /// cancelled a layout's pending preedit).
    #[must_use]
    pub fn backspace(delete: usize) -> Self {
        TextEdit {
            delete,
            insert: String::new(),
            backspace: true,
        }
    }

    /// A key press that changes no committed text (a preedit-only key).
    #[must_use]
    pub fn none() -> Self {
        TextEdit::default()
    }

    /// Apply the edit to `text`. Returns the code points actually deleted.
    pub fn apply_to(&self, text: &mut String) -> usize {
        let mut removed = 0;
        while removed < self.delete && text.pop().is_some() {
            removed += 1;
        }
        text.push_str(&self.insert);
        removed
    }
}

/// Layout knowledge the matcher consults for the last typed unit.
///
/// `is_intermediate` returns `true` when `typed` is a state a correct key
/// sequence for `expected` passes through (for example `य` on the way to
/// `र्य`, whose reph is struck last). Such a unit is kept pending instead of
/// being scored as wrong.
pub trait Composition<U>: Debug + Send + Sync {
    /// Whether `typed` is a legitimate intermediate state toward `expected`.
    fn is_intermediate(&self, typed: &U, expected: &U) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_clamps_the_deletion_and_appends() {
        let mut s = String::from("ab");
        assert_eq!(TextEdit::replace(5, "x").apply_to(&mut s), 2);
        assert_eq!(s, "x");
        let mut s = String::from("क्");
        assert_eq!(TextEdit::replace(1, "").apply_to(&mut s), 1);
        assert_eq!(s, "क");
    }

    #[test]
    fn constructors_set_the_backspace_flag_only_for_backspace() {
        assert!(TextEdit::backspace(1).backspace);
        assert!(!TextEdit::insert("a").backspace);
        assert_eq!(TextEdit::none(), TextEdit::replace(0, ""));
    }
}
