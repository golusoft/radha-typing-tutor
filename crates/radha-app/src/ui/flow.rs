//! The Exercise → Typing → Result flow, wired to the window.
//!
//! [`Flow`] owns navigation and the practice state; the Slint globals
//! (`ExerciseView`, `TypingView`, `ResultView`) only display what it writes
//! and raise requests it answers (`RULES.MD` Rule 16). The rules themselves
//! (mode matrix, filters, virtual window, timer studio) are in
//! [`crate::exercise`]; this file applies them.
//!
//! Database use is confined to screen changes: the metadata list (and the
//! profile's bests in one query) when a course opens, one exercise's text
//! when it is selected, and one `save_session` transaction when a session
//! completes. Key presses, scrolling and timer ticks touch RAM only
//! (`RULES.MD` Rule 14).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use radha_content::{ContentStatus, Exercise, ExerciseRepository, ProductId, ProgressRepository};
use radha_core::{CompletionReason, DisplayMode, SessionState, TimerMode};
use radha_layouts::krutidev::to_unicode;
use radha_layouts::LayoutId;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::{
    apply_screen, AppWindow, DiffLine, DiffWord, Exams, ExerciseCard, ExerciseView, FilterPillItem,
    FingerBar, FlowBar, FlowHesitation, FlowView, FlowWeakKey, HeatKey, HeatRow, LearnChip,
    PassageLine, PassageSegment, PickItem, Products, ResultView, SegmentOption, TimerChoice,
    TypingView, UtilityView, WeakKeyRow,
};
use crate::audio::{Audio, Sound};
use crate::exam_mode;
use crate::exercise::{
    card_code, category_counts, coaching_note, difficulty_score, drill_profile, empty_message,
    estimated_minutes, exam_carrier, exam_layout_allowed, filter_cards, grid_rows,
    hindi_only_carrier, initial_window, mode_matrix, readout, start_block_reason, start_label,
    status_label, status_summary, suggested_targets, timer_for, timer_sentence, visible_window,
    CardMeta, DifficultyBand, Dock, DrillProfile, ExamLanguage, ListFilter, PracticeMode,
    TimerKind, TimerPolicy, TimerStudio, CARD_HEIGHT, ROW_PITCH, STEP_SECONDS,
};
use crate::hindi_learn::{self, LearnProgress};
use crate::navigation::{escape_action, EscapeAction, NavigationState, Screen, SessionContext};
use crate::passage;
use crate::practice::{
    board, classify_key, clock, iso_utc, HintView, KeyInput, LoggedKey, Practice, Replayed,
};
use crate::recovery::{self, Journal, JOURNAL_EVERY};
use crate::report::{self, Certification, DiffKind, WordDiff};
use crate::rhythm::{self, WAVE_BARS};
use crate::typing_rules::{
    effective_guide, finger_sentence, guide_show, parse_word_limit, ring_arc, BackspacePolicy,
    GuideMode, HighlightMode, Streak, TypingRules, KEY_LIMIT,
};
use radha_core::flow::{CadenceBar, FlowConfig, FlowReport, FlowTracker, RhythmLabel};
use radha_layouts::KeyEffect;

/// How often the typing clock is redrawn (and a countdown checked).
const TICK: Duration = Duration::from_millis(100);
/// Columns assumed until the screen reports its geometry.
const DEFAULT_COLUMNS: usize = 3;

/// One-line description of a timer mode (typing screen header).
#[must_use]
pub fn timer_summary(mode: TimerMode) -> String {
    match mode {
        TimerMode::Countdown(l) => format!("{} countdown", clock(f64::from(l.seconds()))),
        TimerMode::Stopwatch => "stopwatch".into(),
        TimerMode::Off => "no timer".into(),
    }
}

/// The stored exercise a session journal of `product` resumes, typed as
/// `product`. A borrowed passage (InScript types the GAIL course) is stored
/// under its source product, so it matches that and must still be typable
/// on the journal's layout. `None` when the journal does not match.
fn journaled_exercise(
    product: ProductId,
    mut stored: Exercise,
    typable: impl FnOnce(&Exercise) -> bool,
) -> Option<Exercise> {
    let source = product.content_source();
    if stored.product != source || (source != product && !typable(&stored)) {
        return None;
    }
    stored.product = product;
    Some(stored)
}

/// Font size of the "Your typing" pane (typing.slint `input-font`).
const INPUT_FONT_PX: f32 = 24.0;
/// Lines the "Your typing" pane shows.
const INPUT_LINES: usize = 3;

/// The "Your typing" pane: the typed text from byte `anchor` wrapped at
/// `cols`, keeping the last `max_lines` lines. Returns the new anchor (the
/// first shown line's byte offset; it only moves forward, so a key costs
/// O(shown text)), the earlier lines joined by newlines, and the last line
/// (spaces as NBSP so a trailing space shows before the caret).
fn input_lines(
    typed: &str,
    anchor: usize,
    cols: usize,
    max_lines: usize,
) -> (usize, String, String) {
    let anchor = if typed.is_char_boundary(anchor) {
        anchor
    } else {
        0
    };
    let rest = typed.get(anchor..).unwrap_or(typed);
    let chars: Vec<(usize, char)> = rest.char_indices().collect();
    let cols = cols.max(1);
    let mut starts = vec![0usize];
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        let word_start = i;
        while i < chars.len() && !chars[i].1.is_whitespace() {
            i += 1;
        }
        let word_end = i;
        while i < chars.len() && chars[i].1.is_whitespace() {
            i += 1;
        }
        if word_start > start && word_end - start > cols {
            start = word_start;
            starts.push(start);
        }
        while word_end - start > cols && word_end - word_start > cols {
            start += cols;
            starts.push(start);
        }
    }
    if anchor > 0 && starts.len() < max_lines.max(1) {
        // Backspace removed shown lines: show the earlier ones again.
        return input_lines(typed, 0, cols, max_lines);
    }
    let drop = starts.len().saturating_sub(max_lines.max(1));
    let first = starts.get(drop).copied().unwrap_or(0);
    let byte = |c: usize| chars.get(c).map_or(rest.len(), |&(b, _)| b);
    let new_anchor = anchor + byte(first);
    let shown = &starts[drop..];
    let mut head = String::new();
    for w in shown.windows(2) {
        if !head.is_empty() {
            head.push('\n');
        }
        head.push_str(rest.get(byte(w[0])..byte(w[1])).unwrap_or(""));
    }
    let last = shown.last().copied().unwrap_or(0);
    let tail: String = rest
        .get(byte(last)..)
        .unwrap_or("")
        .chars()
        .map(|c| if c == ' ' { '\u{a0}' } else { c })
        .collect();
    (new_anchor, head, tail)
}

/// Byte offset in `typed` where the input pane's last line starts. The
/// earlier lines from [`input_lines`] are verbatim slices joined by `\n`.
fn tail_start(anchor: usize, head: &str) -> usize {
    anchor + head.len() - head.matches('\n').count()
}

/// How the input pane's last line moved since `mark` (columns, where it
/// started): (glide, change of the wrap counter). Exactly one line forward
/// (the old last line is now the one above) or back (Backspace removed the
/// last line) glides; anything else (restart, rewrap, a new session) snaps.
fn typed_wrap_step(
    mark: Option<(usize, usize)>,
    cols: usize,
    typed: &str,
    anchor: usize,
    head: &str,
) -> (bool, i32) {
    let start = tail_start(anchor, head);
    let Some((old_cols, old)) = mark else {
        return (false, 0);
    };
    if start == old {
        return (false, 0);
    }
    let step = if start > old { 1 } else { -1 };
    if old_cols != cols || typed.is_empty() {
        return (false, step);
    }
    let glide = if start > old {
        let above = head.rsplit('\n').next().unwrap_or("");
        !head.is_empty() && start.checked_sub(above.len()) == Some(old)
    } else {
        old >= typed.len()
    };
    (glide, step)
}

/// Printout Mode: a self-contained, print-ready page of the passage (no
/// network; it opens the print dialog on load).
fn passage_print_html(title: &str, layout: &str, words: u32, text: &str) -> String {
    let esc = report::escape_html;
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>{t}</title><style>\
         body{{font-family:'Noto Sans Devanagari','Mangal','Segoe UI',sans-serif;\
         color:#352B30;margin:18mm;line-height:1.9;font-size:15pt}}\
         h1{{font-size:17pt;margin:0 0 4pt}}p.meta{{font-size:10pt;color:#6A5A62;\
         margin:0 0 14pt;border-bottom:1px solid #DCC9B8;padding-bottom:6pt}}\
         p.text{{text-align:justify;white-space:pre-wrap}}\
         @media print{{body{{margin:12mm}}}}</style></head>\
         <body onload=\"window.print()\"><h1>{t}</h1>\
         <p class=\"meta\">Radha Typing Tutor · {l} · {words} words · practice passage</p>\
         <p class=\"text\">{x}</p></body></html>\n",
        t = esc(title),
        l = esc(layout),
        x = esc(text),
    )
}

/// Open a file the app just wrote with its default program.
fn open_file(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut command = std::process::Command::new("explorer");
    #[cfg(not(windows))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path).spawn().map(|_| ())
}

/// Accent token of a product (matches `data/products.slint`).
fn accent(product: ProductId) -> &'static str {
    match product {
        ProductId::EnglishClassic => "primary",
        ProductId::TypingFlow => "accent",
        ProductId::NumberPunctuation => "secondary",
        ProductId::HindiMastery(LayoutId::RemingtonGail | LayoutId::Qwerty)
        | ProductId::HindiLearn(LayoutId::RemingtonGail | LayoutId::Qwerty) => "success",
        ProductId::HindiMastery(LayoutId::RemingtonCbi)
        | ProductId::HindiLearn(LayoutId::RemingtonCbi) => "accent",
        ProductId::HindiMastery(LayoutId::KrutiDev) => "primary",
        ProductId::HindiLearn(LayoutId::KrutiDev) => "secondary",
        ProductId::HindiMastery(LayoutId::Inscript) | ProductId::HindiLearn(LayoutId::Inscript) => {
            "secondary"
        }
    }
}

/// Identifies what the Exercise screen lists: a product, or an exam's pool
/// in one language.
fn course_key(ctx: &SessionContext) -> String {
    match &ctx.exam_profile_id {
        Some(id) => format!(
            "exam:{id}:{}",
            ExamLanguage::of_layout(ctx.product.layout()).id()
        ),
        None => ctx.product.slug().to_owned(),
    }
}

/// Layout switch ids of a Hindi exam (ids are `LayoutId::as_str`).
const HINDI_LAYOUTS: [(&str, &str, LayoutId); 4] = [
    ("gail", "GAIL", LayoutId::RemingtonGail),
    ("cbi", "CBI", LayoutId::RemingtonCbi),
    ("krutidev", "Kruti Dev", LayoutId::KrutiDev),
    ("inscript", "InScript", LayoutId::Inscript),
];

/// Layout switch of the typing-mastery courses: QWERTY types the English
/// course, each Hindi layout its own course.
const MASTERY_LAYOUTS: [(&str, &str, LayoutId); 5] = [
    ("qwerty", "QWERTY", LayoutId::Qwerty),
    ("gail", "GAIL", LayoutId::RemingtonGail),
    ("cbi", "CBI", LayoutId::RemingtonCbi),
    ("krutidev", "Kruti Dev", LayoutId::KrutiDev),
    ("inscript", "InScript", LayoutId::Inscript),
];

/// Whether a product's Exercise screen offers the layout switch.
fn layout_switchable(product: ProductId) -> bool {
    matches!(
        product,
        ProductId::EnglishClassic | ProductId::HindiMastery(_)
    )
}

/// The typing-mastery product of a layout.
fn mastery_product(layout: LayoutId) -> ProductId {
    match layout {
        LayoutId::Qwerty => ProductId::EnglishClassic,
        hindi => ProductId::HindiMastery(hindi),
    }
}

fn layout_slug(layout: LayoutId) -> &'static str {
    HINDI_LAYOUTS
        .iter()
        .find(|(_, _, l)| *l == layout)
        .map_or("", |(id, _, _)| id)
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn segment(id: &str, label: &str) -> SegmentOption {
    SegmentOption {
        id: id.into(),
        label: label.into(),
        disabled: false,
    }
}

fn best_text(best: Option<(f64, f64)>) -> String {
    best.map(|(wpm, acc)| format!("Best {wpm:.0} WPM · {acc:.0}%"))
        .unwrap_or_default()
}

/// The exam card a launcher belongs to and its `radha-exams` profile.
#[derive(Debug, Clone, Default)]
struct ExamInfo {
    title: String,
    font_layout: String,
    /// All-in-One Mangal: Hindi only, on the Mangal Unicode layouts.
    hindi_only: bool,
    /// The card's profile (`None` only if it failed to load: practice-only).
    profile: Option<&'static radha_exams::ExamProfile>,
}

impl ExamInfo {
    /// The profile's test in `language`.
    fn test(&self, language: ExamLanguage) -> Option<&'static radha_exams::TypingTest> {
        exam_mode::test(self.profile, language)
    }

    /// Whether the card may be typed on `layout`.
    fn allows(&self, layout: LayoutId) -> bool {
        exam_layout_allowed(self.hindi_only, layout)
            && exam_mode::layout_allowed(self.profile, layout)
    }
}

/// The flow state behind the three practice screens.
pub struct Flow {
    window: slint::Weak<AppWindow>,
    nav: NavigationState,
    repo: Rc<radha_db::SqliteRepository>,
    db: Rc<RefCell<radha_db::Database>>,
    profile_id: i64,

    // Exercise screen
    cards: Vec<CardMeta>,
    course: Option<String>,
    list_failed: bool,
    filter: ListFilter,
    shown: Vec<usize>,
    columns: usize,
    scroll_y: f32,
    viewport_h: f32,
    window_range: Range<usize>,
    scroll_token: i32,
    preview: Option<Exercise>,
    preview_error: Option<String>,
    drill_cache: HashMap<(i64, LayoutId), DrillProfile>,
    studio: TimerStudio,
    guide_mode: &'static str,
    exam: ExamInfo,

    // Typing screen
    me: std::rc::Weak<RefCell<Flow>>,
    practice: Option<Practice>,
    exercise: Option<Exercise>,
    wrapped: passage::Wrapped,
    /// Width of the passage text area the screen last reported (px).
    passage_w: f32,
    lines_model: Rc<VecModel<PassageLine>>,
    lines_cache: Vec<Vec<(String, i32)>>,
    saved: bool,
    ticker: slint::Timer,
    rules: TypingRules,
    guide: GuideMode,
    /// Passage lines that fit in the card (reported by the screen).
    passage_rows: usize,
    /// A manual scroll (wheel, scrollbar): the first line shown.
    scroll_first: Option<usize>,
    /// Word number at every unit and the word total ("Word 6 of 400").
    word_no: Vec<u32>,
    word_total: u32,
    /// Char index in the typed text where the input pane's lines start.
    typed_anchor: usize,
    /// Smooth scrolling: the first passage line last drawn.
    shown_first: Option<usize>,
    /// The input pane's last line: (columns, byte where it starts), and
    /// the wrap counter the pane glides on.
    typed_mark: Option<(usize, usize)>,
    typed_wraps: i32,
    /// Add Exercise: the stored custom passages.
    customs: Vec<crate::custom::CustomExercise>,
    custom_path: PathBuf,
    /// The custom passage being typed (index into `customs`).
    custom: Option<usize>,
    /// The course the typing preferences (guide, keyboard) were set for.
    typing_course: Option<String>,
    streak: Streak,
    opened_at: Instant,
    last_input: Option<Instant>,
    recent_error: bool,
    last_hint: Option<HintView>,
    coach: String,
    notice: String,
    timer_warned: bool,
    fullscreen: bool,
    drill: Option<Drill>,
    audio: Audio,
    learn: LearnProgress,
    learn_path: PathBuf,
    journal_path: PathBuf,
    last_journal: Instant,

    // Typing Flow (P07): the rhythm engine of the running Flow session.
    flow_cfg: FlowConfig,
    rhythm: Option<FlowTracker>,
    /// Unit index the next key is expected at (from the last render).
    last_cursor: usize,
    /// What the "Esc armed" chip last showed (written only on change).
    esc_chip: bool,
    bars_buf: [CadenceBar; WAVE_BARS],
    bars_model: Rc<VecModel<FlowBar>>,
    /// What each bar row last showed (level in 1/1000, smooth), so only
    /// changed rows are written.
    bars_shown: [(u16, bool); WAVE_BARS],
    flow_label: Option<RhythmLabel>,
    flow_live_pct: Option<u8>,
    flow_scored: u64,
    pulse: bool,
    /// Width of the Flow word stream the screen last reported (px).
    flow_w: f32,
    /// Lines that fit in the Flow word stream (reported by the Flow
    /// screen; the Typing screen's count is `passage_rows`).
    flow_rows: usize,
    /// Live run, hesitation marks and missed keys of the Flow session.
    live_flow: rhythm::LiveFlow,
    /// What the live Flow strip last showed (tempo ms, streak, best run,
    /// hesitations), so unchanged values are not rewritten.
    flow_strip: (i32, u32, u32, u32),

    // Result screen
    diff: Vec<WordDiff>,
    recovery_text: Option<String>,
    /// The minute curve in both views (RS-02).
    minute_wpm: Vec<f64>,
    minute_kdph: Vec<f64>,
    /// What an export writes (RS-09); candidate filled at export time.
    scorecard: Option<report::Scorecard>,
    /// Default folder of exported scorecards.
    export_dir: PathBuf,

    // Settings defaults (P08 Settings overlay).
    default_guide: &'static str,
    /// Settings → Keyboard: the preferred layout.
    preferred_layout: LayoutId,
    /// Whether a passage can be typed on a layout, by (layout, exam pool,
    /// id); filled lazily the first time the course is listed.
    typable_cache: RefCell<HashMap<(LayoutId, bool, i64), bool>>,
    /// Passages of the listed course its layout cannot type (not listed).
    hidden: std::cell::Cell<usize>,
}

/// A generated beginner drill or the Result's recovery drill. Drills are
/// not `exercises` rows: they update the Learn path, not exercise history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drill {
    Step(usize),
    Matra,
    Half,
    Recovery,
}

impl Flow {
    /// Build the flow and wire every callback of the three screens.
    pub fn install(
        window: &AppWindow,
        nav: NavigationState,
        repo: Rc<radha_db::SqliteRepository>,
        db: Rc<RefCell<radha_db::Database>>,
        profile_id: i64,
        config_dir: &std::path::Path,
    ) -> Rc<RefCell<Flow>> {
        let learn_path = config_dir.join("hindi-learn.toml");
        let journal_path = config_dir.join("session-journal.toml");
        let custom_path = config_dir.join("custom-exercises.toml");
        let lines_model = Rc::new(VecModel::<PassageLine>::default());
        window
            .global::<TypingView>()
            .set_lines(ModelRc::from(lines_model.clone()));
        let bars_model = Rc::new(VecModel::from(vec![
            FlowBar {
                level: 0.0,
                smooth: false,
            };
            WAVE_BARS
        ]));
        window
            .global::<FlowView>()
            .set_bars(ModelRc::from(bars_model.clone()));
        // Thresholds are data (content/flow/flow-config.toml, schema 1).
        let flow_cfg = FlowConfig::shipped();
        let flow = Rc::new(RefCell::new(Flow {
            window: window.as_weak(),
            nav,
            repo,
            db,
            profile_id,
            cards: Vec::new(),
            course: None,
            list_failed: false,
            filter: ListFilter::default(),
            shown: Vec::new(),
            columns: DEFAULT_COLUMNS,
            scroll_y: 0.0,
            viewport_h: 0.0,
            window_range: 0..0,
            scroll_token: 0,
            preview: None,
            preview_error: None,
            drill_cache: HashMap::new(),
            studio: TimerStudio::default(),
            guide_mode: "full",
            exam: ExamInfo::default(),
            me: std::rc::Weak::new(),
            practice: None,
            exercise: None,
            wrapped: passage::Wrapped::default(),
            passage_w: 0.0,
            lines_model,
            lines_cache: Vec::new(),
            saved: false,
            ticker: slint::Timer::default(),
            rules: TypingRules::default(),
            guide: GuideMode::Full,
            passage_rows: passage::VISIBLE_LINES,
            scroll_first: None,
            word_no: Vec::new(),
            word_total: 0,
            typed_anchor: 0,
            shown_first: None,
            typed_mark: None,
            typed_wraps: 0,
            customs: crate::custom::load(&custom_path),
            custom_path,
            custom: None,
            typing_course: None,
            streak: Streak::default(),
            opened_at: Instant::now(),
            last_input: None,
            recent_error: false,
            last_hint: None,
            coach: String::new(),
            notice: String::new(),
            timer_warned: false,
            fullscreen: false,
            drill: None,
            audio: Audio::new(true),
            learn: LearnProgress::load(&learn_path),
            learn_path,
            journal_path,
            last_journal: Instant::now(),
            flow_cfg,
            rhythm: None,
            last_cursor: 0,
            esc_chip: false,
            bars_buf: [CadenceBar::default(); WAVE_BARS],
            bars_model,
            bars_shown: [(0, false); WAVE_BARS],
            flow_label: None,
            flow_live_pct: None,
            flow_scored: 0,
            pulse: false,
            flow_w: 0.0,
            flow_rows: 5,
            live_flow: rhythm::LiveFlow::new(),
            flow_strip: (-1, 0, 0, 0),
            diff: Vec::new(),
            recovery_text: None,
            minute_wpm: Vec::new(),
            minute_kdph: Vec::new(),
            scorecard: None,
            export_dir: crate::export::default_dir(config_dir),
            default_guide: "full",
            preferred_layout: LayoutId::RemingtonGail,
            typable_cache: RefCell::new(HashMap::new()),
            hidden: std::cell::Cell::new(0),
        }));
        {
            let mut f = flow.borrow_mut();
            f.me = Rc::downgrade(&flow);
            let screen = f.nav.current().clone();
            if matches!(screen, Screen::Exercise(_)) {
                f.load_course(true);
            }
            f.show(&screen);
            f.write_home_smoothness();
            f.restore_journal();
        }
        wire(window, &flow);
        flow
    }

    fn with_window(&self, f: impl FnOnce(&AppWindow)) {
        if let Some(w) = self.window.upgrade() {
            f(&w);
        }
    }

    fn show(&self, screen: &Screen) {
        self.with_window(|w| {
            apply_screen(w, screen);
            if matches!(screen, Screen::Typing(_)) {
                w.invoke_focus_keys();
            }
        });
    }

    /// The Exercise screen's draft context.
    fn draft(&self) -> Option<SessionContext> {
        match self.nav.current() {
            Screen::Exercise(ctx) => Some(ctx.clone()),
            _ => None,
        }
    }

    fn edit_draft(&mut self, f: impl FnOnce(&mut SessionContext)) {
        if let Some(ctx) = self.nav.draft_mut() {
            f(ctx);
        }
    }

    // --- Home / Exercise ------------------------------------------------------

