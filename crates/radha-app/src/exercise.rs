//! Exercise screen v2 rules (PRD 17.2 / 17.4, DESIGN 20.6).
//!
//! Everything the Exercise screen decides lives here as plain, tested
//! functions over plain data (`RULES.MD` Rule 16): which mode toggle a
//! product shows (the mode matrix), which cards a search and filters keep,
//! which slice of the grid is on screen (virtualisation, `RULES.MD` Rule 34),
//! the timer studio and its MM:SS validation (`RULES.MD` Rule 30), and the
//! inspector's derived figures. `ui/flow.rs` only applies the results to the
//! Slint models.

use std::collections::HashMap;
use std::ops::Range;

use radha_content::{ContentStatus, ExamPool, ProductId};
use radha_core::{TimeLimit, TimerMode, TimerPreset};
use radha_layouts::{load_layout, Finger, Guide, Hand, Layer, LayoutId, PlanStep};

use crate::catalog::matches_query;

// --- Mode matrix ---------------------------------------------------------------

/// How a session is run, as chosen on the Exercise screen
/// (`typing_sessions.mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PracticeMode {
    /// Mastery LEARN: no timer, guide on, never a verdict.
    Learn,
    /// Mastery TEST: timer studio, countdown, target comparison.
    Test,
    /// Typing Flow and Number + Punctuation: always user-timed, no LEARN.
    Flow,
    /// Hindi Learn, beginner track.
    Beginner,
    /// Hindi Learn, classic track.
    Classic,
    /// Exam practice: timer and rules fixed by the profile.
    Exam,
}

/// Who sets the timer in a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerPolicy {
    /// No timer at all (LEARN, Hindi Learn).
    None,
    /// The timer studio (TEST, Flow, Number + Punctuation).
    UserSet,
    /// Fixed by the exam profile; the user cannot change it.
    Locked,
}

/// What the Exercise screen's right-hand dock shows. Never two at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dock {
    /// Guide mode and keyboard visibility (LEARN, BEGINNER, CLASSIC).
    Guide,
    /// The timer studio.
    TimerStudio,
    /// A read-only, locked timer (exam).
    LockedTimer,
}

impl PracticeMode {
    /// `typing_sessions.mode` value; also the id the UI toggle sends.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            PracticeMode::Learn => "learn",
            PracticeMode::Test => "test",
            PracticeMode::Flow => "flow",
            PracticeMode::Beginner => "beginner",
            PracticeMode::Classic => "classic",
            PracticeMode::Exam => "exam",
        }
    }

    /// Parse [`PracticeMode::slug`].
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [
            PracticeMode::Learn,
            PracticeMode::Test,
            PracticeMode::Flow,
            PracticeMode::Beginner,
            PracticeMode::Classic,
            PracticeMode::Exam,
        ]
        .into_iter()
        .find(|m| m.slug() == slug)
    }

    /// Toggle / breadcrumb label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PracticeMode::Learn => "LEARN",
            PracticeMode::Test => "TEST",
            PracticeMode::Flow => "PRACTICE",
            PracticeMode::Beginner => "BEGINNER",
            PracticeMode::Classic => "CLASSIC",
            PracticeMode::Exam => "EXAM PRACTICE",
        }
    }

    /// Who sets the timer.
    #[must_use]
    pub fn timer_policy(self) -> TimerPolicy {
        match self {
            PracticeMode::Learn | PracticeMode::Beginner | PracticeMode::Classic => {
                TimerPolicy::None
            }
            PracticeMode::Test | PracticeMode::Flow => TimerPolicy::UserSet,
            PracticeMode::Exam => TimerPolicy::Locked,
        }
    }

    /// The dock for this mode.
    #[must_use]
    pub fn dock(self) -> Dock {
        match self.timer_policy() {
            TimerPolicy::None => Dock::Guide,
            TimerPolicy::UserSet => Dock::TimerStudio,
            TimerPolicy::Locked => Dock::LockedTimer,
        }
    }

    /// The Hindi Learn track this mode lists (`exercises.track`).
    #[must_use]
    pub fn track(self) -> Option<&'static str> {
        match self {
            PracticeMode::Beginner => Some("beginner"),
            PracticeMode::Classic => Some("classic"),
            _ => None,
        }
    }
}

/// A product's row of the PRD 17.2 matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeMatrix {
    /// The two header toggle options, or `None` when the header has no toggle.
    pub toggle: Option<[PracticeMode; 2]>,
    /// The mode a product opens in.
    pub default: PracticeMode,
}

impl ModeMatrix {
    /// Whether `mode` is allowed for this product.
    #[must_use]
    pub fn allows(self, mode: PracticeMode) -> bool {
        match self.toggle {
            Some(options) => options.contains(&mode),
            None => mode == self.default,
        }
    }
}

/// PRD 17.2 for one product.
#[must_use]
pub fn mode_matrix(product: ProductId) -> ModeMatrix {
    match product {
        ProductId::EnglishClassic | ProductId::HindiMastery(_) => ModeMatrix {
            toggle: Some([PracticeMode::Learn, PracticeMode::Test]),
            // The reference opens on the timer studio.
            default: PracticeMode::Test,
        },
        ProductId::TypingFlow | ProductId::NumberPunctuation => ModeMatrix {
            toggle: None,
            default: PracticeMode::Flow,
        },
        ProductId::HindiLearn(_) => ModeMatrix {
            toggle: Some([PracticeMode::Beginner, PracticeMode::Classic]),
            default: PracticeMode::Beginner,
        },
    }
}

/// The exam row of PRD 17.2: the header toggle is the language, the mode is
/// always [`PracticeMode::Exam`].
#[cfg_attr(not(test), allow(dead_code))]
pub const EXAM_MATRIX: ModeMatrix = ModeMatrix {
    toggle: None,
    default: PracticeMode::Exam,
};

/// Language of an exam test (the exam header toggle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExamLanguage {
    English,
    Hindi,
}

impl ExamLanguage {
    /// Parse the launcher / toggle id (`"eng"` / `"hin"`).
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "eng" | "english" => Some(ExamLanguage::English),
            "hin" | "hindi" => Some(ExamLanguage::Hindi),
            _ => None,
        }
    }

    /// Toggle id.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            ExamLanguage::English => "eng",
            ExamLanguage::Hindi => "hin",
        }
    }

    /// The shared passage pool (`RULES.MD` Rule 33).
    #[must_use]
    pub fn pool(self) -> ExamPool {
        match self {
            ExamLanguage::English => ExamPool::English,
            ExamLanguage::Hindi => ExamPool::Hindi,
        }
    }

    /// The language a product typing layout implies.
    #[must_use]
    pub fn of_layout(layout: LayoutId) -> Self {
        if layout == LayoutId::Qwerty {
            ExamLanguage::English
        } else {
            ExamLanguage::Hindi
        }
    }
}

