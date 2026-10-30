//! Hindi grapheme spike (RULES.MD Rule 35), kept as a permanent regression
//! test. Decision record: `docs/hindi-grapheme-decision.md`.
//!
//! For 27 words/drills covering conjuncts, matras, reph, nukta (both
//! spellings), ZWJ/ZWNJ, chandrabindu, anusvara and visarga it checks that
//! `radha_core::akshar::segment` yields the hand-authored aksharas, that
//! segmentation is lossless over NFC, and that the akshar matcher scores a
//! correct word as 0 errors and a one-akshar typo as exactly 1 error.
//!
//! It also runs a test-only port of Golu Expert AI's `IndicGraphemeSegmenter`
//! (Expert AI is read-only and never built, so the C# cannot be run). .NET's
//! `StringInfo` predates the Unicode 15.1 GB9c conjunct rule, so its clusters
//! are emulated as "extended clusters, split again before every base letter
//! after the first", then Golu's two fixes are applied. The port is validated
//! against Golu's own segmentation test vectors before its table column is
//! trusted. Run with `--nocapture` to print the comparison table.

use radha_core::akshar::{normalize, segment};
use radha_core::{Akshar, Matcher};
use unicode_normalization::char::is_combining_mark;
use unicode_segmentation::UnicodeSegmentation;

/// (label, text, expected aksharas).
fn words() -> Vec<(&'static str, String, Vec<&'static str>)> {
    vec![
        ("namaste", "नमस्ते".into(), vec!["न", "म", "स्ते"]),
        ("kshetra", "क्षेत्र".into(), vec!["क्षे", "त्र"]),
        ("rashtriya", "राष्ट्रीय".into(), vec!["रा", "ष्ट्री", "य"]),
        ("vidyalaya", "विद्यालय".into(), vec!["वि", "द्या", "ल", "य"]),
        ("hindi", "हिन्दी".into(), vec!["हि", "न्दी"]),
        ("karya (reph)", "कार्य".into(), vec!["का", "र्य"]),
        ("dharma (reph)", "धर्म".into(), vec!["ध", "र्म"]),
        ("shri", "श्री".into(), vec!["श्री"]),
        ("gyan", "ज्ञान".into(), vec!["ज्ञा", "न"]),
        ("trishul", "त्रिशूल".into(), vec!["त्रि", "शू", "ल"]),
        ("stri (2 viramas)", "स्त्री".into(), vec!["स्त्री"]),
        ("sanskrit (anusvara)", "संस्कृत".into(), vec!["सं", "स्कृ", "त"]),
        ("drishti", "दृष्टि".into(), vec!["दृ", "ष्टि"]),
        ("dvar", "द्वार".into(), vec!["द्वा", "र"]),
        ("udghatan", "उद्घाटन".into(), vec!["उ", "द्घा", "ट", "न"]),
        ("hansna (chandrabindu)", "हँसना".into(), vec!["हँ", "स", "ना"]),
        ("chand (chandrabindu)", "चाँद".into(), vec!["चाँ", "द"]),
        ("aankh (chandrabindu)", "आँख".into(), vec!["आँ", "ख"]),
        ("dukh (visarga)", "दुःख".into(), vec!["दुः", "ख"]),
        ("rishi", "ऋषि".into(), vec!["ऋ", "षि"]),
        (
            "zaroor (nukta, decomposed)",
            "\u{091C}\u{093C}\u{0930}\u{0942}\u{0930}".into(),
            vec!["\u{091C}\u{093C}", "\u{0930}\u{0942}", "\u{0930}"],
        ),
        (
            "phone (U+095E precomposed)",
            "\u{095E}\u{094B}\u{0928}".into(),
            vec!["\u{092B}\u{093C}\u{094B}", "\u{0928}"],
        ),
        (
            "sadak (U+095C precomposed)",
            "\u{0938}\u{095C}\u{0915}".into(),
            vec!["\u{0938}", "\u{0921}\u{093C}", "\u{0915}"],
        ),
        (
            "padhai (nukta + matra)",
            "\u{092A}\u{0922}\u{093C}\u{093E}\u{0908}".into(),
            vec!["\u{092A}", "\u{0922}\u{093C}\u{093E}", "\u{0908}"],
        ),
        (
            "ksha + ZWJ",
            "\u{0915}\u{094D}\u{200D}\u{0937}".into(),
            vec!["\u{0915}\u{094D}\u{200D}\u{0937}"],
        ),
        (
            "ksha + ZWNJ",
            "\u{0915}\u{094D}\u{200C}\u{0937}".into(),
            vec!["\u{0915}\u{094D}\u{200C}", "\u{0937}"],
        ),
        (
            "isolated matra drill",
            "\u{0902} \u{0947} \u{093F}".into(),
            vec!["\u{0902}", " ", "\u{0947}", " ", "\u{093F}"],
        ),
    ]
}

/// .NET `char.IsLetter`: a letter that is not a combining mark.
fn is_letter(c: char) -> bool {
    c.is_alphabetic() && !is_combining_mark(c)
}

fn is_virama(c: char) -> bool {
    matches!(
        c,
        '\u{094D}'
            | '\u{09CD}'
            | '\u{0A4D}'
            | '\u{0ACD}'
            | '\u{0B4D}'
            | '\u{0BCD}'
            | '\u{0C4D}'
            | '\u{0CCD}'
            | '\u{0D4D}'
    )
}

