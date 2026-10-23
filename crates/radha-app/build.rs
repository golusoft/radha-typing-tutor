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

    // Re-run if any asset the UI references changes. Slint already tracks
    // `.slint` imports; these cover the non-Slint assets.
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=../../assets");
}