    /// A Home card was activated.
    pub fn open_product(&mut self, product: ProductId) {
        let screen = self.nav.open_product(product);
        self.load_course(true);
        self.show(&screen);
    }

    /// An Exam Hub launcher was activated (`language`: "eng" | "hin").
    pub fn open_exam(&mut self, exam_id: &str, language: &str) {
        let Some(language) = ExamLanguage::from_id(language) else {
            log::warn!("Ignoring exam launch with unknown language '{language}'");
            return;
        };
        let Some(info) = self.exam_info(exam_id) else {
            log::warn!("Ignoring launch of unknown exam '{exam_id}'");
            return;
        };
        // A language the profile has no test for opens its other test.
        let language = if exam_mode::offers(info.profile, language) {
            language
        } else {
            match language {
                ExamLanguage::English => ExamLanguage::Hindi,
                ExamLanguage::Hindi => ExamLanguage::English,
            }
        };
        let carrier = self.exam_carrier_for(&info, language);
        let timer = exam_mode::timer(info.test(language));
        let key = info
            .profile
            .map_or_else(|| "no profile".to_owned(), |p| p.key());
        log::info!("Exam {exam_id} opened under {key}");
        self.exam = info;
        let screen = self.nav.open_exam(exam_id, carrier);
        // The exam timer is fixed by the profile (RULES 30).
        self.edit_draft(|d| d.timer = timer);
        let screen = self
            .nav
            .draft_mut()
            .map_or(screen, |d| Screen::Exercise(d.clone()));
        self.load_course(true);
        self.show(&screen);
    }

    /// The carrier product of an exam test: the card's own choice (Mangal
    /// layouts for a Hindi-only card, Kruti Dev when the card names it, else
    /// the preferred layout) unless the profile does not allow it.
    fn exam_carrier_for(&self, info: &ExamInfo, language: ExamLanguage) -> ProductId {
        let fallback = if info.hindi_only {
            // No English on this card, and only the Mangal Unicode layouts.
            hindi_only_carrier(self.preferred_layout)
        } else {
            self.preferred_carrier(exam_carrier(language, &info.font_layout))
        };
        exam_mode::carrier(info.profile, language, self.preferred_layout, fallback)
    }

    /// Take the Settings overlay's defaults (font, rules, sound, guide,
    /// keyboard, focus). A running exam keeps its locked rules.
    pub fn apply_settings(&mut self, s: &crate::settings::Settings) {
        if !self.rules.locked {
            self.rules.font_px = s.typography.typing_font_px;
            self.rules.bold = s.typography.bold;
            self.rules.paper = s.typing.paper;
            self.rules.backspace = s.backspace();
            self.rules.highlight = s.highlight();
            self.rules.focus_mode = s.focus.focus_mode;
            self.rules.word_limit = s.typing.word_limit;
            self.rules.show_scrollbar = s.typing.show_scrollbar;
            self.rules.auto_scroll = s.typing.auto_scroll;
            self.rules.key_guide = s.typing.key_guide;
        }
        self.rules.sound = s.sound.enabled;
        self.audio.set_enabled(s.sound.enabled);
        self.default_guide = s.guide_mode().slug();
        let layout = s.keyboard.layout_id();
        let changed = layout != self.preferred_layout;
        self.preferred_layout = layout;
        if self.practice.is_some() {
            self.write_rules();
        }
        // Settings → Keyboard on an open Exercise screen switches its course.
        if changed && matches!(self.nav.current(), Screen::Exercise(_)) {
            self.switch_layout(layout, false);
        }
    }

    /// A Hindi exam's carrier on the preferred layout, unless the exam card
    /// names Kruti Dev (then Kruti Dev, as the exam is typed in that font).
    fn preferred_carrier(&self, carrier: ProductId) -> ProductId {
        let pref = self.preferred_layout;
        match carrier {
            ProductId::HindiMastery(LayoutId::RemingtonGail) if pref != LayoutId::Qwerty => {
                ProductId::HindiMastery(pref)
            }
            other => other,
        }
    }

    /// Quick Resume (HOME-11): the last session's product, mode and
    /// exercise on its Exercise screen.
    pub fn resume(&mut self, product: ProductId, exercise_id: i64, mode: Option<&str>) {
        self.open_product(product);
        if let Some(m) = mode.and_then(PracticeMode::from_slug) {
            self.set_mode(m.slug());
        }
        if self.cards.iter().any(|c| c.id == exercise_id) {
            self.choose(exercise_id);
        } else {
            log::info!("Quick Resume: exercise {exercise_id} is no longer listed");
        }
        self.write_flow_nav();
    }

    /// The saved candidate (RS-10), `None` when no name was entered.
    fn candidate(&self) -> Option<(String, String)> {
        let db = self.db.try_borrow().ok()?;
        let get = |k| db.profile_value(self.profile_id, k).ok().flatten();
        let name = get(super::utility::CANDIDATE_NAME_KEY)?;
        Some((
            name,
            get(super::utility::CANDIDATE_ROLL_KEY).unwrap_or_default(),
        ))
    }

    /// Re-read the candidate into an open exam Result.
    pub fn refresh_candidate(&self) {
        let exam = self
            .scorecard
            .as_ref()
            .is_some_and(|c| c.cert != Certification::Practice);
        let text = if exam {
            candidate_line(self.candidate().as_ref())
        } else {
            String::new()
        };
        self.with_window(|w| w.global::<ResultView>().set_candidate(text.into()));
    }

    /// The minute curve in the WPM or KDPH view (RS-02).
    fn write_chart(&self, view: &str) {
        let kdph = view == "kdph";
        let values = if kdph {
            &self.minute_kdph
        } else {
            &self.minute_wpm
        };
        let chart = report::chart_path(values);
        let peak = values.iter().copied().fold(0.0_f64, f64::max);
        let (title, value) = if kdph {
            (
                "Key depressions per hour, per minute",
                format!("peak {peak:.0} KDPH"),
            )
        } else {
            (
                "Speed per minute (gross WPM)",
                format!("peak {peak:.0} WPM"),
            )
        };
        let footer = format!(
            "{} active minute{} · {} per minute, pauses excluded",
            values.len(),
            if values.len() == 1 { "" } else { "s" },
            if kdph { "KDPH" } else { "gross WPM" }
        );
        self.with_window(|w| {
            let v = w.global::<ResultView>();
            v.set_chart_view(if kdph { "kdph" } else { "wpm" }.into());
            v.set_chart_title(title.into());
            match &chart {
                Some(c) => {
                    v.set_chart_has_data(true);
                    v.set_chart_line(c.line.as_str().into());
                    v.set_chart_area(c.area.as_str().into());
                    v.set_chart_peak_x(c.peak_x);
                    v.set_chart_peak_y(c.peak_y);
                }
                None => v.set_chart_has_data(false),
            }
            v.set_chart_value(value.into());
            v.set_chart_footer(footer.into());
        });
    }

    /// Write the HTML scorecard (RS-09) to a user-chosen new file.
    fn export_scorecard(&self, path: &str) {
        let status = match (
            &self.scorecard,
            crate::export::validate_target(path, &["html", "htm"]),
        ) {
            (None, _) => "Nothing to export yet.".to_owned(),
            (_, Err(e)) => e,
            (Some(card), Ok(target)) => {
                let mut card = card.clone();
                if card.cert != Certification::Practice {
                    card.candidate = self.candidate();
                }
                match crate::export::write_new(&target, report::scorecard_html(&card).as_bytes()) {
                    Ok(()) => {
                        log::info!("Scorecard exported to {}", target.display());
                        format!("{} saved to {}", card.title(), target.display())
                    }
                    Err(e) => {
                        log::warn!("Scorecard export failed: {e}");
                        format!("Could not save: {e}")
                    }
                }
            }
        };
        self.with_window(|w| w.global::<ResultView>().set_export_status(status.into()));
    }

    fn exam_info(&self, exam_id: &str) -> Option<ExamInfo> {
        let w = self.window.upgrade()?;
        let all = w.global::<Exams>().get_all();
        let entry = all.iter().find(|e| e.id.as_str() == exam_id)?;
        let profile = exam_mode::profile(exam_id);
        if profile.is_none() {
            log::warn!("Exam card '{exam_id}' has no loaded profile: practice only");
        }
        Some(ExamInfo {
            title: entry.title.to_string(),
            font_layout: entry.font_layout.to_string(),
            hindi_only: entry.hindi_only || profile.is_some_and(|p| p.hindi_only),
            profile,
        })
    }

    /// Back one level (button or Escape off the Typing screen).
    pub fn back(&mut self) {
        let from = self.nav.current().clone();
        let Some(screen) = self.nav.back() else {
            log::debug!("Back requested at the root — ignoring");
            return;
        };
        if matches!(from, Screen::Typing(_)) {
            log::info!("Typing session abandoned; nothing saved");
        }
        self.leave_typing();
        if matches!(screen, Screen::Exercise(_)) {
            // Bests may have changed after a Result; choices are kept.
            self.load_course(false);
        }
        self.show(&screen);
    }

    fn go_home(&mut self) {
        self.leave_typing();
        let screen = self.nav.go_home();
        self.show(&screen);
    }

    fn leave_typing(&mut self) {
        self.ticker.stop();
        self.practice = None;
        self.drill = None;
        // Leaving abandons the session: nothing to recover.
        recovery::clear(&self.journal_path);
        if self.fullscreen {
            self.fullscreen = false;
            self.with_window(|w| {
                w.window().set_fullscreen(false);
                w.global::<TypingView>().set_fullscreen(false);
            });
        }
    }

    /// (Re)load the list of the draft's course. `fresh` (opened from Home)
    /// resets filters and the dock; otherwise they are kept.
    fn load_course(&mut self, fresh: bool) {
        let Some(ctx) = self.draft() else {
            return;
        };
        let key = course_key(&ctx);
        let same = self.course.as_deref() == Some(key.as_str());
        let (cards, failed) = self.fetch_cards(&ctx);
        self.cards = cards;
        self.list_failed = failed;
        self.course = Some(key);
        if fresh || !same {
            self.filter = ListFilter::default();
            self.scroll_y = 0.0;
            self.scroll_token = self.scroll_token.wrapping_add(1);
        }
        if fresh {
            self.studio = TimerStudio::default();
            self.clear_custom_field();
            // The strict rule shows the saved Backspace rule.
            let strict = !ctx.is_exam() && self.rules.backspace == BackspacePolicy::Off;
            self.edit_draft(|d| d.no_backspace = strict);
            self.guide_mode = self.default_guide;
            self.drill_cache.clear();
        }
        self.filter.track = ctx.mode.track();
        self.write_header();
        self.refilter();
        self.write_dock();
    }

    fn fetch_cards(&self, ctx: &SessionContext) -> (Vec<CardMeta>, bool) {
        if ctx.is_exam() {
            let pool = ExamLanguage::of_layout(ctx.product.layout()).pool();
            let inscript = ctx.product.layout() == LayoutId::Inscript;
            self.hidden.set(0);
            return match self.repo.list_exam_pool(pool) {
                Ok(list) => (
                    list.into_iter()
                        .filter(|p| !inscript || self.typable(LayoutId::Inscript, true, p.id))
                        .map(|p| CardMeta {
                            id: p.id,
                            number: p.number,
                            title: p.title,
                            category: p.category,
                            difficulty: p.difficulty,
                            status: p.status,
                            track: None,
                            word_count: Some(p.word_count),
                            best: None,
                            completed: false,
                            flow: None,
                        })
                        .collect(),
                    false,
                ),
                Err(e) => {
                    log::error!("Could not list the {} exam pool: {e}", pool.slug());
                    (Vec::new(), true)
                }
            };
        }
        let product = ctx.product;
        // InScript has no course of its own: the Unicode Hindi course, only
        // the passages InScript can type key for key.
        let source = product.content_source();
        let list = match self.repo.list_meta(source) {
            Ok(list) => list,
            Err(e) => {
                log::error!("Could not list exercises of {}: {e}", source.slug());
                return (Vec::new(), true);
            }
        };
        // Unicode Hindi courses list only passages their layout types (the
        // CBI course predates book 03, which has no key for ॐ or ॰).
        let total = list.len();
        let check = source != product
            || radha_content::import::hides_untypeable(radha_content::ContentTarget::Product(
                product,
            ));
        let list: Vec<_> = if check {
            list.into_iter()
                .filter(|m| self.typable(product.layout(), false, m.id))
                .collect()
        } else {
            list
        };
        self.hidden.set(total - list.len());
        let progress: HashMap<i64, radha_db::CardProgress> =
            match self.repo.card_progress(self.profile_id, product) {
                Ok(rows) => rows.into_iter().map(|p| (p.exercise_id, p)).collect(),
                Err(e) => {
                    // Bests are decoration; the list still works without them.
                    log::warn!("Could not read progress of {}: {e}", product.slug());
                    HashMap::new()
                }
            };
        // Typing Flow: each exercise's last and best smoothness (one query).
        let flow: HashMap<i64, (f64, f64)> = if product == ProductId::TypingFlow {
            self.flow_history()
        } else {
            HashMap::new()
        };
        let cards = list
            .into_iter()
            .map(|m| {
                let p = progress.get(&m.id);
                CardMeta {
                    best: p.and_then(|p| p.best_net_wpm.zip(p.best_accuracy)),
                    completed: p.is_some_and(|p| p.completed),
                    flow: flow.get(&m.id).copied(),
                    id: m.id,
                    number: m.number,
                    title: m.title,
                    category: m.category,
                    difficulty: m.difficulty,
                    status: m.status,
                    track: m.track,
                    word_count: m.word_count,
                }
            })
            .collect();
        (cards, false)
    }

    fn write_header(&self) {
        let Some(ctx) = self.draft() else {
            return;
        };
        let product = ctx.product;
        let layout = product.layout();
        let track_cards: Vec<CardMeta> = self
            .cards
            .iter()
            .filter(|c| {
                ctx.mode
                    .track()
                    .is_none_or(|t| c.track.as_deref() == Some(t))
            })
            .cloned()
            .collect();
        let count = track_cards.len();
        let summary = status_summary(&track_cards);
        let (name, crumb, mode_options, mode_current, layout_options, banner, level) = if ctx
            .is_exam()
        {
            let language = ExamLanguage::of_layout(layout);
            let lang_name = match language {
                ExamLanguage::English => "English",
                ExamLanguage::Hindi => "Hindi",
            };
            let layouts = if language == ExamLanguage::Hindi {
                HINDI_LAYOUTS
                    .iter()
                    .filter(|(_, _, l)| self.exam.allows(*l))
                    .map(|(id, label, _)| segment(id, label))
                    .collect()
            } else {
                Vec::new()
            };
            // The languages the profile has a test for (Hindi only on the
            // All-in-One Mangal card).
            let languages: Vec<SegmentOption> = [
                (ExamLanguage::English, "eng", "English"),
                (ExamLanguage::Hindi, "hin", "Hindi"),
            ]
            .into_iter()
            .filter(|(l, _, _)| {
                !(self.exam.hindi_only && *l == ExamLanguage::English)
                    && exam_mode::offers(self.exam.profile, *l)
            })
            .map(|(_, id, label)| segment(id, label))
            .collect();
            let (banner, level) = exam_mode::banner(self.exam.profile, language, &self.exam.title);
            (
                self.exam.title.clone(),
                format!("Home / Exam Hub / {lang_name} test"),
                languages,
                language.id().to_owned(),
                layouts,
                banner,
                level,
            )
        } else {
            let matrix = mode_matrix(product);
            let options = matrix
                .toggle
                .map(|opts| opts.iter().map(|m| segment(m.slug(), m.label())).collect())
                .unwrap_or_default();
            let any = |s| track_cards.iter().any(|c| c.status == s);
            let (banner, level) = if product.content_source() != product {
                (
                    format!(
                        "{} has no course of its own. These are the {count} {} passages \
                         that {} types key for key; the rest are left out, not altered.",
                        layout.display_name(),
                        product.content_source().display_name(),
                        layout.display_name()
                    ),
                    "provisional",
                )
            } else if any(ContentStatus::Provisional) {
                (
                    "Provisional content: imported without a completed review. It is \
                         labelled on every card until reviewed."
                        .to_owned(),
                    "provisional",
                )
            } else if any(ContentStatus::GeneratedReviewPending) {
                (
                    "Generated content awaiting human review. It is never shown as verified."
                        .to_owned(),
                    "generated",
                )
            } else {
                (String::new(), "")
            };
            let hidden = self.hidden.get();
            let (banner, level) = if hidden > 0 && product.content_source() == product {
                let note = format!(
                    "{hidden} passages use characters {} has no key for (its verified \
                     keyboard book), so they are not listed.",
                    layout.display_name()
                );
                let joined = if banner.is_empty() {
                    note
                } else {
                    format!("{banner} {note}")
                };
                (
                    joined,
                    if level.is_empty() {
                        "provisional"
                    } else {
                        level
                    },
                )
            } else {
                (banner, level)
            };
            let layouts = if layout_switchable(product) {
                MASTERY_LAYOUTS
                    .iter()
                    .map(|(id, label, _)| segment(id, label))
                    .collect()
            } else {
                Vec::new()
            };
            (
                product.display_name().to_owned(),
                format!("Home / {} / {}", product.display_name(), ctx.mode.label()),
                options,
                ctx.mode.slug().to_owned(),
                layouts,
                banner,
                level,
            )
        };
        let count_text = match (count, ctx.is_exam()) {
            (0, _) => "nothing installed yet".to_owned(),
            (n, true) => format!("{n} shared passages · {summary}"),
            (n, false) => format!("{n} exercises · {summary}"),
        };
        let accent_token = if ctx.is_exam() {
            "secondary"
        } else {
            accent(product)
        };
        let layout_current = if ctx.is_exam() {
            layout_slug(layout)
        } else {
            layout.as_str()
        };
        self.with_window(|w| {
            let v = w.global::<ExerciseView>();
            v.set_product_name(name.into());
            v.set_layout_name(layout.display_name().into());
            v.set_breadcrumb(crumb.into());
            v.set_accent(accent_token.into());
            v.set_mode_options(model(mode_options));
            v.set_mode_current(mode_current.into());
            v.set_layout_options(model(layout_options));
            v.set_layout_current(layout_current.into());
            v.set_count_text(count_text.into());
            v.set_banner(banner.into());
            v.set_banner_level(level.into());
            v.set_hindi(layout != LayoutId::Qwerty);
        });
    }

    /// Re-apply the filters; keep the selection when it is still in the
    /// track, else select the first shown card.
    fn refilter(&mut self) {
        self.shown = filter_cards(&self.cards, &self.filter);
        let selected = self.draft().and_then(|c| c.exercise_id);
        let keep = selected.filter(|id| {
            self.cards.iter().any(|c| {
                c.id == *id
                    && self
                        .filter
                        .track
                        .is_none_or(|t| c.track.as_deref() == Some(t))
            })
        });
        self.write_toolbar();
        match keep {
            Some(id) if self.preview.as_ref().is_some_and(|p| p.id == id) => {
                self.write_grid();
                self.write_cta();
            }
            Some(id) => self.choose(id),
            None => match self.shown.first().and_then(|&i| self.cards.get(i)) {
                Some(c) => {
                    let id = c.id;
                    self.choose(id);
                }
                None => {
                    self.edit_draft(|d| {
                        d.exercise_id = None;
                        d.exercise_number = 0;
                    });
                    self.preview = None;
                    self.preview_error = None;
                    self.write_grid();
                    self.write_inspector();
                    self.write_cta();
                }
            },
        }
    }

    fn write_toolbar(&self) {
        let track = self.filter.track;
        let in_track = self
            .cards
            .iter()
            .filter(|c| track.is_none_or(|t| c.track.as_deref() == Some(t)))
            .count();
        let categories: Vec<FilterPillItem> = category_counts(&self.cards, track)
            .into_iter()
            .map(|(name, n)| FilterPillItem {
                id: name.as_str().into(),
                label: name.as_str().into(),
                count_text: n.to_string().into(),
            })
            .collect();
        let shown = format!("Shown {} of {in_track}", self.shown.len());
        let difficulty = self.filter.difficulty.slug();
        let category = self.filter.category.clone().unwrap_or_else(|| "all".into());
        let query = self.filter.query.clone();
        self.with_window(|w| {
            let v = w.global::<ExerciseView>();
            v.set_categories(model(categories));
            v.set_shown_text(shown.into());
            v.set_difficulty(difficulty.into());
            v.set_category(category.into());
            if v.get_search_text().as_str() != query {
                v.set_search_text(query.into());
            }
        });
    }

    fn card_model(
        &self,
        pos: usize,
        selected: Option<i64>,
        layout: LayoutId,
    ) -> Option<ExerciseCard> {
        let c = self.cards.get(*self.shown.get(pos)?)?;
        let targets = suggested_targets(layout, c.difficulty);
        let (status, tone) = status_label(c.status);
        Some(ExerciseCard {
            index: i32::try_from(pos).unwrap_or(i32::MAX),
            code: card_code(c.number).into(),
            title: c.title.as_str().into(),
            category: c.category.clone().unwrap_or_default().into(),
            level: c.difficulty.map_or(0, i32::from),
            score: difficulty_score(c.difficulty)
                .map(|s| format!("{s}/10"))
                .unwrap_or_default()
                .into(),
            targets: format!("{} WPM · {}%", targets.wpm, targets.accuracy).into(),
            length: c
                .word_count
                .map(|w| format!("≈ {w} words"))
                .unwrap_or_default()
                .into(),
            best: best_text(c.best).into(),
            status: status.into(),
            status_tone: tone.into(),
            completed: c.completed,
            selected: selected == Some(c.id),
            rhythm: rhythm::card_rhythm(c.flow).into(),
        })
    }

    /// Whether the current list (track) has no cards at all, before filters.
    fn list_empty(&self) -> bool {
        let track = self.filter.track;
        !self
            .cards
            .iter()
            .any(|c| track.is_none_or(|t| c.track.as_deref() == Some(t)))
    }

    /// Send the visible window of the grid (and the list state) to Slint.
    fn write_grid(&mut self) {
        let total = self.shown.len();
        let range = if self.viewport_h > 0.0 {
            visible_window(
                total,
                self.columns,
                ROW_PITCH,
                self.scroll_y,
                self.viewport_h,
            )
        } else {
            initial_window(total, self.columns)
        };
        self.window_range = range.clone();
        let ctx = self.draft();
        let selected = ctx.as_ref().and_then(|c| c.exercise_id);
        let layout = ctx
            .as_ref()
            .map_or(LayoutId::Qwerty, |c| c.product.layout());
        let cards: Vec<ExerciseCard> = range
            .filter_map(|pos| self.card_model(pos, selected, layout))
            .collect();
        let rows = i32::try_from(grid_rows(total, self.columns)).unwrap_or(i32::MAX);
        let (state, title, message) = if self.list_failed {
            (
                "error",
                "Could not read the exercise list".to_owned(),
                "The content database could not be read. Your progress is safe; restart Radha, \
                 and if this repeats, check the log file."
                    .to_owned(),
            )
        } else if self.list_empty() {
            let product = ctx.as_ref().filter(|c| !c.is_exam()).map(|c| c.product);
            let (t, m) = empty_message(product);
            ("empty", t.to_owned(), m)
        } else if total == 0 {
            (
                "no-match",
                "No exercises match".to_owned(),
                "Clear the search or choose another difficulty or category.".to_owned(),
            )
        } else {
            ("ready", String::new(), String::new())
        };
        let token = self.scroll_token;
        self.with_window(|w| {
            let v = w.global::<ExerciseView>();
            v.set_cards(model(cards));
            v.set_total_rows(rows);
            v.set_card_height(CARD_HEIGHT);
            v.set_row_pitch(ROW_PITCH);
            v.set_list_state(state.into());
            v.set_state_title(title.into());
            v.set_state_message(message.into());
            if v.get_scroll_token() != token {
                v.set_scroll_token(token);
            }
        });
    }

