//! Navigation state.
//!
//! # Why this lives in Rust
//!
//! `RULES.MD` Rule 16 keeps state logic out of the UI layer. The Slint side raises
//! *requests* ("this card was clicked", "go back") and this module decides what
//! screen results, then the resulting screen is written back to the UI. The UI can
//! therefore never display a screen the Rust side has not recorded.
//!
//! It also means the whole navigation model is unit-testable without starting a
//! window, which is what the tests at the bottom of this file do.
//!
//! # The four-screen rule
//!
//! `RULES.MD` Rule 1 fixes the core training flow:
//!
//! ```text
//! HOME -> EXERCISE -> TYPING -> RESULT
//! ```
//!
//! [`Screen`] models that flow. Utility surfaces (Settings, Profile, Analytics) are
//! deliberately **absent** from this enum: per `RULES.MD` Rule 21 they are
//! overlays, not screens, so they must not be able to participate in navigation
//! history. Representing them here would invite exactly the "5th screen" drift the
//! rule exists to prevent. Exam Mode is not a screen either (`RULES.MD` Rule 31):
//! it is the same flow with a [`SessionContext`] that names an exam profile.
//!
//! The debug component preview (`RADHA_START_SCREEN=preview`, see
//! [`component_preview_requested`]) is likewise an overlay, not a [`Screen`].
//!
//! # The session context
//!
//! Every screen below Home carries a [`SessionContext`]: product, mode,
//! exercise, timer and exam profile (HOME-11 needs exactly this to resume a
//! session). On Exercise it is the draft the user is editing; START copies it
//! into Typing, and Result keeps it, so Retry and "next exercise" are pure
//! navigation and back from Typing or Result lands on the same Exercise
//! screen with the same choices.
//!
//! # Escape
//!
//! [`escape_action`] decides what Escape means: nothing on Home, pause while
//! a typing session is running, back one level everywhere else (a paused or
//! not-yet-started session is abandoned; nothing is saved).

use radha_content::ProductId;
use radha_core::{DisplayMode, SessionState, TimerMode};

use crate::exercise::{mode_matrix, timer_for, PracticeMode, TimerStudio};

/// What an Exercise, Typing or Result screen is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    /// The product whose layout is typed on. For an exam: the carrier
    /// product of the chosen language and layout.
    pub product: ProductId,
    /// LEARN / TEST / FLOW / BEGINNER / CLASSIC / EXAM.
    pub mode: PracticeMode,
    /// The chosen exercise's database id (an exam passage id for exams);
    /// `None` until one is chosen.
    pub exercise_id: Option<i64>,
    /// Its 1-based number within the product or pool (0 when none).
    pub exercise_number: u32,
    /// Selected timer.
    pub timer: TimerMode,
    /// KEY or WORD colouring.
    pub display: DisplayMode,
    /// The exam profile, for exam practice.
    pub exam_profile_id: Option<String>,
    /// Strict rule: Backspace is ignored.
    pub no_backspace: bool,
}

impl SessionContext {
    /// A product's Exercise screen as it opens: the matrix default mode and
    /// its timer, nothing chosen yet.
    #[must_use]
    pub fn for_product(product: ProductId) -> Self {
        let mode = mode_matrix(product).default;
        SessionContext {
            product,
            mode,
            exercise_id: None,
            exercise_number: 0,
            timer: timer_for(mode, TimerStudio::default().mode()),
            display: DisplayMode::Word,
            exam_profile_id: None,
            no_backspace: false,
        }
    }

    /// An exam's Exercise screen, typed on `carrier`'s layout.
    #[must_use]
    pub fn for_exam(profile_id: &str, carrier: ProductId) -> Self {
        SessionContext {
            product: carrier,
            mode: PracticeMode::Exam,
            exercise_id: None,
            exercise_number: 0,
            timer: timer_for(PracticeMode::Exam, TimerMode::Off),
            display: DisplayMode::Word,
            exam_profile_id: Some(profile_id.to_owned()),
            no_backspace: false,
        }
    }

    /// Whether this is exam practice.
    #[must_use]
    pub fn is_exam(&self) -> bool {
        self.exam_profile_id.is_some()
    }