/// Port of Golu `IndicGraphemeSegmenter.SegmentToStrings` over pre-GB9c
/// clusters (see the module doc).
fn golu_emulation(text: &str) -> Vec<String> {
    // Pre-GB9c StringInfo: a consonant after a virama starts a new element.
    let mut legacy: Vec<String> = Vec::new();
    for cluster in text.graphemes(true) {
        let mut piece = String::new();
        for c in cluster.chars() {
            if !piece.is_empty() && is_letter(c) {
                legacy.push(std::mem::take(&mut piece));
            }
            piece.push(c);
        }
        legacy.push(piece);
    }
    // Golu defect 1: split a baseless mark off a non-letter.
    let mut raw: Vec<String> = Vec::new();
    for element in legacy {
        let mut chars = element.chars();
        let first = chars.next();
        let rest = chars.as_str();
        match first {
            Some(f) if !rest.is_empty() && !is_letter(f) && rest.chars().all(is_combining_mark) => {
                raw.push(f.to_string());
                raw.push(rest.to_string());
            }
            _ => raw.push(element),
        }
    }
    // Golu conjunct merge, guarded by defect 2: only a following letter joins.
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let mut cur = raw[i].clone();
        i += 1;
        while i < raw.len()
            && ends_with_virama(&cur)
            && raw[i].chars().next().is_some_and(is_letter)
        {
            cur.push_str(&raw[i]);
            i += 1;
        }
        out.push(cur);
    }
    out
}

fn ends_with_virama(s: &str) -> bool {
    let mut rev = s.chars().rev();
    match rev.next() {
        Some('\u{200D}') => rev.next().is_some_and(is_virama),
        Some(c) => is_virama(c),
        None => false,
    }
}

fn code_points(s: &str) -> String {
    s.chars()
        .map(|c| format!("{:04X}", c as u32))
        .collect::<Vec<_>>()
        .join(" ")
}

fn errors_akshar(target: &str, typed: &str) -> usize {
    let mut m = Matcher::<Akshar>::from_target(target);
    typed.chars().for_each(|c| m.push(c));
    m.evaluate().0.iter().filter(|o| o.is_error()).count()
}

fn errors_char(target: &str, typed: &str) -> usize {
    let mut m = Matcher::new(target);
    typed.chars().for_each(|c| m.push(c));
    m.evaluate().0.iter().filter(|o| o.is_error()).count()
}

#[test]
fn golu_emulation_reproduces_golus_own_vectors() {
    // tests\Golu.Tests\Text\IsolatedMatraSegmentationTests.cs
    let cases: [(&str, &[&str]); 8] = [
        (
            "\u{0902} \u{0902} \u{0902}",
            &["\u{0902}", " ", "\u{0902}", " ", "\u{0902}"],
        ),
        ("\u{0947} \u{0947}", &["\u{0947}", " ", "\u{0947}"]),
        (
            "\u{093F} \u{0915} \u{093F}",
            &["\u{093F}", " ", "\u{0915}", " ", "\u{093F}"],
        ),
        (
            "\u{0936}\u{094D} \u{0936}\u{094D}",
            &["\u{0936}\u{094D}", " ", "\u{0936}\u{094D}"],
        ),
        ("\u{0915}\u{094D}", &["\u{0915}\u{094D}"]),
        (
            "\u{0923}\u{094D}\u{0947}2",
            &["\u{0923}\u{094D}\u{0947}", "2"],
        ),
        ("नमस्ते", &["न", "म", "स्ते"]),
        ("क्षेत्र", &["क्षे", "त्र"]),
    ];
    for (text, expected) in cases {
        assert_eq!(golu_emulation(text), expected, "{text:?}");
        assert_eq!(segment(text), expected, "Radha agrees on {text:?}");
    }
}

#[test]
fn every_spike_word_segments_as_expected_and_scores_per_akshar() {
    let mut failures = Vec::new();
    println!("| word | code points | expected | Radha | Golu (emulated) | Radha = expected | char errors for 1 typo | akshar errors for 1 typo |");
    println!("|---|---|---|---|---|---|---|---|");
    for (label, text, expected) in words() {
        let radha = segment(&text);
        let golu = golu_emulation(&normalize(&text));
        let ok = radha == expected;
        if !ok {
            failures.push(format!("{label}: expected {expected:?}, got {radha:?}"));
        }
        if radha.concat() != normalize(&text) {
            failures.push(format!("{label}: segmentation is not lossless"));
        }
        if errors_akshar(&text, &text) != 0 {
            failures.push(format!("{label}: a correct word scored errors"));
        }
        // One-akshar typo: the last akshar replaced by ॐ (never a prefix).
        let mut typo = radha.clone();
        if let Some(last) = typo.last_mut() {
            *last = "\u{0950}".to_string();
        }
        let typo = typo.concat();
        let akshar_errors = errors_akshar(&text, &typo);
        if akshar_errors != 1 {
            failures.push(format!(
                "{label}: a one-akshar typo scored {akshar_errors} errors"
            ));
        }
        println!(
            "| {label} | {} | {} | {} | {} | {} | {} | {akshar_errors} |",
            code_points(&normalize(&text)),
            expected.join("·"),
            radha.join("·"),
            golu.join("·"),
            if ok { "yes" } else { "NO" },
            errors_char(&text, &typo),
        );
    }
    assert!(words().len() >= 20, "the spike covers at least 20 words");
    assert!(
        failures.is_empty(),
        "spike failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn a_virama_never_swallows_a_following_space() {
    assert_eq!(segment("श् श्"), ["श्", " ", "श्"]);
    assert_eq!(segment("क् "), ["क्", " "]);
}

#[test]
fn a_cluster_typed_key_by_key_is_never_wrong_mid_build() {
    let target = "क्षेत्र";
    let mut m = Matcher::<Akshar>::from_target(target);
    for c in target.chars() {
        m.push(c);
        let e = m.evaluate_detailed();
        assert!(
            e.outcomes.iter().all(|o| !o.is_error()),
            "after {:?}: {:?}",
            m.typed(),
            e.outcomes
        );
    }
    assert!(m.evaluate_detailed().finished);
}
