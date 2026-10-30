//! Persisted application settings.
//!
//! Stored as TOML at `<config-dir>/settings.toml` — human-readable and
//! hand-editable, which makes support ("open this file and tell me what it
//! says") straightforward.
//!
//! # Phase 0 scope
//!
//! Only what the app shell itself needs: theme, window geometry, and the
//! first-launch flag. The full Settings screen (`TASK.MD` Section 27) adds sound,
//! font, typing, keyboard, and guide preferences in a later phase; they slot into
//! this struct as new fields.
//!
//! # Forward compatibility
//!
//! Every field is `#[serde(default)]`, so a settings file written by an older
//! build still loads once new fields are added — the absent fields take their
//! defaults instead of failing the whole parse. Without this, adding a single
//! setting would silently reset every user's preferences.

use std::path::Path;

use radha_core::{CoreError, Result};
use serde::{Deserialize, Serialize};

/// Minimum window width, per `PRD.md` Section 3.1 (1024x768 target display).
///
/// Must stay in step with `WindowMetrics.min-width` in
/// `ui/theme/metrics.slint`, which applies the same limit at the UI layer. Slint
/// needs the value when declaring the window; Rust needs it when validating
/// settings.toml, and neither can read the other's copy.
pub const MIN_WINDOW_WIDTH: u32 = 1024;
/// Minimum window height, per `PRD.md` Section 3.1.
///
/// See [`MIN_WINDOW_WIDTH`] for why this is duplicated in
/// `ui/theme/metrics.slint`.
pub const MIN_WINDOW_HEIGHT: u32 = 768;

/// Which theme variant the UI renders in.
///
/// Both variants use the same Radha palette; neither introduces black or blue
/// (`DESIGN.md` Section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Ivory/cream surfaces — the default.
    #[default]
    Light,
    /// Deep plum surfaces.
    Dark,
}

impl ThemeMode {
    /// The other mode. Drives the theme toggle.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            ThemeMode::Light => ThemeMode::Dark,
            ThemeMode::Dark => ThemeMode::Light,
        }
    }

    /// Whether this is the dark variant — convenient for the Slint boolean
    /// property that drives token selection.
    #[must_use]
    pub fn is_dark(self) -> bool {
        matches!(self, ThemeMode::Dark)
    }
}

/// Saved window geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSettings {
    /// Last window width in logical pixels.
    pub width: u32,
    /// Last window height in logical pixels.
    pub height: u32,
    /// Whether the window was maximised when last closed.
    pub maximized: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        WindowSettings {
            width: MIN_WINDOW_WIDTH,
            height: MIN_WINDOW_HEIGHT,
            maximized: false,
        }
    }
}

impl WindowSettings {
    /// Clamp geometry up to the supported minimum.
    ///
    /// Guards against a hand-edited or stale settings file producing a window too
    /// small to lay the UI out in.
    #[must_use]
    pub fn clamped(self) -> Self {
        WindowSettings {
            width: self.width.max(MIN_WINDOW_WIDTH),
            height: self.height.max(MIN_WINDOW_HEIGHT),
            maximized: self.maximized,
        }
    }
}

/// Typing text size and weight (Settings → Typography).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypographySettings {
    /// Passage font size, `FONT_MIN..=FONT_MAX` px (Hindi never below 18).
    pub typing_font_px: u32,
    /// Bold passage text.
    pub bold: bool,
}

impl Default for TypographySettings {
    fn default() -> Self {
        TypographySettings {
            typing_font_px: crate::typing_rules::FONT_DEFAULT,
            bold: false,
        }
    }
}

/// Sounds (Settings → Sound).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundSettings {
    pub enabled: bool,
}

impl Default for SoundSettings {
    fn default() -> Self {
        SoundSettings { enabled: true }
    }
}

/// Motion (Settings → Animation). Reduce motion turns every transition off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AnimationSettings {
    pub reduce_motion: bool,
}