    /// Whether `other` belongs to the same course: the same exam profile, or
    /// the same (non-exam) product. An exam's language and layout may differ.
    #[must_use]
    pub fn same_course(&self, other: &SessionContext) -> bool {
        match (&self.exam_profile_id, &other.exam_profile_id) {
            (Some(a), Some(b)) => a == b,
            (None, None) => self.product == other.product,
            _ => false,
        }
    }
}

/// A screen in the core training flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// The product dashboard.
    Home,
    /// Exercise selection; the context is the draft being edited.
    Exercise(SessionContext),
    /// Typing an exercise.
    Typing(SessionContext),
    /// The result of a completed session.
    Result(SessionContext),
}

/// What an Escape press does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeAction {
    /// Nothing (Home is the root; Escape never closes the app).
    Ignore,
    /// Pause the running session; the user stays on the Typing screen.
    PauseTyping,
    /// Go back one level.
    Back,
}

/// Escape on `screen`, given the typing session's state when on Typing.
#[must_use]
pub fn escape_action(screen: &Screen, typing: Option<SessionState>) -> EscapeAction {
    match screen {
        Screen::Home => EscapeAction::Ignore,
        Screen::Typing(_) if typing == Some(SessionState::Active) => EscapeAction::PauseTyping,
        Screen::Exercise(_) | Screen::Typing(_) | Screen::Result(_) => EscapeAction::Back,
    }
}

impl Screen {
    /// The screen reached by going back one level, or `None` if already at the
    /// root.
    ///
    /// Deliberately derived from the screen itself rather than popped from a
    /// history stack. The flow is a fixed, shallow hierarchy, so "where does back
    /// go" has one correct answer per screen. A stack would allow states the
    /// product does not have — for example Home appearing twice, or backing out of
    /// Home into nothing.
    #[must_use]
    pub fn parent(&self) -> Option<Screen> {
        match self {
            Screen::Home => None,
            Screen::Exercise(_) => Some(Screen::Home),
            Screen::Typing(ctx) | Screen::Result(ctx) => Some(Screen::Exercise(ctx.clone())),
        }
    }

    /// The product this screen is showing, if any.
    #[must_use]
    pub fn product(&self) -> Option<ProductId> {
        self.context().map(|ctx| ctx.product)
    }

    /// The session context of every screen below Home.
    #[must_use]
    pub fn context(&self) -> Option<&SessionContext> {
        match self {
            Screen::Exercise(ctx) | Screen::Typing(ctx) | Screen::Result(ctx) => Some(ctx),
            Screen::Home => None,
        }
    }

    /// Stable identifier for the UI's screen switch.
    ///
    /// Slint cannot hold a Rust enum, so the active screen crosses the boundary as
    /// this string.
    #[must_use]
    pub fn slug(&self) -> &'static str {
        match self {
            Screen::Home => "home",
            Screen::Exercise(_) => "exercise",
            Screen::Typing(_) => "typing",
            Screen::Result(_) => "result",
        }
    }
}

/// The currently displayed screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationState {
    current: Screen,
}

impl Default for NavigationState {
    fn default() -> Self {
        NavigationState {
            current: Screen::Home,
        }
    }
}

/// Environment variable that overrides the screen the app opens on.
///
/// Format: `home`, or `exercise:<product-slug>`.
///
/// Debug builds only. A developer working on a screen should not have to click
/// through to reach it on every rebuild, and it makes any screen directly
/// reachable for visual inspection. Ignored entirely in release, so it cannot
/// affect a shipped build.
const START_SCREEN_ENV: &str = "RADHA_START_SCREEN";

/// [`START_SCREEN_ENV`] value that opens the debug component preview overlay.
///
/// The preview is an overlay drawn above Home, not a [`Screen`]: it never enters
/// navigation, so it cannot become a "5th screen" (`RULES.MD` Rules 1 and 21).
const PREVIEW_SPEC: &str = "preview";

/// Whether `spec` asks for the component preview (trimmed, ASCII case-insensitive).
fn is_preview_spec(spec: &str) -> bool {
    spec.trim().eq_ignore_ascii_case(PREVIEW_SPEC)
}

