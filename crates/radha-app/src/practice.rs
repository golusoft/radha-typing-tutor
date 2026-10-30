//! One typing attempt, driven by real key presses.
//!
//! [`Practice`] glues the pieces the engine crates already provide: the
//! layout's [`Composer`] turns a key press into a text edit, the
//! [`TypingSession`] scores it, and the layout's [`Guide`] says which key
//! comes next. Unicode Hindi (GAIL, CBI) runs on akshar units with the guide
//! as the composition hook; QWERTY and the Kruti Dev legacy stream run on
//! `char` units.
//!
//! Everything here is RAM only. There is no database handle in this module:
//! a finished attempt becomes a [`SessionRecord`] that the caller writes once
//! (`RULES.MD` Rule 14). Every method that depends on time takes an explicit
//! [`Instant`], so tests drive the clock exactly.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use radha_content::ProductId;
use radha_core::akshar::segment;
use radha_core::{
    unit_states, Akshar, CompletionReason, DisplayMode, Outcome, SessionConfig, SessionState,
    TextUnit, TimerMode, TypingSession, UnitState,
};
use radha_db::{ErrorAggregate, ErrorKind, SessionMetricsRecord, SessionRecord};
use radha_layouts::plan::PlanStep;
use radha_layouts::{
    load_layout, press_alt_code, press_backspace, press_key, press_numpad_digit, Composer,
    Encoding, Finger, Guide, Hand, KeyEffect, KeyboardLayout, Keystroke, Layer, LayoutId,
    NextAction,
};

/// What one key event from the window means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    /// A character the OS keyboard produced. `altgr` when AltGr was held.
    Char { c: char, altgr: bool },
    /// Backspace.
    Backspace,
    /// Escape (pause, then back).
    Escape,
    /// Ctrl+R.
    Restart,
    /// F11: Rules → Focus mode on or off. Not Esc, which is the GAIL/CBI
    /// dead-key prefix (and pauses elsewhere).
    ToggleFocus,
    /// Modifier-only presses, arrows, other function keys, other shortcuts.
    Ignore,
}

/// Slint's key text for F11 (`Key.F11`).
pub const F11: char = '\u{f70e}';

/// Classify a Slint key event (`text` plus modifiers).
///
/// The OS keyboard is assumed to be US, as every Radha layout is defined on
/// the US board: the produced character identifies the physical key and its
/// Shift/Caps layer (`Keystroke::from_us_char`). Slint encodes special keys
/// as control characters (`\u{10}` Shift … `\u{18}`) and private-use code
/// points (arrows, F-keys); those are ignored.
#[must_use]
pub fn classify_key(text: &str, control: bool, alt: bool) -> KeyInput {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return KeyInput::Ignore;
    };
    match c {
        '\u{1b}' => return KeyInput::Escape,
        '\u{8}' => return KeyInput::Backspace,
        // Slint's Key.F11 (a private-use code point no layout produces).
        F11 => return KeyInput::ToggleFocus,
        _ => {}
    }
    let altgr = control && alt;
    if control && !alt {
        // Windows may deliver Ctrl+R as U+0012 instead of "r".
        return if matches!(c, 'r' | 'R' | '\u{12}') {
            KeyInput::Restart
        } else {
            KeyInput::Ignore
        };
    }
    // AltGr+letter can arrive as the letter's control code.
    let c = if altgr && ('\u{1}'..='\u{1a}').contains(&c) {
        char::from(b'a' + (c as u8) - 1)
    } else {
        c
    };
    if c.is_control() || ('\u{e000}'..='\u{f8ff}').contains(&c) {
        return KeyInput::Ignore;
    }
    if alt && !control && c.is_ascii() {
        // On a US Windows board the right Alt key reports plain Alt (no
        // synthesized Ctrl), so Alt + a printable key is the AltGr layer the
        // guide names ("AltGr+B" for ई). Digits stay out so Alt+numpad codes
        // are untouched; Alt+Space (window menu) and other non-printables
        // are not typing.
        return if c.is_ascii_graphic() && !c.is_ascii_digit() {
            KeyInput::Char { c, altgr: true }
        } else {
            KeyInput::Ignore
        };
    }
    KeyInput::Char { c, altgr }
}

/// The session, over the unit type of the layout's encoding.
#[derive(Debug, Clone)]
enum Engine {
    Chars(TypingSession<char>),
    Akshars(TypingSession<Akshar>),
}

/// Run `$body` with `$s` bound to the inner session, whatever its unit type.
macro_rules! with_session {
    ($engine:expr, $s:ident => $body:expr) => {
        match $engine {
            Engine::Chars($s) => $body,
            Engine::Akshars($s) => $body,
        }
    };
}

/// The live numbers of the typing screen.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveStats {
    pub state: SessionState,
    pub elapsed_seconds: f64,
    pub remaining_seconds: Option<f64>,
    pub wpm: f64,
    pub net_wpm: f64,
    pub accuracy: f64,
    pub errors: usize,
    pub completion: Option<CompletionReason>,
    /// Key depressions per hour.
    pub kdph: f64,
    /// Accepted presses.
    pub presses: u32,
    /// Committed words, and how many of them were correct.
    pub words: usize,
    pub words_correct: usize,
}

/// Everything the typing screen draws for one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct PracticeView {
    pub stats: LiveStats,
    /// One state per target unit, under the session's display mode.
    pub states: Vec<UnitState>,
    /// Target unit the next key is compared with.
    pub cursor: usize,
    /// The next key, for layouts with a guide panel.
    pub hint: Option<HintView>,
    /// Target-unit span of the word being typed (empty once all are done).
    pub word_span: std::ops::Range<usize>,
    /// Committed words none of whose units was typed (omitted words).
    pub skipped: Vec<std::ops::Range<usize>>,
}

/// The guide panel's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintView {
    /// Keycap legend, `Alt+165` for an Alt code.
    pub key: String,
    /// The physical key's unshifted US character (`" "` for Space), the id
    /// the virtual keyboard highlights; empty for an Alt code.
    pub key_id: String,
    pub shift: bool,
    pub altgr: bool,
    pub hand: &'static str,
    pub finger: &'static str,
    /// What the key emits.
    pub output: String,
    /// Kruti Dev: the Devanagari the font draws.
    pub meaning: Option<String>,
}

/// A key the typist missed most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeakKey {
    /// The expected unit.
    pub unit: String,
    /// The key sequence that types it.
    pub keys: String,
    pub errors: u32,
    pub attempts: u32,
}

/// The Result screen's figures.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultSummary {
    pub wpm: f64,
    pub net_wpm: f64,
    pub accuracy: f64,
    pub errors: usize,
    pub consistency: f64,
    pub elapsed_seconds: f64,
    pub word_accuracy: f64,
    pub backspaces: u32,
    pub characters_typed: usize,
    pub completion: Option<CompletionReason>,
    pub weak_keys: Vec<WeakKey>,
}

/// One accepted input, as recorded for crash recovery (TY-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoggedKey {
    Char {
        c: char,
        altgr: bool,
    },
    Backspace,
    /// Esc armed the GAIL/CBI dead-key layer for the next key.
    DeadKey,
}

/// One input of a crash-recovery replay ([`Practice::replay_timed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replayed {
    /// When it is replayed (the journalled active time).
    pub at: Instant,
    pub key: LoggedKey,
    /// Whether the session applied it.
    pub applied: bool,
    /// The unit index the cursor was on before the key.
    pub cursor: usize,
    /// The first character of that unit (what the key was aimed at).
    pub aimed: Option<char>,
}

/// One attempt at one exercise.
pub struct Practice {
    layout: &'static dyn KeyboardLayout,
    composer: Composer,
    engine: Engine,
    guide: Arc<Guide>,
    content: String,
    timer: TimerMode,
    mode: DisplayMode,
    units: Vec<String>,
    wall_start: Option<SystemTime>,
    /// Every applied input, in order (RAM; journalled on pause/blur).
    key_log: Vec<LoggedKey>,
    /// (time, code points typed) after every applied input, for the
    /// Result screen's minute-by-minute curve.
    timeline: Vec<(Instant, usize)>,
    /// Pause spans `(from, to)`; `to` is `None` while paused.
    pauses: Vec<(Instant, Option<Instant>)>,
}

impl std::fmt::Debug for Practice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Practice")
            .field("layout", &self.layout.id())
            .field("units", &self.units.len())
            .field("state", &self.state())
            .finish()
    }
}

fn build_engine(
    layout: &'static dyn KeyboardLayout,
    content: &str,
    timer: TimerMode,
    guide: &Arc<Guide>,
) -> Engine {
    let config = SessionConfig { timer };
    match layout.encoding() {
        Encoding::Unicode => Engine::Akshars(
            TypingSession::<Akshar>::configured(content, config).with_composition(guide.clone()),
        ),
        Encoding::Latin | Encoding::Legacy => {
            Engine::Chars(TypingSession::<char>::configured(content, config))
        }
    }
}