    /// The grid reported its geometry (scroll, resize, column change).
    fn grid_scrolled(&mut self, columns: usize, viewport_h: f32, scroll_y: f32) {
        let columns = columns.max(1);
        let changed_columns = columns != self.columns;
        self.columns = columns;
        self.viewport_h = viewport_h;
        self.scroll_y = scroll_y;
        let range = visible_window(self.shown.len(), columns, ROW_PITCH, scroll_y, viewport_h);
        if changed_columns || range != self.window_range {
            self.write_grid();
        }
    }

    /// A card was activated (its filtered position).
    fn select(&mut self, pos: usize) {
        let Some(id) = self
            .shown
            .get(pos)
            .and_then(|&i| self.cards.get(i))
            .map(|c| c.id)
        else {
            return;
        };
        self.choose(id);
    }

    /// Select exercise `id`: update the draft, load its text, redraw.
    fn choose(&mut self, id: i64) {
        let Some(meta) = self.cards.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        self.edit_draft(|d| {
            d.exercise_id = Some(meta.id);
            d.exercise_number = meta.number;
        });
        let Some(ctx) = self.draft() else {
            return;
        };
        match self.load_text(&ctx, &meta) {
            Ok(e) => {
                self.preview = Some(e);
                self.preview_error = None;
            }
            Err(e) => {
                log::error!("Could not load exercise {}: {e}", meta.id);
                self.preview = None;
                self.preview_error = Some(format!(
                    "{} could not be loaded. Choose another exercise; nothing was lost.",
                    card_code(meta.number)
                ));
            }
        }
        self.write_grid();
        self.write_inspector();
        self.write_cta();
    }

    /// The full text of a card, for the draft's product or exam layout.
    fn load_text(&self, ctx: &SessionContext, meta: &CardMeta) -> anyhow::Result<Exercise> {
        if ctx.is_exam() {
            let kruti = ctx.product.layout() == LayoutId::KrutiDev;
            let content = self
                .repo
                .load_exam_passage(meta.id, kruti)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            return Ok(Exercise {
                id: meta.id,
                product: ctx.product,
                number: meta.number,
                title: meta.title.clone(),
                category: meta.category.clone(),
                difficulty: meta.difficulty,
                content,
            });
        }
        let mut e = self
            .repo
            .load(meta.id)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        // A borrowed passage (InScript) is typed and saved as the draft's
        // own product.
        e.product = ctx.product;
        Ok(e)
    }

    fn write_inspector(&mut self) {
        let Some(ctx) = self.draft() else {
            return;
        };
        let error = self.preview_error.clone().unwrap_or_default();
        let Some(e) = self.preview.clone() else {
            self.with_window(|w| {
                let v = w.global::<ExerciseView>();
                v.set_insp_visible(false);
                v.set_insp_error(error.into());
            });
            return;
        };
        let layout = ctx.product.layout();
        let meta = self.cards.iter().find(|c| c.id == e.id).cloned();
        let status = meta
            .as_ref()
            .map_or(ContentStatus::Provisional, |m| m.status);
        let targets = suggested_targets(layout, e.difficulty);
        let words = meta
            .as_ref()
            .and_then(|m| m.word_count)
            .unwrap_or_else(|| u32::try_from(e.content.split_whitespace().count()).unwrap_or(0));
        let drill = self
            .drill_cache
            .entry((e.id, layout))
            .or_insert_with(|| drill_profile(layout, &e.content))
            .clone();
        let best = if ctx.is_exam() {
            "Exam practice is not saved yet".to_owned()
        } else {
            match self.repo.exercise_best(self.profile_id, e.id) {
                Ok(Some(b)) => format!("Best {:.0} WPM · {:.0}%", b.net_wpm, b.accuracy),
                Ok(None) => "Not practised yet".to_owned(),
                Err(err) => {
                    log::warn!("Could not read the best of exercise {}: {err}", e.id);
                    String::new()
                }
            }
        };
        let (status_text, tone) = status_label(status);
        let level = match (e.difficulty, difficulty_score(e.difficulty)) {
            (Some(d), Some(s)) => format!("Level {d} · {s}/10"),
            _ => String::new(),
        };
        let title = e.title.clone();
        self.with_window(|w| {
            let v = w.global::<ExerciseView>();
            v.set_insp_visible(true);
            v.set_insp_error(SharedString::new());
            v.set_insp_code(card_code(e.number).into());
            v.set_insp_category(e.category.clone().unwrap_or_default().to_uppercase().into());
            v.set_insp_level(level.into());
            v.set_insp_title(title.into());
            v.set_insp_status(status_text.into());
            v.set_insp_status_tone(tone.into());
            v.set_insp_note(coaching_note(ctx.mode, targets).into());
            v.set_insp_keys(model(
                drill
                    .keys
                    .iter()
                    .map(|k| SharedString::from(k.as_str()))
                    .collect(),
            ));
            v.set_insp_target_wpm(format!("{} WPM", targets.wpm).into());
            v.set_insp_min_accuracy(format!("{}%", targets.accuracy).into());
            v.set_insp_length(
                format!("{words} w · ~{} min", estimated_minutes(words, targets.wpm)).into(),
            );
            v.set_insp_fingers(model(
                drill
                    .fingers
                    .iter()
                    .map(|f| SharedString::from(f.as_str()))
                    .collect(),
            ));
            v.set_insp_best(best.into());
        });
    }

    fn write_dock(&self) {
        let Some(ctx) = self.draft() else {
            return;
        };
        let dock = match ctx.mode.dock() {
            Dock::TimerStudio => "timer",
            Dock::Guide => "guide",
            Dock::LockedTimer => "locked",
        };
        let active = self.studio.active_preset();
        let chips: Vec<TimerChoice> = TimerStudio::preset_labels()
            .into_iter()
            .enumerate()
            .map(|(i, label)| TimerChoice {
                label: label.into(),
                selected: active == Some(i),
            })
            .collect();
        let timer = ctx.timer;
        let kind = self.studio.kind().slug();
        let error = self.studio.error().unwrap_or("");
        let locked = if ctx.is_exam() {
            let language = ExamLanguage::of_layout(ctx.product.layout());
            format!(
                "Set by the exam profile; you cannot change it: {}.",
                exam_mode::timer_note(self.exam.test(language))
            )
        } else {
            String::new()
        };
        let guide = self.guide_mode;
        let key_guide = self.rules.key_guide;
        self.with_window(|w| {
            let v = w.global::<ExerciseView>();
            v.set_dock(dock.into());
            v.set_timer_readout(readout(timer).into());
            v.set_timer_sentence(timer_sentence(timer).into());
            v.set_timer_kind(kind.into());
            v.set_timers(model(chips));
            v.set_custom_timer_error(error.into());
            v.set_no_backspace(ctx.no_backspace);
            v.set_locked_reason(locked.into());
            v.set_guide_mode(guide.into());
            v.set_key_guide(key_guide);
            v.set_key_mode(ctx.display == DisplayMode::Key);
        });
        self.write_learn_path();
        self.write_cta();
    }

    /// The Hindi Learn beginner path in the Guide dock (HL-01, HL-07).
    fn write_learn_path(&self) {
        let Some(ctx) = self.draft() else {
            return;
        };
        let layout = ctx.product.layout();
        let steps = if ctx.mode == PracticeMode::Beginner {
            hindi_learn::steps(layout)
        } else {
            Vec::new()
        };
        let progress = self.learn.layout(layout);
        let chips: Vec<LearnChip> = learn_chips(&steps, &progress, None);
        let (status, plan) = if steps.is_empty() {
            (String::new(), String::new())
        } else {
            let today: String = iso_utc(std::time::SystemTime::now())
                .chars()
                .take(10)
                .collect();
            (
                format!(
                    "Step {} of {} open · unlock the next with {:.0}% accuracy over {}+ characters · guide ramp: {} · generated drills, review pending",
                    progress.unlocked.min(steps.len() - 1) + 1,
                    steps.len(),
                    hindi_learn::UNLOCK_ACCURACY,
                    hindi_learn::UNLOCK_MIN_CHARS,
                    progress.guide_mode().slug()
                ),
                progress.daily_plan(&steps, &today).summary(),
            )
        };
        self.with_window(|w| {
            let v = w.global::<ExerciseView>();
            v.set_learn_steps(model(chips));
            v.set_learn_status(status.into());
            v.set_learn_plan(plan.into());
        });
    }

    fn write_cta(&self) {
        let Some(ctx) = self.draft() else {
            return;
        };
        let ready = self
            .preview
            .as_ref()
            .is_some_and(|p| Some(p.id) == ctx.exercise_id);
        let reason = if self.list_failed {
            Some("The exercise list could not be read")
        } else {
            start_block_reason(self.list_empty(), ready, ctx.mode, self.studio.is_valid())
        };
        let text = if ctx.product == ProductId::TypingFlow && !ctx.is_exam() {
            rhythm::begin_label(ctx.timer)
        } else {
            start_label(ctx.mode, ctx.timer)
        };
        self.with_window(|w| {
            let v = w.global::<ExerciseView>();
            v.set_start_text(text.into());
            v.set_can_start(reason.is_none());
            v.set_start_reason(reason.unwrap_or("").into());
        });
        self.write_flow_nav();
    }

    fn set_mode(&mut self, id: &str) {
        let Some(ctx) = self.draft() else {
            return;
        };
        if ctx.is_exam() {
            let Some(language) = ExamLanguage::from_id(id) else {
                return;
            };
            if language == ExamLanguage::of_layout(ctx.product.layout())
                || (self.exam.hindi_only && language == ExamLanguage::English)
                || !exam_mode::offers(self.exam.profile, language)
            {
                return;
            }
            let carrier = self.exam_carrier_for(&self.exam, language);
            // Each language test has its own fixed timer.
            let timer = exam_mode::timer(self.exam.test(language));
            self.edit_draft(|d| {
                d.product = carrier;
                d.exercise_id = None;
                d.exercise_number = 0;
                d.timer = timer;
            });
            self.preview = None;
            self.load_course(false);
            return;
        }
        let Some(mode) = PracticeMode::from_slug(id) else {
            return;
        };
        if !mode_matrix(ctx.product).allows(mode) || mode == ctx.mode {
            return;
        }
        let studio = self.studio.mode();
        self.edit_draft(|d| {
            d.mode = mode;
            d.timer = timer_for(mode, studio);
        });
        self.filter.track = mode.track();
        if self.filter.track != ctx.mode.track() {
            // A different Hindi Learn track: its own categories.
            self.filter.category = None;
            self.scroll_y = 0.0;
            self.scroll_token = self.scroll_token.wrapping_add(1);
        }
        self.write_header();
        self.refilter();
        self.write_dock();
        // The coaching note follows the mode.
        self.write_inspector();
    }

    /// The Exercise screen's layout switch; the choice becomes the preferred
    /// layout (Settings → Keyboard).
    fn set_layout(&mut self, id: &str) {
        let Ok(layout) = LayoutId::parse(id) else {
            return;
        };
        if self.switch_layout(layout, true) {
            self.preferred_layout = layout;
        }
    }

    /// Type the draft's course on `layout`: an exam keeps its pool, a
    /// typing-mastery course loads that layout's content (QWERTY English,
    /// each Hindi layout its own; InScript the Unicode Hindi passages it can
    /// type). Lists are lazy and cached, so the switch does not block.
    /// Returns whether anything changed.
    fn switch_layout(&mut self, layout: LayoutId, persist: bool) -> bool {
        let Some(ctx) = self.draft() else {
            return false;
        };
        let from = ctx.product.layout();
        if from == layout {
            return false;
        }
        if ctx.is_exam() {
            if layout == LayoutId::Qwerty || from == LayoutId::Qwerty {
                return false; // the language toggle switches English/Hindi
            }
            if !self.exam.allows(layout) {
                // All-in-One Mangal: InScript, GAIL, CBI only; a profile
                // with a known layout (Patna HC Hindi: GAIL) refuses others.
                return false;
            }
            self.edit_draft(|d| d.product = ProductId::HindiMastery(layout));
            if layout == LayoutId::Inscript || from == LayoutId::Inscript {
                // The InScript list is a filtered view of the pool.
                self.load_course(false);
            } else {
                self.write_header();
            }
            // Kruti Dev types the generated legacy stream: reload the text.
            if let Some(id) = self.draft().and_then(|d| d.exercise_id) {
                if self.cards.iter().any(|c| c.id == id) {
                    self.choose(id);
                }
            }
        } else {
            if !layout_switchable(ctx.product) {
                return false;
            }
            let product = mastery_product(layout);
            let matrix = mode_matrix(product);
            let studio = self.studio.mode();
            self.edit_draft(|d| {
                d.product = product;
                d.exercise_id = None;
                d.exercise_number = 0;
                if !matrix.allows(d.mode) {
                    d.mode = matrix.default;
                    d.timer = timer_for(d.mode, studio);
                }
            });
            self.preview = None;
            self.load_course(false);
        }
        if persist {
            let id = layout.as_str();
            self.with_window(|w| {
                w.global::<UtilityView>()
                    .invoke_set_setting("layout".into(), id.into());
            });
        }
        true
    }

    /// Whether passage `id` (an exam pool passage when `exam`) can be typed
    /// on `layout` (InScript: key by key); checked once, then cached.
    fn typable(&self, layout: LayoutId, exam: bool, id: i64) -> bool {
        if let Some(&ok) = self.typable_cache.borrow().get(&(layout, exam, id)) {
            return ok;
        }
        let text = if exam {
            self.repo.load_exam_passage(id, false).ok()
        } else {
            self.repo.load(id).ok().map(|e| e.content)
        };
        let ok = text.is_some_and(|t| {
            if layout == LayoutId::Inscript {
                radha_layouts::inscript::can_type(&t)
            } else {
                radha_content::import::typable_on(layout, &t)
            }
        });
        self.typable_cache
            .borrow_mut()
            .insert((layout, exam, id), ok);
        ok
    }

    fn search(&mut self, text: &str) {
        if self.filter.query == text {
            return;
        }
        self.filter.query = text.to_owned();
        self.scroll_y = 0.0;
        self.scroll_token = self.scroll_token.wrapping_add(1);
        self.refilter();
    }

    fn set_difficulty(&mut self, id: &str) {
        if let Some(band) = DifficultyBand::from_slug(id) {
            self.filter.difficulty = band;
            self.scroll_y = 0.0;
            self.scroll_token = self.scroll_token.wrapping_add(1);
            self.refilter();
        }
    }

    fn set_category(&mut self, id: &str) {
        self.filter.category = (id != "all" && !id.is_empty()).then(|| id.to_owned());
        self.scroll_y = 0.0;
        self.scroll_token = self.scroll_token.wrapping_add(1);
        self.refilter();
    }

    /// Apply the studio to the draft (user-timed modes only).
    fn sync_timer(&mut self) {
        let studio = self.studio.mode();
        self.edit_draft(|d| {
            if d.mode.timer_policy() == TimerPolicy::UserSet {
                d.timer = studio;
            }
        });
        self.write_dock();
    }

    /// A preset, the stepper or the kind replaced the custom entry: empty
    /// the field so it never shows a value that is not in force.
    fn clear_custom_field(&self) {
        self.with_window(|w| {
            w.global::<ExerciseView>()
                .set_custom_timer_text(SharedString::new());
        });
    }

    fn choose_timer(&mut self, index: usize) {
        if self.studio.choose_preset(index) {
            self.clear_custom_field();
            self.sync_timer();
        }
    }

    fn set_timer_kind(&mut self, id: &str) {
        if let Some(kind) = TimerKind::from_slug(id) {
            self.studio.set_kind(kind);
            self.clear_custom_field();
            self.sync_timer();
        }
    }

    /// The stepper: the UI sends only a direction; the step size is Rust's.
    fn step_timer(&mut self, direction: i64) {
        self.studio.step(direction.signum() * STEP_SECONDS);
        self.clear_custom_field();
        self.sync_timer();
    }

    /// The custom field, on every edit as well as Enter / Set: what the
    /// field shows is the timer START uses (minutes, seconds or MM:SS).
    fn custom_timer(&mut self, text: &str) {
        if text.trim().is_empty() {
            self.studio.clear_custom();
        } else if self.studio.set_custom(text).is_err() {
            log::debug!("Rejected custom timer {text:?}");
        }
        self.sync_timer();
    }

    /// The Exercise screen's strict rule is the Typing screen's Backspace
    /// rule: both show and save the same choice.
    fn toggle_no_backspace(&mut self) {
        self.edit_draft(|d| d.no_backspace = !d.no_backspace);
        let strict = self.draft().is_some_and(|d| d.no_backspace);
        if !self.rules.locked {
            self.rules.backspace = if strict {
                BackspacePolicy::Off
            } else {
                BackspacePolicy::Full
            };
            self.persist("backspace", self.rules.backspace.slug());
        }
        self.write_dock();
    }

    /// Rules → Key Guide, from the Exercise or the Typing screen.
    fn toggle_key_guide(&mut self) {
        self.rules.key_guide = !self.rules.key_guide;
        self.persist("key-guide", if self.rules.key_guide { "on" } else { "off" });
        if matches!(self.nav.current(), Screen::Exercise(_)) {
            self.write_dock();
        }
    }

    /// Save one preference to settings.toml (Settings overlay ids).
    fn persist(&self, key: &str, value: &str) {
        self.with_window(|w| {
            w.global::<UtilityView>()
                .invoke_set_setting(key.into(), value.into());
        });
    }

    fn set_guide_mode(&mut self, id: &str) {
        if let Some(mode) = ["full", "smart", "minimal", "off"]
            .into_iter()
            .find(|m| *m == id)
        {
            self.guide_mode = mode;
            // The next session takes the dock's choice.
            self.typing_course = None;
            self.write_dock();
        }
    }

    fn set_key_mode(&mut self, key: bool) {
        self.edit_draft(|d| {
            d.display = if key {
                DisplayMode::Key
            } else {
                DisplayMode::Word
            };
        });
        self.write_dock();
    }

    /// Start typing the selected exercise.
    pub fn start(&mut self) {
        let Some(ctx) = self.draft() else {
            return;
        };
        if ctx.mode.timer_policy() == TimerPolicy::UserSet && !self.studio.is_valid() {
            return;
        }
        let Some(exercise) = self
            .preview
            .clone()
            .filter(|p| Some(p.id) == ctx.exercise_id)
        else {
            return;
        };
        self.start_exercise(exercise, ctx, None);
    }

    /// Start (or switch to) `exercise`. From the Exercise or Result screen
    /// this starts a session; on the Typing screen it switches to it.
    fn start_exercise(&mut self, exercise: Exercise, base: SessionContext, drill: Option<Drill>) {
        let ctx = SessionContext {
            exercise_id: Some(exercise.id),
            exercise_number: exercise.number,
            ..base
        };
        // Rules → Paragraph settings: the word limit cuts the passage at a
        // word boundary (practice only). An exam types the profile's word
        // count when one is known (Patna HC: 400 / 300), else the whole
        // passage; drills keep their text.
        let limit = if ctx.is_exam() {
            exam_mode::word_limit(
                self.exam
                    .test(ExamLanguage::of_layout(ctx.product.layout())),
            )
        } else if drill.is_some() {
            0
        } else {
            self.rules.word_limit
        };
        let text = passage::truncate_words(&exercise.content, limit);
        let practice = match Practice::new(ctx.product.layout(), text, ctx.timer, ctx.display) {
            Ok(p) => p,
            Err(e) => {
                log::error!("Layout unavailable: {e}");
                return;
            }
        };
        let screen = if matches!(self.nav.current(), Screen::Typing(_)) {
            self.nav.switch_session(ctx)
        } else {
            self.nav.start_session(ctx)
        };
        let Some(screen) = screen else {
            return;
        };
        self.ticker.stop();
        recovery::clear(&self.journal_path);
        self.practice = Some(practice);
        if exercise.id != 0 || drill.is_some() {
            self.custom = None;
        }
        self.exercise = Some(exercise);
        self.drill = drill;
        self.begin_typing();
        self.show(&screen);
    }

    /// A generated beginner drill (Exercise screen path or Typing chips).
    fn start_drill(&mut self, drill: Drill) {
        let Some(base) = self.nav.current().context().cloned() else {
            return;
        };
        let layout = base.product.layout();
        let progress = self.learn.layout(layout);
        let steps = hindi_learn::steps(layout);
        let (number, title, content) = match drill {
            Drill::Step(i) => {
                let Some(s) = steps.get(i) else {
                    return;
                };
                if i > progress.unlocked {
                    log::debug!("Step {} is locked", i + 1);
                    return;
                }
                let weak = progress.weak_units(3);
                (
                    u32::try_from(i + 1).unwrap_or(1),
                    format!("Step {} · {} = {}", i + 1, s.legend, s.glyph),
                    hindi_learn::step_drill(layout, i, &weak),
                )
            }
            Drill::Matra => (
                0,
                "Matra trainer".to_owned(),
                hindi_learn::matra_drill(layout),
            ),
            Drill::Half => (
                0,
                "Half-letter trainer".to_owned(),
                hindi_learn::half_letter_drill(layout),
            ),
            Drill::Recovery => {
                let Some(text) = self.recovery_text.clone() else {
                    return;
                };
                (0, "Recovery drill".to_owned(), text)
            }
        };
        if content.trim().is_empty() {
            log::warn!("No drill text for {drill:?} on {layout:?}");
            return;
        }
        let ctx = SessionContext {
            mode: if drill == Drill::Recovery {
                base.mode
            } else {
                PracticeMode::Beginner
            },
            timer: if drill == Drill::Recovery {
                base.timer
            } else {
                TimerMode::Off
            },
            ..base
        };
        let exercise = Exercise {
            id: 0,
            product: ctx.product,
            number,
            title,
            category: Some("Generated drill".to_owned()),
            difficulty: Some(1),
            content,
        };
        self.start_exercise(exercise, ctx, Some(drill));
    }

    fn start_trainer(&mut self, id: &str) {
        match id {
            "matra" => self.start_drill(Drill::Matra),
            "half" => self.start_drill(Drill::Half),
            _ => log::debug!("Unknown trainer {id:?}"),
        }
    }