/// Whether the debug component preview overlay should open at startup.
///
/// True only in debug builds with `RADHA_START_SCREEN=preview`; release builds
/// always return false, so a shipped binary can never show it.
#[must_use]
pub fn component_preview_requested() -> bool {
    cfg!(debug_assertions)
        && std::env::var_os(START_SCREEN_ENV)
            .is_some_and(|raw| is_preview_spec(&raw.to_string_lossy()))
}

/// The utility overlay named by `overlay:<id>` (trimmed), if any.
fn overlay_spec(spec: &str) -> Option<&'static str> {
    let id = spec.trim().strip_prefix("overlay:")?;
    ["analytics", "profile", "settings", "candidate"]
        .into_iter()
        .find(|o| o.eq_ignore_ascii_case(id))
}

/// The utility overlay to open over Home at startup (debug builds only,
/// `RADHA_START_SCREEN=overlay:analytics` etc.), for visual inspection.
#[must_use]
pub fn start_overlay_requested() -> Option<&'static str> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os(START_SCREEN_ENV).and_then(|raw| overlay_spec(&raw.to_string_lossy()))
}

impl NavigationState {
    /// Start at Home, unless [`START_SCREEN_ENV`] overrides it in a debug build.
    #[must_use]
    pub fn new() -> Self {
        if cfg!(debug_assertions) {
            if let Some(raw) = std::env::var_os(START_SCREEN_ENV) {
                let spec = raw.to_string_lossy();
                // The preview overlay sits above Home; `ui::run` opens it via
                // `component_preview_requested`, so it is not a "bad screen".
                if is_preview_spec(&spec) || overlay_spec(&spec).is_some() {
                    return NavigationState::default();
                }
                match Self::parse_start_screen(&spec) {
                    Some(screen) => {
                        log::warn!("{START_SCREEN_ENV} is set — opening on {spec}");
                        return NavigationState { current: screen };
                    }
                    None => log::warn!(
                        "{START_SCREEN_ENV}='{spec}' is not a recognised screen — starting at Home"
                    ),
                }
            }
        }

        NavigationState::default()
    }

    /// Parse a [`START_SCREEN_ENV`] value.
    ///
    /// Returns `None` for anything unrecognised, so a typo degrades to Home rather
    /// than failing startup.
    fn parse_start_screen(spec: &str) -> Option<Screen> {
        let spec = spec.trim();

        if spec.eq_ignore_ascii_case("home") {
            return Some(Screen::Home);
        }

        let product_slug = spec.strip_prefix("exercise:")?;
        let product = radha_content::ProductId::from_slug(product_slug).ok()?;
        Some(Screen::Exercise(SessionContext::for_product(product)))
    }

    /// The active screen.
    #[must_use]
    pub fn current(&self) -> &Screen {
        &self.current
    }

    /// Open a product's Exercise screen with a fresh context.
    ///
    /// Returns the new screen.
    pub fn open_product(&mut self, product: ProductId) -> Screen {
        self.current = Screen::Exercise(SessionContext::for_product(product));
        log::debug!(
            "Navigated to {} ({})",
            product.display_name(),
            product.slug()
        );
        self.current.clone()
    }

    /// Open an exam's Exercise screen (exam practice, typed on `carrier`).
    pub fn open_exam(&mut self, profile_id: &str, carrier: ProductId) -> Screen {
        self.current = Screen::Exercise(SessionContext::for_exam(profile_id, carrier));
        log::debug!("Navigated to exam {profile_id} ({})", carrier.slug());
        self.current.clone()
    }

    /// The Exercise screen's draft context, to edit in place. `None` off the
    /// Exercise screen.
    pub fn draft_mut(&mut self) -> Option<&mut SessionContext> {
        match &mut self.current {
            Screen::Exercise(ctx) => Some(ctx),
            _ => None,
        }
    }