fn hand_name(hand: Hand) -> &'static str {
    match hand {
        Hand::Left => "Left",
        Hand::Right => "Right",
    }
}

fn finger_name(finger: Finger) -> &'static str {
    match finger {
        Finger::Pinky => "Little",
        Finger::Ring => "Ring",
        Finger::Middle => "Middle",
        Finger::Index => "Index",
        Finger::Thumb => "Thumb",
    }
}

fn step_label(step: &PlanStep) -> String {
    match step {
        PlanStep::Key(entry) => {
            let legend = entry.key.legend();
            match entry.layer {
                Layer::Base => legend.to_owned(),
                Layer::Shift => format!("Shift+{legend}"),
                Layer::AltGr => format!("AltGr+{legend}"),
                Layer::ShiftAltGr => format!("Shift+AltGr+{legend}"),
            }
        }
        PlanStep::Space => "Space".to_owned(),
        PlanStep::AltCode(code) => format!("Alt+{code}"),
    }
}

fn hint_view(action: &NextAction) -> HintView {
    match action {
        NextAction::Key(h) => HintView {
            key: h.legend.to_owned(),
            key_id: h.key.base_char().to_string(),
            shift: h.hold_shift,
            altgr: h.hold_altgr,
            hand: hand_name(h.hand),
            finger: finger_name(h.finger),
            output: h.output.to_owned(),
            meaning: h.meaning.map(str::to_owned),
        },
        NextAction::AltCode {
            code,
            output,
            meaning,
        } => HintView {
            key: format!("Alt+{code}"),
            key_id: String::new(),
            shift: false,
            altgr: false,
            hand: "Left",
            finger: "Thumb",
            output: output.to_string(),
            meaning: Some((*meaning).to_owned()),
        },
    }
}

/// One key of the on-screen keyboard (TY-08).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardKey {
    /// Unshifted US character: the highlight id.
    pub id: String,
    /// Keycap legend ("Q").
    pub legend: String,
    /// What the key types on this layout (base layer; Kruti Dev: the glyph).
    pub glyph: String,
    /// Shift layer glyph.
    pub shift_glyph: String,
    /// 0 little, 1 ring, 2 middle, 3 index, 4 thumb.
    pub finger: u8,
    pub left: bool,
}

/// The four key rows of `layout` with each key's glyphs and finger.
#[must_use]
pub fn board(layout: LayoutId) -> Vec<Vec<BoardKey>> {
    let Ok(l) = load_layout(layout) else {
        return Vec::new();
    };
    let table = l.table();
    radha_layouts::keys::rows()
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|key| {
                    let (hand, finger) = l.finger_for(key);
                    let glyph = |layer| table.keycap(key, layer).unwrap_or("").to_owned();
                    BoardKey {
                        id: key.base_char().to_string(),
                        legend: key.legend().to_owned(),
                        glyph: glyph(Layer::Base),
                        shift_glyph: glyph(Layer::Shift),
                        finger: match finger {
                            Finger::Pinky => 0,
                            Finger::Ring => 1,
                            Finger::Middle => 2,
                            Finger::Index => 3,
                            Finger::Thumb => 4,
                        },
                        left: hand == Hand::Left,
                    }
                })
                .collect()
        })
        .collect()
}

impl Practice {
    /// Prepare an attempt at `content` on `layout`. Nothing is timed until
    /// the first key.
    ///
    /// # Errors
    ///
    /// When the layout's table fails validation (the layout is unavailable).
    pub fn new(
        layout: LayoutId,
        content: &str,
        timer: TimerMode,
        mode: DisplayMode,
    ) -> anyhow::Result<Self> {
        let layout = load_layout(layout).map_err(|e| anyhow::anyhow!("{e}"))?;
        let guide = Arc::new(Guide::new(layout, content));
        let engine = build_engine(layout, content, timer, &guide);
        let units: Vec<String> = with_session!(&engine, s => s
            .matcher()
            .target_units()
            .iter()
            .map(|u| u.as_text().into_owned())
            .collect());
        Ok(Practice {
            layout,
            composer: Composer::new(layout),
            engine,
            guide,
            content: content.to_owned(),
            timer,
            mode,
            units,
            wall_start: None,
            key_log: Vec::new(),
            timeline: Vec::new(),
            pauses: Vec::new(),
        })
    }

    /// Start over with the same text and settings.
    pub fn restart(&mut self) {
        self.engine = build_engine(self.layout, &self.content, self.timer, &self.guide);
        self.composer.reset();
        self.wall_start = None;
        self.key_log.clear();
        self.timeline.clear();
        self.pauses.clear();
    }

    /// Every applied input so far.
    #[must_use]
    pub fn key_log(&self) -> &[LoggedKey] {
        &self.key_log
    }

    /// Whether a pre-posed sign is waiting (Backspace cancels it).
    #[must_use]
    pub fn has_preedit(&self) -> bool {
        self.composer.preedit().is_some()
    }

