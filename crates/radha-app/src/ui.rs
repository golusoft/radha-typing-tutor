//! Slint UI integration.
//!
//! Bridges the Rust side (settings, navigation, app state) to the Slint window,
//! then runs the event loop.
//!
//! # The wiring pattern
//!
//! Established in Phase 0 and followed by everything added since:
//!
//! - Rust owns state — settings, navigation, lifecycle.
//! - Slint owns presentation.
//! - They meet through explicit properties and callbacks, never shared mutable
//!   state. Per `RULES.MD` Rule 16, business logic stays out of the UI layer.
//!
//! Concretely, the UI raises *requests* ("this card was clicked", "go back",
//! "switch theme") and Rust decides the outcome and writes it back. The UI can
//! never hold a state Rust has not recorded, which is what keeps what the user sees
//! and what gets saved in agreement.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Context;
use slint::{Model, ModelRc, VecModel};

use crate::app_state::AppState;
use crate::catalog;
use crate::navigation::{NavigationState, Screen};
use crate::settings::ThemeMode;

mod flow;
mod utility;

slint::include_modules!();

/// Master copies of the Home catalogues.
///
/// The Slint globals hold the unfiltered literals. When the search text changes,
/// the *displayed* models are rewritten from these masters, so clearing the search
/// restores everything without re-reading the UI.
///
/// The matching rule itself is `catalog::matches_query`; this type only applies it.
struct HomeCatalog {
    english: Vec<ProductEntry>,
    hindi_mastery: Vec<ProductEntry>,
    hindi_learn: Vec<ProductEntry>,
    exams: Vec<ExamEntry>,
}

impl HomeCatalog {
    /// Snapshot the masters from the window's globals. Call once, before any filter
    /// has been applied.
    fn capture(window: &AppWindow) -> Self {
        let products = window.global::<Products>();
        let exams = window.global::<Exams>();

        HomeCatalog {
            english: products.get_english().iter().collect(),
            hindi_mastery: products.get_hindi_mastery().iter().collect(),
            hindi_learn: products.get_hindi_learn().iter().collect(),
            exams: exams.get_all().iter().collect(),
        }
    }

    /// Re-filter every displayed model for `query` and the Exam Hub `exam_category`.
    fn apply(&self, window: &AppWindow, query: &str, exam_category: &str) {
        let products = window.global::<Products>();
        products.set_english(filter_products(&self.english, query));
        products.set_hindi_mastery(filter_products(&self.hindi_mastery, query));
        products.set_hindi_learn(filter_products(&self.hindi_learn, query));

        window
            .global::<Exams>()
            .set_shown(filter_exams(&self.exams, query, exam_category));
    }
}

/// Rewrite each Home product card's count chip with what is installed
/// (`catalog::count_chip`). A count that cannot be read keeps the chip.
fn apply_installed_counts(window: &AppWindow, db: &radha_db::Database) {
    let products = window.global::<Products>();
    let fix = |list: ModelRc<ProductEntry>| -> ModelRc<ProductEntry> {
        let entries: Vec<ProductEntry> = list
            .iter()
            .map(|mut e| {
                let Ok(product) = radha_content::ProductId::from_slug(e.slug.as_str()) else {
                    return e;
                };
                match radha_db::content_import::exercise_count(db, product) {
                    Ok(n) => e.meta_1 = catalog::count_chip(n, e.meta_1.as_str()).into(),
                    Err(err) => log::warn!("Could not count {}: {err}", product.slug()),
                }
                e
            })
            .collect();
        ModelRc::new(VecModel::from(entries))
    };
    products.set_english(fix(products.get_english()));
    products.set_hindi_mastery(fix(products.get_hindi_mastery()));
    products.set_hindi_learn(fix(products.get_hindi_learn()));
}

/// Each Exam Hub card's rule fields from its `radha-exams` profile (PRD
/// EM-01..EM-04): speeds, the rule line and the certification level are
/// derived from the profile, never trusted from the Slint literal. A card
/// whose profile did not load shows nothing but practice.
fn apply_exam_profiles(exams: &mut [ExamEntry]) {
    for e in exams.iter_mut() {
        match crate::exam_mode::profile(e.id.as_str()) {
            Some(p) => {
                let card = radha_exams::card_summary(p);
                e.eng_speed = card.eng_speed.into();
                e.hin_speed = card.hin_speed.into();
                e.rule = card.rule.into();
                e.level = card.level.into();
                e.hindi_only = e.hindi_only || p.hindi_only;
            }
            None => {
                log::warn!("Exam card '{}' has no loaded profile", e.id);
                e.eng_speed = "—".into();
                e.hin_speed = "—".into();
                e.rule = "Rules pending verification".into();
                e.level = "practice".into();
            }
        }
    }
}

