//! Radha Typing Tutor — Windows desktop entry point.
//!
//! This file is deliberately thin. It is the application's **error boundary**:
//! every fallible startup step lives in [`app`], and `main` exists only to
//! translate a failure into a clean message plus a non-zero exit code.
//!
//! # Why a boundary at all
//!
//! `RULES.MD` Rule 23 requires the app not to crash on a missing exercise, a bad
//! layout, a database problem, or an interrupted session. A user should never see
//! a Rust panic backtrace. Two mechanisms enforce that:
//!
//! 1. Every fallible path returns `Result`, bubbling up to [`main`], which logs
//!    and reports.
//! 2. A panic hook catches any bug that slips through a `Result`-free path, so
//!    even an unexpected panic is logged rather than lost.

// Release builds detach from the console so launching from Explorer does not
// flash a terminal. Debug builds keep it, since that is where log output is read.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_state;
mod audio;
mod catalog;
mod contact;
mod custom;
mod exam_fonts;
mod exam_mode;
mod exercise;
mod export;
mod hindi_learn;
mod logging;
mod navigation;
mod passage;
mod paths;
mod practice;
mod recovery;
mod report;
mod rhythm;
mod settings;
mod typing_rules;
mod ui;

use std::process::ExitCode;

/// How the run finished.
enum Outcome {
    /// Ran and exited normally.
    Normal,
    /// Failed to start, and the user was told in a window.
    ///
    /// Distinguished from an `Err` return so the failure is not *also* printed to
    /// stderr: the user has already been shown the problem, and duplicating it adds
    /// nothing. The exit code still reflects the failure.
    StartupFailedAndReported,
}

fn main() -> ExitCode {
    install_panic_hook();

    match run() {
        Ok(Outcome::Normal) => ExitCode::SUCCESS,
        Ok(Outcome::StartupFailedAndReported) => ExitCode::FAILURE,
        Err(error) => {
            report_fatal(&error);
            ExitCode::FAILURE
        }
    }
}

/// Start the app, show the window, and run until the user closes it.
fn run() -> anyhow::Result<Outcome> {
    // Stage 1. A failure here can only reach stderr — there is no log yet, and no
    // confidence that a window could be created either.
    let environment = app::init_environment()?;
    let log_file = environment.log_file.clone();

    // Stage 2. A failure here is reported in a real window, because by now logging
    // works and a window can be shown.
    let mut application = match app::App::bootstrap_services(environment) {
        Ok(application) => application,
        Err(error) => {
            log::error!("Startup failed: {error:#}");
            ui::run_startup_error(&error, &log_file)?;
            return Ok(Outcome::StartupFailedAndReported);
        }
    };

    ui::run(&mut application)?;

    application.shutdown();
    Ok(Outcome::Normal)
}

/// Report an unrecoverable error to the log and to the user.
///
/// Writes to both the log (for diagnosis) and stderr (so a developer running
/// from a terminal sees it immediately). Deliberately formats the `anyhow` cause
/// chain, since the root cause is usually the useful part.
fn report_fatal(error: &anyhow::Error) {
    // The logger may not exist yet if the failure happened during its own
    // initialisation, so stderr is the reliable channel.
    eprintln!("\n{} could not start.\n", radha_core::PRODUCT_NAME);
    eprintln!("  Reason: {error}");

    for (depth, cause) in error.chain().skip(1).enumerate() {
        eprintln!(
            "  {:>indent$}Caused by: {cause}",
            "",
            indent = depth * 2 + 2
        );
    }
    eprintln!();

    // Best-effort: only lands if logging was initialised before the failure.
    log::error!("Fatal startup error: {error:#}");
}

/// Route panics into the log instead of letting them vanish.
///
/// A panic is always a bug, but an unlogged panic is a bug that cannot be
/// diagnosed from a user's machine. This keeps the default panic message on
/// stderr while also recording it.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map_or_else(|| "unknown location".to_string(), ToString::to_string);

        // Panic payloads are almost always &str or String.
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic payload".to_string());

        log::error!("PANIC at {location}: {message}");

        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    #[test]
    fn panic_hook_installs_without_panicking() {
        // Installing the hook must itself be safe; a failure here would take down
        // startup before anything is logged.
        super::install_panic_hook();
    }
}
