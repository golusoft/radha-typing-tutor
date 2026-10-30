//! The driver that glues a layout's [`Composer`] to a `TypingSession`, so a
//! caller never sequences the two by hand.
//!
//! Each call first lets the session's countdown expire. A complete session
//! ignores input and resets the composer; a paused one ignores input and
//! keeps the composer as it is (a pending sign survives a pause).

use std::time::Instant;

use radha_core::{SessionState, TextUnit, TypingSession};

use crate::compose::Composer;
use crate::keys::Keystroke;

/// What happened to one key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEffect {
    /// The press was applied to the session.
    Applied {
        /// The composer's preedit after the press.
        preedit: Option<&'static str>,
    },
    /// The key or code has no mapping on this layout; nothing was counted.
    Unmapped,
    /// The session did not take input (paused, complete, nothing to delete).
    Ignored,
}

/// Whether the session can take a press right now.
fn ready<U: TextUnit>(s: &mut TypingSession<U>, c: &mut Composer, now: Instant) -> bool {
    s.tick_at(now);
    match s.state() {
        SessionState::Complete => {
            c.reset();
            false
        }
        SessionState::Paused => false,
        SessionState::Idle | SessionState::Active => true,
    }
}

fn applied<U: TextUnit>(s: &TypingSession<U>, c: &mut Composer) -> KeyEffect {
    if s.state() == SessionState::Complete {
        c.reset();
    }
    KeyEffect::Applied {
        preedit: c.preedit(),
    }
}

/// Strike a key.
pub fn press_key<U: TextUnit>(
    s: &mut TypingSession<U>,
    c: &mut Composer,
    key: Keystroke,
    now: Instant,
) -> KeyEffect {
    if !ready(s, c, now) {
        return KeyEffect::Ignored;
    }
    let Some(out) = c.press(key, s.matcher().typed()) else {
        log::trace!("unmapped keystroke {key:?}");
        return KeyEffect::Unmapped;
    };
    s.apply_edit_at(&out.edit, now);
    applied(s, c)
}

/// Press Backspace. `Ignored` when there is nothing to delete or cancel.
pub fn press_backspace<U: TextUnit>(
    s: &mut TypingSession<U>,
    c: &mut Composer,
    now: Instant,
) -> KeyEffect {
    if !ready(s, c, now) || s.state() == SessionState::Idle {
        return KeyEffect::Ignored;
    }
    let Some(out) = c.backspace(s.matcher().typed()) else {
        return KeyEffect::Ignored;
    };
    s.apply_edit_at(&out.edit, now);
    applied(s, c)
}

/// Press Enter.
pub fn press_enter<U: TextUnit>(
    s: &mut TypingSession<U>,
    c: &mut Composer,
    now: Instant,
) -> KeyEffect {
    if !ready(s, c, now) {
        return KeyEffect::Ignored;
    }
    let out = c.enter(s.matcher().typed());
    s.apply_edit_at(&out.edit, now);
    applied(s, c)
}

/// Press a numpad digit key (`digit` 0..=9) with Caps Lock on or off.
pub fn press_numpad_digit<U: TextUnit>(
    s: &mut TypingSession<U>,
    c: &mut Composer,
    digit: u8,
    caps_lock: bool,
    now: Instant,
) -> KeyEffect {
    if !ready(s, c, now) {
        return KeyEffect::Ignored;
    }
    let Some(out) = c.numpad_digit(digit, caps_lock, s.matcher().typed()) else {
        return KeyEffect::Unmapped;
    };
    s.apply_edit_at(&out.edit, now);
    applied(s, c)
}

/// Type an Alt + numpad code as its digits (`"0188"`, `"33"`).
pub fn press_alt_numpad<U: TextUnit>(
    s: &mut TypingSession<U>,
    c: &mut Composer,
    digits: &str,
    now: Instant,
) -> KeyEffect {
    if !ready(s, c, now) {
        return KeyEffect::Ignored;
    }
    let Some(out) = c.alt_numpad(digits, s.matcher().typed()) else {
        log::trace!("unmapped alt code {digits}");
        return KeyEffect::Unmapped;
    };
    s.apply_edit_at(&out.edit, now);
    applied(s, c)
}

/// Type an Alt + numpad code.
pub fn press_alt_code<U: TextUnit>(
    s: &mut TypingSession<U>,
    c: &mut Composer,
    code: u16,
    now: Instant,
) -> KeyEffect {
    if !ready(s, c, now) {
        return KeyEffect::Ignored;
    }
    let Some(out) = c.alt_code(code, s.matcher().typed()) else {
        log::trace!("unmapped alt code {code}");
        return KeyEffect::Unmapped;
    };
    s.apply_edit_at(&out.edit, now);
    applied(s, c)
}
