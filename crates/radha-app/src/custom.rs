//! Add Exercise (TY-01): passages the user pastes, validated for the layout
//! they will be typed on and stored locally in `custom-exercises.toml`.
//!
//! A custom passage is practice only: it is not an `exercises` row, so its
//! sessions are not written to the progress history.

use std::path::Path;

use radha_layouts::LayoutId;
use serde::{Deserialize, Serialize};

/// Longest passage accepted (characters).
pub const MAX_CHARS: usize = 20_000;
/// Longest title kept (characters).
pub const MAX_TITLE: usize = 80;
/// Most passages kept; the oldest is dropped first.
pub const MAX_STORED: usize = 200;

/// One stored custom passage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomExercise {
    pub title: String,
    /// `LayoutId::as_str`.
    pub layout: String,
    /// The text as typed on the layout (Kruti Dev: the legacy key stream).
    pub text: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    exercise: Vec<CustomExercise>,
}

/// Every stored passage; an absent or unreadable file is an empty list.
#[must_use]
pub fn load(path: &Path) -> Vec<CustomExercise> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match toml::from_str::<Store>(&raw) {
        // The file is user-editable: every passage is checked again for
        // its layout (as `prepare` would on Add), so a hand edit can never
        // put a character the layout cannot type, or an over-long passage,
        // in front of a learner. The newest MAX_STORED are kept.
        Ok(s) => {
            let mut kept: Vec<CustomExercise> = s
                .exercise
                .into_iter()
                .filter_map(|mut e| {
                    let layout = LayoutId::parse(&e.layout).ok()?;
                    match prepare(layout, &e.text) {
                        Ok(t) if t == e.text => {}
                        _ => {
                            log::warn!(
                                "Skipping custom passage {:?}: not typeable as stored",
                                e.title
                            );
                            return None;
                        }
                    }
                    e.title = title(&e.title);
                    Some(e)
                })
                .collect();
            let extra = kept.len().saturating_sub(MAX_STORED);
            kept.drain(..extra);
            kept
        }
        Err(e) => {
            log::warn!("{} is not readable ({e}); ignoring it", path.display());
            Vec::new()
        }
    }
}

/// Write `all` to `path`.
///
/// # Errors
///
/// The I/O or serialisation error, as text for the screen.
pub fn save(path: &Path, all: &[CustomExercise]) -> Result<(), String> {
    let store = Store {
        exercise: all.to_vec(),
    };
    let body = toml::to_string(&store).map_err(|e| e.to_string())?;
    crate::paths::write_replace(path, body.as_bytes()).map_err(|e| e.to_string())
}

/// The plain-keyboard form of typographic punctuation that word
/// processors and web pages put in pasted text.
fn plain_punctuation(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{2032}' => "'",
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => "\"",
        '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2212}' => "-",
        '\u{2026}' => "...",
        _ => return None,
    })
}

/// Replace typographic punctuation the layout cannot type with its plain
/// keyboard form, when the layout can type that (curly quotes, dashes and
/// the ellipsis from Word or a web page). Everything else is left as is.
fn plain_for(layout: LayoutId, text: &str) -> String {
    let typable = |s: &str| {
        if layout == LayoutId::Inscript {
            radha_layouts::inscript::can_type(s)
        } else {
            radha_content::import::typable_on(layout, s)
        }
    };
    let mut out = String::with_capacity(text.len());
    let mut buf = [0u8; 4];
    for c in text.chars() {
        match plain_punctuation(c) {
            Some(p) if !typable(c.encode_utf8(&mut buf)) && typable(p) => out.push_str(p),
            _ => out.push(c),
        }
    }
    out
}

/// Devanagari signs that belong to the akshar before them (matras, nukta,
/// virama, candrabindu, anusvara, visarga) and the joiners.
fn joins_previous(c: char) -> bool {
    matches!(c as u32,
        0x0900..=0x0903 | 0x093A..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963 | 0x200C | 0x200D)
}

/// Validate pasted `text` for `layout` and return what will be typed:
/// whitespace collapsed to single spaces; Unicode Hindi converted to the
/// Kruti Dev key stream on Kruti Dev.
///
/// # Errors
///
/// A message for the screen when the text is empty, too long, or has a
/// character the layout cannot type.
pub fn prepare(layout: LayoutId, text: &str) -> Result<String, String> {
    let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = if layout == LayoutId::KrutiDev {
        clean
    } else {
        plain_for(layout, &clean)
    };
    if clean.is_empty() {
        return Err("Paste or type the passage first".to_owned());
    }
    if clean.chars().count() > MAX_CHARS {
        return Err(format!(
            "The passage is too long (at most {MAX_CHARS} characters)"
        ));
    }
    let typed = if layout == LayoutId::KrutiDev
        && clean
            .chars()
            .any(|c| ('\u{0900}'..='\u{097F}').contains(&c))
    {
        let conv = radha_layouts::krutidev::to_legacy(&clean);
        if !conv.failures.is_empty() {
            return Err(
                "Some Hindi characters have no Kruti Dev 010 key; remove them and try again"
                    .to_owned(),
            );
        }
        conv.legacy
    } else {
        clean
    };
    let ok = if layout == LayoutId::Inscript {
        radha_layouts::inscript::can_type(&typed)
    } else {
        radha_content::import::typable_on(layout, &typed)
    };
    if !ok {
        let bad: String = typed
            .chars()
            .filter(|&c| {
                let s = c.to_string();
                if layout == LayoutId::Inscript {
                    !radha_layouts::inscript::can_type(&s)
                } else {
                    !radha_content::import::typable_on(layout, &s)
                }
            })
            .take(5)
            .collect();
        return Err(format!(
            "{} cannot type these characters: {bad}",
            layout.display_name()
        ));
    }
    Ok(typed)
}

