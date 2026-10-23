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

        let settings: Settings = toml::from_str(&raw)
            .map_err(|e| CoreError::malformed("settings", path, e.to_string()))?;

        Ok(settings.normalised())
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

        std::fs::write(path, document).map_err(|e| CoreError::io(path, e))?;
        log::debug!("Saved settings to {}", path.display());
        Ok(())
    }

    /// Apply validity constraints that deserialisation alone cannot enforce.
    #[must_use]
    fn normalised(mut self) -> Self {
        self.window = self.window.clamped();
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
    fn unknown_fields_are_ignored_rather_than_fatal() {
        // A settings file from a *newer* build, opened by an older one.
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.toml");
        std::fs::write(
            &path,
            "theme = \"light\"\nsome_future_setting = 42\n",
        )
        .unwrap();

        let loaded = Settings::load(&path);
        assert!(
            loaded.is_ok(),
            "an unrecognised key must not make settings unreadable"
        );
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
