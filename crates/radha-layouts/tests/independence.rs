//! RULES 9 source scan: no layout module reaches another layout's table,
//! rules, type or registry entry.
//!
//! Comments and string/char literals are stripped and `#[cfg(test)]` items
//! skipped (same approach as `radha-app/tests/asset_policy.rs`), then the code
//! is split into identifier tokens and matched exactly (never by substring,
//! so `cbi_rules` is not the token `cbi`).

use std::path::{Path, PathBuf};

const LAYOUTS: [(&str, &str, &str); 5] = [
    ("qwerty", "QwertyLayout", "Qwerty"),
    ("gail", "GailLayout", "RemingtonGail"),
    ("cbi", "CbiLayout", "RemingtonCbi"),
    ("krutidev", "KrutiDevLayout", "KrutiDev"),
    ("inscript", "InscriptLayout", "Inscript"),
];

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Blank comments, string literals and char literals (keeping newlines).
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
                while k < chars.len() {
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

/// Remove every `#[cfg(test)]` item from stripped code.
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
        out.extend(item[..end].chars().filter(|&c| c == '\n'));
        rest = &item[end..];
    }
    out.push_str(rest);
    out
}

/// Identifier tokens, with `a::b` paths joined.
fn tokens(code: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    for c in code.chars() {
        if is_ident(c) {
            cur.push(c);
        } else if !cur.is_empty() {
            toks.push(std::mem::take(&mut cur));
        }
        if c == ':' || c == '!' || c == '#' || c == '[' {
            toks.push(c.to_string());
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    toks
}

/// Which module a source file belongs to: `Some(layout)` for `src/<layout>/`,
/// `None` for a shared module.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Owner {
    Layout(usize),
    Shared,
    Root,
}

/// Every rule a file breaks.
fn violations(owner: Owner, file_name: &str, src: &str) -> Vec<String> {
    let code = drop_cfg_test(&strip_rust(src));
    let t = tokens(&code);
    let mut bad = Vec::new();
    if owner == Owner::Root {
        return bad;
    }
    let own = match owner {
        Owner::Layout(i) => Some(i),
        _ => None,
    };
    for (i, (module, ty, variant)) in LAYOUTS.iter().enumerate() {
        if Some(i) == own {
            continue;
        }
        if t.iter().any(|x| x == module) {
            bad.push(format!("names layout module `{module}`"));
        }
        if t.iter().any(|x| x == ty) {
            bad.push(format!("names layout type `{ty}`"));
        }
        let variant_path = t
            .windows(4)
            .any(|w| w[0] == "LayoutId" && w[1] == ":" && w[2] == ":" && w[3] == *variant);
        if variant_path {
            bad.push(format!("names `LayoutId::{variant}`"));
        }
    }
    for forbidden in ["load_layout", "registry"] {
        if t.iter().any(|x| x == forbidden) {
            bad.push(format!("uses `{forbidden}`"));
        }
    }
    if own.is_some()
        && t.windows(2)
            .any(|w| w[0] == "dyn" && w[1] == "KeyboardLayout")
    {
        bad.push("uses `dyn KeyboardLayout`".to_owned());
    }
    for (a, b) in [
        ("include", "!"),
        ("include_str", "!"),
        ("include_bytes", "!"),
    ] {
        if t.windows(2).any(|w| w[0] == a && w[1] == b) {
            bad.push(format!("uses `{a}!`"));
        }
    }
    if t.windows(4)
        .any(|w| w[0] == "super" && w[1] == ":" && w[2] == ":" && w[3] == "super")
    {
        bad.push("uses `super::super`".to_owned());
    }
    if t.windows(2).any(|w| w[0] == "extern" && w[1] == "crate") {
        bad.push("uses `extern crate`".to_owned());
    }
    if code.contains("#[path") {
        bad.push("uses `#[path]`".to_owned());
    }
    let declares_table = code.contains("[KeyEntry;");
    if declares_table && !(own.is_some() && file_name == "table.rs") {
        bad.push("declares a KeyEntry array outside src/<layout>/table.rs".to_owned());
    }
    bad
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read src").flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn the_scanner_flags_cross_layout_routes_and_allows_honest_code() {
    let gail = Owner::Layout(1);
    let cbi = Owner::Layout(2);
    let kd = Owner::Layout(3);
    let flagged = [
        (gail, "use crate::cbi::table::TABLE;"),
        (
            cbi,
            "fn f() { crate::load_layout(LayoutId::RemingtonGail); }",
        ),
        (cbi, "fn f() { crate::GailLayout::new(); }"),
        (kd, "static L: &'static dyn KeyboardLayout = x;"),
        (Owner::Shared, "fn f() -> LayoutId { LayoutId::KrutiDev }"),
        (gail, "const X: &str = include_str!(\"../cbi/table.rs\");"),
        (Owner::Shared, "const T: [KeyEntry; 1] = [];"),
        (Owner::Layout(4), "use crate::gail::table::TABLE;"),
        (gail, "fn f() { crate::inscript::can_type(\"\"); }"),
    ];
    for (owner, sample) in flagged {
        assert!(
            !violations(owner, "mod.rs", sample).is_empty(),
            "missed: {sample}"
        );
    }
    let allowed = [
        (cbi, "fn id() -> LayoutId { LayoutId::RemingtonCbi }"),
        (cbi, "// unlike gail\nconst S: &str = \"gail\";"),
        (cbi, "fn cbi_rules() {}"),
        (Owner::Shared, "fn plan(l: &'static dyn KeyboardLayout) {}"),
        (
            gail,
            "#[cfg(test)]\nmod tests { use crate::cbi::CbiLayout; }",
        ),
    ];
    for (owner, sample) in allowed {
        assert!(
            violations(owner, "mod.rs", sample).is_empty(),
            "false positive: {sample} -> {:?}",
            violations(owner, "mod.rs", sample)
        );
    }
    assert!(violations(
        gail,
        "table.rs",
        "pub(super) const TABLE: [KeyEntry; 1] = [];"
    )
    .is_empty());
}

#[test]
fn no_layout_module_reaches_another_layout() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&src, &mut files);
    assert!(files.len() > 15, "sources not found");
    let mut bad = Vec::new();
    for p in &files {
        let rel = p.strip_prefix(&src).unwrap_or(p);
        let parts: Vec<String> = rel
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        let owner = match parts.as_slice() {
            [dir, _] => LAYOUTS
                .iter()
                .position(|(m, _, _)| m == dir)
                .map_or(Owner::Shared, Owner::Layout),
            [file] if file == "lib.rs" || file == "registry.rs" => Owner::Root,
            _ => Owner::Shared,
        };
        let file_name = parts.last().cloned().unwrap_or_default();
        let text = std::fs::read_to_string(p).expect("read");
        for v in violations(owner, &file_name, &text) {
            bad.push(format!("{}: {v}", rel.display()));
        }
    }
    // Each layout states its own finger mapping (RULES 9 "Finger mapping").
    for (module, _, _) in LAYOUTS {
        let text = std::fs::read_to_string(src.join(module).join("mod.rs")).expect("layout mod.rs");
        if !strip_rust(&text).contains("fn finger_for") {
            bad.push(format!("{module}/mod.rs: no own `fn finger_for`"));
        }
        if !src.join(module).join("table.rs").exists() {
            bad.push(format!("{module}: no own table.rs"));
        }
    }
    assert!(bad.is_empty(), "RULES 9 violations:\n{}", bad.join("\n"));
}