/// The product whose layout an exam test is typed on. Hindi defaults to
/// Kruti Dev when the card names it, otherwise Remington GAIL; the Exercise
/// screen lets the user switch.
///
/// A Hindi-only (All-in-One Mangal) exam is always typed on a Mangal
/// Unicode layout; see [`hindi_only_carrier`].
#[must_use]
pub fn exam_carrier(language: ExamLanguage, font_layout: &str) -> ProductId {
    match language {
        ExamLanguage::English => ProductId::EnglishClassic,
        ExamLanguage::Hindi if font_layout.to_lowercase().contains("kruti") => {
            ProductId::HindiMastery(LayoutId::KrutiDev)
        }
        ExamLanguage::Hindi => ProductId::HindiMastery(LayoutId::RemingtonGail),
    }
}

/// The Mangal Unicode layouts of the All-in-One Mangal exam card, in the
/// order the user named them: InScript, Remington GAIL, Remington CBI. No
/// QWERTY (no English) and no Kruti Dev (a legacy font, not Mangal).
pub const MANGAL_LAYOUTS: [LayoutId; 3] = [
    LayoutId::Inscript,
    LayoutId::RemingtonGail,
    LayoutId::RemingtonCbi,
];

/// Whether an exam may be typed on `layout`: any layout for an ordinary
/// card, only [`MANGAL_LAYOUTS`] for a Hindi-only card.
#[must_use]
pub fn exam_layout_allowed(hindi_only: bool, layout: LayoutId) -> bool {
    !hindi_only || MANGAL_LAYOUTS.contains(&layout)
}

/// A Hindi-only exam's carrier: `preferred` when it is a Mangal layout,
/// else Remington GAIL.
#[must_use]
pub fn hindi_only_carrier(preferred: LayoutId) -> ProductId {
    if MANGAL_LAYOUTS.contains(&preferred) {
        ProductId::HindiMastery(preferred)
    } else {
        ProductId::HindiMastery(LayoutId::RemingtonGail)
    }
}

/// Practice timer for an exam whose profile gives no duration
/// (`exam_mode::timer`). Labelled as a practice default on screen, never as
/// the exam's duration.
pub const EXAM_PRACTICE_SECONDS: u32 = 600;

/// The timer a mode starts with, given the studio's choice.
#[must_use]
pub fn timer_for(mode: PracticeMode, studio: TimerMode) -> TimerMode {
    match mode.timer_policy() {
        TimerPolicy::None => TimerMode::Off,
        TimerPolicy::UserSet => studio,
        TimerPolicy::Locked => TimeLimit::custom(EXAM_PRACTICE_SECONDS)
            .map_or(TimerMode::Stopwatch, TimerMode::Countdown),
    }
}

// --- Cards and filters -----------------------------------------------------------

/// One grid card's data: list metadata plus the profile's progress. Never
/// the exercise text.
#[derive(Debug, Clone, PartialEq)]
pub struct CardMeta {
    pub id: i64,
    pub number: u32,
    pub title: String,
    pub category: Option<String>,
    pub difficulty: Option<u8>,
    pub status: ContentStatus,
    pub track: Option<String>,
    pub word_count: Option<u32>,
    /// Best (net WPM, accuracy).
    pub best: Option<(f64, f64)>,
    pub completed: bool,
    /// Typing Flow only: (last, best) smoothness %, `None` before a
    /// scored rhythm.
    pub flow: Option<(f64, f64)>,
}

/// Difficulty filter (EX-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DifficultyBand {
    #[default]
    All,
    /// Levels 1–2.
    Easy,
    /// Level 3.
    Medium,
    /// Levels 4–5.
    Hard,
}

impl DifficultyBand {
    /// UI id.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            DifficultyBand::All => "all",
            DifficultyBand::Easy => "easy",
            DifficultyBand::Medium => "medium",
            DifficultyBand::Hard => "hard",
        }
    }

    /// Parse [`DifficultyBand::slug`].
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [
            DifficultyBand::All,
            DifficultyBand::Easy,
            DifficultyBand::Medium,
            DifficultyBand::Hard,
        ]
        .into_iter()
        .find(|b| b.slug() == slug)
    }

    /// Whether a card of `difficulty` is in the band. Unrated cards appear
    /// only under All.
    #[must_use]
    pub fn contains(self, difficulty: Option<u8>) -> bool {
        match (self, difficulty) {
            (DifficultyBand::All, _) => true,
            (_, None) => false,
            (DifficultyBand::Easy, Some(d)) => d <= 2,
            (DifficultyBand::Medium, Some(d)) => d == 3,
            (DifficultyBand::Hard, Some(d)) => d >= 4,
        }
    }
}

/// The Exercise screen's filters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListFilter {
    pub query: String,
    pub difficulty: DifficultyBand,
    /// `None` = every category.
    pub category: Option<String>,
    /// Hindi Learn track; `None` lists every card.
    pub track: Option<&'static str>,
}

/// `EX-012`.
#[must_use]
pub fn card_code(number: u32) -> String {
    format!("EX-{number:03}")
}

fn in_track(card: &CardMeta, track: Option<&str>) -> bool {
    track.is_none_or(|t| card.track.as_deref() == Some(t))
}

/// Indices of the cards `filter` keeps, in list order. Search matches the
/// title, category and number (`12`, `EX-012`) with the Home rule.
#[must_use]
pub fn filter_cards(cards: &[CardMeta], filter: &ListFilter) -> Vec<usize> {
    cards
        .iter()
        .enumerate()
        .filter(|(_, c)| in_track(c, filter.track))
        .filter(|(_, c)| filter.difficulty.contains(c.difficulty))
        .filter(|(_, c)| {
            filter
                .category
                .as_deref()
                .is_none_or(|want| c.category.as_deref() == Some(want))
        })
        .filter(|(_, c)| {
            let number = c.number.to_string();
            let code = card_code(c.number);
            matches_query(
                &filter.query,
                &[
                    c.title.as_str(),
                    c.category.as_deref().unwrap_or(""),
                    number.as_str(),
                    code.as_str(),
                ],
            )
        })
        .map(|(i, _)| i)
        .collect()
}

