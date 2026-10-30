//! Application logging.
//!
//! # Behaviour
//!
//! - **Always** appends to `<data-dir>/logs/radha.log`, so a user reporting a
//!   problem has something to send.
//! - **Debug builds additionally echo to stdout**, so `cargo run` shows output
//!   without opening the log file.
//! - Default level is `debug` in debug builds and `info` in release, because
//!   per-keystroke-adjacent `debug!` logging in release would add avoidable work
//!   to the latency budget (`PRD.md` Section 3).
//! - `RUST_LOG` overrides the default level when set, for field diagnostics.
//!
//! # Log rotation
//!
//! The file is rotated once it passes [`MAX_LOG_BYTES`], keeping one previous
//! generation. Radha is a long-lived desktop app on machines that may have
//! little free disk, so an unbounded log is a real (if slow) failure mode.

use std::io::Write;
use std::path::{Path, PathBuf};

use radha_core::{CoreError, Result};

use crate::paths::AppPaths;

/// Rotate the log once it exceeds this size (2 MiB).
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

/// Writes log output to a file and, optionally, to stdout.
///
/// `env_logger` accepts a single sink, so dual output needs an explicit tee.
struct TeeWriter {
    file: std::fs::File,
    echo_to_stdout: bool,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.echo_to_stdout {
            // Console output is a convenience, never a reason to fail a log
            // write — a closed/redirected stdout must not break file logging.
            let _ = std::io::stdout().write_all(buf);
        }
        self.file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.echo_to_stdout {
            let _ = std::io::stdout().flush();
        }
        self.file.flush()
    }
}

/// Rotate the log file if it has grown past [`MAX_LOG_BYTES`].
///
/// Best-effort: a rotation failure is not worth aborting startup over, so the
/// old file is simply left in place and appended to.
fn rotate_if_needed(log_file: &PathBuf) {
    let Ok(metadata) = std::fs::metadata(log_file) else {
        return; // No existing log — nothing to rotate.
    };

    if metadata.len() <= MAX_LOG_BYTES {
        return;
    }

    let previous = log_file.with_extension("log.old");
    let _ = std::fs::remove_file(&previous);
    let _ = std::fs::rename(log_file, &previous);
}

/// Default level filter for this build profile.
fn default_level() -> log::LevelFilter {
    if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    }
}

/// Initialise logging.
///
/// Returns the path of the active log file, so startup can report it.
///
/// # Errors
///
/// - [`CoreError::Io`] if the log directory or file cannot be opened.
/// - [`CoreError::Database`] is never returned; logging does not touch SQLite.
///
/// Calling this more than once in a process returns an error rather than
/// panicking, since `log` permits only one global logger.
pub fn init(paths: &AppPaths) -> Result<PathBuf> {
    let log_dir = paths.log_dir();
    std::fs::create_dir_all(&log_dir).map_err(|e| CoreError::io(&log_dir, e))?;

    let log_file = paths.log_file();
    rotate_if_needed(&log_file);

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .map_err(|e| CoreError::io(&log_file, e))?;

    let writer = TeeWriter {
        file,
        // Only mirror to the console in debug builds. A release build launched
        // from Explorer has no console attached.
        echo_to_stdout: cfg!(debug_assertions),
    };

    let mut builder = env_logger::Builder::new();
    builder
        .filter_level(default_level())
        // Honour RUST_LOG when present, without discarding the default above.
        .parse_default_env()
        .target(env_logger::Target::Pipe(Box::new(writer)))
        .format(|buf, record| {
            writeln!(
                buf,
                "{} [{:<5}] {}: {}",
                // Compact, sortable, locale-independent timestamp.
                buf.timestamp_millis(),
                record.level(),
                record.target(),
                record.args()
            )
        });

    builder.try_init().map_err(|e| CoreError::Malformed {
        kind: "logger",
        path: log_file.clone(),
        detail: e.to_string(),
    })?;

    Ok(log_file)
}

/// Write the startup banner.
///
/// Called immediately after [`init`] so every log file opens with the version
/// and paths — the first three questions any bug report raises.
pub fn log_startup_banner(paths: &AppPaths, log_file: &Path) {
    log::info!(
        "=== {} v{} ===",
        radha_core::PRODUCT_NAME,
        radha_core::VERSION
    );
    log::info!(
        "Build profile: {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    log::info!("Data dir:   {}", paths.data_dir().display());
    log::info!("Config dir: {}", paths.config_dir().display());
    log::info!("Log file:   {}", log_file.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_level_is_quieter_in_release() {
        // Guards the latency rationale in the module docs: release must not
        // default to debug-level logging.
        let level = default_level();
        if cfg!(debug_assertions) {
            assert_eq!(level, log::LevelFilter::Debug);
        } else {
            assert_eq!(level, log::LevelFilter::Info);
        }
    }

    #[test]
    fn tee_writer_writes_to_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.log");

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();

        let mut writer = TeeWriter {
            file,
            // Keep test output clean.
            echo_to_stdout: false,
        };
        writer.write_all(b"hello radha\n").unwrap();
        writer.flush().unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("hello radha"));
    }

    #[test]
    fn small_log_is_not_rotated() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("radha.log");
        std::fs::write(&path, b"small").unwrap();

        rotate_if_needed(&path);

        assert!(path.exists(), "a small log should be left alone");
        assert!(!path.with_extension("log.old").exists());
    }

    #[test]
    fn oversized_log_is_rotated_to_old() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("radha.log");
        // Exceed the threshold.
        std::fs::write(&path, vec![b'x'; (MAX_LOG_BYTES + 1) as usize]).unwrap();

        rotate_if_needed(&path);

        assert!(
            path.with_extension("log.old").exists(),
            "oversized log should have been moved aside"
        );
        assert!(
            !path.exists(),
            "the active log path should be free for a fresh file"
        );
    }

    #[test]
    fn rotation_on_a_missing_file_is_a_no_op() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("does-not-exist.log");

        // First launch has no log yet; this must not panic.
        rotate_if_needed(&path);

        assert!(!path.exists());
    }

    #[test]
    fn init_creates_the_log_file_and_does_not_panic_when_called_twice() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::rooted_at(temp.path());

        let first = init(&paths);

        // Whether this process already installed a logger depends on test
        // ordering, so accept either outcome — the contract under test is
        // "returns a Result, never panics".
        if let Ok(ref log_file) = first {
            assert!(log_file.exists(), "log file should have been created");
        }

        // A second init must be an Err, not a panic.
        let second = init(&paths);
        assert!(
            second.is_err(),
            "installing a second global logger must fail gracefully"
        );
    }
}
