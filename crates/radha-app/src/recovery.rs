//! Crash and blur recovery (PRD TY-14, TASK 37).
//!
//! While a session runs, its key log is journalled to a small TOML file on
//! pause, on window blur and every [`JOURNAL_EVERY`] of active typing, never
//! per keystroke (RULES 14). The database is written once, on completion,
//! and the journal is deleted then (or when the user leaves). On the next
//! launch a journal means the app stopped mid-session: the session is
//! rebuilt by replaying the key log and opened paused.

use std::path::Path;
use std::time::Duration;

use radha_core::{DisplayMode, TimeLimit, TimerMode};
use serde::{Deserialize, Serialize};

use crate::practice::LoggedKey;

/// How often a running session is journalled.
pub const JOURNAL_EVERY: Duration = Duration::from_secs(10);

const BACKSPACE: char = '\u{8}';
const ALTGR: char = '\u{1d}';
/// Esc armed the dead-key layer (GAIL/CBI).
const DEAD_KEY: char = '\u{1b}';

/// One interrupted session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    pub product: String,
    pub exercise_id: i64,
    pub mode: String,
    /// "countdown" | "stopwatch" | "off".
    pub timer: String,
    pub timer_secs: u32,
    pub key_mode: bool,
    pub elapsed_seconds: f64,
    /// The key log: characters, `\u{8}` for Backspace, `\u{1d}` before an
    /// AltGr character, `\u{1b}` for an Esc dead key.
    pub keys: String,
    /// Active milliseconds of every Backspace / character in `keys`
    /// ([`crate::practice::Practice::key_ms`]), so recovery replays the real
    /// rhythm (Typing Flow). Absent in journals of older builds: the keys
    /// are then spaced evenly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_ms: Vec<u32>,
}

/// Encode a key log.
#[must_use]
pub fn encode(log: &[LoggedKey]) -> String {
    let mut s = String::with_capacity(log.len());
    for k in log {
        match *k {
            LoggedKey::Backspace => s.push(BACKSPACE),
            LoggedKey::DeadKey => s.push(DEAD_KEY),
            LoggedKey::Char { c, altgr } => {
                if altgr {
                    s.push(ALTGR);
                }
                s.push(c);
            }
        }
    }
    s
}

/// Decode [`encode`].
#[must_use]
pub fn decode(keys: &str) -> Vec<LoggedKey> {
    let mut out = Vec::with_capacity(keys.len());
    let mut altgr = false;
    for c in keys.chars() {
        match c {
            ALTGR => altgr = true,
            BACKSPACE => {
                out.push(LoggedKey::Backspace);
                altgr = false;
            }
            DEAD_KEY => {
                out.push(LoggedKey::DeadKey);
                altgr = false;
            }
            c => {
                out.push(LoggedKey::Char { c, altgr });
                altgr = false;
            }
        }
    }
    out
}

impl Journal {
    /// The timer mode it records.
    #[must_use]
    pub fn timer_mode(&self) -> TimerMode {
        match self.timer.as_str() {
            "countdown" => TimeLimit::custom(self.timer_secs)
                .map_or(TimerMode::Stopwatch, TimerMode::Countdown),
            "off" => TimerMode::Off,
            _ => TimerMode::Stopwatch,
        }
    }

    /// The display mode it records.
    #[must_use]
    pub fn display(&self) -> DisplayMode {
        if self.key_mode {
            DisplayMode::Key
        } else {
            DisplayMode::Word
        }
    }

    /// Timer fields for a mode.
    #[must_use]
    pub fn timer_fields(mode: TimerMode) -> (String, u32) {
        match mode {
            TimerMode::Countdown(l) => ("countdown".into(), l.seconds()),
            TimerMode::Stopwatch => ("stopwatch".into(), 0),
            TimerMode::Off => ("off".into(), 0),
        }
    }

    /// Read a journal; `None` when there is none or it is unreadable (an
    /// unreadable journal is removed so it never blocks a launch).
    #[must_use]
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        match toml::from_str::<Journal>(&text) {
            Ok(j) if !j.keys.is_empty() => Some(j),
            Ok(_) => None,
            Err(e) => {
                log::warn!("Discarding unreadable session journal: {e}");
                clear(path);
                None
            }
        }
    }

    /// Write the journal (replace atomically: temp file, then rename).
    ///
    /// # Errors
    ///
    /// When it cannot be serialised or written.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string(self)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::paths::write_replace(path, text.as_bytes())?;
        Ok(())
    }
}

/// Delete the journal (session finished or abandoned).
pub fn clear(path: &Path) {
    if path.exists() {
        if let Err(e) = std::fs::remove_file(path) {
            log::warn!("Could not remove the session journal: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::practice::Practice;
    use radha_core::SessionState;
    use radha_layouts::LayoutId;
    use std::time::Instant;

    #[test]
    fn key_logs_round_trip() {
        let log = vec![
            LoggedKey::Char {
                c: 'd',
                altgr: false,
            },
            LoggedKey::Backspace,
            LoggedKey::Char {
                c: 'b',
                altgr: true,
            },
            LoggedKey::Char {
                c: ' ',
                altgr: false,
            },
            LoggedKey::DeadKey,
            LoggedKey::Char {
                c: 'i',
                altgr: false,
            },
        ];
        assert_eq!(decode(&encode(&log)), log);
    }

    #[test]
    fn a_journal_restores_the_session_paused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-journal.toml");
        assert!(Journal::load(&path).is_none());

        let text = "कार्य हिन्दी";
        let mut p = Practice::new(
            LayoutId::RemingtonGail,
            text,
            TimerMode::Stopwatch,
            DisplayMode::Key,
        )
        .unwrap();
        let t0 = Instant::now();
        for (i, c) in "dk;Zx".chars().enumerate() {
            p.press(c, false, t0 + Duration::from_millis(200 * i as u64));
        }
        p.backspace(t0 + Duration::from_millis(1200));
        let typed = p.typed();
        let (timer, timer_secs) = Journal::timer_fields(TimerMode::Stopwatch);
        let j = Journal {
            product: "hindi-learn-gail".into(),
            exercise_id: 7,
            mode: "beginner".into(),
            timer,
            timer_secs,
            key_mode: true,
            elapsed_seconds: 1.2,
            keys: encode(p.key_log()),
            key_ms: p.key_ms(),
        };
        j.save(&path).unwrap();
        let back = Journal::load(&path).unwrap();
        assert_eq!(back, j);

        let mut q = Practice::new(
            LayoutId::RemingtonGail,
            text,
            back.timer_mode(),
            back.display(),
        )
        .unwrap();
        q.replay(&decode(&back.keys), back.elapsed_seconds, Instant::now());
        assert_eq!(q.typed(), typed, "the replay reproduces the text");
        assert_eq!(q.state(), SessionState::Paused, "restored paused (TY-14)");

        clear(&path);
        assert!(Journal::load(&path).is_none());
        std::fs::write(&path, "keys = [").unwrap();
        assert!(Journal::load(&path).is_none());
        assert!(!path.exists(), "a corrupt journal is removed");
        let cd = Journal {
            timer: "countdown".into(),
            timer_secs: 300,
            ..j
        };
        assert_eq!(cd.timer_mode().limit().map(|l| l.seconds()), Some(300));
    }
}
