//! Platform directory resolution.
//!
//! Single place that decides where Radha keeps its files, so logging (Task 2),
//! settings (Task 4), and the database (Task 5) cannot drift apart.
//!
//! On Windows this resolves under `%APPDATA%`:
//!
//! ```text
//! %APPDATA%\Radha\Radha Typing Tutor\
//! ├── settings.toml
//! ├── radha.sqlite
//! └── logs\
//!     └── radha.log
//! ```
//!
//! Writing to the user's app-data directory rather than the install directory
//! matters: `Program Files` is not writable by a standard user, so keeping state
//! there would break on any correctly-installed copy.

use std::path::{Path, PathBuf};

use radha_core::{CoreError, Result};

/// Qualifier / organisation / application triple for `directories`.
const ORGANISATION: &str = "Radha";
const APPLICATION: &str = "Radha Typing Tutor";

/// Resolved locations for everything Radha writes to disk.
#[derive(Debug, Clone)]
pub struct AppPaths {
    data_dir: PathBuf,
    config_dir: PathBuf,
    /// Read-only exercise bundles shipped with the app (`content/exercises`).
    content_dir: PathBuf,
}

/// Where the shipped exercise bundles are: `content/exercises` beside the
/// executable when installed, else the workspace copy (development builds).
fn bundled_content_dir() -> PathBuf {
    let relative = Path::new("content").join("exercises");
    if let Some(beside_exe) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&relative)))
        .filter(|dir| dir.is_dir())
    {
        return beside_exe;
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(relative)
}

/// Replace `path` with `bytes` without ever leaving it half written: write
/// a sibling temp file, flush it to disk, then rename it over `path` (an
/// atomic replace on NTFS). A power cut mid-save leaves the old file, not a
/// torn one that the next launch would have to quarantine or discard.
///
/// # Errors
///
/// The I/O error; `path` is then unchanged.
pub fn write_replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

impl AppPaths {
    /// Resolve the standard per-user locations for this platform.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::PlatformDirUnavailable`] if the OS cannot supply a
    /// home/app-data directory. Rare, but it must surface as an error rather
    /// than a panic (`RULES.MD` Rule 23).
    pub fn resolve() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("", ORGANISATION, APPLICATION).ok_or(
            CoreError::PlatformDirUnavailable {
                purpose: "application data",
            },
        )?;

        Ok(AppPaths {
            data_dir: dirs.data_dir().to_path_buf(),
            config_dir: dirs.config_dir().to_path_buf(),
            content_dir: bundled_content_dir(),
        })
    }

    /// Build paths rooted at an arbitrary directory.
    ///
    /// Used by tests so they never touch the real user profile. Also the seam a
    /// future portable/USB build would use to keep all state beside the
    /// executable, which is why it is kept rather than confined to `#[cfg(test)]`.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn rooted_at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        AppPaths {
            data_dir: root.join("data"),
            config_dir: root.join("config"),
            content_dir: root.join("content").join("exercises"),
        }
    }

    /// Directory of the shipped exercise bundles (read-only).
    #[must_use]
    pub fn content_dir(&self) -> &Path {
        &self.content_dir
    }

    /// Directory for persistent data (database, logs).
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Directory for user configuration.
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Full path to the settings file.
    #[must_use]
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    /// Full path to the SQLite database.
    #[must_use]
    pub fn database_file(&self) -> PathBuf {
        self.data_dir.join("radha.sqlite")
    }

    /// Directory holding log files.
    #[must_use]
    pub fn log_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// Full path to the current log file.
    #[must_use]
    pub fn log_file(&self) -> PathBuf {
        self.log_dir().join("radha.log")
    }

    /// Create every directory Radha needs, if absent.
    ///
    /// Idempotent — safe to call on every launch.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Io`] naming the directory that could not be created.
    pub fn ensure_exist(&self) -> Result<()> {
        for dir in [&self.data_dir, &self.config_dir, &self.log_dir()] {
            std::fs::create_dir_all(dir).map_err(|e| CoreError::io(dir, e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_real_platform_directories() {
        let paths = AppPaths::resolve().expect("Windows should always provide app-data");
        assert!(!paths.data_dir().as_os_str().is_empty());
        assert!(!paths.config_dir().as_os_str().is_empty());
    }

    #[test]
    fn file_names_are_stable() {
        // These names appear in support docs and user-facing troubleshooting,
        // so pin them.
        let paths = AppPaths::rooted_at("C:/tmp/radha");
        assert!(paths.settings_file().ends_with("settings.toml"));
        assert!(paths.database_file().ends_with("radha.sqlite"));
        assert!(paths.log_file().ends_with("radha.log"));
    }

    #[test]
    fn logs_live_under_the_data_directory() {
        let paths = AppPaths::rooted_at("C:/tmp/radha");
        assert!(paths.log_dir().starts_with(paths.data_dir()));
    }

    #[test]
    fn write_replace_swaps_the_whole_file_and_leaves_no_temp() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sub").join("settings.toml");
        write_replace(&path, b"theme = \"dark\"\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"theme = \"dark\"\n");
        write_replace(&path, b"x").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("settings.toml")]);
        // A failing replace (the target is a directory) keeps what is there
        // and cleans its temp file up.
        let dir = temp.path().join("busy");
        std::fs::create_dir_all(dir.join("keep")).unwrap();
        assert!(write_replace(&dir, b"y").is_err());
        assert!(dir.join("keep").is_dir());
        assert!(!temp.path().join("busy.tmp").exists());
    }

    #[test]
    fn ensure_exist_creates_all_directories_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted_at(temp.path());

        paths.ensure_exist().expect("first call should succeed");
        assert!(paths.data_dir().is_dir());
        assert!(paths.config_dir().is_dir());
        assert!(paths.log_dir().is_dir());

        // Every launch calls this, so a second call must not fail.
        paths.ensure_exist().expect("second call should succeed");
    }
}
