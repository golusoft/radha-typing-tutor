//! Repository policy checks that run under `cargo test --workspace`.
//!
//! These enforce, mechanically, rules that are otherwise easy to break by
//! accident in a single edit:
//!
//! * no black and no blue colours in authored `.slint` / `.svg` files
//!   (DESIGN.md Section 2, RULES.MD Rule 27);
//! * no `http(s)://` URL in any `.slint` file — everything is offline, no CDN
//!   (RULES.MD Rule 28);
//! * no exam names in the engine crates (RULES.MD Rule 31);
//! * no `unwrap(` / `expect(` / `panic!` / `unreachable!` / `todo!` /
//!   `unimplemented!` in engine production code (`src/`, outside
//!   `#[cfg(test)]` items; `tests/` is exempt);
//! * the bundled offline assets (fonts + licences, Lucide icons, sounds) are
//!   present and within budget (assets/README.md).
//!
//! Std only, no extra dependencies. Detection is proven by the classifier
//! self-tests on in-memory samples; violations are never planted in repo files.

use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Recursively collect files under `dir` whose extension is in `exts`.
/// Skips `target` directories so build output is never scanned.
fn walk(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            walk(&path, exts, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
        {
            out.push(path);
        }
    }
}

fn files(dir: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, exts, &mut out);
    out.sort();
    out
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn rel(path: &Path) -> String {
    let root = repo_root();
    path.strip_prefix(&root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

// ---------------------------------------------------------------------------
// Scanners: keep code, drop prose
// ---------------------------------------------------------------------------

/// Blank out `// …` and `/* … */` comments and `"…"` string literals of a
/// `.slint` source, keeping newlines so line numbers survive. A single pass
/// state machine, so `//` inside a string is not mistaken for a comment.
fn strip_slint(src: &str) -> String {
    enum State {
        Code,
        Line,
        Block,
        Str,
    }
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut state = State::Code;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let blank = if c == '\n' { '\n' } else { ' ' };
        match state {
            State::Code => {
                if c == '/' && next == Some('/') {
                    state = State::Line;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                if c == '/' && next == Some('*') {
                    state = State::Block;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                if c == '"' {
                    state = State::Str;
                    out.push(' ');
                } else {
                    out.push(c);
                }
            }
            State::Line => {
                if c == '\n' {
                    state = State::Code;
                }
                out.push(blank);
            }
            State::Block => {
                if c == '*' && next == Some('/') {
                    state = State::Code;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                out.push(blank);
            }
            State::Str => {
                if c == '\\' && next.is_some() {
                    out.push(' ');
                    out.push(if next == Some('\n') { '\n' } else { ' ' });
                    i += 2;
                    continue;
                }
                if c == '"' {
                    state = State::Code;
                }
                out.push(blank);
            }
        }
        i += 1;
    }
    out
}

/// Blank out `<!-- … -->` comments of an SVG, keeping newlines. Attribute
/// strings stay: that is where SVG colours live.
fn strip_svg(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let end = after.find("-->").map_or(after.len(), |e| e + 3);
        out.extend(
            after[..end]
                .chars()
                .map(|c| if c == '\n' { '\n' } else { ' ' }),
        );
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Colour extraction + classification
// ---------------------------------------------------------------------------

/// Named colours treated as black/blue regardless of context.
const BANNED_NAMES: [&str; 21] = [
    "black",
    "blue",
    "navy",
    "darkblue",
    "mediumblue",
    "royalblue",
    "steelblue",
    "dodgerblue",
    "deepskyblue",
    "cornflowerblue",
    "midnightblue",
    "skyblue",
    "lightblue",
    "lightskyblue",
    "powderblue",
    "cadetblue",
    "slateblue",
    "darkslateblue",
    "mediumslateblue",
    "lightsteelblue",
    "aliceblue",
];

/// A colour found in code: the literal text and its RGB (None for names).
struct Found {
    literal: String,
    rgb: Option<(u8, u8, u8)>,
}

fn hex_rgb(digits: &str) -> Option<(u8, u8, u8)> {
    let d: Vec<u8> = digits
        .chars()
        .map(|c| c.to_digit(16).map(|v| v as u8))
        .collect::<Option<Vec<u8>>>()?;
    match d.len() {
        // #RGB / #RGBA — alpha ignored.
        3 | 4 => Some((d[0] * 17, d[1] * 17, d[2] * 17)),
        // #RRGGBB / #RRGGBBAA — alpha ignored.
        6 | 8 => Some((d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5])),
        _ => None,
    }
}

/// Parse the numeric arguments of `name(…)` starting at `open` (the `(`).
fn call_args(line: &str, open: usize) -> Option<(Vec<String>, usize)> {
    let close = line[open..].find(')')? + open;
    let args = line[open + 1..close]
        .split([',', ' ', '/'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    Some((args, close))
}

fn channel(arg: &str) -> Option<u8> {
    if let Some(p) = arg.strip_suffix('%') {
        let v: f64 = p.parse().ok()?;
        return Some((v.clamp(0.0, 100.0) * 2.55).round() as u8);
    }
    let v: f64 = arg.parse().ok()?;
    Some(v.clamp(0.0, 255.0).round() as u8)
}

fn fraction(arg: &str) -> Option<f64> {
    let v: f64 = arg.trim_end_matches('%').parse().ok()?;
    Some(
        if arg.ends_with('%') || v > 1.0 {
            v / 100.0
        } else {
            v
        }
        .clamp(0.0, 1.0),
    )
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to(r), to(g), to(b))
}

/// Every colour literal in one line of (already stripped) code.
fn colours_in_line(line: &str) -> Vec<Found> {
    let mut found = Vec::new();
    let chars: Vec<(usize, char)> = line.char_indices().collect();

    for (k, &(i, c)) in chars.iter().enumerate() {
        let prev = if k == 0 { None } else { Some(chars[k - 1].1) };

        // #hex
        if c == '#' {
            let digits: String = line[i + 1..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect();
            let after = line[i + 1 + digits.len()..].chars().next();
            if !after.is_some_and(is_ident) {
                if let Some(rgb) = hex_rgb(&digits) {
                    found.push(Found {
                        literal: format!("#{digits}"),
                        rgb: Some(rgb),
                    });
                }
            }
            continue;
        }

        if !c.is_ascii_alphabetic() || prev.is_some_and(is_ident) {
            continue;
        }
        let word: String = line[i..].chars().take_while(|&c| is_ident(c)).collect();
        let lower = word.to_ascii_lowercase();
        let open = i + word.len();
        let has_paren = line[open..].starts_with('(');

        if has_paren && (lower == "rgb" || lower == "rgba") {
            if let Some((args, close)) = call_args(line, open) {
                if let (Some(r), Some(g), Some(b)) = (
                    args.first().and_then(|a| channel(a)),
                    args.get(1).and_then(|a| channel(a)),
                    args.get(2).and_then(|a| channel(a)),
                ) {
                    found.push(Found {
                        literal: line[i..=close].to_string(),
                        rgb: Some((r, g, b)),
                    });
                }
            }
        } else if has_paren && (lower == "hsl" || lower == "hsla") {
            if let Some((args, close)) = call_args(line, open) {
                let h = args
                    .first()
                    .and_then(|a| a.trim_end_matches("deg").parse::<f64>().ok());
                if let (Some(h), Some(s), Some(l)) = (
                    h,
                    args.get(1).and_then(|a| fraction(a)),
                    args.get(2).and_then(|a| fraction(a)),
                ) {
                    found.push(Found {
                        literal: line[i..=close].to_string(),
                        rgb: Some(hsl_to_rgb(h, s, l)),
                    });
                }
            }
        } else if BANNED_NAMES.contains(&lower.as_str()) {
            found.push(Found {
                literal: word,
                rgb: None,
            });
        }
    }
    found
}

/// Why a colour is banned, or `None` if it is allowed.
fn violation(r: u8, g: u8, b: u8) -> Option<&'static str> {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let (maxf, minf) = (f64::from(max) / 255.0, f64::from(min) / 255.0);
    let l = (maxf + minf) / 2.0;
    let d = maxf - minf;
    let s = if d == 0.0 {
        0.0
    } else {
        d / (1.0 - (2.0 * l - 1.0).abs())
    };

    if max <= 0x18 || (l < 0.12 && s < 0.10) {
        return Some("near-black");
    }

    if d > 0.0 {
        let (rf, gf, bf) = (
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
        );
        let hue = if max == r {
            60.0 * ((gf - bf) / d).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((bf - rf) / d + 2.0)
        } else {
            60.0 * ((rf - gf) / d + 4.0)
        };
        if (190.0..=260.0).contains(&hue) && s >= 0.15 && max - min >= 0x14 {
            return Some("blue hue");
        }
    }
    None
}

/// `line literal reason` for every banned colour in `code` (already stripped).
fn findings(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (n, line) in code.lines().enumerate() {
        for f in colours_in_line(line) {
            let reason = match f.rgb {
                Some((r, g, b)) => violation(r, g, b),
                None => Some("banned colour name"),
            };
            if let Some(reason) = reason {
                out.push(format!("{}: {} ({reason})", n + 1, f.literal));
            }
        }
    }
    out
}

fn flagged(sample: &str) -> bool {
    !findings(sample).is_empty()
}

// ---------------------------------------------------------------------------
// a–b: classifier self-tests
// ---------------------------------------------------------------------------

#[test]
fn classifier_flags_black_and_blue() {
    for sample in [
        "#000", "#000000", "#111111", "#0000FF", "#1E90FF", "#3586F4", "#4F46E5", "#6366F1",
    ] {
        assert!(
            flagged(&format!("c: {sample};")),
            "{sample} must be flagged"
        );
    }
    assert!(flagged("background: blue;"));
    assert!(flagged("background: Colors.black;"));
    assert!(flagged("fill=\"dodgerblue\""));
    assert!(flagged("c: rgb(0, 0, 255);"));
    assert!(flagged("c: hsl(220, 80%, 50%);"));
}

#[test]
fn classifier_accepts_palette_and_prose() {
    let palette = repo_root().join("crates/radha-app/ui/theme/palette.slint");
    let code = strip_slint(&read(&palette));
    let mut count = 0;
    for line in code.lines() {
        for f in colours_in_line(line) {
            let (r, g, b) = f.rgb.expect("palette uses hex literals only");
            assert_eq!(violation(r, g, b), None, "palette colour {}", f.literal);
            count += 1;
        }
    }
    assert!(
        count >= 60,
        "expected the full palette, parsed {count} colours"
    );

    for sample in ["#1A1218", "#5DAE8B", "#241A21"] {
        assert!(!flagged(&format!("c: {sample};")), "{sample} must pass");
    }
    // Identifiers that merely contain a banned word are not colours.
    assert!(!flagged(
        "text-primary: Theme.blueprint-ish; x: blackout-mode;"
    ));

    // Prose in comments and UI strings never reaches the colour check.
    let prose = "// no black, no blue (#000000)\ntext: \"no black, no blue\";\n/* #0000FF */";
    assert!(findings(&strip_slint(prose)).is_empty());
    assert!(findings(&strip_svg("<!-- fill=\"#000\" black -->")).is_empty());
}

// ---------------------------------------------------------------------------
// c–d: colour policy on the real tree
// ---------------------------------------------------------------------------

#[test]
fn slint_files_have_no_black_or_blue_colours() {
    let ui = repo_root().join("crates/radha-app/ui");
    let all = files(&ui, &["slint"]);
    assert!(
        !all.is_empty(),
        "no .slint files found under {}",
        ui.display()
    );
    let bad: Vec<String> = all
        .iter()
        .flat_map(|p| {
            findings(&strip_slint(&read(p)))
                .into_iter()
                .map(move |f| format!("{}:{f}", rel(p)))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "black/blue colours in .slint:\n{}",
        bad.join("\n")
    );
}

#[test]
fn svg_icons_have_no_black_or_blue_colours() {
    let icons = repo_root().join("assets/icons");
    let all = files(&icons, &["svg"]);
    assert!(
        !all.is_empty(),
        "no .svg files found under {}",
        icons.display()
    );
    let bad: Vec<String> = all
        .iter()
        .flat_map(|p| {
            findings(&strip_svg(&read(p)))
                .into_iter()
                .map(move |f| format!("{}:{f}", rel(p)))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "black/blue colours in .svg:\n{}",
        bad.join("\n")
    );
}

// ---------------------------------------------------------------------------
// e: offline — no URLs in .slint
// ---------------------------------------------------------------------------

#[test]
fn slint_files_have_no_http_urls() {
    let all = files(&repo_root().join("crates"), &["slint"]);
    assert!(!all.is_empty());
    let mut bad = Vec::new();
    for p in &all {
        // Raw text, comments included: a URL in a comment is one edit away
        // from becoming a live CDN reference.
        for (n, line) in read(p).lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            if lower.contains("http://") || lower.contains("https://") {
                bad.push(format!("{}:{}: {}", rel(p), n + 1, line.trim()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "URLs in .slint (RULES 28):\n{}",
        bad.join("\n")
    );
}

// ---------------------------------------------------------------------------
// f: engine crates stay exam-agnostic
// ---------------------------------------------------------------------------

/// Exam acronyms, matched case-insensitively as whole words, so the slug form
/// used by exam ids (`ssc-chsl`, `rrb-ntpc`, `uppcl`) is caught too. Sources:
/// RULES.MD Rule 31, EXAM-MODE-AND-GOLU-REUSE-PLAN.md checklist R6, and the
/// exam ids in `crates/radha-app/ui/data/exams.slint`.
const EXAM_ACRONYMS: [&str; 18] = [
    "SSC", "RRB", "UPSSSC", "DSSSB", "BSSC", "CPCT", "AIIMS", "CRPF", "CAPF", "EPFO", "RSSB",
    "BELTRON", "BSEDC", "UPPRPB", "UPPCL", "CHSL", "CGL", "NTPC",
];

/// Exam / body phrases, matched case-insensitively after `-` / `_` become
/// spaces, so `delhi-police-hcm` and `bihar_civil_court` are caught
/// (R6: Patna, Civil Court …).
const EXAM_PHRASES: [&str; 6] = [
    "high court",
    "civil court",
    "patna",
    "staff selection",
    "railway recruitment",
    "delhi police",
];

fn exam_names_in(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for word in line.split(|c: char| !c.is_ascii_alphanumeric()) {
            if EXAM_ACRONYMS.iter().any(|a| a.eq_ignore_ascii_case(word)) {
                hits.push(format!("{}: {word}", n + 1));
            }
        }
        let normalised = line.to_lowercase().replace(['-', '_'], " ");
        for phrase in EXAM_PHRASES {
            if normalised.contains(phrase) {
                hits.push(format!("{}: \"{phrase}\"", n + 1));
            }
        }
    }
    hits
}

#[test]
fn engine_crates_have_no_exam_names() {
    // Sanity: the matcher itself works (in-memory, never planted in files).
    assert!(!exam_names_in("let p = SSC;").is_empty());
    assert!(!exam_names_in("// Patna High Court").is_empty());
    // Slug / lower-case forms, as exam ids appear in data and fixtures.
    for slug in [
        "id: \"ssc-chsl\"",
        "const P: &str = \"rrb-ntpc\";",
        "INSERT INTO t VALUES ('uppcl');",
        "let dsssb = 1;",
        "\"delhi-police-hcm\"",
        "\"bihar-civil-court\"",
        "let high_court_mode = true;",
    ] {
        assert!(!exam_names_in(slug).is_empty(), "missed slug: {slug}");
    }
    // Acronyms only match whole words: these must stay clean.
    assert!(exam_names_in("let misscount = MISSCOUNT; // classic").is_empty());
    assert!(exam_names_in("fn rrbx_helper() {} // cgls").is_empty());

    // radha-exams and radha-app are intentionally not scanned: exam names are
    // allowed there.
    let root = repo_root();
    let mut scanned = Vec::new();
    for krate in [
        "radha-core",
        "radha-layouts",
        "radha-content",
        "radha-db",
        "radha-analytics",
    ] {
        scanned.extend(files(&root.join("crates").join(krate), &["rs", "toml"]));
    }
    scanned.extend(files(&root.join("migrations"), &["sql"]));
    assert!(scanned.len() > 10, "engine sources not found");

    let bad: Vec<String> = scanned
        .iter()
        .flat_map(|p| {
            exam_names_in(&read(p))
                .into_iter()
                .map(move |h| format!("{}:{h}", rel(p)))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "exam names in engine crates (RULES 31):\n{}",
        bad.join("\n")
    );
}

// ---------------------------------------------------------------------------
// f2: engine production code never panics
// ---------------------------------------------------------------------------

/// Blank out comments, string literals and char literals of Rust source,
/// keeping newlines. Raw strings (`r"…"`, `r#"…"#`) are handled; a `'` that
/// is not a char literal is a lifetime and is kept.
fn strip_rust(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
            continue;
        }
        let prev_ident = i > 0 && is_ident(chars[i - 1]);
        if c == 'r' && !prev_ident && matches!(next, Some('"') | Some('#')) {
            let mut j = i + 1;
            let mut hashes = 0;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                let mut k = j + 1;
                loop {
                    if k >= chars.len() {
                        break;
                    }
                    if chars[k] == '"' && (1..=hashes).all(|h| chars.get(k + h) == Some(&'#')) {
                        k += 1 + hashes;
                        break;
                    }
                    k += 1;
                }
                for &ch in &chars[i..k.min(chars.len())] {
                    out.push(blank(ch));
                }
                i = k;
                continue;
            }
        }
        if c == '"' {
            out.push(' ');
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    out.push(' ');
                    out.push(blank(chars[i + 1]));
                    i += 2;
                    continue;
                }
                let end = chars[i] == '"';
                out.push(blank(chars[i]));
                i += 1;
                if end {
                    break;
                }
            }
            continue;
        }
        if c == '\'' {
            // Char literal: 'x' or '\…'. Anything else is a lifetime.
            let literal_end = if next == Some('\\') {
                (i + 3..chars.len()).find(|&k| chars[k] == '\'')
            } else if chars.get(i + 2) == Some(&'\'') {
                Some(i + 2)
            } else {
                None
            };
            if let Some(end) = literal_end {
                for &ch in &chars[i..=end] {
                    out.push(blank(ch));
                }
                i = end + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Remove every item annotated `#[cfg(test)]` from stripped code: from the
/// attribute to the matching `}` of the item's body (or to the `;` of a
/// body-less item such as a `use`).
fn drop_cfg_test(code: &str) -> String {
    const ATTR: &str = "#[cfg(test)]";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(start) = rest.find(ATTR) {
        out.push_str(&rest[..start]);
        let item = &rest[start..];
        let open = item.find('{');
        let semi = item.find(';');
        let end = match (open, semi) {
            (Some(o), Some(s)) if s < o => s + 1,
            (Some(o), _) => {
                let mut depth = 0usize;
                let mut end = item.len();
                for (k, ch) in item[o..].char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = o + k + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                end
            }
            (None, Some(s)) => s + 1,
            (None, None) => item.len(),
        };
        // Keep line numbers stable.
        out.extend(item[..end].chars().filter(|&c| c == '\n'));
        rest = &item[end..];
    }
    out.push_str(rest);
    out
}

/// Panicking calls in production code, as `line: call`.
fn panicking_calls(src: &str) -> Vec<String> {
    const CALLS: [&str; 6] = [
        "unwrap(",
        "expect(",
        "panic!",
        "unreachable!",
        "todo!",
        "unimplemented!",
    ];
    let code = drop_cfg_test(&strip_rust(src));
    let mut hits = Vec::new();
    for (n, line) in code.lines().enumerate() {
        for call in CALLS {
            let mut from = 0;
            while let Some(pos) = line[from..].find(call) {
                let at = from + pos;
                let before = line[..at].chars().next_back();
                if !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                    hits.push(format!("{}: {call}", n + 1));
                }
                from = at + call.len();
            }
        }
    }
    hits
}

#[test]
fn panic_scanner_flags_calls_and_ignores_tests_comments_and_strings() {
    for bad in [
        "fn f() { x.unwrap(); }",
        "fn f() { x.expect(\"m\"); }",
        "fn f() { panic!(\"p\"); }",
        "fn f() { unreachable!() }",
        "fn f() { todo!() }",
        "fn f() { unimplemented!() }",
        "fn f() -> &'static str { y.unwrap() }",
    ] {
        assert!(!panicking_calls(bad).is_empty(), "missed: {bad}");
    }
    for ok in [
        "fn f() { x.unwrap_or(0); x.unwrap_or_default(); x.unwrap_or_else(|| 1); }",
        "fn f() { x.expect_err_free(); my_unwrap(); }",
        "// x.unwrap() in a comment\n/* panic!() */ /// doc .expect(\"x\")",
        "let s = \"unwrap() panic!\"; let r = r#\"x.expect(\"y\")\"#; let c = '\"';",
        "fn f() {}\n#[cfg(test)]\nmod t { fn g() { x.unwrap(); panic!(); } }\nfn h() {}",
        "#[cfg(test)]\nuse std::panic;\nfn h() {}",
    ] {
        assert!(
            panicking_calls(ok).is_empty(),
            "false positive: {ok} -> {:?}",
            panicking_calls(ok)
        );
    }
    // Code after a test module is still scanned.
    assert!(
        !panicking_calls("#[cfg(test)]\nmod t { fn g() {} }\nfn h() { x.unwrap(); }").is_empty()
    );
}

#[test]
fn engine_crates_have_no_panicking_calls() {
    let root = repo_root();
    let mut scanned = 0;
    let mut bad = Vec::new();
    for krate in [
        "radha-core",
        "radha-layouts",
        "radha-content",
        "radha-db",
        "radha-analytics",
        "radha-exams",
    ] {
        // Production sources only: `src/`, never `tests/`.
        for p in files(&root.join("crates").join(krate).join("src"), &["rs"]) {
            scanned += 1;
            for hit in panicking_calls(&read(&p)) {
                bad.push(format!("{}:{hit}", rel(&p)));
            }
        }
    }
    assert!(scanned > 10, "engine sources not found ({scanned})");
    assert!(
        bad.is_empty(),
        "panicking calls in engine production code:\n{}",
        bad.join("\n")
    );
}

// ---------------------------------------------------------------------------
// g–i: bundled offline assets
// ---------------------------------------------------------------------------

const LUCIDE_ICONS: [&str; 28] = [
    "globe",
    "message-circle",
    "phone",
    "lightbulb",
    "share-2",
    "shield-check",
    "search",
    "bar-chart-2",
    "user",
    "settings",
    "play",
    "arrow-right",
    "check-circle",
    "zap",
    "hash",
    "keyboard",
    "award",
    "lock",
    "rotate-ccw",
    "pause",
    "skip-back",
    "skip-forward",
    "printer",
    "maximize-2",
    "sliders-horizontal",
    "x",
    "copy",
    "volume-2",
];

#[test]
fn lucide_icons_present_and_themeable() {
    let icons = repo_root().join("assets/icons");
    for name in LUCIDE_ICONS {
        let path = icons.join(format!("{name}.svg"));
        assert!(path.is_file(), "missing icon {}", rel(&path));
        let svg = read(&path);
        assert!(
            svg.contains("viewBox=\"0 0 24 24\""),
            "{name}.svg is not 24x24"
        );
        assert!(svg.contains("currentColor"), "{name}.svg is not themeable");
    }
    let licence = read(&icons.join("LICENSE-lucide.txt"));
    assert!(
        licence.contains("ISC License"),
        "LICENSE-lucide.txt is not the ISC text"
    );
}

#[test]
fn bundled_fonts_present_with_licences() {
    let fonts = repo_root().join("assets/fonts");
    for file in [
        "Inter-Regular.ttf",
        "Inter-Medium.ttf",
        "Inter-SemiBold.ttf",
        "Inter-Bold.ttf",
        "JetBrainsMono-Regular.ttf",
        "JetBrainsMono-Bold.ttf",
        "NotoSansDevanagari-Regular.ttf",
        "NotoSansDevanagari-Bold.ttf",
        "LICENSE-Inter.txt",
        "LICENSE-JetBrainsMono.txt",
        "LICENSE-NotoSansDevanagari.txt",
    ] {
        let path = fonts.join(file);
        let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        assert!(len > 0, "missing or empty {}", rel(&path));
    }
    // Kruti Dev: licence check pending. Mangal: Windows system font.
    let entries = fs::read_dir(&fonts).expect("assets/fonts readable");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        assert!(
            !name.contains("kruti") && !name.contains("mangal"),
            "{name} must not be bundled (see assets/README.md)"
        );
    }
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

#[test]
fn sound_assets_are_small_valid_wavs() {
    let sounds = repo_root().join("assets/sounds");
    let mut total = 0usize;
    for name in ["key", "error", "success", "complete", "timer"] {
        let path = sounds.join(format!("{name}.wav"));
        let b = fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", rel(&path)));
        assert!(b.len() > 44, "{name}.wav too short");
        assert_eq!(&b[0..4], b"RIFF", "{name}.wav");
        assert_eq!(&b[8..12], b"WAVE", "{name}.wav");
        assert_eq!(&b[12..16], b"fmt ", "{name}.wav");
        assert_eq!(u16_at(&b, 20), 1, "{name}.wav must be PCM");
        assert_eq!(u16_at(&b, 22), 1, "{name}.wav must be mono");
        assert_eq!(u32_at(&b, 24), 22_050, "{name}.wav sample rate");
        assert_eq!(u16_at(&b, 34), 16, "{name}.wav bit depth");
        assert_eq!(&b[36..40], b"data", "{name}.wav");
        assert_eq!(
            u32_at(&b, 40) as usize + 44,
            b.len(),
            "{name}.wav data size"
        );
        total += b.len();
    }
    assert!(
        total < 300 * 1024,
        "sounds total {total} B exceeds 300 KB budget"
    );
}
