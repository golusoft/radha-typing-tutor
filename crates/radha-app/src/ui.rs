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

use crate::app_state::AppState;
use crate::navigation::{NavigationState, Screen};
use crate::settings::ThemeMode;

slint::include_modules!();

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

/// Push the active screen into the window.
///
/// Single place that translates a [`Screen`] into the properties the UI switches
/// on, so the mapping cannot drift between the initial render and later
/// navigation.
fn apply_screen(window: &AppWindow, screen: Screen) {
    window.set_active_screen(screen.slug().into());

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

    // Animation master switch. Wired now so the Settings screen has somewhere to
    // write; DESIGN.md Section 14 requires transitions to be switchable off
    // wholesale for slower machines.
    window.global::<Motion>().set_enabled(true);

    window.set_initial_width(settings.window.width as f32);
    window.set_initial_height(settings.window.height as f32);

    log::info!(
        "Window initialised at {}x{}, theme {:?}",
        settings.window.width,
        settings.window.height,
        settings.theme
    );

    // --- Navigation (Rust-owned) -------------------------------------------
    let navigation = Rc::new(RefCell::new(NavigationState::new()));
    apply_screen(&window, navigation.borrow().current());

    {
        let navigation = Rc::clone(&navigation);
        let handle = window.as_weak();

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

            let screen = navigation.borrow_mut().open_product(product);
            if let Some(window) = handle.upgrade() {
                apply_screen(&window, screen);
            }
        });
    }

    {
        let navigation = Rc::clone(&navigation);
        let handle = window.as_weak();

        window.on_back_requested(move || {
            // Returns None at the root, which is how Escape on Home becomes a
            // deliberate no-op instead of closing the app.
            let Some(screen) = navigation.borrow_mut().back() else {
                log::debug!("Back requested at the root — ignoring");
                return;
            };

            if let Some(window) = handle.upgrade() {
                apply_screen(&window, screen);
            }
        });
    }

    // --- Own the theme state on the Rust side -------------------------------
    // The UI raises a toggle *request*; this closure computes the next mode and
    // writes it back. Two reasons for the indirection:
    //
    //  1. The UI can never hold a mode that Rust has not recorded, so what gets
    //     saved to settings.toml always matches what the user saw.
    //  2. `application` cannot be borrowed for the lifetime of the event loop, so
    //     the choice is parked here and merged back once the loop exits.
    let current_theme = Rc::new(RefCell::new(settings.theme));
    let pending_theme: Rc<RefCell<Option<ThemeMode>>> = Rc::new(RefCell::new(None));

    {
        let current_theme = Rc::clone(&current_theme);
        let pending_theme = Rc::clone(&pending_theme);
        // Weak, to avoid a reference cycle between the window and its own handler.
        let window_handle = window.as_weak();

        window.on_theme_toggle_requested(move || {
            let next = current_theme.borrow().toggled();
            *current_theme.borrow_mut() = next;
            *pending_theme.borrow_mut() = Some(next);

            if let Some(window) = window_handle.upgrade() {
                window.global::<Theme>().set_dark(next.is_dark());
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
    if let Some(mode) = *pending_theme.borrow() {
        application.settings.theme = mode;
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
        // Phase 0 ships no font binaries, so this must simply report zero. The
        // app has to start regardless.
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
}