/// Categories of the cards in `track`, in first-seen order, with counts.
#[must_use]
pub fn category_counts(cards: &[CardMeta], track: Option<&str>) -> Vec<(String, usize)> {
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for c in cards.iter().filter(|c| in_track(c, track)) {
        let Some(cat) = &c.category else { continue };
        let n = counts.entry(cat.clone()).or_insert(0);
        if *n == 0 {
            order.push(cat.clone());
        }
        *n += 1;
    }
    order
        .into_iter()
        .map(|c| {
            let n = counts.get(&c).copied().unwrap_or(0);
            (c, n)
        })
        .collect()
}

/// Label and tone token of a review status (shown honestly, `RULES.MD`
/// Rule 32): `(text, "success" | "warning" | "secondary")`.
#[must_use]
pub fn status_label(status: ContentStatus) -> (&'static str, &'static str) {
    match status {
        ContentStatus::Verified => ("Verified", "success"),
        ContentStatus::Provisional => ("Provisional", "warning"),
        ContentStatus::GeneratedReviewPending => ("Generated · review pending", "secondary"),
    }
}

/// One line describing the review status of a whole list, e.g.
/// "200 verified" or "200 verified · 300 generated (review pending)".
#[must_use]
pub fn status_summary(cards: &[CardMeta]) -> String {
    let count = |s| cards.iter().filter(|c| c.status == s).count();
    let parts: Vec<String> = [
        (ContentStatus::Verified, "verified"),
        (ContentStatus::Provisional, "provisional"),
        (
            ContentStatus::GeneratedReviewPending,
            "generated (review pending)",
        ),
    ]
    .into_iter()
    .filter_map(|(s, label)| match count(s) {
        0 => None,
        n => Some(format!("{n} {label}")),
    })
    .collect();
    parts.join(" · ")
}

// --- Grid virtualisation ---------------------------------------------------------

/// Rows built beyond each edge of the viewport, so a fast scroll never shows
/// a gap before Rust answers.
pub const OVERSCAN_ROWS: usize = 2;

/// Rows sent before the screen has reported its geometry. Enough to cover a
/// 4K-tall viewport at the card pitch below.
pub const INITIAL_ROWS: usize = 16;

/// Card height and the vertical distance between card tops (logical px).
pub const CARD_HEIGHT: f32 = 132.0;
pub const ROW_PITCH: f32 = 144.0;

/// Rows a grid of `total` cards in `columns` columns needs.
#[must_use]
pub fn grid_rows(total: usize, columns: usize) -> usize {
    total.div_ceil(columns.max(1))
}

/// The card indices (into the filtered list) to build for a viewport of
/// `viewport_h` scrolled to `scroll_y`, including [`OVERSCAN_ROWS`].
#[must_use]
pub fn visible_window(
    total: usize,
    columns: usize,
    row_pitch: f32,
    scroll_y: f32,
    viewport_h: f32,
) -> Range<usize> {
    let columns = columns.max(1);
    if total == 0 || row_pitch <= 0.0 {
        return 0..0;
    }
    let rows = grid_rows(total, columns);
    let scroll = scroll_y.max(0.0);
    let first_visible = (scroll / row_pitch).floor() as usize;
    let last_visible = ((scroll + viewport_h.max(0.0)) / row_pitch).ceil() as usize;
    let first = first_visible.saturating_sub(OVERSCAN_ROWS).min(rows);
    let end_row = (last_visible + OVERSCAN_ROWS).min(rows).max(first);
    (first * columns).min(total)..(end_row * columns).min(total)
}

/// The window sent before any geometry report.
#[must_use]
pub fn initial_window(total: usize, columns: usize) -> Range<usize> {
    0..(INITIAL_ROWS * columns.max(1)).min(total)
}

// --- Timer studio ----------------------------------------------------------------

/// What the studio's timer does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerKind {
    Countdown,
    Stopwatch,
    Off,
}

impl TimerKind {
    /// UI id.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            TimerKind::Countdown => "countdown",
            TimerKind::Stopwatch => "stopwatch",
            TimerKind::Off => "off",
        }
    }

    /// Parse [`TimerKind::slug`].
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [TimerKind::Countdown, TimerKind::Stopwatch, TimerKind::Off]
            .into_iter()
            .find(|k| k.slug() == slug)
    }
}

/// Seconds the stepper adds or removes.
pub const STEP_SECONDS: i64 = 30;

/// Message for an invalid custom time.
pub const CUSTOM_TIMER_ERROR: &str = "Type minutes (1–60), e.g. 7, or MM:SS from 00:01 to 60:00";

/// Parse a custom timer entry: whole minutes (`7`, `7m`, `7 min`), seconds
/// (`45s`, `45 sec`) or `MM:SS`, 1 s – 60 min. `None` for anything else.
#[must_use]
pub fn parse_custom_time(text: &str) -> Option<TimeLimit> {
    let t = text.trim().to_ascii_lowercase();
    if t.contains(':') {
        return TimeLimit::parse_mm_ss(&t).ok();
    }
    let digits_end = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let (number, unit) = t.split_at(digits_end);
    if number.is_empty() || number.len() > 4 {
        return None;
    }
    let n: u32 = number.parse().ok()?;
    let seconds = match unit.trim() {
        "" | "m" | "min" | "mins" | "minute" | "minutes" => n.checked_mul(60)?,
        "s" | "sec" | "secs" | "second" | "seconds" => n,
        _ => return None,
    };
    TimeLimit::custom(seconds).ok()
}

/// Index of the default preset (5 minutes).
pub const DEFAULT_PRESET: usize = 4;

/// The timer studio (EX-07): ten presets, a ±30 s stepper and a MM:SS
/// field, 1 s – 60 min, countdown / stopwatch / off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerStudio {
    kind: TimerKind,
    limit: TimeLimit,
    /// Text of the last invalid custom entry's error.
    error: Option<&'static str>,
}

impl Default for TimerStudio {
    fn default() -> Self {
        TimerStudio {
            kind: TimerKind::Countdown,
            limit: TimeLimit::from_preset(
                TimerPreset::all()
                    .get(DEFAULT_PRESET)
                    .copied()
                    .unwrap_or(TimerPreset::Min5),
            ),
            error: None,
        }
    }
}

