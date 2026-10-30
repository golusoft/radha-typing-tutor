//! Typing screen v2 rules (PRD 17.5 TY-01..TY-08, DESIGN 20.7), as pure
//! functions the Typing screen applies.
//!
//! Everything here is RAM-only and O(1) per key: the rules sidebar settings,
//! the Hindi Guide display modes, the live telemetry (streak, words, KDPH)
//! and the timer ring geometry.

/// Hindi Guide mode (TY-08, PRD 17.7 HL-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuideMode {
    /// Keyboard highlight, next key and finger, always.
    #[default]
    Full,
    /// Like Full, but only after a hesitation or a mistake.
    Smart,
    /// Keyboard highlight and next key only; no finger coaching.
    Minimal,
    /// Nothing.
    Off,
}

/// How long a typist may hesitate before Smart mode shows the guide.
pub const SMART_HESITATION_MS: u64 = 1200;

impl GuideMode {
    /// UI / settings id.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            GuideMode::Full => "full",
            GuideMode::Smart => "smart",
            GuideMode::Minimal => "minimal",
            GuideMode::Off => "off",
        }
    }

    /// Parse [`GuideMode::slug`].
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [
            GuideMode::Full,
            GuideMode::Smart,
            GuideMode::Minimal,
            GuideMode::Off,
        ]
        .into_iter()
        .find(|m| m.slug() == slug)
    }

    /// One step less help (the guide ramp steps down).
    #[must_use]
    pub fn less(self) -> Self {
        match self {
            GuideMode::Full => GuideMode::Smart,
            GuideMode::Smart => GuideMode::Minimal,
            GuideMode::Minimal | GuideMode::Off => GuideMode::Off,
        }
    }

    /// One step more help (the guide ramp steps back up).
    #[must_use]
    pub fn more(self) -> Self {
        match self {
            GuideMode::Off => GuideMode::Minimal,
            GuideMode::Minimal => GuideMode::Smart,
            GuideMode::Smart | GuideMode::Full => GuideMode::Full,
        }
    }
}

/// What the guide shows right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuideShow {
    /// Highlight the next key on the virtual keyboard.
    pub key_highlight: bool,
    /// Name the next key in the coach strip.
    pub next_key: bool,
    /// Name the hand and finger.
    pub finger: bool,
}

/// The guide for `mode`, `idle_ms` after the last key, `recent_error` when
/// the last key was a mistake.
#[must_use]
pub fn guide_show(mode: GuideMode, idle_ms: u64, recent_error: bool) -> GuideShow {
    let all = GuideShow {
        key_highlight: true,
        next_key: true,
        finger: true,
    };
    match mode {
        GuideMode::Full => all,
        GuideMode::Smart if recent_error || idle_ms >= SMART_HESITATION_MS => all,
        GuideMode::Smart | GuideMode::Off => GuideShow::default(),
        GuideMode::Minimal => GuideShow {
            key_highlight: true,
            next_key: true,
            finger: false,
        },
    }
}

/// Backspace rule (TY-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackspacePolicy {
    /// Backspace anywhere.
    #[default]
    Full,
    /// Only inside the word being typed (not back past a space).
    OneWord,
    /// Backspace is ignored.
    Off,
}

impl BackspacePolicy {
    /// UI id.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            BackspacePolicy::Full => "full",
            BackspacePolicy::OneWord => "word",
            BackspacePolicy::Off => "off",
        }
    }

    /// Parse [`BackspacePolicy::slug`].
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [
            BackspacePolicy::Full,
            BackspacePolicy::OneWord,
            BackspacePolicy::Off,
        ]
        .into_iter()
        .find(|p| p.slug() == slug)
    }

    /// Whether a Backspace may delete from `typed` (what is typed so far;
    /// `preedit` when a pre-posed sign is pending, which Backspace cancels).
    #[must_use]
    pub fn allows(self, typed: &str, preedit: bool) -> bool {
        match self {
            BackspacePolicy::Full => true,
            BackspacePolicy::Off => false,
            BackspacePolicy::OneWord => {
                preedit || (!typed.is_empty() && !typed.ends_with(char::is_whitespace))
            }
        }
    }
}

/// Highlight rule (TY-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HighlightMode {
    /// Active word plus correct / wrong colouring.
    #[default]
    WordError,
    /// Active word only; no correctness colours.
    WordOnly,
    /// Blind: only the cursor.
    Blind,
}