    /// The composer's pending preedit text.
    #[must_use]
    pub fn preedit(&self) -> Option<&'static str> {
        self.composer.preedit()
    }

    fn note_applied(&mut self, effect: KeyEffect, key: LoggedKey, now: Instant) {
        if matches!(effect, KeyEffect::Applied { .. }) {
            self.key_log.push(key);
            let len = with_session!(&self.engine, s => s.matcher().typed().chars().count());
            self.timeline.push((now, len));
        }
        self.drop_unused_dead_key();
    }

    /// An Esc that was disarmed without typing anything (cancelled, or the
    /// next key was unmapped on the Esc layer) leaves no trace in the key
    /// log, so crash recovery never re-arms it for a later key.
    fn drop_unused_dead_key(&mut self) {
        if !self.composer.dead_key_armed() && self.key_log.last() == Some(&LoggedKey::DeadKey) {
            self.key_log.pop();
        }
    }

    /// Active (unpaused) seconds from the first input to `t`.
    fn active_seconds(&self, t: Instant) -> f64 {
        let Some(&(start, _)) = self.timeline.first() else {
            return 0.0;
        };
        let paused: f64 = self
            .pauses
            .iter()
            .filter(|(from, _)| *from < t)
            .map(|(from, to)| {
                let end = to.unwrap_or(t).min(t);
                end.saturating_duration_since(*from).as_secs_f64()
            })
            .sum();
        (t.saturating_duration_since(start).as_secs_f64() - paused).max(0.0)
    }

    /// `(active seconds, code points typed)` after every input.
    #[must_use]
    pub fn progress_points(&self) -> Vec<(f64, usize)> {
        self.timeline
            .iter()
            .map(|&(t, n)| (self.active_seconds(t), n))
            .collect()
    }

    /// What the pressed key is on this layout (the mistake coach).
    #[must_use]
    pub fn pressed_hint(&self, c: char, altgr: bool) -> Option<HintView> {
        let stroke = Keystroke::from_us_char(c, altgr)?;
        self.guide
            .hint_for(stroke, false)
            .map(|h| hint_view(&NextAction::Key(h)))
    }

    /// The target units as text, in order.
    #[must_use]
    pub fn units(&self) -> &[String] {
        &self.units
    }

    /// The layout typed on.
    #[must_use]
    pub fn layout(&self) -> LayoutId {
        self.layout.id()
    }

    /// The display mode.
    #[must_use]
    pub fn mode(&self) -> DisplayMode {
        self.mode
    }

    /// The timer mode.
    #[must_use]
    pub fn timer(&self) -> TimerMode {
        self.timer
    }

    /// Lifecycle state.
    #[must_use]
    pub fn state(&self) -> SessionState {
        with_session!(&self.engine, s => s.state())
    }

    /// What was typed so far.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn typed(&self) -> String {
        with_session!(&self.engine, s => s.matcher().typed().to_owned())
    }

    fn note_start(&mut self, was_idle: bool) {
        if was_idle && self.state() != SessionState::Idle {
            self.wall_start = Some(SystemTime::now());
        }
    }

    /// A character key. Unknown characters (not on a US board and not one of
    /// the layout's Alt codes) are `Unmapped` and count nothing.
    pub fn press(&mut self, c: char, altgr: bool, now: Instant) -> KeyEffect {
        let was_idle = self.state() == SessionState::Idle;
        let effect = if let Some(stroke) = Keystroke::from_us_char(c, altgr) {
            with_session!(&mut self.engine, s => press_key(s, &mut self.composer, stroke, now))
        } else if let Some(code) = self.alt_code_for(c) {
            with_session!(&mut self.engine, s => press_alt_code(s, &mut self.composer, code, now))
        } else if let Some(digit) = self.numpad_digit_for(c) {
            // A Devanagari digit from the OS (the numpad with Caps Lock off
            // under a Hindi input method): the layout's numpad rule.
            with_session!(&mut self.engine, s => press_numpad_digit(s, &mut self.composer, digit, false, now))
        } else {
            KeyEffect::Unmapped
        };
        self.note_start(was_idle);
        self.note_applied(effect, LoggedKey::Char { c, altgr }, now);
        effect
    }

    /// Backspace under the session's backspace rule (TY-05): Full deletes
    /// anywhere, One-word never past the start of the word being typed,
    /// Off never. `None` when the rule refuses it (nothing is counted).
    pub fn backspace_under(
        &mut self,
        policy: crate::typing_rules::BackspacePolicy,
        now: Instant,
    ) -> Option<KeyEffect> {
        let allowed = match policy {
            crate::typing_rules::BackspacePolicy::Full => true,
            crate::typing_rules::BackspacePolicy::Off => false,
            crate::typing_rules::BackspacePolicy::OneWord => {
                policy.allows(&self.typed(), self.has_preedit())
            }
        };
        allowed.then(|| self.backspace(now))
    }

    /// Backspace.
    pub fn backspace(&mut self, now: Instant) -> KeyEffect {
        let effect =
            with_session!(&mut self.engine, s => press_backspace(s, &mut self.composer, now));
        self.note_applied(effect, LoggedKey::Backspace, now);
        effect
    }

    /// Replay a recorded key log (crash recovery), spacing the inputs
    /// evenly over `elapsed_seconds` before `now`, then pause.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn replay(&mut self, log: &[LoggedKey], elapsed_seconds: f64, now: Instant) {
        self.replay_timed(log, &[], elapsed_seconds, now, None);
    }

    /// Active milliseconds (pauses excluded, from the first input) of every
    /// applied key, in key-log order (an Esc dead key has none). Journalled
    /// with the key log so recovery replays the real rhythm.
    #[must_use]
    pub fn key_ms(&self) -> Vec<u32> {
        self.progress_points()
            .iter()
            .map(|&(s, _)| {
                let ms = (s * 1000.0).round();
                if ms >= f64::from(u32::MAX) {
                    u32::MAX
                } else {
                    ms.max(0.0) as u32
                }
            })
            .collect()
    }

    /// [`Practice::replay`] at the journalled key times: `key_ms` holds
    /// one entry per Char/Backspace of `log` ([`Practice::key_ms`]); when it
    /// does not match the log (an older journal), the inputs are spaced
    /// evenly. `on_key`, when given, sees every replayed input with the
    /// unit the cursor was on before it (the Flow rhythm engine).
    pub fn replay_timed(
        &mut self,
        log: &[LoggedKey],
        key_ms: &[u32],
        elapsed_seconds: f64,
        now: Instant,
        mut on_key: Option<&mut dyn FnMut(&Replayed)>,
    ) {
        if log.is_empty() {
            return;
        }
        let timed = key_ms.len()
            == log
                .iter()
                .filter(|k| !matches!(k, LoggedKey::DeadKey))
                .count();
        let last_ms = if timed {
            key_ms.iter().copied().max().unwrap_or(0)
        } else {
            0
        };
        let secs = elapsed_seconds.max(f64::from(last_ms) / 1000.0);
        let span = std::time::Duration::from_secs_f64(secs.clamp(0.0, 86_400.0));
        let start = now.checked_sub(span).unwrap_or(now);
        let n = u32::try_from(log.len()).unwrap_or(u32::MAX).max(1);
        let mut applied_no = 0usize;
        for (i, key) in log.iter().enumerate() {
            let t = if timed {
                let ms = key_ms.get(applied_no).copied().unwrap_or(last_ms);
                (start + std::time::Duration::from_millis(u64::from(ms))).min(now)
            } else {
                let i = u32::try_from(i).unwrap_or(u32::MAX);
                start + span.mul_f64(f64::from(i) / f64::from(n))
            };
            let cursor = on_key.as_ref().map(|_| self.cursor_at(t));
            let effect = match *key {
                LoggedKey::Char { c, altgr } => {
                    applied_no += 1;
                    self.press(c, altgr, t)
                }
                LoggedKey::Backspace => {
                    applied_no += 1;
                    self.backspace(t)
                }
                LoggedKey::DeadKey => {
                    self.arm_dead_key();
                    KeyEffect::Ignored
                }
            };
            if let (Some(f), Some(cursor)) = (on_key.as_mut(), cursor) {
                f(&Replayed {
                    at: t,
                    key: *key,
                    applied: matches!(effect, KeyEffect::Applied { .. }),
                    cursor,
                    aimed: self.units.get(cursor).and_then(|u| u.chars().next()),
                });
            }
        }
        if self.state() == SessionState::Active {
            self.toggle_pause(now);
        }
    }

    /// The unit index the next key is aimed at.
    fn cursor_at(&self, now: Instant) -> usize {
        with_session!(&self.engine, s => s.live_at(now).1.cursor)
    }

    /// Let a countdown expire. Returns `true` when this call completed it.
    pub fn tick(&mut self, now: Instant) -> bool {
        let done = with_session!(&mut self.engine, s => s.tick_at(now));
        if done {
            self.composer.reset();
        }
        done
    }

    /// Pause an active attempt or resume a paused one. Other states are left
    /// as they are. Returns the new state.
    pub fn toggle_pause(&mut self, now: Instant) -> SessionState {
        let before = self.state();
        let result = match before {
            SessionState::Active => with_session!(&mut self.engine, s => s.pause_at(now)),
            SessionState::Paused => with_session!(&mut self.engine, s => s.resume_at(now)),
            SessionState::Idle | SessionState::Complete => Ok(()),
        };
        if let Err(e) = result {
            log::debug!("pause toggle ignored: {e}");
        }
        match (before, self.state()) {
            (SessionState::Active, SessionState::Paused) => self.pauses.push((now, None)),
            (SessionState::Paused, SessionState::Active) => {
                if let Some(last) = self.pauses.last_mut() {
                    last.1 = Some(now);
                }
            }
            _ => {}
        }
        self.state()
    }

    /// Pause an active attempt (window blur); other states are untouched.
    /// Returns `true` when it paused.
    pub fn pause_if_active(&mut self, now: Instant) -> bool {
        self.state() == SessionState::Active && self.toggle_pause(now) == SessionState::Paused
    }

    /// Esc on a layout with an Esc dead-key layer (GAIL, CBI) while the
    /// attempt takes input: the next key is read from that layer. `false`
    /// when the layout has none or the attempt is paused or complete.
    pub fn arm_dead_key(&mut self) -> bool {
        if !matches!(self.state(), SessionState::Idle | SessionState::Active) {
            return false;
        }
        let already = self.composer.dead_key_armed();
        let armed = self.composer.arm_dead_key();
        if armed && !already {
            self.key_log.push(LoggedKey::DeadKey);
        }
        armed
    }

    /// Whether an Esc dead key is waiting for its key (O(1)).
    #[must_use]
    pub fn dead_key_armed(&self) -> bool {
        self.composer.dead_key_armed()
    }

    /// Disarm an armed dead key. Returns whether one was armed.
    pub fn cancel_dead_key(&mut self) -> bool {
        let was_armed = self.composer.cancel_dead_key();
        self.drop_unused_dead_key();
        was_armed
    }

    /// End the attempt now (the user submits).
    pub fn complete(&mut self, now: Instant) {
        with_session!(&mut self.engine, s => s.complete_at(now));
        self.composer.reset();
    }

    /// The numpad digit whose Caps-Lock-off output on this layout is `c`.
    fn numpad_digit_for(&self, c: char) -> Option<u8> {
        if !('\u{0966}'..='\u{096F}').contains(&c) {
            return None;
        }
        let digit = u8::try_from(u32::from(c) - 0x0966).ok()?;
        let mut buf = [0u8; 4];
        let wanted: &str = c.encode_utf8(&mut buf);
        (self.layout.numpad_digit(digit, false) == Some(wanted)).then_some(digit)
    }

    fn alt_code_for(&self, c: char) -> Option<u16> {
        if c.is_ascii() {
            return None;
        }
        (0u16..=255).find(|&code| self.layout.alt_code(code).is_some_and(|e| e.ansi == c))
    }

    /// Everything the typing screen shows at `now`.
    #[must_use]
    pub fn view(&self, now: Instant) -> PracticeView {
        let (stats, states, cursor, pending_tail, word_span) = with_session!(&self.engine, s => {
            // One matcher pass for both the numbers and the display states.
            let (snap, e) = s.live_at(now);
            let tail = if e.pending {
                // Only the last word can hold the pending akshar.
                let typed = s.matcher().typed();
                let start = typed
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace())
                    .map_or(0, |(i, c)| i + c.len_utf8());
                segment(typed.get(start..).unwrap_or("")).pop()
            } else {
                None
            };
            let words = e.word_outcomes.len();
            let words_correct = e
                .word_outcomes
                .iter()
                .filter(|w| matches!(w, radha_core::WordOutcome::Correct))
                .count();
            let stats = LiveStats {
                state: snap.state,
                elapsed_seconds: snap.elapsed_seconds,
                remaining_seconds: snap.remaining_seconds,
                wpm: snap.wpm,
                net_wpm: snap.net_wpm,
                accuracy: snap.accuracy,
                errors: snap.error_count,
                completion: snap.completion,
                kdph: snap.kdph,
                presses: snap.presses,
                words,
                words_correct,
            };
            let span = e
                .current_word
                .and_then(|w| e.word_spans.get(w).cloned())
                .unwrap_or(0..0);
            let skipped: Vec<std::ops::Range<usize>> = e
                .word_spans
                .iter()
                .zip(e.word_outcomes.iter())
                .filter(|(span, outcome)| {
                    matches!(outcome, radha_core::WordOutcome::Incorrect)
                        && e
                            .target_outcome
                            .get((*span).clone())
                            .is_some_and(|slots| slots.iter().all(Option::is_none))
                })
                .map(|(span, _)| span.clone())
                .collect();
            (stats, unit_states(&e, self.mode), e.cursor, tail, (span, skipped))
        });
        let (word_span, skipped) = word_span;
        let hint = if stats.state == SessionState::Complete {
            None
        } else {
            self.hint_at(cursor, pending_tail.as_deref())
        };
        PracticeView {
            stats,
            states,
            cursor,
            hint,
            word_span,
            skipped,
        }
    }

    fn hint_at(&self, cursor: usize, pending_tail: Option<&str>) -> Option<HintView> {
        self.guide
            .next_action(cursor, pending_tail, self.composer.preedit(), false)
            .map(|a| hint_view(&a))
    }

    /// The physical keys (unshifted US characters) that type `unit`.
    #[must_use]
    pub fn physical_keys_for(&self, unit: &str) -> Vec<char> {
        if let Some(entry) = self.guide.lone_preposed(unit) {
            return vec![entry.key.base_char()];
        }
        self.guide
            .plan_for(unit)
            .map(|p| {
                p.steps
                    .iter()
                    .filter_map(|s| s.keystroke().map(|k| k.key.base_char()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The key sequence that types `unit` on this layout ("" when none).
    #[must_use]
    pub fn keys_for(&self, unit: &str) -> String {
        if unit.chars().all(char::is_whitespace) && !unit.is_empty() {
            return "Space".to_owned();
        }
        if let Some(entry) = self.guide.lone_preposed(unit) {
            return step_label(&PlanStep::Key(entry));
        }
        self.guide
            .plan_for(unit)
            .map(|p| p.steps.iter().map(step_label).collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
    }

    /// Per-unit tallies `(unit, attempts, errors, mean latency ms)`.
    #[must_use]
    pub fn unit_tallies(&self) -> Vec<(String, u32, u32, Option<f64>)> {
        with_session!(&self.engine, s => s
            .key_stats()
            .sorted()
            .into_iter()
            .map(|(u, t)| (
                u.as_text().into_owned(),
                t.attempts,
                t.errors,
                t.mean_latency().map(|d| d.as_secs_f64() * 1000.0),
            ))
            .collect())
    }

    /// Word-by-word comparison of the committed words, for the Result
    /// screen's diff inspector.
    #[must_use]
    pub fn word_diff(&self) -> Vec<crate::report::WordDiff> {
        with_session!(&self.engine, s => {
            let e = s.evaluation();
            let kinds = radha_core::word_mistakes(&e);
            let target = s.matcher().target_units();
            let typed: Vec<&str> = s.matcher().typed().split_whitespace().collect();
            crate::report::word_diff(
                &e.word_spans
                    .iter()
                    .map(|r| target.get(r.clone()).unwrap_or(&[]).iter().map(|u| u.as_text()).collect::<String>())
                    .collect::<Vec<_>>(),
                &typed,
                &kinds,
            )
        })
    }

    /// Half / full mistake counts (word level, `radha_core::mistake`).
    #[must_use]
    pub fn mistake_counts(&self, now: Instant) -> (usize, usize) {
        with_session!(&self.engine, s => {
            let m = s.snapshot_at(now).mistakes;
            (m.word_full(), m.word_half())
        })
    }

    /// The final (or current) figures for the Result screen.
    #[must_use]
    pub fn summary(&self, now: Instant) -> ResultSummary {
        with_session!(&self.engine, s => {
            let snap = s.snapshot_at(now);
            let mut weak: Vec<WeakKey> = s
                .key_stats()
                .sorted()
                .into_iter()
                .filter(|(u, t)| t.errors > 0 && !u.is_whitespace())
                .map(|(u, t)| {
                    let unit = u.as_text().into_owned();
                    WeakKey {
                        keys: self.keys_for(&unit),
                        unit,
                        errors: t.errors,
                        attempts: t.attempts,
                    }
                })
                .collect();
            weak.sort_by(|a, b| b.errors.cmp(&a.errors).then(b.attempts.cmp(&a.attempts)));
            weak.truncate(5);
            ResultSummary {
                wpm: snap.wpm,
                net_wpm: snap.net_wpm,
                accuracy: snap.accuracy,
                errors: snap.error_count,
                consistency: snap.consistency,
                elapsed_seconds: snap.elapsed_seconds,
                word_accuracy: snap.word_accuracy,
                backspaces: snap.backspaces,
                characters_typed: snap.characters_typed,
                completion: snap.completion,
                weak_keys: weak,
            }
        })
    }

    /// The completed attempt as one database record.
    #[must_use]
    pub fn record(
        &self,
        profile_id: i64,
        exercise_id: i64,
        product: ProductId,
        now: Instant,
    ) -> SessionRecord {
        let (metrics, errors, elapsed, completion) = with_session!(&self.engine, s => {
            let snap = s.snapshot_at(now);
            let correct = snap.outcomes.iter().filter(|o| !o.is_error()).count();
            let latency = {
                let (total, samples) = s.key_stats().sorted().iter().fold(
                    (std::time::Duration::ZERO, 0u32),
                    |(t, n), (_, k)| (t + k.latency_total, n + k.latency_samples),
                );
                (samples > 0).then(|| total.as_secs_f64() * 1000.0 / f64::from(samples))
            };
            let metrics = SessionMetricsRecord {
                gross_wpm: snap.wpm,
                net_wpm: snap.net_wpm,
                accuracy: snap.accuracy,
                consistency: Some(snap.consistency),
                word_accuracy: Some(snap.word_accuracy),
                correct_chars: count32(correct),
                wrong_chars: count32(snap.breakdown.wrong),
                extra_chars: count32(snap.breakdown.extra),
                missing_chars: count32(snap.breakdown.missing),
                backspaces: snap.backspaces,
                space_errors: count32(snap.breakdown.space),
                avg_latency_ms: latency,
            };
            (
                metrics,
                self.aggregate_errors(&snap.outcomes),
                snap.elapsed_seconds,
                snap.completion,
            )
        });
        let completed_wall = SystemTime::now();
        let started_wall = self.wall_start.unwrap_or(completed_wall);
        SessionRecord {
            profile_id,
            exercise_id,
            product,
            started_at: iso_utc(started_wall),
            completed_at: iso_utc(completed_wall),
            timer_secs: self.timer.limit().map(|l| l.seconds()),
            elapsed_secs: elapsed.round().clamp(0.0, f64::from(u32::MAX)) as u32,
            display_mode: match self.mode {
                DisplayMode::Key => "key",
                DisplayMode::Word => "word",
            },
            guide_mode: match self.layout.id() {
                LayoutId::Qwerty => "off",
                _ => "minimal",
            },
            // PRD 17.2: Flow products are always timed; elsewhere a timer
            // means TEST and none means LEARN. Hindi Learn's track comes
            // with the P05 session context, so it is not guessed here.
            mode: match product {
                ProductId::TypingFlow | ProductId::NumberPunctuation => Some("flow"),
                ProductId::HindiLearn(_) => None,
                _ if self.timer.limit().is_some() => Some("test"),
                _ => Some("learn"),
            },
            finished: completion == Some(CompletionReason::Finished),
            day: iso_utc(completed_wall).chars().take(10).collect(),
            metrics,
            errors,
            // The Typing Flow screen attaches its rhythm analysis.
            flow: None,
        }
    }

    /// Uncorrected errors, one row per (key, expected unit, kind).
    fn aggregate_errors<U: TextUnit>(&self, outcomes: &[Outcome<U>]) -> Vec<ErrorAggregate> {
        let mut rows: BTreeMap<(String, String, ErrorKind), u32> = BTreeMap::new();
        for o in outcomes {
            let (kind, expected, typed) = match o {
                Outcome::Correct(_) => continue,
                Outcome::Wrong { expected, typed } => (ErrorKind::Wrong, Some(expected), typed),
                Outcome::Missing(u) => (ErrorKind::Missing, Some(u), u),
                Outcome::Extra(u) => (ErrorKind::Extra, None, u),
            };
            let attributed = expected.unwrap_or(typed);
            let kind = if attributed.is_whitespace() || typed.is_whitespace() {
                ErrorKind::Space
            } else {
                kind
            };
            let unit = attributed.as_text().into_owned();
            let key = match self.keys_for(&unit) {
                k if k.is_empty() => unit,
                k => k,
            };
            let expected = expected
                .map(|e| e.as_text().into_owned())
                .unwrap_or_default();
            *rows.entry((key, expected, kind)).or_default() += 1;
        }
        rows.into_iter()
            .map(
                |((physical_key, expected_char, kind), occurrences)| ErrorAggregate {
                    physical_key,
                    expected_char,
                    kind,
                    occurrences,
                },
            )
            .collect()
    }
}

fn count32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a wall-clock time (UTC).
#[must_use]
pub fn iso_utc(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

/// `m:ss` for a number of seconds.
#[must_use]
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use radha_core::{TimeLimit, TimerPreset};
    use radha_layouts::krutidev::to_legacy;
    use std::time::Duration;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    /// Type `keys` (US characters) one per 200 ms from `t0`.
    fn type_keys(p: &mut Practice, keys: &str, t0: Instant) -> Instant {
        let mut t = t0;
        for c in keys.chars() {
            if c == '\u{8}' {
                p.backspace(t);
            } else {
                p.press(c, false, t);
            }
            t = at(t, 200);
        }
        t
    }

    fn qwerty(text: &str) -> Practice {
        Practice::new(
            LayoutId::Qwerty,
            text,
            TimerMode::Stopwatch,
            DisplayMode::Word,
        )
        .unwrap()
    }

    #[test]
    fn backspace_full_deletes_across_words() {
        use crate::typing_rules::BackspacePolicy;
        let t0 = Instant::now();
        let mut p = qwerty("the cat sat");
        let t = type_keys(&mut p, "the ca", t0);
        for _ in 0..4 {
            assert!(p.backspace_under(BackspacePolicy::Full, t).is_some());
        }
        assert_eq!(p.typed(), "th", "Full goes back past the space");
    }

    #[test]
    fn backspace_one_word_stops_at_the_current_word_start() {
        use crate::typing_rules::BackspacePolicy;
        let t0 = Instant::now();
        let mut p = qwerty("the cat sat");
        let t = type_keys(&mut p, "the cxt", t0);
        assert!(p.backspace_under(BackspacePolicy::OneWord, t).is_some());
        assert!(p.backspace_under(BackspacePolicy::OneWord, t).is_some());
        assert!(p.backspace_under(BackspacePolicy::OneWord, t).is_some());
        assert_eq!(p.typed(), "the ", "the current word is cleared");
        assert_eq!(
            p.backspace_under(BackspacePolicy::OneWord, t),
            None,
            "never back into the committed word"
        );
        assert_eq!(p.typed(), "the ");
        let keys = p.key_log().len();
        assert_eq!(keys, 7 + 3, "a refused Backspace is not logged");
        // Typing on still works and is scored.
        type_keys(&mut p, "cat", t);
        assert_eq!(p.typed(), "the cat");
    }

    #[test]
    fn backspace_off_never_deletes_and_counts_nothing() {
        use crate::typing_rules::BackspacePolicy;
        let t0 = Instant::now();
        let mut p = qwerty("the cat");
        let t = type_keys(&mut p, "thx", t0);
        let errors = p.view(t).stats.errors;
        assert_eq!(p.backspace_under(BackspacePolicy::Off, t), None);
        assert_eq!(p.typed(), "thx");
        assert_eq!(p.key_log().len(), 3);
        assert_eq!(p.view(t).stats.errors, errors, "stats unchanged");
    }

    /// The US character that strikes a hint's key.
    fn hint_char(h: &HintView) -> char {
        let base = h.key_id.chars().next().expect("a key hint");
        if h.shift {
            radha_layouts::PhysicalKey::from_base(base)
                .map(|k| k.shifted_char())
                .unwrap_or(base)
        } else {
            base
        }
    }

    /// Per-keystroke cost of the hot path the Typing screen runs on every
    /// key: `press` + `view` (engine, matcher, guide hint). Prints p50/p99;
    /// the < 1 ms target is asserted in optimised builds only (debug builds
    /// are 10-30x slower and only report).
    fn bench(layout: LayoutId, text: &str, keys: usize) -> (f64, f64) {
        let mut p = Practice::new(layout, text, TimerMode::Stopwatch, DisplayMode::Key).unwrap();
        let mut t = Instant::now();
        let mut costs = Vec::with_capacity(keys);
        let mut hint = p.view(t).hint;
        for _ in 0..keys {
            let Some(h) = hint.clone() else { break };
            let c = hint_char(&h);
            let start = Instant::now();
            p.press(c, h.altgr, t);
            let v = p.view(t);
            costs.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(v.stats.errors, 0, "following the guide makes no mistake");
            hint = v.hint;
            t += Duration::from_millis(150);
        }
        let first: f64 = costs.iter().take(100).sum::<f64>() / 100.0;
        let last: f64 = costs.iter().rev().take(100).sum::<f64>() / 100.0;
        costs.sort_by(f64::total_cmp);
        let pick = |q: f64| costs[((costs.len() - 1) as f64 * q).round() as usize];
        eprintln!(
            "  {layout:?}: p90 {:.3} ms, max {:.3} ms, mean of first 100 keys {first:.3} ms, last 100 {last:.3} ms",
            pick(0.9),
            pick(1.0)
        );
        (pick(0.5), pick(0.99))
    }

    #[test]
    fn esc_arms_the_gail_and_cbi_dead_key_layer_and_is_replayed() {
        for layout in [LayoutId::RemingtonGail, LayoutId::RemingtonCbi] {
            let mut p = Practice::new(layout, "फ़", TimerMode::Stopwatch, DisplayMode::Key).unwrap();
            let t = Instant::now();
            assert!(p.arm_dead_key(), "{layout:?}");
            assert!(matches!(p.press('i', false, t), KeyEffect::Applied { .. }));
            assert_eq!(p.typed(), radha_core::akshar::normalize("फ़"), "{layout:?}");
            let log = p.key_log().to_vec();
            assert_eq!(log.first(), Some(&LoggedKey::DeadKey));
            // Crash recovery replays the Esc too.
            let mut q = Practice::new(layout, "फ़", TimerMode::Stopwatch, DisplayMode::Key).unwrap();
            q.replay(&log, 1.0, t);
            assert_eq!(q.typed(), p.typed());
        }
        // A second Esc disarms; QWERTY and InScript have no Esc layer.
        let mut p = Practice::new(
            LayoutId::RemingtonGail,
            "क",
            TimerMode::Stopwatch,
            DisplayMode::Key,
        )
        .unwrap();
        assert!(p.arm_dead_key());
        assert!(p.cancel_dead_key());
        assert!(!p.cancel_dead_key());
        for layout in [LayoutId::Qwerty, LayoutId::Inscript] {
            let mut p = Practice::new(layout, "a", TimerMode::Stopwatch, DisplayMode::Key).unwrap();
            assert!(!p.arm_dead_key(), "{layout:?}");
        }
    }

    #[test]
    fn a_cancelled_or_unused_esc_is_not_replayed_by_crash_recovery() {
        for layout in [LayoutId::RemingtonGail, LayoutId::RemingtonCbi] {
            let t = Instant::now();
            // Esc, Esc (cancel), i: the user typed प, not फ़.
            let mut p =
                Practice::new(layout, "पप", TimerMode::Stopwatch, DisplayMode::Key).unwrap();
            assert!(p.arm_dead_key(), "{layout:?}");
            assert!(p.cancel_dead_key(), "{layout:?}");
            assert!(matches!(p.press('i', false, t), KeyEffect::Applied { .. }));
            // Esc, then a key with no Esc character (consumed, nothing typed), then i.
            assert!(p.arm_dead_key(), "{layout:?}");
            assert!(!matches!(p.press('q', false, t), KeyEffect::Applied { .. }));
            assert!(matches!(p.press('i', false, t), KeyEffect::Applied { .. }));
            assert_eq!(p.typed(), "पप", "{layout:?}");
            assert!(!p.key_log().contains(&LoggedKey::DeadKey), "{layout:?}");
            // Through the journal encoding, as a real restart does.
            let log = crate::recovery::decode(&crate::recovery::encode(p.key_log()));
            let mut q =
                Practice::new(layout, "पप", TimerMode::Stopwatch, DisplayMode::Key).unwrap();
            q.replay(&log, 1.0, t);
            assert_eq!(q.typed(), p.typed(), "{layout:?}");
        }
    }

    #[test]
    fn the_esc_armed_chip_shows_only_while_the_dead_key_waits() {
        let t = Instant::now();
        for layout in [LayoutId::RemingtonGail, LayoutId::RemingtonCbi] {
            let mut p =
                Practice::new(layout, "फ़प", TimerMode::Stopwatch, DisplayMode::Key).unwrap();
            assert!(!p.dead_key_armed(), "{layout:?}");
            assert!(p.arm_dead_key());
            assert!(p.dead_key_armed(), "armed after Esc ({layout:?})");
            p.press('i', false, t);
            assert!(!p.dead_key_armed(), "the key consumed it ({layout:?})");
            assert!(p.arm_dead_key());
            assert!(p.cancel_dead_key());
            assert!(!p.dead_key_armed(), "a second Esc cancels it ({layout:?})");
            assert!(p.arm_dead_key());
            p.restart();
            assert!(!p.dead_key_armed(), "restart clears it ({layout:?})");
        }
        let mut p = Practice::new(
            LayoutId::Qwerty,
            "a",
            TimerMode::Stopwatch,
            DisplayMode::Key,
        )
        .unwrap();
        p.arm_dead_key();
        assert!(!p.dead_key_armed(), "QWERTY has no Esc layer");
    }

    #[test]
    fn inscript_types_logical_order_word_vectors() {
        // Book 02: विद्या = b f o d / e; reph typed first (j d k = र्क).
        let mut p = Practice::new(
            LayoutId::Inscript,
            "विद्या र्क",
            TimerMode::Stopwatch,
            DisplayMode::Key,
        )
        .unwrap();
        type_keys(&mut p, "bfod/e jdk", Instant::now());
        assert_eq!(p.typed(), "विद्या र्क");
        assert_eq!(p.state(), SessionState::Complete);
    }

    #[test]
    fn keystroke_cost_benchmark() {
        let words = [
            "कार्य",
            "हिन्दी",
            "भारत",
            "नमस्ते",
            "क्षेत्र",
            "अर्थ",
            "विद्यार्थी",
            "शिक्षा",
        ];
        let hindi: Vec<&str> = (0..600).map(|i| words[i % words.len()]).collect();
        let hindi = hindi.join(" ");
        let english: Vec<&str> = (0..600)
            .map(|i| ["typing", "practice", "makes", "every", "word", "flow"][i % 6])
            .collect();
        let english = english.join(" ");
        assert!(radha_layouts::inscript::can_type(&hindi));
        for (name, layout, text) in [
            ("GAIL 600 words", LayoutId::RemingtonGail, hindi.as_str()),
            ("InScript 600 words", LayoutId::Inscript, hindi.as_str()),
            ("QWERTY 600 words", LayoutId::Qwerty, english.as_str()),
        ] {
            let (p50, p99) = bench(layout, text, 1500);
            eprintln!(
                "keystroke benchmark [{}] {name}: p50 {p50:.3} ms, p99 {p99:.3} ms",
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                }
            );
            if !cfg!(debug_assertions) {
                assert!(p99 < 1.0, "{name}: p99 {p99:.3} ms exceeds the 1 ms target");
            }
        }
    }

    /// Typing Flow per-key hot path on the longest shipped Flow passage:
    /// `press` + rhythm `key_at` + `view` + `fill_bars` + `label`, what the
    /// Flow screen runs per key before writing Slint. Prints a latency
    /// histogram; p99 < 1 ms is asserted in optimised builds.
    #[test]
    fn flow_keystroke_latency_histogram() {
        use radha_content::import::{find_bundle_file, parse_bundle_bytes};
        use radha_content::ContentTarget;
        use radha_core::flow::{CadenceBar, FlowConfig, FlowTracker};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/exercises");
        let text = find_bundle_file(&dir, ContentTarget::Product(ProductId::TypingFlow))
            .and_then(|path| {
                let bytes = std::fs::read(&path).ok()?;
                parse_bundle_bytes(&path, &bytes).ok()
            })
            .and_then(|b| b.exercises.into_iter().max_by_key(|e| e.content.len()))
            .map(|e| e.content)
            .unwrap_or_else(|| "steady hands keep an even beat ".repeat(600));
        let mut p = Practice::new(
            LayoutId::Qwerty,
            &text,
            TimerMode::Stopwatch,
            DisplayMode::Word,
        )
        .unwrap();
        let mut tracker = FlowTracker::new(FlowConfig::shipped());
        let mut bars = [CadenceBar::default(); crate::rhythm::WAVE_BARS];
        let units: Vec<char> = p
            .units()
            .iter()
            .map(|u| u.chars().next().unwrap_or(' '))
            .collect();
        let mut t = Instant::now();
        let mut cursor = 0usize;
        let mut costs = Vec::with_capacity(3000);
        let mut jitter = 0x9E37_79B9_u64;
        for i in 0..3000usize {
            let Some(&aimed) = units.get(cursor) else {
                break;
            };
            let c = if i % 37 == 36 { 'x' } else { aimed };
            let start = Instant::now();
            p.press(c, false, t);
            tracker.key_at(t, aimed, c == aimed);
            let v = p.view(t);
            tracker.fill_bars(&mut bars);
            let _ = tracker.label();
            costs.push(start.elapsed().as_secs_f64() * 1000.0);
            cursor = v.cursor;
            jitter = jitter
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            t += Duration::from_millis(150 + (jitter >> 58));
        }
        assert!(costs.len() >= 2000, "{} keys", costs.len());
        assert!(tracker.smoothness().is_some());
        costs.sort_by(f64::total_cmp);
        let pick = |q: f64| costs[((costs.len() - 1) as f64 * q).round() as usize];
        let edges = [0.05, 0.1, 0.25, 0.5, 1.0];
        let mut hist = [0usize; 6];
        for &c in &costs {
            let bucket = edges.iter().position(|&e| c < e).unwrap_or(edges.len());
            hist[bucket] += 1;
        }
        let (p50, p95, p99) = (pick(0.5), pick(0.95), pick(0.99));
        eprintln!(
            "flow keystroke latency [{}] {} keys over {} chars: p50 {p50:.3} ms, p95 {p95:.3} ms, \
             p99 {p99:.3} ms, max {:.3} ms; histogram <0.05:{} <0.1:{} <0.25:{} <0.5:{} <1:{} >=1:{}",
            if cfg!(debug_assertions) { "debug" } else { "release" },
            costs.len(),
            text.chars().count(),
            pick(1.0),
            hist[0],
            hist[1],
            hist[2],
            hist[3],
            hist[4],
            hist[5]
        );
        if !cfg!(debug_assertions) {
            assert!(p99 < 1.0, "Flow p99 {p99:.3} ms exceeds the 1 ms target");
        }
    }

    #[test]
    fn telemetry_and_report_data_are_recorded_in_ram() {
        let mut p = Practice::new(
            LayoutId::Qwerty,
            "the cat sat",
            TimerMode::Stopwatch,
            DisplayMode::Word,
        )
        .unwrap();
        let t0 = Instant::now();
        let end = type_keys(&mut p, "thx cat\u{8}t sat", t0);
        let v = p.view(end);
        assert_eq!(v.stats.words, 3);
        assert_eq!(v.stats.words_correct, 2, "\"thx\" is wrong");
        assert!(v.stats.kdph > 0.0);
        assert_eq!(p.key_log().len(), 13, "every applied input is logged");
        let diff = p.word_diff();
        assert_eq!(diff.len(), 3);
        assert_eq!(diff[0].expected, "the");
        assert_eq!(diff[0].typed, "thx");
        assert_eq!(
            diff[0].kind,
            crate::report::DiffKind::Half,
            "spelling is half"
        );
        let (full, half) = p.mistake_counts(end);
        assert_eq!((full, half), (0, 1));
        let pts = p.progress_points();
        assert_eq!(pts.len(), 13);
        assert!(pts.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(p.physical_keys_for("t"), ['t']);
    }

    #[test]
    fn paused_time_is_left_out_of_the_minute_curve() {
        let mut p = Practice::new(
            LayoutId::Qwerty,
            "abcdef",
            TimerMode::Stopwatch,
            DisplayMode::Key,
        )
        .unwrap();
        let t0 = Instant::now();
        p.press('a', false, t0);
        p.toggle_pause(at(t0, 1_000));
        p.toggle_pause(at(t0, 61_000));
        p.press('b', false, at(t0, 62_000));
        let pts = p.progress_points();
        assert_eq!(pts[1].0, 2.0, "60 s of pause excluded");
        assert!(p.pause_if_active(at(t0, 63_000)));
        assert!(!p.pause_if_active(at(t0, 64_000)), "already paused");
    }

    #[test]
    fn the_board_has_layout_glyphs_and_fingers() {
        let rows = board(LayoutId::RemingtonGail);
        assert_eq!(
            rows.iter().map(Vec::len).collect::<Vec<_>>(),
            [13, 13, 11, 10]
        );
        let d = rows.iter().flatten().find(|k| k.id == "d").unwrap();
        assert_eq!((d.legend.as_str(), d.glyph.as_str()), ("D", "क"));
        assert!(d.left);
        let q = board(LayoutId::Qwerty);
        let f = q.iter().flatten().find(|k| k.id == "f").unwrap();
        assert_eq!(f.finger, 3, "F is the left index finger");
    }

    #[test]
    fn the_coach_knows_the_pressed_key_and_a_lone_sign_has_keys() {
        let p = Practice::new(
            LayoutId::RemingtonGail,
            "ि का",
            TimerMode::Stopwatch,
            DisplayMode::Key,
        )
        .unwrap();
        let pressed = p.pressed_hint('j', false).unwrap();
        assert_eq!(pressed.key, "J");
        assert_eq!(
            p.keys_for("ि"),
            "F",
            "no more \"—\" for a lone pre-posed sign"
        );
        let hint = p.view(Instant::now()).hint.unwrap();
        assert_eq!(hint.key, "F");
    }

    #[test]
    fn f11_toggles_focus_mode_and_esc_stays_the_gail_cbi_dead_key() {
        // Focus mode hides the Rules panel that switches it on, so it needs
        // its own way out on every layout. F11 types nothing anywhere.
        for (control, alt) in [(false, false), (true, false), (false, true), (true, true)] {
            assert_eq!(
                classify_key("\u{f70e}", control, alt),
                KeyInput::ToggleFocus,
                "F11 ctrl={control} alt={alt}"
            );
        }
        assert_eq!(
            classify_key("\u{f70d}", false, false),
            KeyInput::Ignore,
            "F10"
        );
        // Esc is not the exit: on GAIL/CBI it arms the dead-key layer.
        assert_eq!(classify_key("\u{1b}", false, false), KeyInput::Escape);
        for layout in [LayoutId::RemingtonGail, LayoutId::RemingtonCbi] {
            let mut p = Practice::new(layout, "फ़", TimerMode::Stopwatch, DisplayMode::Key).unwrap();
            assert!(p.arm_dead_key(), "{layout:?}");
            assert!(p.dead_key_armed());
            assert!(p.cancel_dead_key());
        }
    }

    #[test]
    fn classify_maps_window_events_to_inputs() {
        assert_eq!(
            classify_key("a", false, false),
            KeyInput::Char {
                c: 'a',
                altgr: false
            }
        );
        assert_eq!(
            classify_key("A", false, false),
            KeyInput::Char {
                c: 'A',
                altgr: false
            }
        );
        assert_eq!(
            classify_key(" ", false, false),
            KeyInput::Char {
                c: ' ',
                altgr: false
            }
        );
        assert_eq!(classify_key("\u{8}", false, false), KeyInput::Backspace);
        assert_eq!(classify_key("\u{1b}", false, false), KeyInput::Escape);
        assert_eq!(classify_key("r", true, false), KeyInput::Restart);
        assert_eq!(classify_key("\u{12}", true, false), KeyInput::Restart);
        assert_eq!(classify_key("c", true, false), KeyInput::Ignore, "Ctrl+C");
        assert_eq!(
            classify_key("\u{10}", false, false),
            KeyInput::Ignore,
            "Shift alone"
        );
        assert_eq!(
            classify_key("\u{12}", false, true),
            KeyInput::Ignore,
            "Alt alone"
        );
        assert_eq!(
            classify_key("\u{f700}", false, false),
            KeyInput::Ignore,
            "arrow"
        );
        assert_eq!(classify_key("", false, false), KeyInput::Ignore);
        assert_eq!(
            classify_key("b", true, true),
            KeyInput::Char {
                c: 'b',
                altgr: true
            }
        );
        assert_eq!(
            classify_key("\u{2}", true, true),
            KeyInput::Char {
                c: 'b',
                altgr: true
            }
        );
        assert_eq!(
            classify_key("\u{a5}", false, false),
            KeyInput::Char {
                c: '¥',
                altgr: false
            }
        );
    }

    #[test]
    fn plain_alt_with_a_printable_key_is_altgr_on_a_us_board() {
        // Right Alt on a US Windows board reports Alt without Ctrl.
        assert_eq!(
            classify_key("b", false, true),
            KeyInput::Char {
                c: 'b',
                altgr: true
            }
        );
        assert_eq!(
            classify_key(",", false, true),
            KeyInput::Char {
                c: ',',
                altgr: true
            }
        );
        assert_eq!(
            classify_key("5", false, true),
            KeyInput::Ignore,
            "Alt+digit stays out (numpad Alt codes)"
        );
        assert_eq!(
            classify_key(" ", false, true),
            KeyInput::Ignore,
            "Alt+Space is the window menu"
        );

        // The guide's "AltGr + B" for ई types it through the window path.
        // Book-driven: CBI has no AltGr layer (book 03), so only GAIL
        // (GAIL-MAP-122) teaches it; on CBI Alt+b types nothing and the
        // guide teaches b + Shift+Z.
        let layout = LayoutId::RemingtonGail;
        let mut p = Practice::new(layout, "ई", TimerMode::Stopwatch, DisplayMode::Key).unwrap();
        let hint = p.view(Instant::now()).hint.expect("guide hint for ई");
        assert_eq!((hint.key.as_str(), hint.altgr), ("B", true));
        let KeyInput::Char { c, altgr } = classify_key("b", false, true) else {
            panic!("Alt+b must be a typing key");
        };
        assert!(matches!(
            p.press(c, altgr, Instant::now()),
            KeyEffect::Applied { .. }
        ));
        assert_eq!(p.typed(), "ई");
        assert_eq!(p.state(), SessionState::Complete);
        let mut p = Practice::new(
            LayoutId::RemingtonCbi,
            "ई",
            TimerMode::Stopwatch,
            DisplayMode::Key,
        )
        .unwrap();
        let hint = p.view(Instant::now()).hint.expect("guide hint for ई");
        assert_eq!((hint.key.as_str(), hint.altgr), ("B", false));
        assert_eq!(p.press(c, altgr, Instant::now()), KeyEffect::Unmapped);

        // QWERTY has no AltGr layer: Alt+letter counts nothing.
        let mut p = Practice::new(
            LayoutId::Qwerty,
            "b",
            TimerMode::Stopwatch,
            DisplayMode::Key,
        )
        .unwrap();
        let KeyInput::Char { c, altgr } = classify_key("b", false, true) else {
            panic!("Alt+b must reach the layout");
        };
        assert!(matches!(
            p.press(c, altgr, Instant::now()),
            KeyEffect::Unmapped
        ));
        assert_eq!(p.state(), SessionState::Idle);
    }

    #[test]
    fn qwerty_keys_drive_the_session_to_a_finished_result() {
        let mut p = Practice::new(
            LayoutId::Qwerty,
            "the cat",
            TimerMode::Stopwatch,
            DisplayMode::Key,
        )
        .unwrap();
        let t0 = Instant::now();
        assert_eq!(p.units().len(), 7);
        p.press('t', false, t0);
        p.press('x', false, at(t0, 200));
        let v = p.view(at(t0, 200));
        assert_eq!(v.stats.errors, 1);
        assert_eq!(v.states[1], UnitState::Wrong, "KEY mode colours at once");
        assert_eq!(v.cursor, 2);
        p.backspace(at(t0, 400));
        let end = type_keys(&mut p, "he cat", at(t0, 600));
        assert_eq!(p.state(), SessionState::Complete);
        let s = p.summary(end);
        assert_eq!(s.completion, Some(CompletionReason::Finished));
        assert_eq!(s.errors, 0, "the corrected error is gone");
        assert_eq!(s.accuracy, 100.0);
        assert_eq!(s.backspaces, 1);
        assert!(s.wpm > 0.0);
        assert_eq!(s.weak_keys.len(), 1, "but the miss is a weak key");
        assert_eq!(s.weak_keys[0].unit, "h");
        assert_eq!(s.weak_keys[0].keys, "H");
    }

    #[test]
    fn word_mode_waits_for_the_space() {
        let mut p = Practice::new(
            LayoutId::Qwerty,
            "ab cd",
            TimerMode::Stopwatch,
            DisplayMode::Word,
        )
        .unwrap();
        let t0 = Instant::now();
        type_keys(&mut p, "ax", t0);
        assert!(p.view(t0).states.iter().all(|s| *s == UnitState::Pending));
        type_keys(&mut p, " ", t0);
        let states = p.view(t0).states;
        assert_eq!(&states[..2], &[UnitState::Wrong, UnitState::Wrong]);
    }

    #[test]
    fn gail_and_cbi_type_aksharas_through_the_composer() {
        for layout in [LayoutId::RemingtonGail, LayoutId::RemingtonCbi] {
            let mut p =
                Practice::new(layout, "कार्य", TimerMode::Stopwatch, DisplayMode::Key).unwrap();
            assert_eq!(p.units(), ["का", "र्य"], "akshar units");
            let t0 = Instant::now();
            let v = p.view(t0);
            let hint = v.hint.expect("guide hint before the first key");
            assert_eq!(hint.key, "D", "{layout:?}: क is the D key");
            assert!(!hint.shift);
            let mut t = t0;
            for (i, c) in "dk;Z".chars().enumerate() {
                assert!(matches!(p.press(c, false, t), KeyEffect::Applied { .. }));
                t = at(t, 250);
                if i < 3 {
                    assert_eq!(
                        p.view(t).stats.errors,
                        0,
                        "{layout:?}: intermediate is pending"
                    );
                }
            }
            assert_eq!(p.state(), SessionState::Complete, "{layout:?}");
            assert_eq!(p.typed(), "कार्य");
            let s = p.summary(t);
            assert_eq!((s.errors, s.accuracy), (0, 100.0));
        }
    }

    #[test]
    fn kruti_dev_types_the_legacy_stream_per_char() {
        let legacy = to_legacy("भारत कार्य").legacy;
        let mut p = Practice::new(
            LayoutId::KrutiDev,
            &legacy,
            TimerMode::Stopwatch,
            DisplayMode::Word,
        )
        .unwrap();
        assert_eq!(p.units().len(), legacy.chars().count(), "char units");
        let t0 = Instant::now();
        let end = type_keys(&mut p, &legacy, t0);
        assert_eq!(p.state(), SessionState::Complete);
        assert_eq!(p.typed(), legacy);
        assert_eq!(p.summary(end).accuracy, 100.0);
    }

    #[test]
    fn a_countdown_ends_at_the_deadline_via_tick() {
        let limit = TimeLimit::from_preset(TimerPreset::Sec30);
        let mut p = Practice::new(
            LayoutId::Qwerty,
            "a long text that will not be finished",
            TimerMode::Countdown(limit),
            DisplayMode::Word,
        )
        .unwrap();
        let t0 = Instant::now();
        assert!(!p.tick(at(t0, 60_000)), "idle sessions never expire");
        p.press('a', false, t0);
        assert!(!p.tick(at(t0, 29_900)));
        assert_eq!(
            p.view(at(t0, 29_900))
                .stats
                .remaining_seconds
                .map(|r| (r * 10.0).round()),
            Some(1.0)
        );
        assert!(p.tick(at(t0, 30_000)), "time-up at exactly 30 s");
        assert!(!p.tick(at(t0, 31_000)), "only once");
        assert_eq!(p.state(), SessionState::Complete);
        let s = p.summary(at(t0, 45_000));
        assert_eq!(s.completion, Some(CompletionReason::TimeUp));
        assert_eq!(s.elapsed_seconds, 30.0, "completed at the deadline");
        assert!(matches!(
            p.press('b', false, at(t0, 31_000)),
            KeyEffect::Ignored
        ));
    }

    /// Regression (custom timer): a typed custom entry — plain minutes, a
    /// seconds value or MM:SS — is the countdown the session runs, ends at
    /// 0 and reports as its duration.
    #[test]
    fn custom_timer_entries_count_down_and_end_the_session() {
        for (entry, secs) in [("0:01", 1u64), ("45s", 45), ("2", 120), ("60", 3600)] {
            let mut studio = crate::exercise::TimerStudio::default();
            studio.set_custom(entry).unwrap();
            let timer =
                crate::exercise::timer_for(crate::exercise::PracticeMode::Test, studio.mode());
            assert_eq!(
                timer.limit().map(TimeLimit::seconds),
                Some(secs as u32),
                "{entry}"
            );
            let mut p =
                Practice::new(LayoutId::Qwerty, "a long text", timer, DisplayMode::Word).unwrap();
            let t0 = Instant::now();
            p.press('a', false, t0);
            let ms = secs * 1000;
            assert!(!p.tick(at(t0, ms - 100)), "{entry}: still running");
            assert!(p.tick(at(t0, ms)), "{entry}: ends at 0");
            let s = p.summary(at(t0, ms + 5000));
            assert_eq!(s.completion, Some(CompletionReason::TimeUp));
            assert_eq!(s.elapsed_seconds, secs as f64, "{entry}: result duration");
        }
    }

    #[test]
    fn pause_freezes_the_clock_and_restart_resets() {
        let mut p = Practice::new(
            LayoutId::Qwerty,
            "abc def",
            TimerMode::Stopwatch,
            DisplayMode::Key,
        )
        .unwrap();
        let t0 = Instant::now();
        assert_eq!(
            p.toggle_pause(t0),
            SessionState::Idle,
            "nothing to pause yet"
        );
        p.press('a', false, t0);
        assert_eq!(p.toggle_pause(at(t0, 1000)), SessionState::Paused);
        assert!(matches!(
            p.press('b', false, at(t0, 2000)),
            KeyEffect::Ignored
        ));
        assert_eq!(p.toggle_pause(at(t0, 5000)), SessionState::Active);
        assert_eq!(p.view(at(t0, 6000)).stats.elapsed_seconds, 2.0);
        p.restart();
        assert_eq!(p.state(), SessionState::Idle);
        assert_eq!(p.typed(), "");
    }

    #[test]
    fn the_record_carries_metrics_and_aggregated_errors() {
        let mut p = Practice::new(
            LayoutId::Qwerty,
            "ab cd",
            TimerMode::Stopwatch,
            DisplayMode::Word,
        )
        .unwrap();
        let t0 = Instant::now();
        let end = type_keys(&mut p, "ax cx", t0);
        p.complete(end);
        let r = p.record(1, 7, ProductId::EnglishClassic, end);
        assert_eq!((r.profile_id, r.exercise_id), (1, 7));
        assert_eq!(r.display_mode, "word");
        assert_eq!(r.guide_mode, "off");
        assert!(!r.finished, "submitted, not finished");
        assert_eq!(r.metrics.wrong_chars, 2);
        assert_eq!(r.metrics.correct_chars, 3);
        assert_eq!(
            r.errors,
            [
                ErrorAggregate {
                    physical_key: "B".into(),
                    expected_char: "b".into(),
                    kind: ErrorKind::Wrong,
                    occurrences: 1
                },
                ErrorAggregate {
                    physical_key: "D".into(),
                    expected_char: "d".into(),
                    kind: ErrorKind::Wrong,
                    occurrences: 1
                },
            ]
        );
        assert_eq!(r.started_at.len(), 20);
        assert!(r.started_at.ends_with('Z'));
        assert_eq!(r.day.len(), 10);
    }

    #[test]
    fn a_typed_session_is_saved_once_with_its_figures() {
        use radha_content::import::{ContentBundle, ImportedExercise, BUNDLE_FORMAT};
        use radha_content::{ExerciseRepository, ProgressRepository};

        // The app's shape: one write connection, one read connection.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radha.sqlite");
        let mut db = radha_db::Database::open(&path).unwrap();
        radha_db::import_bundle(
            &mut db,
            &ContentBundle {
                format: BUNDLE_FORMAT,
                product: "hindi-mastery-cbi".into(),
                source: "test".into(),
                provisional: true,
                exercises: vec![ImportedExercise {
                    number: 1,
                    title: "Placeholder".into(),
                    category: None,
                    difficulty: None,
                    content: "कार्य".into(),
                    source_id: String::new(),
                    ..ImportedExercise::default()
                }],
            },
        )
        .unwrap();
        let profile = db.ensure_default_profile().unwrap();
        let reader = radha_db::SqliteRepository::new(radha_db::Database::open(&path).unwrap());
        let cbi = ProductId::HindiMastery(LayoutId::RemingtonCbi);
        let id = reader.list_for_product(cbi).unwrap()[0].id;
        let exercise = reader.load(id).unwrap();

        let mut p = Practice::new(
            LayoutId::RemingtonCbi,
            &exercise.content,
            TimerMode::Stopwatch,
            DisplayMode::Word,
        )
        .unwrap();
        // A wrong key first (x), corrected, then the real sequence.
        let end = type_keys(&mut p, "x\u{8}dk;Z", Instant::now());
        assert_eq!(p.state(), SessionState::Complete);
        assert_eq!(
            reader.exercise_best(profile, id).unwrap(),
            None,
            "typing wrote nothing"
        );

        let record = p.record(profile, id, cbi, end);
        assert!(record.finished);
        assert_eq!(record.metrics.backspaces, 1);
        assert_eq!(record.guide_mode, "minimal");
        let saved = db.save_session(&record).unwrap();
        assert!(saved.new_exercise_best);
        let best = reader
            .exercise_best(profile, id)
            .unwrap()
            .expect("best after save");
        assert_eq!(best.accuracy, 100.0);
        let progress = reader.exercise_progress(profile, id).unwrap().unwrap();
        assert_eq!((progress.attempts, progress.completed), (1, true));
    }

    #[test]
    fn iso_dates_are_correct() {
        assert_eq!(iso_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let t = UNIX_EPOCH + Duration::from_secs(1_761_386_096);
        assert_eq!(iso_utc(t), "2025-10-25T09:54:56Z");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(iso_utc(leap), "2000-02-29T00:00:00Z");
        assert_eq!(clock(75.9), "1:15");
        assert_eq!(clock(-3.0), "0:00");
    }
}