    /// Reopen a session the app was stopped in (TY-14): replay its key log
    /// and show it paused.
    fn restore_journal(&mut self) {
        let Some(j) = Journal::load(&self.journal_path) else {
            return;
        };
        let restored = (|| {
            let product = ProductId::from_slug(&j.product).ok()?;
            let stored = self.repo.load(j.exercise_id).ok()?;
            let stored_product = stored.product;
            let Some(exercise) = journaled_exercise(product, stored, |e| {
                self.typable(product.layout(), false, e.id)
            }) else {
                log::warn!(
                    "Session journal names {} but exercise {} is {}",
                    product.slug(),
                    j.exercise_id,
                    stored_product.slug()
                );
                return None;
            };
            let mode = PracticeMode::from_slug(&j.mode)?;
            Some((product, exercise, mode))
        })();
        let Some((product, exercise, mode)) = restored else {
            log::warn!("Session journal does not match any exercise; discarded");
            recovery::clear(&self.journal_path);
            return;
        };
        self.nav.open_product(product);
        self.load_course(true);
        self.edit_draft(|d| {
            d.mode = mode;
            d.timer = j.timer_mode();
            d.display = j.display();
        });
        let Some(ctx) = self.draft() else {
            return;
        };
        self.start_exercise(exercise, ctx, None);
        let now = Instant::now();
        if let Some(p) = self.practice.as_mut() {
            let keys = recovery::decode(&j.keys);
            // Typing Flow: the journalled keys feed the rhythm engine too,
            // at their journalled times, so the recovered result scores
            // the whole session (not only the keys after recovery).
            match self.rhythm.as_mut() {
                Some(t) => {
                    let (live, word_no) = (&mut self.live_flow, &self.word_no);
                    let mut feed = |r: &Replayed| replay_into_rhythm(t, live, word_no, r);
                    p.replay_timed(&keys, &j.key_ms, j.elapsed_seconds, now, Some(&mut feed));
                    // Replay ends paused; a pause gap is not rhythm.
                    t.break_rhythm();
                }
                None => p.replay_timed(&keys, &j.key_ms, j.elapsed_seconds, now, None),
            }
            log::info!(
                "Restored an interrupted session ({} keys, {:.0} s)",
                p.key_log().len(),
                j.elapsed_seconds
            );
        }
        self.notice = "Recovered your interrupted session (paused) · press Esc or Resume".into();
        self.write_journal(now);
        self.render(now);
        if self.rhythm.is_some() {
            self.write_flow_live();
            self.write_flow_weak();
        }
    }

    /// Journal the running session for crash recovery (never per key).
    fn write_journal(&mut self, now: Instant) {
        self.last_journal = now;
        let (Some(p), Some(ctx)) = (&self.practice, self.nav.current().context()) else {
            return;
        };
        let journaled = self.drill.is_none()
            && !ctx.is_exam()
            && matches!(p.state(), SessionState::Active | SessionState::Paused);
        let Some(exercise_id) = ctx.exercise_id.filter(|&id| id > 0 && journaled) else {
            return;
        };
        let (timer, timer_secs) = Journal::timer_fields(p.timer());
        let j = Journal {
            product: ctx.product.slug().to_owned(),
            exercise_id,
            mode: ctx.mode.slug().to_owned(),
            timer,
            timer_secs,
            key_mode: p.mode() == DisplayMode::Key,
            elapsed_seconds: p.view(now).stats.elapsed_seconds,
            keys: recovery::encode(p.key_log()),
            key_ms: p.key_ms(),
        };
        if let Err(e) = j.save(&self.journal_path) {
            log::warn!("Could not journal the session: {e}");
        }
    }

    // --- Typing -----------------------------------------------------------------

    fn session_ctx(&self) -> Option<SessionContext> {
        self.nav.current().context().cloned()
    }

    fn focus_keys(&self) {
        self.with_window(|w| w.invoke_focus_keys());
    }

    /// The passage text area width: reported by the screen, else estimated
    /// from the window (screen padding, sidebar, pane padding).
    fn passage_width(&self) -> f32 {
        let flow = self.is_flow();
        if flow && self.flow_w > 1.0 {
            return self.flow_w;
        }
        if !flow && self.passage_w > 1.0 {
            return self.passage_w;
        }
        let window = self.window.upgrade().map_or(1024.0, |w| {
            let size = w.window().size();
            size.width as f32 / w.window().scale_factor().max(0.1)
        });
        if flow {
            // flow_typing.slint: min(window - 2 x 24, 1080) - 2 x 32.
            return (window - 48.0).min(1080.0) - 64.0;
        }
        let side = if self.rules.focus_mode {
            0.0
        } else {
            252.0 + 12.0
        };
        window - 2.0 * 16.0 - side - 2.0 * 12.0
    }

    /// Rewrap when the column count changed (resize, font, focus mode).
    fn fit_passage(&mut self) -> bool {
        let width = self.passage_width();
        let Some(p) = &self.practice else {
            return false;
        };
        let devanagari = matches!(
            p.layout(),
            LayoutId::RemingtonGail | LayoutId::RemingtonCbi | LayoutId::Inscript
        );
        let changed = self
            .wrapped
            .fit(p.units(), width, self.rules.font_px as f32, devanagari);
        if changed {
            self.lines_cache.clear();
        }
        changed
    }

    /// Write the whole Typing screen for a freshly started practice.
    fn begin_typing(&mut self) {
        let ctx = self.session_ctx();
        let (Some(p), Some(e), Some(ctx)) = (&self.practice, &self.exercise, ctx) else {
            return;
        };
        let layout = p.layout();
        let devanagari = matches!(
            layout,
            LayoutId::RemingtonGail | LayoutId::RemingtonCbi | LayoutId::Inscript
        );
        let learnish = matches!(
            ctx.mode,
            PracticeMode::Learn | PracticeMode::Beginner | PracticeMode::Classic
        );
        // Rules (TY-05): locked by the exam (RULES 38); the Exercise
        // screen's strict rule turns Backspace off.
        self.rules.locked = ctx.is_exam();
        let exam_test = if ctx.is_exam() {
            self.exam.test(ExamLanguage::of_layout(layout))
        } else {
            None
        };
        if ctx.no_backspace {
            self.rules.backspace = BackspacePolicy::Off;
        } else if ctx.is_exam() {
            // The profile's rule when known; allowed otherwise (not invented).
            self.rules.backspace = exam_mode::backspace(exam_test);
        }
        self.rules.fit_script(devanagari);
        // TY-08: guide and keyboard ON in LEARN / BEGINNER, OFF in TEST,
        // EXAM and FLOW; kept across Retry / Next within a course.
        let course = course_key(&ctx);
        if self.typing_course.as_deref() != Some(course.as_str()) {
            let dock = GuideMode::from_slug(self.guide_mode).unwrap_or_default();
            self.guide = if !learnish {
                GuideMode::Off
            } else if ctx.mode == PracticeMode::Beginner && dock == GuideMode::Full {
                // HL-05: the ramp's level, unless the dock chose another.
                self.learn.layout(layout).guide_mode()
            } else {
                dock
            };
            self.typing_course = Some(course);
        }
        self.wrapped = passage::Wrapped::default();
        self.saved = false;
        self.streak = Streak::default();
        self.opened_at = Instant::now();
        self.last_input = None;
        self.recent_error = false;
        self.coach.clear();
        self.notice.clear();
        if let Some(n) = self
            .exam
            .profile
            .filter(|_| ctx.is_exam())
            .and_then(|p| p.practice_notice.as_deref())
        {
            // The unofficial / practice notice on the Typing screen too.
            self.notice = n.to_owned();
        }
        self.timer_warned = false;
        self.lines_cache.clear();
        let title = session_title(
            ctx.is_exam(),
            self.drill.is_some(),
            self.custom.is_some(),
            e,
        );
        let subtitle = if ctx.is_exam() {
            self.exam.title.clone()
        } else {
            e.product.display_name().to_owned()
        };
        let mode_label = format!(
            "{} · {}",
            ctx.mode.label(),
            match p.mode() {
                DisplayMode::Key => "KEY mode",
                DisplayMode::Word => "WORD mode",
            }
        );
        let cert = if ctx.is_exam() {
            exam_mode::pill(self.exam.profile, exam_test)
        } else {
            ""
        };
        let timer_label = timer_summary(p.timer());
        let (word_no, word_total) = passage::word_numbers(p.units());
        self.word_no = word_no;
        self.word_total = word_total;
        self.scroll_first = None;
        self.typed_anchor = 0;
        let passage_subtitle = if ctx.is_exam() {
            if self.exam.profile.is_some_and(|p| p.unofficial_practice) {
                format!(
                    "{} · Practice passage · not the official test",
                    self.exam.title
                )
            } else {
                format!("{} · Practice passage", self.exam.title)
            }
        } else if self.custom.is_some() {
            "Your own passage · practice only".to_owned()
        } else {
            match &e.category {
                Some(c) => format!("{} · {c}", e.product.display_name()),
                None => e.product.display_name().to_owned(),
            }
        };
        // Hindi Learn strip (HL-01, HL-08): beginner track only.
        let beginner = ctx.mode == PracticeMode::Beginner && layout != LayoutId::Qwerty;
        let steps = hindi_learn::steps(layout);
        let progress = self.learn.layout(layout);
        let current = match self.drill {
            Some(Drill::Step(i)) => Some(i),
            _ => None,
        };
        let chips = if beginner {
            learn_chips(&steps, &progress, current)
        } else {
            Vec::new()
        };
        let learn_title = match self.drill {
            Some(Drill::Step(i)) => steps.get(i).map_or_else(String::new, |s| {
                format!(
                    "Step {} of {} · {} types {} · {} hand, {} finger{}",
                    i + 1,
                    steps.len(),
                    s.legend,
                    s.glyph,
                    s.hand,
                    s.finger.to_lowercase(),
                    if s.new_key {
                        " · new key"
                    } else {
                        " · reinforce"
                    }
                )
            }),
            Some(Drill::Matra) => format!("Matra trainer · {}", hindi_learn::MATRAS.join(" ")),
            Some(Drill::Half) => {
                format!("Half-letter trainer · {}", hindi_learn::CONJUNCTS.join(" "))
            }
            Some(Drill::Recovery) => "Recovery drill · your missed words and weak keys".to_owned(),
            None => format!(
                "Beginner lesson {} · {} steps on this layout",
                e.number,
                steps.len()
            ),
        };
        let learn_note = format!(
            "Generated content · review pending. Unlock: {:.0}% accuracy over {}+ characters. Guide ramp: {}.",
            hindi_learn::UNLOCK_ACCURACY,
            hindi_learn::UNLOCK_MIN_CHARS,
            self.guide.slug()
        );
        let has_prev = match self.drill {
            Some(Drill::Step(i)) => i > 0,
            Some(_) => false,
            None => self.adjacent_card(-1).is_some(),
        };
        let has_next = match self.drill {
            Some(Drill::Step(i)) => i < progress.unlocked && i + 1 < steps.len(),
            Some(_) => false,
            None => self.adjacent_card(1).is_some(),
        };
        let flow = ctx.product == ProductId::TypingFlow;
        let layout_name = layout.display_name();
        self.with_window(|w| {
            let v = w.global::<TypingView>();
            v.set_title(title.into());
            v.set_subtitle(subtitle.into());
            v.set_mode_label(mode_label.into());
            v.set_timer_label(timer_label.into());
            v.set_layout_name(layout_name.into());
            v.set_cert_pill(cert.into());
            v.set_devanagari(devanagari);
            v.set_legacy(layout == LayoutId::KrutiDev);
            v.set_passage_subtitle(passage_subtitle.into());
            v.set_learn_visible(beginner);
            v.set_learn_steps(model(chips));
            v.set_learn_title(learn_title.into());
            v.set_learn_note(learn_note.into());
            v.set_flow(flow);
            v.set_has_prev(has_prev);
            v.set_has_next(has_next);
            v.set_coach_message(SharedString::new());
            v.set_focused(true);
        });
        self.fit_passage();
        self.write_rules();
        self.render(Instant::now());
        self.reset_rhythm();
        self.ticker.stop();
        if self.active_guide() == GuideMode::Smart {
            self.start_ticker();
        }
        self.write_typing_header();
    }

    /// The guide in force: nothing unless Rules → Key Guide is on.
    fn active_guide(&self) -> GuideMode {
        effective_guide(self.guide, self.rules.key_guide, self.rules.locked)
    }

    /// The rules panel and the passage card's rule chips.
    fn write_rules(&self) {
        let r = self.rules;
        let limit = |n: u32| {
            if n == 0 {
                "Off".to_owned()
            } else {
                n.to_string()
            }
        };
        let guide = match self.guide {
            GuideMode::Off => GuideMode::Full,
            g => g,
        }
        .slug();
        let guide_on = self.active_guide() != GuideMode::Off;
        let fullscreen = self.fullscreen;
        let chip_limit = if r.word_limit > 0 {
            format!("Limit: {} Words", r.word_limit)
        } else {
            format!("Full passage · {} words", self.word_total)
        };
        self.with_window(|w| {
            let v = w.global::<TypingView>();
            v.set_rules_locked(r.locked);
            v.set_rules_lock_reason("set by the exam profile".into());
            v.set_backspace(r.backspace.slug().into());
            v.set_highlight(r.highlight.slug().into());
            v.set_chip_backspace(r.backspace.label().into());
            v.set_chip_highlight(r.highlight.label().into());
            v.set_chip_limit(chip_limit.into());
            v.set_guide_mode(guide.into());
            v.set_key_guide(r.key_guide && !r.locked);
            v.set_font_size(r.font_px as f32);
            v.set_font_label(format!("{} px", r.font_px).into());
            v.set_bold(r.bold);
            v.set_paper(r.paper);
            v.set_show_scrollbar(r.show_scrollbar);
            v.set_auto_scroll(r.auto_scroll);
            v.set_word_limit_on(r.word_limit > 0);
            if r.word_limit > 0 {
                v.set_word_limit_text(r.word_limit.to_string().into());
            }
            v.set_key_limit(limit(r.key_limit).into());
            v.set_sound(r.sound);
            v.set_focus_mode(r.focus_mode);
            v.set_fullscreen(fullscreen);
            v.set_guide_visible(guide_on);
        });
    }

    /// The header's exercise picker, layout switch and timer menu.
    fn write_typing_header(&self) {
        let Some(ctx) = self.session_ctx() else {
            return;
        };
        let layout = ctx.product.layout();
        let mut items: Vec<PickItem> = self
            .shown
            .iter()
            .filter_map(|&i| self.cards.get(i))
            .map(|c| PickItem {
                label: format!("{:02} · {}", c.number, c.title).into(),
            })
            .collect();
        let customs = self.customs_for(layout);
        items.extend(customs.iter().map(|&i| {
            PickItem {
                label: format!(
                    "Custom · {}",
                    self.customs.get(i).map_or("", |c| c.title.as_str())
                )
                .into(),
            }
        }));
        let pos = match (self.custom, &self.exercise) {
            (Some(c), _) => customs
                .iter()
                .position(|&i| i == c)
                .map(|p| p + self.shown.len()),
            (None, Some(e)) => self
                .shown
                .iter()
                .position(|&i| self.cards.get(i).is_some_and(|c| c.id == e.id)),
            (None, None) => None,
        };
        let total = items.len();
        let picker = self.drill.is_none() && total > 0;
        // The counter names the exercise the header title names (its
        // course number), never its row in the filtered list.
        let counter = match (self.custom, &self.exercise) {
            (Some(c), _) => custom_counter(customs.iter().position(|&i| i == c), customs.len()),
            (None, Some(e)) => exercise_counter(e.number, &self.cards, self.filter.track),
            (None, None) => format!("— / {total}"),
        };
        let options: Vec<SegmentOption> = if ctx.is_exam() {
            if layout == LayoutId::Qwerty {
                vec![segment("qwerty", "English QWERTY")]
            } else {
                HINDI_LAYOUTS
                    .iter()
                    .filter(|(_, _, l)| self.exam.allows(*l))
                    .map(|(id, _, l)| segment(id, l.display_name()))
                    .collect()
            }
        } else if layout_switchable(ctx.product) {
            MASTERY_LAYOUTS
                .iter()
                .map(|(id, _, l)| segment(id, l.display_name()))
                .collect()
        } else {
            vec![segment(layout.as_str(), layout.display_name())]
        };
        let switchable = self.drill.is_none() && options.len() > 1;
        let timer = ctx.timer;
        let menu = match timer {
            TimerMode::Countdown(l) => format!("{} Min", crate::exercise::mm_ss(l.seconds())),
            TimerMode::Stopwatch => "Stopwatch".to_owned(),
            TimerMode::Off => "No timer".to_owned(),
        };
        let active = match timer {
            TimerMode::Countdown(l) => radha_core::TimerPreset::all()
                .iter()
                .position(|p| p.seconds() == l.seconds()),
            TimerMode::Stopwatch => Some(radha_core::TimerPreset::all().len()),
            TimerMode::Off => None,
        };
        let presets: Vec<TimerChoice> = TimerStudio::preset_labels()
            .into_iter()
            .chain(std::iter::once("No limit".to_owned()))
            .enumerate()
            .map(|(i, label)| TimerChoice {
                label: label.into(),
                selected: active == Some(i),
            })
            .collect();
        let locked = ctx.is_exam();
        self.with_window(|w| {
            let v = w.global::<TypingView>();
            v.set_picker_visible(picker);
            v.set_exercise_counter(counter.into());
            v.set_exercises(model(items));
            v.set_exercise_pos(pos.map_or(-1, |p| i32::try_from(p).unwrap_or(-1)));
            v.set_layout_id(layout.as_str().into());
            v.set_layout_switchable(switchable);
            v.set_layout_options(model(options));
            v.set_timer_menu_label(menu.into());
            v.set_timer_presets(model(presets));
            v.set_timer_locked(locked);
            v.set_timer_error(SharedString::new());
        });
    }

    /// Indexes of the stored custom passages typed on `layout`.
    fn customs_for(&self, layout: LayoutId) -> Vec<usize> {
        self.customs
            .iter()
            .enumerate()
            .filter(|(_, c)| c.layout == layout.as_str())
            .map(|(i, _)| i)
            .collect()
    }
    fn start_ticker(&self) {
        if self.ticker.running() {
            return;
        }
        let weak = self.me.clone();
        self.ticker
            .start(slint::TimerMode::Repeated, TICK, move || {
                if let Some(flow) = weak.upgrade() {
                    if let Ok(mut f) = flow.try_borrow_mut() {
                        f.tick();
                    }
                }
            });
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let Some(p) = self.practice.as_mut() else {
            self.ticker.stop();
            return;
        };
        if p.tick(now) {
            self.audio.play(Sound::Timer);
            self.finish(now);
            return;
        }
        if p.state() != SessionState::Active {
            // Smart guide: an idle typist still gets the hint after a pause.
            if p.state() == SessionState::Idle && self.active_guide() == GuideMode::Smart {
                self.render(now);
            }
            return;
        }
        if now.saturating_duration_since(self.last_journal) >= JOURNAL_EVERY {
            self.write_journal(now);
        }
        self.render(now);
    }