/// A Hindi-only exam card's passage chip from the installed Hindi pool
/// (`catalog::pool_chip`). A count that cannot be read keeps the chip.
fn apply_exam_pool_counts(exams: &mut [ExamEntry], db: &radha_db::Database) {
    let count =
        match radha_db::content_import::exam_passage_count(db, radha_content::ExamPool::Hindi) {
            Ok(n) => n,
            Err(err) => {
                log::warn!("Could not count the Hindi exam pool: {err}");
                return;
            }
        };
    for e in exams.iter_mut().filter(|e| e.hindi_only) {
        e.hin_speed = catalog::pool_chip(count).into();
    }
}

/// The product cards that match `query`.
fn filter_products(master: &[ProductEntry], query: &str) -> ModelRc<ProductEntry> {
    let kept: Vec<ProductEntry> = master
        .iter()
        .filter(|entry| {
            catalog::matches_query(
                query,
                &[
                    entry.slug.as_str(),
                    entry.title.as_str(),
                    entry.subtitle.as_str(),
                    entry.description.as_str(),
                    entry.badge.as_str(),
                    entry.identity_label.as_str(),
                    entry.meta_1.as_str(),
                    entry.meta_2.as_str(),
                ],
            )
        })
        .cloned()
        .collect();

    ModelRc::new(VecModel::from(kept))
}

/// The exam cards that match `query` within `category`.
fn filter_exams(master: &[ExamEntry], query: &str, category: &str) -> ModelRc<ExamEntry> {
    let kept: Vec<ExamEntry> = master
        .iter()
        .filter(|entry| catalog::category_matches(category, entry.category.as_str()))
        .filter(|entry| {
            catalog::matches_query(
                query,
                &[
                    entry.id.as_str(),
                    entry.crest.as_str(),
                    entry.dept.as_str(),
                    entry.tag.as_str(),
                    entry.title.as_str(),
                    entry.org.as_str(),
                    entry.description.as_str(),
                    entry.font_layout.as_str(),
                ],
            )
        })
        .cloned()
        .collect();

    ModelRc::new(VecModel::from(kept))
}

/// Font file extensions Slint can embed.
const FONT_EXTENSIONS: [&str; 3] = ["ttf", "otf", "ttc"];

/// Report which font binaries are present in `assets/fonts/`.
///
/// # Why this only reports
///
/// Slint registers fonts at **compile** time, via `import "…/font.ttf";` in a
/// `.slint` file — see `ui/theme/fonts.slint`. There is no runtime registration
/// call to make (Slint 1.18 removed the older `register_font_from_path` in favour
/// of compile-time embedding), so this function cannot load anything; it exists so
/// the log records whether the build is running on bundled fonts or system
/// fallbacks. That distinction matters for Hindi: Kruti Dev has no safe fallback,
/// and "text looks like nonsense Latin characters" is a confusing symptom to
/// diagnose without this line in the log.
///
/// Returns the number of font files found.
fn log_font_status() -> usize {
    let font_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("assets")
        .join("fonts");

    let Ok(entries) = std::fs::read_dir(&font_dir) else {
        log::info!(
            "No fonts directory at {} — using system font fallbacks",
            font_dir.display()
        );
        return 0;
    };

    let found: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();

            let is_font = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| FONT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));

            if !is_font {
                return None;
            }

            Some(path.file_name()?.to_string_lossy().into_owned())
        })
        .collect();

    if found.is_empty() {
        log::info!(
            "No font files in {} — using system font fallbacks (see assets/README.md)",
            font_dir.display()
        );
    } else {
        log::info!(
            "Font files present ({}): {}. These are embedded only if imported in \
             ui/theme/fonts.slint",
            found.len(),
            found.join(", ")
        );
    }

    found.len()
}

/// Exam typography (English Arial, Hindi Unicode Mangal) from the installed
/// Windows fonts, with the embedded fallbacks (`exam_fonts`). Set once.
fn apply_exam_fonts(window: &AppWindow, fonts: crate::exam_fonts::ExamFonts) {
    log::info!(
        "Exam fonts: English passages in {}{}, Hindi Unicode passages in {}{}",
        fonts.latin(),
        if fonts.arial {
            ""
        } else {
            " (Arial not installed)"
        },
        fonts.devanagari(),
        if fonts.mangal {
            ""
        } else {
            " (Mangal not installed: Windows optional feature Hindi Supplemental Fonts)"
        },
    );
    let v = window.global::<TypingView>();
    v.set_font_latin(fonts.latin().into());
    v.set_font_hindi(fonts.devanagari().into());
    v.set_mangal_missing(!fonts.mangal);
}

