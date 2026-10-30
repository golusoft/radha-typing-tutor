//! Build script — compiles the Slint UI into the binary.
//!
//! Slint's `.slint` files are compiled ahead of time into Rust code, which is
//! then pulled in by `slint::include_modules!()`. That means a malformed UI file
//! is a *compile* error rather than a runtime surprise on a user's machine.

fn main() {
    // Entry point of the UI tree. Everything else is reached via `import`.
    let main_ui = "ui/main.slint";

    slint_build::compile(main_ui).unwrap_or_else(|error| {
        panic!("failed to compile {main_ui}: {error}");
    });

    // Reserve an 8 MB main-thread stack (Windows default: 1 MB). The
    // generated `AppWindow::new()` builds every screen and overlay in one
    // call; unoptimised (debug) frames for it outgrew 1 MB in P08. Only
    // address space is reserved; pages are committed on use.
    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:8388608"),
        Ok("gnu") if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") => {
            println!("cargo:rustc-link-arg-bins=-Wl,--stack,8388608");
        }
        _ => {}
    }

    // Re-run if any asset the UI references changes. Slint already tracks
    // `.slint` imports; these cover the non-Slint assets.
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=../../assets");
}