impl HighlightMode {
    /// UI id.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            HighlightMode::WordError => "word-error",
            HighlightMode::WordOnly => "word",
            HighlightMode::Blind => "blind",
        }
    }

    /// Parse [`HighlightMode::slug`].
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [
            HighlightMode::WordError,
            HighlightMode::WordOnly,
            HighlightMode::Blind,
        ]
        .into_iter()
        .find(|h| h.slug() == slug)
    }
}

/// Passage font size range (DESIGN 20.7).
pub const FONT_MIN: u32 = 15;
/// Largest passage font size.
pub const FONT_MAX: u32 = 34;
/// Devanagari minimum, for matra legibility.
pub const FONT_MIN_HINDI: u32 = 18;
/// Default passage font size.
pub const FONT_DEFAULT: u32 = 22;
/// Word limit range (0 = off).
pub const WORD_LIMIT: (u32, u32) = (50, 1500);
/// Keystroke limit range (0 = off).
pub const KEY_LIMIT: (u32, u32) = (200, 50_000);

/// The rules sidebar (TY-05). Editable in practice, locked in exam
/// (RULES 38).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypingRules {
    pub backspace: BackspacePolicy,
    pub highlight: HighlightMode,
    pub font_px: u32,
    pub bold: bool,
    pub paper: bool,
    /// 0 = off.
    pub word_limit: u32,
    /// 0 = off.
    pub key_limit: u32,
    pub sound: bool,
    pub focus_mode: bool,
    /// Passage scrollbar.
    pub show_scrollbar: bool,
    /// The passage follows the cursor.
    pub auto_scroll: bool,
    /// Key Guide / mistake-key marking (off: the highlighter is word-only).
    pub key_guide: bool,
    /// Exam: nothing may be changed.
    pub locked: bool,
}

impl Default for TypingRules {
    fn default() -> Self {
        TypingRules {
            backspace: BackspacePolicy::Full,
            highlight: HighlightMode::WordError,
            font_px: FONT_DEFAULT,
            bold: false,
            paper: false,
            word_limit: 0,
            key_limit: 0,
            sound: true,
            focus_mode: false,
            show_scrollbar: true,
            auto_scroll: true,
            key_guide: false,
            locked: false,
        }
    }
}

impl TypingRules {
    /// Step the font size by `delta` px within the script's range.
    pub fn step_font(&mut self, delta: i32, devanagari: bool) {
        let min = if devanagari { FONT_MIN_HINDI } else { FONT_MIN };
        let next = i64::from(self.font_px) + i64::from(delta);
        self.font_px =
            u32::try_from(next.clamp(i64::from(min), i64::from(FONT_MAX))).unwrap_or(FONT_DEFAULT);
    }

    /// Clamp the font to the script's minimum (a Hindi passage never shows
    /// below 18 px).
    pub fn fit_script(&mut self, devanagari: bool) {
        self.step_font(0, devanagari);
    }

    /// Step a limit: off -> min -> ... -> max -> off, `step` at a time.
    #[must_use]
    pub fn step_limit(current: u32, range: (u32, u32), step: u32, up: bool) -> u32 {
        let (min, max) = range;
        match (current, up) {
            (0, true) => min,
            (0, false) => 0,
            (c, true) => (c + step).min(max),
            (c, false) if c <= min => 0,
            (c, false) => c.saturating_sub(step).max(min),
        }
    }

    /// Whether a word or keystroke limit has been reached.
    #[must_use]
    pub fn limit_reached(&self, words_committed: usize, presses: u32) -> bool {
        (self.word_limit > 0 && words_committed >= self.word_limit as usize)
            || (self.key_limit > 0 && presses >= self.key_limit)
    }
}

/// Live streak tracking (TY-01): consecutive correct units, updated per key
/// from the cursor and error count only (O(1)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Streak {
    pub current: u32,
    pub peak: u32,
    cursor: usize,
    errors: usize,
}

impl Streak {
    /// Apply the session's new cursor and error count. Returns `true` when
    /// this key added an error.
    pub fn update(&mut self, cursor: usize, errors: usize) -> bool {
        let new_error = errors > self.errors;
        if new_error {
            self.current = 0;
        } else if cursor > self.cursor {
            let gained = u32::try_from(cursor - self.cursor).unwrap_or(u32::MAX);
            self.current = self.current.saturating_add(gained);
        }
        self.peak = self.peak.max(self.current);
        self.cursor = cursor;
        self.errors = errors;
        new_error
    }
}