/// The UI's screen switch value: Typing Flow has dedicated Exercise, Typing
/// and Result screens (DESIGN 5.1, 8); every other product shares the
/// family screens. Navigation itself is unchanged (`Screen::slug`).
fn screen_slug(screen: &Screen) -> &'static str {
    let flow = screen
        .context()
        .is_some_and(|c| c.product == radha_content::ProductId::TypingFlow && !c.is_exam());
    match (flow, screen) {
        (true, Screen::Exercise(_)) => "flow-exercise",
        (true, Screen::Typing(_)) => "flow-typing",
        (true, Screen::Result(_)) => "flow-result",
        _ => screen.slug(),
    }
}

/// Push the active screen into the window.
///
/// Single place that translates a [`Screen`] into the properties the UI switches
/// on, so the mapping cannot drift between the initial render and later
/// navigation.
fn apply_screen(window: &AppWindow, screen: &Screen) {
    window.set_active_screen(screen_slug(screen).into());

    match screen.product() {
        Some(product) => {
            window.set_active_product_name(product.display_name().into());
            window.set_active_product_slug(product.slug().into());
        }
        None => {
            window.set_active_product_name(slint::SharedString::new());
            window.set_active_product_slug(slint::SharedString::new());
        }
    }
}

/// Show a window reporting an unrecoverable startup failure, and wait for the user
/// to dismiss it.
///
/// Used when stage 2 of startup fails. Without this the release build — which has
/// no console — would exit silently and look like it had simply not launched.
///
/// # Errors
///
/// Returns an error only if the window itself cannot be created, at which point
/// stderr is the only remaining channel.
pub fn run_startup_error(error: &anyhow::Error, log_file: &std::path::Path) -> anyhow::Result<()> {
    let state = AppState::from_startup_failure(error, &log_file.display().to_string());
    let Some(details) = state.error() else {
        // `from_startup_failure` always produces Error; this is unreachable in
        // practice and returning is preferable to unwrapping.
        return Ok(());
    };

    let window = AppWindow::new().context("could not create the error window")?;

    window.set_app_state(state.slug().into());
    window.set_error_title(details.title.as_str().into());
    window.set_error_detail(details.detail.as_str().into());
    window.set_error_log_hint(details.log_hint.as_str().into());

    {
        let handle = window.as_weak();
        window.on_quit_requested(move || {
            if let Some(window) = handle.upgrade() {
                let _ = window.window().hide();
            }
        });
    }

    log::info!("Showing startup error window");
    window.run().context("the error window event loop failed")?;
    Ok(())
}

