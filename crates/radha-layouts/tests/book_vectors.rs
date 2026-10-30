//! Every test row of the four production keyboard books, replayed through
//! the layout's own composer (GAIL-TC-*, CBI-TC-*, KD010-TC-*, INSC-TC-*).
//!
//! `tests/fixtures/book_<slug>.json` is generated from the books' Chapter 23
//! test suites (one row per TC id, with the PDF's sha256). Input tokens: a US
//! character is that key (Shift layer when the character is shifted),
//! `G:<c>` the same key with AltGr, `ESC` the Esc dead-key prefix, `BS`
//! Backspace, `ENTER` Enter, `ALT:<digits>` an Alt + numpad code as typed,
//! `CTRL+<d>` Ctrl+Shift+digit. Rows the books mark CONFLICT/UNVERIFIED
//! (`not_mapped`) assert that nothing is mapped.

mod common;

use radha_core::akshar::normalize;
use radha_layouts::krutidev::to_unicode;
use radha_layouts::{Composer, Encoding, KeyboardLayout, Keystroke, Layer, PhysicalKey};
use serde_json::Value;

use common::{is_sha256, layout, tests_dir};

fn fixture(slug: &str) -> Value {
    let path = tests_dir()
        .join("fixtures")
        .join(format!("book_{slug}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

/// Type the tokens; returns (committed text, preedit, every token mapped).
fn replay(l: &'static dyn KeyboardLayout, tokens: &[Value]) -> (String, String, bool) {
    let mut c = Composer::new(l);
    let mut text = String::new();
    let mut all_mapped = true;
    for t in tokens {
        let t = t.as_str().unwrap_or_default();
        let out = match t {
            "ESC" => {
                all_mapped &= c.arm_dead_key();
                continue;
            }
            "BS" => c.backspace(&text),
            "ENTER" => Some(c.enter(&text)),
            _ if t.starts_with("ALT:") => c.alt_numpad(&t[4..], &text),
            _ if t.starts_with("CTRL+") => {
                c.ctrl_shift_digit(t[5..].chars().next().unwrap_or(' '), &text)
            }
            _ => {
                let (ch, altgr) = match t.strip_prefix("G:") {
                    Some(rest) => (rest, true),
                    None => (t, false),
                };
                let ch = ch.chars().next().unwrap_or(' ');
                let stroke = Keystroke::from_us_char(ch, altgr)
                    .unwrap_or_else(|| panic!("{ch:?} is not on a US board"));
                c.press(stroke, &text)
            }
        };
        match out {
            Some(o) => {
                o.edit.apply_to(&mut text);
            }
            None if t != "BS" => all_mapped = false,
            None => {}
        }
    }
    (text, c.preedit().unwrap_or_default().to_owned(), all_mapped)
}

/// Check one book; returns (rows checked, problems).
fn check(slug: &str) -> (usize, Vec<String>) {
    let f = fixture(slug);
    assert_eq!(f["schema_version"], 1);
    assert_eq!(f["layout"], slug);
    assert!(is_sha256(f["sha256"].as_str().unwrap_or_default()));
    let l = layout(slug);
    let rows = f["rows"].as_array().expect("rows");
    let mut problems = Vec::new();
    for r in rows {
        let tc = r["tc"].as_str().unwrap_or_default();
        if r["status"] == "not_mapped" {
            continue; // asserted one by one below
        }
        let tokens = r["input"].as_array().expect("input");
        let expect = normalize(r["expect"].as_str().unwrap_or_default());
        let (typed, preedit, mapped) = replay(l, tokens);
        if !mapped {
            problems.push(format!("{tc}: {tokens:?} is unmapped"));
            continue;
        }
        let got = if l.encoding() == Encoding::Legacy {
            legacy_meaning(l, r, tokens, &typed)
        } else {
            format!("{typed}{preedit}")
        };
        if got != expect {
            problems.push(format!(
                "{tc}: {tokens:?} gave {got:?}, book says {expect:?}"
            ));
        }
    }
    (rows.len(), problems)
}

/// Kruti Dev stores raw ASCII/CP1252: check the stored stream, then what the
/// font draws for it.
fn legacy_meaning(
    l: &'static dyn KeyboardLayout,
    r: &Value,
    tokens: &[Value],
    typed: &str,
) -> String {
    if let Some(ansi) = r["ansi"].as_str() {
        if typed != ansi {
            return format!("stored {typed:?} not {ansi:?}");
        }
        let code: u16 = r["input"][0].as_str().unwrap_or_default()[4..]
            .parse()
            .unwrap_or(0);
        return l
            .alt_code(code)
            .map(|a| a.meaning.to_owned())
            .unwrap_or_default();
    }
    if tokens.len() == 1 {
        let t = tokens[0].as_str().unwrap_or_default();
        let Some(stroke) = t
            .chars()
            .next()
            .and_then(|ch| Keystroke::from_us_char(ch, false))
        else {
            return typed.to_owned();
        };
        if stroke.key == radha_layouts::SPACE || t == "BS" || t == "ENTER" {
            return typed.to_owned();
        }
        let entry = l.table().get(stroke).expect("mapped");
        if typed != entry.output {
            return format!("stored {typed:?} not {:?}", entry.output);
        }
        return entry.text().to_owned();
    }
    to_unicode(typed).text
}

#[test]
fn every_book_row_types_what_the_book_says() {
    let mut all = Vec::new();
    for (slug, rows) in [
        ("gail", 162),
        ("cbi", 120),
        ("krutidev", 175),
        ("inscript", 128),
    ] {
        let (n, problems) = check(slug);
        assert_eq!(n, rows, "{slug}: book row count");
        println!("{slug}: {}/{n} book rows pass", n - problems.len());
        all.extend(problems);
    }
    assert!(all.is_empty(), "book mismatches:\n{}", all.join("\n"));
}

#[test]
fn conflict_and_unverified_rows_stay_unmapped() {
    let not_mapped = |slug: &str| -> Vec<String> {
        fixture(slug)["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .filter(|r| r["status"] == "not_mapped")
            .map(|r| r["tc"].as_str().unwrap_or_default().to_owned())
            .collect()
    };
    assert_eq!(not_mapped("gail"), ["GAIL-TC-162"]);
    assert_eq!(not_mapped("cbi"), ["CBI-TC-120"]);
    assert_eq!(not_mapped("krutidev"), ["KD010-TC-052"]);
    assert!(not_mapped("inscript").is_empty());

    // GAIL-MAP-162: Esc then [ and Esc then ] are unverified.
    let gail = layout("gail");
    for key in ['[', ']'] {
        let mut c = Composer::new(gail);
        assert!(c.arm_dead_key());
        let stroke = Keystroke::from_us_char(key, false).expect("key");
        assert!(
            c.press(stroke, "").is_none(),
            "Esc then {key} must stay unmapped"
        );
    }
    // CBI-MAP-120: no legacy vendor keys, and CBI has no AltGr layer at all.
    let cbi = layout("cbi");
    assert!(cbi
        .table()
        .entries()
        .iter()
        .all(|e| matches!(e.layer, Layer::Base | Layer::Shift)));
    // KD010-MAP-052: Shift+4 (`$`) is a conflict.
    let kd = layout("krutidev");
    let four = PhysicalKey::from_base('4').expect("key");
    assert!(kd.table().get(Keystroke::new(four, Layer::Shift)).is_none());
}

/// The word-level vectors listed in the keyboard-books spec.
#[test]
fn word_vectors() {
    let typed = |slug: &str, keys: &[&str]| {
        let tokens: Vec<Value> = keys.iter().map(|k| Value::from(*k)).collect();
        replay(layout(slug), &tokens)
    };
    assert_eq!(typed("cbi", &["d", "f", "l", "k", "u"]).0, "किसान");
    assert_eq!(typed("gail", &["f", "d", "l", "k", "u"]).0, "किसान");
    assert_eq!(
        typed("inscript", &["b", "f", "o", "d", "/", "e"]).0,
        "विद्या"
    );
    assert_eq!(typed("krutidev", &["f", "d", "r", "k", "c"]).0, "fdrkc");
    for (raw, word) in [
        ("fdrkc", "किताब"),
        ("/keZ", "धर्म"),
        ("ikVZ", "पार्ट"),
        ("\u{00E7}dk'k", "प्रकाश"),
    ] {
        assert_eq!(to_unicode(raw).text, word, "{raw}");
    }
    // ç is Alt+0231 (CP1252, leading zero required).
    let (s, _, ok) = typed("krutidev", &["ALT:0231", "d", "k", "'", "k"]);
    assert!(ok);
    assert_eq!(s, "\u{00E7}dk'k");
}

#[test]
fn backspace_rolls_the_sequence_buffer_back() {
    // Backspace removes one code point and the buffer forgets every key
    // whose text is gone, so the next key never joins with deleted text.
    let tok = |keys: &[&str]| -> Vec<Value> { keys.iter().map(|k| Value::from(*k)).collect() };
    // v k s = ओ (one code point); Backspace empties the text and s is a
    // bare े, not a rejoined ओ.
    assert_eq!(replay(layout("gail"), &tok(&["v", "k", "s", "BS"])).0, "");
    assert_eq!(
        replay(layout("gail"), &tok(&["v", "k", "s", "BS", "s"])).0,
        "\u{0947}"
    );
    // d k = का; Backspace leaves क, and s gives के (not को).
    assert_eq!(replay(layout("gail"), &tok(&["d", "k", "BS", "s"])).0, "के");
    // m Backspace Q: उ is gone, so Q is a plain फ.
    assert_eq!(replay(layout("gail"), &tok(&["m", "BS", "Q"])).0, "फ");
    // Poonchh: उ then फ is ऊ; उ Space Backspace फ is उफ (SRC-11 workaround).
    assert_eq!(replay(layout("gail"), &tok(&["m", "Q"])).0, "ऊ");
    assert_eq!(replay(layout("gail"), &tok(&["m", " ", "BS", "Q"])).0, "उफ");
    assert_eq!(replay(layout("cbi"), &tok(&["j", "Q"])).0, "रु");
    // Backspace rolls the buffer back to the state after उ, so Q joins it;
    // only a Space (or any other key) between them breaks the sequence.
    assert_eq!(replay(layout("gail"), &tok(&["m", "d", "BS", "Q"])).0, "ऊ");
    assert_eq!(replay(layout("gail"), &tok(&["m", "d", "Q"])).0, "उकफ");
}

#[test]
fn reph_typed_after_the_syllable_moves_before_the_base() {
    let tok = |keys: &[&str]| -> Vec<Value> { keys.iter().map(|k| Value::from(*k)).collect() };
    for slug in ["gail", "cbi"] {
        assert_eq!(replay(layout(slug), &tok(&["d", "Z"])).0, "र्क", "{slug}");
        assert_eq!(
            replay(layout(slug), &tok(&["U", "d", "Z"])).0,
            "र्न्क",
            "{slug}"
        );
    }
    // InScript: the reph is typed before (j d k = र्क; Shift+4 k = र्क).
    assert_eq!(replay(layout("inscript"), &tok(&["j", "d", "k"])).0, "र्क");
    assert_eq!(replay(layout("inscript"), &tok(&["$", "k"])).0, "र्क");
}

#[test]
fn numpad_follows_caps_lock_on_gail_and_cbi() {
    for slug in ["gail", "cbi"] {
        let mut c = Composer::new(layout(slug));
        let on = c.numpad_digit(7, true, "").expect("digit");
        let off = c.numpad_digit(7, false, "").expect("digit");
        assert_eq!(
            (on.edit.insert.as_str(), off.edit.insert.as_str()),
            ("7", "७"),
            "{slug}"
        );
    }
    let mut q = Composer::new(layout("qwerty"));
    assert_eq!(
        q.numpad_digit(7, false, "").expect("digit").edit.insert,
        "7"
    );
}

#[test]
fn kruti_dev_alt_codes_need_the_leading_zero() {
    let mut c = Composer::new(layout("krutidev"));
    assert!(c.alt_numpad("0188", "").is_some());
    assert!(c.alt_numpad("188", "").is_none(), "Alt+188 is CP437, not ¼");
    let mut g = Composer::new(layout("gail"));
    assert!(g.alt_numpad("63", "").is_some());
    assert!(
        g.alt_numpad("063", "").is_none(),
        "Alt+063 is not the ASCII form"
    );
}

#[test]
fn nukta_letters_match_in_either_form() {
    // U+0958..095F are composition-excluded: NFC keeps consonant + nukta, so
    // a target written precomposed and the typed decomposed form compare
    // equal after normalisation.
    for (pre, dec) in [('\u{0958}', "क\u{093C}"), ('\u{095E}', "फ\u{093C}")] {
        assert_eq!(normalize(&pre.to_string()), dec);
    }
    let tok = |keys: &[&str]| -> Vec<Value> { keys.iter().map(|k| Value::from(*k)).collect() };
    assert_eq!(
        replay(layout("gail"), &tok(&["d", "`"])).0,
        normalize("\u{0958}")
    );
    assert_eq!(
        replay(layout("inscript"), &tok(&["G:k"])).0,
        normalize("\u{0958}")
    );
}