/// `MM:SS` (minutes zero-padded to two digits; 60:00 is the maximum).
#[must_use]
pub fn mm_ss(seconds: u32) -> String {
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// A preset chip label: `30s`, `1m` … `30m`.
#[must_use]
pub fn preset_label(preset: TimerPreset) -> String {
    match preset.seconds() {
        s if s < 60 => format!("{s}s"),
        s => format!("{}m", s / 60),
    }
}

impl TimerStudio {
    /// The ten preset labels, in order.
    #[must_use]
    pub fn preset_labels() -> Vec<String> {
        TimerPreset::all()
            .iter()
            .map(|&p| preset_label(p))
            .collect()
    }

    /// Countdown / stopwatch / off.
    #[must_use]
    pub fn kind(&self) -> TimerKind {
        self.kind
    }

    /// The countdown length.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn limit(&self) -> TimeLimit {
        self.limit
    }

    /// The error of the last custom entry, if it was invalid.
    #[must_use]
    pub fn error(&self) -> Option<&'static str> {
        self.error
    }

    /// The preset matching the current countdown, if any.
    #[must_use]
    pub fn active_preset(&self) -> Option<usize> {
        if self.kind != TimerKind::Countdown {
            return None;
        }
        TimerPreset::all()
            .iter()
            .position(|p| p.seconds() == self.limit.seconds())
    }

    /// Pick preset `index` (also switches to countdown). False for a bad index.
    pub fn choose_preset(&mut self, index: usize) -> bool {
        let Some(&preset) = TimerPreset::all().get(index) else {
            return false;
        };
        self.limit = TimeLimit::from_preset(preset);
        self.kind = TimerKind::Countdown;
        self.error = None;
        true
    }

    /// Switch countdown / stopwatch / off; the countdown length is kept.
    pub fn set_kind(&mut self, kind: TimerKind) {
        self.kind = kind;
        self.error = None;
    }

    /// Add `delta` seconds to the countdown, clamped to 1 s – 60 min.
    pub fn step(&mut self, delta: i64) {
        let next = (i64::from(self.limit.seconds()) + delta).clamp(
            i64::from(TimeLimit::MIN_SECONDS),
            i64::from(TimeLimit::MAX_SECONDS),
        );
        if let Some(limit) = u32::try_from(next)
            .ok()
            .and_then(|s| TimeLimit::custom(s).ok())
        {
            self.limit = limit;
        }
        self.kind = TimerKind::Countdown;
        self.error = None;
    }

    /// Apply a typed MM:SS. An invalid entry keeps the previous limit and
    /// records [`CUSTOM_TIMER_ERROR`] (START is then blocked, EX-13).
    ///
    /// # Errors
    ///
    /// [`CUSTOM_TIMER_ERROR`] when `text` is not MM:SS within 1 s – 60 min.
    pub fn set_custom(&mut self, text: &str) -> Result<(), &'static str> {
        match parse_custom_time(text) {
            Some(limit) => {
                self.limit = limit;
                self.kind = TimerKind::Countdown;
                self.error = None;
                Ok(())
            }
            None => {
                self.error = Some(CUSTOM_TIMER_ERROR);
                Err(CUSTOM_TIMER_ERROR)
            }
        }
    }

    /// The custom field was emptied: no custom entry, so no error; the
    /// last valid timer stays.
    pub fn clear_custom(&mut self) {
        self.error = None;
    }

    /// Take a timer chosen elsewhere (the Typing screen's timer menu).
    pub fn set_mode(&mut self, timer: TimerMode) {
        match timer {
            TimerMode::Countdown(limit) => {
                self.limit = limit;
                self.kind = TimerKind::Countdown;
            }
            TimerMode::Stopwatch => self.kind = TimerKind::Stopwatch,
            TimerMode::Off => self.kind = TimerKind::Off,
        }
        self.error = None;
    }

    /// Whether the studio holds a usable timer.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.error.is_none()
    }

    /// The session timer.
    #[must_use]
    pub fn mode(&self) -> TimerMode {
        match self.kind {
            TimerKind::Countdown => TimerMode::Countdown(self.limit),
            TimerKind::Stopwatch => TimerMode::Stopwatch,
            TimerKind::Off => TimerMode::Off,
        }
    }
}

/// The big digital readout of a timer.
#[must_use]
pub fn readout(timer: TimerMode) -> String {
    match timer {
        TimerMode::Countdown(l) => mm_ss(l.seconds()),
        TimerMode::Stopwatch => "00:00".to_owned(),
        TimerMode::Off => "--:--".to_owned(),
    }
}

/// One line under the readout, e.g. "5 minutes countdown".
#[must_use]
pub fn timer_sentence(timer: TimerMode) -> String {
    match timer {
        TimerMode::Countdown(l) => {
            let s = l.seconds();
            let text = match (s / 60, s % 60) {
                (0, sec) => format!("{sec} second{}", if sec == 1 { "" } else { "s" }),
                (1, 0) => "1 minute".to_owned(),
                (m, 0) => format!("{m} minutes"),
                (m, sec) => format!("{m} min {sec} s"),
            };
            format!("{text} countdown")
        }
        TimerMode::Stopwatch => "Stopwatch: counts up, no time limit".to_owned(),
        TimerMode::Off => "No timer shown".to_owned(),
    }
}

/// The START button's text: it states the consequence (DESIGN 20.6).
#[must_use]
pub fn start_label(mode: PracticeMode, timer: TimerMode) -> String {
    match (mode.dock(), timer) {
        (Dock::Guide, _) => "Start Lesson".to_owned(),
        (Dock::LockedTimer, TimerMode::Countdown(l)) => {
            format!("Begin Exam Practice ({})", mm_ss(l.seconds()))
        }
        (Dock::LockedTimer, _) => "Begin Exam Practice".to_owned(),
        (Dock::TimerStudio, TimerMode::Countdown(l)) => {
            format!("Begin Timed Session ({})", mm_ss(l.seconds()))
        }
        (Dock::TimerStudio, TimerMode::Stopwatch) => "Begin Session (stopwatch)".to_owned(),
        (Dock::TimerStudio, TimerMode::Off) => "Begin Untimed Session".to_owned(),
    }
}

/// Why START is disabled, or `None` when it can start (EX-13).
#[must_use]
pub fn start_block_reason(
    list_empty: bool,
    exercise_ready: bool,
    mode: PracticeMode,
    studio_valid: bool,
) -> Option<&'static str> {
    if list_empty {
        return Some("No exercises are installed for this list yet");
    }
    if !exercise_ready {
        return Some("Choose an exercise to begin");
    }
    if mode.timer_policy() == TimerPolicy::UserSet && !studio_valid {
        return Some("Fix the custom timer first (MM:SS, 00:01 – 60:00)");
    }
    None
}

// --- Inspector -------------------------------------------------------------------

/// Practice targets for a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Targets {
    pub wpm: u32,
    pub accuracy: u32,
}

