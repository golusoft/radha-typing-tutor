//! Shared helpers for the radha-layouts integration tests.
#![allow(dead_code)]

use std::path::PathBuf;

use radha_layouts::{load_layout, Composer, KeyboardLayout, Keystroke, LayoutId};

/// The crate's `tests/` directory.
pub fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// A layout by its stable slug.
pub fn layout(slug: &str) -> &'static dyn KeyboardLayout {
    let id = LayoutId::parse(slug).unwrap_or_else(|e| panic!("bad slug {slug}: {e}"));
    load_layout(id).unwrap_or_else(|e| panic!("{slug} failed to load: {e}"))
}

/// Type `keys` (each char is the US character a key produces; U+0008 is
/// Backspace), then optionally one AltGr key, through a fresh composer.
/// Returns the committed text and the preedit. Unmapped keys do nothing.
pub fn type_keys(
    layout: &'static dyn KeyboardLayout,
    keys: &str,
    altgr_after: Option<char>,
) -> (String, Option<&'static str>) {
    let mut composer = Composer::new(layout);
    let mut text = String::new();
    for c in keys.chars() {
        let out = if c == '\u{8}' {
            composer.backspace(&text)
        } else {
            let stroke = Keystroke::from_us_char(c, false)
                .unwrap_or_else(|| panic!("{c:?} is not on a US board"));
            composer.press(stroke, &text)
        };
        if let Some(out) = out {
            out.edit.apply_to(&mut text);
        }
    }
    if let Some(c) = altgr_after {
        let stroke = Keystroke::from_us_char(c, true).unwrap_or_else(|| panic!("{c:?}"));
        if let Some(out) = composer.press(stroke, &text) {
            out.edit.apply_to(&mut text);
        }
    }
    (text, composer.preedit())
}

/// SplitMix64, the same deterministic generator as radha-core's property
/// tests.
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Whether a string is 64 lowercase hex digits.
pub fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}
