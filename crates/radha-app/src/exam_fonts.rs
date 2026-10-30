//! Exam typography profiles for the typing screens.
//!
//! | Profile               | Passage and "Your typing" font                |
//! |-----------------------|-----------------------------------------------|
//! | English (QWERTY)      | Arial 400                                     |
//! | Hindi Unicode (GAIL, CBI, InScript) | Mangal 400                      |
//! | Hindi legacy (Kruti Dev 010) | the legacy key stream, as before       |
//!
//! Arial and Mangal are Microsoft fonts and Radha never ships their files.
//! They are used only when Windows has them installed: Slint resolves a
//! `font-family` name through the system font collection (DirectWrite) at
//! runtime, so naming the family is enough. Mangal belongs to the optional
//! "Hindi Supplemental Fonts" feature on Windows 10/11 and may be missing;
//! then the embedded Noto Sans Devanagari is used and the typing screen says
//! that Mangal is not installed. Without Arial the existing Latin passage
//! font (the embedded JetBrains Mono) stays.
//!
//! Detection is one file check per font at startup, never per key.

use std::path::{Path, PathBuf};

/// Windows family names (resolved by Slint from the installed fonts).
pub const ARIAL: &str = "Arial";
pub const MANGAL: &str = "Mangal";
/// Embedded fallbacks (`ui/theme/fonts.slint`).
pub const LATIN_FALLBACK: &str = "JetBrains Mono";
pub const DEVANAGARI_FALLBACK: &str = "Noto Sans Devanagari";

/// The regular-weight files Windows installs (`C:\Windows\Fonts`).
const ARIAL_FILE: &str = "arial.ttf";
const MANGAL_FILE: &str = "mangal.ttf";

/// Which exam fonts this PC has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExamFonts {
    pub arial: bool,
    pub mangal: bool,
}

impl ExamFonts {
    /// Look in the Windows font folders (machine-wide and per-user).
    #[must_use]
    pub fn detect() -> Self {
        Self::in_dirs(&font_dirs())
    }

    /// Look for the regular faces in `dirs`.
    #[must_use]
    pub fn in_dirs(dirs: &[PathBuf]) -> Self {
        ExamFonts {
            arial: has_file(dirs, ARIAL_FILE),
            mangal: has_file(dirs, MANGAL_FILE),
        }
    }

    /// English passages: Arial, or the embedded Latin passage font.
    #[must_use]
    pub fn latin(self) -> &'static str {
        if self.arial {
            ARIAL
        } else {
            LATIN_FALLBACK
        }
    }

    /// Hindi Unicode passages: Mangal, or the embedded Noto Sans Devanagari.
    #[must_use]
    pub fn devanagari(self) -> &'static str {
        if self.mangal {
            MANGAL
        } else {
            DEVANAGARI_FALLBACK
        }
    }
}

fn has_file(dirs: &[PathBuf], name: &str) -> bool {
    dirs.iter().any(|d| d.join(name).is_file())
}

/// `%WINDIR%\Fonts` and `%LOCALAPPDATA%\Microsoft\Windows\Fonts` (fonts a
/// user installed without admin rights, Windows 10 1809+).
fn font_dirs() -> Vec<PathBuf> {
    let windows = std::env::var_os("WINDIR")
        .or_else(|| std::env::var_os("SystemRoot"))
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let mut dirs = vec![windows.join("Fonts")];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(
            Path::new(&local)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts"),
        );
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_fonts_are_used_and_missing_ones_fall_back_to_the_embedded_faces() {
        let none = tempfile::tempdir().unwrap();
        let both = tempfile::tempdir().unwrap();
        std::fs::write(both.path().join("arial.ttf"), b"x").unwrap();
        std::fs::write(both.path().join("mangal.ttf"), b"x").unwrap();

        // No Hindi Supplemental Fonts, no Arial: the embedded fonts.
        let f = ExamFonts::in_dirs(&[none.path().to_path_buf()]);
        assert_eq!(
            f,
            ExamFonts {
                arial: false,
                mangal: false
            }
        );
        assert_eq!(f.latin(), "JetBrains Mono");
        assert_eq!(f.devanagari(), "Noto Sans Devanagari");

        // Either folder may hold them (machine-wide or per-user).
        let f = ExamFonts::in_dirs(&[none.path().to_path_buf(), both.path().to_path_buf()]);
        assert!(f.arial && f.mangal);
        assert_eq!(f.latin(), "Arial");
        assert_eq!(f.devanagari(), "Mangal");

        // A missing folder is not an error.
        let f = ExamFonts::in_dirs(&[none.path().join("absent")]);
        assert!(!f.arial && !f.mangal);
    }

    #[test]
    fn the_windows_font_folder_is_searched() {
        let dirs = font_dirs();
        assert!(dirs[0].ends_with("Fonts"), "{dirs:?}");
    }
}