    /// Go back one level.
    ///
    /// Returns the new screen, or `None` if already at the root — which lets the
    /// caller distinguish "went back" from "nothing to go back to" and, for
    /// example, ignore an Escape key press on Home rather than closing the app.
    pub fn back(&mut self) -> Option<Screen> {
        let parent = self.current.parent()?;
        log::debug!("Navigated back to {}", parent.slug());
        self.current = parent.clone();
        Some(parent)
    }

    /// Whether going back is possible from here.
    ///
    /// Not yet called outside tests. It is the seam a back button uses to decide
    /// whether to render at all.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.current.parent().is_some()
    }

    /// Return directly to Home.
    ///
    /// The Result screen's "Home" action uses it — it differs from
    /// [`NavigationState::back`] there, because Result sits two levels below
    /// Home and back lands on Exercise.
    pub fn go_home(&mut self) -> Screen {
        self.current = Screen::Home;
        log::debug!("Navigated to home");
        self.current.clone()
    }

    /// Start typing `ctx`. Allowed from the same course's Exercise screen,
    /// and from its Result screen (Retry, Next Exercise), and only with an
    /// exercise chosen. Returns `None`, leaving the state unchanged,
    /// otherwise.
    pub fn start_session(&mut self, ctx: SessionContext) -> Option<Screen> {
        let allowed = ctx.exercise_id.is_some()
            && match &self.current {
                Screen::Exercise(draft) => draft.same_course(&ctx),
                Screen::Result(prev) => prev.same_course(&ctx),
                Screen::Home | Screen::Typing(_) => false,
            };
        if !allowed {
            log::warn!("Refusing to start a session from {}", self.current.slug());
            return None;
        }
        log::debug!(
            "Typing exercise {} of {} ({})",
            ctx.exercise_number,
            ctx.product.slug(),
            ctx.mode.slug()
        );
        self.current = Screen::Typing(ctx);
        Some(self.current.clone())
    }

    /// Switch the Typing screen to another exercise of the same course
    /// (dock Prev / Next, Hindi Learn step chips). `None`, leaving the state
    /// unchanged, unless typing that course with an exercise chosen.
    pub fn switch_session(&mut self, ctx: SessionContext) -> Option<Screen> {
        let allowed = ctx.exercise_id.is_some()
            && matches!(&self.current, Screen::Typing(cur) if cur.same_course(&ctx));
        if !allowed {
            log::warn!(
                "Refusing to switch the session from {}",
                self.current.slug()
            );
            return None;
        }
        self.current = Screen::Typing(ctx);
        Some(self.current.clone())
    }

    /// The Typing session completed: show its Result. `None` unless typing.
    pub fn finish_session(&mut self) -> Option<Screen> {
        let Screen::Typing(ctx) = &self.current else {
            return None;
        };
        self.current = Screen::Result(ctx.clone());
        Some(self.current.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radha_core::{TimeLimit, TimerPreset};
    use radha_layouts::LayoutId;

    fn exercise(product: ProductId) -> Screen {
        Screen::Exercise(SessionContext::for_product(product))
    }

    #[test]
    fn typing_can_switch_only_within_its_course() {
        let mut nav = NavigationState::default();
        let product = ProductId::HindiLearn(LayoutId::RemingtonGail);
        nav.open_product(product);
        let ctx = SessionContext {
            exercise_id: Some(3),
            ..SessionContext::for_product(product)
        };
        assert!(nav.switch_session(ctx.clone()).is_none(), "not typing yet");
        nav.start_session(ctx.clone()).expect("start");
        let next = SessionContext {
            exercise_id: Some(4),
            ..ctx.clone()
        };
        assert_eq!(nav.switch_session(next.clone()), Some(Screen::Typing(next)));
        let other = SessionContext {
            exercise_id: Some(1),
            ..SessionContext::for_product(ProductId::EnglishClassic)
        };
        assert!(nav.switch_session(other).is_none(), "another course");
        let none = SessionContext {
            exercise_id: None,
            ..ctx
        };
        assert!(nav.switch_session(none).is_none());
    }

    #[test]
    fn starts_at_home() {
        // `NavigationState::new` honours an env override in debug builds, so assert
        // on the default constructor to keep this independent of the environment.
        let nav = NavigationState::default();
        assert_eq!(nav.current(), &Screen::Home);
        assert!(!nav.can_go_back(), "Home is the root");
    }

    #[test]
    fn start_screen_override_parses_home() {
        assert_eq!(
            NavigationState::parse_start_screen("home"),
            Some(Screen::Home)
        );
        assert_eq!(
            NavigationState::parse_start_screen("  HOME  "),
            Some(Screen::Home),
            "should tolerate casing and surrounding whitespace"
        );
    }

    #[test]
    fn start_screen_override_parses_every_product() {
        for product in ProductId::all() {
            let spec = format!("exercise:{}", product.slug());
            assert_eq!(
                NavigationState::parse_start_screen(&spec),
                Some(exercise(product)),
                "override should accept {spec}"
            );
        }
    }

    #[test]
    fn start_screen_override_rejects_nonsense_without_failing() {
        // A typo must degrade to Home rather than take startup down.
        for bad in [
            "",
            "nonsense",
            "exercise:",
            "exercise:not-a-product",
            "Exercise",
        ] {
            assert_eq!(
                NavigationState::parse_start_screen(bad),
                None,
                "'{bad}' should not parse"
            );
        }
    }

    #[test]
    fn preview_spec_is_recognised() {
        assert!(is_preview_spec("preview"));
        assert!(
            is_preview_spec(" PREVIEW "),
            "should tolerate casing and surrounding whitespace"
        );
    }

    #[test]
    fn overlay_specs_name_only_the_utility_overlays() {
        assert_eq!(overlay_spec("overlay:analytics"), Some("analytics"));
        assert_eq!(overlay_spec(" overlay:Settings "), Some("settings"));
        assert_eq!(overlay_spec("overlay:candidate"), Some("candidate"));
        assert_eq!(overlay_spec("overlay:result"), None);
        assert_eq!(overlay_spec("analytics"), None);
        assert_eq!(
            NavigationState::parse_start_screen("overlay:profile"),
            None,
            "an overlay is never a screen"
        );
    }

    #[test]
    fn preview_is_not_a_screen() {
        // The preview is an overlay; it must never become a navigable screen.
        assert_eq!(NavigationState::parse_start_screen("preview"), None);
        assert!(!is_preview_spec("preview:x"));
    }

    #[test]
    fn home_to_exercise_and_back() {
        // The round trip required by Task 2.2.
        let mut nav = NavigationState::new();

        let screen = nav.open_product(ProductId::EnglishClassic);
        assert_eq!(screen, exercise(ProductId::EnglishClassic));
        assert_eq!(nav.current(), &exercise(ProductId::EnglishClassic));
        assert!(nav.can_go_back());

        let back = nav.back().expect("Exercise should go back to Home");
        assert_eq!(back, Screen::Home);
        assert_eq!(nav.current(), &Screen::Home);
        assert!(!nav.can_go_back());
    }

    #[test]
    fn back_from_home_is_none_rather_than_an_error() {
        // Escape on Home must be a no-op, not a crash or an app exit.
        let mut nav = NavigationState::new();
        assert!(nav.back().is_none());
        assert_eq!(nav.current(), &Screen::Home, "state must be unchanged");
    }

    #[test]
    fn repeated_back_from_home_stays_at_home() {
        let mut nav = NavigationState::new();
        for _ in 0..5 {
            assert!(nav.back().is_none());
        }
        assert_eq!(nav.current(), &Screen::Home);
    }

    #[test]
    fn opening_a_second_product_replaces_the_first() {
        // Cards are opened from Home, so Exercise never stacks on Exercise;
        // back must still land on Home rather than the previous product.
        let mut nav = NavigationState::new();
        nav.open_product(ProductId::EnglishClassic);
        nav.open_product(ProductId::HindiLearn(LayoutId::KrutiDev));

        assert_eq!(
            nav.current(),
            &exercise(ProductId::HindiLearn(LayoutId::KrutiDev))
        );
        assert_eq!(nav.back(), Some(Screen::Home));
    }

    #[test]
    fn go_home_works_from_anywhere() {
        let mut nav = NavigationState::new();
        nav.open_product(ProductId::TypingFlow);
        assert_eq!(nav.go_home(), Screen::Home);
        assert_eq!(nav.current(), &Screen::Home);
    }

    #[test]
    fn every_product_can_be_opened_and_closed() {
        // All nine Home cards must navigate, not just the ones wired first.
        let mut nav = NavigationState::new();
        for product in ProductId::all() {
            assert_eq!(nav.open_product(product), exercise(product));
            assert_eq!(
                nav.current().product(),
                Some(product),
                "Exercise screen must carry the product that was opened"
            );
            assert_eq!(nav.back(), Some(Screen::Home));
        }
    }

    #[test]
    fn home_carries_no_product() {
        assert_eq!(Screen::Home.product(), None);
        assert_eq!(Screen::Home.context(), None);
    }

    #[test]
    fn screen_slugs_are_stable() {
        // The UI switches on these strings.
        assert_eq!(Screen::Home.slug(), "home");
        assert_eq!(exercise(ProductId::TypingFlow).slug(), "exercise");
        assert_eq!(
            Screen::Typing(ctx(ProductId::TypingFlow, 1)).slug(),
            "typing"
        );
        assert_eq!(
            Screen::Result(ctx(ProductId::TypingFlow, 1)).slug(),
            "result"
        );
    }

    fn ctx(product: ProductId, number: u32) -> SessionContext {
        SessionContext {
            exercise_id: Some(i64::from(number) + 100),
            exercise_number: number,
            timer: TimerMode::Stopwatch,
            ..SessionContext::for_product(product)
        }
    }

    #[test]
    fn a_fresh_context_follows_the_mode_matrix() {
        let english = SessionContext::for_product(ProductId::EnglishClassic);
        assert_eq!(english.mode, PracticeMode::Test);
        assert_eq!(
            english.timer,
            TimerMode::Countdown(TimeLimit::from_preset(TimerPreset::Min5))
        );
        assert_eq!(english.exercise_id, None);
        assert!(!english.is_exam());

        let learn = SessionContext::for_product(ProductId::HindiLearn(LayoutId::RemingtonCbi));
        assert_eq!(learn.mode, PracticeMode::Beginner);
        assert_eq!(learn.timer, TimerMode::Off, "Hindi Learn has no timer");

        let flow = SessionContext::for_product(ProductId::NumberPunctuation);
        assert_eq!(flow.mode, PracticeMode::Flow);
        assert!(flow.timer.limit().is_some(), "Flow is always timed");
    }

    #[test]
    fn the_full_flow_home_exercise_typing_result_and_back() {
        let gail = ProductId::HindiMastery(LayoutId::RemingtonGail);
        let mut nav = NavigationState::default();
        assert_eq!(nav.start_session(ctx(gail, 1)), None, "not from Home");
        assert_eq!(nav.finish_session(), None, "nothing to finish");

        nav.open_product(gail);
        assert_eq!(
            nav.start_session(SessionContext::for_product(gail)),
            None,
            "not without a chosen exercise"
        );
        let typing = nav.start_session(ctx(gail, 1)).expect("from Exercise");
        assert_eq!(typing, Screen::Typing(ctx(gail, 1)));
        assert_eq!(typing.product(), Some(gail));
        assert_eq!(typing.context().map(|c| c.exercise_number), Some(1));
        assert_eq!(
            nav.start_session(ctx(gail, 2)),
            None,
            "no session on top of a session"
        );

        let result = nav.finish_session().expect("typing finishes");
        assert_eq!(result, Screen::Result(ctx(gail, 1)));
        assert_eq!(nav.finish_session(), None, "finishing twice is refused");

        // Retry and Next Exercise start again from Result.
        assert_eq!(
            nav.start_session(ctx(gail, 1)),
            Some(Screen::Typing(ctx(gail, 1)))
        );
        nav.finish_session();
        assert_eq!(
            nav.start_session(ctx(gail, 2)),
            Some(Screen::Typing(ctx(gail, 2)))
        );
        nav.finish_session();
        assert_eq!(
            nav.start_session(ctx(ProductId::EnglishClassic, 1)),
            None,
            "another product's exercise is not reachable from this Result"
        );

        // Exercise List (back) keeps the context; Home.
        assert_eq!(nav.back(), Some(Screen::Exercise(ctx(gail, 2))));
        nav.start_session(ctx(gail, 3));
        assert_eq!(
            nav.back(),
            Some(Screen::Exercise(ctx(gail, 3))),
            "back from Typing"
        );
        nav.start_session(ctx(gail, 3));
        nav.finish_session();
        assert_eq!(nav.go_home(), Screen::Home);
    }

    #[test]
    fn the_exercise_draft_is_edited_in_place_and_carried_to_typing() {
        let english = ProductId::EnglishClassic;
        let mut nav = NavigationState::default();
        assert!(nav.draft_mut().is_none(), "no draft on Home");
        nav.open_product(english);
        {
            let draft = nav.draft_mut().expect("draft on Exercise");
            draft.mode = PracticeMode::Learn;
            draft.timer = TimerMode::Off;
            draft.exercise_id = Some(42);
            draft.exercise_number = 7;
            draft.display = DisplayMode::Key;
            draft.no_backspace = true;
        }
        let draft = nav.current().context().cloned().expect("context");
        let typing = nav.start_session(draft.clone()).expect("starts");
        assert_eq!(typing.context(), Some(&draft));
        assert!(nav.draft_mut().is_none(), "no draft while typing");
        // Back from Typing restores the same choices (HOME-11 resume shape).
        assert_eq!(nav.back(), Some(Screen::Exercise(draft)));
    }

    #[test]
    fn exam_practice_is_the_same_flow_with_a_profile() {
        let mut nav = NavigationState::default();
        let screen = nav.open_exam("patna-hc-cot", ProductId::EnglishClassic);
        let ctx0 = screen.context().cloned().expect("exam context");
        assert!(ctx0.is_exam());
        assert_eq!(ctx0.mode, PracticeMode::Exam);
        assert_eq!(
            ctx0.timer.limit().map(|l| l.seconds()),
            Some(crate::exercise::EXAM_PRACTICE_SECONDS),
            "locked practice timer"
        );

        // The language / layout switch changes the carrier, not the course.
        let hindi = ProductId::HindiMastery(LayoutId::KrutiDev);
        if let Some(d) = nav.draft_mut() {
            d.product = hindi;
            d.exercise_id = Some(9);
            d.exercise_number = 9;
        }
        let draft = nav.current().context().cloned().expect("draft");
        assert!(draft.same_course(&ctx0));
        let other_exam = SessionContext {
            exercise_id: Some(9),
            ..SessionContext::for_exam("ssc-chsl", hindi)
        };
        assert!(!draft.same_course(&other_exam));
        let plain = SessionContext {
            exercise_id: Some(9),
            ..SessionContext::for_product(hindi)
        };
        assert!(!draft.same_course(&plain), "exam and product never mix");
        assert_eq!(nav.start_session(other_exam), None);
        assert_eq!(nav.start_session(plain), None);
        assert!(nav.start_session(draft.clone()).is_some());
        nav.finish_session();
        assert_eq!(nav.back(), Some(Screen::Exercise(draft)));
        assert_eq!(nav.back(), Some(Screen::Home));
    }

    #[test]
    fn escape_pauses_a_running_session_and_otherwise_goes_back() {
        let c = ctx(ProductId::EnglishClassic, 1);
        assert_eq!(escape_action(&Screen::Home, None), EscapeAction::Ignore);
        assert_eq!(
            escape_action(&Screen::Exercise(c.clone()), None),
            EscapeAction::Back
        );
        assert_eq!(
            escape_action(&Screen::Typing(c.clone()), Some(SessionState::Active)),
            EscapeAction::PauseTyping
        );
        for state in [
            SessionState::Idle,
            SessionState::Paused,
            SessionState::Complete,
        ] {
            assert_eq!(
                escape_action(&Screen::Typing(c.clone()), Some(state)),
                EscapeAction::Back,
                "{state:?}"
            );
        }
        assert_eq!(escape_action(&Screen::Result(c), None), EscapeAction::Back);
    }
}