/// SVG commands of the timer ring's arc for `fraction` (0..1) of a circle,
/// in a `0 0 36 36` viewbox, starting at 12 o'clock and running clockwise.
#[must_use]
pub fn ring_arc(fraction: f32) -> String {
    let f = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if f <= 0.0 {
        return String::new();
    }
    let (cx, cy, r) = (18.0_f32, 18.0_f32, 15.0_f32);
    if f >= 0.9999 {
        // Two half arcs: a single arc cannot close on itself.
        return format!(
            "M {cx} {} A {r} {r} 0 1 1 {cx} {} A {r} {r} 0 1 1 {cx} {}",
            cy - r,
            cy + r,
            cy - r
        );
    }
    let angle = f * std::f32::consts::TAU;
    let x = cx + r * angle.sin();
    let y = cy - r * angle.cos();
    let large = i32::from(f > 0.5);
    format!("M {cx} {} A {r} {r} 0 {large} 1 {x:.2} {y:.2}", cy - r)
}

/// The guide actually in force: nothing unless the Key Guide is on (and
/// never in an exam); with it on, the chosen mode, Full when that was Off.
#[must_use]
pub fn effective_guide(chosen: GuideMode, key_guide: bool, exam: bool) -> GuideMode {
    match (key_guide && !exam, chosen) {
        (false, _) => GuideMode::Off,
        (true, GuideMode::Off) => GuideMode::Full,
        (true, m) => m,
    }
}

impl BackspacePolicy {
    /// Rule chip text.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            BackspacePolicy::Full => "Full Backspace",
            BackspacePolicy::OneWord => "One Word Backspace",
            BackspacePolicy::Off => "Backspace Disabled",
        }
    }
}

impl HighlightMode {
    /// Rule chip text.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            HighlightMode::WordError => "Word + Error Highlight",
            HighlightMode::WordOnly => "Word Highlight Only",
            HighlightMode::Blind => "No Highlight (Blind)",
        }
    }
}

/// Parse the word-limit field: 50–1500 words.
///
/// # Errors
///
/// A message for the field when it is not a number in range.
pub fn parse_word_limit(text: &str) -> Result<u32, &'static str> {
    let (lo, hi) = WORD_LIMIT;
    match text.trim().parse::<u32>() {
        Ok(n) if (lo..=hi).contains(&n) => Ok(n),
        _ => Err("Enter 50 to 1500 words"),
    }
}