/// Suggested practice targets from the difficulty level. The sources carry
/// no per-exercise targets, so the screen labels these "suggested".
#[must_use]
pub fn suggested_targets(layout: LayoutId, difficulty: Option<u8>) -> Targets {
    let d = u32::from(difficulty.unwrap_or(3).clamp(1, 5));
    let base = if layout == LayoutId::Qwerty { 20 } else { 15 };
    Targets {
        wpm: base + 5 * (d - 1),
        accuracy: (90 + d).min(95),
    }
}

/// Difficulty as the reference's x/10 score.
#[must_use]
pub fn difficulty_score(difficulty: Option<u8>) -> Option<u8> {
    difficulty.map(|d| d.clamp(1, 5) * 2)
}

/// Estimated minutes at the target speed, at least 1.
#[must_use]
pub fn estimated_minutes(words: u32, target_wpm: u32) -> u32 {
    if target_wpm == 0 {
        return 1;
    }
    words.div_ceil(target_wpm).max(1)
}

/// The keys and fingers an exercise drills most.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DrillProfile {
    /// Up to eight key legends, most frequent first (`⇧` marks Shift).
    pub keys: Vec<String>,
    /// Up to four finger zones with their share, e.g. "Left index 28%".
    pub fingers: Vec<String>,
}

/// Characters of the text examined (bounds the planning cost of long exam
/// passages; the head of a passage is representative).
const DRILL_SAMPLE_CHARS: usize = 1500;

fn finger_label(hand: Hand, finger: Finger) -> String {
    let hand = match hand {
        Hand::Left => "Left",
        Hand::Right => "Right",
    };
    let finger = match finger {
        Finger::Pinky => "little",
        Finger::Ring => "ring",
        Finger::Middle => "middle",
        Finger::Index => "index",
        Finger::Thumb => "thumb",
    };
    format!("{hand} {finger}")
}

/// Count the physical keys (and their fingers) the text needs on `layout`.
#[must_use]
pub fn drill_profile(layout: LayoutId, content: &str) -> DrillProfile {
    let Ok(board) = load_layout(layout) else {
        return DrillProfile::default();
    };
    let sample: String = content.chars().take(DRILL_SAMPLE_CHARS).collect();
    let guide = Guide::new(board, &sample);
    let mut keys: HashMap<String, usize> = HashMap::new();
    let mut key_order: Vec<String> = Vec::new();
    let mut fingers: HashMap<String, usize> = HashMap::new();
    let mut finger_order: Vec<String> = Vec::new();
    let mut strokes = 0usize;
    for unit in guide.units() {
        let Some(plan) = guide.plan_for(unit) else {
            continue;
        };
        for step in &plan.steps {
            let PlanStep::Key(entry) = step else {
                continue;
            };
            let legend = entry.key.legend();
            let label = match entry.layer {
                Layer::Base => legend.to_owned(),
                Layer::Shift => format!("⇧{legend}"),
                Layer::AltGr => format!("AltGr {legend}"),
                Layer::ShiftAltGr => format!("⇧AltGr {legend}"),
            };
            let (hand, finger) = board.finger_for(entry.key);
            let zone = finger_label(hand, finger);
            strokes += 1;
            let n = keys.entry(label.clone()).or_insert(0);
            if *n == 0 {
                key_order.push(label);
            }
            *n += 1;
            let n = fingers.entry(zone.clone()).or_insert(0);
            if *n == 0 {
                finger_order.push(zone);
            }
            *n += 1;
        }
    }
    let ranked = |order: Vec<String>, counts: &HashMap<String, usize>, take: usize| {
        let mut v: Vec<(String, usize)> = order
            .into_iter()
            .map(|k| {
                let n = counts.get(&k).copied().unwrap_or(0);
                (k, n)
            })
            .collect();
        // Stable: ties keep first-seen order.
        v.sort_by_key(|a| std::cmp::Reverse(a.1));
        v.truncate(take);
        v
    };
    let total = strokes.max(1);
    DrillProfile {
        keys: ranked(key_order, &keys, 8)
            .into_iter()
            .map(|(k, _)| k)
            .collect(),
        fingers: ranked(finger_order, &fingers, 4)
            .into_iter()
            .map(|(f, n)| format!("{f} {}%", (n * 100 + total / 2) / total))
            .collect(),
    }
}

/// The inspector's coaching note for a mode.
#[must_use]
pub fn coaching_note(mode: PracticeMode, targets: Targets) -> String {
    match mode {
        PracticeMode::Learn => {
            "Guide on, no timer and no verdict: follow the next-key hint and keep your eyes on the text."
                .to_owned()
        }
        PracticeMode::Beginner => {
            "Beginner track: one new finger movement at a time. Accuracy first; there is no timer."
                .to_owned()
        }
        PracticeMode::Classic => {
            "Classic track: full words and sentences at your own pace, with the guide available."
                .to_owned()
        }
        PracticeMode::Test => format!(
            "Timed test: hold {}% accuracy first, then push toward {} WPM.",
            targets.accuracy, targets.wpm
        ),
        PracticeMode::Flow => {
            "Keep a steady rhythm: smooth, even keystrokes beat fast bursts.".to_owned()
        }
        PracticeMode::Exam => {
            "Exam practice: the timer, passage size and rules come from this exam's profile. \
             The notice above says which rules are verified; unverified ones give no official result."
                .to_owned()
        }
    }
}