    /// Redraw the typing screen from the session at `now`. Only the passage
    /// rows whose runs changed are written to the model.
    fn render(&mut self, now: Instant) -> Rendered {
        self.sync_esc_chip();
        let Some(p) = &self.practice else {
            return Rendered::default();
        };
        let view = p.view(now);
        self.last_cursor = view.cursor;
        let s = &view.stats;
        let new_error = self.streak.update(view.cursor, s.errors);
        if new_error {
            self.recent_error = true;
        }
        let idle = now.saturating_duration_since(self.last_input.unwrap_or(self.opened_at));
        let idle_ms = u64::try_from(idle.as_millis()).unwrap_or(u64::MAX);
        let guide = self.active_guide();
        let show = guide_show(guide, idle_ms, self.recent_error);
        let time = match (p.timer(), s.remaining_seconds) {
            (TimerMode::Off, _) => "—".to_owned(),
            (_, Some(left)) => clock(left.ceil()),
            (_, None) => clock(s.elapsed_seconds),
        };
        let (ring_visible, arc, warning) = match (p.timer().limit(), s.remaining_seconds) {
            (Some(l), Some(left)) => (
                true,
                ring_arc((left / f64::from(l.seconds().max(1))) as f32),
                left <= 10.0,
            ),
            _ => (false, String::new(), false),
        };
        if warning && !self.timer_warned && s.state == SessionState::Active {
            self.timer_warned = true;
            self.audio.play(Sound::Timer);
        }

        // Passage: the visible window only, diffed row by row.
        let units = p.units();
        let lines = &self.wrapped.lines;
        // Each screen reports its own line count: the Typing screen's
        // (often 8+) would push the cursor line out of the Flow stream.
        let rows_n = if self.is_flow() {
            self.flow_rows
        } else {
            self.passage_rows
        }
        .max(1);
        let window = match self.scroll_first {
            Some(first) => passage::window_from(lines, first, rows_n),
            None if self.rules.auto_scroll => passage::window_n(lines, view.cursor, rows_n),
            None => passage::window_from(lines, 0, rows_n),
        };
        let (scroll_first, scroll_total) = (window.start, lines.len());
        // One buffer line above and below the visible rows, so a one-line
        // scroll glides (the screen offsets the block by `lines-base`).
        // A cleared cache (new text, rewrap, rows) snaps instead.
        let drawn = passage::with_buffer_lines(&window, lines.len());
        let lines_base = drawn.start;
        // The flag is written only when the first line moves: rewriting it
        // on the next key would cut a running glide short.
        let scroll_glide = (self.shown_first != Some(scroll_first)).then(|| {
            !self.lines_cache.is_empty() && passage::glides(self.shown_first, scroll_first)
        });
        self.shown_first = Some(scroll_first);
        let key_cursor = (guide != GuideMode::Off && show.key_highlight).then_some(view.cursor);
        let rows: Vec<Vec<(String, i32)>> = self
            .wrapped
            .lines
            .get(drawn)
            .unwrap_or(&[])
            .iter()
            .map(|line| {
                passage::word_runs(
                    units,
                    &view.states,
                    line.clone(),
                    key_cursor,
                    view.word_span.clone(),
                    &view.skipped,
                    self.rules.highlight,
                )
                .into_iter()
                // NBSP so a lone space run keeps its width.
                .map(|r| (r.text.replace(' ', "\u{a0}"), r.state as i32))
                .collect()
            })
            .collect();
        let to_line = |segs: &[(String, i32)]| PassageLine {
            segments: model(
                segs.iter()
                    .map(|(t, s)| PassageSegment {
                        text: t.as_str().into(),
                        state: *s,
                    })
                    .collect(),
            ),
        };
        if self.lines_model.row_count() != rows.len() || self.lines_cache.len() != rows.len() {
            self.lines_model
                .set_vec(rows.iter().map(|r| to_line(r)).collect::<Vec<_>>());
        } else {
            for (i, r) in rows.iter().enumerate() {
                if self.lines_cache.get(i) != Some(r) {
                    self.lines_model.set_row_data(i, to_line(r));
                }
            }
        }
        self.lines_cache = rows;

        let legacy_reading = if p.layout() == LayoutId::KrutiDev {
            let current = passage::line_of(&self.wrapped.lines, view.cursor);
            self.wrapped
                .lines
                .get(current)
                .and_then(|r| units.get(r.clone()))
                .map(|u| to_unicode(&u.concat()).text)
                .unwrap_or_default()
        } else {
            String::new()
        };
        let progress = if units.is_empty() {
            0.0
        } else {
            view.cursor as f32 / units.len() as f32
        };
        let coach_word = units
            .get(view.word_span.clone())
            .map(<[String]>::concat)
            .unwrap_or_default();
        let typed = p.typed();
        let devanagari = matches!(
            p.layout(),
            LayoutId::RemingtonGail | LayoutId::RemingtonCbi | LayoutId::Inscript
        );
        let cols = passage::columns(self.passage_width() - 24.0, INPUT_FONT_PX, devanagari);
        // One line more than the pane shows: it sits above, clipped, and
        // slides out when the text wraps.
        let (anchor, head, tail) = input_lines(&typed, self.typed_anchor, cols, INPUT_LINES + 1);
        self.typed_anchor = anchor;
        let (typed_glide, wraps) = typed_wrap_step(self.typed_mark, cols, &typed, anchor, &head);
        self.typed_wraps = self.typed_wraps.wrapping_add(wraps);
        self.typed_mark = Some((cols, tail_start(anchor, &head)));
        let typed_wraps = self.typed_wraps;
        let preedit = p.preedit().unwrap_or("");
        let guide_on = guide != GuideMode::Off;
        let word_n = self
            .word_no
            .get(view.cursor)
            .copied()
            .unwrap_or(self.word_total);
        let total_words = self.word_total.max(1);
        let done = if view.cursor >= units.len() {
            total_words
        } else {
            word_n.saturating_sub(1)
        };
        let word_frac = done as f32 / total_words as f32;
        let word_pos = format!(
            "Word {} of {} ({:.0}%)",
            word_n.max(1).min(total_words),
            self.word_total,
            (word_frac * 100.0).floor()
        );
        let exam = self.session_ctx().is_some_and(|c| c.is_exam());
        let (live, live_label) = match s.state {
            SessionState::Active if exam => (true, "Live Exam Session in Progress"),
            SessionState::Active => (true, "Live Session in Progress"),
            SessionState::Paused => (false, "Paused"),
            SessionState::Idle => (false, "Ready to start"),
            _ => (false, "Session complete"),
        };
        let coach = self.coach.clone();
        let notice = self.notice.clone();
        let streak = self.streak.current;
        let hint = view.hint.clone();
        let stats = s.clone();
        self.with_window(|w| {
            let v = w.global::<TypingView>();
            v.set_stat_time(time.into());
            v.set_ring_visible(ring_visible);
            v.set_ring_arc(arc.into());
            v.set_ring_warning(warning);
            v.set_stat_wpm(format!("{:.0}", stats.wpm).into());
            v.set_stat_net(format!("{:.0}", stats.net_wpm).into());
            v.set_stat_accuracy(format!("{:.1}%", stats.accuracy).into());
            v.set_stat_errors(stats.errors.to_string().into());
            v.set_stat_streak(streak.to_string().into());
            v.set_stat_words(stats.words.to_string().into());
            v.set_stat_kdph(format!("{:.0}", stats.kdph).into());
            v.set_stat_keys(stats.presses.to_string().into());
            v.set_progress(progress);
            v.set_paused(stats.state == SessionState::Paused);
            v.set_idle(stats.state == SessionState::Idle);
            v.set_legacy_reading(legacy_reading.into());
            v.set_coach_word(coach_word.into());
            v.set_typed_tail(tail.into());
            v.set_typed_head(head.into());
            if wraps != 0 {
                v.set_typed_glide(typed_glide);
                v.set_typed_wraps(typed_wraps);
            }
            v.set_word_position(word_pos.into());
            v.set_word_progress(word_frac);
            // Glide flag first: the animation reads it when scroll-first moves.
            if let Some(glide) = scroll_glide {
                v.set_scroll_glide(glide);
            }
            v.set_lines_base(i32::try_from(lines_base).unwrap_or(0));
            v.set_scroll_first(i32::try_from(scroll_first).unwrap_or(0));
            v.set_scroll_rows(i32::try_from(rows_n).unwrap_or(1));
            v.set_scroll_total(i32::try_from(scroll_total).unwrap_or(1));
            v.set_live(live);
            v.set_live_label(live_label.into());
            v.set_preedit(preedit.into());
            v.set_coach_message(coach.into());
            v.set_notice(notice.into());
            v.set_guide_visible(guide_on);
            v.set_guide_show_key(show.next_key);
            v.set_guide_show_finger(show.finger);
            match &hint {
                Some(h) => {
                    let mut mods = String::new();
                    if h.shift {
                        mods.push_str("Shift + ");
                    }
                    if h.altgr {
                        mods.push_str("AltGr + ");
                    }
                    v.set_guide_key(h.key.as_str().into());
                    v.set_guide_modifiers(mods.into());
                    v.set_guide_hand(h.hand.into());
                    v.set_guide_finger(finger_sentence(h.hand, h.finger, h.shift, h.altgr).into());
                    v.set_guide_output(h.output.replace(' ', "Space").into());
                    v.set_guide_meaning(h.meaning.clone().unwrap_or_default().into());
                }
                None => {
                    v.set_guide_key("—".into());
                    v.set_guide_modifiers(SharedString::new());
                    v.set_guide_hand("—".into());
                    v.set_guide_finger("—".into());
                    v.set_guide_output(SharedString::new());
                    v.set_guide_meaning(SharedString::new());
                }
            }
        });
        self.last_hint = hint;
        Rendered {
            new_error,
            words: stats.words,
            presses: stats.presses,
        }
    }

    fn write_notice(&self) {
        let coach = self.coach.clone();
        let notice = self.notice.clone();
        self.with_window(|w| {
            let v = w.global::<TypingView>();
            v.set_coach_message(coach.into());
            v.set_notice(notice.into());
        });
    }

    /// The "Esc armed" chip follows the session's dead key. O(1): a bool
    /// compare per render, a property write only when it flips.
    fn sync_esc_chip(&mut self) {
        let armed = self.practice.as_ref().is_some_and(Practice::dead_key_armed);
        if armed != self.esc_chip {
            self.esc_chip = armed;
            self.with_window(|w| w.global::<TypingView>().set_esc_armed(armed));
        }
    }

    /// A key event on the Typing screen.
    fn key(&mut self, text: &str, control: bool, alt: bool) {
        let now = Instant::now();
        match classify_key(text, control, alt) {
            KeyInput::Escape => {
                // GAIL/CBI (books 01/03): Esc then a key is the dead-key
                // layer (Esc then I = फ़). A second Esc cancels it and does
                // what Esc does everywhere else (pause, then back).
                if let Some(p) = self.practice.as_mut() {
                    if p.cancel_dead_key() {
                        self.notice.clear();
                    } else if p.arm_dead_key() {
                        self.notice = "Esc layer: the next key types its Esc character \
                                       (Esc again to pause or leave)"
                            .into();
                        self.write_notice();
                        self.sync_esc_chip();
                        return;
                    }
                    self.write_notice();
                    self.sync_esc_chip();
                }
                self.escape(now);
            }
            KeyInput::Restart => self.restart(),
            // The Typing Flow screen has no focus mode.
            KeyInput::ToggleFocus if !self.is_flow() => self.toggle_focus_mode(),
            KeyInput::ToggleFocus => {}
            KeyInput::Backspace => {
                let policy = self.rules.backspace;
                let Some(p) = self.practice.as_mut() else {
                    return;
                };
                let Some(effect) = p.backspace_under(policy, now) else {
                    self.notice = match policy {
                        BackspacePolicy::Off => "Backspace is disabled for this session".into(),
                        _ => "One Word Backspace: you cannot go back past the current word".into(),
                    };
                    self.write_notice();
                    return;
                };
                if self.rules.auto_scroll {
                    self.scroll_first = None;
                }
                if matches!(effect, KeyEffect::Applied { .. }) {
                    self.last_input = Some(now);
                    self.audio.play(Sound::Key);
                    if let Some(t) = self.rhythm.as_mut() {
                        t.correction_at(now);
                    }
                }
                self.render(now);
                self.write_flow_live();
            }
            KeyInput::Char { c, altgr } => {
                let expected = self.last_hint.clone();
                let Some(p) = self.practice.as_mut() else {
                    return;
                };
                // The unit the key was aimed at (from the last render).
                let aimed = p
                    .units()
                    .get(self.last_cursor)
                    .and_then(|u| u.chars().next());
                let effect = p.press(c, altgr, now);
                let mut weak_changed = false;
                if matches!(effect, KeyEffect::Applied { .. }) {
                    // Typing Flow rhythm engine: O(1), no allocation.
                    if let Some(t) = self.rhythm.as_mut() {
                        let correct = aimed == Some(c);
                        t.key_at(now, aimed.unwrap_or('\0'), correct);
                        let word = self.word_no.get(self.last_cursor).copied().unwrap_or(0);
                        weak_changed = self.live_flow.after_key(t, word, !correct);
                    }
                }
                let pressed = p.pressed_hint(c, altgr);
                let state = p.state();
                let layout = p.layout();
                let applied = matches!(effect, KeyEffect::Applied { .. });
                if applied {
                    self.last_input = Some(now);
                    self.recent_error = false;
                    self.notice.clear();
                    if self.rules.auto_scroll {
                        self.scroll_first = None;
                    }
                }
                if state == SessionState::Complete {
                    self.finish(now);
                    return;
                }
                if state == SessionState::Active {
                    self.start_ticker();
                }
                let r = self.render(now);
                self.write_flow_live();
                if weak_changed {
                    self.write_flow_weak();
                }
                match effect {
                    KeyEffect::Applied { .. } if r.new_error => {
                        self.audio.play(Sound::Error);
                        // HL-02: the mistake coach, wherever the guide is on.
                        self.coach = match (&expected, self.active_guide()) {
                            (_, GuideMode::Off) | (None, _) => String::new(),
                            (Some(e), _) => hindi_learn::coach_line(pressed.as_ref(), e),
                        };
                    }
                    KeyEffect::Applied { .. } => {
                        self.audio.play(Sound::Key);
                        self.coach.clear();
                    }
                    KeyEffect::Unmapped => {
                        self.notice = format!(
                            "That key types nothing on {} (a US Windows keyboard is assumed)",
                            layout.display_name()
                        );
                    }
                    KeyEffect::Ignored => {}
                }
                self.write_notice();
                if self.rules.limit_reached(r.words, r.presses) {
                    self.finish(now);
                }
            }
            KeyInput::Ignore => {}
        }
    }

    fn escape(&mut self, now: Instant) {
        let state = self.practice.as_ref().map(Practice::state);
        match escape_action(self.nav.current(), state) {
            EscapeAction::PauseTyping => self.toggle_pause(now),
            EscapeAction::Back => self.back(),
            EscapeAction::Ignore => {}
        }
    }

    /// Pause / resume (TY-07): the clock freezes and the keyboard focus
    /// stays on the typing scope.
    fn toggle_pause(&mut self, now: Instant) {
        if let Some(p) = self.practice.as_mut() {
            p.toggle_pause(now);
            if let Some(t) = self.rhythm.as_mut() {
                // The gap around a pause is not rhythm.
                t.break_rhythm();
            }
            self.notice.clear();
            self.write_journal(now);
            self.render(now);
        }
        self.focus_keys();
    }

    /// The window lost or regained focus (TY-14).
    fn focus_changed(&mut self, has_focus: bool, window: bool) {
        let now = Instant::now();
        if !has_focus && window {
            if let Some(p) = self.practice.as_mut() {
                if p.pause_if_active(now) {
                    if let Some(t) = self.rhythm.as_mut() {
                        t.break_rhythm();
                    }
                    self.notice = "Paused because the window lost focus".into();
                    self.write_journal(now);
                    self.render(now);
                }
            }
        }
        self.with_window(|w| w.global::<TypingView>().set_focused(has_focus));
    }

    fn restart(&mut self) {
        if let Some(p) = self.practice.as_mut() {
            p.restart();
            self.ticker.stop();
            self.saved = false;
            self.streak = Streak::default();
            self.coach.clear();
            self.notice.clear();
            self.timer_warned = false;
            self.last_input = None;
            self.opened_at = Instant::now();
            self.lines_cache.clear();
            recovery::clear(&self.journal_path);
            self.render(Instant::now());
            self.reset_rhythm();
        }
        self.focus_keys();
    }

    fn passage_resized(&mut self, width: f32) {
        if (width - self.passage_w).abs() < 0.5 {
            return;
        }
        self.passage_w = width;
        if self.fit_passage() {
            self.render(Instant::now());
        }
    }

    /// Apply a sidebar change and hand focus back to typing.
    fn rules_changed(&mut self) {
        self.write_rules();
        if self.fit_passage() {
            self.lines_cache.clear();
        }
        self.lines_cache.clear();
        self.render(Instant::now());
        self.focus_keys();
    }

    fn set_rule(&mut self, f: impl FnOnce(&mut TypingRules)) {
        if self.rules.locked {
            self.focus_keys();
            return;
        }
        f(&mut self.rules);
        self.rules_changed();
    }

    fn font_step(&mut self, d: i32) {
        let devanagari = self.practice.as_ref().is_some_and(|p| {
            matches!(
                p.layout(),
                LayoutId::RemingtonGail | LayoutId::RemingtonCbi | LayoutId::Inscript
            )
        });
        self.rules.step_font(d.signum() * 2, devanagari);
        let px = self.rules.font_px.to_string();
        self.persist("font-px", &px);
        self.rules_changed();
    }

    fn set_typing_guide(&mut self, id: &str) {
        if self.rules.locked {
            self.focus_keys();
            return;
        }
        if let Some(m) = GuideMode::from_slug(id) {
            self.guide = m;
        }
        self.rules_changed();
    }

    /// Rules → Focus mode, the header's "Exit Focus (F11)" button and F11.
    /// Allowed in exams too: it changes the view, not a rule.
    fn toggle_focus_mode(&mut self) {
        self.rules.focus_mode = !self.rules.focus_mode;
        self.rules_changed();
    }

    fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        let on = self.fullscreen;
        self.with_window(|w| w.window().set_fullscreen(on));
        self.rules_changed();
    }

    // --- Typing screen v3: rules panel, picker, timer menu --------------------

    /// How many passage lines fit in the card (the screen reports it).
    fn passage_rows(&mut self, rows: i32) {
        let rows = usize::try_from(rows).unwrap_or(1).clamp(1, 60);
        if rows != self.passage_rows {
            self.passage_rows = rows;
            self.lines_cache.clear();
            self.render(Instant::now());
        }
    }

    /// Wheel / scrollbar: move the passage `delta` lines. With Auto Scroll
    /// on, the next key brings the cursor's line back.
    fn passage_scroll(&mut self, delta: i32) {
        let lines = &self.wrapped.lines;
        let rows = self.passage_rows.max(1);
        let current = self.scroll_first.unwrap_or_else(|| {
            if self.rules.auto_scroll {
                passage::window_n(lines, self.last_cursor, rows).start
            } else {
                0
            }
        });
        let max_first = lines.len().saturating_sub(rows);
        let next = i64::try_from(current)
            .unwrap_or(0)
            .saturating_add(i64::from(delta))
            .clamp(0, i64::try_from(max_first).unwrap_or(0));
        let next = usize::try_from(next).unwrap_or(0);
        if Some(next) != self.scroll_first {
            self.scroll_first = Some(next);
            self.render(Instant::now());
        }
    }

    /// The header / dock exercise picker: row `index` of the filtered list
    /// (then the custom passages). Loads at once and starts a clean session.
    fn pick_exercise(&mut self, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let Some(ctx) = self.session_ctx() else {
            return;
        };
        if self.drill.is_none() {
            if let Some(meta) = self
                .shown
                .get(index)
                .and_then(|&i| self.cards.get(i))
                .cloned()
            {
                match self.load_text(&ctx, &meta) {
                    Ok(exercise) => self.start_exercise(exercise, ctx, None),
                    Err(e) => log::error!("Could not load the exercise: {e}"),
                }
            } else {
                let customs = self.customs_for(ctx.product.layout());
                if let Some(&c) = index
                    .checked_sub(self.shown.len())
                    .and_then(|i| customs.get(i))
                {
                    self.start_custom(c, ctx);
                }
            }
        }
        self.focus_keys();
    }

    fn start_custom(&mut self, index: usize, ctx: SessionContext) {
        let Some(c) = self.customs.get(index).cloned() else {
            return;
        };
        let exercise = Exercise {
            id: 0,
            product: ctx.product,
            number: 0,
            title: c.title,
            category: Some("Custom passage".to_owned()),
            difficulty: None,
            content: c.text,
        };
        self.custom = Some(index);
        self.start_exercise(exercise, ctx, None);
    }

    /// Start the current exercise again with `ctx` (a rule that changes
    /// the text or the clock), with a notice saying so.
    fn restart_with(&mut self, ctx: SessionContext, notice: String) {
        let Some(exercise) = self.exercise.clone() else {
            return;
        };
        let (drill, custom) = (self.drill, self.custom);
        self.start_exercise(exercise, ctx, drill);
        self.custom = custom;
        self.notice = notice;
        self.write_notice();
        self.write_typing_header();
    }

    /// The header's layout switch: the course of that layout (the same
    /// path as the Exercise screen's switch), at the same exercise number.
    fn typing_set_layout(&mut self, id: &str) {
        let Ok(layout) = LayoutId::parse(id) else {
            return;
        };
        let Some(ctx) = self.session_ctx() else {
            return;
        };
        if self.drill.is_some() || ctx.product.layout() == layout {
            self.focus_keys();
            return;
        }
        let Some(exercise) = self.exercise.clone() else {
            return;
        };
        let number = ctx.exercise_number;
        self.ticker.stop();
        recovery::clear(&self.journal_path);
        if self.nav.back().is_none() {
            return;
        }
        if !self.switch_layout(layout, true) {
            // Not switchable: carry on with the same exercise.
            if let Some(draft) = self.draft() {
                self.start_exercise(exercise, draft, None);
            }
            return;
        }
        self.preferred_layout = layout;
        let Some(draft) = self.draft() else {
            return;
        };
        let meta = self
            .shown
            .iter()
            .filter_map(|&i| self.cards.get(i))
            .find(|c| c.number == number)
            .or_else(|| self.shown.first().and_then(|&i| self.cards.get(i)))
            .cloned();
        match meta.map(|m| self.load_text(&draft, &m)) {
            Some(Ok(next)) => self.start_exercise(next, draft, None),
            other => {
                if let Some(Err(e)) = other {
                    log::error!("Could not load the exercise: {e}");
                }
                // Nothing to type on that layout: its Exercise screen says why.
                self.practice = None;
                self.drill = None;
                let screen = self.nav.current().clone();
                self.show(&screen);
            }
        }
    }

    /// The header's timer menu: a new clock restarts the exercise. The
    /// Exercise screen's timer studio follows (both show the same timer).
    fn typing_set_timer(&mut self, timer: TimerMode) {
        let Some(ctx) = self.session_ctx() else {
            return;
        };
        if ctx.is_exam() {
            self.focus_keys();
            return;
        }
        if ctx.mode.timer_policy() == TimerPolicy::UserSet {
            self.studio.set_mode(timer);
        }
        let notice = format!(
            "Timer set to {} · the session starts again",
            timer_summary(timer)
        );
        self.restart_with(SessionContext { timer, ..ctx }, notice);
        self.focus_keys();
    }

    fn typing_choose_timer(&mut self, index: i32) {
        let presets = radha_core::TimerPreset::all();
        let timer = match usize::try_from(index) {
            Ok(i) if i == presets.len() => TimerMode::Stopwatch,
            Ok(i) => match presets.get(i) {
                Some(&p) => TimerMode::Countdown(radha_core::TimeLimit::from_preset(p)),
                None => return,
            },
            Err(_) => return,
        };
        self.typing_set_timer(timer);
    }

    /// The timer menu's custom field: minutes, seconds or MM:SS.
    fn typing_custom_timer(&mut self, text: &str) {
        match crate::exercise::parse_custom_time(text) {
            Some(limit) => {
                self.with_window(|w| {
                    let v = w.global::<TypingView>();
                    v.set_custom_timer_text(SharedString::new());
                    v.set_timer_error(SharedString::new());
                });
                self.typing_set_timer(TimerMode::Countdown(limit));
            }
            None => {
                log::debug!("Rejected custom timer {text:?}");
                self.with_window(|w| {
                    w.global::<TypingView>()
                        .set_timer_error(crate::exercise::CUSTOM_TIMER_ERROR.into());
                });
            }
        }
    }

    /// Backspace rule from the panel; the Exercise screen's strict rule
    /// and settings.toml follow.
    fn typing_set_backspace(&mut self, policy: BackspacePolicy) {
        if self.rules.locked {
            self.focus_keys();
            return;
        }
        self.set_rule(|r| r.backspace = policy);
        self.persist("backspace", policy.slug());
        if let Some(ctx) = self.session_ctx() {
            self.nav.switch_session(SessionContext {
                no_backspace: policy == BackspacePolicy::Off,
                ..ctx
            });
        }
    }

    fn typing_set_highlight(&mut self, mode: HighlightMode) {
        if self.rules.locked {
            self.focus_keys();
            return;
        }
        self.set_rule(|r| r.highlight = mode);
        self.persist("highlight", mode.slug());
    }

    /// A panel switch (scrollbar, auto scroll, bold, paper, sound, key
    /// guide): applied at once and saved.
    fn typing_toggle(&mut self, key: &'static str) {
        let r = &mut self.rules;
        let on = match key {
            "scrollbar" => {
                r.show_scrollbar = !r.show_scrollbar;
                r.show_scrollbar
            }
            "auto-scroll" => {
                r.auto_scroll = !r.auto_scroll;
                self.scroll_first = None;
                r.auto_scroll
            }
            "bold" => {
                r.bold = !r.bold;
                r.bold
            }
            "paper" => {
                r.paper = !r.paper;
                r.paper
            }
            "sound" => {
                r.sound = !r.sound;
                let on = r.sound;
                self.audio.set_enabled(on);
                on
            }
            "key-guide" if !r.locked => {
                r.key_guide = !r.key_guide;
                r.key_guide
            }
            _ => {
                self.focus_keys();
                return;
            }
        };
        self.persist(key, if on { "on" } else { "off" });
        self.rules_changed();
        if self.active_guide() == GuideMode::Smart {
            self.start_ticker();
        }
    }

    /// Paragraph settings: the word limit (50–1500) cuts the passage at a
    /// word boundary; the exercise starts again on the new text.
    fn set_word_limit(&mut self, on: bool, text: &str) {
        if self.rules.locked {
            self.focus_keys();
            return;
        }
        let limit = if on {
            match parse_word_limit(text) {
                Ok(n) => n,
                Err(e) => {
                    self.with_window(|w| {
                        let v = w.global::<TypingView>();
                        v.set_word_limit_error(e.into());
                        v.set_word_limit_on(true);
                    });
                    return;
                }
            }
        } else {
            0
        };
        self.with_window(|w| {
            w.global::<TypingView>()
                .set_word_limit_error(SharedString::new());
        });
        if limit == self.rules.word_limit {
            self.write_rules();
            self.focus_keys();
            return;
        }
        self.rules.word_limit = limit;
        let value = if limit == 0 {
            "off".to_owned()
        } else {
            limit.to_string()
        };
        self.persist("word-limit", &value);
        let notice = if limit == 0 {
            "Word limit off: the full passage · the session starts again".to_owned()
        } else {
            format!(
                "Word limit {limit}: the passage ends at word {limit} · the session starts again"
            )
        };
        if let Some(ctx) = self.session_ctx() {
            self.restart_with(ctx, notice);
        }
        self.write_rules();
        self.focus_keys();
    }

    /// A dialog (Add Exercise) opened over a running session: pause it.
    fn pause_for_dialog(&mut self) {
        let now = Instant::now();
        if let Some(p) = self.practice.as_mut() {
            if p.pause_if_active(now) {
                if let Some(t) = self.rhythm.as_mut() {
                    t.break_rhythm();
                }
                self.notice = "Paused while the dialog is open".into();
                self.write_journal(now);
                self.render(now);
            }
        }
    }

    /// Printout Mode: the passage as a print-ready HTML page, saved beside
    /// the scorecards and opened for printing.
    fn printout(&mut self) {
        let (Some(e), Some(p)) = (&self.exercise, &self.practice) else {
            return;
        };
        let layout = p.layout();
        let typed_text = p.units().concat();
        let text = if layout == LayoutId::KrutiDev {
            to_unicode(&typed_text).text
        } else {
            typed_text
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let path = self.export_dir.join(format!(
            "passage-{}-{stamp}.html",
            crate::export::file_stem(&e.title)
        ));
        let html = passage_print_html(&e.title, layout.display_name(), self.word_total, &text);
        self.notice = match crate::export::write_new(&path, html.as_bytes()) {
            Ok(()) => {
                log::info!("Passage printout saved to {}", path.display());
                if let Err(err) = open_file(&path) {
                    log::warn!("Could not open the printout: {err}");
                }
                format!("Printout saved to {} · opened for printing", path.display())
            }
            Err(err) => {
                log::warn!("Printout failed: {err}");
                format!("Could not save the printout: {err}")
            }
        };
        self.write_notice();
        self.focus_keys();
    }

    /// Add Exercise: validate the pasted text for the layout, store it and
    /// type it now.
    fn add_exercise(&mut self, title: &str, text: &str) {
        let Some(ctx) = self.session_ctx() else {
            return;
        };
        let result = if ctx.is_exam() {
            Err("Exams type their own passages; add yours from a typing course".to_owned())
        } else {
            crate::custom::prepare(ctx.product.layout(), text)
        };
        let content = match result {
            Ok(c) => c,
            Err(e) => {
                self.with_window(|w| {
                    let v = w.global::<TypingView>();
                    v.set_add_status(e.into());
                    v.set_add_ok(false);
                });
                return;
            }
        };
        self.customs.push(crate::custom::CustomExercise {
            title: crate::custom::title(title),
            layout: ctx.product.layout().as_str().to_owned(),
            text: content,
        });
        if self.customs.len() > crate::custom::MAX_STORED {
            self.customs.remove(0);
        }
        let status = match crate::custom::save(&self.custom_path, &self.customs) {
            Ok(()) => "Saved to your passages".to_owned(),
            Err(e) => format!("Typing it now, but it could not be saved: {e}"),
        };
        self.with_window(|w| {
            let v = w.global::<TypingView>();
            v.set_add_status(status.into());
            v.set_add_ok(true);
        });
        let index = self.customs.len() - 1;
        self.start_custom(index, ctx);
        self.focus_keys();
    }

    /// Complete the session, save it once, show the Result.
    fn finish(&mut self, now: Instant) {
        if self.saved {
            return;
        }
        let Some(ctx) = self.session_ctx() else {
            return;
        };
        let Some(p) = self.practice.as_mut() else {
            return;
        };
        if p.state() == SessionState::Idle {
            return; // nothing typed: nothing to finish
        }
        p.complete(now);
        self.ticker.stop();
        self.saved = true;
        let Some(p) = self.practice.as_ref() else {
            return;
        };
        let summary = p.summary(now);
        let live = p.view(now).stats;
        // Typing Flow: the rhythm analysis, once, after the last key.
        let flow_report = if self.is_flow() {
            self.rhythm.as_ref().map(FlowReport::from_tracker)
        } else {
            None
        };
        // The personal bests before this session is saved (the Result
        // compares against them); exercises only, not drills or customs.
        let flow_prev = match (&flow_report, ctx.exercise_id) {
            (Some(_), Some(id)) if self.drill.is_none() && self.custom.is_none() => {
                self.flow_bests(id)
            }
            _ => (None, None),
        };
        let mut saved_ok = false;
        let layout = p.layout();
        let devanagari = matches!(
            layout,
            LayoutId::RemingtonGail | LayoutId::RemingtonCbi | LayoutId::Inscript
        );
        let finished = summary.completion == Some(CompletionReason::Finished);
        let (full, half) = p.mistake_counts(now);

        // Exam result (EM-12): the profile's scoring policy, once, here.
        let exam_test = if ctx.is_exam() {
            self.exam.test(ExamLanguage::of_layout(layout))
        } else {
            None
        };
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        let exam_outcome = match (self.exam.profile.filter(|_| ctx.is_exam()), exam_test) {
            (Some(profile), Some(t)) => {
                let input = radha_exams::TestInput {
                    gross_wpm: summary.wpm,
                    net_wpm: summary.net_wpm,
                    accuracy: summary.accuracy,
                    words_typed: count(live.words),
                    words_correct: count(live.words_correct),
                    full_mistakes: count(full),
                    half_mistakes: count(half),
                    elapsed_seconds: summary.elapsed_seconds,
                };
                let out = exam_mode::outcome(profile, t, &input);
                log::info!(
                    "Exam result under {}: {:?} {:?}",
                    profile.key(),
                    out.measure,
                    out.verdict
                );
                Some((profile, out))
            }
            _ => None,
        };
        let day: String = iso_utc(std::time::SystemTime::now())
            .chars()
            .take(10)
            .collect();

        // Hindi Learn beginner progress (HL-01, HL-05, HL-06, HL-07).
        let mut learn_line = String::new();
        let mut unlocked_next = false;
        if ctx.mode == PracticeMode::Beginner && layout != LayoutId::Qwerty {
            let steps = hindi_learn::steps(layout);
            let tallies: Vec<(String, u32, u32)> = p
                .unit_tallies()
                .into_iter()
                .map(|(u, a, e, _)| (u, a, e))
                .collect();
            let step = match self.drill {
                Some(Drill::Step(i)) => Some(i),
                _ => None,
            };
            let secs = summary.elapsed_seconds.round().clamp(0.0, 86_400.0) as u32;
            let outcome = self.learn.layout_mut(layout).record(
                step,
                steps.len(),
                summary.accuracy,
                summary.characters_typed,
                finished,
                &tallies,
                &day,
                secs,
            );
            unlocked_next = outcome.unlocked_next;
            if let Err(e) = self.learn.save(&self.learn_path) {
                log::warn!("Could not save Hindi Learn progress: {e}");
            }
            learn_line = match step {
                Some(i) if outcome.unlocked_next => format!(" Step {} unlocked.", i + 2),
                Some(_)
                    if !hindi_learn::unlocks(
                        summary.accuracy,
                        summary.characters_typed,
                        finished,
                    ) =>
                {
                    format!(
                        " Unlock needs {:.0}% over {}+ characters, finished.",
                        hindi_learn::UNLOCK_ACCURACY,
                        hindi_learn::UNLOCK_MIN_CHARS
                    )
                }
                _ => String::new(),
            };
            learn_line.push_str(&format!(" Guide ramp: {}.", outcome.guide.slug()));
        }

        // The one database write (RULES 14): exercises only, not drills.
        let (status, new_best) = match ctx.exercise_id {
            _ if self.drill.is_some() => (
                "Generated drill: your Learn path was updated (not an exercise result)".to_owned(),
                false,
            ),
            // Exam passages are not `exercises` rows: an exam result lives
            // on this screen and in its scorecard, under its profile key.
            _ if ctx.is_exam() => (
                format!(
                    "Scored under {}. Exam results are not saved to your progress.",
                    self.exam
                        .profile
                        .map_or_else(|| "no profile".to_owned(), |p| p.key())
                ),
                false,
            ),
            _ if self.custom.is_some() => (
                "Custom passage: practice only, not saved to your progress".to_owned(),
                false,
            ),
            None => (
                "Nothing to save: no exercise was recorded".to_owned(),
                false,
            ),
            Some(exercise_id) => {
                let mut record = p.record(self.profile_id, exercise_id, ctx.product, now);
                record.mode = Some(ctx.mode.slug());
                record.guide_mode = self.active_guide().slug();
                // Same transaction as the session (RULES 14, RS-11).
                record.flow = flow_report.as_ref().map(rhythm::record);
                match self.db.borrow_mut().save_session(&record) {
                    Ok(s) => {
                        saved_ok = true;
                        log::info!(
                            "Session {} saved ({:.1} net WPM, {:.1}% accuracy)",
                            s.session_id,
                            summary.net_wpm,
                            summary.accuracy
                        );
                        ("Saved to your progress".to_owned(), s.new_exercise_best)
                    }
                    Err(e) => {
                        log::error!("Could not save the session: {e}");
                        (format!("Could not save this result: {e}"), false)
                    }
                }
            }
        };
        recovery::clear(&self.journal_path);

        // Verdict (RS-01): practice-honest.
        let cert = if ctx.is_exam() {
            exam_mode::test_certification(self.exam.profile, exam_test)
        } else {
            Certification::Practice
        };
        let difficulty = self.exercise.as_ref().and_then(|e| e.difficulty);
        let (mut headline, mut detail) = report::verdict(
            ctx.mode,
            cert,
            summary.net_wpm,
            summary.accuracy,
            suggested_targets(layout, difficulty),
        );
        let mut good = !headline.starts_with("Below");
        if let Some((profile, out)) = &exam_outcome {
            (headline, detail, good) = exam_mode::verdict_text(profile, out);
        }
        if summary.completion == Some(CompletionReason::TimeUp) {
            detail = format!("Time up. {detail}");
        }
        detail.push_str(&learn_line);
        self.audio.play(if good || unlocked_next {
            Sound::Success
        } else {
            Sound::Complete
        });

        // Minute curve (RS-02), both views, drawn by `write_chart` below.
        let points = p.progress_points();
        self.minute_wpm = report::minute_wpm(&points);
        self.minute_kdph = report::minute_kdph(&points);

        // Penalty audit (RS-03) from the session's scoring policy.
        let mut audit = report::penalty_audit(
            report::ScoringPolicy::Practice82,
            cert,
            summary.wpm,
            summary.net_wpm,
            summary.errors,
            summary.backspaces,
            summary.elapsed_seconds,
        );
        // An exam scored by its own policy shows that policy's formula; the
        // engine's Net WPM (PRD 8.2) stays in the figures.
        if let Some((profile, out)) = exam_outcome
            .as_ref()
            .filter(|(_, o)| o.policy != radha_exams::PolicyKind::PracticeOnly)
        {
            audit.formula = out.formula.clone();
            audit.worked = out.worked.clone();
            audit.note = format!(
                "Scored by the {} policy of {}. {}",
                out.policy.as_str(),
                profile.key(),
                audit.note
            );
        }

        // Diff (RS-05), heatmap (RS-06), recovery drill (RS-07).
        self.diff = p.word_diff();
        let count = |k: DiffKind| self.diff.iter().filter(|d| d.kind == k).count();
        let diff_summary = format!(
            "{} correct · {} half · {} full · {} omitted",
            count(DiffKind::Correct),
            count(DiffKind::Half),
            count(DiffKind::Full),
            count(DiffKind::Omitted)
        );
        let mut per_key: HashMap<char, (u32, u32)> = HashMap::new();
        // Mean latency per physical key (RS-06), weighted by attempts.
        let mut per_key_latency: HashMap<char, report::KeyLatency> = HashMap::new();
        for (unit, attempts, errors, latency) in p.unit_tallies() {
            if unit.trim().is_empty() {
                continue;
            }
            for k in p.physical_keys_for(&unit) {
                let e = per_key.entry(k).or_default();
                e.0 = e.0.saturating_add(attempts);
                e.1 = e.1.saturating_add(errors);
                per_key_latency.entry(k).or_default().add(latency, attempts);
            }
        }
        let mut weakest: Option<(f64, String)> = None;
        let mut finger_keys: Vec<(usize, u32, u32)> = Vec::new();
        let heat_rows: Vec<HeatRow> = board(layout)
            .into_iter()
            .map(|row| HeatRow {
                keys: model(
                    row.into_iter()
                        .map(|k| {
                            let base = k.id.chars().next().unwrap_or(' ');
                            let (a, e) = per_key.get(&base).copied().unwrap_or((0, 0));
                            let (acc, class) = report::heat(a, e);
                            finger_keys.push((report::finger_slot(k.finger, k.left), a, e));
                            let finger = ["little", "ring", "middle", "index", "thumb"]
                                .get(usize::from(k.finger))
                                .copied()
                                .unwrap_or("");
                            let hand = if k.left { "left" } else { "right" };
                            if e > 0 && weakest.as_ref().is_none_or(|(w, _)| acc < *w) {
                                weakest = Some((
                                    acc,
                                    format!(
                                        "Weakest key: {} {} at {acc:.0}% ({hand} {finger} finger).",
                                        k.legend, k.glyph,
                                    ),
                                ));
                            }
                            let detail = if a == 0 {
                                format!("{} {} · not typed this time", k.legend, k.glyph)
                            } else {
                                let latency = per_key_latency
                                    .get(&base)
                                    .and_then(report::KeyLatency::mean_ms)
                                    .map(|ms| format!(", {ms:.0} ms avg"))
                                    .unwrap_or_default();
                                format!(
                                    "{} {} · {a} presses, {e} missed, {acc:.0}%{latency} · {hand} {finger} finger",
                                    k.legend, k.glyph
                                )
                            };
                            HeatKey {
                                detail: detail.trim().replace("  ", " ").into(),
                                legend: k.legend.as_str().into(),
                                glyph: if layout == LayoutId::Qwerty {
                                    SharedString::new()
                                } else {
                                    k.glyph.as_str().into()
                                },
                                pct: if a == 0 {
                                    SharedString::new()
                                } else {
                                    format!("{acc:.0}%").into()
                                },
                                cls: class.code(),
                            }
                        })
                        .collect(),
                ),
            })
            .collect();
        // Finger balance (RS-06): share of presses relative to the busiest
        // finger, coloured by that finger's accuracy.
        let balance = report::finger_balance(&finger_keys);
        let busiest = balance.iter().map(|f| f.share).fold(0.0_f64, f64::max);
        let fingers: Vec<FingerBar> = balance
            .iter()
            .map(|f| {
                let (acc, class) = report::heat(f.presses, f.errors);
                FingerBar {
                    label: f.label.into(),
                    pct: if f.presses == 0 {
                        "—".into()
                    } else {
                        format!("{acc:.0}%").into()
                    },
                    level: if busiest > 0.0 {
                        (f.share / busiest) as f32
                    } else {
                        0.0
                    },
                    cls: class.code(),
                }
            })
            .collect();
        let heat_note = weakest.map_or_else(
            || "No key below 100% this time.".to_owned(),
            |(_, s)| format!("{s} A multi-key akshar counts on each of its keys."),
        );
        let weak_units: Vec<String> = summary.weak_keys.iter().map(|k| k.unit.clone()).collect();
        self.recovery_text = report::recovery_drill(&self.diff, &weak_units);
        // Typing Flow: the recommended drill (real words of this passage
        // carrying the weak keys) replaces the generic recovery drill.
        let flow_drill = flow_report.as_ref().and_then(|r| {
            let source = self.exercise.as_ref().map_or("", |e| e.content.as_str());
            radha_core::flow::drill(r, source, &self.flow_cfg)
        });
        if let Some(d) = &flow_drill {
            self.recovery_text = Some(d.text.clone());
        }
        let flow_advice = flow_report
            .as_ref()
            .map(|r| radha_core::flow::recommend(r, summary.accuracy, &self.flow_cfg));
        let drill_preview = self
            .recovery_text
            .as_deref()
            .map(|t| format!("Recovery drill: {t}"))
            .unwrap_or_default();

        // Next: the next step for a step drill, else the next card.
        let (has_next, next_label) = match self.drill {
            Some(Drill::Step(i)) => (
                i + 1 < hindi_learn::steps(layout).len() && i < self.learn.layout(layout).unlocked,
                "Next step",
            ),
            Some(_) => (false, "Next Exercise"),
            None => (self.adjacent_card(1).is_some(), "Next Exercise"),
        };
        let (title, subtitle) = self
            .exercise
            .as_ref()
            .map(|e| {
                let title = session_title(
                    ctx.is_exam(),
                    self.drill.is_some(),
                    self.custom.is_some(),
                    e,
                );
                if ctx.is_exam() {
                    (title, self.exam.title.clone())
                } else {
                    (title, e.product.display_name().to_owned())
                }
            })
            .unwrap_or_default();
        let peak_streak = self.streak.peak;

        // Export scorecard (RS-09) and candidate row (RS-10, exams only).
        let exam = ctx.is_exam();
        let candidate = if exam {
            candidate_line(self.candidate().as_ref())
        } else {
            String::new()
        };
        let completed_iso = iso_utc(std::time::SystemTime::now());
        // Exam rows: the profile version the result was scored under
        // (`profile_id@version`, EM-12), the exam measure and the verdict.
        let mut exam_rows: Vec<(&'static str, String)> = Vec::new();
        if let Some((profile, out)) = &exam_outcome {
            exam_rows.push(("Exam profile", profile.key()));
            if let Some((label, v)) = out.measure {
                exam_rows.push(("Exam measure", format!("{v:.2} {label}")));
            }
            exam_rows.push((
                "Verdict",
                match out.verdict {
                    Some(radha_exams::Verdict::Qualified) => "Qualified".to_owned(),
                    Some(radha_exams::Verdict::NotQualified) => "Not qualified".to_owned(),
                    None => "No pass or fail (practice only)".to_owned(),
                },
            ));
        }
        let mut card = report::Scorecard {
            cert,
            product: if exam {
                self.exam.title.clone()
            } else {
                subtitle.clone()
            },
            exercise: title.clone(),
            mode: ctx.mode.label().to_owned(),
            layout: layout.display_name().to_owned(),
            date: scorecard_date(&completed_iso),
            figures: vec![
                ("Gross WPM", format!("{:.1}", summary.wpm)),
                ("Net WPM", format!("{:.1}", summary.net_wpm)),
                ("Accuracy", format!("{:.1}%", summary.accuracy)),
                ("KDPH", format!("{:.0}", live.kdph)),
                (
                    "Words (correct / typed)",
                    format!("{}/{}", live.words_correct, live.words),
                ),
                ("Full mistakes", full.to_string()),
                ("Half mistakes", half.to_string()),
                (
                    "Error %",
                    format!("{:.1}%", report::error_percent(summary.accuracy)),
                ),
                ("Backspaces", summary.backspaces.to_string()),
                ("Peak streak", peak_streak.to_string()),
                ("Active time", clock(summary.elapsed_seconds)),
                ("Consistency", format!("{:.0}%", summary.consistency)),
            ],
            audit: audit.clone(),
            mistakes: self
                .diff
                .iter()
                .filter(|d| d.kind != DiffKind::Correct)
                .cloned()
                .collect(),
            candidate: None,
        };
        card.figures.extend(exam_rows);
        let stamp: String = completed_iso
            .chars()
            .filter(char::is_ascii_digit)
            .take(12)
            .collect();
        let export_path = self
            .export_dir
            .join(format!(
                "{}-{stamp}.html",
                crate::export::file_stem(&format!("{subtitle} {title}"))
            ))
            .display()
            .to_string();
        self.with_window(|w| {
            let v = w.global::<ResultView>();
            v.set_title(title.into());
            v.set_subtitle(subtitle.into());
            v.set_headline(headline.into());
            v.set_verdict_detail(detail.into());
            v.set_cert_pill(cert.pill().into());
            v.set_cert_level(cert.badge().into());
            v.set_good(good);
            v.set_wpm(format!("{:.0}", summary.wpm).into());
            v.set_net_wpm(format!("{:.0}", summary.net_wpm).into());
            v.set_kdph(format!("{:.0}", live.kdph).into());
            v.set_words(format!("{}/{}", live.words_correct, live.words).into());
            v.set_full_mistakes(full.to_string().into());
            v.set_half_mistakes(half.to_string().into());
            v.set_error_pct(format!("{:.1}%", report::error_percent(summary.accuracy)).into());
            v.set_backspaces(summary.backspaces.to_string().into());
            v.set_peak_streak(peak_streak.to_string().into());
            v.set_accuracy(format!("{:.1}%", summary.accuracy).into());
            v.set_accuracy_value(summary.accuracy as f32);
            v.set_accuracy_arc(ring_arc((summary.accuracy / 100.0) as f32).into());
            v.set_errors(summary.errors.to_string().into());
            v.set_consistency(format!("{:.0}%", summary.consistency).into());
            v.set_time(clock(summary.elapsed_seconds).into());
            v.set_word_accuracy(format!("{:.0}%", summary.word_accuracy).into());
            v.set_audit_formula(audit.formula.as_str().into());
            v.set_audit_worked(audit.worked.as_str().into());
            v.set_audit_raw(audit.raw.to_string().into());
            v.set_audit_exempted(audit.exempted.to_string().into());
            v.set_audit_penalised(audit.penalised.to_string().into());
            v.set_audit_note(audit.note.as_str().into());
            v.set_exam(exam);
            v.set_candidate(candidate.into());
            v.set_scorecard_title(card.title().into());
            v.set_export_path(export_path.into());
            v.set_export_status("".into());
            v.set_export_open(false);
            v.set_diff_summary(diff_summary.into());
            v.set_heat_rows(model(heat_rows));
            v.set_heat_note(heat_note.into());
            v.set_heat_detail("".into());
            v.set_fingers(model(fingers));
            v.set_weak_keys(model(
                summary
                    .weak_keys
                    .iter()
                    .map(|k| WeakKeyRow {
                        unit: k.unit.as_str().into(),
                        keys: k.keys.as_str().into(),
                        detail: format!("missed {} of {}", k.errors, k.attempts).into(),
                    })
                    .collect(),
            ));
            v.set_drill_available(!drill_preview.is_empty());
            v.set_drill_preview(drill_preview.into());
            v.set_devanagari(devanagari);
            v.set_save_status(status.into());
            v.set_new_best(new_best);
            v.set_has_next(has_next);
            v.set_next_label(next_label.into());
        });
        self.scorecard = Some(card);
        self.write_chart("wpm");
        self.write_diff("annotated");
        if saved_ok {
            // The Home hero reflects this session (computed once, here).
            self.with_window(|w| super::utility::refresh_home(w, &self.db, self.profile_id));
        }
        if let (Some(r), Some(advice)) = (&flow_report, &flow_advice) {
            let compare = self.drill.is_none() && self.custom.is_none();
            self.write_flow_result(r, flow_drill.as_ref(), advice, compare.then_some(flow_prev));
            if let (true, Some(s), Some(id)) = (saved_ok, r.smoothness, ctx.exercise_id) {
                // The chooser shows this session at once (no reload).
                if let Some(c) = self.cards.iter_mut().find(|c| c.id == id) {
                    let best = c.flow.map_or(s, |(_, b)| b.max(s));
                    c.flow = Some((s.clamp(0.0, 100.0), best.clamp(0.0, 100.0)));
                }
            }
            if saved_ok && r.smoothness.is_some() {
                // The Home card shows this session's real smoothness.
                let text = rhythm::home_smoothness(r.smoothness);
                self.with_window(|w| w.global::<Products>().set_flow_smoothness(text.into()));
            }
        }
        if let Some(screen) = self.nav.finish_session() {
            self.show(&screen);
        }
    }

    /// The diff inspector in one of its three views.
    fn write_diff(&self, view: &str) {
        let view = match view {
            "mistakes" | "table" => view,
            _ => "annotated",
        };
        let word = |d: &WordDiff| DiffWord {
            expected: d.expected.as_str().into(),
            typed: d.typed.as_str().into(),
            kind: d.kind.code(),
            label: d.label.into(),
        };
        let shown: Vec<&WordDiff> = self
            .diff
            .iter()
            .filter(|d| view != "mistakes" || d.kind != DiffKind::Correct)
            .collect();
        let mut lines: Vec<DiffLine> = Vec::new();
        let mut current: Vec<DiffWord> = Vec::new();
        let mut width = 0;
        for d in &shown {
            let w = d.expected.chars().count()
                + if d.kind == DiffKind::Correct {
                    0
                } else {
                    d.typed.chars().count() + 4
                }
                + 2;
            if width + w > 64 && !current.is_empty() {
                lines.push(DiffLine {
                    words: model(std::mem::take(&mut current)),
                });
                width = 0;
            }
            width += w;
            current.push(word(d));
        }
        if !current.is_empty() {
            lines.push(DiffLine {
                words: model(current),
            });
        }
        let rows: Vec<DiffWord> = self.diff.iter().map(word).collect();
        self.with_window(|w| {
            let v = w.global::<ResultView>();
            v.set_diff_view(view.into());
            v.set_diff_lines(model(lines));
            v.set_diff_rows(model(rows));
        });
    }

    /// The card `dir` places after (1) or before (-1) the current exercise
    /// in the filtered order.
    fn adjacent_card(&self, dir: i64) -> Option<CardMeta> {
        let id = self.exercise.as_ref()?.id;
        let pos = self
            .shown
            .iter()
            .position(|&i| self.cards.get(i).is_some_and(|c| c.id == id))?;
        let target = i64::try_from(pos).ok()? + dir;
        let &card = self.shown.get(usize::try_from(target).ok()?)?;
        self.cards.get(card).cloned()
    }

    fn retry(&mut self) {
        if let Some(d) = self.drill {
            self.start_drill(d);
            return;
        }
        let (Some(exercise), Some(ctx)) = (self.exercise.clone(), self.session_ctx()) else {
            return;
        };
        self.start_exercise(exercise, ctx, None);
    }

    /// Previous (-1) or next (1) exercise or step.
    fn go_adjacent(&mut self, dir: i64) {
        match self.drill {
            Some(Drill::Step(i)) => {
                let target = i64::try_from(i).unwrap_or(0) + dir;
                if let Ok(t) = usize::try_from(target) {
                    self.start_drill(Drill::Step(t));
                }
            }
            Some(_) => {}
            None => {
                let Some(ctx) = self.session_ctx() else {
                    return;
                };
                let Some(meta) = self.adjacent_card(dir) else {
                    return;
                };
                match self.load_text(&ctx, &meta) {
                    Ok(exercise) => self.start_exercise(exercise, ctx, None),
                    Err(e) => log::error!("Could not load the exercise: {e}"),
                }
            }
        }
        self.focus_keys();
    }

    // --- Typing Flow (P07) ------------------------------------------------------

    /// Whether the current screen belongs to Typing Flow (not an exam).
    fn is_flow(&self) -> bool {
        self.nav
            .current()
            .context()
            .is_some_and(|c| c.product == ProductId::TypingFlow && !c.is_exam())
    }

    /// The Home card's smoothness preview: the last real figure from the
    /// database, "—" when there is none (never a made-up number).
    fn write_home_smoothness(&self) {
        let last = match self.db.try_borrow() {
            Ok(db) => db
                .last_flow_smoothness(self.profile_id)
                .unwrap_or_else(|e| {
                    log::warn!("Could not read the last Flow smoothness: {e}");
                    None
                }),
            Err(_) => None,
        };
        let text = rhythm::home_smoothness(last);
        self.with_window(|w| w.global::<Products>().set_flow_smoothness(text.into()));
    }

    /// The selected card's position in the filtered list.
    fn draft_position(&self) -> Option<usize> {
        let id = self.draft()?.exercise_id?;
        self.shown
            .iter()
            .position(|&i| self.cards.get(i).is_some_and(|c| c.id == id))
    }

    /// Flow Exercise: carousel position and the dial's preset.
    fn write_flow_nav(&self) {
        if !self.is_flow() || self.draft().is_none() {
            return;
        }
        let total = self.shown.len();
        let pos = self.draft_position();
        let text = match pos {
            Some(p) => format!("{} of {total}", p + 1),
            None if total == 0 => "No flows here".to_owned(),
            None => format!("{total} flows"),
        };
        // From the draft's own timer, not the studio: a restored session
        // sets the timer directly, and the dial must match the readout.
        let dial = self
            .draft()
            .and_then(|c| rhythm::dial_index(c.timer))
            .and_then(|i| i32::try_from(i).ok())
            .unwrap_or(-1);
        let selected = self.draft().and_then(|c| c.exercise_id);
        let history = self
            .cards
            .iter()
            .find(|c| Some(c.id) == selected)
            .and_then(|c| c.flow);
        let sel = match history {
            Some(_) => rhythm::card_rhythm(history),
            None => "No Flow rhythm on this one yet".to_owned(),
        };
        self.with_window(|w| {
            let v = w.global::<FlowView>();
            v.set_sel_rhythm(sel.into());
            v.set_position(text.into());
            v.set_dial_index(dial);
        });
    }

    /// Flow Exercise carousel: the previous (-1) or next (1) flow.
    fn flow_step(&mut self, dir: i64) {
        if !matches!(self.nav.current(), Screen::Exercise(_)) || !self.is_flow() {
            return;
        }
        let target = match self.draft_position() {
            Some(p) => i64::try_from(p).unwrap_or(0) + dir.signum(),
            None => 0,
        };
        if let Ok(t) = usize::try_from(target) {
            if t < self.shown.len() {
                self.select(t);
            }
        }
    }

    /// The Flow word stream's width changed: rewrap.
    fn flow_resized(&mut self, width: f32) {
        if (width - self.flow_w).abs() < 0.5 {
            return;
        }
        self.flow_w = width;
        if self.is_flow() && self.practice.is_some() && self.fit_passage() {
            self.render(Instant::now());
        }
    }

    /// A fresh rhythm for a Flow session (reusing the tracker's buffers);
    /// none for any other product.
    fn reset_rhythm(&mut self) {
        if !self.is_flow() || self.practice.is_none() {
            self.rhythm = None;
            return;
        }
        match self.rhythm.as_mut() {
            Some(t) => t.reset(),
            None => self.rhythm = Some(FlowTracker::new(self.flow_cfg)),
        }
        self.flow_label = None;
        self.flow_live_pct = None;
        self.flow_scored = 0;
        self.live_flow.reset();
        // Force every bar and strip value to be rewritten once.
        self.bars_shown = [(u16::MAX, false); WAVE_BARS];
        self.flow_strip = (-1, u32::MAX, u32::MAX, u32::MAX);
        let cfg = self.flow_cfg;
        self.with_window(|w| {
            let v = w.global::<FlowView>();
            v.set_steady_at((cfg.steady_smoothness / 100.0) as f32);
            v.set_settling_at((cfg.settling_smoothness / 100.0) as f32);
            // Bars sit at interval / (2 x tempo): the smooth band is
            // tempo x (1 ± smooth_band), so ±smooth_band / 2 around 0.5.
            v.set_band((cfg.smooth_band / 2.0) as f32);
        });
        self.write_flow_live();
        self.write_flow_weak();
    }

    /// The live "keys missed most" callouts (rewritten only after a miss
    /// changed them).
    fn write_flow_weak(&self) {
        let rows: Vec<FlowWeakKey> = self
            .live_flow
            .weak
            .iter()
            .flatten()
            .map(|&(k, n)| FlowWeakKey {
                key: rhythm::key_label(k).into(),
                detail: format!("{n} misses").into(),
            })
            .collect();
        self.with_window(|w| w.global::<FlowView>().set_live_weak(model(rows)));
    }

    /// Each Flow exercise's (last, best) smoothness, from one query.
    fn flow_history(&self) -> HashMap<i64, (f64, f64)> {
        let rows = match self.db.try_borrow() {
            Ok(db) => db.flow_rhythm_by_exercise(self.profile_id),
            Err(_) => return HashMap::new(),
        };
        match rows {
            Ok(rows) => rows
                .into_iter()
                .map(|r| (r.exercise_id, (r.last, r.best)))
                .collect(),
            Err(e) => {
                // History is decoration; the list still works without it.
                log::warn!("Could not read the Flow history: {e}");
                HashMap::new()
            }
        }
    }

    /// The Flow stream reported how many lines fit.
    fn flow_stream_rows(&mut self, rows: i32) {
        let rows = usize::try_from(rows).unwrap_or(1).clamp(1, 12);
        if rows != self.flow_rows {
            self.flow_rows = rows;
            if self.is_flow() && self.practice.is_some() {
                self.lines_cache.clear();
                self.render(Instant::now());
            }
        }
    }

    /// The live cadence wave and rhythm word. Writes only what changed; the
    /// tracker reads are allocation-free.
    fn write_flow_live(&mut self) {
        let Some(t) = self.rhythm.as_ref() else {
            return;
        };
        t.fill_bars(&mut self.bars_buf);
        let label = t.label();
        let pct = t
            .live_smoothness()
            .map(|s| s.round().clamp(0.0, 100.0) as u8);
        let scored = t.counts().scored;
        for (i, b) in self.bars_buf.iter().enumerate() {
            let shown = ((b.level * 1000.0) as u16, b.smooth);
            if self.bars_shown.get(i) != Some(&shown) {
                if let Some(slot) = self.bars_shown.get_mut(i) {
                    *slot = shown;
                }
                self.bars_model.set_row_data(
                    i,
                    FlowBar {
                        level: b.level,
                        smooth: b.smooth,
                    },
                );
            }
        }
        let pulse_changed = scored != self.flow_scored;
        if pulse_changed {
            self.flow_scored = scored;
            self.pulse = !self.pulse;
        }
        let label_changed = self.flow_label != Some(label);
        let pct_changed = self.flow_live_pct != pct;
        let tempo = t
            .tempo_ms()
            .map_or(0, |m| m.round().clamp(0.0, 5_000.0) as i32);
        let strip = (
            tempo,
            self.live_flow.streak,
            self.live_flow.best_streak,
            self.live_flow.hesitations(),
        );
        let strip_changed = strip != self.flow_strip;
        if !(pulse_changed || label_changed || pct_changed || strip_changed) {
            return;
        }
        let old_strip = self.flow_strip;
        self.flow_strip = strip;
        self.flow_label = Some(label);
        self.flow_live_pct = pct;
        let pulse = self.pulse;
        let pace = (strip.0 != old_strip.0)
            .then(|| rhythm::pace_text((strip.0 > 0).then_some(f64::from(strip.0))));
        self.with_window(|w| {
            let v = w.global::<FlowView>();
            if pulse_changed {
                v.set_pulse(pulse);
            }
            if let Some(pace) = pace {
                v.set_tempo_ms(strip.0);
                v.set_pace(pace.into());
            }
            if strip.1 != old_strip.1 {
                v.set_streak(i32::try_from(strip.1).unwrap_or(i32::MAX));
            }
            if strip.2 != old_strip.2 {
                v.set_best_streak(i32::try_from(strip.2).unwrap_or(i32::MAX));
            }
            if strip.3 != old_strip.3 {
                v.set_live_hesitations(i32::try_from(strip.3).unwrap_or(i32::MAX));
            }
            if label_changed {
                v.set_rhythm_word(label.word().into());
                v.set_rhythm_tone(label.tone().into());
            }
            if pct_changed {
                v.set_live_smoothness(
                    pct.map_or_else(|| "—".to_owned(), |p| format!("{p}%"))
                        .into(),
                );
                v.set_live_level(pct.map_or(-1.0, |p| f32::from(p) / 100.0));
            }
        });
    }

    /// The Flow Result's rhythm analysis.
    /// The bests before this session: (this exercise, any flow).
    fn flow_bests(&self, exercise_id: i64) -> (Option<f64>, Option<f64>) {
        let Ok(db) = self.db.try_borrow() else {
            return (None, None);
        };
        let here = match db.flow_rhythm_by_exercise(self.profile_id) {
            Ok(rows) => rows
                .into_iter()
                .find(|r| r.exercise_id == exercise_id)
                .map(|r| r.best),
            Err(e) => {
                log::warn!("Could not read the Flow history: {e}");
                None
            }
        };
        let any = db
            .best_flow_smoothness(self.profile_id)
            .unwrap_or_else(|e| {
                log::warn!("Could not read the best Flow smoothness: {e}");
                None
            });
        (here, any)
    }

    fn write_flow_result(
        &self,
        report: &FlowReport,
        drill: Option<&radha_core::flow::FlowDrill>,
        advice: &radha_core::flow::Recommendation,
        prev: Option<(Option<f64>, Option<f64>)>,
    ) {
        let cadence: Vec<FlowBar> = rhythm::cadence_chart(
            &report.corridor,
            report.mean_ms,
            self.flow_cfg.steady_smoothness,
        )
        .into_iter()
        .map(|s| FlowBar {
            level: s.level,
            smooth: s.steady,
        })
        .collect();
        let text: String = self
            .practice
            .as_ref()
            .map(|p| p.units().concat())
            .unwrap_or_default();
        let words: Vec<&str> = text.split_whitespace().collect();
        let points = rhythm::hesitation_points(&words, &self.live_flow.marks, 12);
        let hes_note = if report.counts.hesitations == 0 {
            "No hesitation: every gap stayed under the hesitation limit.".to_owned()
        } else if points.is_empty() {
            "Hesitations were counted, but none could be placed on a word.".to_owned()
        } else {
            format!(
                "A hesitation is a gap over {:.1}× your beat. Practise these words until they flow.",
                self.flow_cfg.hesitation_factor
            )
        };
        let hes_rows: Vec<FlowHesitation> = points
            .iter()
            .map(|p| FlowHesitation {
                word: p.word.as_str().into(),
                detail: if p.count > 1 {
                    format!("word {} · ×{}", p.word_no, p.count)
                } else {
                    format!("word {}", p.word_no)
                }
                .into(),
                count: i32::try_from(p.count).unwrap_or(i32::MAX),
            })
            .collect();
        let (best, best_tone) = match prev {
            Some((here, any)) => rhythm::best_line(report.smoothness, here, any),
            None => (
                "Drills and custom passages are not compared with your bests".to_owned(),
                "none",
            ),
        };
        let level = report
            .smoothness
            .map_or(-1.0, |s| (s / 100.0).clamp(0.0, 1.0) as f32);
        let arc = if level >= 0.0 {
            ring_arc(level)
        } else {
            String::new()
        };
        let pace = rhythm::pace_text(report.mean_ms);
        let run = self.live_flow.best_streak.to_string();
        // The flow drill when there is one, else the generic recovery drill
        // (the same text "Start drill" types).
        let drill_text = drill
            .map(|d| d.text.clone())
            .or_else(|| self.recovery_text.clone())
            .unwrap_or_default();
        let corridor = rhythm::corridor_path(&report.corridor);
        let steady_y = rhythm::corridor_y(self.flow_cfg.steady_smoothness);
        let footer = rhythm::corridor_footer(report);
        let keys: Vec<FlowWeakKey> = report
            .weak_keys
            .iter()
            .map(|k| FlowWeakKey {
                key: rhythm::key_label(k.key).into(),
                detail: rhythm::weak_detail(k, report.mean_ms).into(),
            })
            .collect();
        let weak_note = if report.weak_keys.is_empty() {
            format!(
                "No key broke your flow often enough to call it weak (a key needs {}+ attempts).",
                self.flow_cfg.weak_min_attempts
            )
        } else {
            "Scored on misses, slowness against your own beat, and how often they recur.".to_owned()
        };
        let mean = report
            .mean_ms
            .map_or_else(|| "—".to_owned(), |m| format!("{m:.0} ms"));
        let stage = drill.map_or("", |d| rhythm::drill_stage(d.stage));
        self.with_window(|w| {
            let v = w.global::<FlowView>();
            v.set_headline(report.headline().into());
            v.set_smoothness(rhythm::percent(report.smoothness).into());
            v.set_cadence_variance(rhythm::variance(report.cadence_cv).into());
            v.set_mean_interval(mean.into());
            v.set_hesitations(report.counts.hesitations.to_string().into());
            v.set_pauses(report.counts.pauses.to_string().into());
            match &corridor {
                Some(c) => {
                    v.set_corridor_has_data(true);
                    v.set_corridor_line(c.line.as_str().into());
                    v.set_corridor_area(c.area.as_str().into());
                }
                None => v.set_corridor_has_data(false),
            }
            v.set_corridor_steady_y(steady_y);
            v.set_corridor_footer(footer.into());
            v.set_weak_keys(model(keys));
            v.set_weak_note(weak_note.into());
            v.set_advice_title(advice.headline.as_str().into());
            v.set_advice_detail(advice.detail.as_str().into());
            v.set_drill_stage(stage.into());
            v.set_drill_text(drill_text.into());
            v.set_cadence(model(cadence));
            v.set_hesitation_points(model(hes_rows));
            v.set_hesitation_note(hes_note.into());
            v.set_best_line(best.into());
            v.set_best_tone(best_tone.into());
            v.set_smooth_level(level);
            v.set_smooth_arc(arc.into());
            v.set_pace_result(pace.into());
            v.set_longest_run(run.into());
        });
    }
}

/// The Result's candidate row (RS-10); "" without a name.
fn candidate_line(c: Option<&(String, String)>) -> String {
    match c {
        Some((name, roll)) if !roll.trim().is_empty() => format!("Candidate: {name} · Roll {roll}"),
        Some((name, _)) => format!("Candidate: {name}"),
        None => String::new(),
    }
}

/// `2026-10-25T10:01:00Z` -> `2026-10-25 10:01 UTC`.
fn scorecard_date(iso: &str) -> String {
    match (iso.get(..10), iso.get(11..16)) {
        (Some(d), Some(t)) => format!("{d} {t} UTC"),
        _ => iso.to_owned(),
    }
}

/// One replayed key of a recovered session into the Flow rhythm engine,
/// exactly as the live key path feeds it (aimed unit, correctness, word).
fn replay_into_rhythm(
    t: &mut FlowTracker,
    live: &mut rhythm::LiveFlow,
    word_no: &[u32],
    r: &Replayed,
) {
    if !r.applied {
        return;
    }
    match r.key {
        LoggedKey::Char { c, .. } => {
            let correct = r.aimed == Some(c);
            t.key_at(r.at, r.aimed.unwrap_or('\0'), correct);
            let word = word_no.get(r.cursor).copied().unwrap_or(0);
            live.after_key(t, word, !correct);
        }
        LoggedKey::Backspace => t.correction_at(r.at),
        LoggedKey::DeadKey => {}
    }
}

/// The Typing header / Result title of a session.
fn session_title(exam: bool, drill: bool, custom: bool, e: &Exercise) -> String {
    if exam {
        format!("Passage {} · {}", e.number, e.title)
    } else if drill {
        e.title.clone()
    } else if custom {
        format!("Custom · {}", e.title)
    } else {
        format!("Exercise {} · {}", e.number, e.title)
    }
}

/// The header / dock counter "NN / last" of a course exercise: NN is the
/// number the title shows ("Exercise 27 · …"), `last` the highest number
/// in the course track (a track can start past 1: Classic starts at 27).
fn exercise_counter(number: u32, cards: &[CardMeta], track: Option<&str>) -> String {
    let last = cards
        .iter()
        .filter(|c| track.is_none_or(|t| c.track.as_deref() == Some(t)))
        .map(|c| c.number)
        .max()
        .unwrap_or(0)
        .max(number);
    format!("{number:02} / {last:02}")
}

/// The counter of a custom passage: "C2 / 3" (its place among the
/// layout's custom passages).
fn custom_counter(pos: Option<usize>, count: usize) -> String {
    match pos {
        Some(p) => format!("C{} / {count}", p + 1),
        None => format!("C— / {count}"),
    }
}

/// What a render found.
#[derive(Debug, Clone, Copy, Default)]
struct Rendered {
    new_error: bool,
    words: usize,
    presses: u32,
}

/// Step chips: passed below the open step, open, locked above it.
fn learn_chips(
    steps: &[hindi_learn::LearnStep],
    progress: &hindi_learn::LayoutProgress,
    current: Option<usize>,
) -> Vec<LearnChip> {
    steps
        .iter()
        .map(|s| {
            let passed_last = s.index + 1 == steps.len()
                && progress
                    .best
                    .get(&(s.index + 1).to_string())
                    .is_some_and(|&b| b >= hindi_learn::UNLOCK_ACCURACY);
            let state = if s.index < progress.unlocked || passed_last {
                2
            } else if s.index == progress.unlocked {
                1
            } else {
                0
            };
            LearnChip {
                label: s.legend.as_str().into(),
                glyph: s.glyph.as_str().into(),
                state,
                current: current == Some(s.index),
            }
        })
        .collect()
}

/// Connect every callback of the practice screens to `flow`.
fn wire(window: &AppWindow, flow: &Rc<RefCell<Flow>>) {
    let ex = window.global::<ExerciseView>();
    let f = Rc::clone(flow);
    ex.on_mode_selected(move |id| f.borrow_mut().set_mode(id.as_str()));
    let f = Rc::clone(flow);
    ex.on_layout_selected(move |id| f.borrow_mut().set_layout(id.as_str()));
    let f = Rc::clone(flow);
    ex.on_search_edited(move |text| f.borrow_mut().search(text.as_str()));
    let f = Rc::clone(flow);
    ex.on_difficulty_selected(move |id| f.borrow_mut().set_difficulty(id.as_str()));
    let f = Rc::clone(flow);
    ex.on_category_selected(move |id| f.borrow_mut().set_category(id.as_str()));
    let f = Rc::clone(flow);
    ex.on_select(move |i| {
        if let Ok(i) = usize::try_from(i) {
            f.borrow_mut().select(i);
        }
    });
    let f = Rc::clone(flow);
    ex.on_grid_scrolled(move |columns, height, scroll| {
        // A geometry report can arrive while another handler holds the flow
        // (a model write triggers a re-layout); the next report catches up.
        if let Ok(mut flow) = f.try_borrow_mut() {
            flow.grid_scrolled(usize::try_from(columns).unwrap_or(1), height, scroll);
        }
    });
    let f = Rc::clone(flow);
    ex.on_choose_timer(move |i| {
        if let Ok(i) = usize::try_from(i) {
            f.borrow_mut().choose_timer(i);
        }
    });
    let f = Rc::clone(flow);
    ex.on_timer_kind_selected(move |id| f.borrow_mut().set_timer_kind(id.as_str()));
    let f = Rc::clone(flow);
    ex.on_step_timer(move |delta| f.borrow_mut().step_timer(i64::from(delta)));
    let f = Rc::clone(flow);
    ex.on_custom_timer(move |text| f.borrow_mut().custom_timer(text.as_str()));
    let f = Rc::clone(flow);
    ex.on_toggle_no_backspace(move || f.borrow_mut().toggle_no_backspace());
    let f = Rc::clone(flow);
    ex.on_toggle_key_guide(move || f.borrow_mut().toggle_key_guide());
    let f = Rc::clone(flow);
    ex.on_guide_mode_selected(move |id| f.borrow_mut().set_guide_mode(id.as_str()));
    let f = Rc::clone(flow);
    ex.on_set_key_mode(move |key| f.borrow_mut().set_key_mode(key));
    let f = Rc::clone(flow);
    ex.on_start(move || f.borrow_mut().start());
    let f = Rc::clone(flow);
    ex.on_start_step(move |i| {
        if let Ok(i) = usize::try_from(i) {
            f.borrow_mut().start_drill(Drill::Step(i));
        }
    });
    let f = Rc::clone(flow);
    ex.on_start_trainer(move |id| f.borrow_mut().start_trainer(id.as_str()));

    let ty = window.global::<TypingView>();
    let f = Rc::clone(flow);
    ty.on_key_typed(move |text, control, alt| {
        if let Ok(mut flow) = f.try_borrow_mut() {
            flow.key(text.as_str(), control, alt);
        }
    });
    let f = Rc::clone(flow);
    ty.on_escape(move || f.borrow_mut().escape(Instant::now()));
    let f = Rc::clone(flow);
    ty.on_restart(move || f.borrow_mut().restart());
    let f = Rc::clone(flow);
    ty.on_toggle_pause(move || f.borrow_mut().toggle_pause(Instant::now()));
    let f = Rc::clone(flow);
    ty.on_finish(move || f.borrow_mut().finish(Instant::now()));
    let f = Rc::clone(flow);
    ty.on_prev(move || f.borrow_mut().go_adjacent(-1));
    let f = Rc::clone(flow);
    ty.on_next(move || f.borrow_mut().go_adjacent(1));
    let f = Rc::clone(flow);
    ty.on_request_focus(move || {
        if let Ok(flow) = f.try_borrow() {
            flow.focus_keys();
        }
    });
    // Focus events fire synchronously when Rust itself moves focus (while
    // the flow is borrowed); those are ours and need no handling.
    let f = Rc::clone(flow);
    ty.on_focus_changed(move |has, window| {
        if let Ok(mut flow) = f.try_borrow_mut() {
            flow.focus_changed(has, window);
        }
    });
    let f = Rc::clone(flow);
    ty.on_passage_resized(move |w| {
        if let Ok(mut flow) = f.try_borrow_mut() {
            flow.passage_resized(w);
        }
    });
    let f = Rc::clone(flow);
    ty.on_set_backspace(move |id| {
        if let Some(p) = BackspacePolicy::from_slug(id.as_str()) {
            f.borrow_mut().typing_set_backspace(p);
        }
    });
    let f = Rc::clone(flow);
    ty.on_set_highlight(move |id| {
        if let Some(h) = HighlightMode::from_slug(id.as_str()) {
            f.borrow_mut().typing_set_highlight(h);
        }
    });
    let f = Rc::clone(flow);
    ty.on_set_guide(move |id| f.borrow_mut().set_typing_guide(id.as_str()));
    let f = Rc::clone(flow);
    ty.on_font_step(move |d| f.borrow_mut().font_step(d));
    let f = Rc::clone(flow);
    ty.on_toggle_bold(move || f.borrow_mut().typing_toggle("bold"));
    let f = Rc::clone(flow);
    ty.on_toggle_paper(move || f.borrow_mut().typing_toggle("paper"));
    let f = Rc::clone(flow);
    ty.on_toggle_scrollbar(move || f.borrow_mut().typing_toggle("scrollbar"));
    let f = Rc::clone(flow);
    ty.on_toggle_auto_scroll(move || f.borrow_mut().typing_toggle("auto-scroll"));
    let f = Rc::clone(flow);
    ty.on_toggle_key_guide(move || f.borrow_mut().typing_toggle("key-guide"));
    let f = Rc::clone(flow);
    ty.on_toggle_sound(move || f.borrow_mut().typing_toggle("sound"));
    let f = Rc::clone(flow);
    ty.on_set_word_limit(move |on, text| f.borrow_mut().set_word_limit(on, text.as_str()));
    let f = Rc::clone(flow);
    ty.on_key_limit_step(move |d| {
        f.borrow_mut().set_rule(|r| {
            r.key_limit = TypingRules::step_limit(r.key_limit, KEY_LIMIT, 200, d > 0);
        });
    });
    let f = Rc::clone(flow);
    ty.on_toggle_focus_mode(move || f.borrow_mut().toggle_focus_mode());
    let f = Rc::clone(flow);
    ty.on_passage_rows(move |n| {
        if let Ok(mut flow) = f.try_borrow_mut() {
            flow.passage_rows(n);
        }
    });
    let f = Rc::clone(flow);
    ty.on_passage_scroll(move |d| {
        if let Ok(mut flow) = f.try_borrow_mut() {
            flow.passage_scroll(d);
        }
    });
    let f = Rc::clone(flow);
    ty.on_pick_exercise(move |i| f.borrow_mut().pick_exercise(i));
    let f = Rc::clone(flow);
    ty.on_set_layout(move |id| f.borrow_mut().typing_set_layout(id.as_str()));
    let f = Rc::clone(flow);
    ty.on_choose_timer(move |i| f.borrow_mut().typing_choose_timer(i));
    let f = Rc::clone(flow);
    ty.on_custom_timer(move |text| f.borrow_mut().typing_custom_timer(text.as_str()));
    let f = Rc::clone(flow);
    ty.on_printout(move || f.borrow_mut().printout());
    let f = Rc::clone(flow);
    ty.on_add_exercise(move |title, text| {
        f.borrow_mut().add_exercise(title.as_str(), text.as_str());
    });
    let f = Rc::clone(flow);
    ty.on_pause_for_dialog(move || f.borrow_mut().pause_for_dialog());
    let f = Rc::clone(flow);
    ty.on_toggle_rules(move || {
        if let Ok(flow) = f.try_borrow() {
            flow.focus_keys();
        }
    });
    let f = Rc::clone(flow);
    ty.on_toggle_fullscreen(move || f.borrow_mut().toggle_fullscreen());
    let f = Rc::clone(flow);
    ty.on_start_step(move |i| {
        if let Ok(i) = usize::try_from(i) {
            f.borrow_mut().start_drill(Drill::Step(i));
        }
    });

    let fv = window.global::<FlowView>();
    let f = Rc::clone(flow);
    fv.on_step(move |dir| f.borrow_mut().flow_step(i64::from(dir)));
    let f = Rc::clone(flow);
    fv.on_stream_resized(move |w| {
        if let Ok(mut flow) = f.try_borrow_mut() {
            flow.flow_resized(w);
        }
    });
    let f = Rc::clone(flow);
    fv.on_stream_rows(move |n| {
        if let Ok(mut flow) = f.try_borrow_mut() {
            flow.flow_stream_rows(n);
        }
    });

    let rv = window.global::<ResultView>();
    let f = Rc::clone(flow);
    rv.on_retry(move || f.borrow_mut().retry());
    let f = Rc::clone(flow);
    rv.on_next(move || f.borrow_mut().go_adjacent(1));
    let f = Rc::clone(flow);
    rv.on_list(move || f.borrow_mut().back());
    let f = Rc::clone(flow);
    rv.on_home(move || f.borrow_mut().go_home());
    let f = Rc::clone(flow);
    rv.on_drill(move || f.borrow_mut().start_drill(Drill::Recovery));
    let f = Rc::clone(flow);
    rv.on_diff_view_selected(move |id| f.borrow().write_diff(id.as_str()));
    let f = Rc::clone(flow);
    rv.on_chart_view_selected(move |id| f.borrow().write_chart(id.as_str()));
    let f = Rc::clone(flow);
    rv.on_export_scorecard(move |path| f.borrow().export_scorecard(path.as_str()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercise::EXAM_PRACTICE_SECONDS;
    use radha_core::{TimeLimit, TimerPreset};

    #[test]
    fn the_input_pane_keeps_the_last_lines_and_its_anchor_only_moves_forward() {
        let (a, head, tail) = input_lines("the cat", 0, 20, 3);
        assert_eq!((a, head.as_str(), tail.as_str()), (0, "", "the\u{a0}cat"));
        // 4 lines at 8 columns: the first scrolls away.
        let typed = "aaaa bbb cccc ddd eeee fff gggg hhh";
        let (a, head, tail) = input_lines(typed, 0, 8, 3);
        assert_eq!(head, "cccc ddd \neeee fff ");
        assert_eq!(tail, "gggg\u{a0}hhh");
        assert_eq!(&typed[a..], "cccc ddd eeee fff gggg hhh");
        let (a2, _, _) = input_lines(typed, a, 8, 3);
        assert_eq!(a2, a, "stable from the anchor");
        // Backspace past the anchor: a stale anchor is clamped, not a panic.
        let (a3, _, tail) = input_lines("राम", 99, 8, 3);
        assert_eq!((a3, tail.as_str()), (0, "राम"));
        let html = passage_print_html("<T>", "QWERTY", 2, "a & b");
        assert!(html.contains("&lt;T&gt;") && html.contains("a &amp; b"));
        assert!(!html.contains("http"), "no network in the printout");
    }

    #[test]
    fn the_input_pane_glides_one_wrapped_line_and_snaps_on_jumps() {
        let cols = 8;
        let (mut typed, mut anchor, mut mark, mut wraps) = (String::new(), 0, None, 0);
        let step_with =
            |typed: &str, cols: usize, anchor: &mut usize, mark: &mut Option<(usize, usize)>| {
                let (a, head, _) = input_lines(typed, *anchor, cols, INPUT_LINES + 1);
                *anchor = a;
                let r = typed_wrap_step(*mark, cols, typed, a, &head);
                *mark = Some((cols, tail_start(a, &head)));
                r
            };
        // "aaaa bbb " / "cccc ddd " / "eeee fff " / "gggg hhh": each wrap
        // typed glides one line.
        for c in "aaaa bbb cccc ddd eeee fff gggg hhh".chars() {
            typed.push(c);
            let (glide, step) = step_with(&typed, cols, &mut anchor, &mut mark);
            assert!(step == 0 || (glide && step == 1), "{typed:?}");
            wraps += step;
        }
        assert_eq!(wraps, 3);
        assert_eq!(mark, Some((cols, "aaaa bbb cccc ddd eeee fff ".len())));
        // Backspace back onto the line above glides down.
        typed.truncate("aaaa bbb cccc ddd eeee fff ".len());
        assert_eq!(step_with(&typed, cols, &mut anchor, &mut mark), (true, -1));
        // A rewrap (another width) and a restart snap.
        let (glide, _) = step_with(&typed, 12, &mut anchor, &mut mark);
        assert!(!glide, "rewrap");
        typed.clear();
        let (glide, step) = step_with(&typed, 12, &mut anchor, &mut mark);
        assert_eq!((glide, step), (false, -1), "restart");
    }

    #[test]
    fn timer_summaries_describe_the_mode() {
        let five = TimerMode::Countdown(TimeLimit::from_preset(TimerPreset::Min5));
        assert_eq!(timer_summary(five), "5:00 countdown");
        assert_eq!(timer_summary(TimerMode::Stopwatch), "stopwatch");
        assert_eq!(timer_summary(TimerMode::Off), "no timer");
        let custom = TimeLimit::parse_mm_ss("1:30").unwrap();
        assert_eq!(
            timer_summary(TimerMode::Countdown(custom)),
            "1:30 countdown"
        );
        assert_eq!(
            timer_for(PracticeMode::Exam, TimerMode::Off)
                .limit()
                .map(|l| l.seconds()),
            Some(EXAM_PRACTICE_SECONDS)
        );
    }

    #[test]
    fn empty_products_say_content_is_coming_without_inventing_it() {
        for p in [
            ProductId::TypingFlow,
            ProductId::NumberPunctuation,
            ProductId::HindiLearn(LayoutId::KrutiDev),
        ] {
            assert!(
                empty_message(Some(p)).1.contains("none were invented"),
                "{p:?}"
            );
        }
    }

    #[test]
    fn course_keys_separate_products_exams_and_languages() {
        let english = SessionContext::for_product(ProductId::EnglishClassic);
        assert_eq!(course_key(&english), "english-classic");
        let eng = SessionContext::for_exam("ssc-chsl", ProductId::EnglishClassic);
        let hin = SessionContext::for_exam("ssc-chsl", ProductId::HindiMastery(LayoutId::KrutiDev));
        let cbi =
            SessionContext::for_exam("ssc-chsl", ProductId::HindiMastery(LayoutId::RemingtonCbi));
        assert_eq!(course_key(&eng), "exam:ssc-chsl:eng");
        assert_eq!(course_key(&hin), "exam:ssc-chsl:hin");
        assert_eq!(
            course_key(&hin),
            course_key(&cbi),
            "layout does not change the pool"
        );
        assert_eq!(layout_slug(LayoutId::KrutiDev), "krutidev");
        assert_eq!(layout_slug(LayoutId::Qwerty), "");
    }

    #[test]
    fn the_typing_header_title_and_counter_name_the_same_exercise() {
        // English Typing Mastery TEST: categories Foundation 1–14, Skill
        // Builder 15–26, Practice 27–200 (174 cards). With the Practice
        // category chosen the first row is Exercise 27; the counter used to
        // show its row in the filtered list ("01 / 174") next to
        // "Exercise 27 · Technology Practice" (typing-v3-light.png).
        let card = |n: u32| CardMeta {
            id: i64::from(n) + 1000,
            number: n,
            title: if n == 27 {
                "Technology Practice".into()
            } else {
                format!("Passage {n}")
            },
            category: Some(
                match n {
                    1..=14 => "Foundation",
                    15..=26 => "Skill Builder",
                    _ => "Practice",
                }
                .into(),
            ),
            difficulty: None,
            status: radha_content::ContentStatus::Verified,
            track: Some(if n <= 26 { "beginner" } else { "classic" }.into()),
            word_count: None,
            best: None,
            completed: false,
            flow: None,
        };
        let cards: Vec<CardMeta> = (1..=200).map(card).collect();
        // The category filter (TEST mode lists every track) and the Classic
        // track filter both give the 174-row list starting at 27.
        let by_category = ListFilter {
            category: Some("Practice".into()),
            ..ListFilter::default()
        };
        let by_track = ListFilter {
            track: Some("classic"),
            ..ListFilter::default()
        };
        assert_eq!(
            filter_cards(&cards, &by_category),
            filter_cards(&cards, &by_track)
        );
        let filter = by_category;
        let shown = filter_cards(&cards, &filter);
        assert_eq!(shown.len(), 174);
        let first = &cards[shown[0]];
        let e = Exercise {
            id: first.id,
            product: ProductId::EnglishClassic,
            number: first.number,
            title: first.title.clone(),
            category: None,
            difficulty: None,
            content: "x".into(),
        };
        let title = session_title(false, false, false, &e);
        let counter = exercise_counter(e.number, &cards, filter.track);
        assert_eq!(title, "Exercise 27 · Technology Practice");
        assert_eq!(counter, "27 / 200");
        let in_title: u32 = title["Exercise ".len()..]
            .split(' ')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap();
        let in_counter: u32 = counter.split(" / ").next().unwrap().parse().unwrap();
        assert_eq!(in_title, in_counter, "{title} vs {counter}");
        // Every row of the list: the counter follows the title.
        for &i in &shown {
            let n = cards[i].number;
            assert!(exercise_counter(n, &cards, filter.track).starts_with(&format!("{n:02} /")));
        }
        // In the Classic track (CLASSIC mode) the total is still the
        // course's last number, so "27 / 200" reads the same.
        assert_eq!(exercise_counter(27, &cards, Some("classic")), "27 / 200");
        // Beginner track: "05 / 26"; an exam pool (no track): its last passage.
        assert_eq!(exercise_counter(5, &cards, Some("beginner")), "05 / 26");
        assert_eq!(exercise_counter(3, &cards, None), "03 / 200");
        // Custom passages say so in both places.
        let mut custom = e.clone();
        custom.number = 0;
        custom.title = "My notes".into();
        assert_eq!(
            session_title(false, false, true, &custom),
            "Custom · My notes"
        );
        assert_eq!(custom_counter(Some(1), 3), "C2 / 3");
        assert_eq!(
            session_title(true, false, false, &e),
            "Passage 27 · Technology Practice"
        );
    }

    #[test]
    fn a_recovered_flow_session_replays_its_journalled_keys_into_the_rhythm_engine() {
        let text = "the quick brown fox jumps over the lazy dog";
        let cfg = FlowConfig::default();
        let new_practice = || {
            Practice::new(
                LayoutId::Qwerty,
                text,
                TimerMode::Stopwatch,
                DisplayMode::Word,
            )
        };
        let mut p = new_practice().unwrap();
        let (word_no, _) = passage::word_numbers(p.units());
        // The live session, fed as `Flow::key` feeds it: uneven gaps, a
        // miss, two corrections.
        let (mut live_t, mut live_f) = (FlowTracker::new(cfg), rhythm::LiveFlow::new());
        let t0 = Instant::now();
        let gaps = [150u64, 182, 96, 240, 151, 133, 310, 170];
        let mut ms = 0u64;
        let mut keys: Vec<Option<char>> = "the quick brpwn".chars().map(Some).collect();
        keys.extend([None, None]);
        keys.extend("own fox jum".chars().map(Some));
        for (i, k) in keys.iter().enumerate() {
            ms += gaps[i % gaps.len()];
            let at = t0 + Duration::from_millis(ms);
            match *k {
                Some(c) => {
                    let cursor = p.view(at).cursor;
                    let aimed = p.units().get(cursor).and_then(|u| u.chars().next());
                    if matches!(p.press(c, false, at), KeyEffect::Applied { .. }) {
                        let correct = aimed == Some(c);
                        live_t.key_at(at, aimed.unwrap_or('\0'), correct);
                        live_f.after_key(&live_t, word_no[cursor], !correct);
                    }
                }
                None => {
                    if matches!(p.backspace(at), KeyEffect::Applied { .. }) {
                        live_t.correction_at(at);
                    }
                }
            }
        }
        let crash = t0 + Duration::from_millis(ms + 400);
        let j = Journal {
            product: ProductId::TypingFlow.slug().to_owned(),
            exercise_id: 3,
            mode: "classic".into(),
            timer: "stopwatch".into(),
            timer_secs: 0,
            key_mode: false,
            elapsed_seconds: p.view(crash).stats.elapsed_seconds,
            keys: recovery::encode(p.key_log()),
            key_ms: p.key_ms(),
        };
        let j: Journal = toml::from_str(&toml::to_string(&j).unwrap()).unwrap();
        assert_eq!(j.key_ms.len(), keys.len());

        // The next launch: replay into a fresh tracker.
        let later = Instant::now() + Duration::from_secs(60);
        let mut q = new_practice().unwrap();
        let (mut t, mut f) = (FlowTracker::new(cfg), rhythm::LiveFlow::new());
        let mut feed = |r: &Replayed| replay_into_rhythm(&mut t, &mut f, &word_no, r);
        q.replay_timed(
            &recovery::decode(&j.keys),
            &j.key_ms,
            j.elapsed_seconds,
            later,
            Some(&mut feed),
        );
        assert_eq!(q.typed(), p.typed());
        assert_eq!(q.state(), SessionState::Paused);
        let live = FlowReport::from_tracker(&live_t);
        assert!(live.smoothness.is_some(), "the live session was scored");
        assert_eq!(
            FlowReport::from_tracker(&t),
            live,
            "same rhythm as the live session"
        );
        assert_eq!(f, live_f, "same streaks, hesitation marks and weak keys");

        // An older journal (no key times) still replays the text, spaced
        // evenly; it is not mistaken for the real rhythm.
        let mut r = new_practice().unwrap();
        let mut even = FlowTracker::new(cfg);
        let mut ef = rhythm::LiveFlow::new();
        let mut feed = |x: &Replayed| replay_into_rhythm(&mut even, &mut ef, &word_no, x);
        r.replay_timed(
            &recovery::decode(&j.keys),
            &[],
            j.elapsed_seconds,
            later,
            Some(&mut feed),
        );
        assert_eq!(r.typed(), p.typed());
        assert_ne!(FlowReport::from_tracker(&even).mean_ms, live.mean_ms);
    }

    #[test]
    fn an_inscript_journal_resumes_its_borrowed_gail_passage() {
        let gail = ProductId::HindiMastery(LayoutId::RemingtonGail);
        let inscript = ProductId::HindiMastery(LayoutId::Inscript);
        let stored = |product| Exercise {
            id: 12,
            product,
            number: 12,
            title: "किसान".into(),
            category: None,
            difficulty: Some(1),
            content: "किसान".into(),
        };
        // The journal stores the slug; it must read back as InScript.
        assert_eq!(ProductId::from_slug(inscript.slug()).ok(), Some(inscript));
        let resumed = journaled_exercise(inscript, stored(gail), |e| {
            radha_layouts::inscript::can_type(&e.content)
        });
        assert_eq!(resumed.map(|e| e.product), Some(inscript));
        // A passage InScript cannot type key for key is not resumed.
        assert!(journaled_exercise(inscript, stored(gail), |_| false).is_none());
        // Own-course journals are unchanged; a mismatched row is refused.
        assert_eq!(
            journaled_exercise(gail, stored(gail), |_| false).map(|e| e.product),
            Some(gail)
        );
        let kruti = ProductId::HindiMastery(LayoutId::KrutiDev);
        assert!(journaled_exercise(inscript, stored(kruti), |_| true).is_none());
        assert!(journaled_exercise(gail, stored(inscript), |_| true).is_none());
    }
}