/// Default typing rules for practice sessions (Settings → Typing). Exams
/// override them (RULES 38).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypingSettings {
    /// `BackspacePolicy::slug`.
    pub backspace: String,
    /// `HighlightMode::slug`.
    pub highlight: String,
    pub paper: bool,
    /// Passage word limit, 0 (off) or 50–1500 (`typing_rules::WORD_LIMIT`).
    pub word_limit: u32,
    /// Typing screen: the passage scrollbar.
    pub show_scrollbar: bool,
    /// Typing screen: the passage follows the cursor.
    pub auto_scroll: bool,
    /// Key Guide / mistake-key marking: next key, finger and key marks.
    /// Off by default: the highlighter is word-based.
    pub key_guide: bool,
}

impl Default for TypingSettings {
    fn default() -> Self {
        TypingSettings {
            backspace: crate::typing_rules::BackspacePolicy::Full.slug().to_owned(),
            highlight: crate::typing_rules::HighlightMode::WordError
                .slug()
                .to_owned(),
            paper: false,
            word_limit: 0,
            show_scrollbar: true,
            auto_scroll: true,
            key_guide: false,
        }
    }
}

/// Virtual keyboard (Settings → Keyboard).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyboardSettings {
    /// Legacy: the on-screen keyboard toggle of builds before the keyboard
    /// was removed from the typing screen. Still read so an old
    /// `settings.toml` loads unchanged, but it has no effect and is never
    /// written back (the next save drops it from the file). Its successor
    /// is `typing.key_guide`, which keeps its own value: the old toggle is
    /// not copied over, as the Key Guide is off unless the user turns it on.
    #[serde(skip_serializing)]
    pub show_keyboard: bool,
    /// The preferred typing layout (`LayoutId::as_str`: `qwerty`, `gail`,
    /// `cbi`, `krutidev`, `inscript`). Opening Hindi Typing Mastery or a
    /// Hindi exam starts on it when it is a Hindi layout; the Exercise
    /// screen's layout switch writes it back.
    pub layout: String,
}

impl Default for KeyboardSettings {
    fn default() -> Self {
        KeyboardSettings {
            show_keyboard: false,
            layout: radha_layouts::LayoutId::RemingtonGail.as_str().to_owned(),
        }
    }
}

impl KeyboardSettings {
    /// The preferred layout (the default when the stored id is unknown).
    #[must_use]
    pub fn layout_id(&self) -> radha_layouts::LayoutId {
        radha_layouts::LayoutId::parse(&self.layout)
            .unwrap_or(radha_layouts::LayoutId::RemingtonGail)
    }
}

/// Hindi Guide default (Settings → Guide).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuideSettings {
    /// `GuideMode::slug` used in LEARN / BEGINNER lessons.
    pub mode: String,
}

impl Default for GuideSettings {
    fn default() -> Self {
        GuideSettings {
            mode: crate::typing_rules::GuideMode::Full.slug().to_owned(),
        }
    }
}

/// Focus mode default (Settings → Focus Mode).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FocusSettings {
    pub focus_mode: bool,
}

/// All persisted user preferences.
// `#[serde(default)]` on the container applies per field, so a file missing any
// field still loads. Unknown fields are ignored (serde's default behaviour), which
// is what lets an older build open a settings file written by a newer one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    /// Active theme variant.
    pub theme: ThemeMode,
    /// Window geometry.
    pub window: WindowSettings,
    /// `true` until the app has completed one successful launch.
    ///
    /// Reserved for the first-run experience; the flag is cleared on the first
    /// save so later phases can branch on it.
    pub first_launch: bool,
    pub typography: TypographySettings,
    pub sound: SoundSettings,
    pub animation: AnimationSettings,
    pub typing: TypingSettings,
    pub keyboard: KeyboardSettings,
    pub guide: GuideSettings,
    pub focus: FocusSettings,
}

/// Take one top-level value of `table` as `T`, keeping `fallback` (with a
/// log line) when it has the wrong shape.
fn section<T: serde::de::DeserializeOwned>(table: &toml::Table, key: &str, fallback: T) -> T {
    match table.get(key) {
        None => fallback,
        Some(value) => match value.clone().try_into() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("settings.toml: invalid `{key}` ({e}); using the default");
                fallback
            }
        },
    }
}

