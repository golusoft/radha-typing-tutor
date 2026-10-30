//! P09 Part B checks on the typing screens' UI source: the exam fonts come
//! from the installed Windows fonts (never bundled), Focus mode has a way
//! out that does not depend on the Rules panel or Esc, and the passage
//! scrolls through a gliding offset.

use std::fs;
use std::path::{Path, PathBuf};

fn app() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn ui(name: &str) -> String {
    fs::read_to_string(app().join("ui").join(name)).unwrap()
}

/// The text between two markers of `src`.
fn section<'a>(src: &'a str, from: &str, to: &str) -> &'a str {
    let start = src.find(from).unwrap_or_else(|| panic!("no {from:?}"));
    let rest = &src[start..];
    let end = rest
        .find(to)
        .unwrap_or_else(|| panic!("no {to:?} after {from:?}"));
    &rest[..end]
}

#[test]
fn focus_mode_can_be_left_from_the_header_and_with_f11() {
    let typing = ui("screens/typing.slint");
    // The Rules panel (where Focus mode is switched on) is hidden in focus
    // mode, so the exit must live outside it: in the header.
    assert!(typing
        .contains("property <bool> rules-shown: TypingView.rules-open && !TypingView.focus-mode;"));
    let header = section(&typing, "// Header (TY-01)", "// Stats strip");
    let exit = section(header, "if TypingView.focus-mode:", "text: \"Rules\"");
    assert!(exit.contains("text: \"Exit Focus (F11)\";"), "{exit}");
    assert!(exit.contains("TypingView.toggle-focus-mode();"), "{exit}");
    // F11 reaches Rust (every key on the Typing screen does) and is
    // classified there; Esc is not used, it is the GAIL/CBI dead key.
    let main = ui("main.slint");
    assert!(main.contains(
        "TypingView.key-typed(event.text, event.modifiers.control, event.modifiers.alt);"
    ));
    let practice = fs::read_to_string(app().join("src/practice.rs")).unwrap();
    assert!(practice.contains("pub const F11: char = '\\u{f70e}';"));
    assert!(practice.contains("F11 => return KeyInput::ToggleFocus,"));
    let flow = fs::read_to_string(app().join("src/ui/flow.rs")).unwrap();
    assert!(flow.contains("KeyInput::ToggleFocus if !self.is_flow() => self.toggle_focus_mode(),"));
    assert!(flow.contains("ty.on_toggle_focus_mode(move || f.borrow_mut().toggle_focus_mode());"));
}

#[test]
fn exam_fonts_are_named_not_bundled_and_used_in_both_boxes() {
    // Arial and Mangal are Microsoft fonts: never in assets/fonts, never
    // imported into the binary.
    let fonts_dir = app().join("../../assets/fonts");
    for entry in fs::read_dir(&fonts_dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        assert!(
            !name.contains("arial") && !name.contains("mangal"),
            "{name} is bundled"
        );
    }
    let fonts = ui("theme/fonts.slint");
    for line in fonts
        .lines()
        .filter(|l| l.trim_start().starts_with("import"))
    {
        let l = line.to_lowercase();
        assert!(
            !l.contains("arial") && !l.contains("mangal") && !l.contains("windows"),
            "{line}"
        );
    }
    // The defaults are the embedded fallbacks; Rust names Arial / Mangal
    // when Windows has them.
    let practice = ui("data/practice.slint");
    assert!(practice.contains("in-out property <string> font-latin: \"JetBrains Mono\";"));
    assert!(practice.contains("in-out property <string> font-hindi: \"Noto Sans Devanagari\";"));
    let typing = ui("screens/typing.slint");
    // Passage box and YOUR TYPING box share the profile font; Kruti Dev
    // keeps its legacy key-stream view.
    assert!(typing.contains(
        "property <string> passage-font: TypingView.legacy ? Typography.family-mono\n        : TypingView.devanagari ? TypingView.font-hindi\n        : TypingView.font-latin;"
    ));
    let input = section(
        &typing,
        "// --- Your typing (input pane)",
        "// Exam Rules & Settings panel",
    );
    assert_eq!(
        input
            .matches("font-family: TypingView.legacy ? Typography.family-mono : root.passage-font;")
            .count(),
        2
    );
    assert!(
        input.contains("font-family: TypingView.font-hindi;"),
        "preedit"
    );
    assert!(typing.contains("if TypingView.devanagari && TypingView.mangal-missing: RuleChip"));
    let flow = ui("screens/flow/flow_typing.slint");
    assert!(flow.contains("font-family: TypingView.font-latin;"));
}

#[test]
fn passage_lines_glide_with_motion_and_snap_without_it() {
    for (file, pos) in [
        (
            "screens/typing.slint",
            "y: (TypingView.lines-base - root.scroll-pos) * root.line-h;",
        ),
        (
            "screens/flow/flow_typing.slint",
            "y: (TypingView.lines-base - root.scroll-pos) * root.line-h;",
        ),
    ] {
        let src = ui(file);
        assert!(src.contains(pos), "{file}");
        assert!(
            src.contains("property <float> scroll-pos: TypingView.scroll-first;"),
            "{file}"
        );
        // Motion.fast is 0 ms under Settings → Reduce motion.
        assert!(
            src.contains("duration: TypingView.scroll-glide ? Motion.fast : 0ms;"),
            "{file}"
        );
    }
    let typing = ui("screens/typing.slint");
    assert!(typing.contains("duration: TypingView.typed-glide ? Motion.fast : 0ms;"));
}