/// "Right Index + Left Shift" for the coach strip.
#[must_use]
pub fn finger_sentence(hand: &str, finger: &str, shift: bool, altgr: bool) -> String {
    let mut s = format!("{hand} {finger}");
    let other = if hand == "Left" { "Right" } else { "Left" };
    if shift {
        s.push_str(&format!(" + {other} Shift"));
    }
    if altgr {
        s.push_str(" + Right Alt (AltGr)");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guide_modes_show_what_they_promise() {
        let all = guide_show(GuideMode::Full, 0, false);
        assert!(all.key_highlight && all.next_key && all.finger);
        assert_eq!(
            guide_show(GuideMode::Off, 99_999, true),
            GuideShow::default()
        );
        let min = guide_show(GuideMode::Minimal, 0, true);
        assert!(min.key_highlight && min.next_key && !min.finger);
        // Smart waits for a hesitation or a mistake.
        assert_eq!(
            guide_show(GuideMode::Smart, 300, false),
            GuideShow::default()
        );
        assert_eq!(
            guide_show(GuideMode::Smart, SMART_HESITATION_MS, false),
            all
        );
        assert_eq!(guide_show(GuideMode::Smart, 0, true), all);
        for m in ["full", "smart", "minimal", "off"] {
            assert_eq!(GuideMode::from_slug(m).map(GuideMode::slug), Some(m));
        }
        assert_eq!(GuideMode::from_slug("x"), None);
        assert_eq!(GuideMode::Full.less().less().less().less(), GuideMode::Off);
        assert_eq!(GuideMode::Off.more().more().more().more(), GuideMode::Full);
    }

    #[test]
    fn backspace_policies() {
        assert!(BackspacePolicy::Full.allows("abc ", false));
        assert!(!BackspacePolicy::Off.allows("abc", false));
        let one = BackspacePolicy::OneWord;
        assert!(one.allows("the ca", false), "inside the word");
        assert!(!one.allows("the ", false), "not back past a space");
        assert!(!one.allows("", false));
        assert!(one.allows("the ", true), "a pending sign may be cancelled");
        for p in ["full", "word", "off"] {
            assert_eq!(
                BackspacePolicy::from_slug(p).map(BackspacePolicy::slug),
                Some(p)
            );
        }
        for h in ["word-error", "word", "blind"] {
            assert_eq!(
                HighlightMode::from_slug(h).map(HighlightMode::slug),
                Some(h)
            );
        }
    }

    #[test]
    fn font_and_limits_stay_in_range() {
        let mut r = TypingRules {
            font_px: 16,
            ..Default::default()
        };
        r.fit_script(true);
        assert_eq!(r.font_px, FONT_MIN_HINDI, "Hindi never below 18 px");
        r.step_font(-10, false);
        assert_eq!(r.font_px, FONT_MIN);
        r.step_font(100, false);
        assert_eq!(r.font_px, FONT_MAX);

        let step = TypingRules::step_limit;
        assert_eq!(step(0, WORD_LIMIT, 50, true), 50);
        assert_eq!(step(50, WORD_LIMIT, 50, false), 0, "below min is off");
        assert_eq!(step(1480, WORD_LIMIT, 50, true), 1500);
        assert_eq!(step(0, KEY_LIMIT, 200, false), 0);
        r.word_limit = 50;
        assert!(!r.limit_reached(49, 10_000));
        assert!(r.limit_reached(50, 0));
        r.word_limit = 0;
        r.key_limit = 200;
        assert!(r.limit_reached(0, 200));
    }

    #[test]
    fn streak_counts_correct_units_and_resets_on_an_error() {
        let mut s = Streak::default();
        assert!(!s.update(3, 0));
        assert_eq!(s.current, 3);
        assert!(s.update(4, 1), "a new error");
        assert_eq!((s.current, s.peak), (0, 3));
        assert!(!s.update(9, 1));
        assert_eq!((s.current, s.peak), (5, 5));
        assert!(!s.update(8, 1), "backspace keeps the streak");
        assert_eq!(s.current, 5);
    }

    #[test]
    fn ring_arc_geometry() {
        assert_eq!(ring_arc(0.0), "");
        assert_eq!(ring_arc(f32::NAN), "");
        let quarter = ring_arc(0.25);
        assert!(
            quarter.starts_with("M 18 3 A 15 15 0 0 1 33.00 18.00"),
            "{quarter}"
        );
        assert!(ring_arc(0.75).contains(" 0 1 1 "), "large arc flag");
        assert_eq!(ring_arc(1.0).matches(" A ").count(), 2);
    }

    #[test]
    fn the_key_guide_is_off_by_default_and_never_in_exams() {
        assert!(!TypingRules::default().key_guide);
        assert_eq!(
            effective_guide(GuideMode::Full, false, false),
            GuideMode::Off
        );
        assert_eq!(
            effective_guide(GuideMode::Off, true, false),
            GuideMode::Full
        );
        assert_eq!(
            effective_guide(GuideMode::Smart, true, false),
            GuideMode::Smart
        );
        assert_eq!(effective_guide(GuideMode::Full, true, true), GuideMode::Off);
        assert_eq!(parse_word_limit("400"), Ok(400));
        assert_eq!(parse_word_limit(" 50 "), Ok(50));
        assert_eq!(parse_word_limit("1500"), Ok(1500));
        for bad in ["49", "1501", "", "4OO", "-50"] {
            assert!(parse_word_limit(bad).is_err(), "{bad:?}");
        }
        assert_eq!(BackspacePolicy::OneWord.label(), "One Word Backspace");
        assert_eq!(HighlightMode::Blind.label(), "No Highlight (Blind)");
    }

    #[test]
    fn finger_sentences_name_the_shift_hand() {
        assert_eq!(
            finger_sentence("Right", "Little", true, false),
            "Right Little + Left Shift"
        );
        assert_eq!(
            finger_sentence("Left", "Index", false, true),
            "Left Index + Right Alt (AltGr)"
        );
    }
}