/// Build the window, wire it to application state, and run the event loop.
///
/// Returns once the user closes the window.
///
/// # Errors
///
/// Returns an error if the window cannot be created or the event loop fails.
pub fn run(application: &mut crate::app::App) -> anyhow::Result<()> {
    log_font_status();

    let window = AppWindow::new().context("could not create the application window")?;

    // What the compiled UI actually embedded (ui/theme/fonts.slint), as opposed
    // to what happens to be on disk (log_font_status above).
    log::info!("{}", window.global::<Fonts>().get_status());
    apply_exam_fonts(&window, crate::exam_fonts::ExamFonts::detect());

    // The window exists but nothing has been pushed into it yet, so show the
    // loading state rather than a momentarily blank Home. In practice this lasts
    // microseconds; it earns its place because later phases move real work here
    // (exercise content loading) and the UI already has a coherent state for it.
    window.set_app_state(AppState::Loading.slug().into());

    // --- Push persisted settings into the UI --------------------------------
    let settings = application.settings.clone();

    // Write the theme straight into the Slint global that components read.
    // Going through a window property and a `changed` handler does not work here:
    // that handler does not fire for a value set before the event loop starts, so
    // the saved theme would be ignored and the window would always open light.
    window.global::<Theme>().set_dark(settings.theme.is_dark());

    // Animation master switch: "Reduce motion" in Settings turns every
    // transition off (DESIGN.md Section 14, 20.9).
    utility::apply_window_settings(&window, &settings);

    window.set_initial_width(settings.window.width as f32);
    window.set_initial_height(settings.window.height as f32);

    log::info!(
        "Window initialised at {}x{}, theme {:?}",
        settings.window.width,
        settings.window.height,
        settings.theme
    );

    // --- Navigation and the practice flow (Rust-owned) ----------------------
    // `Flow` owns navigation and the Exercise -> Typing -> Result state; it
    // writes the initial screen (honouring RADHA_START_SCREEN in debug).
    let flow = flow::Flow::install(
        &window,
        NavigationState::new(),
        Rc::clone(&application.repository),
        Rc::clone(&application.database),
        application.profile_id,
        application.paths.data_dir(),
    );

    // Debug-only component preview overlay (P01). A plain property set before
    // `run()`, not a changed-handler, so the value is honoured at startup.
    let preview = crate::navigation::component_preview_requested();
    if preview {
        log::warn!("RADHA_START_SCREEN=preview — showing the component preview overlay");
    }
    window.set_show_component_preview(preview);

    // --- Settings, Analytics, Profile, Candidate info (P08) -------------------
    // One shared copy of the settings: the overlay and the theme toggle both
    // change it and save settings.toml at once; it is merged back on exit.
    let shared_settings = Rc::new(RefCell::new(settings.clone()));
    flow.borrow_mut().apply_settings(&settings);
    let utility = utility::Utility::install(
        &window,
        Rc::clone(&application.database),
        application.profile_id,
        Rc::clone(&shared_settings),
        application.paths.settings_file(),
        application.paths.data_dir().to_path_buf(),
        Rc::clone(&flow),
    );
    // Debug-only: RADHA_START_SCREEN=overlay:<id> opens an overlay over Home.
    if let Some(id) = crate::navigation::start_overlay_requested() {
        log::warn!("RADHA_START_SCREEN=overlay:{id} — opening the {id} overlay");
        if let Ok(mut u) = utility.try_borrow_mut() {
            u.open(id);
        }
    }

    {
        let flow = Rc::clone(&flow);

        window.on_navigate_requested(move |slug| {
            // The UI hands over an opaque slug; Rust decides what it means. An
            // unrecognised value is logged and ignored rather than panicking, so a
            // stale card can never take the app down (RULES.MD Rule 23).
            let product = match radha_content::ProductId::from_slug(slug.as_str()) {
                Ok(product) => product,
                Err(err) => {
                    log::warn!("Ignoring navigation to unknown product '{slug}': {err}");
                    return;
                }
            };
            flow.borrow_mut().open_product(product);
        });
    }

    {
        let flow = Rc::clone(&flow);
        // Back at the root is a no-op (Escape on Home never closes the app).
        window.on_back_requested(move || flow.borrow_mut().back());
    }

    // --- Home search and Exam Hub filter ------------------------------------
    // The UI reports the search text and the exam category; Rust applies the
    // matching rule and rewrites the displayed models.
    // Count chips from the installed content, before the masters are
    // captured (a planned "100 Drills" with nothing installed is not shown).
    apply_installed_counts(&window, &application.database.borrow());
    let home_catalog = {
        let mut catalog = HomeCatalog::capture(&window);
        apply_exam_profiles(&mut catalog.exams);
        apply_exam_pool_counts(&mut catalog.exams, &application.database.borrow());
        // Show the rewritten masters (the Slint default shows the literals).
        window
            .global::<Exams>()
            .set_shown(filter_exams(&catalog.exams, "", "all"));
        Rc::new(catalog)
    };

    {
        let home_catalog = Rc::clone(&home_catalog);
        let handle = window.as_weak();

        window.on_filters_changed(move |query, category| {
            if let Some(window) = handle.upgrade() {
                home_catalog.apply(&window, query.as_str(), category.as_str());
            }
        });
    }

    // An exam launcher opens that exam's Exercise screen: the shared passage
    // pool under the card's `radha-exams` profile (fixed timer, locked
    // rules, the profile's scoring policy and certification).
    {
        let flow = Rc::clone(&flow);
        window.on_exam_launch_requested(move |id, language| {
            log::info!("Exam launch requested: {id} / {language}");
            flow.borrow_mut().open_exam(id.as_str(), language.as_str());
        });
    }

    // Contact and social links: the UI sends a fixed id, Rust owns the URL.
    window.on_open_link_requested(
        |id| match crate::contact::ContactLink::from_id(id.as_str()) {
            Some(link) => {
                if let Err(e) = crate::contact::open(link) {
                    log::warn!("Could not open the {} link: {e}", link.id());
                }
            }
            None => log::warn!("Ignoring unknown link id '{id}'"),
        },
    );

    // --- Own the theme state on the Rust side -------------------------------
    // The UI raises a toggle *request*; this closure computes the next mode and
    // writes it back. Two reasons for the indirection:
    //
    //  1. The UI can never hold a mode that Rust has not recorded, so what gets
    //     saved to settings.toml always matches what the user saw.
    //  2. `application` cannot be borrowed for the lifetime of the event loop, so
    //     the choice is parked here and merged back once the loop exits.
    {
        let shared = Rc::clone(&shared_settings);
        let settings_path = application.paths.settings_file();
        // Weak, to avoid a reference cycle between the window and its own handler.
        let window_handle = window.as_weak();

        window.on_theme_toggle_requested(move || {
            let next: ThemeMode = shared.borrow().theme.toggled();
            shared.borrow_mut().theme = next;
            // Save at once so a crash or kill does not lose the choice.
            if let Err(e) = shared.borrow().save(&settings_path) {
                log::warn!("Could not save settings: {e}");
            }

            if let Some(window) = window_handle.upgrade() {
                window.global::<Theme>().set_dark(next.is_dark());
                window
                    .global::<UtilityView>()
                    .set_st_theme(if next.is_dark() { "dark" } else { "light" }.into());
            }

            log::debug!("Theme switched to {next:?}");
        });
    }

    {
        let handle = window.as_weak();
        window.on_quit_requested(move || {
            if let Some(window) = handle.upgrade() {
                let _ = window.window().hide();
            }
        });
    }

    // --- Ready -------------------------------------------------------------
    // Everything is wired; hand the window over to the user.
    let ready = AppState::Ready;
    debug_assert!(ready.is_ready());
    window.set_app_state(ready.slug().into());

    // --- Run ---------------------------------------------------------------
    log::info!("Entering UI event loop");
    window.run().context("the UI event loop failed")?;
    log::info!("UI event loop exited");

    // --- Merge UI changes back into persisted state ------------------------
    // Everything the overlay and the theme toggle changed; the window
    // geometry and first-launch flag are the shell's own.
    {
        let changed = shared_settings.borrow();
        let window_geometry = application.settings.window;
        let first_launch = application.settings.first_launch;
        application.settings = changed.clone();
        application.settings.window = window_geometry;
        application.settings.first_launch = first_launch;
    }

    // Persist the final window geometry so the app reopens as the user left it.
    let size = window.window().size();
    let scale = window.window().scale_factor().max(0.1);
    application.settings.window.width = (size.width as f32 / scale).round() as u32;
    application.settings.window.height = (size.height as f32 / scale).round() as u32;

    // Guard against a degenerate value (e.g. a minimised window reporting 0)
    // being written back and producing an unusable window next launch.
    application.settings.window = application.settings.window.clamped();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_status_check_does_not_fail_without_bundled_fonts() {
        // Since P01 the fonts ship in assets/fonts/ and are embedded at compile
        // time; this check only reports what is on disk. It must never fail —
        // a missing or unreadable directory is logged and the app still starts.
        let _ = log_font_status();
    }

    #[test]
    fn theme_mode_maps_to_the_ui_boolean() {
        // The UI exposes a bool; this is the mapping the wiring above relies on.
        assert!(ThemeMode::Dark.is_dark());
        assert!(!ThemeMode::Light.is_dark());
    }

    #[test]
    fn settings_are_cloneable_for_ui_handoff() {
        // `run` clones settings to avoid holding a borrow across the event loop.
        let settings = crate::settings::Settings::default();
        let copy = settings.clone();
        assert_eq!(settings, copy);
    }

    #[test]
    fn every_product_slug_resolves_for_navigation() {
        // `on_navigate_requested` parses whatever slug the UI sends. Every slug the
        // Home screen can emit must therefore round-trip, or a card would silently
        // do nothing.
        for product in radha_content::ProductId::all() {
            let parsed = radha_content::ProductId::from_slug(product.slug())
                .expect("Home card slug must resolve");
            assert_eq!(parsed, product);
        }
    }

    #[test]
    fn typing_flow_gets_its_dedicated_screens_and_nothing_else_does() {
        use crate::navigation::SessionContext;
        use radha_content::ProductId;
        let flow = SessionContext::for_product(ProductId::TypingFlow);
        assert_eq!(
            screen_slug(&Screen::Exercise(flow.clone())),
            "flow-exercise"
        );
        assert_eq!(screen_slug(&Screen::Typing(flow.clone())), "flow-typing");
        assert_eq!(screen_slug(&Screen::Result(flow)), "flow-result");
        assert_eq!(screen_slug(&Screen::Home), "home");
        let english = SessionContext::for_product(ProductId::EnglishClassic);
        assert_eq!(screen_slug(&Screen::Typing(english)), "typing");
        let number = SessionContext::for_product(ProductId::NumberPunctuation);
        assert_eq!(screen_slug(&Screen::Exercise(number)), "exercise");
    }
}