/// The empty-list message of a product or pool (EX-12): honest, never
/// invented content.
#[must_use]
pub fn empty_message(product: Option<ProductId>) -> (&'static str, String) {
    match product {
        Some(ProductId::NumberPunctuation) => (
            "Number + Punctuation drills are on their way",
            "The 50 Track 03 drills are not installed, and none were invented to fill this \
             screen. They appear here after the next content import."
                .to_owned(),
        ),
        Some(p @ (ProductId::TypingFlow | ProductId::HindiLearn(_))) => (
            "Content coming",
            format!(
                "{} exercises have not been imported yet, and none were invented to fill \
                 this list.",
                p.display_name()
            ),
        ),
        Some(_) => (
            "No exercises installed",
            "Run the content importer (tools/import-golu-content) and restart Radha.".to_owned(),
        ),
        None => (
            "Passage pool not installed",
            "The shared exam passage pool is empty. Run the content importer and restart Radha."
                .to_owned(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(n: u32, cat: &str, d: Option<u8>, track: Option<&str>) -> CardMeta {
        CardMeta {
            id: i64::from(n) + 1000,
            number: n,
            title: format!("Lesson {n}"),
            category: Some(cat.to_owned()),
            difficulty: d,
            status: ContentStatus::Verified,
            track: track.map(str::to_owned),
            word_count: Some(120),
            best: None,
            completed: false,
            flow: None,
        }
    }

    #[test]
    fn mode_matrix_matches_prd_17_2_for_every_product() {
        for product in ProductId::all() {
            let m = mode_matrix(product);
            assert!(m.allows(m.default), "{product:?} default must be allowed");
            match product {
                ProductId::EnglishClassic | ProductId::HindiMastery(_) => {
                    assert_eq!(m.toggle, Some([PracticeMode::Learn, PracticeMode::Test]));
                    assert_eq!(PracticeMode::Learn.timer_policy(), TimerPolicy::None);
                    assert_eq!(PracticeMode::Test.timer_policy(), TimerPolicy::UserSet);
                }
                ProductId::TypingFlow | ProductId::NumberPunctuation => {
                    // EX-09: no LEARN option, always user-timed.
                    assert_eq!(m.toggle, None);
                    assert_eq!(m.default, PracticeMode::Flow);
                    assert!(!m.allows(PracticeMode::Learn));
                    assert_eq!(m.default.dock(), Dock::TimerStudio);
                }
                ProductId::HindiLearn(_) => {
                    // EX-10: BEGINNER | CLASSIC, no timer either way.
                    assert_eq!(
                        m.toggle,
                        Some([PracticeMode::Beginner, PracticeMode::Classic])
                    );
                    assert!(!m.allows(PracticeMode::Test));
                    for mode in [PracticeMode::Beginner, PracticeMode::Classic] {
                        assert_eq!(mode.dock(), Dock::Guide);
                        assert_eq!(timer_for(mode, TimerMode::Stopwatch), TimerMode::Off);
                    }
                }
            }
        }
        assert_eq!(EXAM_MATRIX.default.dock(), Dock::LockedTimer);
        assert!(!EXAM_MATRIX.allows(PracticeMode::Test));
    }

    #[test]
    fn learn_never_has_a_timer_and_exam_timer_is_locked() {
        let studio = TimerStudio::default().mode();
        assert_eq!(timer_for(PracticeMode::Learn, studio), TimerMode::Off);
        assert_eq!(timer_for(PracticeMode::Test, studio), studio);
        assert_eq!(
            timer_for(PracticeMode::Flow, TimerMode::Off),
            TimerMode::Off
        );
        let exam = timer_for(PracticeMode::Exam, TimerMode::Off);
        assert_eq!(
            exam.limit().map(|l| l.seconds()),
            Some(EXAM_PRACTICE_SECONDS)
        );
    }

    #[test]
    fn mode_slugs_round_trip_and_match_the_sessions_check_constraint() {
        for slug in ["learn", "test", "beginner", "classic", "flow", "exam"] {
            assert_eq!(
                PracticeMode::from_slug(slug).map(PracticeMode::slug),
                Some(slug)
            );
        }
        assert_eq!(PracticeMode::from_slug("nope"), None);
        assert_eq!(PracticeMode::Beginner.track(), Some("beginner"));
        assert_eq!(PracticeMode::Test.track(), None);
    }

    #[test]
    fn the_all_in_one_mangal_exam_types_only_mangal_unicode_layouts() {
        for layout in MANGAL_LAYOUTS {
            assert!(exam_layout_allowed(true, layout));
            assert_eq!(hindi_only_carrier(layout), ProductId::HindiMastery(layout));
        }
        assert!(!exam_layout_allowed(true, LayoutId::Qwerty), "no English");
        assert!(
            !exam_layout_allowed(true, LayoutId::KrutiDev),
            "Kruti Dev is not Mangal"
        );
        assert!(exam_layout_allowed(false, LayoutId::KrutiDev));
        assert_eq!(
            hindi_only_carrier(LayoutId::KrutiDev),
            ProductId::HindiMastery(LayoutId::RemingtonGail)
        );
        assert_eq!(
            hindi_only_carrier(LayoutId::Qwerty),
            ProductId::HindiMastery(LayoutId::RemingtonGail)
        );
    }

    #[test]
    fn exam_language_and_carrier() {
        assert_eq!(ExamLanguage::from_id("eng"), Some(ExamLanguage::English));
        assert_eq!(ExamLanguage::from_id("hin"), Some(ExamLanguage::Hindi));
        assert_eq!(ExamLanguage::from_id("x"), None);
        assert_eq!(ExamLanguage::Hindi.pool(), ExamPool::Hindi);
        assert_eq!(
            exam_carrier(ExamLanguage::English, "Kruti Dev"),
            ProductId::EnglishClassic
        );
        assert_eq!(
            exam_carrier(ExamLanguage::Hindi, "English · Kruti Dev 010"),
            ProductId::HindiMastery(LayoutId::KrutiDev)
        );
        assert_eq!(
            exam_carrier(ExamLanguage::Hindi, "English · Hindi"),
            ProductId::HindiMastery(LayoutId::RemingtonGail)
        );
        assert_eq!(
            ExamLanguage::of_layout(LayoutId::RemingtonCbi),
            ExamLanguage::Hindi
        );
    }

    #[test]
    fn filters_combine_search_difficulty_category_and_track() {
        let cards = vec![
            card(1, "Home Row", Some(1), Some("beginner")),
            card(2, "Home Row", Some(3), Some("beginner")),
            card(3, "Top Row", Some(4), Some("classic")),
            card(4, "Top Row", None, Some("classic")),
        ];
        let all = ListFilter::default();
        assert_eq!(filter_cards(&cards, &all), vec![0, 1, 2, 3]);

        let easy = ListFilter {
            difficulty: DifficultyBand::Easy,
            ..ListFilter::default()
        };
        assert_eq!(filter_cards(&cards, &easy), vec![0]);
        let medium = ListFilter {
            difficulty: DifficultyBand::Medium,
            ..ListFilter::default()
        };
        assert_eq!(filter_cards(&cards, &medium), vec![1]);
        let hard = ListFilter {
            difficulty: DifficultyBand::Hard,
            ..ListFilter::default()
        };
        assert_eq!(
            filter_cards(&cards, &hard),
            vec![2],
            "unrated only under All"
        );

        let top = ListFilter {
            category: Some("Top Row".into()),
            ..ListFilter::default()
        };
        assert_eq!(filter_cards(&cards, &top), vec![2, 3]);

        let beginner = ListFilter {
            track: Some("beginner"),
            ..ListFilter::default()
        };
        assert_eq!(filter_cards(&cards, &beginner), vec![0, 1]);

        for (q, want) in [
            ("lesson 3", vec![2]),
            ("EX-004", vec![3]),
            ("ex-00", vec![0, 1, 2, 3]),
            ("home", vec![0, 1]),
            ("TOP row", vec![2, 3]),
            ("zzz", vec![]),
        ] {
            let f = ListFilter {
                query: q.into(),
                ..ListFilter::default()
            };
            assert_eq!(filter_cards(&cards, &f), want, "query {q:?}");
        }

        let combined = ListFilter {
            query: "lesson".into(),
            difficulty: DifficultyBand::Hard,
            category: Some("Top Row".into()),
            track: Some("classic"),
        };
        assert_eq!(filter_cards(&cards, &combined), vec![2]);
    }

    #[test]
    fn difficulty_band_slugs_round_trip() {
        for b in [
            DifficultyBand::All,
            DifficultyBand::Easy,
            DifficultyBand::Medium,
            DifficultyBand::Hard,
        ] {
            assert_eq!(DifficultyBand::from_slug(b.slug()), Some(b));
        }
        assert_eq!(DifficultyBand::from_slug("brutal"), None);
    }

    #[test]
    fn categories_keep_first_seen_order_and_respect_the_track() {
        let cards = vec![
            card(1, "B", Some(1), Some("beginner")),
            card(2, "A", Some(1), Some("beginner")),
            card(3, "B", Some(1), Some("classic")),
        ];
        assert_eq!(
            category_counts(&cards, None),
            vec![("B".into(), 2), ("A".into(), 1)]
        );
        assert_eq!(
            category_counts(&cards, Some("classic")),
            vec![("B".into(), 1)]
        );
    }

    #[test]
    fn status_is_reported_honestly() {
        let mut cards = vec![card(1, "x", None, None), card(2, "x", None, None)];
        cards[1].status = ContentStatus::GeneratedReviewPending;
        assert_eq!(
            status_summary(&cards),
            "1 verified · 1 generated (review pending)"
        );
        assert_eq!(status_label(ContentStatus::Provisional).1, "warning");
        assert_ne!(
            status_label(ContentStatus::GeneratedReviewPending).0,
            "Verified"
        );
    }

    #[test]
    fn virtual_window_covers_the_viewport_plus_overscan() {
        // 500 cards, 3 columns -> 167 rows.
        assert_eq!(grid_rows(500, 3), 167);
        assert_eq!(grid_rows(0, 3), 0);
        assert_eq!(grid_rows(7, 0), 7, "zero columns is treated as one");

        // Top: rows 0..(ceil(600/144)=5)+2 = 7 rows -> 21 cards.
        assert_eq!(visible_window(500, 3, ROW_PITCH, 0.0, 600.0), 0..21);
        // Scrolled to row 10 exactly: first visible 10, minus overscan 8.
        let w = visible_window(500, 3, ROW_PITCH, 1440.0, 600.0);
        assert_eq!(w, 24..51);
        // Never more than the viewport's rows + 2 * overscan + 1 partial.
        assert!(w.len() <= (5 + 2 * OVERSCAN_ROWS + 1) * 3);
        // Bottom: clamped to the total, last row partial (500 = 166*3 + 2).
        let end = visible_window(500, 3, ROW_PITCH, 167.0 * ROW_PITCH, 600.0);
        assert_eq!(end.end, 500);
        assert!(end.start < end.end);
        // Past the end (stale scroll after a filter shrinks the list).
        let past = visible_window(10, 3, ROW_PITCH, 99_999.0, 600.0);
        assert!(past.end <= 10 && past.start <= past.end);
        // Negative scroll (overscroll bounce) behaves like 0.
        assert_eq!(
            visible_window(500, 3, ROW_PITCH, -50.0, 600.0),
            visible_window(500, 3, ROW_PITCH, 0.0, 600.0)
        );
        assert_eq!(visible_window(0, 3, ROW_PITCH, 0.0, 600.0), 0..0);
        assert_eq!(visible_window(5, 2, 0.0, 0.0, 600.0), 0..0);
        // Two columns at the narrow breakpoint.
        assert_eq!(visible_window(200, 2, ROW_PITCH, 0.0, 300.0), 0..10);
        assert_eq!(initial_window(500, 3), 0..48);
        assert_eq!(initial_window(20, 3), 0..20);
    }

    #[test]
    fn timer_studio_presets_stepper_and_kinds() {
        let labels = TimerStudio::preset_labels();
        assert_eq!(
            labels,
            ["30s", "1m", "2m", "3m", "5m", "10m", "15m", "20m", "25m", "30m"]
        );
        let mut s = TimerStudio::default();
        assert_eq!(s.active_preset(), Some(DEFAULT_PRESET));
        assert_eq!(readout(s.mode()), "05:00");
        assert!(s.choose_preset(0));
        assert_eq!(readout(s.mode()), "00:30");
        assert!(!s.choose_preset(10), "only ten presets");
        s.step(STEP_SECONDS);
        assert_eq!(s.limit().seconds(), 60);
        assert_eq!(s.active_preset(), Some(1), "60 s is the 1m preset");
        s.step(-STEP_SECONDS);
        s.step(-STEP_SECONDS);
        assert_eq!(s.limit().seconds(), 1, "clamped at 1 s");
        assert_eq!(s.active_preset(), None);
        assert!(s.choose_preset(9));
        s.step(STEP_SECONDS * 100);
        assert_eq!(s.limit().seconds(), 3600, "clamped at 60 min");
        assert_eq!(readout(s.mode()), "60:00");

        s.set_kind(TimerKind::Stopwatch);
        assert_eq!(s.mode(), TimerMode::Stopwatch);
        assert_eq!(s.active_preset(), None);
        s.set_kind(TimerKind::Off);
        assert_eq!(s.mode(), TimerMode::Off);
        assert_eq!(readout(s.mode()), "--:--");
        s.step(STEP_SECONDS);
        assert_eq!(
            s.kind(),
            TimerKind::Countdown,
            "stepping implies a countdown"
        );
        for k in [TimerKind::Countdown, TimerKind::Stopwatch, TimerKind::Off] {
            assert_eq!(TimerKind::from_slug(k.slug()), Some(k));
        }
    }

    #[test]
    fn timer_studio_rejects_invalid_mm_ss_and_blocks_start() {
        let mut s = TimerStudio::default();
        assert_eq!(s.set_custom(" 1:30 "), Ok(()));
        assert_eq!(s.limit().seconds(), 90);
        for bad in [
            "", "90", "1:5", "1:60", "00:00", "60:01", "61:00", "abc", "1:3a", "-1:00", "123:00",
            ":30", "1:",
        ] {
            assert_eq!(s.set_custom(bad), Err(CUSTOM_TIMER_ERROR), "{bad:?}");
            assert!(!s.is_valid());
            assert_eq!(s.limit().seconds(), 90, "{bad:?} keeps the last valid time");
            assert_eq!(
                start_block_reason(false, true, PracticeMode::Test, s.is_valid()),
                Some("Fix the custom timer first (MM:SS, 00:01 – 60:00)")
            );
        }
        // An invalid entry does not block a mode without a user timer.
        assert_eq!(
            start_block_reason(false, true, PracticeMode::Learn, s.is_valid()),
            None
        );
        assert_eq!(s.set_custom("60:00"), Ok(()));
        assert_eq!(s.set_custom("00:01"), Ok(()));
        assert!(s.is_valid());
        assert_eq!(s.error(), None);
        assert!(s.set_custom("x").is_err());
        assert!(s.choose_preset(2), "a preset clears the error");
        assert!(s.is_valid());
    }

    /// Regression (custom timer "does not work"): typing plain minutes was
    /// rejected because only strict MM:SS was accepted.
    #[test]
    fn custom_timer_accepts_minutes_seconds_and_mm_ss() {
        let secs = |t: &str| parse_custom_time(t).map(TimeLimit::seconds);
        assert_eq!(secs("7"), Some(420));
        assert_eq!(secs(" 60 "), Some(3600));
        assert_eq!(secs("1"), Some(60));
        assert_eq!(secs("7m"), Some(420));
        assert_eq!(secs("7 min"), Some(420));
        assert_eq!(secs("45s"), Some(45));
        assert_eq!(secs("1 sec"), Some(1));
        assert_eq!(secs("3600s"), Some(3600));
        assert_eq!(secs("0:01"), Some(1));
        assert_eq!(secs("12:34"), Some(754));
        for bad in [
            "", "0", "61", "90", "0s", "3601s", "7h", "m", "1.5", "-3", "99999",
        ] {
            assert_eq!(secs(bad), None, "{bad:?}");
        }
        let mut s = TimerStudio::default();
        assert_eq!(s.set_custom("7"), Ok(()));
        assert_eq!(
            s.mode(),
            TimerMode::Countdown(TimeLimit::custom(420).unwrap())
        );
        s.set_custom("x").unwrap_err();
        s.clear_custom();
        assert!(s.is_valid(), "an emptied field is not an error");
        assert_eq!(s.limit().seconds(), 420, "and keeps the last valid timer");
        s.set_mode(TimerMode::Countdown(TimeLimit::custom(5).unwrap()));
        assert_eq!(s.limit().seconds(), 5);
        s.set_mode(TimerMode::Stopwatch);
        assert_eq!(s.mode(), TimerMode::Stopwatch);
    }

    #[test]
    fn start_is_blocked_with_a_reason_until_ready() {
        assert!(start_block_reason(true, false, PracticeMode::Test, true).is_some());
        assert_eq!(
            start_block_reason(false, false, PracticeMode::Learn, true),
            Some("Choose an exercise to begin")
        );
        assert_eq!(
            start_block_reason(false, true, PracticeMode::Exam, false),
            None
        );
    }

    #[test]
    fn start_label_states_the_consequence() {
        let five = TimerMode::Countdown(TimeLimit::from_preset(TimerPreset::Min5));
        assert_eq!(
            start_label(PracticeMode::Test, five),
            "Begin Timed Session (05:00)"
        );
        assert_eq!(
            start_label(PracticeMode::Learn, TimerMode::Off),
            "Start Lesson"
        );
        assert_eq!(
            start_label(PracticeMode::Beginner, TimerMode::Off),
            "Start Lesson"
        );
        assert_eq!(
            start_label(PracticeMode::Flow, TimerMode::Stopwatch),
            "Begin Session (stopwatch)"
        );
        assert_eq!(
            start_label(
                PracticeMode::Exam,
                timer_for(PracticeMode::Exam, TimerMode::Off)
            ),
            "Begin Exam Practice (10:00)"
        );
        assert_eq!(timer_sentence(five), "5 minutes countdown");
        let ninety = TimerMode::Countdown(TimeLimit::custom(90).unwrap());
        assert_eq!(timer_sentence(ninety), "1 min 30 s countdown");
        let one = TimerMode::Countdown(TimeLimit::custom(1).unwrap());
        assert_eq!(timer_sentence(one), "1 second countdown");
    }

    #[test]
    fn targets_scale_with_difficulty() {
        assert_eq!(
            suggested_targets(LayoutId::Qwerty, Some(1)),
            Targets {
                wpm: 20,
                accuracy: 91
            }
        );
        assert_eq!(
            suggested_targets(LayoutId::RemingtonGail, Some(5)),
            Targets {
                wpm: 35,
                accuracy: 95
            }
        );
        assert_eq!(suggested_targets(LayoutId::Qwerty, None).wpm, 30);
        assert_eq!(difficulty_score(Some(3)), Some(6));
        assert_eq!(difficulty_score(None), None);
        assert_eq!(estimated_minutes(130, 25), 6);
        assert_eq!(estimated_minutes(3, 25), 1);
        assert_eq!(estimated_minutes(3, 0), 1);
    }

    #[test]
    fn drill_profile_counts_keys_and_fingers() {
        let p = drill_profile(LayoutId::Qwerty, "asdf jkl; asdf");
        assert!(p.keys.contains(&"A".to_owned()), "{:?}", p.keys);
        assert!(!p.keys.iter().any(|k| k == "Space"));
        assert!(p.keys.len() <= 8);
        assert!(!p.fingers.is_empty() && p.fingers.len() <= 4);
        assert!(p.fingers[0].ends_with('%'));
        let shifted = drill_profile(LayoutId::Qwerty, "AAA");
        assert_eq!(shifted.keys, ["⇧A"]);
        // Hindi plans through the layout's own table.
        let hindi = drill_profile(LayoutId::RemingtonGail, "कार्य");
        assert!(hindi.keys.contains(&"D".to_owned()), "{:?}", hindi.keys);
        assert_eq!(drill_profile(LayoutId::Qwerty, ""), DrillProfile::default());
    }

    #[test]
    fn empty_states_are_honest() {
        let (title, body) = empty_message(Some(ProductId::NumberPunctuation));
        assert!(title.contains("Number + Punctuation"));
        assert!(body.contains("none were invented"));
        assert!(empty_message(Some(ProductId::TypingFlow))
            .1
            .contains("none were invented"));
        assert!(empty_message(None).0.contains("pool"));
    }
}
