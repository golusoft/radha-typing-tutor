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
//! rule exists to prevent.
//!
//! # Phase 2 scope
//!
//! Only `Home` and `Exercise` exist so far. `Typing` and `Result` are added in
//! Phase 5+; the `back` behaviour below is already shaped to accommodate them.

use radha_content::ProductId;

/// A screen in the core training flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The product dashboard.
    Home,
    /// Exercise selection for a chosen product.
    Exercise(ProductId),
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
    pub fn parent(self) -> Option<Screen> {
        match self {
            Screen::Home => None,
            Screen::Exercise(_) => Some(Screen::Home),
        }
    }

    /// The product this screen is showing, if any.
    #[must_use]
    pub fn product(self) -> Option<ProductId> {
        match self {
            Screen::Home => None,
            Screen::Exercise(product) => Some(product),
        }
    }

    /// Stable identifier for the UI's screen switch.
    ///
    /// Slint cannot hold a Rust enum, so the active screen crosses the boundary as
    /// this string.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Screen::Home => "home",
            Screen::Exercise(_) => "exercise",
        }
    }
}

/// The currently displayed screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl NavigationState {
    /// Start at Home, unless [`START_SCREEN_ENV`] overrides it in a debug build.
    #[must_use]
    pub fn new() -> Self {
        if cfg!(debug_assertions) {
            if let Some(raw) = std::env::var_os(START_SCREEN_ENV) {
                let spec = raw.to_string_lossy();
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
        Some(Screen::Exercise(product))
    }

    /// The active screen.
    #[must_use]
    pub fn current(self) -> Screen {
        self.current
    }

    /// Open a product's Exercise screen.
    ///
    /// Returns the new screen.
    pub fn open_product(&mut self, product: ProductId) -> Screen {
        self.current = Screen::Exercise(product);
        log::debug!("Navigated to {} ({})", product.display_name(), product.slug());
        self.current
    }

    /// Go back one level.
    ///
    /// Returns the new screen, or `None` if already at the root — which lets the
    /// caller distinguish "went back" from "nothing to go back to" and, for
    /// example, ignore an Escape key press on Home rather than closing the app.
    pub fn back(&mut self) -> Option<Screen> {
        let parent = self.current.parent()?;
        self.current = parent;
        log::debug!("Navigated back to {}", parent.slug());
        Some(parent)
    }

    /// Whether going back is possible from here.
    ///
    /// Not yet called outside tests. It is the seam a back button uses to decide
    /// whether to render at all, which the Exercise and Result screens need in
    /// Phase 5; the stub screen's single button does not.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn can_go_back(self) -> bool {
        self.current.parent().is_some()
    }

    /// Return directly to Home.
    ///
    /// Not yet called outside tests. The Result screen's "Home" action uses it in
    /// Phase 5 — it differs from [`NavigationState::back`] there, because Result
    /// sits two levels below Home and back would land on Typing.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn go_home(&mut self) -> Screen {
        self.current = Screen::Home;
        log::debug!("Navigated to home");
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radha_layouts::LayoutId;

    #[test]
    fn starts_at_home() {
        // `NavigationState::new` honours an env override in debug builds, so assert
        // on the default constructor to keep this independent of the environment.
        let nav = NavigationState::default();
        assert_eq!(nav.current(), Screen::Home);
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
                Some(Screen::Exercise(product)),
                "override should accept {spec}"
            );
        }
    }

    #[test]
    fn start_screen_override_rejects_nonsense_without_failing() {
        // A typo must degrade to Home rather than take startup down.
        for bad in ["", "nonsense", "exercise:", "exercise:not-a-product", "Exercise"] {
            assert_eq!(
                NavigationState::parse_start_screen(bad),
                None,
                "'{bad}' should not parse"
            );
        }
    }

    #[test]
    fn home_to_exercise_and_back() {
        // The round trip required by Task 2.2.
        let mut nav = NavigationState::new();

        let screen = nav.open_product(ProductId::EnglishClassic);
        assert_eq!(screen, Screen::Exercise(ProductId::EnglishClassic));
        assert_eq!(nav.current(), Screen::Exercise(ProductId::EnglishClassic));
        assert!(nav.can_go_back());

        let back = nav.back().expect("Exercise should go back to Home");
        assert_eq!(back, Screen::Home);
        assert_eq!(nav.current(), Screen::Home);
        assert!(!nav.can_go_back());
    }

    #[test]
    fn back_from_home_is_none_rather_than_an_error() {
        // Escape on Home must be a no-op, not a crash or an app exit.
        let mut nav = NavigationState::new();
        assert!(nav.back().is_none());
        assert_eq!(nav.current(), Screen::Home, "state must be unchanged");
    }

    #[test]
    fn repeated_back_from_home_stays_at_home() {
        let mut nav = NavigationState::new();
        for _ in 0..5 {
            assert!(nav.back().is_none());
        }
        assert_eq!(nav.current(), Screen::Home);
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
            Screen::Exercise(ProductId::HindiLearn(LayoutId::KrutiDev))
        );
        assert_eq!(nav.back(), Some(Screen::Home));
    }

    #[test]
    fn go_home_works_from_anywhere() {
        let mut nav = NavigationState::new();
        nav.open_product(ProductId::TypingFlow);
        assert_eq!(nav.go_home(), Screen::Home);
        assert_eq!(nav.current(), Screen::Home);
    }

    #[test]
    fn every_product_can_be_opened_and_closed() {
        // All nine Home cards must navigate, not just the ones wired first.
        let mut nav = NavigationState::new();
        for product in ProductId::all() {
            assert_eq!(nav.open_product(product), Screen::Exercise(product));
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
    }

    #[test]
    fn screen_slugs_are_stable() {
        // The UI switches on these strings.
        assert_eq!(Screen::Home.slug(), "home");
        assert_eq!(Screen::Exercise(ProductId::TypingFlow).slug(), "exercise");
    }
}
