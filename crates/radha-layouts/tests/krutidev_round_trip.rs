//! Kruti Dev Unicode <-> legacy round trips over generated syllables.

mod common;

use radha_layouts::krutidev::{alt_codes, expected_round_trip, to_legacy, to_unicode};
use radha_layouts::TokenKind;

use common::{layout, SplitMix64};

/// Every full consonant Kruti Dev can type: direct keys and half + `k`.
fn consonants() -> Vec<String> {
    let mut out = Vec::new();
    for e in layout("krutidev").table().entries() {
        let m = e.text();
        match e.kind {
            TokenKind::Consonant | TokenKind::Conjunct => out.push(m.to_owned()),
            TokenKind::Half => {
                if let Some(full) = m.strip_suffix('\u{094D}') {
                    out.push(full.to_owned());
                }
            }
            _ => {}
        }
    }
    out.sort();
    out.dedup();
    out
}

const SIGNS: [&str; 15] = [
    "", "\u{093E}", "\u{093F}", "\u{0940}", "\u{0941}", "\u{0942}", "\u{0943}", "\u{0947}",
    "\u{0948}", "\u{094B}", "\u{094C}", "\u{0902}", "\u{0903}", "\u{0945}", "\u{0949}",
];

fn round_trips(x: &str) -> Result<(), String> {
    let c = to_legacy(x);
    if !c.failures.is_empty() {
        return Err(format!("{x}: failures {:?}", c.failures));
    }
    let back = to_unicode(&c.legacy).text;
    if back != expected_round_trip(x) {
        return Err(format!("{x} -> {} -> {back}", c.legacy));
    }
    let again = to_legacy(&back).legacy;
    if again != c.legacy {
        return Err(format!("{x}: legacy {} re-converts to {again}", c.legacy));
    }
    Ok(())
}

#[test]
fn generated_syllables_round_trip() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for cons in consonants() {
        for sign in SIGNS {
            let plain = format!("{cons}{sign}");
            let rakar = format!("{cons}\u{094D}र{sign}");
            let reph = format!("र\u{094D}{cons}{sign}");
            let nukta = format!("{cons}\u{093C}{sign}");
            // Well-formed aksharas only: ready-made conjuncts take signs but
            // no rakar/reph/nukta; रु/रू take nothing; nukta only on the
            // consonants Hindi writes with it.
            let multi = cons.chars().count() > 1;
            if multi
                && cons.ends_with(|c: char| ('\u{093E}'..='\u{094C}').contains(&c))
                && !sign.is_empty()
            {
                continue;
            }
            let mut variants = vec![plain];
            if !multi && cons != "र" {
                variants.push(rakar);
                variants.push(reph);
            }
            if ["क", "ख", "ग", "ज", "ड", "ढ", "फ"].contains(&cons.as_str()) {
                variants.push(nukta);
            }
            for x in variants {
                checked += 1;
                if let Err(e) = round_trips(&x) {
                    failures.push(e);
                }
            }
        }
    }
    for v in ["अ", "आ", "इ", "ई", "उ", "ऊ", "ऋ", "ए", "ऐ", "ओ", "औ", "ऑ"] {
        checked += 1;
        if let Err(e) = round_trips(v) {
            failures.push(e);
        }
    }
    for d in ["0", "5", "९", "१२३"] {
        checked += 1;
        if let Err(e) = round_trips(d) {
            failures.push(e);
        }
    }
    // Book 04 adds glyph shortcuts that start with a dependent sign (`िं`
    // Alt+0199, `ीं` 0200, `ीर्` 0202): a description of a drawn glyph, not
    // a syllable, so there is no Unicode text to round-trip. Every other
    // code's meaning must round-trip.
    for alt in alt_codes().iter().filter(|a| {
        !a.meaning
            .chars()
            .next()
            .is_some_and(|c| ('\u{093E}'..='\u{094C}').contains(&c))
    }) {
        checked += 1;
        if let Err(e) = round_trips(alt.meaning) {
            failures.push(e);
        }
    }
    println!(
        "krutidev round trip: {checked} generated strings, {} failures",
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "round-trip failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn words_and_sentences_round_trip() {
    for x in [
        "हिंदी भारत",
        "विद्यार्थी कार्य करते हैं।",
        "क्रिया शिक्षा कृषि",
        "उद्देश्य और डॉक्टर",
        "माँ (क) डा॰",
        "पूर्ण धर्म कर्म",
    ] {
        round_trips(x).unwrap_or_else(|e| panic!("{e}"));
    }
}

#[test]
fn legacy_output_stays_inside_its_alphabet() {
    let alt: Vec<char> = alt_codes().iter().map(|a| a.ansi).collect();
    let mut rng = SplitMix64(0x5EED_4B44);
    for _ in 0..500 {
        let len = rng.below(12);
        let s: String = (0..len)
            .map(|_| {
                let r = rng.below(10);
                if r == 0 {
                    ' '
                } else if r == 1 {
                    char::from(b'a' + rng.below(26) as u8)
                } else {
                    char::from_u32(0x0900 + rng.below(0x80) as u32).unwrap_or('क')
                }
            })
            .collect();
        let c = to_legacy(&s);
        for ch in c.legacy.chars() {
            assert!(
                (ch.is_ascii_graphic() || matches!(ch, ' ' | '\n' | '\t') || alt.contains(&ch)),
                "{s:?} gave {ch:?}"
            );
        }
        let _ = to_unicode(&c.legacy);
        let raw: String = (0..len)
            .map(|_| char::from(32 + rng.below(95) as u8))
            .collect();
        let _ = to_unicode(&raw);
    }
}