impl Settings {
    /// Load settings from `path`.
    ///
    /// # Errors
    ///
    /// - [`CoreError::ResourceMissing`] if the file does not exist.
    /// - [`CoreError::Io`] if it cannot be read.
    /// - [`CoreError::Malformed`] if it is not valid TOML / has bad values.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        if !path.exists() {
            return Err(CoreError::ResourceMissing {
                path: path.to_path_buf(),
            });
        }

        let raw = std::fs::read_to_string(path).map_err(|e| CoreError::io(path, e))?;

        Settings::parse(&raw).map_err(|e| CoreError::malformed("settings", path, e))
    }

    /// Parse a settings document. A value of the wrong type costs only its
    /// own section (logged, default kept); only text that is not TOML at all
    /// is an error. Out-of-range values are then normalised.
    ///
    /// # Errors
    ///
    /// The TOML syntax error, when `raw` is not a TOML document.
    pub fn parse(raw: &str) -> std::result::Result<Self, String> {
        match toml::from_str::<Settings>(raw) {
            Ok(settings) => Ok(settings.normalised()),
            Err(full) => {
                let table: toml::Table = toml::from_str(raw).map_err(|e| e.to_string())?;
                log::warn!("settings.toml has invalid values ({full}); keeping the valid sections");
                let d = Settings::default();
                Ok(Settings {
                    theme: section(&table, "theme", d.theme),
                    window: section(&table, "window", d.window),
                    first_launch: section(&table, "first_launch", d.first_launch),
                    typography: section(&table, "typography", d.typography),
                    sound: section(&table, "sound", d.sound),
                    animation: section(&table, "animation", d.animation),
                    typing: section(&table, "typing", d.typing),
                    keyboard: section(&table, "keyboard", d.keyboard),
                    guide: section(&table, "guide", d.guide),
                    focus: section(&table, "focus", d.focus),
                }
                .normalised())
            }
        }
    }

    /// Change one preference from the Settings overlay. `key` and `value`
    /// are fixed ids the UI sends; anything unknown or invalid is ignored
    /// and returns `false` (the overlay then redraws the stored value).
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        use crate::typing_rules::{BackspacePolicy, GuideMode, HighlightMode, FONT_MAX, FONT_MIN};
        let on = match value {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        };
        match (key, on) {
            ("theme", _) => match value {
                "light" => self.theme = ThemeMode::Light,
                "dark" => self.theme = ThemeMode::Dark,
                _ => return false,
            },
            ("font", _) => {
                let px = self.typography.typing_font_px;
                self.typography.typing_font_px = match value {
                    "up" => (px + 1).min(FONT_MAX),
                    "down" => px.saturating_sub(1).max(FONT_MIN),
                    _ => return false,
                };
            }
            ("font-px", _) => match value.parse::<u32>() {
                Ok(px) if (FONT_MIN..=FONT_MAX).contains(&px) => {
                    self.typography.typing_font_px = px;
                }
                _ => return false,
            },
            ("word-limit", _) => {
                let (lo, hi) = crate::typing_rules::WORD_LIMIT;
                self.typing.word_limit = match value.parse::<u32>() {
                    _ if value == "off" => 0,
                    Ok(n) if (lo..=hi).contains(&n) => n,
                    _ => return false,
                };
            }
            ("scrollbar", Some(b)) => self.typing.show_scrollbar = b,
            ("auto-scroll", Some(b)) => self.typing.auto_scroll = b,
            ("key-guide", Some(b)) => self.typing.key_guide = b,
            ("bold", Some(b)) => self.typography.bold = b,
            ("sound", Some(b)) => self.sound.enabled = b,
            ("reduce-motion", Some(b)) => self.animation.reduce_motion = b,
            ("paper", Some(b)) => self.typing.paper = b,
            // The old on-screen keyboard id: the Key Guide replaced it.
            ("keyboard", Some(b)) => self.typing.key_guide = b,
            ("layout", _) if radha_layouts::LayoutId::parse(value).is_ok() => {
                value.clone_into(&mut self.keyboard.layout);
            }
            ("focus", Some(b)) => self.focus.focus_mode = b,
            ("backspace", _) if BackspacePolicy::from_slug(value).is_some() => {
                value.clone_into(&mut self.typing.backspace);
            }
            ("highlight", _) if HighlightMode::from_slug(value).is_some() => {
                value.clone_into(&mut self.typing.highlight);
            }
            ("guide", _) if GuideMode::from_slug(value).is_some() => {
                value.clone_into(&mut self.guide.mode);
            }
            _ => return false,
        }
        true
    }

    /// The default backspace rule.
    #[must_use]
    pub fn backspace(&self) -> crate::typing_rules::BackspacePolicy {
        crate::typing_rules::BackspacePolicy::from_slug(&self.typing.backspace).unwrap_or_default()
    }

    /// The default highlight rule.
    #[must_use]
    pub fn highlight(&self) -> crate::typing_rules::HighlightMode {
        crate::typing_rules::HighlightMode::from_slug(&self.typing.highlight).unwrap_or_default()
    }

    /// The default guide mode.
    #[must_use]
    pub fn guide_mode(&self) -> crate::typing_rules::GuideMode {
        crate::typing_rules::GuideMode::from_slug(&self.guide.mode).unwrap_or_default()
    }

    /// Load settings, falling back to defaults if absent or unreadable.
    ///
    /// This is what startup uses. A corrupt settings file must not stop the user
    /// from typing (`RULES.MD` Rule 23) — it is logged and replaced by defaults.
    /// The bad file is preserved as `settings.toml.invalid` rather than
    /// overwritten, so a user's customisations are recoverable.
    #[must_use]
    pub fn load_or_default(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();

        match Settings::load(path) {
            Ok(settings) => {
                log::info!("Loaded settings from {}", path.display());
                settings
            }
            Err(CoreError::ResourceMissing { .. }) => {
                log::info!(
                    "No settings file at {} — using defaults (first launch)",
                    path.display()
                );
                Settings::first_run()
            }
            Err(err) => {
                log::warn!(
                    "Could not read settings ({err}); falling back to defaults. \
                     The existing file will be kept as settings.toml.invalid"
                );
                let quarantine = path.with_extension("toml.invalid");
                let _ = std::fs::rename(path, &quarantine);
                Settings::default()
            }
        }
    }

    /// Defaults for a brand-new installation.
    #[must_use]
    pub fn first_run() -> Self {
        Settings {
            first_launch: true,
            ..Settings::default()
        }
    }

    /// Write settings to `path`, creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// - [`CoreError::Io`] if the file or its directory cannot be written.
    /// - [`CoreError::Malformed`] if serialisation fails (should not happen).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
        }

        let body = toml::to_string_pretty(self)
            .map_err(|e| CoreError::malformed("settings", path, e.to_string()))?;

        let document = format!(
            "# {} — user settings\n\
             # Edited automatically; hand edits are preserved when valid.\n\n{body}",
            radha_core::PRODUCT_NAME
        );

        // Atomic: a torn settings.toml would be quarantined on the next
        // launch and every preference reset.
        crate::paths::write_replace(path, document.as_bytes())
            .map_err(|e| CoreError::io(path, e))?;
        log::debug!("Saved settings to {}", path.display());
        Ok(())
    }

    /// Apply validity constraints that deserialisation alone cannot enforce.
    #[must_use]
    fn normalised(mut self) -> Self {
        use crate::typing_rules::{BackspacePolicy, GuideMode, HighlightMode, FONT_MAX, FONT_MIN};
        self.window = self.window.clamped();
        let d = Settings::default();
        let px = self.typography.typing_font_px;
        if !(FONT_MIN..=FONT_MAX).contains(&px) {
            log::warn!(
                "settings.toml: typing_font_px {px} is outside {FONT_MIN}-{FONT_MAX}; using {}",
                d.typography.typing_font_px
            );
            self.typography.typing_font_px = d.typography.typing_font_px;
        }
        if BackspacePolicy::from_slug(&self.typing.backspace).is_none() {
            log::warn!(
                "settings.toml: unknown backspace rule '{}'; using the default",
                self.typing.backspace
            );
            self.typing.backspace = d.typing.backspace;
        }
        if HighlightMode::from_slug(&self.typing.highlight).is_none() {
            log::warn!(
                "settings.toml: unknown highlight rule '{}'; using the default",
                self.typing.highlight
            );
            self.typing.highlight = d.typing.highlight;
        }
        if GuideMode::from_slug(&self.guide.mode).is_none() {
            log::warn!(
                "settings.toml: unknown guide mode '{}'; using the default",
                self.guide.mode
            );
            self.guide.mode = d.guide.mode;
        }
        let (lo, hi) = crate::typing_rules::WORD_LIMIT;
        let limit = self.typing.word_limit;
        if limit != 0 && !(lo..=hi).contains(&limit) {
            log::warn!("settings.toml: word_limit {limit} is outside {lo}-{hi}; turning it off");
            self.typing.word_limit = 0;
        }
        if radha_layouts::LayoutId::parse(&self.keyboard.layout).is_err() {
            log::warn!(
                "settings.toml: unknown keyboard layout '{}'; using the default",
                self.keyboard.layout
            );
            self.keyboard.layout = d.keyboard.layout;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_light_theme_at_minimum_size() {
        let s = Settings::default();
        assert_eq!(s.theme, ThemeMode::Light);
        assert_eq!(s.window.width, MIN_WINDOW_WIDTH);
        assert_eq!(s.window.height, MIN_WINDOW_HEIGHT);
        assert!(!s.window.maximized);
    }

    #[test]
    fn theme_toggles_both_ways() {
        assert_eq!(ThemeMode::Light.toggled(), ThemeMode::Dark);
        assert_eq!(ThemeMode::Dark.toggled(), ThemeMode::Light);
        assert!(ThemeMode::Dark.is_dark());
        assert!(!ThemeMode::Light.is_dark());
    }

    #[test]
    fn round_trips_through_toml() {
        let original = Settings {
            theme: ThemeMode::Dark,
            window: WindowSettings {
                width: 1440,
                height: 900,
                maximized: true,
            },
            first_launch: false,
            ..Settings::default()
        };

        let encoded = toml::to_string_pretty(&original).expect("should serialise");
        let decoded: Settings = toml::from_str(&encoded).expect("should deserialise");

        assert_eq!(decoded, original);
    }

    #[test]
    fn saves_and_loads_from_disk() {
        let temp = tempfile::tempdir().unwrap();
        // Nested so directory creation is exercised.
        let path = temp.path().join("config").join("settings.toml");

        let original = Settings {
            theme: ThemeMode::Dark,
            window: WindowSettings {
                width: 1280,
                height: 800,
                maximized: false,
            },
            first_launch: false,
            ..Settings::default()
        };
        original.save(&path).expect("save should succeed");

        assert!(path.exists());
        let loaded = Settings::load(&path).expect("load should succeed");
        assert_eq!(loaded, original);
    }

    #[test]
    fn missing_file_reports_resource_missing() {
        let temp = tempfile::tempdir().unwrap();
        let err = Settings::load(temp.path().join("absent.toml")).unwrap_err();
        assert!(matches!(err, CoreError::ResourceMissing { .. }));
    }

    #[test]
    fn missing_file_yields_first_run_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings::load_or_default(temp.path().join("absent.toml"));

        assert_eq!(settings.theme, ThemeMode::Light);
        assert!(
            settings.first_launch,
            "an absent settings file means this is a first launch"
        );
    }

    #[test]
    fn malformed_file_falls_back_and_is_quarantined() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.toml");
        std::fs::write(&path, "this is not valid toml {{{").unwrap();

        let settings = Settings::load_or_default(&path);

        // The app keeps running on defaults rather than refusing to start.
        assert_eq!(settings, Settings::default());
        assert!(
            path.with_extension("toml.invalid").exists(),
            "the unreadable file should be preserved for recovery, not deleted"
        );
    }

    #[test]
    fn undersized_window_is_clamped_to_the_supported_minimum() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.toml");
        std::fs::write(
            &path,
            "theme = \"light\"\nfirst_launch = false\n\n[window]\nwidth = 320\nheight = 240\nmaximized = false\n",
        )
        .unwrap();

        let loaded = Settings::load(&path).expect("should load");
        assert_eq!(loaded.window.width, MIN_WINDOW_WIDTH);
        assert_eq!(loaded.window.height, MIN_WINDOW_HEIGHT);
    }

    #[test]
    fn partial_file_uses_defaults_for_absent_fields() {
        // Simulates a settings file written by an older build that predates some
        // fields. It must load, not fail.
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.toml");
        std::fs::write(&path, "theme = \"dark\"\n").unwrap();

        let loaded = Settings::load(&path).expect("partial settings should load");
        assert_eq!(loaded.theme, ThemeMode::Dark);
        assert_eq!(
            loaded.window.width, MIN_WINDOW_WIDTH,
            "absent window section should default"
        );
    }

    #[test]
    fn an_unknown_keyboard_layout_falls_back_to_the_default() {
        let s = Settings::parse("[keyboard]\nlayout = \"dvorak\"\nshow_keyboard = false\n")
            .expect("toml");
        assert_eq!(s.keyboard.layout, "gail");
        assert!(!s.keyboard.show_keyboard, "the rest of the section is kept");
        let s = Settings::parse("[keyboard]\nlayout = \"krutidev\"\n").expect("toml");
        assert_eq!(s.keyboard.layout_id(), radha_layouts::LayoutId::KrutiDev);
    }

    #[test]
    fn the_legacy_show_keyboard_setting_loads_but_is_dropped_on_save() {
        // A settings.toml written before the on-screen keyboard was removed.
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.toml");
        std::fs::write(
            &path,
            "theme = \"dark\"\nfirst_launch = false\n\n[typing]\nkey_guide = false\n\n\
             [keyboard]\nshow_keyboard = true\nlayout = \"cbi\"\n",
        )
        .unwrap();
        let loaded = Settings::load(&path).expect("an old file still loads");
        assert_eq!(loaded.theme, ThemeMode::Dark);
        assert_eq!(
            loaded.keyboard.layout_id(),
            radha_layouts::LayoutId::RemingtonCbi
        );
        assert!(
            !loaded.typing.key_guide,
            "the old toggle does not switch the Key Guide on"
        );
        loaded.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("show_keyboard"), "{text}");
        assert!(text.contains("layout = \"cbi\""), "{text}");
        let again = Settings::load(&path).unwrap();
        assert_eq!(again.keyboard.layout, "cbi");
        assert_eq!(again.theme, ThemeMode::Dark);
    }

    #[test]
    fn unknown_fields_are_ignored_rather_than_fatal() {
        // A settings file from a *newer* build, opened by an older one.
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.toml");
        std::fs::write(&path, "theme = \"light\"\nsome_future_setting = 42\n").unwrap();

        let loaded = Settings::load(&path);
        assert!(
            loaded.is_ok(),
            "an unrecognised key must not make settings unreadable"
        );
    }

    #[test]
    fn every_preference_round_trips_through_disk() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config").join("settings.toml");
        let mut s = Settings::default();
        for (k, v) in [
            ("theme", "dark"),
            ("font", "up"),
            ("bold", "on"),
            ("sound", "off"),
            ("reduce-motion", "on"),
            ("paper", "on"),
            ("keyboard", "off"),
            ("focus", "on"),
            ("backspace", "word"),
            ("highlight", "blind"),
            ("guide", "smart"),
            ("layout", "inscript"),
        ] {
            assert!(s.set(k, v), "{k}={v}");
        }
        assert_eq!(s.keyboard.layout_id(), radha_layouts::LayoutId::Inscript);
        s.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded, s);
        assert_eq!(loaded.typography.typing_font_px, 23);
        assert_eq!(
            loaded.backspace(),
            crate::typing_rules::BackspacePolicy::OneWord
        );
        assert_eq!(loaded.guide_mode(), crate::typing_rules::GuideMode::Smart);
        assert!(loaded.animation.reduce_motion && !loaded.sound.enabled);
    }

    #[test]
    fn invalid_ui_values_change_nothing() {
        let mut s = Settings::default();
        for (k, v) in [
            ("theme", "blue"),
            ("font", "99"),
            ("bold", "yes"),
            ("backspace", "sometimes"),
            ("guide", ""),
            ("unknown", "on"),
            ("layout", "remington"),
            ("layout", ""),
        ] {
            assert!(!s.set(k, v), "{k}={v} must be rejected");
        }
        assert_eq!(s, Settings::default());
        // The font stepper stops at the range ends.
        for _ in 0..40 {
            s.set("font", "up");
        }
        assert_eq!(s.typography.typing_font_px, crate::typing_rules::FONT_MAX);
        for _ in 0..40 {
            s.set("font", "down");
        }
        assert_eq!(s.typography.typing_font_px, crate::typing_rules::FONT_MIN);
    }

    #[test]
    fn bad_values_fall_back_per_section_without_failing() {
        let raw = "theme = \"dark\"\n\
                   [typography]\ntyping_font_px = 300\nbold = true\n\
                   [sound]\nenabled = \"loud\"\n\
                   [typing]\nbackspace = \"sometimes\"\nhighlight = \"blind\"\n\
                   [guide]\nmode = 7\n\
                   [animation]\nreduce_motion = true\n";
        let s = Settings::parse(raw).expect("valid TOML never fails");
        let d = Settings::default();
        assert_eq!(s.theme, ThemeMode::Dark, "valid sections are kept");
        assert!(s.typography.bold);
        assert_eq!(s.typography.typing_font_px, d.typography.typing_font_px);
        assert_eq!(s.sound, d.sound, "wrong type: section default");
        assert_eq!(s.typing.backspace, "full", "unknown slug: default");
        assert_eq!(s.typing.highlight, "blind");
        assert_eq!(s.guide, d.guide);
        assert!(s.animation.reduce_motion);

        // Through the file path too: it loads, nothing is quarantined.
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.toml");
        std::fs::write(&path, raw).unwrap();
        assert_eq!(Settings::load_or_default(&path), s);
        assert!(!path.with_extension("toml.invalid").exists());
    }

    #[test]
    fn typing_screen_rules_persist_and_are_validated() {
        let mut s = Settings::default();
        assert!(!s.typing.key_guide, "the key guide is off by default");
        assert!(s.typing.show_scrollbar && s.typing.auto_scroll);
        assert!(s.set("word-limit", "400"));
        assert!(!s.set("word-limit", "49"));
        assert!(!s.set("word-limit", "1501"));
        assert!(!s.set("word-limit", "abc"));
        assert_eq!(s.typing.word_limit, 400);
        assert!(s.set("word-limit", "off"));
        assert_eq!(s.typing.word_limit, 0);
        assert!(s.set("key-guide", "on"));
        assert!(s.set("scrollbar", "off"));
        assert!(s.set("auto-scroll", "off"));
        assert!(s.set("font-px", "26"));
        assert!(!s.set("font-px", "99"));
        assert!(s.set("word-limit", "1500"));
        let parsed = Settings::parse(&toml::to_string(&s).unwrap()).unwrap();
        assert_eq!(parsed.typing, s.typing);
        assert_eq!(parsed.typography.typing_font_px, 26);
        let bad = Settings::parse("[typing]\nword_limit = 7\n").unwrap();
        assert_eq!(bad.typing.word_limit, 0, "out of range turns the limit off");
    }

    #[test]
    fn saved_file_is_commented_for_humans() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.toml");
        Settings::default().save(&path).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            contents.contains("Radha Typing Tutor"),
            "saved settings should carry a header naming the product"
        );
    }
}