/// A title for the list: trimmed, at most [`MAX_TITLE`] characters.
#[must_use]
pub fn title(text: &str) -> String {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        return "Custom passage".to_owned();
    }
    let chars: Vec<char> = t.chars().collect();
    if chars.len() <= MAX_TITLE {
        return t;
    }
    // Cut between aksharas, never inside one: a matra, nukta or virama
    // left without its consonant (or a half letter left hanging) is wrong
    // Hindi on screen.
    let mut end = MAX_TITLE;
    while end > 0
        && (chars.get(end).is_some_and(|&c| joins_previous(c))
            || chars
                .get(end - 1)
                .is_some_and(|&c| c == '\u{094D}' || c == '\u{200D}'))
    {
        end -= 1;
    }
    if end == 0 {
        return "Custom passage".to_owned();
    }
    chars[..end]
        .iter()
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_text_is_cleaned_and_checked_for_the_layout() {
        assert_eq!(
            prepare(LayoutId::Qwerty, "  Work is\n\tworship.  ").as_deref(),
            Ok("Work is worship.")
        );
        assert!(prepare(LayoutId::Qwerty, "   ").is_err());
        assert!(prepare(LayoutId::Qwerty, "राम").is_err(), "Hindi on QWERTY");
        assert!(prepare(LayoutId::Qwerty, &"a".repeat(MAX_CHARS + 1)).is_err());
    }

    #[test]
    fn hindi_text_is_checked_and_converted_for_kruti_dev() {
        assert_eq!(
            prepare(LayoutId::RemingtonGail, "राम  घर").as_deref(),
            Ok("राम घर")
        );
        assert!(prepare(LayoutId::RemingtonGail, "राम ☺").is_err());
        let kd = prepare(LayoutId::KrutiDev, "राम").unwrap();
        assert!(
            kd.is_ascii() || !kd.contains('र'),
            "{kd:?} is the key stream"
        );
    }

    #[test]
    fn the_store_round_trips_and_titles_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom-exercises.toml");
        assert!(load(&path).is_empty());
        let e = CustomExercise {
            title: title("  My   passage "),
            layout: "qwerty".into(),
            text: "the cat".into(),
        };
        save(&path, std::slice::from_ref(&e)).unwrap();
        assert_eq!(load(&path), vec![e]);
        std::fs::write(&path, "not toml [").unwrap();
        assert!(load(&path).is_empty());
        assert_eq!(title(""), "Custom passage");
        assert_eq!(title(&"x".repeat(200)).chars().count(), MAX_TITLE);
    }

    #[test]
    fn a_hand_edited_store_never_serves_untypeable_or_unbounded_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custom-exercises.toml");
        let mut body = String::from(
            "[[exercise]]\ntitle = \"ok\"\nlayout = \"qwerty\"\ntext = \"the cat\"\n\
             [[exercise]]\ntitle = \"hindi on qwerty\"\nlayout = \"qwerty\"\ntext = \"राम\"\n\
             [[exercise]]\ntitle = \"emoji\"\nlayout = \"gail\"\ntext = \"राम ☺\"\n",
        );
        body.push_str(&format!(
            "[[exercise]]\ntitle = \"long\"\nlayout = \"qwerty\"\ntext = \"{}\"\n",
            "a".repeat(MAX_CHARS + 1)
        ));
        std::fs::write(&path, body).unwrap();
        let got = load(&path);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].text, "the cat");
        // Only the newest MAX_STORED are kept.
        let many: Vec<CustomExercise> = (0..MAX_STORED + 5)
            .map(|i| CustomExercise {
                title: format!("p{i}"),
                layout: "qwerty".into(),
                text: format!("passage {i}"),
            })
            .collect();
        save(&path, &many).unwrap();
        let got = load(&path);
        assert_eq!(got.len(), MAX_STORED);
        assert_eq!(got[0].title, "p5");
    }

    #[test]
    fn titles_are_cut_between_aksharas() {
        // 79 x's then क्ष: a plain cut at 80 would leave "क्" (a half letter).
        let t = title(&format!("{}क्षमा", "x".repeat(79)));
        assert!(!t.ends_with('\u{094D}'), "{t:?}");
        assert_eq!(t, "x".repeat(79));
        // A matra is never separated from its consonant.
        let t = title(&format!("{}कि", "x".repeat(79)));
        assert_eq!(t, "x".repeat(79));
        let t = title(&"रा".repeat(60));
        assert!(t.ends_with('ा'), "{t:?}");
    }

    #[test]
    fn pasted_typographic_punctuation_becomes_typeable() {
        assert_eq!(
            prepare(LayoutId::Qwerty, "It’s “fine” — ok…").as_deref(),
            Ok("It's \"fine\" - ok...")
        );
        // Text that was already plain is untouched.
        assert_eq!(
            prepare(LayoutId::Qwerty, "It's 'fine'").as_deref(),
            Ok("It's 'fine'")
        );
    }
}
